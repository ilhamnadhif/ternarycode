//! Shell integration on real shells over a real PTY: the user's startup files load exactly once
//! and in order, their aliases and hooks keep working, and the OSC 133 marks (and OSC 7) come out
//! as `tc_term`'s scanner parses them. Without the environment from `tc_agent::shell::env` the
//! same shells write no marks.
//!
//! Shells run under `env -i` with a temporary HOME, so nothing of the developer's environment or
//! dotfiles gets in. Login shells start the way TernaryCode's own (`/usr/bin/login ... /bin/zsh
//! -fc "exec -a -zsh $SHELL"`) do, minus `login`, which would reset HOME.

#![cfg(unix)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tc_agent::shell;
use tc_term::alacritty_terminal::grid::Dimensions;
use tc_term::alacritty_terminal::index::{Column, Line};
use tc_term::core::{Feeder, GridSize, new_terminal, term_config};
use tc_term::osc::{OscSignal, PromptMark};
use tc_term::pty::{ExitInfo, LocalPty, Session, SessionEvent, SpawnOptions, start};
use tc_term::scheme::SCHEMES;

use PromptMark::{CommandExecuted as C, CommandFinished, CommandStart as B, PromptStart as A};

const TIMEOUT: Duration = Duration::from_secs(20);
/// Text every test prompt contains.
const PROMPT: &str = "TCPROMPT";

fn d(code: i32) -> PromptMark {
    CommandFinished(Some(code))
}

