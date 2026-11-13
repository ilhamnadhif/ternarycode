//! One pane at run time: a terminal fed by a local shell, what runs in its foreground, and its
//! status tracker.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use eframe::egui;
use parking_lot::Mutex;
use tc_agent::protocol::HookMsg;
use tc_core::agent::{AgentKind, recognize};
use tc_core::status::{Event, Foreground, ShellMark, Tracker};
use tc_core::{PaneId, ProjectId};
use tc_term::core::{self as tcore, Feeder, GridSize, QueryPalette, SharedTerm};
use tc_term::osc::{OscSignal, PromptMark};
use tc_term::pty::{self, ExitInfo, LocalPty, Session, SessionEvent, SpawnOptions};
use tc_term::widget::{self, TermView};

use crate::i18n::tf;

/// Messages from runtime threads to the UI thread.
pub enum UiMsg {
    /// An event of a pane's session, and the start of its shell it came from
    /// ([`Pane::generation`]).
    Pane(PaneId, u64, SessionEvent),
    Hook(HookMsg),
}

/// What a pane needs to start its shell.
pub struct SpawnCtx<'a> {
    pub rt: &'a tokio::runtime::Handle,
    pub tx: &'a Sender<UiMsg>,
    pub ctx: &'a egui::Context,
    /// Variables every pane gets (hook flags, shell integration).
    pub env: &'a [(String, String)],
    /// The hook socket and this pane's token.
    pub hook: Option<(String, String)>,
    pub scrollback: usize,
    pub palette: QueryPalette,
    pub cursor: tc_term::prefs::CursorStyle,
    pub blink: bool,
    /// Commands at least this long get a "finished" notice.
    pub long_command: Duration,
}

/// How long after Enter the foreground is watched for the command to start.
const ENTER_WATCH: Duration = Duration::from_secs(2);
/// How long a pane's size must stay the same before the PTY is resized.
const RESIZE_SETTLE: Duration = Duration::from_millis(120);
/// The most a program may put on the clipboard at once (OSC 52), in bytes.
const MAX_COPY: usize = 4 * 1024 * 1024;
/// A program's title is cut to this many chars.
const MAX_TITLE: usize = 100;
/// How a pane was on screen in the last frame, which sets how soon its new output is drawn:
/// what is typed into shows at once, other panes share frames, panes out of view only have
/// their messages taken in now and then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Shown {
    Hidden = 0,
    /// A Grid tile not being typed into: a preview, ~30 frames a second is plenty.
    Preview = 1,
    /// In Focus, not being typed into: at most one frame per display refresh.
    Visible = 2,
    /// Has the keyboard: reads this close share one frame (typing echo stays immediate).
    Typing = 3,
}

impl Shown {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Preview,
            2 => Self::Visible,
            3 => Self::Typing,
            _ => Self::Hidden,
        }
    }

    /// How a pane just drawn is shown: `Typing` while it has the keyboard and a key went in
    /// lately (its echo must show at once), else `otherwise`.
    pub fn of(ui: &egui::Ui, pane: PaneId, view: &tc_term::widget::TermView, otherwise: Self) -> Self {
        let focused = ui.memory(|m| m.has_focus(tc_term::widget::pane_widget_id(pane)));
        if focused && view.since_input(ui.input(|i| i.time)) < 2.0 { Self::Typing } else { otherwise }
    }

    fn period(self) -> Duration {
        Duration::from_millis(match self {
            Self::Typing => 4,
            Self::Visible => 16,
            Self::Preview => 33,
            Self::Hidden => 500,
        })
    }

    /// How long until output that arrives now is drawn: the next tick of this kind's period,
    /// on a clock all panes share, so output from many panes lands in the same frames (four
    /// busy Grid tiles make ~30 frames a second, not one frame per line each prints).
    pub fn output_delay(self) -> Duration {
        static EPOCH: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
        let period = self.period().as_nanos();
        let since = EPOCH.elapsed().as_nanos();
        Duration::from_nanos((period - since % period) as u64)
    }
}
/// The same before the shell printed anything: it must know its size before it draws its first
/// prompt (still long enough to take the sizes of a window settling as one).
const EARLY_SETTLE: Duration = Duration::from_millis(40);

