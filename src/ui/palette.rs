//! The command palette (⌘K): one list for everything, in scopes (projects ⌘P, agents ⇧⌘A in its
//! list form, files ⌘O, the output of every pane ⇧⌘F, commands) that ⇥ moves between, and the
//! language server's lists of places (references, symbols, problems). With nothing typed, what
//! waits for the user comes first.

use std::path::PathBuf;

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Rect, Sense, Stroke, pos2, vec2};
use tc_core::status::Status;

use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::{App, PaletteMode};
use crate::i18n::{t, tf};

/// Results kept for the list (the rest by typing more).
const MAX_ROWS: usize = 200;
const ROW_H: f32 = 38.0;
const GROUP_H: f32 = 28.0;
const WIDTH: f32 = 640.0;
/// With room for it, the palette is wider and shows the chosen row at its right.
const WIDE: f32 = 940.0;
const PREVIEW_W: f32 = 360.0;
/// The scopes, in the order ⇥ goes through them.
const SCOPES: [PaletteMode; 6] =
    [PaletteMode::All, PaletteMode::Projects, PaletteMode::Presets, PaletteMode::Files, PaletteMode::Search, PaletteMode::Commands];

/// What sits before a row's label.
#[derive(Clone)]
enum Lead {
    Icon(&'static str, Option<Color32>),
    /// A project's tile (its letters and colour).
    Tile(String, Color32),
    /// Something waiting: its status glyph and its project's tile.
    Waiting(Status, String, Color32),
    /// An agent preset: play, then its logo.
    Agent(Option<tc_core::agent::AgentKind>),
}

/// What the preview at the palette's right shows for a row.
#[derive(Clone, Copy, PartialEq)]
enum Preview {
    None,
    Pane(tc_core::PaneId),
    Project(tc_core::ProjectId),
}

#[derive(Clone)]
struct Item {
    lead: Lead,
    label: String,
    /// The label is code (a line of a file or of a terminal): monospace.
    mono: bool,
    detail: String,
    /// At the right: a status pill, then quiet text (a shortcut, a line number…).
    pill: Option<(super::sidebar::RowMark, String, Color32)>,
    right: String,
    right_color: Option<Color32>,
    /// A Review button at the right (an agent that finished).
    review: Option<tc_core::PaneId>,
    /// The group it is listed under while nothing is typed.
    group: String,
    /// The agent whose logo leads the detail.
    agent: Option<tc_core::agent::AgentKind>,
    /// The detail is code (a path, a command): in the data font.
    mono_detail: bool,
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
            pill: None,
            right: String::new(),
            right_color: None,
            review: None,
            group: String::new(),
            agent: None,
            mono_detail: false,
            preview: Preview::None,
            action,
        }
    }
}

/// Quick Open's rows: what they were made for (query, project, file list, how many files, git
/// letters, theme) and the best of them.
type FileRowsKey = (String, Option<tc_core::ProjectId>, u64, usize, usize, bool);
type FileRows = Option<(FileRowsKey, std::rc::Rc<Vec<(i32, Item)>>)>;

