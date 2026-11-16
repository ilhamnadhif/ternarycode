//! Restarting in place, for an update (macOS and Linux): the new version is started with `exec`
//! in this same process, so the programs running in the panes, children of this process, keep
//! running and their terminals stay open. Before it, each pane's terminal is drawn into a file
//! and its fd kept open through `exec`, the hook socket too, and what the panes knew (status,
//! agent, titles) is written here. The new run reads it at launch and takes them over: the
//! agents carry on as if nothing happened.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tc_core::PaneId;
use tc_core::agent::AgentKind;
use tc_core::status::TrackerSave;

/// The variable that tells the new run where the handoff is.
pub const ENV: &str = "TERNARYCODE_HANDOFF";

/// The format of the file (a run that does not know it starts as after a normal launch).
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct Handoff {
    version: u32,
    pub hooks: Option<tc_agent::server::Handoff>,
    pub panes: Vec<Handed>,
}

/// One pane whose program keeps running.
#[derive(Serialize, Deserialize)]
pub struct Handed {
    pub id: PaneId,
    /// The terminal's fd and its program's pid.
    pub master: i32,
    pub pid: u32,
    /// The grid the terminal had, which the screen file was drawn at.
    pub cols: usize,
    pub rows: usize,
    /// The terminal drawn as bytes (`tc_term::snapshot`).
    pub screen: PathBuf,
    pub extras: tc_term::snapshot::Extras,
    pub tracker: TrackerSave,
    pub hook_env: Option<(String, String)>,
    pub title: Option<String>,
    pub program_title: Option<String>,
    pub cwd: PathBuf,
    pub preset: Option<String>,
    pub agent_session: Option<String>,
    pub last_agent: Option<AgentKind>,
    pub progress: Option<(u8, Option<u8>)>,
    pub shell_pgid: Option<i32>,
    pub typed: bool,
    /// The agent run's tree, how long ago it started (ms), and its agent (Review's "Since").
    pub run_snapshot: Option<(String, u64, AgentKind)>,
    pub snapshot_armed: bool,
}

/// Where the handoff and the screens go: in the data folder, closed to other users.
pub fn dir() -> PathBuf {
    crate::paths::data_dir().join("handoff")
}

impl Handoff {
    pub fn new(hooks: Option<tc_agent::server::Handoff>, panes: Vec<Handed>) -> Self {
        Self { version: VERSION, hooks, panes }
    }

    /// Closes the fds kept open for the new run, when it did not start: they must not go to
    /// the programs this run starts next, nor stay open with it.
    pub fn close(&self) {
        // SAFETY: fds this run set aside for the handoff and no longer uses otherwise (a
        // pane's terminal stays open in its pane, only marked close-on-exec again).
        unsafe {
            for p in &self.panes {
                libc::fcntl(p.master, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            if let Some(h) = &self.hooks {
                libc::close(h.fd);
            }
        }
    }

    /// Writes the handoff, for [`take`] in the new run. Returns its path.
    pub fn write(&self) -> std::io::Result<PathBuf> {
        let path = dir().join("handoff.json");
        tc_core::persist::save_json(&path, self)?;
        Ok(path)
    }
}

/// The handoff read at launch, until the app takes it.
static TAKEN: std::sync::Mutex<Option<Handoff>> = std::sync::Mutex::new(None);

/// Reads the handoff left by the previous run, when this run is its restart in place: at the
/// very start of `main`, before other threads run, since the variable is cleared (the panes'
/// programs and anything started later must not see it). The file is removed; only one in this
/// app's own handoff folder is taken. True when there is one, for [`take`].
pub fn read_at_launch() -> bool {
    let Some(path) = std::env::var_os(ENV).map(PathBuf::from) else {
        return false;
    };
    // SAFETY: at the very start of main, before other threads run.
    unsafe { std::env::remove_var(ENV) };
    let ours = path.parent().is_some_and(|p| same_dir(p, &dir()));
    let text = ours.then(|| std::fs::read_to_string(&path).ok()).flatten();
    let _ = std::fs::remove_file(&path);
    if let Some(t) = &text {
        keep_from_children(t);
    }
    let handoff = text
        .and_then(|t| serde_json::from_str::<Handoff>(&t).ok())
        .filter(|h| h.version == VERSION);
    let found = handoff.is_some();
    *TAKEN.lock().unwrap_or_else(|e| e.into_inner()) = handoff;
    found
}

/// The fds handed over came through `exec` open: closed on exec again at once, before this run
/// starts anything (a login shell, git), which would get the panes' terminals otherwise and
/// keep them open after their panes close. Taking a pane over sets it again; a handoff not taken
/// (of another format) has them closed on exec too.
fn keep_from_children(text: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return };
    let masters = v["panes"].as_array().into_iter().flatten().filter_map(|p| p["master"].as_i64());
    for fd in masters.chain(v["hooks"]["fd"].as_i64()) {
        // Never the standard ones (a damaged file).
        if let Ok(fd) = i32::try_from(fd)
            && fd > 2
        {
            // SAFETY: only the fd's close-on-exec flag changes.
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
    }
}

/// The handoff [`read_at_launch`] found, once. Without one, what an unfinished restart left
/// behind is removed.
pub fn take() -> Option<Handoff> {
    let handoff = TAKEN.lock().unwrap_or_else(|e| e.into_inner()).take();
    if handoff.is_none() {
        clean();
    }
    handoff
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Writes a pane's screen for the new run, readable by the user only. Not synced to disk: the
/// new run reads it a moment later.
pub fn write_screen(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?.write_all(bytes)
}

/// Removes what a handoff left behind (screens of panes not taken over).
pub fn clean() {
    let _ = std::fs::remove_dir_all(dir());
}

/// How long ago `t` was, in ms.
pub fn ago(now: Instant, t: Instant) -> u64 {
    now.saturating_duration_since(t).as_millis() as u64
}

/// The instant `ms` before `now`.
pub fn back(now: Instant, ms: u64) -> Instant {
    now.checked_sub(Duration::from_millis(ms)).unwrap_or(now)
}