pub struct Pane {
    pub id: PaneId,
    pub project: ProjectId,
    pub shared: SharedTerm,
    pub feeder: Arc<Mutex<Feeder>>,
    pub session: Option<Session>,
    /// Counts the shells started here: events of an earlier one are not this terminal's.
    pub generation: u64,
    pub view: TermView,
    pub tracker: Tracker,
    /// Set by the user (rename).
    pub title: Option<String>,
    /// What the program in the foreground says it is doing (its OSC 0/2 title, cleaned up),
    /// shown under the label, never as it. Gone once the shell is back at its prompt.
    pub program_title: Option<String>,
    /// The terminal's title as last read, to notice a new one.
    seen_title: Option<String>,
    /// Last known working directory of the shell.
    pub cwd: PathBuf,
    /// The preset that started the agent in this pane (for resume and restore).
    pub preset: Option<String>,
    /// Typed into the shell once it has printed something.
    pending_command: Option<String>,
    got_output: bool,
    pub exit: Option<ExitInfo>,
    /// The shell's own process group (the PTY's child is `login` on macOS).
    shell_pgid: Option<i32>,
    last_pgid: Option<i32>,
    /// Foreground process group seen but not readable yet (between fork and exec): retries.
    fg_retries: u8,
    /// A foreground check is due at this time.
    pub fg_due: Option<Instant>,
    /// Offered after a restart: the agent that ran here and the command that resumes it.
    pub resume: Option<(AgentKind, String)>,
    /// The agent last seen in this pane (kept when it exits, for saving).
    pub last_agent: Option<AgentKind>,
    /// The session the agent here works in, as its hooks say (Run Agent goes here instead of
    /// starting the same session twice).
    pub agent_session: Option<String>,
    /// The hook socket and this pane's token, as given to its shell.
    pub hook_env: Option<(String, String)>,
    /// TernaryCode itself ended the shell (pane closed, app quitting): its exit is not a failure.
    closing: bool,
    /// When the current shell started.
    started: Instant,
    /// The user typed into the current shell (Ctrl+D included).
    typed: bool,
    /// When Enter was last pressed: the shell may take a moment to start the command.
    enter_at: Option<Instant>,
    /// How often the PTY was resized (Gate C: switching views must not resize).
    pub resizes: u32,
    /// How it was drawn in the last frame (a [`Shown`]; read by the reader thread).
    pub shown: Arc<std::sync::atomic::AtomicU8>,
    /// When output last arrived.
    pub last_output: Instant,
    /// Scrollback lines kept (lowered when all panes together exceed the budget).
    pub scrollback_limit: usize,
    /// The tree of the working copy when the current agent run started, when it started, and
    /// the agent (FR-E4).
    pub run_snapshot: Option<(String, Instant, AgentKind)>,
    /// A new agent run may start (no agent since the last snapshot).
    pub snapshot_armed: bool,
    pub snapshot_pending: bool,
    /// A new size for the PTY, sent once the pane stops changing size (FR-T5).
    pub pending_resize: Option<(GridSize, u16, u16, Instant)>,
    /// The grid the PTY has now (as started, or last resized to).
    pty_grid: Option<GridSize>,
    /// Progress a program reports (OSC 9;4): its state (1 normal, 2 error, 3 indeterminate,
    /// 4 paused) and percentage. Gone when it reports 0 or the shell is back at its prompt.
    pub progress: Option<(u8, Option<u8>)>,
    /// Why the shell could not start (shown over the pane, with Try Again).
    pub start_error: Option<String>,
}

