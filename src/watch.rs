//! File-system events for each project folder (FSEvents on macOS, inotify on Linux): open files
//! that changed on disk, the git summary, and agents editing the same working tree (FR-A6).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use eframe::egui;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tc_core::ProjectId;

use crate::app::AppMsg;

/// Paths under these folders are not reported.
const IGNORED: &[&str] = &[".git", "target", "node_modules", ".next", "dist", "build", ".venv", "__pycache__"];

/// Only what is inside the project counts: a `build` folder above it says nothing.
pub fn ignored(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else { return false };
    rel.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        IGNORED.contains(&s.as_ref())
    })
}

/// Linux: inotify takes one watch per folder, and a recursive watch would take them for every
/// folder under `node_modules`, `target`… too, until `max_user_watches` runs out. So folders are
/// watched one by one, skipping those, and new ones as they appear.
#[cfg(target_os = "linux")]
type Held = std::sync::Arc<std::sync::Mutex<RecommendedWatcher>>;
#[cfg(not(target_os = "linux"))]
type Held = RecommendedWatcher;

#[derive(Default)]
pub struct Watchers {
    by_project: HashMap<ProjectId, Held>,
}

impl Watchers {
    /// Starts watching a project folder. One that can't be watched (a folder not there yet) is
    /// said on stderr and tried again the next time this is asked.
    pub fn watch(&mut self, project: ProjectId, root: PathBuf, tx: Sender<AppMsg>, ctx: egui::Context) {
        if self.by_project.contains_key(&project) {
            return;
        }
        #[cfg(target_os = "linux")]
        let (folders_tx, folders) = std::sync::mpsc::channel::<PathBuf>();
        #[cfg(target_os = "linux")]
        let new_folders = folders_tx.clone();
        let r = root.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            let paths: Vec<PathBuf> = event.paths.into_iter().filter(|p| !ignored(&r, p)).collect();
            if paths.is_empty() {
                return;
            }
            // A folder made or moved in is watched too (not from here: this is the watcher's
            // own thread, which adding a watch waits for).
            #[cfg(target_os = "linux")]
            if matches!(
                event.kind,
                notify::EventKind::Create(notify::event::CreateKind::Folder) | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
            ) {
                for p in &paths {
                    let _ = new_folders.send(p.clone());
                }
            }
            let _ = tx.send(AppMsg::Files(project, paths));
            // Bursts of changes (an agent writing many files) share one look.
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        });
        let mode = if cfg!(target_os = "linux") { RecursiveMode::NonRecursive } else { RecursiveMode::Recursive };
        let watcher = watcher.and_then(|mut w| w.watch(&root, mode).map(|()| w));
        match watcher {
            #[cfg(target_os = "linux")]
            Ok(w) => {
                let w = std::sync::Arc::new(std::sync::Mutex::new(w));
                let weak = std::sync::Arc::downgrade(&w);
                let _ = folders_tx.send(root.clone());
                let _ = std::thread::Builder::new()
                    .name("ternarycode-watch".into())
                    .spawn(move || watch_folders(weak, root, folders));
                self.by_project.insert(project, w);
            }
            #[cfg(not(target_os = "linux"))]
            Ok(w) => {
                self.by_project.insert(project, w);
            }
            Err(e) => eprintln!("TernaryCode: {} is not watched for changes: {e}", root.display()),
        }
    }

    pub fn unwatch(&mut self, project: ProjectId) {
        self.by_project.remove(&project);
    }
}

/// Watches each folder sent (and the folders in it, but not ignored ones) on its own. Ends with
/// the watcher: its event handler holds the sending end.
#[cfg(target_os = "linux")]
fn watch_folders(watcher: std::sync::Weak<std::sync::Mutex<RecommendedWatcher>>, root: PathBuf, folders: std::sync::mpsc::Receiver<PathBuf>) {
    while let Ok(folder) = folders.recv() {
        let Some(held) = watcher.upgrade() else { return };
        let mut todo = vec![folder];
        while let Some(dir) = todo.pop() {
            // Links are not followed (as a recursive watch does).
            if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            let Ok(mut w) = held.lock() else { return };
            if let Err(e) = w.watch(&dir, RecursiveMode::NonRecursive) {
                if matches!(e.kind, notify::ErrorKind::MaxFilesWatch) {
                    eprintln!("TernaryCode: inotify's watch limit (fs.inotify.max_user_watches) is reached: changes under {} are not all seen.", root.display());
                    return;
                }
                continue;
            }
            drop(w);
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|t| t.is_dir()) && !ignored(&root, &path) {
                    todo.push(path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_git_folders_are_ignored() {
        let root = Path::new("/r");
        assert!(ignored(root, Path::new("/r/.git/index")));
        assert!(ignored(root, Path::new("/r/target/debug/x")));
        assert!(ignored(root, Path::new("/r/web/node_modules/a/b.js")));
        assert!(!ignored(root, Path::new("/r/src/main.rs")));
        assert!(!ignored(root, Path::new("/r/targets.md")));
        // Only inside the project: a project under a `build` folder still reports its files.
        let under = Path::new("/home/me/build/app");
        assert!(!ignored(under, Path::new("/home/me/build/app/src/main.rs")));
        assert!(ignored(under, Path::new("/home/me/build/app/dist/app.js")));
        assert!(!ignored(under, Path::new("/elsewhere/dist/x")));
    }
}
