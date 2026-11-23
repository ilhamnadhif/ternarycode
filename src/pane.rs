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
    /// An agent's browser tool call, answered through the reply (`browser_agent`).
    Call(tc_agent::protocol::CallMsg, tc_agent::server::Reply),
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

/// A command waiting to be typed into a shell that marks its prompt is typed after this long
/// without output even if no prompt mark came (its startup files undid the integration).
const PROMPT_MARK_WAIT: Duration = Duration::from_secs(45);
/// A command waiting to be typed into a shell that does not mark its prompt is typed once the
/// shell has been quiet this long.
const QUIET_BEFORE_TYPING: Duration = Duration::from_millis(300);
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

    /// When output that arrives now is drawn: the next tick of this kind's period, on a clock
    /// all panes share, so output from many panes lands in the same frames (four busy Grid
    /// tiles make ~30 frames a second, not one frame per line each prints). How long until it,
    /// and which tick it is (its time on that clock).
    pub fn output_tick(self) -> (Duration, u64) {
        static EPOCH: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
        let period = self.period().as_nanos();
        let since = EPOCH.elapsed().as_nanos();
        let tick = (since / period + 1) * period;
        (Duration::from_nanos((tick - since) as u64), tick as u64)
    }
}

/// What a pane's signals said last, so a repeat (an unchanged progress, directory or server)
/// does not ask for a frame of its own: it is drawn with the next output tick.
#[derive(Default)]
struct Said {
    progress: Option<(u8, Option<u8>)>,
    cwd: Option<String>,
    servers: Vec<String>,
}

impl Said {
    /// Whether this output needs a frame now: a notification, a command starting or ending,
    /// the prompt back, or a progress, directory or local server that changed.
    fn news(&mut self, f: &tc_term::core::Fed) -> bool {
        let mut news = false;
        for s in &f.signals {
            match s {
                OscSignal::Notify { .. } => news = true,
                OscSignal::Prompt(PromptMark::CommandExecuted | PromptMark::CommandFinished(_) | PromptMark::PromptStart) => {
                    // The prompt clears the pane's servers: the same address printed again is news.
                    self.servers.clear();
                    news = true;
                }
                OscSignal::Prompt(_) | OscSignal::PromptRedraw(_) => {}
                OscSignal::Cwd(path) => {
                    if self.cwd.as_ref() != Some(path) {
                        self.cwd = Some(path.clone());
                        news = true;
                    }
                }
                OscSignal::Progress { state, value } => {
                    if self.progress != Some((*state, *value)) {
                        self.progress = Some((*state, *value));
                        news = true;
                    }
                }
            }
        }
        for found in &f.servers {
            if !self.servers.contains(&found.url) {
                if self.servers.len() >= 8 {
                    self.servers.remove(0);
                }
                self.servers.push(found.url.clone());
                news = true;
            }
        }
        news
    }
}
/// The same before the shell printed anything: it must know its size before it draws its first
/// prompt (still long enough to take the sizes of a window settling as one).
const EARLY_SETTLE: Duration = Duration::from_millis(40);

/// A local server the program in a pane said it listens on (`http://localhost:5173/`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub url: String,
    pub port: u16,
    /// It answered the last probe (shown only then).
    pub alive: bool,
    /// Probes it did not answer since it last did (or since it was found).
    pub misses: u8,
}

/// Probes a server that never answered may miss before it is forgotten.
pub const SERVER_MISSES: u8 = 3;
/// How often listed servers are probed.
pub const SERVER_PROBE_EVERY: Duration = Duration::from_secs(5);

/// What a probe of `port` found, in a pane's `servers`; whether what is shown changed.
fn probed(servers: &mut Vec<Server>, port: u16, answers: bool) -> bool {
    let Some(i) = servers.iter().position(|s| s.port == port) else { return false };
    let s = &mut servers[i];
    let was = s.alive;
    if answers {
        s.alive = true;
        s.misses = 0;
    } else if was {
        servers.remove(i);
    } else {
        s.misses += 1;
        if s.misses >= SERVER_MISSES {
            servers.remove(i);
        }
    }
    was != answers
}