impl Pane {
    /// A pane whose terminal starts at `size` (the grid it will be drawn at, so the shell does
    /// not start with one width and get another right away). Without `start` it sleeps: no
    /// shell until it is first shown or typed into (`dormant`).
    pub fn new(id: PaneId, project: ProjectId, cwd: PathBuf, size: GridSize, spawn: &SpawnCtx, start: bool) -> Self {
        let (shared, rx) = tcore::new_terminal(
            tcore::term_config(spawn.scrollback, spawn.cursor, spawn.blink),
            size,
            spawn.palette.clone(),
        );
        let feeder = Arc::new(Mutex::new(Feeder::new(shared.clone(), rx)));
        let mut pane = Self {
            id,
            project,
            shared,
            feeder,
            session: None,
            generation: 0,
            view: TermView::new(widget::pane_widget_id(id)),
            tracker: Tracker::new(Instant::now()),
            title: None,
            program_title: None,
            seen_title: None,
            cwd,
            preset: None,
            pending_command: None,
            got_output: false,
            exit: None,
            shell_pgid: None,
            last_pgid: None,
            fg_retries: 0,
            fg_due: None,
            resume: None,
            agent_session: None,
            last_agent: None,
            hook_env: None,
            closing: false,
            started: Instant::now(),
            typed: false,
            enter_at: None,
            resizes: 0,
            shown: Arc::new(std::sync::atomic::AtomicU8::new(Shown::Hidden as u8)),
            last_output: Instant::now(),
            scrollback_limit: spawn.scrollback,
            run_snapshot: None,
            snapshot_armed: true,
            snapshot_pending: false,
            pending_resize: None,
            pty_grid: None,
            progress: None,
            start_error: None,
        };
        if start {
            pane.start_shell(spawn);
        }
        pane
    }

    /// No shell has been started in this pane yet (restored, not shown since).
    pub fn dormant(&self) -> bool {
        self.session.is_none() && self.exit.is_none()
    }

    /// Starts (or restarts) the shell in this pane's terminal. Nothing while one runs: a second
    /// shell would orphan the first, both feeding this terminal.
    pub fn start_shell(&mut self, spawn: &SpawnCtx) {
        if self.alive() {
            return;
        }
        // A session that ended but was kept is let go first; whatever it still sends is dropped.
        if let Some(old) = self.session.take() {
            old.pty.hang_up(Duration::from_millis(1500));
        }
        self.generation += 1;
        self.start_error = None;
        let size = {
            let term = self.shared.term.lock();
            use tc_term::alacritty_terminal::grid::Dimensions;
            GridSize {
                cols: term.columns(),
                rows: term.screen_lines(),
            }
        };
        let mut env: HashMap<String, String> = spawn.env.iter().cloned().collect();
        self.hook_env = spawn.hook.clone();
        if let Some((sock, token)) = &spawn.hook {
            env.insert(tc_agent::ENV_SOCK.into(), sock.clone());
            env.insert(tc_agent::ENV_PANE.into(), self.id.to_string());
            env.insert(tc_agent::ENV_TOKEN.into(), token.clone());
        }
        let cwd = if self.cwd.is_dir() {
            self.cwd.clone()
        } else {
            crate::paths::home_dir()
        };
        let ws = *self.shared.window.lock();
        self.pty_grid = Some(size);
        let opts = SpawnOptions {
            program: login_shell(&env),
            cwd,
            env,
            size,
            cell_px: (ws.cell_width, ws.cell_height),
        };
        self.exit = None;
        self.closing = false;
        self.started = Instant::now();
        self.typed = false;
        self.got_output = false;
        self.shell_pgid = None;
        self.last_pgid = None;
        self.tracker = Tracker::new(Instant::now());
        self.tracker.long_command = spawn.long_command;
        self.program_title = None;
        match LocalPty::spawn(&opts) {
            Ok(pty) => {
                let tx = spawn.tx.clone();
                let ctx = spawn.ctx.clone();
                let (id, generation) = (self.id, self.generation);
                let shown = self.shown.clone();
                let on_event: Arc<dyn Fn(SessionEvent) + Send + Sync> = Arc::new(move |e| {
                    // Signals (agent status, notifications), the bell and the end of the session
                    // are handled at once. Plain output is drawn when the pane is on screen,
                    // reads a few ms apart sharing a frame; a pane out of view only needs its
                    // messages taken in now and then (its status, a command waiting for the
                    // prompt), not the whole window redrawn for every line it prints.
                    let (urgent, output) = match &e {
                        SessionEvent::Output(f) => (!f.signals.is_empty() || f.bell, f.repaint),
                        _ => (true, false),
                    };
                    let _ = tx.send(UiMsg::Pane(id, generation, e));
                    if urgent {
                        ctx.request_repaint();
                    } else if output {
                        let how = Shown::from_u8(shown.load(std::sync::atomic::Ordering::Relaxed));
                        ctx.request_repaint_after(how.output_delay());
                    }
                });
                match pty::start(spawn.rt, pty, self.feeder.clone(), on_event) {
                    Ok(s) => self.session = Some(s),
                    Err(e) => self.fail_to_start(&e.to_string()),
                }
            }
            Err(e) => self.fail_to_start(&e.to_string()),
        }
    }