/// A temporary HOME (with a cwd below it whose name needs percent-encoding) and an installed
/// copy of the scripts.
struct Fixture {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    cwd: PathBuf,
    dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let home = root.join("home");
        let cwd = home.join("my project é%20");
        fs::create_dir_all(&cwd).unwrap();
        let dir = root.join("data/shell");
        shell::install(&dir).unwrap();
        Self {
            _tmp: tmp,
            home,
            cwd,
            dir,
        }
    }

    fn write(&self, rel: &str, content: &str) {
        assert!(Path::new(rel).is_relative());
        let path = self.home.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// Lines the user's startup files appended to `$HOME/log`.
    fn log(&self) -> Vec<String> {
        fs::read_to_string(self.home.join("log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// The environment of a pane, before the integration's variables.
    fn base_env(&self, extra: &[(&str, &str)]) -> Vec<(String, String)> {
        let mut env = vec![
            ("HOME".to_string(), self.home.display().to_string()),
            ("USER".into(), "tester".into()),
            ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
            ("TERM".into(), "xterm-256color".into()),
            ("TERM_PROGRAM".into(), "TernaryCode".into()),
            ("LANG".into(), "en_US.UTF-8".into()),
            ("BASH_SILENCE_DEPRECATION_WARNING".into(), "1".into()),
        ];
        env.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        env
    }

    /// `base` plus the integration's variables for `shell`, computed from `base`.
    fn integrated(&self, shell_path: &str, base: Vec<(String, String)>) -> Vec<(String, String)> {
        let map: HashMap<String, String> = base.iter().cloned().collect();
        let added = shell::env(&self.dir, shell_path, |k| map.get(k).cloned());
        assert!(!added.is_empty(), "no integration for {shell_path}");
        let mut env: Vec<_> = base
            .into_iter()
            .filter(|(k, _)| !added.iter().any(|(a, _)| a == k))
            .collect();
        env.extend(added);
        env
    }
}

struct Pane {
    _rt: tokio::runtime::Runtime,
    feeder: Arc<Mutex<Feeder>>,
    events: mpsc::Receiver<SessionEvent>,
    session: Session,
    signals: Vec<OscSignal>,
    exit: Option<ExitInfo>,
}

impl Pane {
    /// Runs `argv` under `env -i` with exactly `env`.
    fn spawn(env: &[(String, String)], argv: &[&str], cwd: &Path) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let size = GridSize {
            cols: 200,
            rows: 60,
        };
        let (shared, rx) = new_terminal(
            term_config(1000, Default::default(), false),
            size,
            (&SCHEMES[0]).into(),
        );
        let feeder = Arc::new(Mutex::new(Feeder::new(shared, rx)));
        let mut args = vec!["-i".to_string()];
        args.extend(env.iter().map(|(k, v)| format!("{k}={v}")));
        args.extend(argv.iter().map(|a| a.to_string()));
        let pty = LocalPty::spawn(&SpawnOptions {
            program: Some(("/usr/bin/env".into(), args)),
            cwd: cwd.to_path_buf(),
            env: HashMap::new(),
            size,
            cell_px: (8, 16),
        })
        .unwrap();
        let (tx, events) = mpsc::channel();
        let tx = Mutex::new(tx);
        let session = start(
            rt.handle(),
            pty,
            feeder.clone(),
            Arc::new(move |e| {
                let _ = tx.lock().send(e);
            }),
        )
        .unwrap();
        Self {
            _rt: rt,
            feeder,
            events,
            session,
            signals: Vec::new(),
            exit: None,
        }
    }

    fn screen(&self) -> String {
        let f = self.feeder.lock();
        let term = f.shared.term.lock();
        (0..term.screen_lines() as i32)
            .map(|l| {
                let row = &term.grid()[Line(l)];
                (0..term.columns())
                    .map(|c| row[Column(c)].c)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Prompts drawn so far (lines showing the test prompt).
    fn prompts(&self) -> usize {
        self.screen().lines().filter(|l| l.contains(PROMPT)).count()
    }

    /// Takes events until `done` holds; panics on timeout or exit.
    fn wait(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let end = Instant::now() + TIMEOUT;
        while !done(self) {
            if self.exit.is_some() {
                panic!("shell exited waiting for {what}:\n{}", self.screen());
            }
            let left = end.saturating_duration_since(Instant::now());
            match self
                .events
                .recv_timeout(left.min(Duration::from_millis(200)))
            {
                Ok(SessionEvent::Output(fed)) => self.signals.extend(fed.signals),
                Ok(SessionEvent::Exited(info)) => self.exit = Some(info),
                Ok(SessionEvent::Foreground) => {}
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < end => {}
                Err(_) => panic!("timed out waiting for {what}:\n{}", self.screen()),
            }
        }
    }

    /// Types `line` + Enter at the prompt and waits for the next prompt.
    fn run(&mut self, line: &str) {
        let before = self.prompts();
        // Let the line editor settle on the prompt first.
        std::thread::sleep(Duration::from_millis(50));
        self.session.write(format!("{line}\r").into_bytes());
        self.wait(&format!("the prompt after {line:?}"), |p| {
            p.prompts() > before
        });
    }

    fn exit(mut self) -> Self {
        std::thread::sleep(Duration::from_millis(50));
        self.session.write(b"exit\r".to_vec());
        self.wait("exit", |p| p.exit.is_some());
        self
    }

    /// The marks, a prompt start written again at once counted once (zsh writes it from precmd
    /// and again from its prompt, which also repeats it on every redraw).
    fn marks(&self) -> Vec<PromptMark> {
        let mut marks: Vec<PromptMark> = self
            .signals
            .iter()
            .filter_map(|s| match s {
                OscSignal::Prompt(m) => Some(m.clone()),
                _ => None,
            })
            .collect();
        marks.dedup_by(|b, a| *a == A && *b == A);
        marks
    }

    fn cwds(&self) -> Vec<PathBuf> {
        self.signals
            .iter()
            .filter_map(|s| match s {
                OscSignal::Cwd(p) => Some(PathBuf::from(p)),
                _ => None,
            })
            .collect()
    }
}

/// Starts a shell, waits for its first prompt, runs `lines`, exits.
fn session(env: &[(String, String)], argv: &[&str], cwd: &Path, lines: &[&str]) -> Pane {
    let mut pane = Pane::spawn(env, argv, cwd);
    pane.wait("the first prompt", |p| p.prompts() >= 1);
    for line in lines {
        pane.run(line);
    }
    pane.exit()
}

fn command_v(name: &str) -> Option<String> {
    let out = std::process::Command::new("/bin/sh")
        .args(["-c", &format!("command -v {name}")])
        .output()
        .ok()?;
    let path = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && path.starts_with('/')).then_some(path)
}

/// Commands every test runs, with the marks each one adds after the first prompt's: `tcalias`
/// (the user's alias, prints ALIAS_42), `false`, an empty line, `true`, then `exit` (C only).
const LINES: &[&str] = &["tcalias", "false", "", "true"];

fn command_marks() -> Vec<PromptMark> {
    vec![
        C,
        d(0),
        A,
        B,
        C,
        d(1),
        A,
        B,
        // The empty line: a new prompt, no command.
        A,
        B,
        C,
        d(0),
        A,
        B,
        // exit
        C,
    ]
}

// zsh

/// Startup files that log themselves and ZDOTDIR; .zshrc adds an alias and a precmd hook that
/// sets PS1 from $? (as themes do).
fn zsh_files(fx: &Fixture, zdotdir: &str) {
    let log = r#"print -r -- "$1 ZDOTDIR=${ZDOTDIR-unset} saved=${TERNARYCODE_ZSH_ZDOTDIR-unset}" >> $HOME/log"#;
    let files = [
        (".zshenv", "zshenv"),
        (".zprofile", "zprofile"),
        (".zshrc", "zshrc"),
        (".zlogin", "zlogin"),
    ];
    for (file, name) in files {
        let mut body = format!("() {{ {log} }} {name}\n");
        if name == "zshrc" {
            body.push_str(
                r#"alias tcalias='echo ALIAS_$((40+2))'
_user_theme() { local st=$?; PS1="[$st]TCPROMPT%# "; true }
autoload -Uz add-zsh-hook
add-zsh-hook precmd _user_theme
print -r -- "HISTFILE=$HISTFILE" >> $HOME/log.hist
"#,
            );
        }
        fx.write(&Path::new(zdotdir).join(file).to_string_lossy(), &body);
    }
}

const ZSH: &str = "/bin/zsh";
const ZSH_LOGIN: &[&str] = &[ZSH, "-fc", "exec -a -zsh /bin/zsh"];

fn zsh_log(zdotdir: &str) -> Vec<String> {
    ["zshenv", "zprofile", "zshrc", "zlogin"]
        .iter()
        .map(|f| format!("{f} ZDOTDIR={zdotdir} saved=unset"))
        .collect()
}

#[test]
fn zsh_login_shell_with_integration() {
    let fx = Fixture::new();
    zsh_files(&fx, "");
    let env = fx.integrated(ZSH, fx.base_env(&[]));
    let pane = session(&env, ZSH_LOGIN, &fx.cwd, LINES);
    assert_eq!(fx.log(), zsh_log("unset"));
    // /etc/zshrc (macOS) saw the user's ZDOTDIR, not ours.
    let hist = fs::read_to_string(fx.home.join("log.hist")).unwrap();
    assert!(!hist.contains(fx.dir.to_str().unwrap()), "{hist}");
    let screen = pane.screen();
    assert!(screen.contains("ALIAS_42"), "{screen}");
    // The theme's hook still sees the command's status.
    assert!(screen.contains("[1]TCPROMPT"), "{screen}");
    // First prompt: A only (the theme's hook ran after ours and replaced PS1); after that the
    // integration's hook comes last and B is in every prompt.
    let mut expected = vec![A];
    expected.extend(command_marks());
    assert_eq!(pane.marks(), expected, "{screen}");
    let cwds = pane.cwds();
    assert!(
        !cwds.is_empty() && cwds.iter().all(|c| *c == fx.cwd),
        "{cwds:?}"
    );
    assert_eq!(pane.exit, Some(ExitInfo::Code(0)));
}

#[test]
fn zsh_without_integration_writes_no_marks() {
    let fx = Fixture::new();
    zsh_files(&fx, "");
    let pane = session(&fx.base_env(&[]), ZSH_LOGIN, &fx.cwd, LINES);
    assert_eq!(fx.log(), zsh_log("unset"));
    assert!(pane.screen().contains("ALIAS_42"));
    assert_eq!(pane.marks(), vec![], "{}", pane.screen());
}

#[test]
fn zsh_with_the_users_own_zdotdir() {
    let fx = Fixture::new();
    zsh_files(&fx, "zdot");
    let zdotdir = fx.home.join("zdot").display().to_string();
    let env = fx.integrated(ZSH, fx.base_env(&[("ZDOTDIR", &zdotdir)]));
    assert!(
        env.iter()
            .any(|(k, v)| k == "TERNARYCODE_ZSH_ZDOTDIR" && *v == zdotdir)
    );
    let pane = session(&env, ZSH_LOGIN, &fx.cwd, LINES);
    assert_eq!(fx.log(), zsh_log(&zdotdir));
    let mut expected = vec![A];
    expected.extend(command_marks());
    assert_eq!(pane.marks(), expected, "{}", pane.screen());
}

#[test]
fn zsh_where_the_users_zshenv_sets_zdotdir() {
    let fx = Fixture::new();
    zsh_files(&fx, "zdot");
    // ~/.zshenv moves everything else to ~/zdot, which has its own .zshenv too (not read: zsh
    // has read .zshenv already by then).
    fx.write(
        ".zshenv",
        r#"print -r -- "home-zshenv" >> $HOME/log; export ZDOTDIR=$HOME/zdot"#,
    );
    let env = fx.integrated(ZSH, fx.base_env(&[]));
    let pane = session(&env, ZSH_LOGIN, &fx.cwd, LINES);
    let zdotdir = fx.home.join("zdot").display().to_string();
    let mut log = vec!["home-zshenv".to_string()];
    log.extend(zsh_log(&zdotdir).into_iter().skip(1));
    assert_eq!(fx.log(), log);
    assert!(pane.screen().contains("ALIAS_42"));
    let mut expected = vec![A];
    expected.extend(command_marks());
    assert_eq!(pane.marks(), expected, "{}", pane.screen());
}

// bash

const BASH: &str = "/bin/bash";
const BASH_LOGIN: &[&str] = &[ZSH, "-fc", "exec -a -bash /bin/bash"];

/// A login setup as many people have it: .bash_profile sources .bashrc; .bashrc adds an alias,
/// appends a theme function to PROMPT_COMMAND that sets PS1 from $?, and sets a DEBUG trap.
fn bash_files(fx: &Fixture) {
    fx.write(
        ".bash_profile",
        r#"echo "bash_profile PROMPT_COMMAND=${PROMPT_COMMAND:+set}" >> $HOME/log
[ -r ~/.bashrc ] && . ~/.bashrc
"#,
    );
    fx.write(
        ".bashrc",
        r#"echo "bashrc" >> $HOME/log
alias tcalias='echo ALIAS_$((40+2))'
_user_theme() { local st=$?; PS1="[$st]TCPROMPT\\$ "; }
PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }_user_theme"
trap 'userdbg=$((${userdbg:-0}+1))' DEBUG
"#,
    );
    // Never read by a login shell that has .bash_profile.
    fx.write(".profile", "echo profile >> $HOME/log\n");
}

#[test]
fn bash_login_shell_with_integration() {
    let fx = Fixture::new();
    bash_files(&fx);
    let env = fx.integrated(BASH, fx.base_env(&[]));
    let mut lines = LINES.to_vec();
    lines.extend([
        // The user's DEBUG trap still runs; PROMPT_COMMAND is no longer exported.
        "echo DBG_$((userdbg > 0))",
        "printenv PROMPT_COMMAND || echo NOT_EXPORTED_$((1+1))",
    ]);
    let pane = session(&env, BASH_LOGIN, &fx.cwd, &lines);
    assert_eq!(
        fx.log(),
        ["bash_profile PROMPT_COMMAND=set", "bashrc"].map(String::from)
    );
    let screen = pane.screen();
    assert!(screen.contains("ALIAS_42"), "{screen}");
    assert!(screen.contains("[1]TCPROMPT"), "{screen}");
    assert!(screen.contains("DBG_1"), "{screen}");
    assert!(screen.contains("NOT_EXPORTED_2"), "{screen}");
    // First prompt: A only (the theme in PROMPT_COMMAND ran after the bootstrap and replaced
    // PS1); after that the integration's hook comes last and B is in every prompt.
    let mut expected = vec![A];
    let mut commands = command_marks();
    let exit = commands.pop().unwrap();
    expected.extend(commands);
    expected.extend([C, d(0), A, B, C, d(0), A, B, exit]);
    assert_eq!(pane.marks(), expected, "{screen}");
    let cwds = pane.cwds();
    assert!(
        !cwds.is_empty() && cwds.iter().all(|c| *c == fx.cwd),
        "{cwds:?}"
    );
}

#[test]
fn bash_without_integration_writes_no_marks() {
    let fx = Fixture::new();
    bash_files(&fx);
    let pane = session(&fx.base_env(&[]), BASH_LOGIN, &fx.cwd, LINES);
    assert_eq!(
        fx.log(),
        ["bash_profile PROMPT_COMMAND=", "bashrc"].map(String::from)
    );
    assert!(pane.screen().contains("ALIAS_42"));
    assert_eq!(pane.marks(), vec![], "{}", pane.screen());
}

/// Non-login interactive bash with a plain .bashrc that leaves PROMPT_COMMAND alone: the
/// bootstrap is all of PROMPT_COMMAND, so B is in the first prompt already.
#[test]
fn bash_interactive_shell_with_integration() {
    let fx = Fixture::new();
    fx.write(
        ".bashrc",
        r#"echo "bashrc" >> $HOME/log
alias tcalias='echo ALIAS_$((40+2))'
PS1='[$?]TCPROMPT\$ '
"#,
    );
    fx.write(".bash_profile", "echo bash_profile >> $HOME/log\n");
    let env = fx.integrated(BASH, fx.base_env(&[]));
    let pane = session(&env, &[BASH, "-i"], &fx.cwd, LINES);
    assert_eq!(fx.log(), ["bashrc"].map(String::from));
    let screen = pane.screen();
    assert!(screen.contains("ALIAS_42"), "{screen}");
    assert!(screen.contains("[1]TCPROMPT"), "{screen}");
    let mut expected = vec![A, B];
    expected.extend(command_marks());
    assert_eq!(pane.marks(), expected, "{screen}");
}

// fish

#[test]
fn fish_with_integration() {
    let Some(fish) = command_v("fish") else {
        eprintln!("fish is not installed; skipping");
        return;
    };
    let fx = Fixture::new();
    fx.write(
        ".config/fish/config.fish",
        r#"echo config >> $HOME/log
alias tcalias 'echo ALIAS_(math 40+2)'
function fish_prompt; echo -n "[$status]TCPROMPT> "; end
echo "xdg=$XDG_DATA_DIRS saved=$TERNARYCODE_FISH_XDG_DIR" >> $HOME/log
"#,
    );
    let path = format!(
        "{}:/usr/bin:/bin",
        Path::new(&fish).parent().unwrap().display()
    );
    let base = fx.base_env(&[("PATH", &path)]);
    let env = fx.integrated(&fish, base.clone());
    let fish_login = [fish.as_str(), "-l"];
    let version = std::process::Command::new(&fish)
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let major: u32 = version
        .rsplit(' ')
        .next()
        .and_then(|v| v.trim().split('.').next()?.parse().ok())
        .unwrap_or(0);
    let pane = session(&env, &fish_login, &fx.cwd, LINES);
    // config.fish ran once, with XDG_DATA_DIRS back to unset.
    assert_eq!(fx.log(), ["config", "xdg= saved="].map(String::from));
    let screen = pane.screen();
    assert!(screen.contains("ALIAS_42"), "{screen}");
    // fish 4.0+ writes its own marks (and B since 4.3): only check what every version has.
    let marks: Vec<_> = pane.marks().into_iter().filter(|m| *m != B).collect();
    let mut expected = vec![A];
    expected.extend(command_marks().into_iter().filter(|m| *m != B));
    if major < 4 {
        // fish 3 (our script): the empty line's prompt comes without a `fish_prompt` event, so
        // without A, and `exit` still runs `fish_postexec` (D).
        expected = vec![A, C, d(0), A, C, d(1), A, C, d(0), A, C, d(0)];
    }
    assert_eq!(marks, expected, "{screen}");

    // Without the integration, fish before 4.0 writes no marks.
    let _ = fs::remove_file(fx.home.join("log"));
    let plain = session(&base, &fish_login, &fx.cwd, LINES);
    assert_eq!(fx.log(), ["config", "xdg= saved="].map(String::from));
    if major < 4 {
        assert_eq!(plain.marks(), vec![], "{}", plain.screen());
    }
}
