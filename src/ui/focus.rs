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
/// How far either side of a divider (or the Code drawer's edge) a drag takes it.
pub(crate) const DIVIDER_GRAB: f32 = 5.0;
/// Space between a pane's edge and where the terminal's own padding starts (12 px to the text
/// at the sides, as the design has it).
const TERM_PAD: f32 = 2.0;
/// The banner over a restored pane whose agent can be resumed, and the bar under one whose
/// shell ended.
const BANNER_H: f32 = 40.0;
const FOOT_H: f32 = 38.0;

/// What draws a part of the top bar in the rect it is given.
pub type BarPart<'a> = dyn FnMut(&mut App, &mut Ui, Rect) + 'a;

/// How long a dragged tab is held over another tab before that one shows.
const TAB_SPRING: std::time::Duration = std::time::Duration::from_millis(400);

impl App {
    /// The bar at the top of the content: the window drag area, what is shown on the left
    /// (drawn by `left`), an optional control in the middle (`centre`: its width, and what draws
    /// it in the rect it is given) and the toolbar on the right. Returns the rect below it.
    pub fn top_bar(
        &mut self,
        ui: &mut Ui,
        full: Rect,
        left: impl FnOnce(&mut Self, &mut Ui),
        centre: Option<(f32, &mut BarPart<'_>)>,
    ) -> Rect {
        let p = style::pal(ui);
        let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
        ui.painter().rect_filled(header, CornerRadius::ZERO, p.chrome);
        ui.painter().hline(header.x_range(), header.bottom() - 0.5, Stroke::new(1.0, p.border));
        super::view::window_drag_area(ui, header, "content-top");
        // Windows: the window buttons at the right end (no system caption above the bar).
        let caption = crate::platform::custom_caption().then(|| Rect::from_min_max(pos2(header.right() - super::caption::CAPTION_W, header.top()), header.max));
        if let Some(rect) = caption {
            super::caption::caption_buttons(ui, rect, self.window_focused);
        }

        // Toolbar, from the right.
        let right = caption.map_or(header.right(), |c| c.left()) - 8.0;
        let inner = Rect::from_min_max(pos2(header.left() + 12.0, header.top()), pos2(right, header.bottom()));
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

        // Middle part: centred on the bar, moved left when it would reach the toolbar.
        let mut left_end = used.left() - 12.0;
        if let Some((w, draw)) = centre {
            let mut r = Rect::from_center_size(header.center(), vec2(w, kit::SEGMENTED_H));
            if r.right() > left_end {
                r = r.translate(vec2(left_end - r.right(), 0.0));
            }
            draw(self, ui, r);
            left_end = r.left() - 12.0;
        }

        // Left part, clipped before the middle part or the toolbar.
        let mut left_rect = Rect::from_min_max(inner.min, pos2(left_end.max(inner.left()), inner.bottom()));
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
                // The views' switch lives at the top of the sidebar; with the sidebar hidden, here.
                if self.settings.sidebar_hidden && !self.projects.is_empty() {
                    self.view_switch_compact(ui);
                    ui.add_space(6.0);
                }
                left(self, ui);
            },
        );
        Rect::from_min_max(pos2(full.left(), header.bottom()), full.max)
    }

    /// The Focus | Grid switch in the top bar, for when the sidebar (where it lives) is hidden.
    fn view_switch_compact(&mut self, ui: &mut Ui) {
        let focus_tip = kit::tip(t("Focus View"), Action::ToggleGrid);
        let grid_tip = kit::tip(t("Grid of All Projects"), Action::ToggleGrid);
        let options = [kit::Seg::new(t("Focus"), &focus_tip), kit::Seg::new(t("Grid"), &grid_tip)];
        if let Some(i) = kit::segmented(ui, &options, (self.view == View::Grid) as usize) {
            self.actions.push(Action::SetView(if i == 1 { View::Grid } else { View::Focus }));
        }
    }

    /// Right side of the top bar (added right to left): what acts on the active project.
    fn toolbar(&mut self, ui: &mut Ui) {
        if self.review_full() {
            self.review_toolbar(ui);
            return;
        }
        if self.view == View::Grid && !self.projects.is_empty() {
            self.grid_toolbar(ui);
            return;
        }
        if self.view != View::Focus || self.active.is_none() {
            return;
        }
        let editor_on = self.active.and_then(|id| self.editors.get(&id)).is_some_and(|e| e.visible);
        let e = kit::icon_button_ex(ui, icons::CODE, &kit::tip(t("Code"), Action::ToggleEditor), editor_on, 26.0);
        #[cfg(debug_assertions)]
        crate::devtest::record("toggle-editor", e.rect);
        if e.clicked() {
            self.actions.push(Action::ToggleEditor);
        }
        self.browser_button(ui);
        kit::vdivider(ui, 18.0);
        if kit::icon_button_ex(ui, icons::SQUARE_SPLIT_VERTICAL, &kit::tip(t("Split Down"), Action::SplitDown), false, 26.0).clicked() {
            self.actions.push(Action::SplitDown);
        }
        if kit::icon_button_ex(ui, icons::SQUARE_SPLIT_HORIZONTAL, &kit::tip(t("Split Right"), Action::SplitRight), false, 26.0).clicked() {
            self.actions.push(Action::SplitRight);
        }
        ui.add_space(4.0);
        let run = kit::button_keys(ui, kit::Kind::Primary, Some(icons::PLAY), t("Run Agent"), kit::keys(&Action::PickPreset), true);
        #[cfg(debug_assertions)]
        crate::devtest::record("run-agent", run.rect);
        if run.on_hover_text(t("Start an agent in a new pane")).clicked() {
            self.actions.push(Action::PickPreset);
        }
    }

    pub fn focus_view(&mut self, ui: &mut Ui) {
        self.note_tab_seen();
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
        // A worktree: its project's name before its own, and its branch beside its base.
        let parent = project.parent.and_then(|x| self.project(x)).map(|x| (x.id, x.name.clone()));
        let worktree = project.worktree.clone();
        let active_tab = project.active_tab;
        // Each tab with the status its icon shows and, for its tooltip, where that came from and
        // what the program in its pane says it is doing.
        let tabs: Vec<_> = project
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let label = tab.title.clone().unwrap_or_else(|| self.leaf_label(tab.focused));
                let waiting = tab
                    .layout
                    .panes()
                    .iter()
                    .filter_map(|id| self.panes.get(id))
                    .filter(|pane| pane.tracker.wants_attention())
                    .min_by_key(|pane| pane.tracker.status.priority())
                    .or_else(|| {
                        self.panes
                            .get(&tab.focused)
                            .filter(|pane| !matches!(pane.tracker.status, Status::Idle | Status::Unknown))
                    })
                    .map(|pane| (pane.tracker.status, status::described(&pane.tracker)));
                let doing = self.panes.get(&tab.focused).and_then(|pane| pane.program_title.clone());
                let progress = tab.layout.panes().iter().filter_map(|id| self.panes.get(id)).find_map(|pane| pane.progress);
                // A browser tab: Some(whether its page is asleep).
                let web = self.browsers.get(&tab.focused).map(|b| b.asleep && !b.url.is_empty());
                (i, label, waiting, doing, tab.focused, progress, web)
            })
            .collect();
        let servers: Vec<(String, String)> = self
            .project_servers(pid)
            .into_iter()
            .map(|(id, s)| (s.url, self.panes.get(&id).map(|p| p.label()).unwrap_or_default()))
            .collect();
        let mut tab_rects: Vec<Rect> = Vec::with_capacity(tabs.len());
        // The part of the strip in view (tabs scrolled out of it are no place to drop on).
        let mut strip_shown: Option<Rect> = None;
        let body = self.top_bar(ui, full, |app, ui| {
            let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, color);
            ui.add_space(2.0);
            if let (Some((parent_id, parent_name)), Some(wt)) = (&parent, &worktree) {
                // "ternary-engine › generics": the project opens it; the worktree's name opens
                // its menu.
                let r = ui.add(egui::Label::new(RichText::new(parent_name).font(style::medium(14.0)).color(p.text_muted)).sense(Sense::click()));
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tf("Open {}", &[parent_name])).clicked() {
                    app.actions.push(Action::SelectProject(*parent_id));
                }
                ui.label(RichText::new(icons::CARET_RIGHT).font(style::icon_font(11.0)).color(p.text_faint));
                let g = ui.painter().layout_no_wrap(name.clone(), style::semibold(14.0), p.text);
                let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 12.0, 24.0), Sense::click());
                let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
                if resp.hovered() || open {
                    ui.painter().rect_filled(rect, CornerRadius::same(style::RADIUS), p.hover);
                }
                ui.painter().galley(pos2(rect.left() + 6.0, rect.center().y - g.size().y / 2.0), g, p.text);
                #[cfg(debug_assertions)]
                crate::devtest::record("worktree-title", rect);
                let resp = resp.on_hover_text(t("Worktree"));
                egui::Popup::menu(&resp).align(egui::RectAlign::BOTTOM_START).show(|ui| {
                    ui.set_min_width(250.0);
                    ui.set_max_width(250.0);
                    app.worktree_menu(ui, pid);
                });
                if let Some(g) = &git {
                    let r = worktree_chip(ui, &wt.branch, g.ahead, &wt.base, g.changed);
                    #[cfg(debug_assertions)]
                    crate::devtest::record("branch-chip", r.rect);
                    if r.on_hover_text(kit::tip(&tf("Review Against {}", &[&wt.base]), Action::Review)).clicked() {
                        app.actions.push(Action::Review);
                    }
                }
            } else {
                ui.label(RichText::new(&name).font(style::semibold(14.0)).color(p.text));
                if let Some(g) = &git {
                    let r = branch_chip(ui, &g.branch, g.changed);
                    #[cfg(debug_assertions)]
                    crate::devtest::record("branch-chip", r.rect);
                    if r.on_hover_text(kit::tip(t("Review Changes…"), Action::Review)).clicked() {
                        app.actions.push(Action::Review);
                    }
                }
            }
            if !servers.is_empty()
                && let Some(url) = server_chip(ui, &servers)
            {
                app.actions.push(Action::OpenWeb(url));
            }
            kit::vdivider(ui, 18.0);
            // More tabs than fit scroll sideways (wheel or trackpad), a fade at the edge that
            // hides some; the tab in view is scrolled to when it changes. "+" stays outside.
            let row = ui.max_rect();
            let plus_w = 26.0 + ui.spacing().item_spacing.x;
            let strip = Rect::from_min_max(
                pos2(ui.cursor().min.x, row.center().y - 14.0),
                pos2((row.right() - plus_w).max(ui.cursor().min.x), row.center().y + 14.0),
            );
            let seen_id = egui::Id::new(("tab-strip-active", pid));
            let scroll_to_active = ui.data(|d| d.get_temp::<(usize, usize)>(seen_id)) != Some((active_tab, tabs.len()));
            ui.data_mut(|d| d.insert_temp(seen_id, (active_tab, tabs.len())));
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(strip)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.style_mut().always_scroll_the_only_direction = true;
                    // Our own fades: egui's take the colour of the panel behind, not the bar's.
                    ui.spacing_mut().scroll.fade.strength = 0.0;
                    let out = egui::ScrollArea::horizontal()
                        .id_salt(("tab-strip", pid))
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .scroll_source(egui::scroll_area::ScrollSource::MOUSE_WHEEL)
                        .show(ui, |ui| {
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                for (i, label, waiting, doing, focused, progress, web) in &tabs {
                                    let dragging = app.tab_drag == Some(*i);
                                    let on = *i == active_tab;
                                    let (r, close) =
                                        terminal_tab(ui, label, waiting.as_ref().map(|w| w.0), on, dragging, tabs.len() > 1, *progress, *web);
                                    if close {
                                        app.actions.push(Action::CloseTab(*i));
                                    }
                                    if on && scroll_to_active {
                                        r.scroll_to_me(None);
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
                                    // Dragged: along the strip to reorder, onto a pane to split it
                                    // (handled below, once the panes are laid out).
                                    if r.drag_started() && tabs.len() > 1 {
                                        app.tab_drag = Some(*i);
                                    }
                                    // One tab has nothing to go beside: Split Right says how
                                    // to get a second terminal there.
                                    let hint = if tabs.len() > 1 {
                                        t("Drag to reorder, or onto a pane to split it").to_string()
                                    } else {
                                        kit::tip(t("Split Right"), Action::SplitRight)
                                    };
                                    let mut tip = match waiting {
                                        Some((_, said)) => format!("{said}\n{hint}"),
                                        None => hint,
                                    };
                                    if let Some(doing) = doing {
                                        tip = format!("{doing}\n{tip}");
                                    }
                                    let r = r.on_hover_text(tip);
                                    if r.double_clicked() {
                                        let current = app.panes.get(focused).and_then(|p| p.title.clone()).unwrap_or_default();
                                        app.dialog = Some(crate::app::Dialog::RenamePane(*focused, current));
                                    }
                                }
                            });
                        });
                    let shown = out.inner_rect;
                    strip_shown = Some(shown);
                    let offset = out.state.offset.x;
                    let fade_w = 24.0_f32.min(shown.width() / 2.0);
                    if offset > 0.5 {
                        let r = Rect::from_min_max(shown.min, pos2(shown.left() + fade_w, shown.bottom()));
                        kit::fade(ui.painter(), r, p.chrome, false);
                    }
                    if offset + shown.width() < out.content_size.x - 0.5 {
                        let r = Rect::from_min_max(pos2(shown.right() - fade_w, shown.top()), shown.max);
                        kit::fade(ui.painter(), r, p.chrome, true);
                    }
                },
            );
            if kit::icon_button_ex(ui, icons::PLUS, &kit::tip(t("New Tab"), Action::NewTab), false, 26.0).clicked() {
                app.actions.push(Action::NewTab);
            }
        }, None);

        // ----- editor beside the panes ------------------------------------------------------------
        self.last_full_body = Some(body);
        let mut drawer_edge = None;
        let body = if let Some(split_x) = self.editor_split(pid, body) {
            let editor_rect = Rect::from_min_max(pos2(split_x + 1.0, body.top()), body.max);
            let root = self.project(pid).map(|x| x.path.clone()).unwrap_or_default();
            // Review of this project, unless full screen, takes the drawer.
            let review_here = self.review.as_ref().is_some_and(|r| r.project == pid && !r.expanded);
            ui.scope_builder(egui::UiBuilder::new().max_rect(editor_rect), |ui| {
                ui.set_clip_rect(editor_rect);
                if review_here {
                    self.review_drawer(ui);
                } else {
                    self.code_drawer(ui, pid, root);
                }
            });
            // Its edge is dragged to size it (after the panes are drawn, below).
            drawer_edge = Some((split_x, body));
            Rect::from_min_max(body.min, pos2(split_x, body.bottom()))
        } else {
            body
        };

        // ----- the Browser panel beside the panes (the Code drawer closed) --------------------
        // Terminals side by side that would get too narrow show stacked while it is open; with no
        // room even so, it floats over them.
        self.panel_follow(pid);
        let cell = widget::cell_size(ui.ctx(), self.settings.font_size);
        let place = match self.project(pid).and_then(|p| p.tab()) {
            Some(tab) if drawer_edge.is_none() => self.focus_place(pid, &tab.layout, body, cell),
            _ => crate::project::PanelPlace::without(body),
        };
        let told = self.panel_of(pid).is_some_and(|x| x.told_stacked);
        if let Some(panel) = self.panel_mut(pid) {
            panel.stacked = place.stacked;
            panel.floating = place.float;
            panel.told_stacked |= place.stacked;
        }
        if place.stacked && !told {
            self.toast_with(
                crate::app::ToastKind::Info,
                t("Terminals stacked while the browser is open").to_string(),
                Some((t("Keep Side by Side").to_string(), Action::PanelSideBySide(pid))),
            );
        }
        let panel_full = body;
        let body = place.panes;

        // ----- panes -------------------------------------------------------------------------
        let Some(tab) = self.project(pid).and_then(|p| p.tab()) else {
            return;
        };
        let focused = tab.focused;
        let (rects, dividers) = if place.max {
            (Vec::new(), Vec::new())
        } else if place.stacked {
            tab.layout.stacked().arrange(body)
        } else {
            tab.layout.arrange(body)
        };
        let single = rects.len() == 1;
        // Where panes go, for sizing new ones before they start (no resize right after).
        self.last_body = Some(body);
        let pane_areas = rects.clone();

        // Dividers (dragged after the panes are drawn, below).
        for d in &dividers {
            ui.painter().rect_filled(d.rect, CornerRadius::ZERO, p.border);
        }

        let style = self.term_style(ui);
        let scheme = style.scheme;
        self.cell = Some(style.cell);
        let input_enabled = self.terminals_take_keys();
        let window_focused = self.window_focused;
        let now = Instant::now();
        for (pane_id, rect) in rects {
            let is_focused = pane_id == focused;
            // A browser pane: its toolbar is its header, its page the rest.
            if self.browsers.contains_key(&pane_id) {
                self.browser_pane(ui, pane_id, rect, is_focused, single);
                continue;
            }
            // One pane: the tab above says what it is; several: each gets a header.
            let header_h = if single { 0.0 } else { PANE_HEADER_H };
            let header = Rect::from_min_size(rect.min, vec2(rect.width(), header_h));
            let pane_body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
            // The terminal's own background around its text.
            ui.painter().rect_filled(pane_body, CornerRadius::ZERO, scheme.background);
            let mut term_rect = term_rect(rect, single);
            if !single {
                self.pane_header(ui, pane_id, header, is_focused);
            }
            // A restored agent that can be resumed: a banner at the top; a shell that ended: a
            // bar at the bottom. The terminal takes what is left.
            if self.panes.get(&pane_id).is_some_and(|x| x.resume.is_some()) {
                let banner = Rect::from_min_size(pane_body.min, vec2(pane_body.width(), BANNER_H));
                self.resume_banner(ui, pane_id, banner, false);
                term_rect.min.y += BANNER_H;
            }
            if self.panes.get(&pane_id).is_some_and(|x| x.exit.is_some() && !x.alive()) {
                let foot = Rect::from_min_max(pos2(pane_body.left(), pane_body.bottom() - FOOT_H), pane_body.max);
                self.exit_bar(ui, pane_id, foot);
                term_rect.max.y = foot.top() - 2.0;
            }
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("pane:{pane_id}"), term_rect);
            self.pane_rects.insert(pane_id, term_rect);
            self.term_rects.push((pane_id, term_rect.intersect(ui.clip_rect())));
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
            if let Some(bytes) = resp.paste_without_text {
                pane.paste_without_text(bytes);
            }
            if resp.activated && !is_focused {
                self.actions.push(Action::FocusPane(pane_id));
            }
            if resp.reconnect {
                self.actions.push(Action::RestartShell(pane_id));
            }
            if let Some(word) = resp.link
                && let Some(open) = term_link(&word, &pane.cwd)
            {
                self.actions.push(open);
            }
            if request && resp.focused {
                self.focus_request = None;
            }
            // The pane that takes the keyboard (several panes): an accent line on its header.
            if is_focused && !single && window_focused {
                ui.painter().hline(header.x_range(), header.top() + 1.0, Stroke::new(2.0, p.accent));
            }
        }
        if self.focus_request.is_some_and(|f| !self.panes.contains_key(&f)) {
            self.focus_request = None;
        }
        // The dividers take the drag after the panes are drawn, so that a terminal beside one
        // does not take the strip along its edge (a page's web view keeps clear of it).
        for (i, d) in dividers.iter().enumerate() {
            let id = egui::Id::new(("divider", pid, active_tab, d.path.clone()));
            let grab = d.rect.expand(DIVIDER_GRAB - tc_term::layout::DIVIDER / 2.0);
            let r = ui.interact(grab, id, Sense::drag());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("divider:{i}"), grab);
            let _ = i;
            if r.hovered() || r.dragged() {
                ui.ctx().set_cursor_icon(match d.dir {
                    tc_term::layout::SplitDir::Horizontal => egui::CursorIcon::ResizeHorizontal,
                    tc_term::layout::SplitDir::Vertical => egui::CursorIcon::ResizeVertical,
                });
                ui.painter().rect_filled(d.rect, CornerRadius::ZERO, p.accent);
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
        }
        // The Code drawer's edge, dragged to size it: after the panes and the drawer, so that
        // neither takes the strip along it.
        if let Some((split_x, full)) = drawer_edge {
            let divider = Rect::from_min_max(pos2(split_x - DIVIDER_GRAB, full.top()), pos2(split_x + DIVIDER_GRAB, full.bottom()));
            let dr = ui.interact(divider, egui::Id::new(("editor-divider", pid)), Sense::drag());
            #[cfg(debug_assertions)]
            crate::devtest::record("editor-divider", divider);
            if dr.hovered() || dr.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if dr.dragged()
                && let Some(pos) = dr.interact_pointer_pos()
                && let Some(e) = self.editors.get_mut(&pid)
            {
                e.width = ((full.right() - pos.x) / full.width()).clamp(0.2, 0.8);
            }
            if dr.drag_stopped() {
                self.mark_dirty();
            }
            let line = if dr.hovered() || dr.dragged() { p.accent } else { p.border };
            ui.painter().vline(split_x, full.y_range(), Stroke::new(1.0, line));
        }
        // The Browser panel, after the panes: a floating one is drawn over them, and its edge
        // takes the drag along it.
        if let Some(r) = place.panel {
            self.panel_view(ui, pid, r, place.float);
            if !place.max {
                self.panel_edge(ui, pid, r.left(), panel_full);
            }
            // Floating: a click on the terminals under it puts it away.
            if place.float
                && !egui::Popup::is_any_open(ui.ctx())
                && ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pos| place.panes.contains(pos) && !r.contains(pos)))
            {
                self.hide_panel(pid);
            }
        }
        self.tab_drop(ui, &tab_rects, strip_shown, &pane_areas, active_tab);
        self.find_bar(ui);
    }

    /// How terminals are drawn, in Focus and in the Grid alike: the settings' font (its real
    /// bold and italic faces where it has them) and the scheme of the current theme.
    pub(super) fn term_style(&self, ui: &Ui) -> widget::TermStyle<'static> {
        let key = (ui.ctx().cumulative_pass_nr(), ui.visuals().dark_mode);
        if let Some((k, style)) = self.term_style_memo.get()
            && k == key
        {
            return style;
        }
        let choice = tc_term::fonts::effective_choice(&tc_term::prefs::FontChoice {
            name: self.settings.font_name.clone(),
            path: self.settings.font_path.clone(),
            index: 0,
        });
        let style = widget::TermStyle {
            font_size: self.settings.font_size,
            cell: widget::cell_size(ui.ctx(), self.settings.font_size),
            scheme: self.scheme(ui.visuals().dark_mode),
            real_bold: tc_term::fonts::has_real_face(&choice, true, false),
            real_italic: tc_term::fonts::has_real_face(&choice, false, true),
            option_as_meta: self.settings.option_as_meta,
            copy_on_select: self.settings.copy_on_select,
            cursor_blink: self.settings.cursor_blink,
        };
        self.term_style_memo.set(Some((key, style)));
        style
    }

    /// A tab or a pane header being dragged. A tab: a line where it would land in the strip,
    /// or the half of a pane it would take (it becomes a split of the tab in view). A pane: the
    /// half of another pane it would move beside, or the tab strip (it becomes a tab).
    /// `strip_shown`: the part of the strip in view, when it scrolls.
    fn tab_drop(
        &mut self,
        ui: &mut Ui,
        tab_rects: &[Rect],
        strip_shown: Option<Rect>,
        panes: &[(PaneId, Rect)],
        active_tab: usize,
    ) {
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
        // Esc ends the drag where it began.
        let cancel = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let Some(pos) = ui.ctx().pointer_interact_pos().filter(|_| !cancel) else {
            if released || cancel {
                self.tab_drag = None;
                self.pane_drag = None;
                self.tab_spring = None;
                self.end_tab_drag_view();
            }
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let fg = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("tab-drop")));
        let strip = tab_rects
            .iter()
            .copied()
            .reduce(|a, b| a.union(b))
            // Tabs scrolled out of view lie under the toolbar or off the window.
            .map(|r| strip_shown.map_or(r, |s| r.intersect(s)).expand2(vec2(40.0, 8.0)));
        let over_pane = panes.iter().find(|(_, r)| r.contains(pos)).copied();
        let mut drop: Option<Action> = None;
        let in_strip = strip.is_some_and(|r| r.contains(pos));
        if !in_strip {
            self.tab_spring = None;
        }
        if in_strip {
            match source {
                Source::Tab(from) => {
                    // Held over another tab: after a moment that tab shows, to drop beside it.
                    let over = tab_rects.iter().position(|r| r.contains(pos)).filter(|h| *h != from && *h != active_tab);
                    match (over, self.tab_spring) {
                        (Some(h), Some((s, since))) if s == h => {
                            let f = (since.elapsed().as_secs_f32() / TAB_SPRING.as_secs_f32()).min(1.0);
                            if f >= 1.0 {
                                self.tab_spring = None;
                                self.show_tab_while_dragging(h);
                            } else if let Some(r) = tab_rects.get(h) {
                                let x0 = r.left() + 8.0;
                                fg.hline(x0..=x0 + (r.width() - 16.0) * f, r.bottom() - 2.0, Stroke::new(2.0, p.accent));
                            }
                        }
                        (Some(h), _) => self.tab_spring = Some((h, std::time::Instant::now())),
                        (None, _) => self.tab_spring = None,
                    }
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
            // The open tab dragged onto its own panes: the tab looked at before it shows instead
            // (from the next frame), to drop it beside.
            if let Source::Tab(from) = source
                && from == active_tab
                && let Some(partner) = self.drag_partner(from)
            {
                self.show_tab_while_dragging(partner);
            }
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
                // A tab shown for the drop: says which tab goes beside which.
                if let Source::Tab(from) = source
                    && self.tab_drag_home.is_some()
                {
                    let area = panes.iter().map(|(_, r)| *r).reduce(|a, b| a.union(b)).unwrap_or(r);
                    let text = tf("Drop {} beside {}", &[&self.tab_label(from), &self.tab_label(active_tab)]);
                    let g = fg.layout_no_wrap(text, egui::FontId::proportional(12.0), p.text);
                    let pill = Rect::from_center_size(pos2(area.center().x, area.top() + 22.0), g.size() + vec2(24.0, 12.0));
                    fg.rect(pill, CornerRadius::same((pill.height() / 2.0) as u8), p.surface, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
                    fg.galley(pill.min + vec2(12.0, 6.0), g, p.text);
                }
            }
        }
        // What is dragged follows the pointer.
        let label = match source {
            Source::Tab(from) => self.tab_label(from),
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
            self.tab_spring = None;
            match drop {
                // Merged into the tab shown: it stays in view.
                Some(a @ Action::MergeTab { .. }) => {
                    self.tab_drag_home = None;
                    self.actions.push(a);
                }
                other => {
                    self.end_tab_drag_view();
                    if let Some(a) = other {
                        self.actions.push(a);
                    }
                }
            }
        }
    }

    /// A tab's name in the strip: its title, else its focused pane's.
    fn tab_label(&self, i: usize) -> String {
        self.active_project()
            .and_then(|pr| pr.tabs.get(i))
            .map(|tab| tab.title.clone().unwrap_or_else(|| self.leaf_label(tab.focused)))
            .unwrap_or_default()
    }

    /// Notes the tab in view of the active project (by its first pane), and the one before it.
    fn note_tab_seen(&mut self) {
        if self.tab_drag.is_some() {
            return;
        }
        let Some(p) = self.active_project() else { return };
        let Some(first) = p.tab().and_then(|t| t.layout.panes().first().copied()) else { return };
        let id = p.id;
        let seen = self.tab_history.entry(id).or_insert((first, None));
        if seen.0 != first {
            *seen = (first, Some(seen.0));
        }
    }

    /// The tab a dragged open tab goes beside: the one looked at before it, else its neighbour.
    fn drag_partner(&self, from: usize) -> Option<usize> {
        let p = self.active_project()?;
        if p.tabs.len() < 2 {
            return None;
        }
        let before = self.tab_history.get(&p.id).and_then(|(_, b)| *b);
        before
            .and_then(|pane| p.tabs.iter().position(|t| t.layout.contains(pane)))
            .filter(|i| *i != from)
            .or(Some(if from > 0 { from - 1 } else { from + 1 }))
    }

    /// Shows tab `i` while a tab is dragged (no keyboard move, nothing saved); the tab in view
    /// before is remembered, to show again if the drag ends without a drop on a pane.
    fn show_tab_while_dragging(&mut self, i: usize) {
        let Some(home) = self.active_project().filter(|p| i < p.tabs.len() && i != p.active_tab).map(|p| p.active_tab) else {
            return;
        };
        if let Some(p) = self.active.and_then(|id| self.project_mut(id)) {
            p.active_tab = i;
        }
        self.tab_drag_home.get_or_insert(home);
        self.ctx.request_repaint();
    }

    /// Shows again the tab in view before a drag showed another one.
    pub(crate) fn end_tab_drag_view(&mut self) {
        let Some(home) = self.tab_drag_home.take() else { return };
        if let Some(p) = self.active.and_then(|id| self.project_mut(id))
            && home < p.tabs.len()
        {
            p.active_tab = home;
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
        // Kept inside the pane, however narrow (else it covers the pane beside it, or leaves the
        // window): the field gives up width first.
        let field_w = (rect.width() - 200.0).clamp(60.0, 170.0);
        egui::Area::new(egui::Id::new("find-bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2((rect.right() - 330.0).max(rect.left() + 6.0), rect.top() + 6.0))
            .constrain_to(rect)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut search.query)
                                .hint_text(t("Find in this pane"))
                                .desired_width(field_w),
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

    fn pane_header(&mut self, ui: &mut Ui, pane_id: u64, rect: Rect, focused: bool) {
        let p = style::pal(ui);
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("pane-header:{pane_id}"), rect);
        let Some(pane) = self.panes.get(&pane_id) else {
            return;
        };
        let root = self.project(pane.project).map(|x| x.path.clone()).unwrap_or_default();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, if focused { p.surface } else { p.chrome });
        painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
        let s = pane.tracker.status;
        let attention = pane.tracker.wants_attention();
        let c = status::color(s, &p);
        let cy = rect.center().y;
        let resume = pane.resume.clone();
        let label = pane.label();
        let place = pane.place(&root);
        let message = pane.tracker.message.clone().filter(|_| s == Status::NeedsInput);
        let doing = pane.program_title.clone();
        let tip = status::described(&pane.tracker);
        let hovered = ui.rect_contains_pointer(rect);
        let hooks = pane.tracker.hooks_active;
        let hooks_label = pane.tracker.agent.map(|a| a.label().to_string()).unwrap_or_default();
        let unconnected = self.unconnected(pane).map(|a| (pane_id, a));

        // Buttons first, from the right (split and close only while hovered); the text on the
        // left is clipped before them.
        let mut right = rect.right() - 6.0;
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
        let _ = resume;
        // Where the status comes from, when it is the agent's own hooks.
        if hooks {
            let painter = ui.painter();
            let g = painter.layout_no_wrap(t("hooks").to_string(), egui::FontId::proportional(11.0), p.text_faint);
            let gw = g.size().x;
            let x = right - 4.0 - gw;
            painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
            painter.text(pos2(x - 3.0, cy), Align2::RIGHT_CENTER, icons::PLUGS_CONNECTED, style::icon_font(11.0), p.text_faint);
            let r = Rect::from_min_max(pos2(x - 16.0, rect.top()), pos2(right, rect.bottom()));
            ui.interact(r, egui::Id::new(("hooks", pane_id)), Sense::hover())
                .on_hover_text(crate::i18n::tf("Status from {} hooks", &[&hooks_label]));
            right = x - 16.0 - 6.0;
        } else if let Some(agent) = unconnected {
            // An agent without the hooks: it can't say when it needs the user. Click: Connect.
            let painter = ui.painter();
            let g = painter.layout_no_wrap(t("not connected").to_string(), egui::FontId::proportional(11.0), p.warn);
            let gw = g.size().x;
            let b = Rect::from_min_max(pos2(right - 4.0 - gw - 16.0 - 6.0, cy - 10.0), pos2(right - 2.0, cy + 10.0));
            let r = ui.interact(b, egui::Id::new(("unconnected", pane_id)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("unconnected:{pane_id}"), b);
            let painter = ui.painter();
            if r.hovered() {
                painter.rect_filled(b, CornerRadius::same(5), style::tint(p.warn, ui.visuals().dark_mode));
            }
            painter.galley(pos2(b.right() - 4.0 - gw, cy - g.size().y / 2.0), g, p.warn);
            painter.text(pos2(b.right() - 4.0 - gw - 3.0, cy), Align2::RIGHT_CENTER, icons::WARNING, style::icon_font(11.0), p.warn);
            if r.on_hover_text(self.unconnected_tip(&[agent])).clicked() {
                self.actions.push(Action::PlanInstall(agent.1));
            }
            right = b.left() - 6.0;
        }

        let clip = Rect::from_min_max(rect.min, pos2((right - 4.0).max(rect.left()), rect.max.y));
        let painter = painter.with_clip_rect(clip);
        let mut x = rect.left() + 12.0;
        let quiet = matches!(s, Status::Idle | Status::Unknown);
        painter.text(
            pos2(x + 6.5, cy),
            Align2::CENTER_CENTER,
            if quiet { icons::TERMINAL } else { status::icon(s) },
            style::icon_font(13.0),
            if quiet { p.text_faint } else { c },
        );
        x += 13.0 + 7.0;
        let label_c = if focused { p.text } else { p.text_muted };
        let g = painter.layout_no_wrap(label, style::medium(12.5), label_c);
        let lw = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, label_c);
        x += lw + 7.0;
        if let Some(place) = place {
            let g = painter.layout_no_wrap(place, egui::FontId::proportional(style::SMALL), p.text_faint);
            let w = g.size().x;
            painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
            x += w + 7.0;
        }
        let pill = (!quiet).then(|| painter.layout_no_wrap(status::text(s), style::medium(11.0), c));
        // What the program says it is doing, quiet, cut short before the status (a question
        // the pane asks says more).
        let room = clip.right() - x - pill.as_ref().map_or(0.0, |g| g.size().x + 14.0 + 16.0 + 7.0);
        if let Some(doing) = doing.as_ref().filter(|_| message.is_none() && room > 40.0) {
            let mut job = egui::text::LayoutJob::simple_singleline(doing.clone(), egui::FontId::proportional(style::SMALL), p.text_faint);
            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
            let g = painter.layout_job(job);
            let w = g.size().x;
            painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
            x += w + 10.0;
        }
        // The status as a pill, unless the pane is just sitting at its prompt.
        if pill.is_some() {
            let r = kit::paint_pill(&painter, pos2(x, cy), Some(status::icon(s)), &status::text(s), c, style::tint(c, ui.visuals().dark_mode));
            x = r.right() + 7.0;
        }
        if let Some(msg) = message {
            let max = (clip.right() - x).max(0.0);
            let mut job = egui::text::LayoutJob::simple_singleline(msg, egui::FontId::proportional(style::SMALL), p.text_muted);
            job.wrap = egui::text::TextWrapping::truncate_at_width(max);
            let mg = painter.layout_job(job);
            painter.galley(pos2(x, cy - mg.size().y / 2.0), mg, p.text_muted);
        }
        // The whole header (left of its buttons) can be dragged to move the pane.
        let grab = Rect::from_min_max(rect.min, pos2(clip.right(), rect.max.y));
        let tip = match &doing {
            Some(doing) => format!("{doing}\n{tip}"),
            None => tip,
        };
        let label_resp = ui
            .interact(grab, egui::Id::new(("pane-status", pane_id)), Sense::click_and_drag())
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
        label_resp.context_menu(|ui| self.pane_menu(ui, pane_id));
        if attention {
            painter.rect_filled(
                Rect::from_min_size(rect.left_top(), vec2(3.0, rect.height())),
                CornerRadius::ZERO,
                c,
            );
        }
    }

    /// The banner over a restored pane whose agent can continue its last session, in Focus or
    /// in a Grid tile (`tile`).
    pub(super) fn resume_banner(&mut self, ui: &mut Ui, pane_id: PaneId, rect: Rect, tile: bool) {
        let p = style::pal(ui);
        let Some((agent, _)) = self.panes.get(&pane_id).and_then(|x| x.resume.clone()) else { return };
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, p.surface);
        painter.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
        let cy = rect.center().y;
        painter.text(pos2(rect.left() + 12.0 + 7.5, cy), Align2::CENTER_CENTER, icons::CLOCK_COUNTER_CLOCKWISE, style::icon_font(15.0), p.accent_text);
        // Buttons from the right.
        let inner = Rect::from_min_max(pos2(rect.left() + 12.0, rect.top()), pos2(rect.right() - 12.0, rect.bottom()));
        let (resume, dismiss, left) = ui
            .scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let d = kit::button_ex(ui, kit::Kind::Ghost, None, t("Dismiss"), true);
                let r = kit::button_ex(ui, kit::Kind::Primary, Some(icons::PLAY), t("Resume Session"), true);
                (r, d, ui.min_rect().left())
            })
            .inner;
        #[cfg(debug_assertions)]
        {
            let at = if tile { "tile-" } else { "" };
            crate::devtest::record(format!("{at}resume:{pane_id}"), resume.rect);
            crate::devtest::record(format!("{at}dismiss:{pane_id}"), dismiss.rect);
        }
        let text_x = rect.left() + 12.0 + 15.0 + 10.0;
        let mut job = egui::text::LayoutJob::simple_singleline(
            crate::i18n::tf("{} ran here before the restart.", &[agent.label()]),
            egui::FontId::proportional(12.5),
            p.text,
        );
        // Too narrow (a small Grid tile): the icon and the buttons only.
        let room = left - 10.0 - text_x;
        if room >= 40.0 {
            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
            let g = painter.layout_job(job);
            painter.galley(pos2(text_x, cy - g.size().y / 2.0), g, p.text);
        }
        if resume.on_hover_text(tf("Continue {}'s last session here", &[agent.label()])).clicked() {
            self.actions.push(Action::Resume(pane_id));
        }
        if dismiss.clicked() {
            self.actions.push(Action::DismissResume(pane_id));
        }
    }

    /// The bar under a pane whose shell ended (or could not start): what happened, and a new
    /// shell (Enter) or closing the pane.
    fn exit_bar(&mut self, ui: &mut Ui, pane_id: PaneId, rect: Rect) {
        let p = style::pal(ui);
        let Some(pane) = self.panes.get(&pane_id) else { return };
        let failed = pane.start_error.clone();
        let (icon, color, text) = match (&failed, pane.exit) {
            (Some(why), _) => (icons::X_CIRCLE, p.err, tf("The shell could not start: {}", &[why])),
            (None, Some(tc_term::pty::ExitInfo::Code(c))) => (icons::STOP, p.text_faint, tf("Exited with code {}", &[&c.to_string()])),
            (None, Some(tc_term::pty::ExitInfo::Signal(s))) => (icons::STOP, p.text_faint, tf("Ended by signal {}", &[&s.to_string()])),
            _ => (icons::STOP, p.text_faint, t("Shell ended").to_string()),
        };
        let painter = ui.painter_at(rect);
        let fill = if failed.is_some() { style::over(p.chrome, p.err, 0.10) } else { p.chrome };
        painter.rect_filled(rect, CornerRadius::ZERO, fill);
        painter.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, p.border));
        let cy = rect.center().y;
        painter.text(pos2(rect.left() + 12.0 + 7.5, cy), Align2::CENTER_CENTER, icon, style::icon_font(15.0), color);
        let inner = Rect::from_min_max(pos2(rect.left() + 12.0, rect.top()), pos2(rect.right() - 12.0, rect.bottom()));
        let (restart, close, left) = ui
            .scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if failed.is_some() {
                    let r = kit::button_ex(ui, kit::Kind::Secondary, None, t("Try Again"), true);
                    (r, None, ui.min_rect().left())
                } else {
                    let c = kit::button_keys(ui, kit::Kind::Ghost, None, t("Close Pane"), kit::keys(&Action::ClosePane), true);
                    let r = kit::button_keys(ui, kit::Kind::Secondary, None, t("New Shell"), Some("↵"), true);
                    (r, Some(c), ui.min_rect().left())
                }
            })
            .inner;
        let text_x = rect.left() + 12.0 + 15.0 + 10.0;
        let text_c = if failed.is_some() { p.text } else { p.text_muted };
        let mut job = egui::text::LayoutJob::simple_singleline(text, egui::FontId::proportional(12.5), text_c);
        job.wrap = egui::text::TextWrapping::truncate_at_width((left - 10.0 - text_x).max(10.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(text_x, cy - g.size().y / 2.0), g, text_c);
        if restart.clicked() {
            self.actions.push(Action::RestartShell(pane_id));
        }
        if close.is_some_and(|c| c.clicked()) {
            self.actions.push(Action::ClosePaneId(pane_id));
        }
    }

    /// A pane header's right-click menu.
    fn pane_menu(&mut self, ui: &mut Ui, pane_id: PaneId) {
        kit::menu_body(ui);
        if kit::menu_item(ui, Some(icons::PENCIL_SIMPLE), t("Rename…"), Some(t("double-click")), false).clicked() {
            let current = self.panes.get(&pane_id).and_then(|p| p.title.clone()).unwrap_or_default();
            self.dialog = Some(crate::app::Dialog::RenamePane(pane_id, current));
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::SQUARE_SPLIT_HORIZONTAL), t("Split Right"), kit::keys(&Action::SplitRight), false).clicked() {
            self.actions.push(Action::FocusPane(pane_id));
            self.actions.push(Action::SplitRight);
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::ARROWS_OUT_SIMPLE), t("Move to New Tab"), None, false).clicked() {
            self.actions.push(Action::PaneToTab(pane_id));
            ui.close();
        }
        kit::menu_sep(ui);
        if let Some((agent, _)) = self.panes.get(&pane_id).and_then(|x| x.resume.clone())
            && kit::menu_item(ui, Some(icons::CLOCK_COUNTER_CLOCKWISE), &tf("Resume {}", &[agent.label()]), None, false).clicked()
        {
            self.actions.push(Action::Resume(pane_id));
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::BROOM), t("Clear Scrollback"), kit::keys(&Action::ClearScrollback), false).clicked() {
            self.actions.push(Action::FocusPane(pane_id));
            self.actions.push(Action::ClearScrollback);
            ui.close();
        }
        kit::menu_sep(ui);
        if kit::menu_item(ui, Some(icons::X), t("Close Pane"), kit::keys(&Action::ClosePane), true).clicked() {
            self.actions.push(Action::ClosePaneId(pane_id));
            ui.close();
        }
    }

    /// The top bar of the Grid: its title and counts, the filter in the middle, how many tiles
    /// fit a screen at the right.
    pub fn overview_bar(&mut self, ui: &mut Ui) -> Rect {
        use tc_core::model::GridFilter;
        let p = style::pal(ui);
        let full = ui.max_rect();
        let n = self.projects.len();
        let app_worktrees = self.projects.iter().filter(|x| x.is_worktree()).count();
        let waiting = self.attention_queue().len();
        let waiting_projects = self.waiting_projects().len();
        let active = self.active_projects();
        // The filter, in the middle where it is easy to find.
        let options = [
            kit::Seg::new(t("All"), t("Every project's terminal; scroll for more")).count(n, p.text_faint),
            kit::Seg::new(t("Active"), t("Only projects where something runs or waits, or an agent can be resumed")).count(active, p.text_faint),
            kit::Seg::new(t("Waiting"), t("Only projects with something waiting for you")).count(waiting_projects, if waiting_projects > 0 { p.warn } else { p.text_faint }),
        ];
        let width = kit::segmented_width(ui, &options);
        let on = match self.grid_filter {
            GridFilter::All => 0,
            GridFilter::Active => 1,
            GridFilter::Waiting => 2,
        };
        let mut filter = |app: &mut Self, ui: &mut Ui, rect: Rect| {
            #[cfg(debug_assertions)]
            for (r, name) in kit::segment_rects(rect, 3).into_iter().zip(["grid:all", "grid:active", "grid:waiting"]) {
                crate::devtest::record(name, r);
            }
            let id = egui::Id::new("grid-filter");
            if let Some(i) = kit::segmented_in(ui, rect, id, &options, on) {
                app.actions.push(Action::GridFilter([GridFilter::All, GridFilter::Active, GridFilter::Waiting][i]));
            }
        };
        let body = self.top_bar(
            ui,
            full,
            |_, ui| {
                ui.label(RichText::new(t("All Projects")).font(style::semibold(14.0)).color(p.text));
                ui.add_space(-2.0);
                // Worktrees are counted on their own ("7 projects · 2 worktrees · …").
                let worktrees = app_worktrees;
                let mut parts = vec![crate::i18n::count(n - worktrees, t("1 project"), t("{} projects"))];
                if worktrees > 0 {
                    parts.push(crate::i18n::count(worktrees, t("1 worktree"), t("{} worktrees")));
                }
                parts.push(tf("{} active", &[&active.to_string()]));
                parts.push(tf("{} waiting", &[&waiting.to_string()]));
                ui.label(
                    RichText::new(parts.join(" · "))
                    .size(style::SMALL)
                    .color(p.text_faint),
                );
            },
            Some((width, &mut filter)),
        );
        // The same area as the Focus view's (one top bar above both): panes shown in the Grid
        // are sized against it.
        self.last_full_body = Some(body);
        body
    }

    /// The Grid's toolbar: how many tiles a screen holds.
    fn grid_toolbar(&mut self, ui: &mut Ui) {
        let label = match self.grid_columns {
            0 => t("Auto").to_string(),
            n => format!("{n} × {n}"),
        };
        let p = style::pal(ui);
        let text = format!("{}  {label}", icons::SQUARES_FOUR);
        let r = kit::button_ex(ui, kit::Kind::Ghost, None, &format!("{text}  {}", icons::CARET_DOWN), true);
        #[cfg(debug_assertions)]
        crate::devtest::record("grid-layout", r.rect);
        let r = r.on_hover_text(t("Tiles in view"));
        let current = self.grid_columns;
        egui::Popup::menu(&r).align(egui::RectAlign::BOTTOM_END).show(|ui| {
            kit::menu_body(ui);
            ui.set_min_width(170.0);
            kit::menu_header(ui, t("Tiles in view"));
            for (n, l) in [(0, t("Automatic").to_string()), (1, "1 × 1".into()), (2, "2 × 2".into()), (3, "3 × 3".into()), (4, "4 × 4".into())] {
                if kit::menu_item(ui, (current == n).then_some(icons::CHECK), &l, None, false).clicked() {
                    self.actions.push(Action::GridColumns(n));
                    ui.close();
                }
            }
            kit::menu_sep(ui);
            ui.add_enabled_ui(false, |ui| kit::menu_item(ui, Some(icons::INFO), t("Ordered by project name"), None, false));
        });
        let _ = p;
    }
}

