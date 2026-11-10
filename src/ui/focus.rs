//! The Focus view (FR-V1): one project, its tabs, and the split panes of the active tab, each
//! with a header showing its status and where that status came from.

use std::time::Instant;

use eframe::egui::{self, Align2, CornerRadius, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use tc_core::PaneId;
use tc_core::model::View;
use tc_core::status::Status;
use tc_term::layout::SplitDir;
use tc_term::widget;

use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};

pub const HEADER_H: f32 = super::sidebar::TOP_H;
const PANE_HEADER_H: f32 = 28.0;
/// Space between a pane's edge and its text.
const TERM_PAD: f32 = 6.0;

impl App {
    /// The bar at the top of the content: the window drag area, what is shown on the left
    /// (drawn by `left`), and the toolbar on the right. Returns the rect below it.
    pub fn top_bar(&mut self, ui: &mut Ui, full: Rect, left: impl FnOnce(&mut Self, &mut Ui)) -> Rect {
        let p = style::pal(ui);
        let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
        ui.painter().rect_filled(header, CornerRadius::ZERO, p.chrome);
        ui.painter().hline(header.x_range(), header.bottom() - 0.5, Stroke::new(1.0, p.border));
        super::view::window_drag_area(ui, header, "content-top");

        // Toolbar, from the right.
        let inner = header.shrink2(vec2(10.0, 0.0));
        let used = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    self.toolbar(ui);
                    ui.min_rect()
                },
            )
            .inner;

        // Left part, clipped before the toolbar.
        let mut left_rect = Rect::from_min_max(inner.min, pos2((used.left() - 12.0).max(inner.left()), inner.bottom()));
        if self.settings.sidebar_hidden {
            let fullscreen = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
            left_rect.min.x = header.left() + crate::platform::traffic_light_inset(fullscreen).max(10.0);
        }
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(left_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.set_clip_rect(left_rect.intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 6.0;
                if self.settings.sidebar_hidden
                    && kit::icon_button_ex(ui, icons::SIDEBAR_SIMPLE, &kit::tip(t("Show Sidebar"), Action::ToggleSidebar), false, 26.0)
                        .clicked()
                {
                    self.actions.push(Action::ToggleSidebar);
                }
                left(self, ui);
            },
        );
        Rect::from_min_max(pos2(full.left(), header.bottom()), full.max)
    }

    /// Right side of the top bar (added right to left).
    fn toolbar(&mut self, ui: &mut Ui) {
        if self.projects.is_empty() {
            return;
        }
        let view = self.view;
        let (focus_tip, grid_tip) = (kit::tip(t("Focus View"), Action::ToggleGrid), kit::tip(t("Grid of All Projects"), Action::ToggleGrid));
        let segs = [
            kit::Segment {
                icon: icons::TERMINAL_WINDOW,
                tooltip: &focus_tip,
                on: view == View::Focus,
            },
            kit::Segment {
                icon: icons::COLUMNS,
                tooltip: &grid_tip,
                on: view == View::Grid,
            },
        ];
        // Right to left: the group is added as one widget, so order the segments normally.
        let r = kit::button_group(ui, &segs);
        for (i, v) in [View::Focus, View::Grid].into_iter().enumerate() {
            if r[i].clicked() {
                self.actions.push(Action::SetView(v));
            }
        }
        if view != View::Focus || self.active.is_none() {
            return;
        }
        kit::vdivider(ui, 18.0);
        let editor_on = self.active.and_then(|id| self.editors.get(&id)).is_some_and(|e| e.visible);
        let e = kit::icon_toggle(ui, icons::CODE, &kit::tip(t("Show/Hide Editor"), Action::ToggleEditor), editor_on);
        #[cfg(debug_assertions)]
        crate::devtest::record("toggle-editor", e.rect);
        if e.clicked() {
            self.actions.push(Action::ToggleEditor);
        }
        if kit::icon_button(ui, icons::SQUARE_SPLIT_VERTICAL, &kit::tip(t("Split Down"), Action::SplitDown)).clicked() {
            self.actions.push(Action::SplitDown);
        }
        if kit::icon_button(ui, icons::SQUARE_SPLIT_HORIZONTAL, &kit::tip(t("Split Right"), Action::SplitRight)).clicked() {
            self.actions.push(Action::SplitRight);
        }
        ui.add_space(4.0);
        let run = kit::button_ex(ui, kit::Kind::Primary, Some(icons::PLAY), t("Agent"), true);
        #[cfg(debug_assertions)]
        crate::devtest::record("run-agent", run.rect);
        if run.on_hover_text(kit::tip(t("Start an agent in a new pane"), Action::PickPreset)).clicked() {
            self.actions.push(Action::PickPreset);
        }
    }

    pub fn focus_view(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let Some(project) = self.active_project() else {
            return;
        };
        let pid = project.id;
        let full = ui.max_rect();

        // ----- project header: name, branch, terminal tabs ------------------------------------
        let name = project.name.clone();
        let color = project.color32();
        let git = project.git.clone();
        let active_tab = project.active_tab;
        let tabs: Vec<(usize, String, Option<Status>, PaneId)> = project
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let label = tab.title.clone().unwrap_or_else(|| {
                    self.panes.get(&tab.focused).map(|pane| pane.label()).unwrap_or_default()
                });
                let waiting = tab
                    .layout
                    .panes()
                    .iter()
                    .filter_map(|id| self.panes.get(id))
                    .filter(|pane| pane.tracker.wants_attention())
                    .map(|pane| pane.tracker.status)
                    .min_by_key(|s| s.priority())
                    .or_else(|| {
                        self.panes
                            .get(&tab.focused)
                            .map(|pane| pane.tracker.status)
                            .filter(|s| !matches!(s, Status::Idle | Status::Unknown))
                    });
                (i, label, waiting, tab.focused)
            })
            .collect();
        let mut tab_rects: Vec<Rect> = Vec::with_capacity(tabs.len());
        let body = self.top_bar(ui, full, |app, ui| {
            kit::dot(ui, color, 4.5);
            ui.label(RichText::new(&name).font(style::semibold(14.0)).color(p.text));
            if let Some(g) = &git {
                let text = super::sidebar::branch_text(g);
                let r = kit::button_ex(ui, kit::Kind::Ghost, None, &text, true);
                if r.on_hover_text(kit::tip(t("Review Changes…"), Action::Review)).clicked() {
                    app.actions.push(Action::Review);
                }
            }
            kit::vdivider(ui, 18.0);
            for (i, label, waiting, focused) in &tabs {
                let dragging = app.tab_drag == Some(*i);
                let (r, close) = terminal_tab(ui, label, *waiting, *i == active_tab, dragging, tabs.len() > 1);
                if close {
                    app.actions.push(Action::CloseTab(*i));
                }
                #[cfg(debug_assertions)]
                crate::devtest::record(format!("tab:{i}"), r.rect);
                tab_rects.push(r.rect);
                if r.hovered() || r.is_pointer_button_down_on() {
                    app.tabs_hot = true;
                }
                if r.clicked() && !close {
                    app.actions.push(Action::SelectTab(*i));
                }
                if r.middle_clicked() && tabs.len() > 1 {
                    app.actions.push(Action::CloseTab(*i));
                }
                // Dragged: along the strip to reorder, onto a pane to split it (handled below,
                // once the panes are laid out).
                if r.drag_started() && tabs.len() > 1 {
                    app.tab_drag = Some(*i);
                }
                let r = r.on_hover_text(t("Drag to reorder, or onto a pane to split it"));
                if r.double_clicked() {
                    let current = app.panes.get(focused).and_then(|p| p.title.clone()).unwrap_or_default();
                    app.dialog = Some(crate::app::Dialog::RenamePane(*focused, current));
                }
            }
            if kit::icon_button_ex(ui, icons::PLUS, &kit::tip(t("New Tab"), Action::NewTab), false, 26.0).clicked() {
                app.actions.push(Action::NewTab);
            }
        });

        // ----- editor beside the panes ------------------------------------------------------------
        self.last_full_body = Some(body);
        let body = if let Some(split_x) = self.editor_split(pid, body) {
            let editor_rect = Rect::from_min_max(pos2(split_x + 1.0, body.top()), body.max);
            let divider = Rect::from_min_max(pos2(split_x - 3.0, body.top()), pos2(split_x + 3.0, body.bottom()));
            let dr = ui.interact(divider, egui::Id::new(("editor-divider", pid)), Sense::drag());
            if dr.hovered() || dr.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if dr.dragged()
                && let Some(pos) = dr.interact_pointer_pos()
                && let Some(e) = self.editors.get_mut(&pid)
            {
                e.width = ((body.right() - pos.x) / body.width()).clamp(0.2, 0.8);
            }
            if dr.drag_stopped() {
                self.mark_dirty();
            }
            let line = if dr.hovered() || dr.dragged() { p.accent } else { p.border };
            ui.painter().vline(split_x, body.y_range(), Stroke::new(1.0, line));
            let root = self.project(pid).map(|x| x.path.clone()).unwrap_or_default();
            ui.scope_builder(egui::UiBuilder::new().max_rect(editor_rect), |ui| {
                ui.set_clip_rect(editor_rect);
                self.editor_panel(ui, pid, root);
            });
            Rect::from_min_max(body.min, pos2(split_x, body.bottom()))
        } else {
            body
        };

        // ----- panes -------------------------------------------------------------------------
        let Some(tab) = self.project(pid).and_then(|p| p.tab()) else {
            return;
        };
        let focused = tab.focused;
        let (rects, dividers) = tab.layout.arrange(body);
        let single = rects.len() == 1;
        // Where panes go, for sizing new ones before they start (no resize right after).
        self.last_body = Some(body);
        let pane_areas = rects.clone();

        // Dividers.
        for d in &dividers {
            let id = egui::Id::new(("divider", pid, active_tab, d.path.clone()));
            let r = ui.interact(d.rect.expand(2.0), id, Sense::drag());
            if r.hovered() || r.dragged() {
                ui.ctx().set_cursor_icon(match d.dir {
                    tc_term::layout::SplitDir::Horizontal => egui::CursorIcon::ResizeHorizontal,
                    tc_term::layout::SplitDir::Vertical => egui::CursorIcon::ResizeVertical,
                });
            }
            if r.dragged()
                && let Some(pos) = r.interact_pointer_pos()
            {
                let ratio = tc_term::layout::ratio_at(d, pos);
                if let Some(tab) = self.projects.iter_mut().find(|x| x.id == pid).and_then(|x| x.tab_mut()) {
                    tab.layout.set_ratio(&d.path, ratio);
                }
            }
            if r.drag_stopped() {
                self.mark_dirty();
            }
            ui.painter().rect_filled(d.rect, CornerRadius::ZERO, p.border);
        }

        let dark = ui.visuals().dark_mode;
        let scheme = self.scheme(dark);
        let cell = widget::cell_size(ui.ctx(), self.settings.font_size);
        self.cell = Some(cell);
        let choice = tc_term::fonts::effective_choice(&tc_term::prefs::FontChoice {
            name: self.settings.font_name.clone(),
            path: self.settings.font_path.clone(),
            index: 0,
        });
        let style = widget::TermStyle {
            font_size: self.settings.font_size,
            cell,
            scheme,
            real_bold: tc_term::fonts::has_real_face(&choice, true, false),
            real_italic: tc_term::fonts::has_real_face(&choice, false, true),
            option_as_meta: self.settings.option_as_meta,
            copy_on_select: self.settings.copy_on_select,
            cursor_blink: self.settings.cursor_blink,
        };
        let input_enabled = self.dialog.is_none() && self.palette.is_none();
        let window_focused = self.window_focused;
        let now = Instant::now();
        for (pane_id, rect) in rects {
            let is_focused = pane_id == focused;
            // One pane: the tab above says what it is; several: each gets a header.
            let header_h = if single { 0.0 } else { PANE_HEADER_H };
            let header = Rect::from_min_size(rect.min, vec2(rect.width(), header_h));
            let pane_body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
            // The terminal's own background around its text.
            ui.painter().rect_filled(pane_body, CornerRadius::ZERO, scheme.background);
            let term_rect = term_rect(rect, single);
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("pane:{pane_id}"), term_rect);
            self.pane_rects.insert(pane_id, term_rect);
            if !single {
                self.pane_header(ui, pane_id, header, is_focused, single);
            }
            let request = self.focus_request == Some(pane_id);
            let Some(pane) = self.panes.get_mut(&pane_id) else {
                continue;
            };
            if request {
                ui.memory_mut(|m| m.request_focus(widget::pane_widget_id(pane_id)));
            }
            // Restored asleep: its shell starts now, at the size this frame gives its grid.
            if pane.dormant() {
                self.actions.push(Action::WakePane(pane_id));
                ui.ctx().request_repaint();
            }
            let alive = pane.alive();
            let resp = widget::show(
                ui,
                term_rect,
                &mut pane.view,
                &pane.shared,
                &style,
                is_focused,
                input_enabled,
                alive,
            );
            let how = crate::pane::Shown::of(ui, pane_id, &pane.view, crate::pane::Shown::Visible);
            self.shown_now.insert(pane_id, how);
            if let Some((size, w, h)) = resp.resized {
                pane.resize_pty(size, w, h);
            }
            if !resp.input.is_empty() {
                if resp.input.contains(&b'\r') {
                    pane.entered(now);
                }
                pane.input(resp.input);
            }
            if resp.activated && !is_focused {
                self.actions.push(Action::FocusPane(pane_id));
            }
            if resp.reconnect {
                self.actions.push(Action::RestartShell(pane_id));
            }
            if let Some(word) = resp.link {
                if word.starts_with("http://") || word.starts_with("https://") {
                    crate::platform::reveal(std::path::Path::new(&word));
                } else if let Some((path, line)) = crate::editor::parse_location(&word, &pane.cwd) {
                    self.actions.push(Action::OpenFile(path, line));
                }
            }
            if request && resp.focused {
                self.focus_request = None;
            }
            if single {
                self.resume_button(ui, pane_id, term_rect);
            }
            // The pane that takes the keyboard (several panes): an accent line on its header.
            if is_focused && !single && window_focused {
                ui.painter().hline(header.x_range(), header.top() + 1.0, Stroke::new(2.0, p.accent));
            }
        }
        if self.focus_request.is_some_and(|f| !self.panes.contains_key(&f)) {
            self.focus_request = None;
        }
        self.tab_drop(ui, &tab_rects, &pane_areas, active_tab);
        self.find_bar(ui);
    }

    /// A tab or a pane header being dragged. A tab: a line where it would land in the strip,
    /// or the half of a pane it would take (it becomes a split of the tab in view). A pane: the
    /// half of another pane it would move beside, or the tab strip (it becomes a tab).
    fn tab_drop(&mut self, ui: &mut Ui, tab_rects: &[Rect], panes: &[(PaneId, Rect)], active_tab: usize) {
        enum Source {
            Tab(usize),
            Pane(PaneId),
        }
        let source = match (self.tab_drag, self.pane_drag) {
            (Some(t), _) => Source::Tab(t),
            (None, Some(p)) => Source::Pane(p),
            _ => return,
        };
        let p = style::pal(ui);
        let released = ui.input(|i| i.pointer.any_released());
        let Some(pos) = ui.ctx().pointer_interact_pos() else {
            if released {
                self.tab_drag = None;
                self.pane_drag = None;
            }
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let fg = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("tab-drop")));
        let strip = tab_rects
            .iter()
            .copied()
            .reduce(|a, b| a.union(b))
            .map(|r| r.expand2(vec2(40.0, 8.0)));
        let over_pane = panes.iter().find(|(_, r)| r.contains(pos)).copied();
        let mut drop: Option<Action> = None;
        if strip.is_some_and(|r| r.contains(pos)) {
            match source {
                Source::Tab(from) => {
                    // Reorder: before the first tab whose middle is right of the pointer.
                    let to = tab_rects.iter().position(|r| pos.x < r.center().x).unwrap_or(tab_rects.len());
                    if to != from && to != from + 1 {
                        let x = tab_rects
                            .get(to)
                            .map(|r| r.left() - 3.0)
                            .or_else(|| tab_rects.last().map(|r| r.right() + 3.0));
                        if let (Some(x), Some(r)) = (x, tab_rects.first()) {
                            fg.vline(x, r.y_range().expand(2.0), Stroke::new(2.0, p.accent));
                        }
                        drop = Some(Action::MoveTab { from, to });
                    }
                }
                Source::Pane(pane) => {
                    // Its own tab, after the one in view.
                    if let Some(r) = tab_rects.get(active_tab) {
                        fg.vline(r.right() + 3.0, r.y_range().expand(2.0), Stroke::new(2.0, p.accent));
                    }
                    drop = Some(Action::PaneToTab(pane));
                }
            }
        } else if let Some((target, r)) = over_pane {
            let allowed = match source {
                Source::Tab(from) => from != active_tab,
                Source::Pane(pane) => pane != target,
            };
            if allowed {
                // The half nearest the pointer.
                let fx = (pos.x - r.left()) / r.width();
                let fy = (pos.y - r.top()) / r.height();
                let edges = [(fx, 0u8), (1.0 - fx, 1), (fy, 2), (1.0 - fy, 3)];
                let side = edges.iter().min_by(|a, b| a.0.total_cmp(&b.0)).map_or(1, |e| e.1);
                let (dir, before, zone) = match side {
                    0 => (SplitDir::Horizontal, true, Rect::from_min_max(r.min, pos2(r.center().x, r.bottom()))),
                    1 => (SplitDir::Horizontal, false, Rect::from_min_max(pos2(r.center().x, r.top()), r.max)),
                    2 => (SplitDir::Vertical, true, Rect::from_min_max(r.min, pos2(r.right(), r.center().y))),
                    _ => (SplitDir::Vertical, false, Rect::from_min_max(pos2(r.left(), r.center().y), r.max)),
                };
                fg.rect(
                    zone.shrink(4.0),
                    CornerRadius::same(style::RADIUS),
                    style::soft(p.accent, 0.16),
                    Stroke::new(1.5, style::soft(p.accent, 0.8)),
                    egui::StrokeKind::Inside,
                );
                drop = Some(match source {
                    Source::Tab(from) => Action::MergeTab { tab: from, target, dir, before },
                    Source::Pane(pane) => Action::MovePane { pane, target, dir, before },
                });
            }
        }
        // What is dragged follows the pointer.
        let label = match source {
            Source::Tab(from) => self
                .active_project()
                .and_then(|pr| pr.tabs.get(from))
                .map(|tab| {
                    tab.title
                        .clone()
                        .unwrap_or_else(|| self.panes.get(&tab.focused).map(|x| x.label()).unwrap_or_default())
                })
                .unwrap_or_default(),
            Source::Pane(pane) => self.panes.get(&pane).map(|x| x.label()).unwrap_or_default(),
        };
        let g = fg.layout_no_wrap(label, style::medium(12.5), p.text);
        let ghost = Rect::from_min_size(pos + vec2(12.0, 10.0), g.size() + vec2(20.0, 10.0));
        fg.rect(ghost, CornerRadius::same(style::RADIUS), p.surface, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
        fg.galley(ghost.min + vec2(10.0, 5.0), g, p.text);
        ui.ctx().request_repaint();
        if released {
            self.tab_drag = None;
            self.pane_drag = None;
            if let Some(a) = drop {
                self.actions.push(a);
            }
        }
    }

    /// The ⌘F bar over the pane being searched: Enter finds older output, Shift+Enter newer.
    fn find_bar(&mut self, ui: &mut Ui) {
        let Some(search) = self.pane_search.as_mut() else { return };
        let Some(rect) = self.pane_rects.get(&search.pane).copied() else {
            self.pane_search = None;
            return;
        };
        let Some(shared) = self.panes.get(&search.pane).map(|p| p.shared.clone()) else {
            self.pane_search = None;
            return;
        };
        let p = style::pal(ui);
        // Esc closes the bar (taken before the text field, which would swallow it).
        let mut close = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        egui::Area::new(egui::Id::new("find-bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.right_top() + vec2(-330.0, 6.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut search.query)
                                .hint_text(t("Find in this pane"))
                                .desired_width(170.0),
                        );
                        if search.opened {
                            r.request_focus();
                            search.opened = false;
                        }
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let shift = ui.input(|i| i.modifiers.shift);
                        let mut go: Option<bool> = None;
                        if r.changed() {
                            search.current = None;
                            go = Some(true);
                        }
                        if enter {
                            go = Some(!shift);
                            r.request_focus();
                        }
                        if kit::icon_button(ui, icons::ARROW_UP, t("Older")).clicked() {
                            go = Some(true);
                        }
                        if kit::icon_button(ui, icons::ARROW_DOWN, t("Newer")).clicked() {
                            go = Some(false);
                        }
                        if let Some(backward) = go {
                            match tc_term::search::find(&shared, &search.query, search.current.as_ref(), backward) {
                                Some(m) => {
                                    tc_term::search::reveal(&shared, &m);
                                    search.current = Some(m);
                                    search.missing = false;
                                }
                                None => search.missing = !search.query.is_empty(),
                            }
                        }
                        if search.missing {
                            ui.label(RichText::new(t("Not found")).size(style::SMALL).color(p.err));
                        }
                        if kit::icon_button(ui, icons::X, t("Close")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if close {
            let pane = search.pane;
            self.pane_search = None;
            self.focus_request = Some(pane);
        }
    }

    fn pane_header(&mut self, ui: &mut Ui, pane_id: u64, rect: Rect, focused: bool, single: bool) {
        let p = style::pal(ui);
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("pane-header:{pane_id}"), rect);
        let Some(pane) = self.panes.get(&pane_id) else {
            return;
        };
        let root = self.project(pane.project).map(|x| x.path.clone()).unwrap_or_default();
        let painter = ui.painter_at(rect);
        let lit = focused && !single;
        painter.rect_filled(rect, CornerRadius::ZERO, if lit { p.surface } else { p.chrome });
        painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
        let s = pane.tracker.status;
        let attention = pane.tracker.wants_attention();
        let c = status::color(s, &p);
        let cy = rect.center().y;
        let resume = pane.resume.clone();
        let label = pane.label();
        let place = pane.place(&root);
        let message = pane.tracker.message.clone().filter(|_| s == Status::NeedsInput);
        let src = status::source(pane.tracker.source);
        let since = pane.tracker.since;
        let hovered = ui.rect_contains_pointer(rect);

        // Buttons first, from the right (split and close only while hovered); the text on the
        // left is clipped before them.
        let mut right = rect.right() - 4.0;
        let button = |ui: &mut Ui, icon: &str, tip: &str, right: &mut f32| {
            let r = Rect::from_center_size(pos2(*right - 11.0, cy), vec2(22.0, 22.0));
            *right -= 24.0;
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::icon_button_ex(ui, icon, tip, false, 22.0))
                .inner
        };
        if hovered || focused {
            if button(ui, icons::X, &kit::tip(t("Close Pane"), Action::ClosePane), &mut right).clicked() {
                self.actions.push(Action::ClosePaneId(pane_id));
            }
            if hovered {
                if button(ui, icons::SQUARE_SPLIT_VERTICAL, t("Split Down"), &mut right).clicked() {
                    self.actions.push(Action::FocusPane(pane_id));
                    self.actions.push(Action::SplitDown);
                }
                if button(ui, icons::SQUARE_SPLIT_HORIZONTAL, t("Split Right"), &mut right).clicked() {
                    self.actions.push(Action::FocusPane(pane_id));
                    self.actions.push(Action::SplitRight);
                }
            }
        }
        if let Some((agent, _)) = resume {
            let long = tf("Resume {}", &[agent.label()]);
            let text = if rect.width() > 420.0 { long } else { t("Resume").to_string() };
            let w = style::text_width(ui, &text) + 34.0;
            let r = Rect::from_min_size(pos2(right - w, rect.top() + 2.0), vec2(w, rect.height() - 4.0));
            right -= w + 4.0;
            let resp = ui
                .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                    kit::button_ex(ui, kit::Kind::Secondary, Some(icons::PLAY), &text, true)
                })
                .inner;
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("resume:{pane_id}"), resp.rect);
            if resp
                .on_hover_text(tf("Continue {}'s last session here", &[agent.label()]))
                .clicked()
            {
                self.actions.push(Action::Resume(pane_id));
            }
        }

        let clip = Rect::from_min_max(rect.min, pos2((right - 4.0).max(rect.left()), rect.max.y));
        let painter = painter.with_clip_rect(clip);
        let mut x = rect.left() + 10.0;
        let quiet = matches!(s, Status::Idle | Status::Unknown);
        painter.text(
            pos2(x + 6.0, cy),
            Align2::CENTER_CENTER,
            if quiet { icons::TERMINAL } else { status::icon(s) },
            style::icon_font(13.0),
            if quiet { p.text_faint } else { c },
        );
        x += 18.0;
        let g = painter.layout_no_wrap(label, style::medium(12.5), if focused { p.text } else { p.text_muted });
        let lw = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
        x += lw + 8.0;
        if let Some(place) = place {
            let g = painter.layout_no_wrap(place, egui::FontId::proportional(style::SMALL), p.text_faint);
            let w = g.size().x;
            painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
            x += w + 10.0;
        }
        // The status as a pill, unless the pane is just sitting at its prompt.
        if !quiet {
            let g = painter.layout_no_wrap(status::text(s), style::medium(style::SMALL), c);
            let pill = Rect::from_min_size(pos2(x, cy - 9.0), vec2(g.size().x + 14.0, 18.0));
            painter.rect_filled(pill, CornerRadius::same(9), style::soft(c, if ui.visuals().dark_mode { 0.18 } else { 0.12 }));
            painter.galley(pill.center() - g.size() / 2.0, g, c);
            x = pill.right() + 8.0;
        }
        if let Some(msg) = message {
            let max = (clip.right() - x).max(0.0);
            let mut job = egui::text::LayoutJob::simple_singleline(msg, egui::FontId::proportional(style::SMALL), p.text_muted);
            job.wrap = egui::text::TextWrapping::truncate_at_width(max);
            let mg = painter.layout_job(job);
            painter.galley(pos2(x, cy - mg.size().y / 2.0), mg, p.text_muted);
        }
        let status_rect = Rect::from_min_max(rect.min, pos2(x.min(clip.right()), rect.max.y));
        let tip = if src.is_empty() {
            status::text(s)
        } else {
            tf("{} · from {}, {} ago", &[&status::text(s), &src, &status::age(since)])
        };
        // The whole header (left of its buttons) can be dragged to move the pane.
        let grab = Rect::from_min_max(rect.min, pos2(clip.right(), rect.max.y));
        let label_resp = ui
            .interact(grab.union(status_rect), egui::Id::new(("pane-status", pane_id)), Sense::click_and_drag())
            .on_hover_text(format!("{tip}\n{}", t("Double-click to rename · drag to move")));
        if label_resp.drag_started() {
            self.pane_drag = Some(pane_id);
        }
        if label_resp.double_clicked() {
            let current = self.panes.get(&pane_id).and_then(|p| p.title.clone()).unwrap_or_default();
            self.dialog = Some(crate::app::Dialog::RenamePane(pane_id, current));
        }
        if label_resp.clicked() && !focused {
            self.actions.push(Action::FocusPane(pane_id));
        }
        label_resp.context_menu(|ui| {
            if ui.button(t("Move to New Tab")).clicked() {
                self.actions.push(Action::PaneToTab(pane_id));
                ui.close();
            }
            if ui.button(t("Rename…")).clicked() {
                let current = self.panes.get(&pane_id).and_then(|p| p.title.clone()).unwrap_or_default();
                self.dialog = Some(crate::app::Dialog::RenamePane(pane_id, current));
                ui.close();
            }
            ui.separator();
            if ui.button(t("Close Pane")).clicked() {
                self.actions.push(Action::ClosePaneId(pane_id));
                ui.close();
            }
        });
        if attention {
            painter.rect_filled(
                Rect::from_min_size(rect.left_top(), vec2(3.0, rect.height())),
                CornerRadius::ZERO,
                c,
            );
        }
    }

    /// "Resume Claude Code" over the top right of a pane without a header.
    fn resume_button(&mut self, ui: &mut Ui, pane_id: PaneId, term: Rect) {
        let Some((agent, _)) = self.panes.get(&pane_id).and_then(|p| p.resume.clone()) else { return };
        let text = tf("Resume {}", &[agent.label()]);
        let w = style::text_width(ui, &text) + 34.0;
        let r = Rect::from_min_size(pos2(term.right() - w - 12.0, term.top() + 6.0), vec2(w, 26.0));
        let resp = ui
            .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                kit::button_ex(ui, kit::Kind::Primary, Some(icons::PLAY), &text, true)
            })
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("resume:{pane_id}"), resp.rect);
        if resp
            .on_hover_text(tf("Continue {}'s last session here", &[agent.label()]))
            .clicked()
        {
            self.actions.push(Action::Resume(pane_id));
        }
    }

    /// The top bar of the Grid.
    pub fn overview_bar(&mut self, ui: &mut Ui) -> Rect {
        let p = style::pal(ui);
        let full = ui.max_rect();
        let n = self.projects.len();
        let waiting = self.attention_queue().len();
        let active = self.active_projects();
        let body = self.top_bar(ui, full, |app, ui| {
            ui.label(RichText::new(t("All Projects")).font(style::semibold(14.0)).color(p.text));
            ui.label(
                RichText::new(format!(
                    "{} · {} · {}",
                    crate::i18n::count(n, t("1 project"), t("{} projects")),
                    tf("{} active", &[&active.to_string()]),
                    tf("{} waiting", &[&waiting.to_string()])
                ))
                    .size(style::SMALL)
                    .color(p.text_faint),
            );
            ui.add_space(6.0);
            let options = [
                (t("All"), t("Every project's terminal, two columns; scroll for more")),
                (t("Active"), t("Only projects where something runs or waits")),
            ];
            if let Some(i) = kit::segmented(ui, &options, app.grid_active_only as usize) {
                app.actions.push(Action::GridActiveOnly(i == 1));
            }
        });
        // The same area as the Focus view's (one top bar above both): panes shown in the Grid
        // are sized against it.
        self.last_full_body = Some(body);
        body
    }
}