thread_local! {
    static FILE_ROWS: std::cell::RefCell<FileRows> = const { std::cell::RefCell::new(None) };
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

impl App {
    fn palette_items(&self, mode: PaletteMode) -> Vec<Item> {
        let p = style::palette(self.ctx.global_style().visuals.dark_mode);
        let mut items = Vec::new();
        match mode {
            PaletteMode::Search => {
                if let Some(pal) = &self.palette {
                    for (pane, m, text) in &pal.results {
                        let Some(x) = self.panes.get(pane) else { continue };
                        let project = self.project_label(x.project);
                        let n = pal.results.iter().filter(|r| r.0 == *pane).count();
                        let mut it = Item::new(Lead::Icon(icons::TERMINAL, Some(p.text_faint)), text.trim_end(), Action::RevealMatch(*pane, m.clone()));
                        it.mono = true;
                        it.group = format!("{project} · {}  —  {}", x.label(), crate::i18n::count(n, t("1 match"), t("{} matches")));
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
                        let lead = if l.error { Lead::Icon(icons::X_CIRCLE, Some(p.err)) } else { Lead::Icon(icons::FILE_CODE, Some(p.text_muted)) };
                        let mut it = Item::new(lead, l.label.clone(), Action::GotoLocation(l.path.clone(), l.line, l.col));
                        it.mono = !l.error;
                        it.right = (l.line + 1).to_string();
                        if l.error {
                            it.detail = l.detail.clone();
                            it.right.clear();
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
                        let (icon, color) = icons::file_icon(&name, false, false, &p);
                        let mut it = Item::new(Lead::Icon(icon, Some(color)), name, Action::OpenFile(full.clone(), None));
                        it.detail = f.parent().map(|d| d.display().to_string()).unwrap_or_default();
                        it.mono_detail = true;
                        match changes.as_ref().and_then(|g| g.changes.get(&full)) {
                            Some(c) => {
                                it.right = c.letter().to_string();
                                it.right_color = Some(super::explorer_ui::change_color(*c, &p));
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
            _ => {}
        }
        // What waits, first (with nothing typed, its own group at the top).
        if mode == PaletteMode::All {
            for (i, e) in self.attention_queue().iter().enumerate() {
                let Some(x) = self.panes.get(&e.pane) else { continue };
                let project = self.project_label(x.project);
                let (letters, color) = self.project(x.project).map(|pr| (pr.letters(), pr.color32())).unwrap_or_default();
                let mut it = Item::new(Lead::Waiting(e.status, letters, color), project, Action::FocusPane(e.pane));
                it.detail = super::sidebar::attention_short(x);
                it.agent = x.tracker.agent.filter(|a| super::agent_logo::has(*a));
                it.preview = Preview::Pane(e.pane);
                it.right = status::short_age(e.since);
                if i == 0 {
                    it.right = format!("{}  {}", it.right, kit::keys(&Action::NextAttention).unwrap_or_default());
                }
                if e.status == Status::Done {
                    it.review = Some(e.pane);
                }
                it.group = t("Waiting for you").to_string();
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
                it.right = self.top_index(pr.id).and_then(|i| kit::keys(&Action::SelectProjectIndex(i))).unwrap_or_default().to_string();
                let row = self.project_row(pr, &p);
                it.pill = row.chip;
                it.group = t("Projects").to_string();
                items.push(it);
            }
        }
        if matches!(mode, PaletteMode::All | PaletteMode::Presets) {
            let project = self.active.map(|id| self.project_label(id)).unwrap_or_default();
            for preset in self.presets() {
                let mut it = Item::new(Lead::Agent(preset.agent), preset.name.clone(), Action::RunPreset(preset.id.clone()));
                it.detail = preset.command.clone();
                it.mono_detail = true;
                it.right = self.hooks_note(preset.agent.unwrap_or(tc_core::agent::AgentKind::Custom));
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
                it.right = kit::keys(&it.action).unwrap_or_default().to_string();
                it.group = group.clone();
                items.push(it);
            };
            add(icons::GIT_DIFF, t("Review Changes…"), Action::Review);
            add(icons::GIT_PULL_REQUEST, t("Commit…"), Action::Commit);
            // The branch's pull request, once it is known.
            if let Some(n) = self.active.and_then(|id| self.ship.get(&id)).and_then(|s| s.pr.as_ref()).map(|pr| pr.number) {
                add(icons::GIT_PULL_REQUEST, &tf("Open Pull Request #{}", &[&n.to_string()]), Action::OpenPullRequest);
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
    pub fn hooks_note(&self, agent: tc_core::agent::AgentKind) -> String {
        use tc_core::agent::AgentKind;
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

    /// The palette's right column for the chosen row: a waiting agent's or a project's terminal
    /// (its last lines, as a small tile), what it is, and Open; for other rows what they are.
    /// Returns whether Open was clicked.
    #[allow(clippy::too_many_arguments)]
    fn palette_preview(&mut self, ui: &mut egui::Ui, rect: Rect, preview: Preview, label: &str, detail: &str, right: &str, action: &Action) -> bool {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, if dark { style::over(p.overlay, p.chrome, 0.3) } else { Color32::from_rgb(0xfa, 0xfa, 0xfa) });
        painter.vline(rect.left(), rect.y_range(), Stroke::new(1.0, p.border));
        let inner = rect.shrink2(vec2(16.0, 14.0));
        let mut y = inner.top();
        // The pane it is about.
        let pane_id = match preview {
            Preview::Pane(id) => Some(id),
            Preview::Project(id) => self.project_pane(id),
            Preview::None => None,
        };
        let pane = pane_id.and_then(|id| self.panes.get(&id));
        let project = pane.and_then(|x| self.project(x.project)).or_else(|| if let Preview::Project(id) = preview { self.project(id) } else { None });
        if let Some(pr) = project {
            // A small tile: its header, then the terminal's last lines.
            let tile = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 196.0));
            let scheme = self.scheme(dark);
            painter.rect(tile, CornerRadius::same(9), scheme.background, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
            let hcy = tile.top() + 16.0;
            kit::paint_project_tile(&painter, Rect::from_min_size(pos2(tile.left() + 10.0, hcy - 9.0), vec2(18.0, 18.0)), &pr.letters(), pr.color32(), dark);
            let g = painter.layout_no_wrap(pr.name.clone(), style::semibold(13.0), p.text);
            let nw = g.size().x;
            painter.galley(pos2(tile.left() + 10.0 + 18.0 + 8.0, hcy - g.size().y / 2.0), g, p.text);
            let mut rx = tile.right() - 8.0;
            if let Some(x) = pane {
                let s = x.tracker.status;
                if !matches!(s, Status::Idle | Status::Ended) {
                    let text = if matches!(s, Status::Running | Status::Unknown) { x.label() } else { status::text(s) };
                    let w = kit::state_pill_width(&painter, s, &text);
                    let r = kit::paint_state_pill(&painter, pos2(rx - w, hcy), s, &text, &p, dark);
                    rx = r.left() - 8.0;
                }
            }
            if let Some(g) = &pr.git {
                let bx = tile.left() + 10.0 + 18.0 + 8.0 + nw + 8.0;
                if rx - bx > 40.0 {
                    painter.text(pos2(bx + 5.5, hcy), Align2::CENTER_CENTER, icons::GIT_BRANCH, style::icon_font(11.0), p.text_faint);
                    let mut job = egui::text::LayoutJob::simple_singleline(g.branch.clone(), style::mono(10.5), p.text_faint);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(rx - bx - 16.0);
                    let bg = painter.layout_job(job);
                    painter.galley(pos2(bx + 15.0, hcy - bg.size().y / 2.0), bg, p.text_faint);
                }
            }
            if let Some(x) = pane {
                let lines = tc_term::core::tail_text(&x.shared, 11);
                let tp = painter.with_clip_rect(tile.shrink(2.0));
                for (i, line) in lines.iter().enumerate() {
                    tp.text(pos2(tile.left() + 12.0, tile.top() + 34.0 + i as f32 * 14.0), Align2::LEFT_TOP, line, style::mono(9.5), scheme.foreground);
                }
            }
            y = tile.bottom() + 14.0;
            // What it is.
            let mut rows: Vec<(&str, String)> = Vec::new();
            if let Some(x) = pane {
                if let Some(a) = x.tracker.agent {
                    rows.push((t("Agent"), a.label().to_string()));
                }
                if let Some(m) = x.tracker.message.clone().filter(|_| x.tracker.status == Status::NeedsInput) {
                    rows.push((t("Asks"), m.split_once(": ").map_or(m.clone(), |(_, w)| w.to_string())));
                }
                if x.tracker.wants_attention() {
                    rows.push((t("Waiting"), status::age(x.tracker.since)));
                }
                rows.push((t("Pane"), x.label()));
            }
            rows.push((t("Folder"), crate::paths::contract_tilde(&pr.path)));
            for (k, v) in rows {
                painter.text(pos2(inner.left(), y + 8.0), Align2::LEFT_CENTER, k, egui::FontId::proportional(12.0), p.text_faint);
                let mono = matches!(k, k if k == t("Asks") || k == t("Folder"));
                let mut job = egui::text::LayoutJob::simple_singleline(v, if mono { style::mono(11.0) } else { egui::FontId::proportional(12.0) }, p.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width(inner.width() - 84.0);
                let g = painter.layout_job(job);
                painter.galley(pos2(inner.left() + 84.0, y + 8.0 - g.size().y / 2.0), g, p.text);
                y += 22.0;
            }
        } else {
            // A command, an agent, a file: what it is, and its keys.
            painter.text(pos2(inner.left(), y), Align2::LEFT_TOP, label, style::semibold(13.0), p.text);
            y += 24.0;
            if !detail.is_empty() {
                let mut job = egui::text::LayoutJob::simple(detail.to_string(), style::mono(11.0), p.text_muted, inner.width());
                job.wrap.max_rows = 4;
                let g = painter.layout_job(job);
                let h = g.size().y;
                painter.galley(pos2(inner.left(), y), g, p.text_muted);
                y += h + 10.0;
            }
            if !right.is_empty() {
                painter.text(pos2(inner.left(), y), Align2::LEFT_TOP, right, style::mono(11.0), p.text_faint);
            }
        }
        // Open, at the bottom.
        let b = Rect::from_min_size(pos2(inner.left(), inner.bottom() - 24.0), vec2(kit::button_width(ui, None, t("Open"), Some("↵"), true), 24.0));
        let _ = action;
        ui.scope_builder(egui::UiBuilder::new().max_rect(b), |ui| kit::button_keys(ui, kit::Kind::Primary, None, t("Open"), Some("↵"), true)).inner.clicked()
    }

    /// Changes the palette's scope, keeping what was typed.
    fn palette_scope(&mut self, mode: PaletteMode) {
        let query = self.palette.as_ref().map(|p| p.query.clone()).unwrap_or_default();
        self.open_palette(mode);
        if let Some(p) = self.palette.as_mut() {
            p.query = query;
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
        // Quick Open lists up to 20 000 files: its rows are made and scored again only when the
        // query, the file list or their git letters change, not on every frame it is open.
        let files_key = (pal_ref.mode == PaletteMode::Files).then(|| {
            let git = self.active_project().and_then(|x| x.git.as_ref()).map_or(0, |g| std::sync::Arc::as_ptr(g) as usize);
            (pal_ref.query.clone(), self.active, self.files_rev, pal_ref.files.len(), git, self.ctx.global_style().visuals.dark_mode)
        });
        let kept = files_key.as_ref().and_then(|k| FILE_ROWS.with(|c| c.borrow().as_ref().filter(|(key, _)| key == k).map(|(_, rows)| std::rc::Rc::clone(rows))));
        let all = if kept.is_some() { Vec::new() } else { self.palette_items(pal_ref.mode) };
        let Some(mut pal) = self.palette.take() else { return };
        let typed = !pal.query.trim().is_empty() && pal.mode != PaletteMode::Search;
        let items: std::rc::Rc<Vec<(i32, Item)>> = match kept {
            Some(rows) => rows,
            None => {
                let mut items: Vec<(i32, Item)> = all
                    .into_iter()
                    .filter_map(|it| {
                        if pal.mode == PaletteMode::Search {
                            return Some((0, it));
                        }
                        score(&pal.query, &format!("{} {}", it.label, it.detail)).map(|s| (s, it))
                    })
                    .collect();
                if typed {
                    items.sort_by_key(|x| std::cmp::Reverse(x.0));
                }
                items.truncate(MAX_ROWS);
                let items = std::rc::Rc::new(items);
                if let Some(k) = files_key {
                    FILE_ROWS.with(|c| *c.borrow_mut() = Some((k, std::rc::Rc::clone(&items))));
                }
                items
            }
        };
        // Group headers while nothing is typed (and always for places and search results).
        let grouped = !typed || matches!(pal.mode, PaletteMode::Locations | PaletteMode::Search);
        pal.selected = pal.selected.min(items.len().saturating_sub(1));
        let mut chosen: Option<Action> = None;
        let mut scope: Option<PaletteMode> = None;
        let mut close = false;
        let title = format!("{} — {}", pal.title, t("type to filter"));
        let hint = match pal.mode {
            PaletteMode::All => t("Type a command, project, agent or file…"),
            PaletteMode::Projects => t("Switch to project…"),
            PaletteMode::Presets => t("Start which agent?"),
            PaletteMode::Files => t("Open which file of this project?"),
            PaletteMode::Search => t("Search the output of every pane…"),
            PaletteMode::Commands => t("Type a command…"),
            PaletteMode::Locations => title.as_str(),
        };
        let enter_label = match pal.mode {
            PaletteMode::Projects => t("switch"),
            PaletteMode::Search => t("show in pane"),
            PaletteMode::Locations => t("go to"),
            PaletteMode::Presets => t("run"),
            _ => t("open"),
        };
        // Wide enough: the chosen row is shown at the right (what a waiting agent asks, a
        // project's terminal).
        let wide = ctx.content_rect().width() >= 1200.0 && pal.mode != PaletteMode::Locations;
        let width = if wide { WIDE } else { WIDTH };
        let list_w = if wide { WIDE - PREVIEW_W } else { WIDTH };
        let area = egui::Area::new(egui::Id::new("palette-area")).anchor(Align2::CENTER_TOP, vec2(0.0, 70.0));
        let dark = ctx.global_style().visuals.dark_mode;
        let modal = egui::Modal::new(egui::Id::new("palette"))
            .area(area)
            .backdrop_color(Color32::from_black_alpha(if dark { 90 } else { 40 }))
            .frame(super::dialogs::frame(ctx).inner_margin(egui::Margin::ZERO))
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let p = style::pal(ui);
                let top = ui.max_rect();
                kit::paint_highlight(ui.painter(), Rect::from_min_size(top.min, vec2(width, 1.0)), style::RADIUS_DIALOG, &p);
                // The field: the magnifier, the scope's token (when the palette was opened in a
                // scope), what is typed, ⌘K.
                let (row, _) = ui.allocate_exact_size(vec2(width, 54.0), Sense::hover());
                ui.painter().text(pos2(row.left() + 18.0 + 9.0, row.center().y), Align2::CENTER_CENTER, icons::MAGNIFYING_GLASS, style::icon_font(18.0), p.text_faint);
                let mut fx = row.left() + 18.0 + 18.0 + 10.0;
                let token = match pal.mode {
                    PaletteMode::Projects => Some((icons::FOLDER_SIMPLE, t("Projects"))),
                    PaletteMode::Presets => Some((icons::PLAY, t("Agents"))),
                    PaletteMode::Files => Some((icons::FILE, t("Files"))),
                    PaletteMode::Search => Some((icons::TERMINAL_WINDOW, t("Output"))),
                    PaletteMode::Commands => Some((icons::COMMAND, t("Commands"))),
                    _ => None,
                };
                if let Some((icon, label)) = token {
                    let g = ui.painter().layout_no_wrap(label.to_string(), style::medium(12.0), p.accent_text);
                    let tr = Rect::from_min_size(pos2(fx, row.center().y - 12.0), vec2(8.0 + 13.0 + 6.0 + g.size().x + 8.0, 24.0));
                    ui.painter().rect(tr, CornerRadius::same(6), p.accent_tint, Stroke::new(1.0, style::soft(p.accent, 0.3)), egui::StrokeKind::Inside);
                    ui.painter().text(pos2(tr.left() + 8.0 + 6.5, tr.center().y), Align2::CENTER_CENTER, icon, style::icon_font(13.0), p.accent_text);
                    ui.painter().galley(pos2(tr.left() + 8.0 + 13.0 + 6.0, tr.center().y - g.size().y / 2.0), g, p.accent_text);
                    fx = tr.right() + 8.0;
                }
                let kbd = kit::keys(&Action::Palette).unwrap_or_default();
                let kw = if kbd.is_empty() || token.is_some() { 0.0 } else { kit::kbd_width(ui.painter(), kbd) + 12.0 };
                if kw > 0.0 {
                    kit::paint_kbd_text(ui.painter(), pos2(row.right() - 16.0 - kw + 12.0, row.center().y), kbd, &p);
                }
                let field = Rect::from_min_max(pos2(fx, row.top()), pos2(row.right() - 16.0 - kw, row.bottom()));
                let r = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                        ui.add_sized(
                            field.size(),
                            egui::TextEdit::singleline(&mut pal.query)
                                .hint_text(egui::RichText::new(hint).color(p.text_faint))
                                .font(egui::FontId::proportional(15.0))
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
                // Backspace in an empty field leaves the scope.
                if token.is_some() && pal.query.is_empty() && r.has_focus() && ui.input(|i| i.key_pressed(Key::Backspace)) {
                    scope = Some(PaletteMode::All);
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
                let (down, up, enter, tab, back_tab) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                        i.key_pressed(Key::Enter),
                        i.consume_key(egui::Modifiers::NONE, Key::Tab),
                        i.consume_key(egui::Modifiers::SHIFT, Key::Tab),
                    )
                });
                if down {
                    pal.selected = (pal.selected + 1).min(items.len().saturating_sub(1));
                }
                if up {
                    pal.selected = pal.selected.saturating_sub(1);
                }
                moved |= down || up;

                // The scopes, as tabs between two lines (not for a list of places).
                if pal.mode != PaletteMode::Locations {
                    let i = SCOPES.iter().position(|m| *m == pal.mode).unwrap_or(0);
                    if tab {
                        scope = Some(SCOPES[(i + 1) % SCOPES.len()]);
                    } else if back_tab {
                        scope = Some(SCOPES[(i + SCOPES.len() - 1) % SCOPES.len()]);
                    }
                    let (chips, _) = ui.allocate_exact_size(vec2(width, 36.0), Sense::hover());
                    ui.painter().hline(chips.x_range(), chips.top() + 0.5, Stroke::new(1.0, p.border));
                    ui.painter().hline(chips.x_range(), chips.bottom() - 0.5, Stroke::new(1.0, p.border));
                    let mut x = chips.left() + 12.0;
                    let cy = chips.center().y;
                    for m in SCOPES {
                        let (label, keys) = match m {
                            PaletteMode::All => (t("All"), None),
                            PaletteMode::Projects => (t("Projects"), kit::keys(&Action::SwitchProject)),
                            PaletteMode::Presets => (t("Agents"), kit::keys(&Action::PickPreset)),
                            PaletteMode::Files => (t("Files"), kit::keys(&Action::QuickOpen)),
                            PaletteMode::Search => (t("Output"), kit::keys(&Action::FindAll)),
                            _ => (t("Commands"), None),
                        };
                        let on = m == pal.mode;
                        let lg = ui.painter().layout_no_wrap(label.to_string(), style::medium(12.0), if on { p.text } else { p.text_faint });
                        let kg = keys.map(|k| ui.painter().layout_no_wrap(k.to_string(), style::mono(10.0), p.text_ghost));
                        let w = 9.0 + lg.size().x + kg.as_ref().map_or(0.0, |g| g.size().x + 6.0) + 9.0;
                        let rect = Rect::from_min_size(pos2(x, cy - 13.0), vec2(w, 26.0));
                        let resp = ui.interact(rect, egui::Id::new(("scope", m as u8)), Sense::click());
                        let fill = if on { p.subtle } else if resp.hovered() { p.hover } else { Color32::TRANSPARENT };
                        ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
                        let lw = lg.size().x;
                        let lc = if on || resp.hovered() { p.text } else { p.text_faint };
                        ui.painter().galley(pos2(rect.left() + 9.0, cy - lg.size().y / 2.0), lg, lc);
                        if let Some(g) = kg {
                            ui.painter().galley(pos2(rect.left() + 9.0 + lw + 6.0, cy - g.size().y / 2.0), g, p.text_ghost);
                        }
                        if resp.clicked() && !on {
                            scope = Some(m);
                        }
                        x = rect.right() + 2.0;
                    }
                    // At the right: ⇥ goes to the next scope.
                    let hint = ui.painter().layout_no_wrap(t("next scope").to_string(), egui::FontId::proportional(11.0), p.text_ghost);
                    let hx = chips.right() - 14.0 - hint.size().x;
                    if hx > x + 40.0 {
                        let hw = hint.size().x;
                        ui.painter().galley(pos2(hx, cy - hint.size().y / 2.0), hint, p.text_ghost);
                        let _ = hw;
                        kit::paint_kbd_text(ui.painter(), pos2(hx - 6.0 - kit::kbd_width(ui.painter(), "⇥"), cy), "⇥", &p);
                    }
                }

                // The list, and at its right (wide) the preview.
                let max_h = (ctx.content_rect().height() - 70.0 - 54.0 - 36.0 - 38.0 - 80.0).clamp(3.0 * ROW_H, 12.0 * ROW_H + 3.0 * GROUP_H);
                let body = ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    ui.vertical(|ui| {
                        ui.set_width(list_w);
                        ui.add_space(4.0);
                        if items.is_empty() {
                            let text = if pal.loading { t("Listing the project's files…") } else { t("Nothing matches.") };
                            let (r, _) = ui.allocate_exact_size(vec2(list_w, 40.0), Sense::hover());
                            ui.painter().text(pos2(r.left() + 18.0, r.center().y), Align2::LEFT_CENTER, text, egui::FontId::proportional(13.0), p.text_muted);
                        }
                        egui::ScrollArea::vertical().id_salt("palette-list").max_height(max_h).auto_shrink([false, !wide]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                            let row_w = list_w - 16.0;
                            let mut group = String::new();
                            for (i, (_, item)) in items.iter().enumerate() {
                                if grouped && item.group != group && !item.group.is_empty() {
                                    group = item.group.clone();
                                    let (gr, _) = ui.allocate_exact_size(vec2(list_w, GROUP_H), Sense::hover());
                                    if ui.is_rect_visible(gr) {
                                        let g = kit::section_galley(ui.painter(), &group, p.text_faint);
                                        ui.painter().galley(pos2(gr.left() + 18.0, gr.bottom() - 6.0 - g.size().y), g, p.text_faint);
                                    }
                                }
                                let (full, _) = ui.allocate_exact_size(vec2(list_w, ROW_H), Sense::hover());
                                let rect = Rect::from_min_size(pos2(full.left() + 8.0, full.top()), vec2(row_w, ROW_H));
                                let resp = ui.interact(rect, egui::Id::new(("pal-row", i)), Sense::click());
                                let selected = i == pal.selected;
                                if selected && moved {
                                    ui.scroll_to_rect(full, None);
                                }
                                if !ui.is_rect_visible(rect) {
                                    if resp.clicked() || (enter && selected) {
                                        chosen = Some(item.action.clone());
                                    }
                                    continue;
                                }
                                if selected {
                                    ui.painter().rect(rect, CornerRadius::same(8), if dark { p.subtle } else { Color32::WHITE }, Stroke::new(1.0, if dark { p.border } else { p.border_strong }), egui::StrokeKind::Inside);
                                } else if resp.hovered() {
                                    ui.painter().rect_filled(rect, CornerRadius::same(8), p.hover);
                                }
                                if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                                    pal.selected = i;
                                }
                                let cy = rect.center().y;
                                // The lead: an icon, a project's tile, a status and a tile, a logo.
                                let mut x = rect.left() + 10.0;
                                match &item.lead {
                                    Lead::Icon(icon, c) => {
                                        ui.painter().text(pos2(x + 11.0, cy), Align2::CENTER_CENTER, *icon, style::icon_font(15.0), c.unwrap_or(p.text_faint));
                                        x += 22.0 + 10.0;
                                    }
                                    Lead::Tile(letters, c) => {
                                        x += 22.0 + 10.0;
                                        kit::paint_project_tile(ui.painter(), Rect::from_min_size(pos2(x - 32.0 + 2.0, cy - 9.0), vec2(18.0, 18.0)), letters, *c, dark);
                                    }
                                    Lead::Waiting(st, letters, c) => {
                                        status::paint_glyph(ui.painter(), pos2(x + 11.0, cy), *st, &p, 14.0);
                                        x += 22.0 + 10.0;
                                        kit::paint_project_tile(ui.painter(), Rect::from_min_size(pos2(x, cy - 9.0), vec2(18.0, 18.0)), letters, *c, dark);
                                        x += 18.0 + 10.0;
                                    }
                                    Lead::Agent(a) => {
                                        ui.painter().text(pos2(x + 11.0, cy), Align2::CENTER_CENTER, icons::PLAY, style::icon_font(14.0), p.text_faint);
                                        x += 22.0 + 10.0;
                                        if let Some(a) = a.filter(|a| super::agent_logo::has(*a)) {
                                            super::agent_logo::paint(ui.painter(), Rect::from_center_size(pos2(x + 7.0, cy), vec2(14.0, 14.0)), a, p.text, false);
                                            x += 14.0 + 10.0;
                                        }
                                    }
                                }
                                // Right side first: a Review button, the pill, the quiet text.
                                let mut right = rect.right() - 10.0;
                                let mut reviewed = false;
                                if let Some(pane) = item.review {
                                    let w = kit::button_width(ui, None, t("Review"), None, true);
                                    let br = Rect::from_center_size(pos2(right - w / 2.0, cy), vec2(w, 24.0));
                                    right -= w + 6.0;
                                    if ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| kit::button_ex(ui, kit::Kind::Secondary, None, t("Review"), true)).inner.clicked() {
                                        self.actions.push(Action::FocusPane(pane));
                                        self.actions.push(Action::ReviewAgent);
                                        reviewed = true;
                                    }
                                }
                                if !item.right.is_empty() {
                                    let c = item.right_color.unwrap_or(p.text_faint);
                                    let font = if item.right_color.is_some() { style::mono_semibold(10.5) } else { style::mono(10.5) };
                                    let g = ui.painter().layout_no_wrap(item.right.clone(), font, c);
                                    right -= g.size().x;
                                    ui.painter().galley(pos2(right, cy - g.size().y / 2.0), g, c);
                                    right -= 8.0;
                                }
                                if let Some((mark, text, c)) = &item.pill {
                                    let w = super::sidebar::RowMark::pill_width(ui.painter(), text);
                                    mark.paint_pill(ui.painter(), pos2(right - w, cy), text, *c, &p, dark);
                                    right -= w + 8.0;
                                }
                                // The label (matched letters in the accent), then the detail.
                                let room = (right - 8.0 - x).max(20.0);
                                let font = if item.mono { style::mono(12.0) } else { style::medium(13.0) };
                                let marks = if pal.query.is_empty() { Vec::new() } else { matched(&pal.query, &item.label) };
                                let mut job = egui::text::LayoutJob::default();
                                for (ci, ch) in item.label.chars().enumerate() {
                                    let hit = marks.get(ci).copied().unwrap_or(false);
                                    let f = if hit && !item.mono { style::semibold(13.0) } else { font.clone() };
                                    job.append(&ch.to_string(), 0.0, egui::TextFormat::simple(f, if hit { p.accent_text } else { p.text }));
                                }
                                let logo_at = if item.agent.is_some() && !item.detail.is_empty() { Some(job.text.chars().count()) } else { None };
                                if !item.detail.is_empty() {
                                    let gap = if logo_at.is_some() { 10.0 + 11.0 + 5.0 } else { 10.0 };
                                    let font = if item.mono_detail { style::mono(11.0) } else { egui::FontId::proportional(12.0) };
                                    job.append(&item.detail, gap, egui::TextFormat::simple(font, p.text_faint));
                                }
                                job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                                let g = ui.painter().layout_job(job);
                                // The agent's logo in the gap before the detail.
                                if let (Some(a), Some(_)) = (item.agent, logo_at) {
                                    let label_w = ui.painter().layout_no_wrap(item.label.clone(), font.clone(), p.text).size().x;
                                    if label_w + 30.0 < room {
                                        super::agent_logo::paint(ui.painter(), Rect::from_center_size(pos2(x + label_w + 10.0 + 5.5, cy), vec2(11.0, 11.0)), a, p.text_muted, false);
                                    }
                                }
                                ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
                                if !reviewed && (resp.clicked() || (enter && selected)) {
                                    chosen = Some(item.action.clone());
                                } else if reviewed {
                                    close = true;
                                }
                            }
                            ui.add_space(8.0);
                        });
                    });
                    if wide {
                        let preview_of = items.get(pal.selected).map(|(_, it)| (it.preview, it.label.clone(), it.detail.clone(), it.right.clone(), it.action.clone()));
                        let (pr, _) = ui.allocate_exact_size(vec2(PREVIEW_W, max_h + 4.0 + 8.0), Sense::hover());
                        if let Some((preview, label, detail, right, action)) = &preview_of {
                            let open = self.palette_preview(ui, pr, *preview, label, detail, right, action);
                            if open {
                                chosen = Some(action.clone());
                            }
                        } else {
                            ui.painter().rect_filled(pr, CornerRadius { nw: 0, ne: 0, sw: 0, se: 0 }, if dark { style::over(p.overlay, p.chrome, 0.3) } else { Color32::from_rgb(0xfa, 0xfa, 0xfa) });
                        }
                    }
                });
                let _ = body;

                // The keys.
                let (foot, _) = ui.allocate_exact_size(vec2(width, 38.0), Sense::hover());
                ui.painter().hline(foot.x_range(), foot.top() + 0.5, Stroke::new(1.0, p.border));
                let hints: Vec<(&[&str], &str)> = if pal.mode == PaletteMode::Locations {
                    vec![(&["↑", "↓"], t("choose")), (&["↵"], enter_label)]
                } else {
                    vec![(&["↑", "↓"], t("choose")), (&["↵"], enter_label), (&["⇥"], t("scope"))]
                };
                super::statusbar::paint_key_hints(ui, Rect::from_min_max(pos2(foot.left() + 14.0, foot.top()), pos2(foot.right() - 14.0, foot.bottom())), &hints, Some("esc"));
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