/// ⌘-click (Ctrl-click elsewhere) on a link in a terminal: a web or e-mail address opens in
/// the browser (nothing to do here); a `file://` hyperlink or a word that is a file, maybe with
/// a line (`src/app.rs:120`), gives the action that opens it in the editor (a folder is shown in
/// the file manager). Other schemes are ignored: a program must not start apps through them.
pub(super) fn term_link(target: &str, cwd: &std::path::Path) -> Option<Action> {
    // A web address: the project's browser pane (⌥ or the setting: the default browser).
    if crate::platform::openable_url(target) {
        return Some(Action::OpenWeb(target.to_string()));
    }
    if let Some(path) = file_url_path(target) {
        if path.is_dir() {
            crate::platform::reveal_in_folder(&path);
            return None;
        }
        return path.is_file().then_some(Action::OpenFile(path, None));
    }
    if target.contains("://") {
        return None;
    }
    crate::editor::parse_location(target, cwd).map(|(path, line)| Action::OpenFile(path, line))
}

/// The path of a `file://host/path` address (`%20` and the like decoded).
fn file_url_path(url: &str) -> Option<std::path::PathBuf> {
    let rest = url.strip_prefix("file://")?;
    // The host (often this machine's name) comes before the path.
    let path = &rest[rest.find('/')?..];
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (bytes.get(i + 1).and_then(|&b| hex(b)), bytes.get(i + 2).and_then(|&b| hex(b)))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(out).ok()?;
    // Windows: `file:///C:/x` is `C:/x`.
    let path = if cfg!(windows) { path.trim_start_matches('/').to_string() } else { path };
    Some(std::path::PathBuf::from(path))
}

