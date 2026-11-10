//! A project at run time: its folder, tabs (each a split layout of panes) and git summary.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tc_core::model::{ProjectSave, SavedDir, SavedLayout, TabSave};
use tc_core::{PaneId, ProjectId};
use tc_term::layout::{Layout, SplitDir};

pub struct Tab {
    pub layout: Layout,
    pub focused: PaneId,
    pub title: Option<String>,
}

pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,
    pub color: u8,
    pub preset: Option<String>,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    pub git: Option<std::sync::Arc<GitInfo>>,
    /// What kind of project it is (its icon in the sidebar), found with the git summary.
    pub kind: Kind,
}

/// The kind of a project, from the files at its top (the sidebar shows its icon).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Folder,
    Rust,
    Go,
    TypeScript,
    JavaScript,
    React,
    Flutter,
    Dart,
    Python,
    Swift,
    Java,
    Cpp,
    Web,
}

impl Kind {
    /// Looks at the project folder (a few file checks; run it off the UI thread).
    pub fn detect(dir: &Path) -> Self {
        let has = |name: &str| dir.join(name).exists();
        if has("Cargo.toml") {
            return Self::Rust;
        }
        if has("go.mod") {
            return Self::Go;
        }
        if has("pubspec.yaml") {
            let flutter = std::fs::read_to_string(dir.join("pubspec.yaml"))
                .is_ok_and(|s| s.contains("flutter"));
            return if flutter { Self::Flutter } else { Self::Dart };
        }
        if has("package.json") {
            let pkg = std::fs::read_to_string(dir.join("package.json")).unwrap_or_default();
            if pkg.contains("\"react\"") || pkg.contains("\"next\"") || pkg.contains("\"react-native\"") {
                return Self::React;
            }
            return if has("tsconfig.json") || pkg.contains("\"typescript\"") {
                Self::TypeScript
            } else {
                Self::JavaScript
            };
        }
        if has("Package.swift") || std::fs::read_dir(dir).is_ok_and(|mut d| {
            d.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().ends_with(".xcodeproj")))
        }) {
            return Self::Swift;
        }
        if has("pyproject.toml") || has("requirements.txt") || has("setup.py") || has("Pipfile") {
            return Self::Python;
        }
        if has("pom.xml") || has("build.gradle") || has("build.gradle.kts") || has("settings.gradle") {
            return Self::Java;
        }
        if has("CMakeLists.txt") || has("meson.build") || has("Makefile") && has("src") {
            return Self::Cpp;
        }
        if has("index.html") {
            return Self::Web;
        }
        Self::Folder
    }

    /// Its name, for the tooltip.
    pub fn label(self) -> &'static str {
        match self {
            Self::Folder => "",
            Self::Rust => "Rust",
            Self::Go => "Go",
            Self::TypeScript => "TypeScript",
            Self::JavaScript => "JavaScript",
            Self::React => "React",
            Self::Flutter => "Flutter",
            Self::Dart => "Dart",
            Self::Python => "Python",
            Self::Swift => "Swift",
            Self::Java => "Java / Kotlin",
            Self::Cpp => "C / C++",
            Self::Web => "Web",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitInfo {
    pub branch: String,
    /// Changed files in the whole repository.
    pub changed: usize,
    /// Changed files under the project folder (absolute paths), for the file tree.
    pub changes: HashMap<PathBuf, Change>,
    /// Folders under the project holding a changed file.
    pub changed_dirs: HashSet<PathBuf>,
}

/// How a file differs from the last commit (`git status`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflict,
}

impl Change {
    /// The letter shown beside the file (as in most editors).
    pub fn letter(self) -> &'static str {
        match self {
            Change::Modified => "M",
            Change::Added => "A",
            Change::Deleted => "D",
            Change::Renamed => "R",
            Change::Untracked => "U",
            Change::Conflict => "C",
        }
    }

    fn from_xy(x: u8, y: u8) -> Self {
        match (x, y) {
            (b'?', b'?') => Change::Untracked,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => Change::Conflict,
            (b'R', _) | (_, b'R') => Change::Renamed,
            (b'A', _) => Change::Added,
            (b'D', _) | (_, b'D') => Change::Deleted,
            _ => Change::Modified,
        }
    }
}

