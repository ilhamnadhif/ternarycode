//! Commit, push and pull request with one button: the commit box under Review's file list.
//!
//! [`plan`] reads what the button does from the branch as it is (what is left to commit, where
//! the branch goes, whether a pull request is open); [`App::ship_run`] runs its steps one after
//! the other off the UI thread, each reported as it ends. git runs without prompts (see
//! `tc_git::ship`): a step that needs a password stops at once, and the box offers a terminal.
//! What is done stays done: a failed push never undoes the commit.
//!
//! The message starts from the task the user gave the agent (its prompt's first line) and the
//! agent's last words; ✦ asks that agent (Claude Code, Codex) to write it from the diff.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use tc_core::agent::AgentKind;
use tc_core::model::{ShipAction, ShipPrefs};
use tc_core::{PaneId, ProjectId};
use tc_git::ship::{Busy, Checks, Facts, Forge, GhError, Pr, PrState, PushError, Web};

use crate::actions::Action;
use crate::app::{App, AppMsg};
use crate::i18n::{t, tf};

/// One step of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepKind {
    /// A branch for the pull request, made first (the user was on the base branch).
    Branch(String),
    Commit,
    Push,
    /// With gh the pull request is made; without, its form opens in the Browser panel.
    PullRequest { gh: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepState {
    Todo,
    Running,
    Done,
    Failed(Failure),
}

/// Why a step stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The remote has commits the branch has not (someone pushed first).
    Rejected(String),
    /// A password, token or passphrase is needed: only a terminal can ask for it.
    Credentials(String),
    /// gh is not signed in.
    SignedOut(String),
    /// What git (or a hook, or gh) printed.
    Output(String),
}