/// The branch in the project header, as a quiet button that opens the Review: its icon, its
/// name, and how many files changed.
fn branch_chip(ui: &mut Ui, branch: &str, changed: usize) -> egui::Response {
    let p = style::pal(ui);
    let font = style::medium(12.0);
    let mut job = egui::text::LayoutJob::default();
    job.append(branch, 0.0, egui::TextFormat::simple(font.clone(), p.text_muted));
    if changed > 0 {
        job.append("·", 5.0, egui::TextFormat::simple(font.clone(), p.text_faint));
        job.append(&tf("{} changed", &[&changed.to_string()]), 5.0, egui::TextFormat::simple(font, p.warn));
    }
    let g = ui.painter().layout_job(job);
    let w = 8.0 + 13.0 + 5.0 + g.size().x + 8.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(style::RADIUS), p.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(pos2(rect.left() + 8.0 + 6.5, rect.center().y), Align2::CENTER_CENTER, icons::GIT_BRANCH, style::icon_font(13.0), p.text_faint);
    ui.painter().galley(pos2(rect.left() + 8.0 + 13.0 + 5.0, rect.center().y - g.size().y / 2.0), g, p.text_muted);
    resp
}

/// A worktree's branch in its project header: "⑂ tc/generics · ↑2 on main · 3 changed". A click
/// reviews it against its base.
fn worktree_chip(ui: &mut Ui, branch: &str, ahead: usize, base: &str, changed: usize) -> egui::Response {
    let p = style::pal(ui);
    let font = style::medium(12.0);
    let mut job = egui::text::LayoutJob::default();
    job.append(branch, 0.0, egui::TextFormat::simple(font.clone(), p.text_muted));
    job.append("·", 5.0, egui::TextFormat::simple(font.clone(), p.text_faint));
    job.append(&tf("↑{} on {}", &[&ahead.to_string(), base]), 5.0, egui::TextFormat::simple(font.clone(), p.text_muted));
    if changed > 0 {
        job.append("·", 5.0, egui::TextFormat::simple(font.clone(), p.text_faint));
        job.append(&tf("{} changed", &[&changed.to_string()]), 5.0, egui::TextFormat::simple(font, p.warn));
    }
    let g = ui.painter().layout_job(job);
    let w = 8.0 + 13.0 + 5.0 + g.size().x + 8.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(style::RADIUS), p.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(pos2(rect.left() + 8.0 + 6.5, rect.center().y), Align2::CENTER_CENTER, icons::GIT_FORK, style::icon_font(13.0), p.text_faint);
    ui.painter().galley(pos2(rect.left() + 8.0 + 13.0 + 5.0, rect.center().y - g.size().y / 2.0), g, p.text_muted);
    resp
}

