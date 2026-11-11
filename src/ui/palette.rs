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
const ROW_H: f32 = 34.0;
const GROUP_H: f32 = 27.0;
const WIDTH: f32 = 640.0;
/// The scopes, in the order ⇥ goes through them.
const SCOPES: [PaletteMode; 6] =
    [PaletteMode::All, PaletteMode::Projects, PaletteMode::Presets, PaletteMode::Files, PaletteMode::Search, PaletteMode::Commands];

/// What sits before a row's label.
#[derive(Clone)]
enum Lead {
    Icon(&'static str, Option<Color32>),
    /// A project's colour.
    Dot(Color32),
}

#[derive(Clone)]
struct Item {
    lead: Lead,
    label: String,
    /// The label is code (a line of a file or of a terminal): monospace.
    mono: bool,
    detail: String,
    /// At the right: a status pill, then quiet text (a shortcut, a line number…).
    pill: Option<(&'static str, String, Color32)>,
    right: String,
    right_color: Option<Color32>,
    /// A Review button at the right (an agent that finished).
    review: Option<tc_core::PaneId>,
    /// The group it is listed under while nothing is typed.
    group: String,
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
            action,
        }
    }
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
fn matched(query: &str, text: &str) -> Vec<bool> {
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
                        let project = self.project(x.project).map(|x| x.name.clone()).unwrap_or_default();
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
                let project = self.project(x.project).map(|x| x.name.clone()).unwrap_or_default();
                let mut it = Item::new(
                    Lead::Icon(status::icon(e.status), Some(status::color(e.status, &p))),
                    format!("{project} · {}", x.label()),
                    Action::FocusPane(e.pane),
                );
                it.detail = format!("{} · {}", super::sidebar::attention_what(x), status::short_age(e.since));
                if i == 0 {
                    it.right = kit::keys(&Action::NextAttention).unwrap_or_default().to_string();
                }
                if e.status == Status::Done {
                    it.review = Some(e.pane);
                }
                it.group = t("Waiting for you").to_string();
                items.push(it);
            }
        }
        if matches!(mode, PaletteMode::All | PaletteMode::Projects) {
            for (i, pr) in self.projects.iter().enumerate() {
                let mut it = Item::new(Lead::Dot(pr.color32()), pr.name.clone(), Action::SelectProject(pr.id));
                it.detail = crate::paths::contract_tilde(&pr.path);
                it.right = kit::keys(&Action::SelectProjectIndex(i)).unwrap_or_default().to_string();
                let row = self.project_row(pr, &p);
                it.pill = row.chip;
                it.group = t("Projects").to_string();
                items.push(it);
            }
        }
        if matches!(mode, PaletteMode::All | PaletteMode::Presets) {
            let project = self.active_project().map(|x| x.name.clone()).unwrap_or_default();
            for preset in self.presets() {
                let mut it = Item::new(Lead::Icon(icons::PLAY, None), preset.name.clone(), Action::RunPreset(preset.id.clone()));
                it.detail = preset.command.clone();
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
        let all = self.palette_items(pal_ref.mode);
        let Some(mut pal) = self.palette.take() else { return };
        let typed = !pal.query.trim().is_empty() && pal.mode != PaletteMode::Search;
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
        let area = egui::Area::new(egui::Id::new("palette-area")).anchor(Align2::CENTER_TOP, vec2(0.0, 72.0));
        let modal = egui::Modal::new(egui::Id::new("palette"))
            .area(area)
            .frame(
                egui::Frame::popup(&ctx.global_style())
                    .inner_margin(egui::Margin::ZERO)
                    .corner_radius(CornerRadius::same(12))
                    .shadow(ctx.global_style().visuals.window_shadow),
            )
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let p = style::pal(ui);
                // The field.
                let (row, _) = ui.allocate_exact_size(vec2(WIDTH, 50.0), Sense::hover());
                ui.painter().hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, p.border));
                ui.painter().text(pos2(row.left() + 16.0 + 9.0, row.center().y), Align2::CENTER_CENTER, icons::MAGNIFYING_GLASS, style::icon_font(18.0), p.text_muted);
                let field = Rect::from_min_max(pos2(row.left() + 16.0 + 18.0 + 10.0, row.top()), pos2(row.right() - 14.0, row.bottom()));
                let r = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                        ui.add_sized(
                            field.size(),
                            egui::TextEdit::singleline(&mut pal.query)
                                .hint_text(egui::RichText::new(hint).color(p.text_faint))
                                .font(egui::FontId::proportional(16.0))
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

                // The scopes (not for a list of places).
                if pal.mode != PaletteMode::Locations {
                    let i = SCOPES.iter().position(|m| *m == pal.mode).unwrap_or(0);
                    if tab {
                        scope = Some(SCOPES[(i + 1) % SCOPES.len()]);
                    } else if back_tab {
                        scope = Some(SCOPES[(i + SCOPES.len() - 1) % SCOPES.len()]);
                    }
                    let (chips, _) = ui.allocate_exact_size(vec2(WIDTH, 34.0), Sense::hover());
                    let mut x = chips.left() + 12.0;
                    let cy = chips.top() + 8.0 + 11.0;
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
                        let lg = ui.painter().layout_no_wrap(label.to_string(), style::medium(11.5), if on { p.text } else { p.text_muted });
                        let kg = keys.map(|k| ui.painter().layout_no_wrap(k.to_string(), egui::FontId::monospace(10.0), p.text_faint));
                        let w = 8.0 + lg.size().x + kg.as_ref().map_or(0.0, |g| g.size().x + 5.0) + 8.0;
                        let rect = Rect::from_min_size(pos2(x, cy - 11.0), vec2(w, 22.0));
                        let resp = ui.interact(rect, egui::Id::new(("scope", m as u8)), Sense::click());
                        let fill = if on { p.accent_soft } else if resp.hovered() { p.hover } else { Color32::TRANSPARENT };
                        ui.painter().rect(rect, CornerRadius::same(11), fill, Stroke::new(1.0, if on { p.border_strong } else { p.border }), egui::StrokeKind::Inside);
                        let lw = lg.size().x;
                        ui.painter().galley(pos2(rect.left() + 8.0, cy - lg.size().y / 2.0), lg, p.text);
                        if let Some(g) = kg {
                            ui.painter().galley(pos2(rect.left() + 8.0 + lw + 5.0, cy - g.size().y / 2.0), g, p.text_faint);
                        }
                        if resp.clicked() && !on {
                            scope = Some(m);
                        }
                        x = rect.right() + 4.0;
                    }
                }

                // The list.
                ui.add_space(4.0);
                if items.is_empty() {
                    let text = if pal.loading { t("Listing the project's files…") } else { t("Nothing matches.") };
                    let (r, _) = ui.allocate_exact_size(vec2(WIDTH, 40.0), Sense::hover());
                    ui.painter().text(pos2(r.left() + 18.0, r.center().y), Align2::LEFT_CENTER, text, egui::FontId::proportional(13.0), p.text_muted);
                }
                // A dozen rows show at a time (fewer in a low window), the rest scroll.
                let max_h = (ctx.content_rect().height() - 72.0 - 50.0 - 34.0 - 32.0 - 80.0).clamp(3.0 * ROW_H, 12.0 * ROW_H + 3.0 * GROUP_H);
                egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    let width = WIDTH - 16.0;
                    let mut group = String::new();
                    for (i, (_, item)) in items.iter().enumerate() {
                        if grouped && item.group != group && !item.group.is_empty() {
                            group = item.group.clone();
                            let (gr, _) = ui.allocate_exact_size(vec2(WIDTH, GROUP_H), Sense::hover());
                            if ui.is_rect_visible(gr) {
                                let g = kit::section_galley(ui.painter(), &group, p.text_faint);
                                ui.painter().galley(pos2(gr.left() + 16.0, gr.top() + 10.0), g, p.text_faint);
                            }
                        }
                        let (full, _) = ui.allocate_exact_size(vec2(WIDTH, ROW_H), Sense::hover());
                        let rect = Rect::from_min_size(pos2(full.left() + 8.0, full.top()), vec2(width, ROW_H));
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
                        if selected || resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(7), if selected { p.accent_soft } else { p.faint_row() });
                        }
                        let cy = rect.center().y;
                        match &item.lead {
                            Lead::Icon(icon, c) => {
                                ui.painter().text(pos2(rect.left() + 10.0 + 9.0, cy), Align2::CENTER_CENTER, *icon, style::icon_font(16.0), c.unwrap_or(p.text_muted));
                            }
                            Lead::Dot(c) => {
                                ui.painter().circle_filled(pos2(rect.left() + 10.0 + 9.0, cy), 4.0, *c);
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
                                self.actions.push(Action::Review);
                                reviewed = true;
                            }
                        }
                        if !item.right.is_empty() {
                            let c = item.right_color.unwrap_or(p.text_faint);
                            let font = if item.right_color.is_some() { style::medium(11.0) } else { egui::FontId::proportional(11.0) };
                            let g = ui.painter().layout_no_wrap(item.right.clone(), font, c);
                            right -= g.size().x;
                            ui.painter().galley(pos2(right, cy - g.size().y / 2.0), g, c);
                            right -= 6.0;
                        }
                        if let Some((icon, text, c)) = &item.pill {
                            let w = kit::pill_width(ui.painter(), Some(icon), text);
                            kit::paint_pill(ui.painter(), pos2(right - w, cy), Some(icon), text, *c, style::tint(*c, ui.visuals().dark_mode));
                            right -= w + 6.0;
                        }
                        // The label (matched letters stronger), then the detail, cut before the right.
                        let x = rect.left() + 10.0 + 18.0 + 10.0;
                        let room = (right - 10.0 - x).max(20.0);
                        let font = if item.mono { egui::FontId::monospace(12.0) } else { style::medium(13.5) };
                        let marks = if pal.query.is_empty() { Vec::new() } else { matched(&pal.query, &item.label) };
                        let mut job = egui::text::LayoutJob::default();
                        for (ci, ch) in item.label.chars().enumerate() {
                            let hit = marks.get(ci).copied().unwrap_or(false);
                            let f = if hit && !item.mono { style::semibold(13.5) } else { font.clone() };
                            job.append(&ch.to_string(), 0.0, egui::TextFormat::simple(f, if hit { p.accent_text } else { p.text }));
                        }
                        if !item.detail.is_empty() {
                            job.append(&item.detail, 10.0, egui::TextFormat::simple(egui::FontId::proportional(style::SMALL), p.text_faint));
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
                    ui.add_space(8.0);
                });

                // The keys.
                let (foot, _) = ui.allocate_exact_size(vec2(WIDTH, 32.0), Sense::hover());
                ui.painter().rect_filled(foot, CornerRadius { nw: 0, ne: 0, sw: 12, se: 12 }, style::over(p.surface, p.chrome, 0.5));
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
                self.focus_request = self.current_pane();
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