/// A terminal tab in the project header: its status icon (when something runs or waits), its
/// label and, when hovered or active and there are several tabs, a close button. Returns the
/// tab's response and whether the close button was clicked.
fn terminal_tab(
    ui: &mut Ui,
    label: &str,
    waiting: Option<Status>,
    on: bool,
    dragging: bool,
    closable: bool,
) -> (egui::Response, bool) {
    let p = style::pal(ui);
    let font = style::medium(12.5);
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), font, egui::Color32::PLACEHOLDER);
    job.wrap = egui::text::TextWrapping::truncate_at_width(160.0);
    let galley = ui.painter().layout_job(job);
    let close_w = if closable { 20.0 } else { 0.0 };
    let w = galley.size().x + 12.0 + 18.0 + 10.0 + close_w;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click_and_drag());
    let close_rect = Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(18.0, 18.0));
    let close_resp = closable.then(|| ui.interact(close_rect, resp.id.with("close"), Sense::click()));
    let painter = ui.painter_at(rect);
    if dragging {
        painter.rect_stroke(
            rect.shrink(0.5),
            CornerRadius::same(style::RADIUS),
            Stroke::new(1.0, p.border_strong),
            egui::StrokeKind::Inside,
        );
    } else if on {
        painter.rect(
            rect,
            CornerRadius::same(style::RADIUS),
            p.surface,
            Stroke::new(1.0, p.border_strong),
            egui::StrokeKind::Inside,
        );
    } else if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(style::RADIUS), p.hover);
    }
    let (icon, color) = match waiting {
        Some(s) => (status::icon(s), status::color(s, &p)),
        None => (icons::TERMINAL, if on { p.accent_text } else { p.text_faint }),
    };
    painter.text(pos2(rect.left() + 18.0, rect.center().y), Align2::CENTER_CENTER, icon, style::icon_font(13.0), color);
    let text_color = if on { p.text } else { p.text_muted };
    painter.galley(pos2(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0), galley, text_color);
    let mut closed = false;
    if let Some(c) = close_resp {
        let hot = c.hovered();
        if hot {
            painter.rect_filled(close_rect, CornerRadius::same(4), p.subtle);
        }
        if hot || resp.hovered() || on {
            painter.text(
                close_rect.center(),
                Align2::CENTER_CENTER,
                icons::X,
                style::icon_font(11.0),
                if hot { p.text } else { p.text_faint },
            );
        }
        closed = c.on_hover_text(t("Close Tab")).clicked();
    }
    (resp, closed)
}

/// Where a pane's terminal text goes inside its rect: below its header (when the tab has
/// several panes), with a little padding.
pub fn term_rect(pane: Rect, single: bool) -> Rect {
    let top = pane.top() + if single { 0.0 } else { PANE_HEADER_H };
    Rect::from_min_max(
        pos2(pane.left() + TERM_PAD, top + 4.0),
        pos2(pane.right() - TERM_PAD + 2.0, pane.bottom() - 2.0),
    )
}