impl Failure {
    pub fn output(&self) -> &str {
        match self {
            Failure::Rejected(s) | Failure::Credentials(s) | Failure::SignedOut(s) | Failure::Output(s) => s,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Step {
    pub kind: StepKind,
    pub state: StepState,
    /// What it made: the commit's id, where it pushed, the pull request.
    pub detail: String,
    pub took: Option<Duration>,
}

/// A run of the button: its steps, then the pull request it made or the form it opened.
#[derive(Clone, Debug)]
pub struct Run {
    pub id: u64,
    pub title: String,
    pub steps: Vec<Step>,
    pub pr: Option<Pr>,
    /// The pull request form opened in the browser (no gh).
    pub form: Option<String>,
    /// Every step ended (done or one failed).
    pub finished: bool,
    /// What the run was asked to do, kept for Run in Terminal and the fixes.
    pub req: RunReq,
}

impl Run {
    pub fn failed(&self) -> Option<(usize, &Failure)> {
        self.steps.iter().enumerate().find_map(|(i, s)| match &s.state {
            StepState::Failed(f) => Some((i, f)),
            _ => None,
        })
    }
}

/// The commit box of one project: the branch's facts, its pull request, the message being
/// written, the files left out, and the run in progress or just finished.
#[derive(Default)]
pub struct Ship {
    pub facts: Option<Facts>,
    /// The folder the facts are of (a worktree has its own).
    pub root: Option<PathBuf>,
    pub loading: bool,
    load: u64,
    pub loaded_at: Option<Instant>,
    /// gh, when it is installed (found on the user's PATH).
    pub gh: Option<PathBuf>,
    /// glab (GitLab CLI), when it is installed (found on the user's PATH).
    pub glab: Option<PathBuf>,
    pub pr: Option<Pr>,
    /// The message (subject, blank line, body).
    pub message: String,
    /// The message was made by TernaryCode (from the task, or by ✦) and not edited since: it
    /// may be made again when its source changes.
    pub auto: bool,
    /// What an automatic message was made from (the task and the last words).
    pub auto_from: Option<(Option<String>, Option<String>)>,
    /// ✦ wrote the message: the one before, for Undo.
    pub undo: Option<String>,
    /// ✦ is writing (the agent that does it).
    pub writing: Option<AgentKind>,
    pub written_by: Option<AgentKind>,
    /// Why ✦ could not write it.
    pub write_error: Option<String>,
    /// Files (paths) left out of the commit.
    pub out: HashSet<String>,
    /// "Commit to a New Branch…": the branch's name being typed.
    pub new_branch: Option<String>,
    pub show_commands: bool,
    pub run: Option<Run>,
}

/// What the button does now.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub steps: Vec<StepKind>,
    /// Why nothing can be done now (a merge in progress, conflicts, a detached HEAD).
    pub blocked: Option<String>,
    /// The branch it commits on and pushes (a new one, when one is made first).
    pub branch: String,
    pub remote: Option<String>,
    /// Where a pull request goes.
    pub base: Option<String>,
    /// The open pull request a push adds to.
    pub updates: Option<u64>,
    /// Commits made before this one that the remote does not have.
    pub unpushed: usize,
    /// On the branch a pull request would go into: a new branch can be made for one.
    pub can_branch: bool,
    /// The remote has no pages TernaryCode knows (a folder, a server of its own).
    pub no_pages: bool,
    /// Which forge the remote belongs to (GitHub vs GitLab), for PR / MR wording.
    pub forge: Option<Forge>,
}

/// Picks the CLI binary (`gh` or `glab`) matching `web`'s forge.
pub fn cli_for<'a>(web: Option<&Web>, gh: &'a Option<PathBuf>, glab: &'a Option<PathBuf>) -> Option<&'a PathBuf> {
    match web.map(|w| w.forge) {
        Some(Forge::GitHub) => gh.as_ref(),
        Some(Forge::GitLab) => glab.as_ref(),
        None => None,
    }
}

impl Plan {
    pub fn has(&self, k: &StepKind) -> bool {
        self.steps.contains(k)
    }
    pub fn pr_step(&self) -> Option<bool> {
        self.steps.iter().find_map(|s| match s {
            StepKind::PullRequest { gh } => Some(*gh),
            _ => None,
        })
    }
    pub fn idle(&self) -> bool {
        self.blocked.is_none() && self.steps.is_empty()
    }
}

/// What [`plan`] reads.
pub struct Inputs<'a> {
    pub facts: &'a Facts,
    /// Files checked to go into the commit (not committed yet).
    pub files: usize,
    pub prefs: ShipPrefs,
    pub new_branch: Option<&'a str>,
    /// A worktree's base: its pull request goes there.
    pub base: Option<&'a str>,
    pub pr: Option<&'a Pr>,
    pub gh: bool,
}

/// What the button does for the branch as it is: commit what is checked, push what the remote
/// lacks, open a pull request into the base (not from the base itself: there it pushes).
pub fn plan(i: &Inputs) -> Plan {
    let f = i.facts;
    let base = i.base.map(String::from).or_else(|| f.default_branch.clone());
    let mut p = Plan {
        steps: Vec::new(),
        blocked: None,
        branch: f.branch.clone().unwrap_or_default(),
        remote: f.remote.as_ref().map(|r| r.name.clone()),
        base,
        updates: None,
        unpushed: f.unpushed,
        can_branch: false,
        no_pages: f.remote.as_ref().is_some_and(|r| r.web.is_none()),
        forge: f.remote.as_ref().and_then(|r| r.web.as_ref()).map(|w| w.forge),
    };
    if let Some(b) = f.busy {
        p.blocked = Some(match b {
            Busy::Merge if !f.conflicts.is_empty() => conflicts_note(&f.conflicts),
            Busy::Merge => t("A merge is in progress: commit it in the terminal (git commit), then come back.").to_string(),
            Busy::Rebase => t("A rebase is in progress: finish it in the terminal first.").to_string(),
            Busy::CherryPick => t("A cherry-pick is in progress: finish it in the terminal first.").to_string(),
            Busy::Revert => t("A revert is in progress: finish it in the terminal first.").to_string(),
        });
        return p;
    }
    if !f.conflicts.is_empty() {
        p.blocked = Some(conflicts_note(&f.conflicts));
        return p;
    }
    let Some(branch) = f.branch.clone() else {
        p.blocked = Some(t("HEAD is detached: switch to a branch to commit here.").to_string());
        return p;
    };
    let web = f.remote.as_ref().and_then(|r| r.web.as_ref());
    let new = i.new_branch.map(str::trim).filter(|n| !n.is_empty() && *n != branch);
    let on_base = p.base.as_deref() == Some(branch.as_str());
    p.can_branch = on_base && new.is_none() && web.is_some() && i.prefs.action == ShipAction::PullRequest && (i.files > 0 || f.unpushed > 0);
    if let Some(n) = new {
        p.steps.push(StepKind::Branch(n.to_string()));
        p.branch = n.to_string();
    }
    if i.files > 0 {
        p.steps.push(StepKind::Commit);
    }
    let open_pr = i.pr.filter(|pr| new.is_none() && matches!(pr.state, PrState::Open | PrState::Draft));
    p.updates = open_pr.map(|pr| pr.number);
    let wants_push = i.prefs.action != ShipAction::Commit && f.remote.is_some();
    // What a push carries: the new commit, earlier ones, or a branch the remote has never seen.
    let pushes = wants_push && (i.files > 0 || f.unpushed > 0 || new.is_some() || (f.upstream.is_none() && f.has_commits));
    if pushes {
        p.steps.push(StepKind::Push);
    }
    // A pull request from a branch with something of its own (not one whose request was merged
    // or closed with nothing new since), into a base that is another branch.
    let pr_over = i.pr.is_some_and(|pr| matches!(pr.state, PrState::Merged | PrState::Closed)) && i.files == 0 && f.unpushed == 0 && new.is_none();
    let wants_pr = i.prefs.action == ShipAction::PullRequest
        && web.is_some()
        && (new.is_some() || !on_base)
        && open_pr.is_none()
        && !pr_over
        && p.base.as_deref().is_some_and(|b| b != p.branch);
    if wants_pr && (pushes || f.upstream.is_some()) {
        p.steps.push(StepKind::PullRequest { gh: i.gh });
    }
    p
}

fn conflicts_note(files: &[String]) -> String {
    let names: Vec<&str> = files.iter().take(2).map(|f| f.rsplit('/').next().unwrap_or(f)).collect();
    let mut list = names.join(", ");
    if files.len() > 2 {
        list.push_str(", …");
    }
    tf("Finish the merge first: {} still have conflicts ({}).", &[&crate::i18n::count(files.len(), t("1 file"), t("{} files")), &list])
}

/// The button's words: the whole of what it will do.
pub fn label(p: &Plan) -> String {
    let commit = p.has(&StepKind::Commit);
    let push = p.has(&StepKind::Push);
    let pr = p.pr_step();
    let n = p.unpushed;
    let gitlab = p.forge == Some(Forge::GitLab);
    match (commit, push, pr, gitlab) {
        (true, true, Some(true), true) => t("Commit, Push & Create MR").to_string(),
        (true, true, Some(false), true) => t("Commit, Push & Open MR Form").to_string(),
        (true, true, Some(true), false) => t("Commit, Push & Create PR").to_string(),
        (true, true, Some(false), false) => t("Commit, Push & Open PR Form").to_string(),
        (true, true, None, _) => t("Commit & Push").to_string(),
        (true, false, _, _) => t("Commit").to_string(),
        (false, true, Some(true), true) if n > 0 => crate::i18n::count(n, t("Push 1 Commit & Create MR"), t("Push {} Commits & Create MR")),
        (false, true, Some(false), true) if n > 0 => crate::i18n::count(n, t("Push 1 Commit & Open MR Form"), t("Push {} Commits & Open MR Form")),
        (false, true, Some(true), false) if n > 0 => crate::i18n::count(n, t("Push 1 Commit & Create PR"), t("Push {} Commits & Create PR")),
        (false, true, Some(false), false) if n > 0 => crate::i18n::count(n, t("Push 1 Commit & Open PR Form"), t("Push {} Commits & Open PR Form")),
        (false, true, Some(true), true) => t("Push & Create MR").to_string(),
        (false, true, Some(false), true) => t("Push & Open MR Form").to_string(),
        (false, true, Some(true), false) => t("Push & Create PR").to_string(),
        (false, true, Some(false), false) => t("Push & Open PR Form").to_string(),
        (false, true, None, _) if n > 0 => crate::i18n::count(n, t("Push 1 Commit"), t("Push {} Commits")),
        (false, true, None, _) => t("Push").to_string(),
        (false, false, Some(true), true) => t("Create MR").to_string(),
        (false, false, Some(false), true) => t("Open MR Form").to_string(),
        (false, false, Some(true), false) => t("Create PR").to_string(),
        (false, false, Some(false), false) => t("Open PR Form").to_string(),
        (false, false, None, _) => String::new(),
    }
}

/// A subject made from the task the user gave the agent: its first clause, at most 72
/// characters, starting with a capital.
pub fn title_from_task(task: &str) -> String {
    let mut s = task.trim().trim_start_matches(['-', '*', '>', '#', ' ']).to_string();
    let lower = s.to_ascii_lowercase();
    let mut end = s.len();
    for sep in [", then ", " then ", ", and then ", "; ", ". ", "? ", "! ", ": "] {
        if let Some(i) = lower.find(sep)
            && i >= 8
        {
            end = end.min(i);
        }
    }
    s.truncate(end);
    let s = s.trim().trim_end_matches(['.', ',', ';', ':', '!', '?']).trim().to_string();
    let s = if s.chars().count() > 72 {
        let cut: String = s.chars().take(72).collect();
        match cut.rfind(' ') {
            Some(i) if i >= 30 => cut[..i].trim_end_matches([',', ';', ':']).to_string(),
            _ => cut,
        }
    } else {
        s
    };
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The message made from an agent's task and last words (either may be missing).
pub fn message_from(task: Option<&str>, words: Option<&str>) -> String {
    let title = task.map(title_from_task).filter(|t| !t.is_empty());
    let words = words.map(str::trim).filter(|w| !w.is_empty() && !w.ends_with('?'));
    match (title, words) {
        (Some(t), Some(w)) => format!("{t}\n\n{w}"),
        (Some(t), None) => t,
        (None, Some(w)) => {
            // No task: the first sentence of what the agent said.
            let first = w.split_inclusive(['.', '!']).next().unwrap_or(w).trim().trim_end_matches('.');
            let rest = w[first.len().min(w.len())..].trim_start_matches('.').trim();
            let t = title_from_task(first);
            if rest.is_empty() { t } else { format!("{t}\n\n{rest}") }
        }
        (None, None) => String::new(),
    }
}

/// What ✦ asks the agent.
const WRITE_PROMPT: &str = "Write a git commit message for the change below. First line: a summary in the imperative mood, at most 72 characters, no period at the end. Then a blank line and one to three short sentences on what changed and why. Plain text only: no quotes, no markdown, no code fences. Answer with the message and nothing else.";

/// The agents ✦ can ask (they answer once, without tools, from a prompt).
pub fn can_write(agent: AgentKind) -> bool {
    matches!(agent, AgentKind::Claude | AgentKind::Codex)
}

/// Asks `agent` (its program found on `path`) to write a commit message from `diff`. Blocking.
pub fn write_message(agent: AgentKind, root: &Path, diff: &str, path: Option<&OsString>) -> Result<String, String> {
    let path = tc_git::ship::path_or_env(path);
    let program = match agent {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
        _ => return Err(t("This agent can’t write messages yet.").to_string()),
    };
    let bin = path
        .as_ref()
        .and_then(|p| tc_lsp::servers::resolve(program, p))
        .ok_or_else(|| tf("{} isn’t on your PATH.", &[program]))?;
    let mut cmd = std::process::Command::new(&bin);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
    cmd.current_dir(root);
    if let Some(p) = &path {
        cmd.env("PATH", p);
    }
    // Not a pane's agent: no hook of TernaryCode's reports it.
    for var in [tc_agent::ENV_SOCK, tc_agent::ENV_PANE, tc_agent::ENV_TOKEN] {
        cmd.env_remove(var);
    }
    let out_dir = crate::paths::data_dir();
    let _ = crate::paths::ensure_private_dir(out_dir);
    let out_file = out_dir.join(format!("tc-message-{}-{}.txt", std::process::id(), next_work()));
    match agent {
        AgentKind::Claude => {
            cmd.args(["-p", "--output-format", "text", "--no-session-persistence", "--tools", "", "--strict-mcp-config", WRITE_PROMPT]);
        }
        _ => {
            cmd.args(["exec", "--ephemeral", "--skip-git-repo-check", "--sandbox", "read-only", "--color", "never", "-o"]);
            cmd.arg(&out_file);
            cmd.arg(WRITE_PROMPT);
        }
    }
    let result = run_with_input(&mut cmd, diff.as_bytes(), Duration::from_secs(120));
    let text = match (agent, result) {
        (AgentKind::Codex, Ok(_)) => {
            let text = std::fs::read_to_string(&out_file).map_err(|e| e.to_string());
            let _ = std::fs::remove_file(&out_file);
            text?
        }
        (_, Ok(out)) => out,
        (_, Err(e)) => {
            let _ = std::fs::remove_file(&out_file);
            return Err(e);
        }
    };
    let message = clean_message(&text);
    if message.is_empty() {
        return Err(tf("{} answered nothing.", &[agent.label()]));
    }
    Ok(message)
}

/// An agent's answer as a message: no fences, no quotes around it, no blank lines piled up.
pub fn clean_message(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim_start().starts_with("```")).collect();
    let mut out = lines.join("\n").trim().trim_matches(['"', '\'', '`']).trim().to_string();
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    let (title, body) = tc_git::ship::split_message(&out);
    let title = title.trim_start_matches(['#', '*', ' ']).trim_end_matches('.').trim().to_string();
    if body.is_empty() { title } else { format!("{title}\n\n{body}") }
}

fn run_with_input(cmd: &mut std::process::Command, input: &[u8], timeout: Duration) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::process::Stdio;
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let input = input.to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = err.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = out_t.join().unwrap_or_default();
                let stderr = err_t.join().unwrap_or_default();
                return if status.success() { Ok(stdout) } else { Err(last_lines(&format!("{stdout}\n{stderr}"), 6)) };
            }
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(t("It took too long and was stopped.").to_string());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// The last `n` lines of `text` that say something.
pub fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Words a shell takes as one argument.
pub fn quote(s: &str) -> String {
    let safe = if cfg!(windows) { "-_./:+=" } else { "-_./:@+=,%" };
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || safe.contains(c)) {
        return s.to_string();
    }
    if cfg!(windows) {
        return format!("'{}'", s.replace('\'', "''"));
    }
    if !s.contains('\'') {
        return format!("'{s}'");
    }
    if s.contains('!') {
        return format!("'{}'", s.replace('\'', "'\\''"));
    }
    let mut out = String::from("\"");
    for c in s.chars() {
        if matches!(c, '\\' | '"' | '$' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Commands joined so each runs only if the one before succeeded (PowerShell 5 has no `&&`).
pub fn chain(cmds: &[String]) -> String {
    if cfg!(windows) {
        let mut out = cmds.first().cloned().unwrap_or_default();
        for c in &cmds[1.min(cmds.len())..] {
            out.push_str(&format!("; if ($?) {{ {c} }}"));
        }
        out
    } else {
        cmds.join(" && ")
    }
}

/// What a run asks for.
#[derive(Clone, Debug)]
pub struct RunReq {
    pub project: ProjectId,
    pub root: PathBuf,
    pub steps: Vec<StepKind>,
    /// Paths to commit (a renamed file's old path too).
    pub paths: Vec<String>,
    pub message: String,
    pub remote: Option<String>,
    pub branch: String,
    pub base: Option<String>,
    /// The branch tracks nothing on the remote yet.
    pub set_upstream: bool,
    pub draft: bool,
    pub web: Option<Web>,
    pub gh: Option<PathBuf>,
    pub glab: Option<PathBuf>,
    pub path: Option<OsString>,
}

impl RunReq {
    /// The same steps as shell commands, for a terminal (from `from`: the steps not done yet).
    /// Writes the commit message and PR body files into the git directory.
    pub fn commands(&self, from: usize) -> Vec<String> {
        self.build_commands(from, true)
    }

    /// Preview of the shell commands for rendering (no git subprocesses or disk writes).
    pub fn preview_commands(&self, from: usize) -> Vec<String> {
        self.build_commands(from, false)
    }

    fn build_commands(&self, from: usize, write: bool) -> Vec<String> {
        let mut out = Vec::new();
        let paths: Vec<String> = self.paths.iter().map(|p| quote(p)).collect();
        for step in self.steps.iter().skip(from) {
            match step {
                StepKind::Branch(name) => out.push(format!("git switch -c {}", quote(name))),
                StepKind::Commit => {
                    out.push(format!("git add -A -- {}", paths.join(" ")));
                    let file = if write {
                        message_file(&self.root, &self.message).map(|p| p.display().to_string()).unwrap_or_default()
                    } else {
                        ".git/TC_COMMIT_MSG".to_string()
                    };
                    out.push(format!("git commit -F {} -- {}", quote(&file), paths.join(" ")));
                }
                StepKind::Push => {
                    let remote = self.remote.clone().unwrap_or_else(|| "origin".into());
                    out.push(format!("git push{} {} {}", if self.set_upstream { " -u" } else { "" }, quote(&remote), quote(&self.branch)));
                }
                StepKind::PullRequest { gh: true } => {
                    let (title, desc) = tc_git::ship::split_message(&self.message);
                    if self.web.as_ref().is_some_and(|w| w.forge == Forge::GitLab) {
                        let repo = self.web.as_ref().map(|w| w.glab_repo()).unwrap_or_default();
                        out.push(format!(
                            "glab mr create -R {} --source-branch {} --target-branch {} --title {}{} --description {} --yes",
                            quote(&repo),
                            quote(&self.branch),
                            quote(self.base.as_deref().unwrap_or("main")),
                            quote(&title),
                            if self.draft { " --draft" } else { "" },
                            quote(&desc)
                        ));
                    } else {
                        let repo = self.web.as_ref().map(|w| w.gh_repo()).unwrap_or_default();
                        let body = if write {
                            body_file(&self.root, &self.message).map(|p| p.display().to_string()).unwrap_or_default()
                        } else {
                            ".git/TC_PR_BODY".to_string()
                        };
                        out.push(format!(
                            "gh pr create -R {} --base {} --head {} --title {}{} --body-file {}",
                            quote(&repo),
                            quote(self.base.as_deref().unwrap_or("main")),
                            quote(&self.branch),
                            quote(&title),
                            if self.draft { " --draft" } else { "" },
                            quote(&body)
                        ));
                    }
                }
                StepKind::PullRequest { gh: false } => {}
            }
        }
        out
    }
}

/// The message in a file inside the repository's git folder, for `git commit -F`.
fn message_file(root: &Path, message: &str) -> Option<PathBuf> {
    let path = tc_git::git_path(root, "TC_COMMIT_MSG").ok()?;
    std::fs::write(&path, message).ok()?;
    Some(path)
}

fn body_file(root: &Path, message: &str) -> Option<PathBuf> {
    let path = tc_git::git_path(root, "TC_PR_BODY").ok()?;
    std::fs::write(&path, tc_git::ship::split_message(message).1).ok()?;
    Some(path)
}

/// Results of ship work off the UI thread.
pub enum ShipMsg {
    Facts { project: ProjectId, load: u64, root: PathBuf, facts: Box<Facts>, gh: Option<PathBuf>, glab: Option<PathBuf>, pr: Option<Pr> },
    /// A step ended (or started: `Running`).
    Step { project: ProjectId, run: u64, index: usize, state: StepState, detail: String, took: Duration },
    /// The pull request made, or the form to open.
    Pr { project: ProjectId, run: u64, pr: Option<Pr>, form: Option<String> },
    Finished { project: ProjectId, run: u64 },
    Written { project: ProjectId, agent: AgentKind, result: Result<String, String> },
    /// A pull request's state read again (its checks).
    PrState { project: ProjectId, branch: String, pr: Option<Pr> },
}

/// Numbers runs and loads, so a late answer of an earlier one is told apart.
static WORK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn next_work() -> u64 {
    WORK.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// How often a pull request's checks are read again while they run.
pub const CHECKS_EVERY: Duration = Duration::from_secs(60);
/// How long a project's facts are trusted before the box reads them again by itself.
const FACTS_FRESH: Duration = Duration::from_secs(300);

impl App {
    /// ⇧⌘C: the commit box of the project in view, its message taking the keyboard. From the
    /// Grid it goes to the project's Focus first (the box is in the Code drawer).
    pub fn ship_open(&mut self) {
        let Some(project) = self.active else { return };
        if self.view == tc_core::model::View::Grid {
            self.actions.push(Action::SetView(tc_core::model::View::Focus));
            self.actions.push(Action::Commit);
            return;
        }
        let open_here = self.review.as_ref().is_some_and(|r| r.project == project);
        if !open_here {
            self.open_review(None);
        } else if let Some(e) = self.editors.get_mut(&project) {
            e.visible = true;
        }
        self.ship_focus = true;
        self.ship_load(project);
    }

    /// The project's commit box.
    pub fn ship_mut(&mut self, project: ProjectId) -> &mut Ship {
        self.ship.entry(project).or_default()
    }

    /// Reads the branch's facts (and its pull request, with gh) again, off the UI thread.
    pub fn ship_load(&mut self, project: ProjectId) {
        let Some(p) = self.project(project) else { return };
        let folder = p.path.clone();
        let path = self.lsp.path_env.clone();
        let load = next_work();
        let s = self.ship_mut(project);
        s.loading = true;
        s.load = load;
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let root = tc_git::repo_root(&folder).unwrap_or(folder);
            let facts = tc_git::ship::facts(&root);
            let path = path.or_else(|| Some(tc_lsp::servers::user_path()));
            // The end-to-end tests give stand-in `gh` / `glab` binaries (no network reached).
            let gh_stand_in = std::env::var_os("TERNARYCODE_GH").filter(|_| cfg!(debug_assertions)).map(PathBuf::from);
            let gh = gh_stand_in.or_else(|| path.as_ref().and_then(|p| tc_lsp::servers::resolve("gh", p)));
            let glab_stand_in = std::env::var_os("TERNARYCODE_GLAB").filter(|_| cfg!(debug_assertions)).map(PathBuf::from);
            let glab = glab_stand_in.or_else(|| path.as_ref().and_then(|p| tc_lsp::servers::resolve("glab", p)));
            let web = facts.remote.as_ref().and_then(|r| r.web.as_ref());
            let pr = match (cli_for(web, &gh, &glab), web, &facts.branch) {
                (Some(cli), Some(web), Some(branch))
                    if facts.upstream.is_some() && facts.default_branch.as_deref() != Some(branch.as_str()) =>
                {
                    tc_git::ship::find_request(cli, &root, web, branch, path.as_deref()).ok().flatten()
                }
                _ => None,
            };
            let _ = tx.send(AppMsg::Ship(ShipMsg::Facts { project, load, root, facts: Box::new(facts), gh, glab, pr }));
            ctx.request_repaint();
        });
    }

    /// The facts are read when the box shows and they are missing or old.
    pub fn ship_want_facts(&mut self, project: ProjectId) {
        let s = self.ship_mut(project);
        if s.loading || s.run.as_ref().is_some_and(|r| !r.finished) {
            return;
        }
        if s.facts.is_none() || s.loaded_at.is_none_or(|t| t.elapsed() > FACTS_FRESH) {
            self.ship_load(project);
        }
    }

    pub fn on_ship_msg(&mut self, msg: ShipMsg) {
        match msg {
            ShipMsg::Facts { project, load, root, facts, gh, glab, pr } => {
                let s = self.ship_mut(project);
                if s.load != load {
                    return;
                }
                s.loading = false;
                s.loaded_at = Some(Instant::now());
                s.root = Some(root);
                s.facts = Some(*facts);
                s.gh = gh;
                s.glab = glab;
                let pending = pr.as_ref().is_some_and(|pr| pr.checks == Checks::Pending);
                s.pr = pr;
                // Checks running: read again a minute from now (not at once).
                if pending {
                    self.ship_checks_at.insert(project, Instant::now());
                }
                // Refresh open PRs/MRs in Commit History if it is open for this project.
                if self.history.as_ref().is_some_and(|h| h.project == project && h.open_prs.is_empty() && !h.prs_loading) {
                    self.history_load_open_prs();
                }
            }
            ShipMsg::Step { project, run, index, state, detail, took } => {
                let s = self.ship_mut(project);
                if let Some(r) = s.run.as_mut().filter(|r| r.id == run)
                    && let Some(step) = r.steps.get_mut(index)
                {
                    step.state = state;
                    if !detail.is_empty() {
                        step.detail = detail;
                    }
                    if took > Duration::ZERO {
                        step.took = Some(took);
                    }
                }
            }
            ShipMsg::Pr { project, run, pr, form } => {
                if let Some(url) = &form {
                    self.actions.push(Action::OpenWeb(url.clone()));
                }
                let s = self.ship_mut(project);
                if let Some(r) = s.run.as_mut().filter(|r| r.id == run) {
                    r.pr = pr.clone();
                    r.form = form;
                }
                if pr.is_some() {
                    s.pr = pr;
                }
            }
            ShipMsg::Finished { project, run } => {
                let mut ok = false;
                let mut title = String::new();
                {
                    let s = self.ship_mut(project);
                    if let Some(r) = s.run.as_mut().filter(|r| r.id == run) {
                        r.finished = true;
                        ok = r.failed().is_none();
                        let gitlab = r.req.web.as_ref().is_some_and(|w| w.forge == Forge::GitLab);
                        title = match (&r.pr, &r.form) {
                            (Some(pr), _) if gitlab => tf("MR !{} opened", &[&pr.number.to_string()]),
                            (Some(pr), _) => tf("PR #{} opened", &[&pr.number.to_string()]),
                            (None, Some(_)) if gitlab => t("The merge request form is open").to_string(),
                            (None, Some(_)) => t("The pull request form is open").to_string(),
                            _ if r.steps.iter().any(|s| s.kind == StepKind::Push) => t("Pushed").to_string(),
                            _ => t("Committed").to_string(),
                        };
                    }
                    if ok {
                        // The message went into the commit: the next one starts again.
                        s.message.clear();
                        s.auto = false;
                        s.auto_from = None;
                        s.undo = None;
                        s.written_by = None;
                        s.new_branch = None;
                        s.out.clear();
                    }
                }
                // What changed: the file list, the branch, the counts.
                self.ship_load(project);
                if self.review.as_ref().is_some_and(|r| r.project == project) {
                    let base = self.review.as_ref().map(|r| r.base);
                    self.open_review(base);
                }
                if self.history.as_ref().is_some_and(|h| h.project == project) {
                    self.history_reload(false);
                }
                self.refresh_git(project);
                // Told when not looking at it.
                let looking = self.window_focused && self.active == Some(project) && self.review.as_ref().is_some_and(|r| r.project == project);
                if ok && !looking {
                    let name = self.project(project).map(|p| p.name.clone()).unwrap_or_default();
                    self.toast_with(crate::app::ToastKind::Ok, format!("{name} · {title}"), Some((t("Show").to_string(), Action::SelectProject(project))));
                }
            }
            ShipMsg::Written { project, agent, result } => {
                let s = self.ship_mut(project);
                s.writing = None;
                match result {
                    Ok(message) => {
                        s.undo = Some(std::mem::take(&mut s.message));
                        s.message = message;
                        s.auto = false;
                        s.written_by = Some(agent);
                        s.write_error = None;
                    }
                    Err(e) => s.write_error = Some(e),
                }
            }
            ShipMsg::PrState { project, branch, pr } => {
                let s = self.ship_mut(project);
                if s.facts.as_ref().and_then(|f| f.branch.as_deref()) == Some(branch.as_str()) {
                    s.pr = pr;
                }
            }
        }
    }

    /// Runs what the button does now: its steps one after the other, off the UI thread.
    pub fn ship_run(&mut self, project: ProjectId, req: RunReq) {
        let id = next_work();
        let (title, _) = tc_git::ship::split_message(&req.message);
        let steps = req.steps.iter().map(|k| Step { kind: k.clone(), state: StepState::Todo, detail: String::new(), took: None }).collect();
        self.ship_mut(project).run = Some(Run { id, title, steps, pr: None, form: None, finished: false, req: req.clone() });
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let send = |m: ShipMsg| {
                let _ = tx.send(AppMsg::Ship(m));
                ctx.request_repaint();
            };
            for (index, step) in req.steps.iter().enumerate() {
                send(ShipMsg::Step { project, run: id, index, state: StepState::Running, detail: String::new(), took: Duration::ZERO });
                let started = Instant::now();
                let result: Result<String, Failure> = match step {
                    StepKind::Branch(name) => tc_git::ship::create_branch(&req.root, name).map(|_| name.clone()).map_err(|e| Failure::Output(last_lines(&e, 8))),
                    StepKind::Commit => tc_git::ship::commit(&req.root, &req.paths, &req.message).map_err(|e| Failure::Output(last_lines(&e, 12))),
                    StepKind::Push => {
                        let remote = req.remote.clone().unwrap_or_else(|| "origin".into());
                        tc_git::ship::push(&req.root, &remote, &req.branch, req.set_upstream, req.path.as_deref())
                            .map(|_| format!("{remote}/{}", req.branch))
                            .map_err(|e| match e {
                                PushError::Rejected(o) => Failure::Rejected(last_lines(&o, 6)),
                                PushError::Credentials(o) => Failure::Credentials(last_lines(&o, 6)),
                                PushError::Failed(o) => Failure::Output(last_lines(&o, 8)),
                            })
                    }
                    StepKind::PullRequest { gh } => {
                        let (title, body) = tc_git::ship::split_message(&req.message);
                        let base = req.base.clone().unwrap_or_else(|| "main".into());
                        match (&req.web, gh, cli_for(req.web.as_ref(), &req.gh, &req.glab)) {
                            (Some(web), true, Some(bin)) => {
                                match tc_git::ship::create_request(bin, &req.root, web, &base, &req.branch, &title, &body, req.draft, req.path.as_deref()) {
                                    Ok(pr) => {
                                        let detail = if web.forge == Forge::GitLab {
                                            tf("!{} into {}", &[&pr.number.to_string(), &base])
                                        } else {
                                            tf("#{} into {}", &[&pr.number.to_string(), &base])
                                        };
                                        send(ShipMsg::Pr { project, run: id, pr: Some(pr), form: None });
                                        Ok(detail)
                                    }
                                    Err(GhError::SignedOut(o)) => Err(Failure::SignedOut(last_lines(&o, 4))),
                                    Err(GhError::Failed(o)) => Err(Failure::Output(last_lines(&o, 8))),
                                }
                            }
                            (Some(web), _, _) => {
                                let url = web.new_pr_url(&base, &req.branch, &title, &body);
                                send(ShipMsg::Pr { project, run: id, pr: None, form: Some(url) });
                                Ok(tf("form, into {}", &[&base]))
                            }
                            (None, ..) => Err(Failure::Output(t("The remote has no pull requests TernaryCode knows.").to_string())),
                        }
                    }
                };
                let took = started.elapsed();
                match result {
                    Ok(detail) => send(ShipMsg::Step { project, run: id, index, state: StepState::Done, detail, took }),
                    Err(f) => {
                        send(ShipMsg::Step { project, run: id, index, state: StepState::Failed(f), detail: String::new(), took });
                        break;
                    }
                }
            }
            send(ShipMsg::Finished { project, run: id });
        });
    }

    /// Asks the agent that did the work to write the message from the diff of the checked
    /// files (the last commit against the working tree `to`).
    pub fn ship_write(&mut self, project: ProjectId, agent: AgentKind, root: PathBuf, to: String, paths: Vec<String>) {
        let path = self.lsp.path_env.clone();
        let s = self.ship_mut(project);
        if s.writing.is_some() {
            return;
        }
        s.writing = Some(agent);
        s.write_error = None;
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let path = path.or_else(|| Some(tc_lsp::servers::user_path()));
            let from = match tc_git::head_tree(&root) {
                Some(tree) => Ok(tree),
                None => tc_git::empty_tree(&root),
            };
            let result = from
                .and_then(|from| tc_git::ship::diff_text(&root, &from, &to, &paths, 60_000))
                .and_then(|diff| write_message(agent, &root, &diff, path.as_ref()));
            let _ = tx.send(AppMsg::Ship(ShipMsg::Written { project, agent, result }));
            ctx.request_repaint();
        });
    }

    /// Each frame: the project in view's facts (its pull request for the top bar) when they
    /// are missing or old, and its checks again while they run (a wakeup when they are due).
    pub fn ship_tick(&mut self, ctx: &egui::Context) {
        let Some(project) = self.active else { return };
        if self.project(project).is_some_and(|p| p.git.is_some()) {
            self.ship_want_facts(project);
        }
        self.ship_poll_checks();
        let pending = self.ship.get(&project).and_then(|s| s.pr.as_ref()).is_some_and(|pr| pr.checks == Checks::Pending);
        if pending && let Some(at) = self.ship_checks_at.get(&project) {
            ctx.request_repaint_after(CHECKS_EVERY.saturating_sub(at.elapsed()) + Duration::from_millis(50));
        }
    }

    /// git says something changed in the project (a commit made in the terminal, a pull): the
    /// box's facts are read again when it shows, at most every few seconds.
    pub fn ship_git_changed(&mut self, project: ProjectId) {
        let shown = self.review.as_ref().is_some_and(|r| r.project == project);
        let Some(s) = self.ship.get(&project) else { return };
        if !shown || s.loading || s.run.as_ref().is_some_and(|r| !r.finished) || s.loaded_at.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
            return;
        }
        self.ship_load(project);
    }

    /// The pull/merge request's checks read again (while they run, the project in view).
    pub fn ship_poll_checks(&mut self) {
        let Some(project) = self.active else { return };
        let Some(s) = self.ship.get(&project) else { return };
        let (Some(pr), Some(facts), Some(root)) = (s.pr.clone(), s.facts.clone(), s.root.clone()) else { return };
        let web_opt = facts.remote.as_ref().and_then(|r| r.web.as_ref());
        let Some(cli) = cli_for(web_opt, &s.gh, &s.glab).cloned() else { return };
        if pr.checks != Checks::Pending || s.loading {
            return;
        }
        let due = self.ship_checks_at.get(&project).is_none_or(|t| t.elapsed() >= CHECKS_EVERY);
        if !due {
            return;
        }
        self.ship_checks_at.insert(project, Instant::now());
        let (Some(web), Some(branch)) = (facts.remote.and_then(|r| r.web), facts.branch) else { return };
        let path = self.lsp.path_env.clone();
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            if let Ok(pr) = tc_git::ship::find_request(&cli, &root, &web, &branch, path.as_deref()) {
                let _ = tx.send(AppMsg::Ship(ShipMsg::PrState { project, branch, pr }));
                ctx.request_repaint();
            }
        });
    }

    /// The commands of a run's remaining steps typed in a new tab of its project (Run in
    /// Terminal, and the ways on after a failure).
    pub fn ship_in_terminal(&mut self, ctx: &egui::Context, project: ProjectId, commands: Vec<String>) {
        if commands.is_empty() {
            return;
        }
        self.dispatch(ctx, Action::SelectProject(project));
        self.dispatch(ctx, Action::SetView(tc_core::model::View::Focus));
        self.dispatch(ctx, Action::NewTab);
        if let Some(pane) = self.current_pane()
            && let Some(p) = self.panes.get_mut(&pane)
        {
            p.type_command(chain(&commands));
        }
    }

    /// The agent that did the work, and its pane: the one Review shows, else the project's
    /// latest with a task.
    pub fn ship_agent_pane(&self, project: ProjectId) -> Option<(PaneId, AgentKind)> {
        let current = self.current_pane().filter(|id| self.panes.get(id).is_some_and(|p| p.project == project));
        let with_agent = |id: &PaneId| self.panes.get(id).and_then(|p| p.tracker.agent.or(p.last_agent).map(|a| (*id, a)));
        if let Some(found) = current.as_ref().and_then(with_agent) {
            return Some(found);
        }
        let mut panes: Vec<(&PaneId, &crate::pane::Pane)> = self.panes.iter().filter(|(_, p)| p.project == project && (p.task.is_some() || p.last_words.is_some())).collect();
        panes.sort_by_key(|(_, p)| std::cmp::Reverse(p.tracker.since));
        panes.first().and_then(|(id, _)| with_agent(id))
    }

    /// Fills the message from the agent's task and last words while the user has not written
    /// one of their own.
    pub fn ship_prefill(&mut self, project: ProjectId) {
        let source = self.ship_agent_pane(project).and_then(|(id, _)| self.panes.get(&id)).map(|p| (p.task.clone(), p.last_words.clone()));
        let s = self.ship_mut(project);
        if !(s.message.trim().is_empty() || s.auto) || s.writing.is_some() || s.written_by.is_some() {
            return;
        }
        let Some(source) = source.filter(|(a, b)| a.is_some() || b.is_some()) else { return };
        if s.auto_from.as_ref() == Some(&source) {
            return;
        }
        s.message = message_from(source.0.as_deref(), source.1.as_deref());
        s.auto = !s.message.is_empty();
        s.auto_from = Some(source);
    }

    /// Sends what stopped the commit to the agent that wrote the code, as its next prompt.
    pub fn ship_ask_fix(&mut self, project: ProjectId, output: &str) {
        let Some((pane, agent)) = self.ship_agent_pane(project) else { return };
        // The agent's prompt is in English whatever the app's language: so is the request.
        let text = format!("The commit was stopped by git:\n\n{output}\n\nPlease fix this so the commit can go through.");
        if let Some(p) = self.panes.get_mut(&pane)
            && p.alive()
        {
            let bytes = tc_term::widget::paste_bytes(&p.shared, &text);
            p.input(bytes);
            p.write(b"\r".to_vec());
            self.actions.push(Action::FocusPane(pane));
            self.toast(tf("Sent to {}.", &[agent.label()]));
        }
    }
}

