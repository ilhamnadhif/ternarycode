//! The command palette (⌘K): one list for everything, in scopes (projects ⌘P, sessions, files
//! ⌘O, the output of every pane ⇧⌘F, commands `>`) that ⇥ moves between, and the language
//! server's lists of places (references, symbols, problems). What waits for the user comes
//! first; with room, the chosen row is shown at the palette's right.

use std::path::PathBuf;
use std::rc::Rc;

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Rect, Sense, Stroke, pos2, vec2};
use tc_core::agent::AgentKind;
use tc_core::status::Status;
use tc_core::{PaneId, ProjectId};

use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::{App, PaletteMode};
use crate::i18n::{t, tf};
use crate::profiles::ProjectSessionItem;

/// Results kept for the list (the rest by typing more).
const MAX_ROWS: usize = 200;
/// A row, and the room under it.
const ROW_H: f32 = 36.0;
const ROW_GAP: f32 = 2.0;
/// A group's label (Label 11/14); the room under it is [`ROW_GAP`].
const GROUP_H: f32 = 14.0;
/// Room around the list's rows.
const LIST_PAD: f32 = 8.0;
const FIELD_H: f32 = 56.0;
const SCOPES_H: f32 = 40.0;
const FOOT_H: f32 = 40.0;
/// Narrow, the list alone; wide (with room for it), the list and the chosen row at its right.
const WIDTH: f32 = 640.0;
const WIDE: f32 = 940.0;
const LIST_W: f32 = 560.0;
const HEIGHT: f32 = 580.0;
/// The mini terminal of the preview.
const MINI_H: f32 = 230.0;
/// Rows of a group All lists at most (its scope has every one).
const SESSIONS_IN_ALL: usize = 5;
const FILES_IN_ALL: usize = 5;
/// The scopes, in the order ⇥ goes through them.
const SCOPES: [PaletteMode; 6] =
    [PaletteMode::All, PaletteMode::Projects, PaletteMode::Sessions, PaletteMode::Files, PaletteMode::Search, PaletteMode::Commands];

/// What sits before a row's label.
#[derive(Clone)]
enum Lead {
    /// An icon (16) in its colour (the secondary text when none).
    Icon(&'static str, Option<Color32>),
    /// A project's avatar (M).
    Tile(String, Color32),
    /// Something waiting: its status glyph (M) and its project's avatar (S).
    Waiting(Status, String, Color32),
    /// An agent's avatar (M).
    Agent(Option<AgentKind>),
}

/// What sits at a row's right.
#[derive(Clone, Default)]
enum Right {
    #[default]
    None,
    /// Quiet mono text: a shortcut, a line number.
    Keys(String),
    /// A quiet word ("resume").
    Word(String),
    /// A git letter in its colour.
    Letter(String, Color32),
    /// How long it waits, and a key cap (⌘J) for the first one.
    Wait(String, Option<&'static str>),
    /// A live session: how long in its state, and its status glyph.
    Live(String, Status),
}

/// What the preview at the palette's right shows for a row.
#[derive(Clone)]
enum Preview {
    None,
    Pane(PaneId),
    Project(ProjectId),
    /// A session on disk, to resume.
    Session(ProjectId, Box<ProjectSessionItem>),
    /// A file of a project.
    File(ProjectId, PathBuf),
}

#[derive(Clone)]
struct Item {
    lead: Lead,
    label: String,
    /// The label is code (a line of a file or of a terminal): monospace.
    mono: bool,
    detail: String,
    /// The detail is code (a path, a command): in the data font.
    mono_detail: bool,
    /// A status pill before the right side (a project's state).
    pill: Option<Status>,
    right: Right,
    /// A Review button at the right (an agent that finished).
    review: Option<PaneId>,
    /// The group it is listed under.
    group: String,
    /// All lists at most this many of its group (0: all).
    cap: usize,
    preview: Preview,
    action: Action,
}

impl Item {
    fn new(lead: Lead, label: impl Into<String>, action: Action) -> Self {
        Self {
            lead,
            label: label.into(),
            mono: false,
            detail: String::new(),
            mono_detail: false,
            pill: None,
            right: Right::None,
            review: None,
            group: String::new(),
            cap: 0,
            preview: Preview::None,
            action,
        }
    }
}

/// Quick Open's rows: what they were made for (query, project, file list, how many files, git
/// letters, theme) and the best of them.
type FileRowsKey = (String, Option<tc_core::ProjectId>, u64, usize, usize, bool);
type FileRows = Option<(FileRowsKey, Rc<Vec<(i32, Item)>>)>;
/// All's files: the best few of every project's (query, file lists, theme).
type AllFiles = Option<((String, u64, bool), Rc<Vec<Item>>)>;

thread_local! {
    static FILE_ROWS: std::cell::RefCell<FileRows> = const { std::cell::RefCell::new(None) };
    static ALL_FILES: std::cell::RefCell<AllFiles> = const { std::cell::RefCell::new(None) };
    /// The first lines of the file the preview shows.
    static FILE_HEAD: std::cell::RefCell<Option<(PathBuf, Rc<Vec<String>>)>> = const { std::cell::RefCell::new(None) };
}

/// Subsequence match: higher is better, `None` when `query` doesn't match.
pub(crate) fn score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let text = text.to_lowercase();
    // Char positions, not byte offsets: letters after an "é" are still next to each other.
    let mut chars = text.chars().enumerate();
    let mut score = 0;
    let mut last = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let (i, _) = chars.by_ref().find(|(_, c)| *c == q)?;
        score += match last {
            Some(l) if i == l + 1 => 3,
            _ => 1,
        };
        if i == 0 {
            score += 2;
        }
        last = Some(i);
    }
    Some(score * 100 - text.chars().count() as i32)
}

/// The characters of `text` a query matched (the first place each letter fits), for
/// highlighting.
pub(crate) fn matched(query: &str, text: &str) -> Vec<bool> {
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    let mut out = vec![false; lower.len()];
    let mut from = 0;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        match lower.iter().skip(from).position(|c| *c == q) {
            Some(i) => {
                out[from + i] = true;
                from += i + 1;
            }
            None => return vec![false; lower.len()],
        }
    }
    out
}