/// The address a local server is shown by: its host and port (`localhost:5173`).
pub(super) fn server_label(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
}

/// The local servers a project's panes started (that answer), in the project header: the
/// first one's address with a green dot, "+N" for the others. A click opens it in the browser;
/// with several, a menu lists them all with the pane each runs in. Returns the address
/// clicked (it opens in the project's browser pane; with ⌥, in the default browser).
fn server_chip(ui: &mut Ui, servers: &[(String, String)]) -> Option<String> {
    let p = style::pal(ui);
    let (url, pane) = servers.first()?;
    let font = style::medium(12.0);
    let mut job = egui::text::LayoutJob::default();
    job.append(server_label(url), 0.0, egui::TextFormat::simple(font.clone(), p.text_muted));
    if servers.len() > 1 {
        job.append(&format!("+{}", servers.len() - 1), 5.0, egui::TextFormat::simple(font, p.text_faint));
    }
    let g = ui.painter().layout_job(job);
    let w = 8.0 + 7.0 + 6.0 + g.size().x + 6.0 + 12.0 + 8.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    #[cfg(debug_assertions)]
    crate::devtest::record("server-chip", rect);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(style::RADIUS), p.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let cy = rect.center().y;
    ui.painter().circle_filled(pos2(rect.left() + 8.0 + 3.5, cy), 3.5, p.ok);
    let x = rect.left() + 8.0 + 7.0 + 6.0;
    ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g.clone(), p.text_muted);
    ui.painter().text(pos2(x + g.size().x + 6.0 + 6.0, cy), Align2::CENTER_CENTER, icons::ARROW_SQUARE_OUT, style::icon_font(12.0), p.text_faint);
    if servers.len() == 1 {
        let resp = resp.on_hover_text(format!("{}\n{pane}", tf("Open {} in the browser", &[url])));
        return resp.clicked().then(|| url.clone());
    }
    let resp = resp.on_hover_text(t("Local servers"));
    let mut chosen = None;
    egui::Popup::menu(&resp).align(egui::RectAlign::BOTTOM_START).show(|ui| {
        kit::menu_body(ui);
        ui.set_min_width(220.0);
        kit::menu_header(ui, t("Local servers"));
        for (url, pane) in servers {
            let r = kit::menu_item(ui, Some(icons::ARROW_SQUARE_OUT), server_label(url), Some(pane), false);
            if r.on_hover_text(tf("Open {} in the browser", &[url])).clicked() {
                chosen = Some(url.clone());
                ui.close();
            }
        }
    });
    chosen
}