/// Paths with changes, one map per project root, for Review's checkboxes: whether a file goes
/// into the commit.
pub fn checked(files: &[tc_git::FileChange], uncommitted: &HashSet<String>, out: &HashSet<String>) -> Vec<String> {
    let mut paths = Vec::new();
    for f in files {
        if uncommitted.contains(&f.path) && !out.contains(&f.path) {
            paths.push(f.path.clone());
            if let Some(old) = &f.old_path {
                paths.push(old.clone());
            }
        }
    }
    paths
}

/// When the checks were read last, per project (polling while they run).
pub type ChecksAt = HashMap<ProjectId, Instant>;

#[cfg(test)]
mod tests {
    use super::*;
    use tc_git::ship::{Forge, Remote};

    fn facts(branch: &str, upstream: bool, unpushed: usize, web: bool) -> Facts {
        Facts {
            branch: Some(branch.into()),
            has_commits: true,
            remote: Some(Remote {
                name: "origin".into(),
                url: "git@github.com:acme/app.git".into(),
                web: web.then(|| Web { forge: Forge::GitHub, host: "github.com".into(), repo: "acme/app".into() }),
            }),
            upstream: upstream.then(|| format!("origin/{branch}")),
            unpushed,
            behind: 0,
            default_branch: Some("main".into()),
            busy: None,
            conflicts: Vec::new(),
        }
    }