/// Files a run changed, and the lines it added and removed.
pub type DiffStat = (usize, u32, u32);

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
    /// Typed into the shell once it waits at its prompt (`type_pending`).
    pending_command: Option<String>,
    got_output: bool,
    /// The shell loads TernaryCode's integration, so it marks its prompt (OSC 133).
    expects_marks: bool,
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
    /// The task the user last gave the agent here (its prompt's first line) and the agent's
    /// last words when it finished that turn: a commit message is made from them. In memory
    /// only.
    pub task: Option<String>,
    pub last_words: Option<String>,
    /// Local servers its program named, at most three, the ones that answer shown.
    pub servers: Vec<Server>,
    /// The next probe of `servers` is due.
    pub servers_probe_at: Option<Instant>,
    /// The hook socket and this pane's token, as given to its shell.
    pub hook_env: Option<(String, String)>,
    /// Windows: the PATH its shell started with (a program installed since is not on it).
    pub spawn_path: Option<std::ffi::OsString>,
    /// Its shell was started by an earlier run of the app (handed over in an update): it lacks
    /// the variables this run's agent flags name (`$TC_CODEX_GUIDE`…), so no agent starts in it.
    pub adopted: bool,
    /// TernaryCode itself ended the shell (pane closed, app quitting): its exit is not a failure.
    closing: bool,
    /// When the current shell started.
    started: Instant,
    /// The user typed into the current shell (Ctrl+D included).
    typed: bool,
    /// When the user last pressed Enter in it (sent a prompt to its agent, say). Not merely typed:
    /// keys typed while an agent works are no new prompt.
    pub last_prompt: Option<Instant>,
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
    /// What the agent's run changed when it finished (files, lines added, removed), for the
    /// status it was worked out at (`tracker.since`); `None` inside while it is worked out.
    pub done_stats: Option<(Instant, Option<DiffStat>)>,
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
    /// Multi-account profile id bound to this pane (e.g. `claude1`, `chatgpt2`).
    pub active_profile: Option<String>,
    /// Active inline rate-limit / failover recovery banner for this pane.
    pub rate_limit: Option<crate::profiles::RateLimitBanner>,
    /// Last time this pane's screen tail was scanned for rate-limit output.
    pub last_limit_check: Instant,
    /// Dedup key of the last rate-limit line handled in this pane.
    pub last_limit_sig: Option<String>,
    /// User explicitly closed the rate-limit notice in this pane (suppresses forced header failover CTA).
    pub rate_limit_dismissed: bool,
    /// Rate-limit signatures already dismissed or superseded in this pane so stale scrollback never re-triggers them.
    pub dismissed_limit_sigs: Vec<String>,
    /// Suppresses rate-limit re-scanning immediately after failover until new output replaces old text.
    pub limit_ignore_until: Option<Instant>,
    /// Quick composer draft text for sending follow-up instructions directly to an agent pane.
    pub composer_text: String,
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
            expects_marks: false,
            exit: None,
            shell_pgid: None,
            last_pgid: None,
            fg_retries: 0,
            fg_due: None,
            resume: None,
            agent_session: None,
            task: None,
            last_words: None,
            servers: Vec::new(),
            servers_probe_at: None,
            last_agent: None,
            hook_env: None,
            spawn_path: None,
            adopted: false,
            closing: false,
            started: Instant::now(),
            typed: false,
            last_prompt: None,
            enter_at: None,
            resizes: 0,
            shown: Arc::new(std::sync::atomic::AtomicU8::new(Shown::Hidden as u8)),
            last_output: Instant::now(),
            scrollback_limit: spawn.scrollback,
            run_snapshot: None,
            done_stats: None,
            snapshot_armed: true,
            snapshot_pending: false,
            pending_resize: None,
            pty_grid: None,
            progress: None,
            start_error: None,
            active_profile: None,
            rate_limit: None,
            last_limit_check: Instant::now(),
            last_limit_sig: None,
            rate_limit_dismissed: false,
            dismissed_limit_sigs: Vec::new(),
            limit_ignore_until: None,
            composer_text: String::new(),
        };
        if start {
            pane.start_shell(spawn);
        }
        pane
    }

    /// Plain-text snapshot of the last `max_lines` visible lines of this terminal (trimmed), used
    /// for rate-limit detection, failover handoff context, and Mobile Companion live streaming.
    pub fn screen_tail(&self, max_lines: usize) -> String {
        use tc_term::alacritty_terminal::grid::Dimensions;
        use tc_term::alacritty_terminal::index::{Column, Line};
        let term = self.shared.term.lock();
        let rows = term.screen_lines();
        let cols = term.columns();
        let mut lines: Vec<String> = (0..rows as i32)
            .map(|l| {
                let row = &term.grid()[Line(l)];
                (0..cols)
                    .map(|c| row[Column(c)].c)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        if lines.len() > max_lines {
            lines.drain(..lines.len() - max_lines);
        }
        lines.join("\n")
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
        // Windows: the PATH as it is now (the one in `spawn.env` is from the app's start).
        #[cfg(windows)]
        if let Some(path) = crate::cli::pane_path() {
            env.insert("PATH".into(), path.to_string_lossy().into_owned());
            self.spawn_path = Some(path);
        }
        self.hook_env = spawn.hook.clone();
        self.adopted = false;
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
        self.expects_marks = shell_integrated(&env);
        let ws = *self.shared.window.lock();
        self.pty_grid = Some(size);
        let opts = SpawnOptions {
            program: login_shell(&env),
            cwd,
            env,
            size,
            cell_px: (ws.cell_width, ws.cell_height),
        };
        let was_dormant = self.dormant();
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
        self.progress = None;
        self.servers.clear();
        self.servers_probe_at = None;
        if !was_dormant {
            self.active_profile = None;
            self.task = None;
            self.last_words = None;
        }
        self.rate_limit = None;
        self.last_limit_sig = None;
        self.limit_ignore_until = None;
        match LocalPty::spawn(&opts) {
            Ok(pty) => self.attach(pty, spawn),
            Err(e) => self.fail_to_start(&e.to_string()),
        }
    }

    /// Starts reading `pty` into this pane's terminal: its output is parsed there, its events
    /// come to the app as `UiMsg::Pane`.
    fn attach(&mut self, pty: Arc<LocalPty>, spawn: &SpawnCtx) {
        let tx = spawn.tx.clone();
        let ctx = spawn.ctx.clone();
        let (id, generation) = (self.id, self.generation);
        let shown = self.shown.clone();
        let said = std::sync::Mutex::new(Said::default());
        // The tick (and the frame count) last asked for: reads before that tick would
        // each ask for the same frame again, and wake the window every time. Any frame
        // drawn after a read draws it (its bytes are in the terminal by then).
        let asked = std::sync::Mutex::new((u64::MAX, u64::MAX));
        let on_event: Arc<dyn Fn(SessionEvent) + Send + Sync> = Arc::new(move |e| {
            // What changes the pane's status or calls for the user (a notification, a
            // command starting or ending, the bell, the end of the session) is handled
            // at once. Plain output, a program starting, and a progress, directory or
            // server said again are drawn on the pane's output tick: reads a few ms
            // apart share a frame, and a pane out of view only needs its messages taken
            // in now and then, not the whole window redrawn for every line it prints.
            let (urgent, output) = match &e {
                SessionEvent::Output(f) => {
                    let news = said.lock().unwrap_or_else(|p| p.into_inner()).news(f);
                    (f.bell || news, f.repaint || !f.signals.is_empty() || !f.servers.is_empty())
                }
                SessionEvent::Foreground => (false, true),
                _ => (true, false),
            };
            let _ = tx.send(UiMsg::Pane(id, generation, e));
            if urgent {
                ctx.request_repaint();
            } else if output {
                let how = Shown::from_u8(shown.load(std::sync::atomic::Ordering::Relaxed));
                let (mut delay, mut tick) = how.output_tick();
                let pass = ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT);
                let mut asked = asked.lock().unwrap_or_else(|p| p.into_inner());
                let (asked_tick, asked_pass) = *asked;
                if asked_pass == pass && asked_tick >= tick {
                    // That frame (or a later one) is asked for, and none drawn since.
                    return;
                }
                if asked_pass != pass && asked_tick == tick && how != Shown::Typing {
                    // The frame asked for this tick is drawn already: egui starts a
                    // frame its expected length before the time asked, so it came a
                    // few ms early. What came after it waits for the next tick, not a
                    // second frame right after (typing's echo is never held back).
                    let period = how.period();
                    tick += period.as_nanos() as u64;
                    delay += period;
                }
                *asked = (tick, pass);
                ctx.request_repaint_after(delay);
            }
        });
        match pty::start(spawn.rt, pty, self.feeder.clone(), on_event) {
            Ok(s) => self.session = Some(s),
            Err(e) => self.fail_to_start(&e.to_string()),
        }
    }

    /// Stops reading this pane's terminal, for the restart in place (`crate::handoff`): what
    /// its program prints from now on waits in the terminal for the new run.
    #[cfg(unix)]
    pub fn stop_reading(&self) {
        if let Some(session) = &self.session
            && self.exit.is_none()
        {
            session.stop_reading(Duration::from_millis(500));
        }
    }

    /// This pane as the new run takes it over: its terminal drawn into a file in `dir`, its fd
    /// kept open through `exec`, what it knows. `None` without a running program (the pane
    /// then comes back asleep, as after a normal launch).
    #[cfg(unix)]
    pub fn hand_over(&mut self, dir: &Path, now: Instant) -> Option<crate::handoff::Handed> {
        let session = self.session.as_ref()?;
        if self.exit.is_some() || !session.pty.is_alive() {
            return None;
        }
        let (bytes, extras, cols, rows) = {
            let mut feeder = self.feeder.lock();
            let bytes = tc_term::snapshot::snapshot(&mut feeder);
            let extras = tc_term::snapshot::extras(&self.shared);
            use tc_term::alacritty_terminal::grid::Dimensions;
            let term = self.shared.term.lock();
            (bytes, extras, term.columns(), term.screen_lines())
        };
        let screen = dir.join(format!("{}.screen", self.id));
        crate::handoff::write_screen(&screen, &bytes).ok()?;
        let (master, pid) = session.pty.prepare_handoff()?;
        Some(crate::handoff::Handed {
            id: self.id,
            master,
            pid,
            cols,
            rows,
            screen,
            extras,
            tracker: self.tracker.save(now),
            hook_env: self.hook_env.clone(),
            title: self.title.clone(),
            program_title: self.program_title.clone(),
            cwd: self.cwd.clone(),
            preset: self.preset.clone(),
            active_profile: self.active_profile.clone(),
            agent_session: self.agent_session.clone(),
            last_agent: self.last_agent,
            progress: self.progress,
            shell_pgid: self.shell_pgid,
            typed: self.typed,
            run_snapshot: self.run_snapshot.as_ref().map(|(tree, at, agent)| (tree.clone(), crate::handoff::ago(now, *at), *agent)),
            snapshot_armed: self.snapshot_armed,
        })
    }

    /// Takes over a pane the previous run handed over: its program keeps running in this
    /// pane, its terminal shows what it showed. False when the program is not there any more
    /// (the pane then stays asleep).
    #[cfg(unix)]
    pub fn adopt(&mut self, h: crate::handoff::Handed, spawn: &SpawnCtx) -> bool {
        let screen = std::fs::read(&h.screen);
        let _ = std::fs::remove_file(&h.screen);
        let Ok(pty) = LocalPty::adopt(h.master, h.pid) else {
            return false;
        };
        let now = Instant::now();
        // A terminal of the size the screen was drawn at; the layout resizes it after.
        let size = GridSize { cols: h.cols, rows: h.rows };
        let (shared, rx) = tcore::new_terminal(
            tcore::term_config(spawn.scrollback, spawn.cursor, spawn.blink),
            size,
            spawn.palette.clone(),
        );
        let mut feeder = Feeder::new(shared.clone(), rx);
        self.seen_title = h.extras.title.clone();
        let found = match screen {
            Ok(bytes) => tc_term::snapshot::restore(&mut feeder, &bytes, h.extras).servers,
            Err(_) => Vec::new(),
        };
        self.shared = shared;
        self.feeder = Arc::new(Mutex::new(feeder));
        self.generation += 1;
        self.start_error = None;
        self.hook_env = h.hook_env;
        self.adopted = true;
        self.pty_grid = Some(size);
        self.exit = None;
        self.closing = false;
        self.started = now;
        self.typed = h.typed;
        self.got_output = true;
        self.shell_pgid = h.shell_pgid;
        self.last_pgid = None;
        self.tracker = Tracker::restore(h.tracker, now);
        self.tracker.long_command = spawn.long_command;
        self.title = h.title;
        self.program_title = h.program_title;
        self.cwd = h.cwd;
        self.preset = h.preset;
        self.active_profile = h.active_profile;
        self.agent_session = h.agent_session;
        self.last_agent = h.last_agent;
        self.resume = None;
        self.progress = h.progress;
        self.run_snapshot = h.run_snapshot.map(|(tree, ms, agent)| (tree, crate::handoff::back(now, ms), agent));
        self.snapshot_armed = h.snapshot_armed;
        for f in found {
            self.found_server(f, now);
        }
        self.attach(pty, spawn);
        // What runs in the foreground now (it may have changed while the app restarted).
        self.schedule_fg_check(now);
        true
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

    /// A shell nobody has used yet (nothing typed, no program or agent run, nothing waiting to
    /// run): a project just opened. Run Agent starts the agent in it rather than in a second tab
    /// beside it (not in a shell an earlier run started: see `adopted`).
    pub fn is_fresh(&self) -> bool {
        !self.adopted
            && !self.typed
            && self.preset.is_none()
            && self.exit.is_none()
            && self.pending_command.is_none()
            && self.last_agent.is_none()
            && self.resume.is_none()
            && self.tracker.agent.is_none()
            && !matches!(self.tracker.fg, Foreground::Program { .. })
    }

    /// A shell an agent can start in instead of a tab beside it: one nobody has used yet, or one
    /// waiting at its prompt with nothing typed (what it showed before stays above).
    pub fn takes_agent(&self) -> bool {
        self.is_fresh()
            || (self.alive()
                && !self.adopted
                && self.pending_command.is_none()
                && self.resume.is_none()
                && self.tracker.agent.is_none()
                && self.tracker.shell_marks
                && !matches!(self.tracker.fg, Foreground::Program { .. })
                && tcore::command_line_empty(&self.shared))
    }

    /// Keys typed or text pasted by the user.
    pub fn input(&mut self, bytes: Vec<u8>) {
        self.typed = true;
        self.write(bytes);
    }

    pub fn alive(&self) -> bool {
        self.session.is_some() && self.exit.is_none()
    }

    /// A local server named in the output: listed (once per port) and probed at once.
    fn found_server(&mut self, found: tc_term::servers::Found, now: Instant) {
        if self.servers.iter().any(|s| s.port == found.port) || self.servers.len() >= 3 {
            return;
        }
        self.servers.push(Server { url: found.url, port: found.port, alive: false, misses: 0 });
        self.servers_probe_at = Some(now);
    }

    /// What a probe of `port` found. A server that stops answering is dropped; one that never
    /// answered is given a few probes to start.
    pub fn probed(&mut self, port: u16, answers: bool) -> bool {
        let changed = probed(&mut self.servers, port, answers);
        if self.servers.is_empty() {
            self.servers_probe_at = None;
        }
        changed
    }

    /// The servers that answer.
    pub fn live_servers(&self) -> impl Iterator<Item = &Server> {
        self.servers.iter().filter(|s| s.alive)
    }

    /// The agent in the foreground, when it reads an image from the clipboard on Ctrl+V
    /// (Claude Code, Codex, OpenCode, Antigravity CLI, Gemini CLI).
    pub fn reads_clipboard_images(&self) -> bool {
        matches!(
            &self.tracker.fg,
            Foreground::Program {
                agent: Some(AgentKind::Claude | AgentKind::Codex | AgentKind::Gemini | AgentKind::OpenCode | AgentKind::Antigravity),
                ..
            }
        )
    }

    /// The paste shortcut found no text on the clipboard: an agent gets Ctrl+V (`bytes`) to
    /// take the image from it.
    pub fn paste_without_text(&mut self, bytes: Vec<u8>) {
        if self.reads_clipboard_images() {
            self.input(bytes);
        }
    }

    pub fn write(&self, bytes: Vec<u8>) {
        if let Some(s) = &self.session {
            s.write(bytes);
        }
    }

    /// Types `command` and Enter into the shell, as the user would, once it waits at its prompt
    /// (`type_pending`).
    pub fn type_command(&mut self, command: String) {
        // A command typed in for the user (an agent, a resume, a merge) is the pane in use.
        self.typed = true;
        self.pending_command = Some(command);
        self.type_pending(Instant::now());
    }

    /// Types the command waiting to be typed once the shell is ready for it, and says when to
    /// look again. A shell's startup files can ask something first (oh-my-zsh: "Would you like to
    /// update? [Y/n]", which takes the first key typed, so `claude` ran as `laude`): a shell that
    /// marks its prompt (TernaryCode's integration) gets the command at its prompt, after its
    /// startup files ran and whatever they asked was answered. Another shell gets it once it has
    /// printed something and gone quiet a moment. A shell with the integration that never marks
    /// its prompt (its startup files undo it) gets it after a long quiet.
    pub fn type_pending(&mut self, now: Instant) -> Option<Instant> {
        self.pending_command.as_ref()?;
        if !self.got_output || self.session.is_none() {
            return None;
        }
        let at_prompt = self.shared.prompt.lock().at_prompt;
        let in_program = matches!(self.tracker.fg, tc_core::status::Foreground::Program { .. });
        let quiet = if self.tracker.shell_marks || self.expects_marks { PROMPT_MARK_WAIT } else { QUIET_BEFORE_TYPING };
        let due = self.last_output + quiet;
        if !at_prompt && (in_program || now < due) {
            return Some(if in_program { now + Duration::from_millis(200) } else { due });
        }
        let command = self.pending_command.take()?;
        self.write(format!("{command}\r").into_bytes());
        self.schedule_fg_check(now + Duration::from_millis(500));
        None
    }

    /// The user pressed Enter in this pane.
    pub fn entered(&mut self, now: Instant) {
        self.enter_at = Some(now);
        self.last_prompt = Some(now);
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
                }
                // The shell's own titles come with its command mark (`preexec`), in the same read.
                let command_started = fed
                    .signals
                    .iter()
                    .any(|s| matches!(s, OscSignal::Prompt(PromptMark::CommandExecuted)));
                for signal in fed.signals {
                    changed |= self.on_signal(signal, now);
                }
                // The shell came to its prompt: a command waiting for it is typed now.
                self.type_pending(now);
                for found in fed.servers {
                    self.found_server(found, now);
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
                self.progress = None;
                self.servers.clear();
                self.servers_probe_at = None;
                self.active_profile = None;
                self.rate_limit = None;
                self.last_limit_sig = None;
                self.limit_ignore_until = None;
                self.task = None;
                self.last_words = None;
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
                // Back at the prompt: what the program said it was doing is over, and the
                // servers it started with it.
                if matches!(mark, PromptMark::PromptStart | PromptMark::CommandFinished(_)) {
                    self.program_title = None;
                    self.servers.clear();
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
        // macOS starts the shell through `login`, as its child (on Linux the shell is the pty's
        // own process, and a child of it is a program the shell started). Windows reports the
        // shell's newest child as the foreground, and that is a program.
        let is_shell = pgid == pty_pid
            || Some(pgid) == self.shell_pgid
            || (cfg!(target_os = "macos")
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
            self.servers.clear();
        }
        if fg == Foreground::Shell
            && self.tracker.agent.is_some()
            && self.pending_command.is_none()
            && self.limit_ignore_until.is_none_or(|u| now >= u)
        {
            // The agent was quit (and no failover relaunch is in flight): clear agent & profile state.
            self.last_agent = None;
            self.preset = None;
            self.agent_session = None;
            self.active_profile = None;
            self.rate_limit = None;
            self.last_limit_sig = None;
            self.rate_limit_dismissed = false;
            self.dismissed_limit_sigs.clear();
            self.task = None;
            self.last_words = None;
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
        // A new task: the last words were about the one before.
        if let Some(prompt) = &msg.prompt {
            self.task = Some(prompt.clone());
            self.last_words = None;
        }
        let Some(signal) = tc_agent::protocol::classify(agent, &msg.event, msg.kind.as_deref())
        else {
            return false;
        };
        if signal == tc_core::status::HookSignal::TurnDone
            && let Some(words) = &msg.message
        {
            self.last_words = Some(words.clone());
        }
        // Once the agent actively works or finishes a turn, any prior manual rate-limit notice is
        // stale (e.g. user switched models or quota reset): clear it and remember its signature.
        if matches!(signal, tc_core::status::HookSignal::Working | tc_core::status::HookSignal::TurnDone)
            && self.rate_limit.as_ref().is_some_and(|b| b.auto_switched_to.is_none())
            && let Some(banner) = self.rate_limit.take()
        {
            self.remember_dismissed_limit(banner.reason);
        }
        self.tracker.apply(
            Event::Hook {
                agent,
                signal,
                message: msg.message.clone(),
            },
            now,
        )
    }

    /// Remembers a rate-limit error line that the user dismissed (or worked past) so stale
    /// scrollback lines do not re-trigger the same notification.
    pub fn remember_dismissed_limit(&mut self, sig: String) {
        if sig.is_empty() || self.dismissed_limit_sigs.iter().any(|s| s == &sig) {
            return;
        }
        if self.dismissed_limit_sigs.len() >= 8 {
            self.dismissed_limit_sigs.remove(0);
        }
        self.dismissed_limit_sigs.push(sig);
    }

    /// The pane's label: the user's title, the foreground program, or the folder.
    /// Its agent finished its turn (and said so) and nothing was typed here since: the pane
    /// offers Review and Commit. (A turn finished in view reads as Idle, not Done.)
    pub fn turn_finished(&self) -> bool {
        self.alive()
            && self.tracker.agent.is_some()
            && matches!(self.tracker.status, tc_core::status::Status::Done | tc_core::status::Status::Idle)
            && (self.last_words.is_some() || self.done_stats.as_ref().is_some_and(|(at, _)| *at == self.tracker.since))
            && self.enter_at.is_none_or(|e| e < self.tracker.since)
    }

    /// What a pane is called in its tab, its header and the sidebar: the name the user gave
    /// it, else the task its agent works on (its prompt's first line), else [`Pane::label`].
    pub fn name(&self) -> String {
        if let Some(t) = &self.title {
            return t.clone();
        }
        let agent_here = self.tracker.agent.is_some() || matches!(self.tracker.fg, Foreground::Program { agent: Some(_), .. });
        match &self.task {
            Some(task) if agent_here && !task.trim().is_empty() => task.clone(),
            _ => self.label(),
        }
    }

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

    /// The pane's program, taken to be ended with the others as the app quits
    /// (`pty::hang_up_all`).
    pub fn take_for_quit(&mut self) -> Option<Arc<tc_term::pty::LocalPty>> {
        self.closing = true;
        self.session.take().map(|s| s.pty)
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

/// Whether a shell started with `env` loads TernaryCode's integration (`tc_agent::shell::env`),
/// so it marks its prompt.
fn shell_integrated(env: &HashMap<String, String>) -> bool {
    let zsh = crate::paths::shell_dir().join("zsh");
    env.get("ZDOTDIR").is_some_and(|z| std::path::Path::new(z) == zsh)
        || env.get("PROMPT_COMMAND").is_some_and(|p| p.contains("ternarycode.bash"))
        || env.contains_key("TERNARYCODE_FISH_XDG_DIR")
        || env.contains_key(tc_agent::shell::ENV_PWSH)
}

#[cfg(test)]
mod place_tests {
    use super::*;

    #[test]
    fn a_server_shows_while_it_answers() {
        let mut servers = vec![Server { url: "http://localhost:5173/".into(), port: 5173, alive: false, misses: 0 }];
        let mut probe = |port, answers| {
            probed(&mut servers, port, answers);
            servers.iter().map(|s| (s.port, s.alive)).collect::<Vec<_>>()
        };
        // Not answering yet (still starting): kept a few probes, then forgotten.
        assert_eq!(probe(5173, false), vec![(5173, false)]);
        assert_eq!(probe(5173, true), vec![(5173, true)]);
        // It stops: gone.
        assert_eq!(probe(5173, false), vec![]);
        let mut never = vec![Server { url: "http://localhost:1/".into(), port: 1, alive: false, misses: 0 }];
        for _ in 0..SERVER_MISSES {
            probed(&mut never, 1, false);
        }
        assert!(never.is_empty());
    }

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