/// A terminal tab in the project header: its status icon (the terminal's when nothing runs or
/// waits), its label, a close button when there are several tabs, and a thin progress line
/// when a program in it reports one. Returns the tab's response and whether close was clicked.
#[allow(clippy::too_many_arguments)]
fn terminal_tab(
    ui: &mut Ui,
    label: &str,
    waiting: Option<Status>,
    on: bool,
    dragging: bool,
    closable: bool,
    progress: Option<(u8, Option<u8>)>,
    web: Option<bool>,
) -> (egui::Response, bool) {
    let p = style::pal(ui);
    let font = style::medium(12.5);
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), font, egui::Color32::PLACEHOLDER);
    job.wrap = egui::text::TextWrapping::truncate_at_width(160.0);
    let galley = ui.painter().layout_job(job);
    let close_w = if closable { 7.0 + 16.0 } else { 0.0 };
    let w = 9.0 + 13.0 + 7.0 + galley.size().x + close_w + 8.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click_and_drag());
    let close_rect = Rect::from_center_size(pos2(rect.right() - 8.0 - 8.0, rect.center().y), vec2(16.0, 16.0));
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
        None => (
            match web {
                // A page let go to save memory: it loads again when shown.
                Some(true) => icons::MOON,
                Some(false) => icons::GLOBE,
                None => icons::TERMINAL,
            },
            if on { p.accent_text } else { p.text_faint },
        ),
    };
    painter.text(pos2(rect.left() + 9.0 + 6.5, rect.center().y), Align2::CENTER_CENTER, icon, style::icon_font(13.0), color);
    let text_color = if on { p.text } else { p.text_muted };
    painter.galley(pos2(rect.left() + 9.0 + 13.0 + 7.0, rect.center().y - galley.size().y / 2.0), galley, text_color);
    // What a program in the tab reports of its progress.
    if let Some((state, value)) = progress {
        let track = Rect::from_min_max(pos2(rect.left() + 8.0, rect.bottom() - 4.0), pos2(rect.right() - 8.0, rect.bottom() - 2.0));
        painter.rect_filled(track, CornerRadius::same(1), p.subtle);
        let c = if state == 2 { p.err } else if state == 4 { p.warn } else { p.info };
        let f = value.map_or(0.3, |v| v.min(100) as f32 / 100.0);
        painter.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * f, 2.0)), CornerRadius::same(1), c);
    }
    let mut closed = false;
    if let Some(c) = close_resp {
        let hot = c.hovered();
        if hot {
            painter.rect_filled(close_rect, CornerRadius::same(4), p.subtle);
        }
        painter.text(
            close_rect.center(),
            Align2::CENTER_CENTER,
            icons::X,
            style::icon_font(11.0),
            if hot { p.text } else { p.text_faint },
        );
        closed = c.on_hover_text(t("Close Tab")).clicked();
    }
    (resp, closed)
}