    fn fail_to_start(&mut self, why: &str) {
        let msg = tf("[The shell could not start: {}]", &[why]);
        tcore::write_status(&mut self.feeder.lock(), &msg);
        self.exit = Some(ExitInfo::Unknown);
        self.start_error = Some(why.to_string());
        self.tracker
            .apply(Event::Exited(Some(127)), Instant::now());
    }

    /// The shell has printed something (it is up).
    pub fn ready(&self) -> bool {
        self.got_output
    }

    /// The shell ended normally (Ctrl+D, `exit`): the pane closes, as in other terminals. Kept,
    /// with its message: a shell killed by a signal, and one that died within 2 seconds
    /// without anything typed into it (a broken setup would otherwise start and die forever).
    pub fn ended_by_user(&self, now: Instant) -> bool {
        !self.closing
            && matches!(self.exit, Some(ExitInfo::Code(_) | ExitInfo::Unknown))
            && (self.typed || now.duration_since(self.started) >= Duration::from_secs(2))
    }

    /// Keys typed or text pasted by the user.
    pub fn input(&mut self, bytes: Vec<u8>) {
        self.typed = true;
        self.write(bytes);
    }

    pub fn alive(&self) -> bool {
        self.session.is_some() && self.exit.is_none()
    }

    pub fn write(&self, bytes: Vec<u8>) {
        if let Some(s) = &self.session {
            s.write(bytes);
        }
    }

    /// Types `command` and Enter into the shell, as the user would, once it is ready.
    pub fn type_command(&mut self, command: String) {
        if self.got_output {
            self.write(format!("{command}\r").into_bytes());
            self.schedule_fg_check(Instant::now() + Duration::from_millis(500));
        } else {
            self.pending_command = Some(command);
        }
    }

    /// The user pressed Enter in this pane.
    pub fn entered(&mut self, now: Instant) {
        self.enter_at = Some(now);
        self.tracker.apply(Event::Enter, now);
        self.schedule_fg_check(now + Duration::from_millis(150));
    }

    pub fn schedule_fg_check(&mut self, at: Instant) {
        self.fg_due = Some(self.fg_due.map_or(at, |d| d.min(at)));
    }

    /// Text a program put on the clipboard through the terminal (OSC 52) since the last look.
    /// Nothing empty, nor more than any copy needs: a runaway program could hand over
    /// gigabytes.
    pub fn take_copy(&self) -> Option<String> {
        let text = self.shared.notes.lock().clipboard.take()?;
        (!text.is_empty() && text.len() <= MAX_COPY).then_some(text)
    }