    fn inputs<'a>(f: &'a Facts, files: usize, pr: Option<&'a Pr>) -> Inputs<'a> {
        Inputs { facts: f, files, prefs: ShipPrefs::default(), new_branch: None, base: None, pr, gh: true }
    }

    #[test]
    fn a_feature_branch_commits_pushes_and_opens_a_pull_request() {
        let f = facts("feature/cart", true, 0, true);
        let p = plan(&inputs(&f, 3, None));
        assert_eq!(p.steps, vec![StepKind::Commit, StepKind::Push, StepKind::PullRequest { gh: true }]);
        assert_eq!(label(&p), "Commit, Push & Create PR");
        assert_eq!(p.base.as_deref(), Some("main"));
        // Without gh the form opens instead.
        let p = plan(&Inputs { gh: false, ..inputs(&f, 3, None) });
        assert_eq!(label(&p), "Commit, Push & Open PR Form");
    }

    #[test]
    fn on_main_it_pushes_to_main() {
        let f = facts("main", true, 0, true);
        let p = plan(&inputs(&f, 2, None));
        assert_eq!(p.steps, vec![StepKind::Commit, StepKind::Push]);
        assert_eq!(label(&p), "Commit & Push");
        assert!(p.can_branch);
        // Branch & PR: the branch first, then the pull request from it.
        let p = plan(&Inputs { new_branch: Some("fix-rounding"), ..inputs(&f, 2, None) });
        assert_eq!(p.steps, vec![StepKind::Branch("fix-rounding".into()), StepKind::Commit, StepKind::Push, StepKind::PullRequest { gh: true }]);
        assert_eq!(p.branch, "fix-rounding");
        assert!(!p.can_branch);
    }