/// Project colours (index saved in the workspace).
pub const COLORS: [(u8, u8, u8); 8] = [
    (0x5b, 0x9b, 0xd5),
    (0x4f, 0xb0, 0xa0),
    (0x8f, 0xb5, 0x5e),
    (0xd9, 0xa4, 0x41),
    (0xd9, 0x7a, 0x5b),
    (0xc0, 0x76, 0xa8),
    (0x63, 0xb4, 0xc9),
    (0x9c, 0x9c, 0x9c),
];

impl Project {
    pub fn color32(&self) -> eframe::egui::Color32 {
        let (r, g, b) = COLORS[self.color as usize % COLORS.len()];
        eframe::egui::Color32::from_rgb(r, g, b)
    }

    pub fn tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active_tab)
    }

    pub fn tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active_tab)
    }

    pub fn panes(&self) -> Vec<PaneId> {
        self.tabs.iter().flat_map(|t| t.layout.panes()).collect()
    }

    /// The focused pane of the active tab.
    pub fn current_pane(&self) -> Option<PaneId> {
        self.tab().map(|t| t.focused)
    }

    /// The tab holding `pane`.
    pub fn tab_of(&self, pane: PaneId) -> Option<usize> {
        self.tabs.iter().position(|t| t.layout.contains(pane))
    }

    pub fn save(&self, panes: Vec<tc_core::model::PaneSave>) -> ProjectSave {
        ProjectSave {
            id: self.id,
            name: self.name.clone(),
            path: self.path.clone(),
            color: self.color,
            preset: self.preset.clone(),
            tabs: self
                .tabs
                .iter()
                .map(|t| TabSave {
                    layout: to_saved(&t.layout),
                    focused: t.focused,
                    title: t.title.clone(),
                })
                .collect(),
            active_tab: self.active_tab,
            panes,
            editor_width: 0.0,
        }
    }
}

pub fn to_saved(l: &Layout) -> SavedLayout {
    match l {
        Layout::Pane(p) => SavedLayout::Pane(*p),
        Layout::Split { dir, ratio, a, b } => SavedLayout::Split {
            dir: match dir {
                SplitDir::Horizontal => SavedDir::Horizontal,
                SplitDir::Vertical => SavedDir::Vertical,
            },
            ratio: *ratio,
            a: Box::new(to_saved(a)),
            b: Box::new(to_saved(b)),
        },
    }
}

pub fn from_saved(l: &SavedLayout) -> Layout {
    match l {
        SavedLayout::Pane(p) => Layout::Pane(*p),
        SavedLayout::Split { dir, ratio, a, b } => Layout::Split {
            dir: match dir {
                SavedDir::Horizontal => SplitDir::Horizontal,
                SavedDir::Vertical => SplitDir::Vertical,
            },
            ratio: ratio.clamp(0.1, 0.9),
            a: Box::new(from_saved(a)),
            b: Box::new(from_saved(b)),
        },
    }
}