    /// Handles one event from the PTY. Returns whether the status changed.
    pub fn on_session_event(&mut self, event: SessionEvent, now: Instant) -> bool {
        match event {
            SessionEvent::Output(fed) => {
                self.last_output = now;
                let mut changed = false;
                if !self.got_output && (fed.repaint || !fed.reply.is_empty()) {
                    self.got_output = true;
                    if let Some(cmd) = self.pending_command.take() {
                        self.write(format!("{cmd}\r").into_bytes());
                        self.schedule_fg_check(now + Duration::from_millis(500));
                    }
                }
                // The shell's own titles come with its command mark (`preexec`), in the same read.
                let command_started = fed
                    .signals
                    .iter()
                    .any(|s| matches!(s, OscSignal::Prompt(PromptMark::CommandExecuted)));
                for signal in fed.signals {
                    changed |= self.on_signal(signal, now);
                }
                self.read_title(command_started);
                if fed.bell {
                    changed |= self.tracker.apply(Event::Bell, now);
                }
                changed
            }
            SessionEvent::Foreground => {
                // The reader saw another foreground process: find out what it is.
                self.schedule_fg_check(now);
                false
            }
            SessionEvent::Exited(info) => {
                self.exit = Some(info);
                self.session = None;
                self.program_title = None;
                let code = match info {
                    _ if self.closing => Some(0),
                    ExitInfo::Code(c) => Some(c),
                    ExitInfo::Signal(s) => Some(128 + s),
                    ExitInfo::Unknown => None,
                };
                let text = match info {
                    ExitInfo::Code(c) => tf(
                        "[Shell exited with code {}. Press Enter for a new shell.]",
                        &[&c.to_string()],
                    ),
                    ExitInfo::Signal(s) => tf(
                        "[Shell ended by signal {}. Press Enter for a new shell.]",
                        &[&s.to_string()],
                    ),
                    ExitInfo::Unknown => {
                        crate::i18n::t("[Shell ended. Press Enter for a new shell.]").to_string()
                    }
                };
                tcore::end_session(&mut self.feeder.lock(), &text);
                self.tracker.apply(Event::Exited(code), now)
            }
        }
    }

    fn on_signal(&mut self, signal: OscSignal, now: Instant) -> bool {
        match signal {
            OscSignal::Notify { title, body } => {
                let message = match (title, body.is_empty()) {
                    (Some(t), true) => Some(t),
                    (Some(t), false) => Some(format!("{t}: {body}")),
                    (None, false) => Some(body),
                    (None, true) => None,
                };
                self.tracker.apply(Event::Notify { message }, now)
            }
            OscSignal::Prompt(mark) => {
                // Back at the prompt: what the program said it was doing is over.
                if matches!(mark, PromptMark::PromptStart | PromptMark::CommandFinished(_)) {
                    self.program_title = None;
                }
                let mark = match mark {
                    PromptMark::CommandExecuted => ShellMark::CommandStart,
                    PromptMark::CommandFinished(code) => ShellMark::CommandFinished(code),
                    _ => return false,
                };
                self.tracker.apply(Event::Shell(mark), now)
            }
            OscSignal::Cwd(path) => {
                let p = PathBuf::from(path);
                if p.is_dir() {
                    self.cwd = p;
                }
                false
            }
            OscSignal::Progress { state, value } => {
                self.progress = (state != 0).then_some((state, value));
                false
            }
            OscSignal::PromptRedraw(_) => false,
        }
    }

    /// Takes in a title the terminal was given since the last look. Only a program's counts:
    /// shell themes title every prompt (`user@host: ~/dir`), which says nothing of a task.
    /// With shell integration the prompt marks tell (parsed in order with the title); without,
    /// the foreground process does.
    fn read_title(&mut self, command_started: bool) {
        let notes = self.shared.notes.lock();
        if notes.title == self.seen_title {
            return;
        }
        self.seen_title = notes.title.clone();
        drop(notes);
        let program = if self.tracker.shell_marks {
            !command_started && !self.shared.prompt.lock().at_prompt
        } else {
            matches!(self.tracker.fg, Foreground::Program { .. })
        };
        self.program_title = self.seen_title.as_deref().filter(|_| program).and_then(clean_title);
    }

