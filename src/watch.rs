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

pub fn ignored(root: &Path, path: &Path) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        IGNORED.contains(&s.as_ref())
    })
}

#[derive(Default)]
pub struct Watchers {
    by_project: HashMap<ProjectId, RecommendedWatcher>,
}

impl Watchers {
    pub fn watch(&mut self, project: ProjectId, root: PathBuf, tx: Sender<AppMsg>, ctx: egui::Context) {
        if self.by_project.contains_key(&project) {
            return;
        }
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
            let _ = tx.send(AppMsg::Files(project, paths));
            // Bursts of changes (an agent writing many files) share one look.
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        });
        if let Ok(mut w) = watcher
            && w.watch(&root, RecursiveMode::Recursive).is_ok()
        {
            self.by_project.insert(project, w);
        }
    }

    pub fn unwatch(&mut self, project: ProjectId) {
        self.by_project.remove(&project);
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
    }
}
