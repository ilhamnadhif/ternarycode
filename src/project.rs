//! A project at run time: its folder, tabs (each a split layout of panes) and git summary.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tc_core::model::{ProjectSave, SavedDir, SavedLayout, TabSave, WorktreeSave, WorktreeSetup};
use tc_core::{PaneId, ProjectId};
use eframe::egui::Rect;
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
    /// The project this one is a worktree of (listed under it).
    pub parent: Option<ProjectId>,
    /// A worktree made for an agent run: its branch and where it started.
    pub worktree: Option<WorktreeSave>,
    /// What a new worktree of this project gets.
    pub worktree_setup: Option<WorktreeSetup>,
    /// Its worktrees are folded away in the sidebar.
    pub folded: bool,
    /// Run Agent last chose a new worktree here.
    pub new_worktree: bool,
    /// Sites its agents may open in its browser without asking.
    pub sites: Vec<String>,
    /// Its folder is gone (deleted outside TernaryCode).
    pub missing: bool,
    /// Its merged branch was offered for removal already (once per run of the app).
    pub merge_offered: bool,
    /// Its Browser panel: its pages (one per agent, and the user's), at the right of Focus.
    pub panel: Panel,
}

/// A project's Browser panel: at the right of the terminals in Focus, like the Code drawer
/// (one of the two shows). Each agent has a page of its own in it, and so does the user; the
/// page in front follows the terminal the user picks.
#[derive(Clone, Debug, Default)]
pub struct Panel {
    /// Its pages (browser ids), in the order of their tabs.
    pub pages: Vec<PaneId>,
    /// The page in front.
    pub front: Option<PaneId>,
    /// Shown (in Focus, and in a tile big enough).
    pub open: bool,
    /// Share of the area beside the sidebar it takes (0: the default).
    pub width: f32,
    /// It fills the area of the terminals for a while.
    pub max: bool,
    /// The user kept the terminals side by side, squeezed (no stacking for this project).
    pub side_by_side: bool,
    /// The terminals were shown stacked for it, and the user was told (once per run).
    pub told_stacked: bool,
    /// The terminal last seen focused (the page in front follows a change of it).
    pub seen_focus: Option<PaneId>,
    /// How the terminals were laid beside it the last time Focus showed it (a tile does the same),
    /// and whether it floated over them.
    pub stacked: bool,
    pub floating: bool,
    /// The width its page had the last time Focus showed it (a tile zooms its page from it).
    pub focus_width: f32,
}

/// The share of the area a panel takes by default, and its bounds.
pub const PANEL_SHARE: f32 = 0.45;
/// The least width a panel gets (a page narrower than this is a phone's).
pub const PANEL_MIN: f32 = 440.0;

/// Where Focus lays out a tab's panes and its project's panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelPlace {
    /// Where the panes go.
    pub panes: Rect,
    /// Where the panel goes (none: not shown).
    pub panel: Option<Rect>,
    /// The panes show stacked (one above the other) while the panel is open.
    pub stacked: bool,
    /// The panel floats over the panes (no room beside them).
    pub float: bool,
    /// The panel fills the area (the panes are not shown).
    pub max: bool,
}

impl PanelPlace {
    pub fn without(body: Rect) -> Self {
        Self { panes: body, panel: None, stacked: false, float: false, max: false }
    }
}

/// Places a panel in `body` beside `layout`'s panes. `room(rects)`: the panes laid out so each
/// gets what a terminal needs. Side by side when they have it; else stacked (when they are
/// side by side and the user did not keep them so) if that gives it; else, the user's choice,
/// side by side squeezed; else the panel floats over them.
pub fn place_panel(body: Rect, panel: &Panel, layout: &Layout, room: impl Fn(&[(PaneId, Rect)]) -> bool) -> PanelPlace {
    let none = PanelPlace::without(body);
    if panel.shown().is_none() || body.width() < 1.0 {
        return none;
    }
    if panel.max {
        return PanelPlace { panel: Some(body), max: true, ..none };
    }
    let w = (body.width() * panel.share()).round().max(PANEL_MIN).min(body.width());
    let x = body.right() - w;
    let right = Rect::from_min_max(eframe::egui::pos2(x, body.top()), body.max);
    // The sheets beside it end a gap before it.
    let rest = Rect::from_min_max(body.min, eframe::egui::pos2(x - crate::ui::style::GAP, body.bottom()));
    let beside = PanelPlace { panes: rest, panel: Some(right), ..none };
    if rest.width() >= 1.0 {
        if room(&layout.arrange(rest).0) {
            return beside;
        }
        if layout.has_columns() && !panel.side_by_side && room(&layout.stacked().arrange(rest).0) {
            return PanelPlace { stacked: true, ..beside };
        }
        if panel.side_by_side && rest.width() >= PANEL_MIN / 2.0 {
            return beside;
        }
    }
    PanelPlace { panel: Some(right), float: true, ..none }
}