    /// Looks at the PTY's foreground process group when a check is due.
    pub fn check_foreground(&mut self, now: Instant) -> bool {
        if self.fg_due.is_none_or(|d| d > now) {
            return false;
        }
        self.fg_due = None;
        let Some(session) = &self.session else {
            return false;
        };
        let pty_pid = session.pty.pid() as i32;
        let Some(pgid) = session.pty.foreground_pgid() else {
            return false;
        };
        if Some(pgid) == self.last_pgid {
            // A command was just entered: keep looking for a little while (shell hooks such
            // as `preexec` can delay its start, and a quiet command prints nothing).
            if self.enter_at.is_some_and(|t| now.duration_since(t) < ENTER_WATCH) {
                self.schedule_fg_check(now + Duration::from_millis(300));
            }
            return false;
        }
        self.enter_at = None;
        self.last_pgid = Some(pgid);
        // macOS starts the shell through `login`, as its child. Windows reports the shell's
        // newest child as the foreground, and that is a program.
        let is_shell = pgid == pty_pid
            || Some(pgid) == self.shell_pgid
            || (cfg!(unix)
                && self.shell_pgid.is_none()
                && tc_agent::proc::ppid(pgid) == Some(pty_pid));
        if std::env::var_os("TERNARYCODE_DEBUG_FG").is_some() {
            eprintln!(
                "fg pane {}: pgid {pgid} pty {pty_pid} shell {:?} ppid {:?} -> shell {is_shell}",
                self.id,
                self.shell_pgid,
                tc_agent::proc::ppid(pgid)
            );
        }
        let fg = if is_shell {
            self.fg_retries = 0;
            if pgid != pty_pid {
                self.shell_pgid = Some(pgid);
            }
            if let Some(dir) = tc_agent::proc::cwd(pgid) {
                self.cwd = dir;
            }
            Foreground::Shell
        } else {
            match tc_agent::proc::info(pgid) {
                Some(info) => {
                    self.fg_retries = 0;
                    let agent = recognize(&info.argv, info.exe.as_deref());
                    Foreground::Program {
                        name: info.name(),
                        agent,
                    }
                }
                // Caught between fork and exec (its arguments can't be read yet), or gone: look
                // again in a moment rather than keep a nameless program (or miss an agent).
                None if self.fg_retries < 5 => {
                    self.fg_retries += 1;
                    self.last_pgid = None;
                    self.schedule_fg_check(now + Duration::from_millis(200));
                    return false;
                }
                None => Foreground::Program {
                    name: "?".into(),
                    agent: None,
                },
            }
        };
        if std::env::var_os("TERNARYCODE_DEBUG_FG").is_some() {
            eprintln!("fg pane {}: {fg:?}", self.id);
        }
        if let Foreground::Program {
            agent: Some(a), ..
        } = &fg
        {
            self.last_agent = Some(*a);
            self.resume = None;
        }
        if fg == Foreground::Shell {
            self.program_title = None;
            self.progress = None;
        }
        if fg == Foreground::Shell && self.tracker.agent.is_some() {
            // The agent was quit: nothing to resume later.
            self.last_agent = None;
            self.preset = None;
            self.agent_session = None;
        }
        self.tracker.apply(Event::Foreground(fg), now)
    }

    /// Applies a hook message for this pane.
    pub fn on_hook(&mut self, msg: &HookMsg, now: Instant) -> bool {
        let Some(agent) = AgentKind::from_slug(&msg.agent) else {
            return false;
        };
        if let Some(cwd) = msg.cwd.as_deref().map(PathBuf::from).filter(|p| p.is_dir()) {
            self.cwd = cwd;
        }
        self.last_agent = Some(agent);
        self.resume = None;
        if msg.event == "SessionEnd" {
            self.agent_session = None;
        } else if let Some(session) = &msg.session {
            self.agent_session = Some(session.clone());
        }
        let Some(signal) = tc_agent::protocol::classify(agent, &msg.event, msg.kind.as_deref())
        else {
            return false;
        };
        self.tracker.apply(
            Event::Hook {
                agent,
                signal,
                message: msg.message.clone(),
            },
            now,
        )
    }

