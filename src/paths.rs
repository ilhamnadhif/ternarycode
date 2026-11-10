//! Where TernaryCode keeps its files.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const APP_NAME: &str = "ternarycode";

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// `~/Library/Application Support/ternarycode` (macOS), `~/.local/share/ternarycode` (Linux), or
/// `TERNARYCODE_DATA_DIR` in a debug build (tests and scripted UI checks; release builds ignore
/// it).
pub fn data_dir() -> &'static Path {
    DATA_DIR.get_or_init(|| {
        if cfg!(debug_assertions)
            && let Some(dir) = std::env::var_os("TERNARYCODE_DATA_DIR")
        {
            return PathBuf::from(dir);
        }
        dirs::data_dir()
            .unwrap_or_else(|| home_dir().join("Library/Application Support"))
            .join(APP_NAME)
    })
}

pub fn logs_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home_dir().join("Library/Logs/TernaryCode")
    } else if cfg!(windows) {
        dirs::data_local_dir()
            .unwrap_or_else(|| home_dir().join("AppData/Local"))
            .join(APP_NAME)
            .join("logs")
    } else {
        dirs::state_dir()
            .unwrap_or_else(|| home_dir().join(".local/state"))
            .join(APP_NAME)
            .join("logs")
    }
}

/// `path` made absolute and resolved (links, `..`), as `canonicalize` does, or as given when it
/// does not exist. On Windows without the `\\?\` prefix `canonicalize` adds: other programs
/// (git, shells, language servers) and people read the plain form.
pub fn canonical(path: &Path) -> PathBuf {
    let Ok(p) = path.canonicalize() else {
        return path.to_path_buf();
    };
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\")
            && rest.as_bytes().get(1) == Some(&b':')
        {
            return PathBuf::from(rest);
        }
    }
    p
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn settings_file() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn workspace_file() -> PathBuf {
    data_dir().join("workspace.json")
}

/// Files TernaryCode generates for agent hooks (the `--settings` file, install records,
/// backups of agent configuration files).
pub fn integrations_dir() -> PathBuf {
    data_dir().join("integrations")
}

/// Shell integration scripts (OSC 133) for zsh, bash and fish.
pub fn shell_dir() -> PathBuf {
    data_dir().join("shell")
}

pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    tc_core::persist::ensure_private_dir(dir)
}

pub fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    tc_core::persist::write_private(path, data)
}

pub use tc_term::prefs::{contract_tilde, expand_tilde};

/// Writes files off the UI thread (an atomic write ends with a flush to disk, which can take
/// tens of milliseconds). Writes happen one at a time and the newest content for a file always
/// lands last: a write still waiting is replaced by a newer one for the same file.
#[derive(Default)]
pub struct Saver {
    pending: std::sync::Mutex<std::collections::HashMap<PathBuf, Vec<u8>>>,
    writing: std::sync::Mutex<()>,
}

impl Saver {
    pub fn queue(self: &std::sync::Arc<Self>, rt: &tokio::runtime::Runtime, path: PathBuf, data: Vec<u8>) {
        if let Ok(mut p) = self.pending.lock() {
            p.insert(path, data);
        }
        let me = self.clone();
        rt.spawn_blocking(move || me.flush());
    }

    /// Writes what is waiting, after any write in progress (also used at quit, on the UI
    /// thread, so nothing is lost).
    pub fn flush(&self) {
        let _one_at_a_time = self.writing.lock();
        let jobs: Vec<(PathBuf, Vec<u8>)> = match self.pending.lock() {
            Ok(mut p) => p.drain().collect(),
            Err(_) => return,
        };
        for (path, data) in jobs {
            let _ = write_private(&path, &data);
        }
    }
}