impl Panel {
    /// The share of the area it takes.
    pub fn share(&self) -> f32 {
        if self.width > 0.0 { self.width.clamp(0.25, 0.75) } else { PANEL_SHARE }
    }

    /// Shown with a page in front.
    pub fn shown(&self) -> Option<PaneId> {
        self.front.filter(|f| self.open && self.pages.contains(f))
    }

    /// Takes a page away; the one beside it comes to the front if it was. Returns whether it was
    /// there. With no page left the panel closes.
    pub fn remove(&mut self, id: PaneId) -> bool {
        let Some(i) = self.pages.iter().position(|p| *p == id) else { return false };
        self.pages.remove(i);
        if self.front == Some(id) {
            self.front = self.pages.get(i.min(self.pages.len().saturating_sub(1))).copied();
        }
        if self.pages.is_empty() {
            self.front = None;
            self.open = false;
            self.max = false;
        }
        true
    }
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

    /// Two letters for the project's tile ("Rs", "Ts"); none for a plain folder (its name's
    /// initials are used).
    pub fn letters(self) -> Option<&'static str> {
        Some(match self {
            Self::Folder => return None,
            Self::Rust => "Rs",
            Self::Go => "Go",
            Self::TypeScript => "Ts",
            Self::JavaScript => "Js",
            Self::React => "Re",
            Self::Flutter => "Fl",
            Self::Dart => "Dt",
            Self::Python => "Py",
            Self::Swift => "Sw",
            Self::Java => "Jv",
            Self::Cpp => "C+",
            Self::Web => "Wb",
        })
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
    /// A worktree: commits on its branch that its base has not (ahead) and the other way.
    pub ahead: usize,
    pub behind: usize,
    /// A worktree whose branch has commits, all of them in its base now.
    pub merged: bool,
    /// A worktree: files its commits changed since it left its base.
    pub branch_files: usize,
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
    // Sky, teal, lime, sand, coral, rose, violet, slate: apart from each other and from the
    // status colours (amber, green, red, blue) where they could be taken for one.
    (0x5a, 0xa6, 0xe6),
    (0x45, 0xb5, 0xa8),
    (0x9c, 0xbf, 0x5a),
    (0xc9, 0xa4, 0x6a),
    (0xe5, 0x8a, 0x6b),
    (0xe0, 0x7a, 0xa0),
    (0x9d, 0x8c, 0xf0),
    (0x8e, 0x9a, 0xab),
];

/// Two letters from a name: the first letters of its first two words, else its first two
/// letters, the first upper case ("docs-site" → "Ds", "infra" → "In", "x" → "X").
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let mut out: Vec<char> = match words.as_slice() {
        [a, b, ..] => a.chars().take(1).chain(b.chars().take(1)).collect(),
        [a] => a.chars().take(2).collect(),
        [] => vec!['?'],
    };
    if let Some(c) = out.first_mut() {
        *c = c.to_uppercase().next().unwrap_or(*c);
    }
    for c in out.iter_mut().skip(1) {
        *c = c.to_lowercase().next().unwrap_or(*c);
    }
    out.into_iter().collect()
}