    #[test]
    fn commits_made_by_the_agent_are_pushed() {
        let f = facts("tc/cart", false, 2, true);
        let p = plan(&Inputs { base: Some("main"), ..inputs(&f, 0, None) });
        assert_eq!(p.steps, vec![StepKind::Push, StepKind::PullRequest { gh: true }]);
        assert_eq!(label(&p), "Push 2 Commits & Create PR");
    }

    #[test]
    fn an_open_pull_request_is_updated() {
        let f = facts("feature/cart", true, 0, true);
        let pr = Pr { number: 42, url: "u".into(), state: PrState::Open, checks: Checks::Pending };
        let p = plan(&inputs(&f, 1, Some(&pr)));
        assert_eq!(p.steps, vec![StepKind::Commit, StepKind::Push]);
        assert_eq!(p.updates, Some(42));
        assert_eq!(label(&p), "Commit & Push");
        // Nothing new: nothing to do.
        assert!(plan(&inputs(&f, 0, Some(&pr))).idle());
        // A merged request with nothing new is not opened again.
        let merged = Pr { state: PrState::Merged, ..pr };
        assert!(plan(&inputs(&f, 0, Some(&merged))).idle());
    }

    #[test]
    fn choices_and_remotes() {
        let f = facts("feature/cart", true, 0, true);
        let commit = ShipPrefs { action: ShipAction::Commit, draft: false };
        assert_eq!(plan(&Inputs { prefs: commit, ..inputs(&f, 1, None) }).steps, vec![StepKind::Commit]);
        let push = ShipPrefs { action: ShipAction::CommitPush, draft: false };
        assert_eq!(label(&plan(&Inputs { prefs: push, ..inputs(&f, 1, None) })), "Commit & Push");
        // A remote with no pages: no pull request.
        let f = facts("feature/cart", true, 0, false);
        let p = plan(&inputs(&f, 1, None));
        assert_eq!(p.steps, vec![StepKind::Commit, StepKind::Push]);
        assert!(p.no_pages);
        // No remote at all: commit only.
        let mut f = facts("feature/cart", true, 0, true);
        f.remote = None;
        assert_eq!(label(&plan(&inputs(&f, 1, None))), "Commit");
        // Pushed, no request yet, nothing new: the request alone.
        let f = facts("feature/cart", true, 0, true);
        assert_eq!(label(&plan(&inputs(&f, 0, None))), "Create PR");
    }