/// Where a pane's terminal text goes inside its rect: below its header (when the tab has
/// several panes), with a little padding.
pub fn term_rect(pane: Rect, single: bool) -> Rect {
    let top = pane.top() + if single { 0.0 } else { PANE_HEADER_H };
    Rect::from_min_max(
        pos2(pane.left() + TERM_PAD, top),
        pos2(pane.right() - TERM_PAD, pane.bottom() - 2.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_open_only_what_they_safely_can() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a b.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        // `file://host/Users/…` here, `file:///C:/Users/…` on Windows.
        let path = file.to_string_lossy().replace('\\', "/").replace(' ', "%20");
        let url = if path.starts_with('/') { format!("file://host{path}") } else { format!("file:///{path}") };
        assert_eq!(file_url_path(&url).as_deref(), Some(file.as_path()));
        assert!(matches!(term_link(&url, dir.path()), Some(Action::OpenFile(p, None)) if p == file));
        // Schemes that could start other apps, and files that are not there: nothing.
        for target in ["javascript:alert(1)", "smb://server/share", "ssh://host", "file:///nonexistent/x.rs", "missing.rs:3"] {
            assert!(term_link(target, dir.path()).is_none(), "{target}");
        }
        std::fs::write(dir.path().join("main.rs"), "x\n").unwrap();
        assert!(matches!(term_link("main.rs:2", dir.path()), Some(Action::OpenFile(_, Some(2)))));
    }
}