impl Project {
    /// A project at run time from what was saved (no tabs yet).
    pub fn from_save(saved: &ProjectSave) -> Self {
        Project {
            id: saved.id,
            name: saved.name.clone(),
            path: saved.path.clone(),
            color: saved.color,
            preset: saved.preset.clone(),
            tabs: Vec::new(),
            active_tab: 0,
            git: None,
            kind: Kind::default(),
            parent: saved.parent,
            worktree: saved.worktree.clone(),
            worktree_setup: saved.worktree_setup.clone(),
            folded: saved.folded,
            new_worktree: saved.new_worktree,
            sites: saved.browser_sites.clone(),
            missing: false,
            merge_offered: false,
            panel: saved.browser_panel.as_ref().map_or_else(Panel::default, |b| Panel {
                open: b.open,
                width: b.width,
                front: b.front,
                ..Panel::default()
            }),
        }
    }

    /// A worktree TernaryCode made (listed under its project).
    pub fn is_worktree(&self) -> bool {
        self.parent.is_some()
    }

    /// The two letters of its tile: its kind's, else its name's initials ("docs-site" → "Ds",
    /// "infra" → "In").
    pub fn letters(&self) -> String {
        if let Some(l) = self.kind.letters() {
            return l.to_string();
        }
        initials(&self.name)
    }

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
            parent: self.parent,
            worktree: self.worktree.clone(),
            worktree_setup: self.worktree_setup.clone(),
            folded: self.folded,
            new_worktree: self.new_worktree,
            browser_sites: self.sites.clone(),
            browser_panel: (!self.panel.pages.is_empty() || self.panel.width > 0.0).then_some(tc_core::model::PanelSave {
                open: self.panel.open && !self.panel.pages.is_empty(),
                width: self.panel.width,
                front: self.panel.front,
            }),
        }
    }
}

/// The order of the project list (sidebar, Grid): projects by name, each followed by its
/// worktrees by name. Returns the projects' places in `names` in that order; a worktree whose
/// project is gone is listed as a project.
pub fn list_order(items: &[(ProjectId, Option<ProjectId>, &str, &Path)]) -> Vec<usize> {
    let known: HashSet<ProjectId> = items.iter().map(|x| x.0).collect();
    let by_name = |a: &usize, b: &usize| {
        let (x, y) = (&items[*a], &items[*b]);
        name_order(x.2, y.2).then_with(|| x.3.cmp(y.3))
    };
    let mut tops: Vec<usize> = (0..items.len()).filter(|&i| items[i].1.is_none_or(|p| !known.contains(&p))).collect();
    tops.sort_by(by_name);
    let mut out = Vec::with_capacity(items.len());
    for t in tops {
        out.push(t);
        let mut kids: Vec<usize> = (0..items.len()).filter(|&i| items[i].1 == Some(items[t].0)).collect();
        kids.sort_by(by_name);
        out.extend(kids);
    }
    out
}