    #[test]
    fn blocked_states() {
        let mut f = facts("main", true, 0, true);
        f.busy = Some(Busy::Merge);
        f.conflicts = vec!["src/Header.tsx".into(), "src/cart.ts".into()];
        let p = plan(&inputs(&f, 2, None));
        assert!(p.blocked.as_deref().unwrap().contains("Header.tsx, cart.ts"));
        assert!(p.steps.is_empty());
        let mut f = facts("main", true, 0, true);
        f.branch = None;
        assert!(plan(&inputs(&f, 1, None)).blocked.is_some());
    }

    #[test]
    fn titles_from_tasks() {
        assert_eq!(title_from_task("add a cart badge to the header, then check it in the browser"), "Add a cart badge to the header");
        assert_eq!(title_from_task("The checkout total is a cent off. Fix it and run the tests"), "The checkout total is a cent off");
        assert_eq!(title_from_task("- fix: the parser closes on the first >"), "Fix: the parser closes on the first >");
        let long = title_from_task(&"word ".repeat(30));
        assert!(long.chars().count() <= 72, "{long}");
        assert_eq!(message_from(Some("Fix the rounding"), Some("Round once, after the discount.")), "Fix the rounding\n\nRound once, after the discount.");
        assert_eq!(message_from(None, Some("Added idempotency keys. Repeated requests now get the first response.")), "Added idempotency keys\n\nRepeated requests now get the first response.");
        // A question is not a summary.
        assert_eq!(message_from(Some("Fix it"), Some("Should I also update the tests?")), "Fix it");
        assert_eq!(message_from(None, None), "");
        // Non-ASCII uppercase characters whose Unicode lowercase changes byte length (e.g. 'İ').
        assert_eq!(title_from_task("İzmir, İstanbul, İzmit: öö"), "İzmir, İstanbul, İzmit");
    }