    /// The pane's label: the user's title, the foreground program, or the folder.
    pub fn label(&self) -> String {
        if let Some(t) = &self.title {
            return t.clone();
        }
        match &self.tracker.fg {
            Foreground::Program { agent: Some(a), .. } => a.label().to_string(),
            Foreground::Program { name, .. } => name.clone(),
            _ => match self.tracker.agent {
                Some(a) => a.label().to_string(),
                None => shell_name().to_string(),
            },
        }
    }

    /// Where the shell is, when that is not the project folder itself: `./src` inside the
    /// project, `~/elsewhere` outside it.
    pub fn place(&self, project_root: &Path) -> Option<String> {
        place(&self.cwd, project_root)
    }

    /// The program that quitting would interrupt (not the shell at its prompt).
    pub fn busy_with(&self) -> Option<String> {
        if !self.alive() {
            return None;
        }
        match &self.tracker.fg {
            Foreground::Program { agent: Some(a), .. } => Some(a.label().to_string()),
            Foreground::Program { name, .. } => Some(name.clone()),
            _ => None,
        }
    }

    /// Ends the shell (closing the pane or quitting).
    pub fn hang_up(&mut self) {
        self.closing = true;
        if let Some(s) = self.session.take() {
            s.pty.hang_up(Duration::from_millis(1500));
        }
    }

    /// Records a new size; the program hears about it (SIGWINCH) once the size has settled, so
    /// dragging a divider or the window edge doesn't make full-screen programs redraw on every
    /// frame (FR-T5).
    pub fn resize_pty(&mut self, size: GridSize, px_w: u32, px_h: u32) {
        let cell_w = (px_w / size.cols.max(1) as u32) as u16;
        let cell_h = (px_h / size.rows.max(1) as u32) as u16;
        // A shell still starting draws its first prompt for the size it read: tell it now, not
        // after the settle delay, or zsh's end-of-line mark wraps and a lone "%" stays above
        // the prompt.
        let now = Instant::now();
        let at = if self.got_output {
            now
        } else {
            now.checked_sub(RESIZE_SETTLE - EARLY_SETTLE).unwrap_or(now)
        };
        self.pending_resize = Some((size, cell_w, cell_h, at));
    }

    /// Sends a settled size to the PTY. Returns when to look again, if a size is still pending.
    pub fn apply_resize(&mut self, now: Instant) -> Option<Instant> {
        let (size, w, h, at) = self.pending_resize?;
        let due = at + RESIZE_SETTLE;
        if now < due {
            return Some(due);
        }
        self.pending_resize = None;
        // A prompt cleared for the resize is redrawn by the shell after SIGWINCH, or put back
        // when the size ended where it was (no SIGWINCH comes then).
        let same = self.pty_grid == Some(size);
        tcore::settle_prompt(&self.shared, same);
        // Same columns and rows: nothing to tell the program (a new pane started at its final
        // size). Only the pixel size would change, and macOS would still send SIGWINCH, which
        // makes shells redraw their prompt for nothing. The terminal answers pixel-size
        // queries itself.
        if same {
            return None;
        }
        self.pty_grid = Some(size);
        // A pane asleep has no PTY yet: its shell starts at the grid's size.
        if let Some(s) = &self.session {
            self.resizes += 1;
            s.pty.resize(size, w, h);
        }
        None
    }
}

/// The user's shell, by name (`zsh`): what a pane at its prompt is called.
pub fn shell_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        // Windows: `pwsh`, `powershell` or `cmd`, without `.exe`.
        #[cfg(windows)]
        let shell = Some(tc_term::pty::default_shell().trim_end_matches(".exe").to_string());
        #[cfg(not(windows))]
        let shell = std::env::var("SHELL").ok();
        shell
            .and_then(|s| Path::new(&s).file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "shell".into())
    })
}