/// Branch, changed files and which files under the project changed, from `git status` (run
/// off the UI thread).
pub fn git_info(path: &Path) -> Option<GitInfo> {
    let git = |args: &[&str]| {
        crate::platform::command("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .ok()
            .filter(|o| o.status.success())
    };
    let out = git(&["status", "--porcelain=v1", "-z", "-b", "--untracked-files=normal"])?;
    // Status paths are relative to the repository root; the project may be a folder inside it.
    let prefix = git(&["rev-parse", "--show-prefix"])
        .map(|o| String::from_utf8_lossy(&o.stdout).trim_end_matches('\n').to_string())
        .unwrap_or_default();
    parse_status(&out.stdout, &prefix, path)
}

/// Parses `git status --porcelain=v1 -z -b` output for the project at `project`, which is
/// `prefix` below the repository root.
pub fn parse_status(out: &[u8], prefix: &str, project: &Path) -> Option<GitInfo> {
    let mut records = out.split(|b| *b == 0);
    let head = std::str::from_utf8(records.next()?).ok()?.strip_prefix("## ")?;
    let branch = head
        .split("...")
        .next()
        .unwrap_or(head)
        .trim_start_matches("No commits yet on ")
        .to_string();
    let mut info = GitInfo {
        branch,
        ..GitInfo::default()
    };
    while let Some(rec) = records.next() {
        if rec.len() < 4 {
            continue;
        }
        let (x, y) = (rec[0], rec[1]);
        if matches!(x, b'R' | b'C') {
            // The original path follows as a record of its own.
            records.next();
        }
        info.changed += 1;
        let rel = bytes_path(&rec[3..]);
        let rel = rel.to_string_lossy();
        let rel = rel.trim_end_matches('/');
        let Some(inside) = rel.strip_prefix(prefix) else { continue };
        if inside.is_empty() {
            continue;
        }
        let full = project.join(inside);
        let mut dir = full.parent();
        while let Some(d) = dir {
            if !d.starts_with(project) || d == project {
                break;
            }
            info.changed_dirs.insert(d.to_path_buf());
            dir = d.parent();
        }
        info.changes.insert(full, Change::from_xy(x, y));
    }
    Some(info)
}

#[cfg(unix)]
fn bytes_path(b: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(b))
}

#[cfg(not(unix))]
fn bytes_path(b: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(b).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_round_trip() {
        let l = Layout::Split {
            dir: SplitDir::Vertical,
            ratio: 0.3,
            a: Box::new(Layout::Pane(1)),
            b: Box::new(Layout::Split {
                dir: SplitDir::Horizontal,
                ratio: 0.5,
                a: Box::new(Layout::Pane(2)),
                b: Box::new(Layout::Pane(3)),
            }),
        };
        assert_eq!(from_saved(&to_saved(&l)), l);
    }

    #[test]
    fn git_summary_of_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap()
        };
        run(&["init", "-q", "-b", "trunk"]);
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        let info = git_info(dir.path()).unwrap();
        assert_eq!(info.branch, "trunk");
        assert_eq!(info.changed, 1);
        assert_eq!(info.changes.get(&dir.path().join("a.txt")), Some(&Change::Untracked));
        assert!(git_info(&std::env::temp_dir().join("surely-not-a-repo-tc")).is_none());
    }

    #[test]
    fn status_of_each_file_under_the_project() {
        let out = b"## main...origin/main [ahead 1]\0 M app/src/main.rs\0A  app/new.rs\0R  app/b.rs\0app/a.rs\0?? app/tmp/\0 D other/x.rs\0UU app/c.rs\0";
        let root = Path::new("/r/app");
        let info = parse_status(out, "app/", root).unwrap();
        assert_eq!(info.branch, "main");
        assert_eq!(info.changed, 6);
        let get = |p: &str| info.changes.get(&root.join(p)).copied();
        assert_eq!(get("src/main.rs"), Some(Change::Modified));
        assert_eq!(get("new.rs"), Some(Change::Added));
        assert_eq!(get("b.rs"), Some(Change::Renamed));
        assert_eq!(get("a.rs"), None, "the old name of a rename is not a change of its own");
        assert_eq!(get("tmp"), Some(Change::Untracked));
        assert_eq!(get("c.rs"), Some(Change::Conflict));
        // Outside the project: counted for the repository, not in the tree.
        assert_eq!(info.changes.len(), 5);
        assert!(info.changed_dirs.contains(&root.join("src")));
        assert!(!info.changed_dirs.contains(root));
        // A repository without commits.
        let fresh = parse_status(b"## No commits yet on trunk\0?? x\0", "", root).unwrap();
        assert_eq!(fresh.branch, "trunk");
        assert_eq!(fresh.changes.get(&root.join("x")), Some(&Change::Untracked));
    }
}