    #[test]
    fn agents_answers_become_messages() {
        assert_eq!(clean_message("```\nFix the rounding.\n\n\n\nRound once.\n```"), "Fix the rounding\n\nRound once.");
        assert_eq!(clean_message("\"Add a badge\""), "Add a badge");
        assert_eq!(clean_message("# Add a badge\n\nBody"), "Add a badge\n\nBody");
    }

    #[test]
    fn shell_words() {
        assert_eq!(quote("src/a.ts"), "src/a.ts");
        if !cfg!(windows) {
            assert_eq!(quote("my file.ts"), "'my file.ts'");
            assert_eq!(quote("it's"), "\"it's\"");
            assert_eq!(quote("don't!"), "'don'\\''t!'");
            assert_eq!(chain(&["a".into(), "b".into()]), "a && b");
        } else {
            assert_eq!(quote("a,b"), "'a,b'");
            assert_eq!(quote("@args"), "'@args'");
        }
    }

    #[test]
    fn checked_files() {
        let f = |path: &str, old: Option<&str>| tc_git::FileChange {
            path: path.into(),
            old_path: old.map(String::from),
            kind: tc_git::ChangeKind::Modified,
            additions: Some(1),
            deletions: Some(0),
            binary: false,
        };
        let files = vec![f("a", None), f("b", Some("old-b")), f("c", None)];
        let uncommitted: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let out: HashSet<String> = ["a"].iter().map(|s| s.to_string()).collect();
        assert_eq!(checked(&files, &uncommitted, &out), vec!["b".to_string(), "old-b".to_string()]);
    }
}