/// A program's title as shown: control characters out, white space one space, at most
/// `MAX_TITLE` chars. None when nothing is left, or for a remote shell's prompt title (over
/// SSH the program is `ssh`): `user@host: ~/dir`, `[user@host dir]$`.
fn clean_title(raw: &str) -> Option<String> {
    // Bidi controls would turn the rest of the line around.
    let bidi = |c: char| matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
    let mut text = String::new();
    for c in raw.chars() {
        if c.is_whitespace() {
            if !text.is_empty() && !text.ends_with(' ') {
                text.push(' ');
            }
        } else if !c.is_control() && !bidi(c) {
            text.push(c);
        }
    }
    let text = text.trim_end();
    let first = text.trim_start_matches('[').split([':', ' ', ']']).next().unwrap_or("");
    let login = first.split_once('@').is_some_and(|(user, host)| {
        let name = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || "._-".contains(c));
        name(user) && name(host)
    });
    if text.is_empty() || login {
        return None;
    }
    Some(match text.char_indices().nth(MAX_TITLE) {
        Some((cut, _)) => format!("{}…", text[..cut].trim_end()),
        None => text.to_string(),
    })
}

fn place(cwd: &Path, root: &Path) -> Option<String> {
    if cwd == root {
        return None;
    }
    match cwd.strip_prefix(root) {
        Ok(rel) => Some(format!("./{}", rel.display())),
        Err(_) => Some(crate::paths::contract_tilde(cwd)),
    }
}

/// macOS: the user's shell through `login` (so it is a login session, as in Terminal), quiet:
/// no "Last login" or "You have mail" in every new pane. Same flags as the terminal library's
/// default, plus `-q`. Elsewhere, and when the user is unknown, the library's default.
#[cfg(target_os = "macos")]
fn login_shell(_env: &HashMap<String, String>) -> Option<(String, Vec<String>)> {
    let user = std::env::var("USER").ok().filter(|u| !u.is_empty())?;
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
    // A script for zsh: a path with spaces or quotes must stay one word.
    let quote = crate::platform::shell_quote;
    let exec = format!("exec -a {} {}", quote(&format!("-{}", shell_name())), quote(&shell));
    Some((
        "/usr/bin/login".into(),
        vec!["-qflp".into(), user, "/bin/zsh".into(), "-fc".into(), exec],
    ))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn login_shell(_env: &HashMap<String, String>) -> Option<(String, Vec<String>)> {
    None
}

/// Windows: the default shell (PowerShell), with the shell integration when the pane's
/// environment asks for it.
#[cfg(windows)]
fn login_shell(env: &HashMap<String, String>) -> Option<(String, Vec<String>)> {
    let shell = tc_term::pty::default_shell().to_string();
    let args = tc_agent::shell::windows_shell_args(&shell, env.contains_key(tc_agent::shell::ENV_PWSH));
    Some((shell, args))
}

#[cfg(test)]
mod place_tests {
    use super::*;

    #[test]
    fn panes_say_where_they_are_relative_to_the_project() {
        let root = Path::new("/w/app");
        assert_eq!(place(root, root), None);
        assert_eq!(place(Path::new("/w/app/src/ui"), root).as_deref(), Some("./src/ui"));
        assert_eq!(place(Path::new("/tmp"), root).as_deref(), Some("/tmp"));
        assert!(!shell_name().is_empty());
    }

    #[test]
    fn program_titles_are_cleaned_and_prompt_titles_dropped() {
        assert_eq!(clean_title("  Fix the\tlogin\n bug \x07").as_deref(), Some("Fix the login bug"));
        assert_eq!(clean_title("a\u{202e}b").as_deref(), Some("ab"));
        assert_eq!(clean_title(" \x1b "), None);
        let long = clean_title(&"é".repeat(150)).unwrap();
        assert_eq!(long.chars().count(), MAX_TITLE + 1);
        assert!(long.ends_with('…'));
        for prompt in ["ilham@mac: ~/dev", "root@db-1:/var/log", "[me@host src]$", "me@host"] {
            assert_eq!(clean_title(prompt), None, "{prompt}");
        }
        assert_eq!(clean_title("✳ Review the PR").as_deref(), Some("✳ Review the PR"));
        assert_eq!(clean_title("vim notes.md").as_deref(), Some("vim notes.md"));
        assert_eq!(clean_title("Mail to bob@example.com").as_deref(), Some("Mail to bob@example.com"));
    }
}