/// The order projects are listed in (sidebar, Grid, ⌘1–⌘9): by name, as people read names.
/// Case is ignored and runs of digits compare as numbers (`api2` before `api10`); names equal
/// that way fall back to their exact text.
pub fn name_order(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let digits = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut n = String::new();
                    while let Some(c) = it.next_if(char::is_ascii_digit) {
                        n.push(c);
                    }
                    n
                };
                let (m, n) = (digits(&mut x), digits(&mut y));
                // Leading zeros aside, the longer number is the larger one.
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let by_value = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if by_value != Ordering::Equal {
                    return by_value;
                }
            }
            (Some(c), Some(d)) => {
                let by_letter = c.to_lowercase().cmp(d.to_lowercase());
                if by_letter != Ordering::Equal {
                    return by_letter;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// A project's saved tabs with the browser pages older versions kept in them (`is_page`) taken
/// out: they are its Browser panel's pages now.
pub struct Restored<'a> {
    /// The tabs left (a tab that held only pages goes), each with what was saved of it.
    pub tabs: Vec<(Layout, &'a TabSave)>,
    /// The pages taken out, in the order found.
    pub pages: Vec<PaneId>,
    /// The tab in view among those left (the one before it when it went).
    pub active: usize,
    /// A page was in the tab in view: the panel shows.
    pub page_in_view: bool,
}

pub fn take_out_pages(tabs: &[TabSave], active: usize, is_page: impl Fn(PaneId) -> bool) -> Restored<'_> {
    let mut out = Restored { tabs: Vec::new(), pages: Vec::new(), active: 0, page_in_view: false };
    for (i, tab) in tabs.iter().enumerate() {
        let mut layout = from_saved(&tab.layout);
        let mut empty = false;
        for id in layout.panes() {
            if !is_page(id) {
                continue;
            }
            out.pages.push(id);
            out.page_in_view |= i == active;
            if !layout.remove(id) {
                empty = true;
            }
        }
        if empty {
            continue;
        }
        if i <= active {
            out.active = out.tabs.len();
        }
        out.tabs.push((layout, tab));
    }
    out
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
    // That place does not change: asked once per folder (again when the folder becomes a
    // repository of its own).
    static PREFIXES: std::sync::LazyLock<std::sync::Mutex<HashMap<(PathBuf, bool), String>>> = std::sync::LazyLock::new(Default::default);
    let key = (path.to_path_buf(), path.join(".git").exists());
    let known = PREFIXES.lock().ok().and_then(|m| m.get(&key).cloned());
    let prefix = match known {
        Some(p) => p,
        None => {
            let p = git(&["rev-parse", "--show-prefix"]).map(|o| String::from_utf8_lossy(&o.stdout).trim_end_matches('\n').to_string());
            if let (Some(p), Ok(mut m)) = (p.as_ref(), PREFIXES.lock()) {
                m.insert(key, p.clone());
            }
            p.unwrap_or_default()
        }
    };
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
        if matches!(x, b'R' | b'C') || matches!(y, b'R' | b'C') {
            // The original path follows as a record of its own (a rename in the index, or one
            // git sees in the working tree: ` R new\0old`).
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
    fn tiles_get_two_letters() {
        assert_eq!(initials("docs-site"), "Ds");
        assert_eq!(initials("infra"), "In");
        assert_eq!(initials("x"), "X");
        assert_eq!(initials("my_great app"), "Mg");
        assert_eq!(initials("--"), "?");
        assert_eq!(Kind::Rust.letters(), Some("Rs"));
        assert_eq!(Kind::Folder.letters(), None);
    }

    #[test]
    fn pages_in_tabs_move_out_of_them() {
        let tab = |layout: SavedLayout, focused: PaneId| TabSave { layout, focused, title: None };
        let split = |a: PaneId, b: PaneId| SavedLayout::Split { dir: SavedDir::Horizontal, ratio: 0.5, a: Box::new(SavedLayout::Pane(a)), b: Box::new(SavedLayout::Pane(b)) };
        // Tab 0: a terminal beside a page; tab 1: a page alone; tab 2: a terminal.
        let tabs = [tab(split(1, 2), 2), tab(SavedLayout::Pane(3), 3), tab(SavedLayout::Pane(4), 4)];
        let page = |id: PaneId| id == 2 || id == 3;
        let r = take_out_pages(&tabs, 2, page);
        assert_eq!(r.pages, [2, 3]);
        assert_eq!(r.tabs.iter().map(|(l, _)| l.panes()).collect::<Vec<_>>(), [vec![1], vec![4]]);
        assert_eq!(r.active, 1, "the tab in view keeps its place among those left");
        assert!(!r.page_in_view);
        // The page's own tab was in view: the tab before it is, and the panel shows.
        let r = take_out_pages(&tabs, 1, page);
        assert_eq!(r.active, 0);
        assert!(r.page_in_view);
        // Nothing but pages: no tab left (the caller makes a terminal).
        let r = take_out_pages(&tabs[1..2], 0, page);
        assert!(r.tabs.is_empty());
        assert_eq!(r.pages, [3]);
        // No pages: all as saved.
        let r = take_out_pages(&tabs, 2, |_| false);
        assert_eq!(r.tabs.len(), 3);
        assert_eq!(r.active, 2);
        assert!(r.pages.is_empty());
    }

    #[test]
    fn a_panel_closes_with_its_last_page() {
        let mut p = Panel { pages: vec![5, 6, 7], front: Some(6), open: true, ..Panel::default() };
        assert_eq!(p.shown(), Some(6));
        assert!(p.remove(6));
        assert_eq!(p.front, Some(7), "the page after it comes to the front");
        assert!(p.remove(7));
        assert_eq!(p.front, Some(5));
        assert!(!p.remove(9));
        assert!(p.remove(5));
        assert_eq!((p.front, p.open, p.shown()), (None, false, None));
        assert!((Panel::default().share() - PANEL_SHARE).abs() < f32::EPSILON);
        assert!((Panel { width: 0.9, ..Panel::default() }.share() - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn a_panel_is_placed_without_squeezing_the_terminals() {
        use eframe::egui::{pos2, vec2};
        let body = Rect::from_min_size(pos2(0.0, 0.0), vec2(1040.0, 700.0));
        let shown = Panel { pages: vec![9], front: Some(9), open: true, ..Panel::default() };
        // A terminal needs 470 wide and 300 high here.
        let room = |rects: &[(PaneId, Rect)]| rects.iter().all(|(_, r)| r.width() >= 470.0 && r.height() >= 300.0);
        let one = Layout::Pane(1);
        let mut two = Layout::Pane(1);
        two.split(1, 2, SplitDir::Horizontal);
        let mut four = two.clone();
        four.split(1, 3, SplitDir::Vertical);
        four.split(2, 4, SplitDir::Vertical);

        // Hidden, or no page: the panes have it all.
        assert_eq!(place_panel(body, &Panel::default(), &one, room), PanelPlace::without(body));
        // One terminal: beside it, 45%.
        let p = place_panel(body, &shown, &one, room);
        assert_eq!(p.panel.map(|r| r.width()), Some(468.0));
        assert!(!p.stacked && !p.float && p.panes.right() < p.panel.unwrap().left());
        // Two side by side: they don't fit beside it, stacked they do.
        let p = place_panel(body, &shown, &two, room);
        assert!(p.stacked && !p.float);
        // Kept side by side by the user: squeezed, as asked.
        let kept = Panel { side_by_side: true, ..shown.clone() };
        let p = place_panel(body, &kept, &two, room);
        assert!(!p.stacked && !p.float && p.panel.is_some());
        // Four (2 × 2): stacked they would be too low; the panel floats over them.
        let p = place_panel(body, &shown, &four, room);
        assert!(p.float && p.panes == body);
        // Too narrow for anything beside it: it floats, at its least width.
        let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 700.0));
        let p = place_panel(narrow, &shown, &one, room);
        assert!(p.float);
        assert_eq!(p.panel.map(|r| r.width()), Some(PANEL_MIN));
        // Filling the window.
        let p = place_panel(body, &Panel { max: true, ..shown.clone() }, &two, room);
        assert!(p.max && p.panel == Some(body));
    }

    #[test]
    fn projects_sort_by_name_as_read() {
        let mut names = ["web", "api10", "Shop-admin", "api2", "shop", "API1", "Ölbaum", "zeta", "app01", "app1"];
        names.sort_by(|a, b| name_order(a, b));
        assert_eq!(names, ["API1", "api2", "api10", "app01", "app1", "shop", "Shop-admin", "web", "zeta", "Ölbaum"]);
        // Only case apart: still one order, every time.
        assert_eq!(name_order("Api", "api"), "Api".cmp("api"));
        assert_eq!(name_order("api", "api"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn worktrees_follow_their_project() {
        let p = Path::new("/x");
        let items = [
            (1, None, "ternary-engine", p),
            (2, Some(1), "lexer-fix", p),
            (3, None, "api", p),
            (4, Some(1), "generics", p),
            (5, Some(99), "orphan", p),
            (6, None, "web", p),
        ];
        let order: Vec<u64> = list_order(&items).into_iter().map(|i| items[i].0).collect();
        assert_eq!(order, [3, 5, 1, 4, 2, 6]);
    }

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
        // A rename git sees in the working tree (`git add -N` of the new name): its old name
        // follows too.
        let moved = parse_status(b"## main\0 R src/new_name.rs\0src/old_name.rs\0", "", root).unwrap();
        assert_eq!(moved.changed, 1);
        assert_eq!(moved.changes.get(&root.join("src/new_name.rs")), Some(&Change::Renamed));
        assert_eq!(moved.changes.len(), 1);
    }
}