/// The first lines of a text file (up to 8 KB of it; none for a binary one), kept while the
/// preview shows it.
fn file_head(path: &std::path::Path) -> Rc<Vec<String>> {
    if let Some(kept) = FILE_HEAD.with(|c| c.borrow().as_ref().filter(|(p, _)| p == path).map(|(_, l)| Rc::clone(l))) {
        return kept;
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    let _ = std::fs::File::open(path).and_then(|f| f.take(8192).read_to_end(&mut bytes));
    let lines: Vec<String> = if bytes.contains(&0) {
        Vec::new()
    } else {
        String::from_utf8_lossy(&bytes).lines().take(14).map(|l| l.replace('\t', "    ")).collect()
    };
    let lines = Rc::new(lines);
    FILE_HEAD.with(|c| *c.borrow_mut() = Some((path.to_path_buf(), Rc::clone(&lines))));
    lines
}

impl App {
    /// The agent sessions of every project: the live ones first (in the projects' order), then
    /// the ones on disk, newest first. `skip`: live panes listed elsewhere (what waits).
    fn session_items(&self, skip: &[PaneId], group_live: &str, group_earlier: &str) -> Vec<Item> {
        let mut live = Vec::new();
        let mut earlier: Vec<(std::time::SystemTime, Item)> = Vec::new();
        for pr in &self.projects {
            let project = self.project_label(pr.id);
            for s in self.merged_project_sessions(pr.id) {
                let mut it;
                match s.live_pane {
                    Some(pane) => {
                        if skip.contains(&pane) {
                            continue;
                        }
                        let Some(x) = self.panes.get(&pane) else { continue };
                        it = Item::new(Lead::Agent(Some(s.agent)), s.title.clone(), Action::FocusPane(pane));
                        it.detail = project.clone();
                        it.right = Right::Live(status::short_age(x.tracker.since), x.tracker.status);
                        it.preview = Preview::Pane(pane);
                        it.group = group_live.to_string();
                        live.push(it);
                    }
                    None => {
                        it = Item::new(
                            Lead::Agent(Some(s.agent)),
                            s.title.clone(),
                            Action::ResumeProjectSession { project: pr.id, agent: s.agent, session_id: s.id.clone(), profile_id: s.profile_id.clone() },
                        );
                        it.detail = format!("{project} · {}", tf("{} ago", &[&super::dialogs::ago(s.updated)]));
                        it.right = Right::Word(t("resume").to_string());
                        it.group = group_earlier.to_string();
                        let updated = s.updated;
                        it.preview = Preview::Session(pr.id, Box::new(s));
                        earlier.push((updated, it));
                    }
                }
            }
        }
        earlier.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        live.extend(earlier.into_iter().map(|(_, it)| it));
        live
    }

    /// The files of every project whose name matches `query`, the best few (All's Files group).
    fn all_files(&self, query: &str, p: &style::Palette) -> Rc<Vec<Item>> {
        let key = (query.to_string(), self.files_rev, self.ctx.global_style().visuals.dark_mode);
        if let Some(kept) = ALL_FILES.with(|c| c.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, rows)| Rc::clone(rows))) {
            return kept;
        }
        let mut hits: Vec<(i32, ProjectId, &PathBuf)> = Vec::new();
        for pr in self.projects.iter().filter(|pr| !pr.is_worktree()) {
            let Some(files) = self.project_files.get(&pr.id) else { continue };
            for f in files {
                let name = f.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
                if let Some(s) = score(query, &name) {
                    hits.push((s, pr.id, f));
                }
            }
        }
        hits.sort_by_key(|h| std::cmp::Reverse(h.0));
        hits.truncate(FILES_IN_ALL);
        let rows: Vec<Item> = hits
            .into_iter()
            .filter_map(|(_, id, f)| {
                let pr = self.project(id)?;
                let full = pr.path.join(f);
                let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let (icon, _) = icons::file_icon(&name, false, false, p);
                let mut it = Item::new(Lead::Icon(icon, None), name, Action::OpenFile(full.clone(), None));
                let dir = f.parent().map(|d| d.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                it.detail = if dir.is_empty() { pr.name.clone() } else { format!("{}/{dir}", pr.name) };
                it.mono_detail = true;
                if let Some(c) = pr.git.as_ref().and_then(|g| g.changes.get(&full)) {
                    it.right = Right::Letter(c.letter().to_string(), super::explorer_ui::change_color(*c, p));
                }
                it.group = t("Files").to_string();
                it.cap = FILES_IN_ALL;
                it.preview = Preview::File(id, full);
                Some(it)
            })
            .collect();
        let rows = Rc::new(rows);
        ALL_FILES.with(|c| *c.borrow_mut() = Some((key, Rc::clone(&rows))));
        rows
    }

    fn palette_items(&self, mode: PaletteMode, query: &str) -> Vec<Item> {
        let p = style::palette(self.ctx.global_style().visuals.dark_mode);
        let mut items = Vec::new();
        match mode {
            PaletteMode::Search => {
                if let Some(pal) = &self.palette {
                    for (pane, m, text) in &pal.results {
                        let Some(x) = self.panes.get(pane) else { continue };
                        let project = self.project_label(x.project);
                        let n = pal.results.iter().filter(|r| r.0 == *pane).count();
                        let mut it = Item::new(Lead::Icon(icons::TERMINAL, None), text.trim_end(), Action::RevealMatch(*pane, m.clone()));
                        it.mono = true;
                        it.group = format!("{project} · {}  —  {}", x.label(), crate::i18n::count(n, t("1 match"), t("{} matches")));
                        it.preview = Preview::Pane(*pane);
                        items.push(it);
                    }
                }
                return items;
            }
            PaletteMode::Locations => {
                if let Some(pal) = &self.palette {
                    let root = self.active_project().map(|x| x.path.clone()).unwrap_or_default();
                    for l in &pal.locations {
                        let rel = l.path.strip_prefix(&root).unwrap_or(&l.path).to_string_lossy().into_owned();
                        let n = pal.locations.iter().filter(|x| x.path == l.path).count();
                        let lead = if l.error { Lead::Icon(icons::X_CIRCLE, Some(p.err)) } else { Lead::Icon(icons::FILE_CODE, None) };
                        let mut it = Item::new(lead, l.label.clone(), Action::GotoLocation(l.path.clone(), l.line, l.col));
                        it.mono = !l.error;
                        if l.error {
                            it.detail = l.detail.clone();
                        } else {
                            it.right = Right::Keys((l.line + 1).to_string());
                        }
                        it.group = format!("{rel}  ·  {n}");
                        items.push(it);
                    }
                }
                return items;
            }
            PaletteMode::Files => {
                let project = self.active_project();
                let root = project.map(|x| x.path.clone()).unwrap_or_default();
                let changes = project.and_then(|x| x.git.clone());
                if let Some(pal) = &self.palette {
                    let mut changed: Vec<Item> = Vec::new();
                    let mut rest: Vec<Item> = Vec::new();
                    for f in &pal.files {
                        let full: PathBuf = root.join(f);
                        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let (icon, _) = icons::file_icon(&name, false, false, &p);
                        let mut it = Item::new(Lead::Icon(icon, None), name, Action::OpenFile(full.clone(), None));
                        it.detail = f.parent().map(|d| d.display().to_string()).unwrap_or_default();
                        it.mono_detail = true;
                        if let Some(id) = project.map(|x| x.id) {
                            it.preview = Preview::File(id, full.clone());
                        }
                        match changes.as_ref().and_then(|g| g.changes.get(&full)) {
                            Some(c) => {
                                it.right = Right::Letter(c.letter().to_string(), super::explorer_ui::change_color(*c, &p));
                                it.group = t("Changed by the agent").to_string();
                                changed.push(it);
                            }
                            None => {
                                it.group = t("Files").to_string();
                                rest.push(it);
                            }
                        }
                    }
                    items.extend(changed);
                    items.extend(rest);
                }
                return items;
            }
            PaletteMode::Sessions => return self.session_items(&[], t("Open now"), t("Earlier")),
            _ => {}
        }
        // What waits, first.
        let queue = if mode == PaletteMode::All { self.attention_queue() } else { Vec::new() };
        for (i, e) in queue.iter().enumerate() {
            let Some(x) = self.panes.get(&e.pane) else { continue };
            let project = self.project_label(x.project);
            let (letters, color) = self.project(x.project).map(|pr| (pr.letters(), pr.color32())).unwrap_or_default();
            let mut it = Item::new(Lead::Waiting(e.status, letters, color), x.name(), Action::FocusPane(e.pane));
            it.detail = format!("{project} · {}", super::sidebar::attention_short(x));
            it.mono_detail = true;
            it.preview = Preview::Pane(e.pane);
            it.right = Right::Wait(status::short_age(e.since), if i == 0 { kit::keys(&Action::NextAttention) } else { None });
            if e.status == Status::Done {
                it.review = Some(e.pane);
            }
            it.group = t("Waiting for you").to_string();
            items.push(it);
        }
        if mode == PaletteMode::All {
            let skip: Vec<PaneId> = queue.iter().map(|e| e.pane).collect();
            let sessions = t("Sessions");
            for mut it in self.session_items(&skip, sessions, sessions) {
                it.cap = SESSIONS_IN_ALL;
                items.push(it);
            }
        }
        if matches!(mode, PaletteMode::All | PaletteMode::Projects) {
            for pr in &self.projects {
                // A worktree is named with its project: "ternary-engine › generics".
                let mut it = Item::new(Lead::Tile(pr.letters(), pr.color32()), self.project_label(pr.id), Action::SelectProject(pr.id));
                it.detail = crate::paths::contract_tilde(&pr.path);
                it.mono_detail = true;
                it.preview = Preview::Project(pr.id);
                it.right = Right::Keys(self.top_index(pr.id).and_then(|i| kit::keys(&Action::SelectProjectIndex(i))).unwrap_or_default().to_string());
                it.pill = self.project_status(pr).map(|(s, _)| s);
                it.group = t("Projects").to_string();
                items.push(it);
            }
        }
        if mode == PaletteMode::All && !query.is_empty() {
            items.extend(self.all_files(query, &p).iter().cloned());
        }
        if mode == PaletteMode::All {
            let project = self.active.map(|id| self.project_label(id)).unwrap_or_default();
            for preset in self.presets() {
                let mut it = Item::new(Lead::Agent(preset.agent), preset.name.clone(), Action::RunPreset(preset.id.clone()));
                it.detail = preset.command.clone();
                it.mono_detail = true;
                it.right = Right::Word(self.hooks_note(preset.agent.unwrap_or(AgentKind::Custom)));
                it.group = tf("Run agent in {}", &[&project]);
                items.push(it);
            }
        }
        if matches!(mode, PaletteMode::All | PaletteMode::Commands) {
            // The editor's language commands, with a file open in it.
            let code = self.settings.lsp && self.active.and_then(|id| self.editors.get(&id)).and_then(|e| e.file()).is_some();
            let undo = self.lsp.ide.undo.is_some();
            let group = t("Commands").to_string();
            // Each with its shortcut, from the one table.
            let mut add = |icon, label: &str, action| {
                let mut it = Item::new(Lead::Icon(icon, None), label, action);
                it.right = Right::Keys(kit::keys(&it.action).unwrap_or_default().to_string());
                it.group = group.clone();
                items.push(it);
            };
            add(icons::GIT_DIFF, t("Review Changes…"), Action::Review);
            add(icons::CLOCK_COUNTER_CLOCKWISE, t("Commit History"), Action::History);
            add(icons::GIT_PULL_REQUEST, t("Commit…"), Action::Commit);
            // The branch's pull / merge request, once it is known.
            if let Some(s) = self.active.and_then(|id| self.ship.get(&id))
                && let Some(pr) = &s.pr
            {
                let is_gitlab = s
                    .facts
                    .as_ref()
                    .and_then(|f| f.remote.as_ref())
                    .and_then(|r| r.web.as_ref())
                    .is_some_and(|w| w.forge == tc_git::ship::Forge::GitLab);
                let label = if is_gitlab {
                    tf("Open Merge Request !{}", &[&pr.number.to_string()])
                } else {
                    tf("Open Pull Request #{}", &[&pr.number.to_string()])
                };
                add(icons::GIT_PULL_REQUEST, &label, Action::OpenPullRequest);
            }
            // The worktree in view: its own commands.
            if let Some((id, base)) = self.active_project().and_then(|x| Some((x.id, x.worktree.as_ref()?.base.clone()))) {
                add(icons::GIT_MERGE, &tf("Merge into {}…", &[&base]), Action::MergeWorktree(id));
                add(icons::TRASH, t("Remove Worktree…"), Action::RemoveWorktree(id));
            }
            add(icons::SQUARES_FOUR, t("Grid of All Projects"), Action::ToggleGrid);
            add(icons::TERMINAL_WINDOW, t("Focus View"), Action::SetView(tc_core::model::View::Focus));
            add(icons::BELL, t("Go to Next Waiting Agent"), Action::NextAttention);
            add(icons::ARROW_LEFT, t("Back to Where I Was"), Action::BackAttention);
            add(icons::TRAY, t("Show Attention List"), Action::ToggleAttention);
            add(icons::PLAY, t("Run Agent…"), Action::PickPreset);
            add(icons::FOLDER_PLUS, t("Add Project…"), Action::AddProject);
            add(icons::MAGNIFYING_GLASS, t("Find in Pane…"), Action::Find);
            add(icons::MAGNIFYING_GLASS, t("Find in All Panes…"), Action::FindAll);
            add(icons::FILE, t("Open File…"), Action::QuickOpen);
            add(icons::CODE, t("Show/Hide Code"), Action::ToggleEditor);
            add(icons::ROWS, t("Show/Hide Terminal Panel"), Action::ToggleDrawer);
            add(icons::ROWS, t("Dock Panel Bottom"), Action::SetDrawerDock(tc_core::model::DrawerDock::Bottom));
            add(icons::COLUMNS, t("Dock Panel Right"), Action::SetDrawerDock(tc_core::model::DrawerDock::Right));
            add(icons::USER, t("Accounts & Usage…"), Action::OpenProfiles);
            add(icons::DEVICE_MOBILE, t("Phone…"), Action::OpenPhonePairing);
            add(icons::SIDEBAR_SIMPLE, t("Show/Hide Files"), Action::ToggleFiles);
            add(icons::PLUS, t("New Tab"), Action::NewTab);
            add(icons::GLOBE, t("Browser"), Action::ShowBrowser);
            add(icons::GLOBE, t("New Browser Page"), Action::NewBrowserTab);
            add(icons::CLOCK_COUNTER_CLOCKWISE, t("Reopen Closed Page"), Action::ReopenClosedPage);
            add(icons::SQUARE_SPLIT_HORIZONTAL, t("Split Right"), Action::SplitRight);
            add(icons::SQUARE_SPLIT_VERTICAL, t("Split Down"), Action::SplitDown);
            add(icons::X, t("Close Pane"), Action::ClosePane);
            add(icons::ARROW_RIGHT, t("Show Next Tab"), Action::NextTab);
            add(icons::ARROW_LEFT, t("Show Previous Tab"), Action::PrevTab);
            add(icons::ARROW_RIGHT, t("Next Pane"), Action::NextPane);
            add(icons::ARROW_LEFT, t("Previous Pane"), Action::PrevPane);
            add(icons::TEXT_AA, t("Bigger Text"), Action::FontBigger);
            add(icons::TEXT_AA, t("Smaller Text"), Action::FontSmaller);
            add(icons::TEXT_AA, t("Actual Size"), Action::FontReset);
            add(icons::BROOM, t("Clear Scrollback"), Action::ClearScrollback);
            add(icons::SIDEBAR_SIMPLE, t("Show/Hide Sidebar"), Action::ToggleSidebar);
            add(icons::SIDEBAR_SIMPLE, t("Narrow Sidebar"), Action::ToggleCompactSidebar);
            add(icons::ARROWS_OUT, t("Zen Mode"), Action::ToggleZenMode);
            add(icons::PLUGS_CONNECTED, t("Agent Integrations…"), Action::Integrations);
            add(icons::GEAR_SIX, t("Settings…"), Action::Settings);
            add(icons::ARROW_CIRCLE_UP, t("Check for Updates…"), Action::CheckForUpdates);
            if code {
                use crate::lsp::{Goto, IdeCmd};
                add(icons::ARROW_SQUARE_OUT, t("Go to Definition"), Action::Ide(IdeCmd::Goto(Goto::Definition)));
                add(icons::ARROW_SQUARE_OUT, t("Go to Type Definition"), Action::Ide(IdeCmd::Goto(Goto::TypeDefinition)));
                add(icons::ARROW_SQUARE_OUT, t("Go to Implementation"), Action::Ide(IdeCmd::Goto(Goto::Implementation)));
                add(icons::LIST, t("Find References"), Action::Ide(IdeCmd::References));
                add(icons::PENCIL_SIMPLE, t("Rename Symbol"), Action::Ide(IdeCmd::Rename));
                add(icons::LIGHTNING, t("Code Actions…"), Action::Ide(IdeCmd::CodeActions));
                add(icons::TEXT_AA, t("Format Document"), Action::Ide(IdeCmd::Format));
                add(icons::LIST, t("Go to Symbol in File…"), Action::Ide(IdeCmd::Symbols));
                add(icons::MAGNIFYING_GLASS, t("Go to Symbol in Project…"), Action::Ide(IdeCmd::WorkspaceSymbols));
                add(icons::WARNING, t("Problems"), Action::Ide(IdeCmd::Problems));
                add(icons::ARROW_LEFT, t("Back"), Action::Ide(IdeCmd::Back));
                add(icons::ARROW_RIGHT, t("Forward"), Action::Ide(IdeCmd::Forward));
                add(icons::ARROWS_CLOCKWISE, t("Restart Language Server"), Action::Ide(IdeCmd::RestartServer));
            }
            if undo {
                add(icons::CLOCK_COUNTER_CLOCKWISE, t("Undo Refactor"), Action::Ide(crate::lsp::IdeCmd::UndoRefactor));
            }
        }
        items
    }

    /// How an agent tells TernaryCode it needs the user, short: "hooks · per launch".
    pub fn hooks_note(&self, agent: AgentKind) -> String {
        use tc_core::model::Integration;
        if self.installed(agent).is_some() {
            return t("hooks · installed").to_string();
        }
        let mode = self.settings.integrations.get(agent.slug()).copied().unwrap_or_default();
        match (agent, mode) {
            (AgentKind::Claude | AgentKind::Codex, Integration::PerLaunch) => t("hooks · per launch").to_string(),
            (AgentKind::Custom, _) => t("custom").to_string(),
            _ => t("no hooks").to_string(),
        }
    }

    /// Lists made fresh when the palette opens: every project's files (All's Files group) and its
    /// sessions on disk (both at most once a minute or so).
    fn palette_lists_fresh(&mut self, mode: PaletteMode) {
        let ids: Vec<(ProjectId, PathBuf)> = self.projects.iter().filter(|p| !p.is_worktree()).map(|p| (p.id, p.path.clone())).collect();
        for (id, path) in ids {
            if mode == PaletteMode::All {
                self.want_project_files(id);
            }
            let stale = self.sessions_scan_at.get(&id).is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(45));
            if stale && !self.sessions_running.contains(&id) {
                self.refresh_project_sessions(id, path);
            }
        }
    }

    /// A project's mini terminal (the preview's top): its avatar, name, branch and state over
    /// the last lines of `pane`; or, for a file, the file's first lines. Returns its bottom.
    #[allow(clippy::too_many_arguments)]
    fn paint_mini_sheet(&self, painter: &egui::Painter, rect: Rect, project: Option<&crate::project::Project>, pane: Option<&crate::pane::Pane>, file: Option<&std::path::Path>, p: &style::Palette, dark: bool) {
        let scheme = self.scheme(dark);
        painter.rect(rect, CornerRadius::same(12), p.canvas, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        let hcy = rect.top() + 18.0;
        let mut x = rect.left() + 10.0;
        let mut rx = rect.right() - 10.0;
        if let Some(x) = pane {
            let s = x.tracker.status;
            if !matches!(s, Status::Idle | Status::Ended) {
                let text = status::text(s);
                let w = kit::state_pill_width(painter, s, &text);
                let r = kit::paint_state_pill(painter, pos2(rx - w, hcy), s, &text, p, dark);
                rx = r.left() - 8.0;
            }
        }
        let mut job = egui::text::LayoutJob::default();
        match file {
            Some(f) => {
                let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let (icon, color) = icons::file_icon(&name, false, false, p);
                painter.text(pos2(x + 8.0, hcy), Align2::CENTER_CENTER, icon, style::icon_font(14.0), color);
                x += 16.0 + 8.0;
                job.append(&name, 0.0, egui::TextFormat::simple(style::medium(style::SMALL), p.text));
                if let Some(pr) = project {
                    job.append(&pr.name, 8.0, egui::TextFormat::simple(style::mono(style::DATA), p.text_faint));
                }
            }
            None => {
                if let Some(pr) = project {
                    kit::paint_project_tile(painter, Rect::from_center_size(pos2(x + 8.0, hcy), vec2(16.0, 16.0)), &pr.letters(), pr.color32(), dark);
                    x += 16.0 + 8.0;
                    job.append(&pr.name, 0.0, egui::TextFormat::simple(style::medium(style::SMALL), p.text));
                    if let Some(g) = &pr.git {
                        job.append(&g.branch, 8.0, egui::TextFormat::simple(style::mono(style::DATA), p.text_faint));
                    }
                }
            }
        }
        job.wrap = egui::text::TextWrapping::truncate_at_width((rx - x).max(20.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(x, hcy - g.size().y / 2.0), g, p.text);
        // The lines: Mono Small, 14 apart, under the header.
        let lines: Vec<String> = match (file, pane) {
            (Some(f), _) => file_head(f).as_ref().clone(),
            (None, Some(x)) => tc_term::core::tail_text(&x.shared, 12),
            _ => Vec::new(),
        };
        let tp = painter.with_clip_rect(rect.shrink(2.0).intersect(painter.clip_rect()));
        let room = ((rect.height() - 36.0 - 4.0 - 10.0) / 14.0).floor().max(0.0) as usize;
        let skip = lines.len().saturating_sub(room);
        let ink = if file.is_some() { p.text_muted } else { scheme.foreground };
        for (i, line) in lines.iter().skip(if file.is_some() { 0 } else { skip }).take(room).enumerate() {
            tp.text(pos2(rect.left() + 12.0, rect.top() + 36.0 + 4.0 + i as f32 * 14.0), Align2::LEFT_TOP, line, style::mono(style::DATA), ink);
        }
    }

    /// The palette's right column for the chosen row: a waiting agent's or a project's terminal
    /// (its last lines, in a mini sheet), a file's first lines, what it is, and Open. Returns
    /// whether Open was clicked.
    fn palette_preview(&mut self, ui: &mut egui::Ui, rect: Rect, item: &Item) -> bool {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius { nw: 0, ne: 0, sw: 0, se: 0 }, p.hover);
        let inner = rect.shrink(16.0);
        let mut y = inner.top();
        let mut rows: Vec<(String, String, bool)> = Vec::new();
        let mut open_label = t("Open");
        let pane_id = match &item.preview {
            Preview::Pane(id) => Some(*id),
            Preview::Project(id) => self.project_pane(*id),
            _ => None,
        };
        match &item.preview {
            Preview::Pane(_) | Preview::Project(_) => {
                let pane = pane_id.and_then(|id| self.panes.get(&id));
                let project = pane.and_then(|x| self.project(x.project)).or_else(|| if let Preview::Project(id) = item.preview { self.project(id) } else { None });
                let sheet = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), MINI_H));
                self.paint_mini_sheet(&painter, sheet, project, pane, None, &p, dark);
                y = sheet.bottom() + 16.0;
                if let Some(x) = pane {
                    if let Some(a) = x.tracker.agent {
                        let who = match &x.active_profile {
                            Some(prof) if crate::profiles::supports_profile(a) => format!("{} · {prof}", a.label()),
                            _ => a.label().to_string(),
                        };
                        rows.push((t("Agent").to_string(), who, false));
                    }
                    if let Some(m) = x.tracker.message.clone().filter(|_| x.tracker.status == Status::NeedsInput) {
                        let asks = m.split_once(": ").map_or(m.clone(), |(tool, what)| format!("{tool} · {what}"));
                        rows.push((t("Asks").to_string(), asks, true));
                    }
                    if x.tracker.wants_attention() {
                        rows.push((t("Waiting").to_string(), status::age(x.tracker.since), false));
                    }
                    rows.push((t("Pane").to_string(), x.label(), false));
                }
                if let Some(pr) = project {
                    rows.push((t("Folder").to_string(), crate::paths::contract_tilde(&pr.path), true));
                }
            }
            Preview::Session(id, s) => {
                // The agent's avatar and what the session was about.
                let tile = Rect::from_min_size(pos2(inner.left(), y), vec2(28.0, 28.0));
                kit::paint_agent_tile(&painter, tile, Some(s.agent), &p);
                let mut job = egui::text::LayoutJob::simple(s.title.clone(), style::semibold(style::HEADING), p.text, inner.width() - 28.0 - 12.0);
                job.wrap.max_rows = 3;
                let g = painter.layout_job(job);
                let gh = g.size().y;
                painter.galley(pos2(tile.right() + 12.0, y + 4.0), g, p.text);
                y += gh.max(28.0) + 16.0;
                rows.push((t("Agent").to_string(), match &s.profile_id {
                    Some(prof) => format!("{} · {prof}", s.agent.label()),
                    None => s.agent.label().to_string(),
                }, false));
                if let Some(pr) = self.project(*id) {
                    rows.push((t("Project").to_string(), pr.name.clone(), false));
                }
                if let Some(b) = &s.branch {
                    rows.push((t("Branch").to_string(), b.clone(), true));
                }
                rows.push((t("Updated").to_string(), tf("{} ago", &[&super::dialogs::ago(s.updated)]), false));
                rows.push((t("Session").to_string(), s.id.clone(), true));
                open_label = t("Resume");
            }
            Preview::File(id, path) => {
                let project = self.project(*id);
                let sheet = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), MINI_H));
                self.paint_mini_sheet(&painter, sheet, project, None, Some(path), &p, dark);
                y = sheet.bottom() + 16.0;
                if let Some(pr) = project {
                    rows.push((t("Project").to_string(), pr.name.clone(), false));
                    let dir = path.parent().unwrap_or(path);
                    rows.push((t("Folder").to_string(), crate::paths::contract_tilde(dir), true));
                }
            }
            Preview::None => {
                // A command, an agent to run, a place: what it is, and its keys.
                let mut job = egui::text::LayoutJob::simple(item.label.clone(), style::semibold(style::HEADING), p.text, inner.width());
                job.wrap.max_rows = 3;
                let g = painter.layout_job(job);
                let gh = g.size().y;
                painter.galley(pos2(inner.left(), y), g, p.text);
                y += gh + 8.0;
                if !item.detail.is_empty() {
                    let font = if item.mono_detail { style::mono(style::DATA) } else { egui::FontId::proportional(style::SMALL) };
                    let mut job = egui::text::LayoutJob::simple(item.detail.clone(), font, p.text_muted, inner.width());
                    job.wrap.max_rows = 4;
                    let g = painter.layout_job(job);
                    let h = g.size().y;
                    painter.galley(pos2(inner.left(), y), g, p.text_muted);
                    y += h + 8.0;
                }
                y += 8.0;
                if let Right::Keys(k) = &item.right
                    && !k.is_empty()
                {
                    painter.text(pos2(inner.left(), y + 9.0), Align2::LEFT_CENTER, t("Shortcut"), egui::FontId::proportional(style::SMALL), p.text_faint);
                    kit::paint_kbd_text(&painter, pos2(inner.left() + 104.0, y + 9.0), k, &p);
                }
                if let Right::Word(w) = &item.right {
                    painter.text(pos2(inner.left(), y + 9.0), Align2::LEFT_CENTER, w, egui::FontId::proportional(style::SMALL), p.text_faint);
                }
            }
        }
        // What it is: a quiet label and its value, 8 apart.
        for (k, v, mono) in rows {
            painter.text(pos2(inner.left(), y + 8.0), Align2::LEFT_CENTER, k, egui::FontId::proportional(style::SMALL), p.text_faint);
            let font = if mono { style::mono(style::MONO) } else { egui::FontId::proportional(style::SMALL) };
            let mut job = egui::text::LayoutJob::simple_singleline(v, font, p.text);
            job.wrap = egui::text::TextWrapping::truncate_at_width(inner.width() - 104.0);
            let g = painter.layout_job(job);
            painter.galley(pos2(inner.left() + 104.0, y + 8.0 - g.size().y / 2.0), g, p.text);
            y += 16.0 + 8.0;
        }
        // Open, at the bottom right.
        let bw = kit::button_width(ui, None, open_label, Some("↵"), false);
        let b = Rect::from_min_size(pos2(inner.right() - bw, inner.bottom() - style::CONTROL_H), vec2(bw, style::CONTROL_H));
        let r = ui.scope_builder(egui::UiBuilder::new().max_rect(b), |ui| kit::button_keys(ui, kit::Kind::Primary, None, open_label, Some("↵"), false)).inner;
        #[cfg(debug_assertions)]
        crate::devtest::record("palette-open", r.rect);
        r.clicked()
    }

    /// Changes the palette's scope, keeping what was typed and where focus came from.
    fn palette_scope(&mut self, mode: PaletteMode) {
        let prev = self.palette.as_ref().map(|p| (p.query.clone(), p.from_editor));
        self.open_palette(mode);
        if let Some(p) = self.palette.as_mut()
            && let Some((query, from_editor)) = prev
        {
            p.query = query;
            p.from_editor = from_editor;
        }
    }

    pub fn palette_ui(&mut self, ctx: &egui::Context) {
        // Search across panes: only when the query changed.
        if let Some(pal) = self.palette.as_mut()
            && pal.mode == PaletteMode::Search
            && pal.query != pal.searched
        {
            pal.searched = pal.query.clone();
            pal.results.clear();
            if pal.query.chars().count() >= 2 {
                let mut ids: Vec<_> = self.panes.keys().copied().collect();
                ids.sort_unstable();
                for id in ids {
                    let shared = self.panes[&id].shared.clone();
                    for (m, text) in tc_term::search::find_all(&shared, &pal.query, 5) {
                        pal.results.push((id, m, text));
                    }
                    if pal.results.len() >= 60 {
                        break;
                    }
                }
            }
        }
        let Some(pal_ref) = self.palette.as_ref() else { return };
        let mode = pal_ref.mode;
        if pal_ref.opened && matches!(mode, PaletteMode::All | PaletteMode::Sessions) {
            self.palette_lists_fresh(mode);
        }
        let Some(pal_ref) = self.palette.as_ref() else { return };
        let query = pal_ref.query.trim().to_string();
        // Quick Open lists up to 20 000 files: its rows are made and scored again only when the
        // query, the file list or their git letters change, not on every frame it is open.
        let files_key = (mode == PaletteMode::Files).then(|| {
            let git = self.active_project().and_then(|x| x.git.as_ref()).map_or(0, |g| std::sync::Arc::as_ptr(g) as usize);
            (pal_ref.query.clone(), self.active, self.files_rev, pal_ref.files.len(), git, self.ctx.global_style().visuals.dark_mode)
        });
        let kept = files_key.as_ref().and_then(|k| FILE_ROWS.with(|c| c.borrow().as_ref().filter(|(key, _)| key == k).map(|(_, rows)| Rc::clone(rows))));
        let all = if kept.is_some() { Vec::new() } else { self.palette_items(mode, &query) };
        let Some(mut pal) = self.palette.take() else { return };
        let typed = !query.is_empty() && mode != PaletteMode::Search;
        let items: Rc<Vec<(i32, Item)>> = match kept {
            Some(rows) => rows,
            None => {
                // Each row with its score and its group's place (All keeps its groups in order,
                // the best first within each).
                let mut groups: Vec<String> = Vec::new();
                let mut items: Vec<(i32, usize, Item)> = all
                    .into_iter()
                    .filter_map(|it| {
                        let gi = groups.iter().position(|g| *g == it.group).unwrap_or_else(|| {
                            groups.push(it.group.clone());
                            groups.len() - 1
                        });
                        if mode == PaletteMode::Search {
                            return Some((0, gi, it));
                        }
                        score(&query, &format!("{} {}", it.label, it.detail)).map(|s| (s, gi, it))
                    })
                    .collect();
                if typed {
                    if mode == PaletteMode::All {
                        items.sort_by_key(|x| (x.1, std::cmp::Reverse(x.0)));
                    } else {
                        items.sort_by_key(|x| std::cmp::Reverse(x.0));
                    }
                }
                if mode == PaletteMode::All {
                    let mut counts = vec![0usize; groups.len()];
                    items.retain(|(_, gi, it)| {
                        counts[*gi] += 1;
                        it.cap == 0 || counts[*gi] <= it.cap
                    });
                }
                items.truncate(MAX_ROWS);
                let items = Rc::new(items.into_iter().map(|(s, _, it)| (s, it)).collect::<Vec<_>>());
                if let Some(k) = files_key {
                    FILE_ROWS.with(|c| *c.borrow_mut() = Some((k, Rc::clone(&items))));
                }
                items
            }
        };
        // Group labels: always in All (its groups stay in order), and while nothing is typed
        // (always for places and search results).
        let grouped = mode == PaletteMode::All || !typed || matches!(mode, PaletteMode::Locations | PaletteMode::Search);
        pal.selected = pal.selected.min(items.len().saturating_sub(1));
        let mut chosen: Option<Action> = None;
        let mut scope: Option<PaletteMode> = None;
        let mut close = false;
        let title = format!("{} — {}", pal.title, t("type to filter"));
        let hint = match mode {
            PaletteMode::All => t("Search projects, sessions, files…"),
            PaletteMode::Projects => t("Switch to project…"),
            PaletteMode::Sessions => t("Open or resume which session?"),
            PaletteMode::Files => t("Open which file of this project?"),
            PaletteMode::Search => t("Search the output of every pane…"),
            PaletteMode::Commands => t("Type a command…"),
            PaletteMode::Locations => title.as_str(),
        };
        let enter_label = match mode {
            PaletteMode::Projects => t("switch"),
            PaletteMode::Search => t("show in pane"),
            PaletteMode::Locations => t("go to"),
            _ => t("open"),
        };
        let screen = ctx.content_rect();
        let wide = screen.width() >= 1200.0 && mode != PaletteMode::Locations;
        let width = (if wide { WIDE } else { WIDTH }).min((screen.width() - 24.0).max(280.0));
        let top = if screen.height() >= 700.0 { 90.0 } else { 24.0 };
        let height = HEIGHT.min(screen.height() - top - 24.0).max(240.0);
        let list_w = if wide { LIST_W.min(width - 300.0) } else { width };
        let scopes = mode != PaletteMode::Locations;
        let dark = ctx.global_style().visuals.dark_mode;
        let pp = style::palette(dark);
        let area = egui::Area::new(egui::Id::new("palette-area")).anchor(Align2::CENTER_TOP, vec2(0.0, top));
        let frame = egui::Frame::NONE
            .fill(pp.overlay)
            .stroke(Stroke::new(1.0, pp.border_strong))
            .corner_radius(CornerRadius::same(style::RADIUS_DIALOG))
            .shadow(egui::epaint::Shadow { offset: [0, 24], blur: 64, spread: 0, color: Color32::from_black_alpha(128) });
        let modal = egui::Modal::new(egui::Id::new("palette"))
            .area(area)
            .backdrop_color(if dark { Color32::from_black_alpha(166) } else { Color32::from_black_alpha(77) })
            .frame(frame)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let p = style::pal(ui);
                let (whole, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
                #[cfg(debug_assertions)]
                crate::devtest::record("palette", whole);
                let line = Stroke::new(1.0, p.border);

                // The field: the magnifier, what is typed (Title), esc.
                let row = Rect::from_min_size(whole.min, vec2(width, FIELD_H));
                ui.painter().text(pos2(row.left() + 18.0 + 9.0, row.center().y), Align2::CENTER_CENTER, icons::MAGNIFYING_GLASS, style::icon_font(18.0), p.text_muted);
                let esc_w = kit::kbd_width(ui.painter(), "esc");
                kit::paint_kbd_text(ui.painter(), pos2(row.right() - 18.0 - esc_w, row.center().y), "esc", &p);
                let field = Rect::from_min_max(pos2(row.left() + 18.0 + 18.0 + 10.0, row.top()), pos2(row.right() - 18.0 - esc_w - 10.0, row.bottom()));
                let was_empty = pal.query.is_empty();
                let r = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                        ui.visuals_mut().text_cursor.stroke = Stroke::new(2.0, p.accent_text);
                        ui.add_sized(
                            field.size(),
                            egui::TextEdit::singleline(&mut pal.query)
                                .hint_text(egui::RichText::new(hint).font(egui::FontId::proportional(style::TITLE)).color(p.text_faint))
                                .font(style::semibold(style::TITLE))
                                .text_color(p.text)
                                .frame(egui::Frame::NONE)
                                .margin(egui::Margin::ZERO)
                                .vertical_align(egui::Align::Center),
                        )
                    })
                    .inner;
                if pal.opened {
                    r.request_focus();
                    pal.opened = false;
                }
                // Backspace in an already-empty field leaves the scope; ">" in All goes to the
                // commands.
                if mode != PaletteMode::All && mode != PaletteMode::Locations && was_empty && pal.query.is_empty() && r.has_focus() && ui.input(|i| i.key_pressed(Key::Backspace)) {
                    scope = Some(PaletteMode::All);
                }
                if mode == PaletteMode::All && pal.query.starts_with('>') {
                    pal.query = pal.query[1..].trim_start().to_string();
                    scope = Some(PaletteMode::Commands);
                }
                // The selection is kept in view when the keyboard moves it (the wheel scrolls freely).
                let mut moved = r.changed();
                if r.changed() {
                    pal.selected = 0;
                }
                if r.has_focus() {
                    ui.memory_mut(|m| {
                        m.set_focus_lock_filter(
                            r.id,
                            egui::EventFilter {
                                horizontal_arrows: true,
                                vertical_arrows: true,
                                tab: true,
                                ..Default::default()
                            },
                        )
                    });
                }
                let (down, up, enter, back_tab, tab) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                        i.key_pressed(Key::Enter),
                        i.consume_key(egui::Modifiers::SHIFT, Key::Tab),
                        i.consume_key(egui::Modifiers::NONE, Key::Tab),
                    )
                });
                if down {
                    pal.selected = (pal.selected + 1).min(items.len().saturating_sub(1));
                }
                if up {
                    pal.selected = pal.selected.saturating_sub(1);
                }
                moved |= down || up;
                let mut y = row.bottom();
                ui.painter().hline(whole.x_range(), y + 0.5, line);
                y += 1.0;

                // The scopes (not for a list of places): chips, then "⇥ next scope".
                if scopes {
                    let i = SCOPES.iter().position(|m| *m == mode).unwrap_or(0);
                    if tab {
                        scope = Some(SCOPES[(i + 1) % SCOPES.len()]);
                    } else if back_tab {
                        scope = Some(SCOPES[(i + SCOPES.len() - 1) % SCOPES.len()]);
                    }
                    let chips = Rect::from_min_size(pos2(whole.left(), y), vec2(width, SCOPES_H));
                    let meta: Vec<_> = SCOPES
                        .into_iter()
                        .map(|m| {
                            let (label, keys) = match m {
                                PaletteMode::All => (t("All"), None),
                                PaletteMode::Projects => (t("Projects"), kit::keys(&Action::SwitchProject)),
                                PaletteMode::Sessions => (t("Sessions"), None),
                                PaletteMode::Files => (t("Files"), kit::keys(&Action::QuickOpen)),
                                PaletteMode::Search => (t("Output"), kit::keys(&Action::FindAll)),
                                _ => (t("Commands"), Some(">")),
                            };
                            let on = m == mode;
                            let font = if on { style::medium(style::SMALL) } else { egui::FontId::proportional(style::SMALL) };
                            let lg = ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
                            let kg = keys.map(|k| ui.painter().layout_no_wrap(k.to_string(), style::mono(style::DATA), p.text_faint));
                            (m, on, lg, kg)
                        })
                        .collect();
                    let next = ui.painter().layout_no_wrap(format!("⇥ {}", t("next scope")), egui::FontId::proportional(style::SMALL), p.text_faint);
                    let full_w: f32 = meta.iter().map(|(_, _, lg, kg)| 20.0 + lg.size().x + kg.as_ref().map_or(0.0, |g| g.size().x + 6.0) + 4.0).sum();
                    let show_keys = full_w + next.size().x + 24.0 + 16.0 <= chips.width();
                    let cp = ui.painter().with_clip_rect(chips.intersect(ui.clip_rect()));
                    let mut x = chips.left() + 12.0;
                    let cy = chips.center().y;
                    for (m, on, lg, kg) in meta {
                        let kg = kg.filter(|_| show_keys);
                        let w = 10.0 + lg.size().x + kg.as_ref().map_or(0.0, |g| g.size().x + 6.0) + 10.0;
                        let rect = Rect::from_min_size(pos2(x, cy - 13.0), vec2(w, 26.0));
                        let resp = ui.interact(rect.intersect(chips), egui::Id::new(("scope", m as u8)), Sense::click());
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("palette-scope:{m:?}"), rect);
                        let fill = if on { p.subtle } else if resp.hovered() { p.hover } else { Color32::TRANSPARENT };
                        cp.rect_filled(rect, CornerRadius::same(7), fill);
                        let lw = lg.size().x;
                        let lc = if on || resp.hovered() { p.text } else { p.text_muted };
                        cp.galley(pos2(rect.left() + 10.0, cy - lg.size().y / 2.0), lg, lc);
                        if let Some(g) = kg {
                            cp.galley(pos2(rect.left() + 10.0 + lw + 6.0, cy - g.size().y / 2.0), g, p.text_faint);
                        }
                        if resp.hovered() && !on {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() && !on {
                            scope = Some(m);
                        }
                        x = rect.right() + 4.0;
                    }
                    let hx = chips.right() - 12.0 - next.size().x;
                    if hx > x + 16.0 {
                        ui.painter().galley(pos2(hx, cy - next.size().y / 2.0), next, p.text_faint);
                    }
                    y = chips.bottom();
                    ui.painter().hline(whole.x_range(), y + 0.5, line);
                    y += 1.0;
                }

                // The list, and at its right (wide) the preview.
                let foot_top = whole.bottom() - FOOT_H - 1.0;
                let body = Rect::from_min_max(pos2(whole.left(), y), pos2(whole.right(), foot_top));
                let list = Rect::from_min_max(body.min, pos2(body.left() + list_w, body.bottom()));
                if items.is_empty() {
                    if pal.loading {
                        ui.painter().text(pos2(list.left() + 18.0, list.top() + 28.0), Align2::LEFT_CENTER, t("Listing the project's files…"), egui::FontId::proportional(style::BODY), p.text_muted);
                    } else {
                        let (icon, head, sub) = match mode {
                            _ if !query.is_empty() => (icons::MAGNIFYING_GLASS, tf("Nothing matches “{}”", &[&query]), t("Try fewer letters, or ⇥ to search another scope.").to_string()),
                            PaletteMode::Search => (icons::MAGNIFYING_GLASS, t("Search the output of every pane…").to_string(), String::new()),
                            PaletteMode::Sessions => (icons::CLOCK_COUNTER_CLOCKWISE, t("No sessions yet").to_string(), t("The agents you run show up here, and the sessions you can resume.").to_string()),
                            _ => (icons::MAGNIFYING_GLASS, t("Nothing matches.").to_string(), String::new()),
                        };
                        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
                            kit::empty_state_at(ui, list, icon, &head, &sub, None);
                        });
                    }
                }
                ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
                    ui.set_clip_rect(list.intersect(ui.clip_rect()));
                    egui::ScrollArea::vertical().id_salt("palette-list").max_height(list.height()).auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        ui.add_space(LIST_PAD);
                        let row_w = list_w - 2.0 * LIST_PAD;
                        let mut group = String::new();
                        for (i, (_, item)) in items.iter().enumerate() {
                            if grouped && item.group != group && !item.group.is_empty() {
                                group = item.group.clone();
                                let (gr, _) = ui.allocate_exact_size(vec2(list_w, GROUP_H), Sense::hover());
                                if ui.is_rect_visible(gr) {
                                    let g = kit::section_galley(ui.painter(), &group, p.text_faint);
                                    ui.painter().galley(pos2(gr.left() + LIST_PAD, gr.center().y - g.size().y / 2.0), g, p.text_faint);
                                }
                                ui.add_space(ROW_GAP);
                            }
                            let (full, _) = ui.allocate_exact_size(vec2(list_w, ROW_H), Sense::hover());
                            ui.add_space(ROW_GAP);
                            let rect = Rect::from_min_size(pos2(full.left() + LIST_PAD, full.top()), vec2(row_w, ROW_H));
                            let resp = ui.interact(rect, egui::Id::new(("pal-row", i)), Sense::click());
                            let selected = i == pal.selected;
                            if selected && moved {
                                ui.scroll_to_rect(full.expand2(vec2(0.0, GROUP_H)), None);
                            }
                            if !ui.is_rect_visible(rect) {
                                if resp.clicked() || (enter && selected) {
                                    chosen = Some(item.action.clone());
                                }
                                continue;
                            }
                            #[cfg(debug_assertions)]
                            crate::devtest::record(format!("palette-row:{i}"), rect);
                            if selected {
                                ui.painter().rect(rect, CornerRadius::same(8), p.surface, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, CornerRadius::same(8), p.hover);
                            }
                            if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                                pal.selected = i;
                            }
                            let cy = rect.center().y;
                            // The lead: an icon, a project's avatar, a status and an avatar, an
                            // agent's avatar.
                            let mut x = rect.left() + 10.0;
                            match &item.lead {
                                Lead::Icon(icon, c) => {
                                    ui.painter().text(pos2(x + 8.0, cy), Align2::CENTER_CENTER, *icon, style::icon_font(16.0), c.unwrap_or(p.text_muted));
                                    x += 16.0;
                                }
                                Lead::Tile(letters, c) => {
                                    kit::paint_project_tile(ui.painter(), Rect::from_min_size(pos2(x, cy - 10.0), vec2(20.0, 20.0)), letters, *c, dark);
                                    x += 20.0;
                                }
                                Lead::Waiting(st, letters, c) => {
                                    status::paint_glyph(ui.painter(), pos2(x + 7.0, cy), *st, &p, 14.0);
                                    x += 14.0 + 6.0;
                                    kit::paint_project_tile(ui.painter(), Rect::from_min_size(pos2(x, cy - 8.0), vec2(16.0, 16.0)), letters, *c, dark);
                                    x += 16.0;
                                }
                                Lead::Agent(a) => {
                                    kit::paint_agent_tile(ui.painter(), Rect::from_min_size(pos2(x, cy - 10.0), vec2(20.0, 20.0)), *a, &p);
                                    x += 20.0;
                                }
                            }
                            x += 10.0;
                            // Right side first: a Review button, what is at the right, a pill.
                            let mut right = rect.right() - 10.0;
                            let mut reviewed = false;
                            if let Some(pane) = item.review {
                                let w = kit::button_width(ui, None, t("Review"), None, true);
                                let br = Rect::from_center_size(pos2(right - w / 2.0, cy), vec2(w, 24.0));
                                right -= w + 8.0;
                                if ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| kit::button_ex(ui, kit::Kind::Secondary, None, t("Review"), true)).inner.clicked() {
                                    self.actions.push(Action::FocusPane(pane));
                                    self.actions.push(Action::ReviewAgent);
                                    reviewed = true;
                                }
                            }
                            let quiet = |painter: &egui::Painter, right: &mut f32, text: &str, font: egui::FontId, c: Color32| {
                                let g = painter.layout_no_wrap(text.to_string(), font, c);
                                *right -= g.size().x;
                                painter.galley(pos2(*right, cy - g.size().y / 2.0), g, c);
                                *right -= 10.0;
                            };
                            match &item.right {
                                Right::None => {}
                                Right::Keys(k) if k.is_empty() => {}
                                Right::Keys(k) => quiet(ui.painter(), &mut right, k, style::mono(style::DATA), p.text_faint),
                                Right::Word(w) => quiet(ui.painter(), &mut right, w, egui::FontId::proportional(style::SMALL), p.text_faint),
                                Right::Letter(l, c) => quiet(ui.painter(), &mut right, l, style::mono_semibold(style::DATA), *c),
                                Right::Wait(age, keys) => {
                                    if let Some(k) = keys {
                                        let w = kit::kbd_width(ui.painter(), k);
                                        kit::paint_kbd_text(ui.painter(), pos2(right - w, cy), k, &p);
                                        right -= w + 6.0;
                                    }
                                    quiet(ui.painter(), &mut right, age, style::mono(style::DATA), p.text_faint);
                                }
                                Right::Live(age, s) => {
                                    status::paint_glyph(ui.painter(), pos2(right - 6.0, cy), *s, &p, 12.0);
                                    right -= 12.0 + 7.0;
                                    quiet(ui.painter(), &mut right, age, style::mono(style::DATA), p.text_faint);
                                }
                            }
                            if let Some(s) = item.pill {
                                let text = status::text(s);
                                let w = kit::state_pill_width(ui.painter(), s, &text);
                                kit::paint_state_pill(ui.painter(), pos2(right - w, cy), s, &text, &p, dark);
                                right -= w + 10.0;
                            }
                            // The label (Body; Body Strong when chosen), then the detail.
                            let room = (right - x).max(20.0);
                            let font = if item.mono {
                                style::mono(style::MONO)
                            } else if selected {
                                style::medium(style::BODY)
                            } else {
                                egui::FontId::proportional(style::BODY)
                            };
                            let mut job = egui::text::LayoutJob::default();
                            job.append(&item.label, 0.0, egui::TextFormat::simple(font, p.text));
                            if !item.detail.is_empty() {
                                let font = if item.mono_detail { style::mono(style::DATA) } else { egui::FontId::proportional(style::SMALL) };
                                job.append(&item.detail, 10.0, egui::TextFormat::simple(font, p.text_faint));
                            }
                            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                            let g = ui.painter().layout_job(job);
                            ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
                            if !reviewed && (resp.clicked() || (enter && selected)) {
                                chosen = Some(item.action.clone());
                            } else if reviewed {
                                close = true;
                            }
                        }
                        ui.add_space(LIST_PAD - ROW_GAP);
                    });
                });
                if wide {
                    ui.painter().vline(list.right() + 0.5, body.y_range(), line);
                    let pr = Rect::from_min_max(pos2(list.right() + 1.0, body.top()), body.max);
                    let item = items.get(pal.selected).map(|(_, it)| it.clone());
                    match item {
                        Some(item) => {
                            if self.palette_preview(ui, pr, &item) {
                                chosen = Some(item.action.clone());
                            }
                        }
                        None => {
                            ui.painter().rect_filled(pr, CornerRadius::ZERO, p.hover);
                        }
                    }
                }

                // The keys.
                ui.painter().hline(whole.x_range(), foot_top + 0.5, line);
                let foot = Rect::from_min_max(pos2(whole.left() + 16.0, foot_top + 1.0), pos2(whole.right() - 16.0, whole.bottom()));
                let mut hints: Vec<(&str, &str)> = vec![("↑↓", t("choose")), ("↵", enter_label)];
                if scopes {
                    hints.push(("⇥", t("scope")));
                }
                kit::paint_hints(ui.painter(), pos2(foot.left(), foot.center().y), &hints, &p);
                let esc = [("esc", t("close"))];
                let ew = kit::hints_width(ui.painter(), &esc);
                kit::paint_hints(ui.painter(), pos2(foot.right() - ew, foot.center().y), &esc, &p);
            });
        if modal.should_close() {
            close = true;
        }
        if let Some(m) = scope {
            self.palette = Some(pal);
            self.palette_scope(m);
            return;
        }
        if let Some(a) = chosen {
            self.actions.push(a);
            close = true;
        }
        if close {
            // Back where the keyboard was: the editor (an IDE command came from there) or the
            // terminal.
            if pal.from_editor {
                if let Some(e) = self.active.and_then(|id| self.editors.get_mut(&id)) {
                    e.refocus = true;
                }
            } else {
                // In the Grid, the tile typed into.
                self.focus_request = self.target_pane().or_else(|| self.current_pane());
            }
        } else {
            self.palette = Some(pal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{matched, score};

    #[test]
    fn fuzzy_scores_prefer_compact_early_matches() {
        assert!(score("cla", "Claude Code").unwrap() > score("cla", "OpenCode claw").unwrap_or(i32::MIN));
        assert!(score("xyz", "Claude").is_none());
        assert_eq!(score("", "anything"), Some(0));
        assert!(score("cc", "Claude Code").is_some());
    }

    #[test]
    fn letters_beyond_ascii_count_once() {
        // "é" is two bytes: the "f" after it is still the next letter, and the text two long.
        assert_eq!(score("éf", "éf"), score("ab", "ab"));
    }

    #[test]
    fn the_matched_letters_are_marked() {
        assert_eq!(matched("pay", "payments-api")[..4], [true, true, true, false]);
        assert!(matched("zz", "payments").iter().all(|m| !m));
    }
}
