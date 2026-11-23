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
/// A pane's header, inside its sheet: what runs there, its status, active profile, since when.
pub const PANE_HEADER_H: f32 = 34.0;
/// The Code editor stage's head (`Files | Changes`, Quick Open, close).
const DRAWER_HEAD_H: f32 = 38.0;
/// The secondary drawer's tab bar (`Terminals | Agent Logs | Problems`, dock controls).
pub const DRAWER_BAR_H: f32 = 32.0;
/// Inline rate-limit failover & recovery banner inside a paused pane.
const RATE_LIMIT_H: f32 = 36.0;
/// Inline tool approval quick-action bar when an agent waits for confirmation.
const APPROVAL_H: f32 = 32.0;
/// How far either side of a divider (or the Code editor's edge) a drag takes it.
pub(crate) const DIVIDER_GRAB: f32 = 5.0;
/// Space between a sheet's edge and where the terminal's own padding starts (16 px to the text
/// at the sides, as the design has it).
const TERM_PAD: f32 = 6.0;
/// The banner over a restored pane whose agent can be resumed, and the bar under one whose
/// shell ended.
const BANNER_H: f32 = 38.0;
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
        // The bar sits on the window's base, like the sidebar beside it: no fill, no line.
        let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
        super::view::window_drag_area(ui, header, "content-top");

        // Toolbar, from the right.
        let inner = Rect::from_min_max(pos2(header.left() + 6.0, header.top()), pos2(header.right() - 10.0, header.bottom()));
        let used = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    self.toolbar(ui);
                    ui.min_rect()
                },
            )
            .inner;

        // Middle part: centred on the bar, moved left when it would reach the toolbar.
        let mut left_end = used.left() - 10.0;
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
        let fullscreen = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
        if self.settings.sidebar_hidden || self.zen_mode {
            left_rect.min.x = header.left() + crate::platform::traffic_light_inset(fullscreen).max(10.0);
        } else if self.settings.sidebar_compact {
            // The window's buttons reach past the narrow sidebar.
            let lights = crate::platform::traffic_light_inset(fullscreen) - super::sidebar::COMPACT_W;
            left_rect.min.x = left_rect.min.x.max(header.left() + lights.max(0.0) + 4.0);
        }
        left_rect.max.x = left_rect.max.x.max(left_rect.min.x);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(left_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.set_clip_rect(left_rect.intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 4.0;
                if !self.zen_mode
                    && self.settings.sidebar_hidden
                    && kit::icon_button_ex(ui, icons::SIDEBAR_SIMPLE, &kit::tip(t("Show Sidebar"), Action::ToggleSidebar), false, 26.0)
                        .clicked()
                {
                    self.actions.push(Action::ToggleSidebar);
                }
                // The views' switch lives at the top of the sidebar; with the sidebar hidden or
                // narrow, here.
                if !self.zen_mode && (self.settings.sidebar_hidden || self.settings.sidebar_compact) && !self.projects.is_empty() {
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
        let on = match self.view {
            View::Focus => 0,
            View::Grid => 1,
            _ => usize::MAX,
        };
        if let Some(i) = kit::segmented(ui, &options, on) {
            self.actions.push(Action::SetView(if i == 1 { View::Grid } else { View::Focus }));
        }
    }

    /// Right side of the top bar (added right to left): what acts on the active project.
    fn toolbar(&mut self, ui: &mut Ui) {
        let compact = ui.available_width() < 760.0;
        if self.zen_mode {
            let exit = kit::button_keys(ui, kit::Kind::Secondary, Some(icons::ARROWS_IN), t("Exit Zen"), kit::keys(&Action::ToggleZenMode), true);
            #[cfg(debug_assertions)]
            crate::devtest::record("toggle-zen", exit.rect);
            if exit.clicked() {
                self.actions.push(Action::ToggleZenMode);
            }
            return;
        }
        if crate::ade::is_management_view(self.view) {
            self.telemetry_pill_sized(ui, compact);
            return;
        }
        if self.view == View::Grid && !self.projects.is_empty() {
            self.grid_toolbar(ui);
            kit::vsep(ui);
            self.telemetry_pill_sized(ui, compact);
            return;
        }
        if self.view != View::Focus || self.active.is_none() {
            self.telemetry_pill_sized(ui, compact);
            return;
        }
        let review_here = self.active.is_some_and(|pid| self.review.as_ref().is_some_and(|r| r.project == pid && !r.expanded));
        let history_here = self.active.is_some_and(|pid| self.history.as_ref().is_some_and(|h| h.project == pid));
        let editor_on = !self.drawer_maximized && (review_here || history_here || self.active.and_then(|id| self.editors.get(&id)).is_some_and(|e| e.visible));
        let drawer_on = !editor_on || self.settings.drawer_open;
        let drawer_icon = if self.is_dock_right() { icons::COLUMNS } else { icons::ROWS };
        // Added right to left: Terminal Panel, Code Editor, Browser, Telemetry Pill, Run Agent.
        let dr = kit::icon_button_ex(ui, drawer_icon, &kit::tip(t("Show/Hide Terminal Panel"), Action::ToggleDrawer), drawer_on, 28.0);
        #[cfg(debug_assertions)]
        crate::devtest::record("toggle-drawer", dr.rect);
        if dr.clicked() {
            self.actions.push(Action::ToggleDrawer);
        }
        let e = kit::icon_button_ex(ui, icons::CODE, &kit::tip(t("Code"), Action::ToggleEditor), editor_on, 28.0);
        #[cfg(debug_assertions)]
        crate::devtest::record("toggle-editor", e.rect);
        if e.clicked() {
            self.actions.push(Action::ToggleEditor);
        }
        self.browser_button(ui);
        kit::vsep(ui);
        self.telemetry_pill_sized(ui, compact);
        kit::vsep(ui);
        let run_keys = (!compact).then(|| kit::keys(&Action::PickPreset)).flatten();
        let run = kit::button_keys(ui, kit::Kind::Primary, Some(icons::PLAY), t("Run Agent"), run_keys, false);
        #[cfg(debug_assertions)]
        crate::devtest::record("run-agent", run.rect);
        if run.on_hover_text(kit::tip(t("Start an agent in a new pane"), Action::PickPreset)).clicked() {
            self.actions.push(Action::PickPreset);
        }
    }

    /// Standard top-left back arrow button (`←`) for management pages (`Profiles`, `Monitor`,
    /// `Phone Pairing`), matching `Review` and supporting `Esc` when no overlay or input is active.
    pub fn nav_back_button(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.x = 6.0;
        let tip = format!("{}   Esc", t("Back"));
        let back = kit::icon_button_ex(ui, icons::ARROW_LEFT, &tip, false, 26.0);
        #[cfg(debug_assertions)]
        crate::devtest::record("nav-back", back.rect);
        let mut go_back = back.clicked();
        let no_overlay = self.dialog.is_none()
            && self.palette.is_none()
            && !self.attention_open
            && !self.monitor_popover
            && !self.updater.dialog_open
            && !egui::Popup::is_any_open(ui.ctx())
            && !ui.ctx().egui_wants_keyboard_input();
        let drag_id = egui::Id::new("profiles-drag-idx");
        let profiles_dragging = self.view == View::Profiles
            && ui.data(|d| d.get_temp::<usize>(drag_id).is_some());
        if !go_back && no_overlay && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if profiles_dragging {
                ui.data_mut(|d| d.remove::<usize>(drag_id));
            } else if self.view == View::Profiles && self.profile_draft.is_some() {
                self.profile_draft = None;
            } else {
                go_back = true;
            }
        }
        if go_back {
            let target = if crate::ade::is_management_view(self.prev_view) { View::Focus } else { self.prev_view };
            self.actions.push(Action::SetView(target));
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
                let agent = self.panes.get(&tab.focused).and_then(|pane| pane.tracker.agent).filter(|a| super::agent_logo::has(*a));
                (i, label, waiting, doing, tab.focused, progress, web, agent)
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
        let letters = project.letters();
        let changed = git.as_ref().map_or(0, |g| g.changed);
        // The branch's pull / merge request (read with gh or glab), while it is open.
        let ship_state = self.ship.get(&pid);
        let pr_forge = ship_state
            .and_then(|s| s.facts.as_ref())
            .and_then(|f| f.remote.as_ref())
            .and_then(|r| r.web.as_ref())
            .map(|w| w.forge);
        let pr = ship_state
            .filter(|s| s.facts.as_ref().and_then(|f| f.branch.as_deref()) == git.as_ref().map(|g| g.branch.as_str()))
            .and_then(|s| s.pr.clone())
            .filter(|pr| matches!(pr.state, tc_git::ship::PrState::Open | tc_git::ship::PrState::Draft));
        let body = self.top_bar(ui, full, |app, ui| {
            let dark = ui.visuals().dark_mode;
            let bar_right = ui.max_rect().right();
            let bar_w = ui.max_rect().width();
            // The project: its tile, its name, a caret; a click opens its menu.
            let title_text = match (&parent, &worktree) {
                (Some(_), Some(_)) => None,
                _ => Some(name.clone()),
            };
            if let Some(title_text) = title_text {
                let mut job = egui::text::LayoutJob::simple_singleline(title_text.clone(), style::semibold(14.0), p.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width((bar_w * 0.34).clamp(72.0, 220.0));
                let g = ui.painter().layout_job(job);
                let (rect, resp) = ui.allocate_exact_size(vec2(4.0 + 20.0 + 8.0 + g.size().x + 8.0 + 10.0 + 6.0, 30.0), Sense::click());
                let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
                if resp.hovered() || open {
                    ui.painter().rect_filled(rect, CornerRadius::same(8), p.hover);
                }
                let tile = Rect::from_min_size(pos2(rect.left() + 4.0, rect.center().y - 10.0), vec2(20.0, 20.0));
                kit::paint_project_tile(ui.painter(), tile, &letters, color, dark);
                ui.painter().galley(pos2(tile.right() + 8.0, rect.center().y - g.size().y / 2.0), g.clone(), p.text);
                ui.painter().text(pos2(tile.right() + 8.0 + g.size().x + 8.0 + 5.0, rect.center().y), Align2::CENTER_CENTER, icons::CARET_DOWN, style::icon_font(10.0), p.text_faint);
                #[cfg(debug_assertions)]
                crate::devtest::record("project-title", rect);
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                egui::Popup::menu(&resp).align(egui::RectAlign::BOTTOM_START).show(|ui| {
                    app.project_menu(ui, pid, &title_text);
                });
            }
            if let (Some((parent_id, parent_name)), Some(wt)) = (&parent, &worktree) {
                // "storefront › cart-badge": the project opens it; the worktree's name opens
                // its menu.
                let (r, _) = ui.allocate_exact_size(vec2(24.0, 30.0), Sense::hover());
                let tile = Rect::from_center_size(r.center() + vec2(2.0, 0.0), vec2(20.0, 20.0));
                kit::paint_project_tile(ui.painter(), tile, &letters, color, dark);
                let r = ui.add(egui::Label::new(RichText::new(parent_name).font(style::medium(14.0)).color(p.text_muted)).sense(Sense::click()));
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tf("Open {}", &[parent_name])).clicked() {
                    app.actions.push(Action::SelectProject(*parent_id));
                }
                ui.label(RichText::new(icons::CARET_RIGHT).font(style::icon_font(11.0)).color(p.text_faint));
                let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), style::semibold(14.0), p.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width((bar_w * 0.28).clamp(64.0, 180.0));
                let g = ui.painter().layout_job(job);
                let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 12.0 + 10.0 + 4.0, 28.0), Sense::click());
                let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
                if resp.hovered() || open {
                    ui.painter().rect_filled(rect, CornerRadius::same(style::RADIUS), p.hover);
                }
                ui.painter().galley(pos2(rect.left() + 6.0, rect.center().y - g.size().y / 2.0), g.clone(), p.text);
                ui.painter().text(pos2(rect.left() + 6.0 + g.size().x + 4.0 + 5.0, rect.center().y), Align2::CENTER_CENTER, icons::CARET_DOWN, style::icon_font(10.0), p.text_faint);
                #[cfg(debug_assertions)]
                crate::devtest::record("worktree-title", rect);
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t("Worktree"));
                egui::Popup::menu(&resp).align(egui::RectAlign::BOTTOM_START).show(|ui| {
                    ui.set_min_width(250.0);
                    ui.set_max_width(250.0);
                    app.worktree_menu(ui, pid);
                });
                if let Some(g) = &git
                    && bar_right - ui.cursor().min.x > 260.0
                {
                    let r = worktree_chip(ui, &wt.branch, g.ahead, &wt.base, changed);
                    #[cfg(debug_assertions)]
                    crate::devtest::record("branch-chip", r.rect);
                    if r.on_hover_text(kit::tip(&tf("Review Against {}", &[&wt.base]), Action::Review)).clicked() {
                        app.actions.push(Action::Review);
                    }
                }
            } else if let Some(g) = &git
                && bar_right - ui.cursor().min.x > 220.0
            {
                let label = if changed > 0 {
                    format!("{} · {}", g.branch, tf("{} changed", &[&changed.to_string()]))
                } else {
                    g.branch.clone()
                };
                let r = kit::chip(ui, Some(icons::GIT_BRANCH), &label, true, None);
                #[cfg(debug_assertions)]
                crate::devtest::record("branch-chip", r.rect);
                if r.on_hover_text(kit::tip(t("Review Changes…"), Action::Review)).clicked() {
                    app.actions.push(Action::Review);
                }
            }
            if let Some(pr) = &pr
                && bar_right - ui.cursor().min.x > 250.0
            {
                let r = pr_chip(ui, pr, pr_forge);
                #[cfg(debug_assertions)]
                crate::devtest::record("pr-chip", r.rect);
                let is_gitlab = pr_forge == Some(tc_git::ship::Forge::GitLab);
                let tip = match pr.checks {
                    tc_git::ship::Checks::Pending => t("Checks are running"),
                    tc_git::ship::Checks::Passed => t("Checks passed"),
                    tc_git::ship::Checks::Failed => t("A check failed"),
                    tc_git::ship::Checks::None => if is_gitlab { t("Open the merge request") } else { t("Open the pull request") },
                };
                if r.on_hover_text(tip).clicked() {
                    app.actions.push(Action::OpenWeb(pr.url.clone()));
                }
            }
            if !servers.is_empty()
                && bar_right - ui.cursor().min.x > 240.0
                && let Some(url) = server_chip(ui, &servers)
            {
                app.actions.push(Action::OpenWeb(url));
            }
            kit::vsep(ui);
            // More tabs than fit scroll sideways (wheel or trackpad), a fade at the edge that
            // hides some; the tab in view is scrolled to when it changes. "+" stays outside.
            let row = ui.max_rect();
            let plus_w = 22.0 + ui.spacing().item_spacing.x;
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
                                for (i, label, waiting, doing, focused, progress, web, agent) in &tabs {
                                    let dragging = app.tab_drag == Some(*i);
                                    let on = *i == active_tab;
                                    let (r, close) =
                                        terminal_tab(ui, label, waiting.as_ref().map(|w| w.0), on, dragging, tabs.len() > 1, *progress, *web, *agent);
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
            if kit::icon_button_ex(ui, icons::PLUS, &kit::tip(t("New Tab"), Action::NewTab), false, 22.0).clicked() {
                app.actions.push(Action::NewTab);
            }
        }, None);

        // The sheets sit on the base below the bar, the window's edge beside them (and a margin
        // at the left too when the sidebar is hidden).
        let body = sheets_area(body, self.settings.sidebar_hidden || self.zen_mode);

        // ----- Wide Code Editor primary stage + collapsible Bottom / Right secondary drawer -----
        self.last_full_body = Some(body);
        let (editor_stage, drawer_stage) = self.workspace_split(pid, body);
        let dock_right = self.is_dock_right();
        let editor_active = editor_stage.is_some() || self.drawer_maximized;

        if let Some(stage) = editor_stage {
            #[cfg(debug_assertions)]
            crate::devtest::record("editor-stage", stage);
            paint_sheet(ui.painter(), stage, &p, SheetLook::Plain);
            let root = self.project(pid).map(|x| x.path.clone()).unwrap_or_default();
            let review_here = self.review.as_ref().is_some_and(|r| r.project == pid && !r.expanded);
            let history_here = !review_here && self.history.as_ref().is_some_and(|h| h.project == pid);
            let mode = if review_here { 1 } else if history_here { 2 } else { 0 };
            let head = Rect::from_min_size(stage.min, vec2(stage.width(), DRAWER_HEAD_H));
            self.drawer_head(ui, head, pid, mode);
            let editor_rect = Rect::from_min_max(pos2(stage.left() + 1.0, head.bottom()), pos2(stage.right() - 1.0, stage.bottom() - 1.0));
            ui.painter().hline(editor_rect.x_range(), editor_rect.top() + 0.5, Stroke::new(1.0, p.border));
            let editor_rect = Rect::from_min_max(pos2(editor_rect.left(), editor_rect.top() + 1.0), editor_rect.max);
            // Re-check mode after `drawer_head` so switching Files | Changes | History paints immediately with no 1-frame blank flash.
            let review_now = self.review.as_ref().is_some_and(|r| r.project == pid && !r.expanded);
            let history_now = !review_now && self.history.as_ref().is_some_and(|h| h.project == pid);
            ui.scope_builder(egui::UiBuilder::new().max_rect(editor_rect), |ui| {
                ui.set_clip_rect(editor_rect);
                if review_now {
                    self.review_drawer(ui);
                } else if history_now {
                    self.history_drawer(ui);
                } else {
                    self.code_drawer(ui, pid, root);
                }
            });
        }

        // Track splitter handle between Code Editor stage and Secondary Drawer.
        let split_handle = match (editor_stage, drawer_stage) {
            (Some(ed), Some(dr)) => Some((ed, dr, dock_right)),
            _ => None,
        };

        // If the secondary drawer is collapsed (e.g. Zen Mode or user closed it), finish cleanly.
        let Some(drawer_rect) = drawer_stage else {
            self.tab_drop(ui, &tab_rects, strip_shown, &[], active_tab);
            return;
        };

        // Secondary Drawer chrome: when Code Editor is active (even if the panel is maximized)
        // or when viewing Logs/Problems, render the drawer tab strip + dock/restore controls.
        let show_drawer_bar = editor_active || self.drawer_tab != tc_core::model::DrawerTab::Terminals;
        let drawer_content = if show_drawer_bar {
            #[cfg(debug_assertions)]
            crate::devtest::record("secondary-drawer", drawer_rect);
            let bar = Rect::from_min_size(drawer_rect.min, vec2(drawer_rect.width(), DRAWER_BAR_H));
            self.secondary_drawer_bar(ui, bar, pid, editor_active);
            Rect::from_min_max(pos2(drawer_rect.left(), bar.bottom() + 2.0), drawer_rect.max)
        } else {
            drawer_rect
        };

        // Non-terminal drawer tabs (`Agent Logs`, `Problems`).
        if self.drawer_tab != tc_core::model::DrawerTab::Terminals {
            paint_sheet(ui.painter(), drawer_content, &p, SheetLook::Plain);
            let inner = drawer_content.shrink(1.0);
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                ui.set_clip_rect(inner);
                match self.drawer_tab {
                    tc_core::model::DrawerTab::Logs => self.agent_logs_drawer(ui, pid),
                    tc_core::model::DrawerTab::Problems => self.problems_panel(ui, pid, inner),
                    tc_core::model::DrawerTab::Terminals => {}
                }
            });
            if let Some((ed, dr, right)) = split_handle {
                self.paint_workspace_splitter(ui, pid, body, ed, dr, right);
            }
            self.tab_drop(ui, &tab_rects, strip_shown, &[], active_tab);
            return;
        }

        // ----- the Browser panel beside the panes (when Code editor is hidden) ----------------
        let body_panes = drawer_content;
        self.panel_follow(pid);
        let cell = widget::cell_size(ui.ctx(), self.settings.font_size);
        let place = match self.project(pid).and_then(|p| p.tab()) {
            Some(tab) if !editor_active => self.focus_place(pid, &tab.layout, body_panes, cell),
            _ => crate::project::PanelPlace::without(body_panes),
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
        let panel_full = body_panes;
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

        // Dividers are the gaps between sheets: nothing to draw until one is grabbed (below).

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
                let look = if is_focused && !single && window_focused { SheetLook::Focused } else { SheetLook::Plain };
                paint_sheet(ui.painter(), rect, &p, look);
                self.browser_pane(ui, pane_id, rect, is_focused, single);
                continue;
            }
            // Each pane is a sheet: its header (what runs, its status, since when), then the
            // terminal. A pane that waits for the user is ringed in amber.
            let waiting = self.panes.get(&pane_id).is_some_and(|x| x.tracker.status == Status::NeedsInput);
            let look = if waiting {
                SheetLook::Waiting
            } else if is_focused && !single && window_focused {
                SheetLook::Focused
            } else {
                SheetLook::Plain
            };
            paint_sheet_fill(ui.painter(), rect, scheme.background, &p, look);
            let header = Rect::from_min_size(rect.min, vec2(rect.width(), PANE_HEADER_H));
            let mut term_rect = term_rect(rect, single);
            self.pane_header(ui, pane_id, header, is_focused);
            // Rate-limit failover & recovery banner, restored-session resume banner, or tool approval bar.
            if self.panes.get(&pane_id).is_some_and(|x| x.rate_limit.is_some()) {
                let banner = Rect::from_min_size(pos2(rect.left() + 8.0, header.bottom()), vec2((rect.width() - 16.0).max(40.0), RATE_LIMIT_H));
                self.rate_limit_banner(ui, pane_id, banner);
                term_rect.min.y = banner.bottom() + 2.0;
            } else if self.panes.get(&pane_id).is_some_and(|x| x.resume.is_some()) {
                let banner = Rect::from_min_size(pos2(rect.left() + 8.0, header.bottom()), vec2((rect.width() - 16.0).max(40.0), BANNER_H));
                self.resume_banner(ui, pane_id, banner, false);
                term_rect.min.y = banner.bottom() + 2.0;
            }
            if self.panes.get(&pane_id).is_some_and(|x| x.exit.is_some() && !x.alive()) {
                let foot = Rect::from_min_max(pos2(rect.left() + 6.0, rect.bottom() - 6.0 - FOOT_H), pos2(rect.right() - 6.0, rect.bottom() - 6.0));
                self.exit_bar(ui, pane_id, foot);
                term_rect.max.y = foot.top() - 2.0;
            } else if self.panes.get(&pane_id).is_some_and(|x| {
                x.tracker.status == Status::NeedsInput
                    && x.tracker.agent.is_some()
                    && x.rate_limit.as_ref().is_none_or(|b| b.auto_switched_to.is_some())
            }) && term_rect.height() > 110.0
            {
                let bar = Rect::from_min_max(pos2(rect.left() + 8.0, rect.bottom() - 6.0 - APPROVAL_H), pos2(rect.right() - 8.0, rect.bottom() - 6.0));
                self.approval_bar(ui, pane_id, bar);
                term_rect.max.y = bar.top() - 2.0;
            }
            term_rect.max.y = term_rect.max.y.max(term_rect.min.y);
            term_rect.max.x = term_rect.max.x.max(term_rect.min.x);
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("pane:{pane_id}"), term_rect);
            self.pane_rects.insert(pane_id, term_rect);
            self.term_rects.push((pane_id, term_rect.intersect(ui.clip_rect())));
            let request = self.focus_request == Some(pane_id);
            // The terminal is to take the keyboard: the page the user typed in lets go of it.
            if request && self.browser_focus.is_some() {
                self.keyboard_to_app();
            }
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
                // A 2 px accent line down the middle of the gap.
                let line = match d.dir {
                    tc_term::layout::SplitDir::Horizontal => Rect::from_center_size(d.rect.center(), vec2(2.0, d.rect.height() - 16.0)),
                    tc_term::layout::SplitDir::Vertical => Rect::from_center_size(d.rect.center(), vec2(d.rect.width() - 16.0, 2.0)),
                };
                ui.painter().rect_filled(line, CornerRadius::same(1), p.accent);
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
        // Resizable splitter between the Wide Code Editor stage and the Secondary Drawer.
        if let Some((ed, dr, right)) = split_handle {
            let full = self.last_full_body.unwrap_or(body);
            self.paint_workspace_splitter(ui, pid, full, ed, dr, right);
        }
        // The Browser panel, after the panes: a floating one is drawn over them, and its edge
        // takes the drag along it.
        if let Some(r) = place.panel {
            self.panel_view(ui, pid, r, place.float);
            if !place.max {
                self.panel_edge(ui, pid, r.left(), panel_full, place.float);
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
        let take_keys = !self.overlay_open() && !self.updater.dialog_open && !self.review_full();
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
        let mut close = take_keys && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        // Kept inside the pane, however narrow (else it covers the pane beside it, or leaves the
        // window): the field gives up width first.
        let field_w = (rect.width() - 200.0).clamp(60.0, 170.0);
        let dark = ui.visuals().dark_mode;
        egui::Area::new(egui::Id::new("find-bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2((rect.right() - 330.0).max(rect.left() + 6.0), rect.top() + 6.0))
            .constrain_to(rect)
            .show(ui.ctx(), |ui| {
                egui::Frame::NONE
                    .fill(p.overlay)
                    .stroke(Stroke::new(1.0, p.border_strong))
                    .corner_radius(CornerRadius::same(8))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 8],
                        blur: 20,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(if dark { 120 } else { 32 }),
                    })
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let r = kit::field(
                                ui,
                                egui::Id::new("pane-find-input"),
                                &mut search.query,
                                Some(icons::MAGNIFYING_GLASS),
                                t("Find in this pane"),
                                field_w,
                                true,
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

    /// A pane's header, inside its sheet (no line under it): the agent's logo (or the
    /// terminal's icon), the pane's name, its status as glyph and word, what it does or asks
    /// ("— Bash · flutter test"); at the right where it runs, "hooks" when the status comes
    /// from the agent's own hooks, how long the status has lasted, and the pane's menu. The
    /// whole header drags the pane; double-click renames it.
    fn pane_header(&mut self, ui: &mut Ui, pane_id: u64, rect: Rect, focused: bool) {
        let p = style::pal(ui);
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("pane-header:{pane_id}"), rect);
        let Some(pane) = self.panes.get(&pane_id) else {
            return;
        };
        let root = self.project(pane.project).map(|x| x.path.clone()).unwrap_or_default();
        let s = pane.tracker.status;
        let c = status::color(s, &p);
        let cy = rect.center().y;
        let label = pane.label();
        let place = pane.place(&root);
        let message = pane.tracker.message.clone().filter(|_| s == Status::NeedsInput).map(|m| m.replacen(": ", " · ", 1));
        let doing = pane.program_title.clone();
        let tip = status::described(&pane.tracker);
        let hooks = pane.tracker.hooks_active;
        let hooks_label = pane.tracker.agent.map(|a| a.label().to_string()).unwrap_or_default();
        let agent = pane.tracker.agent.filter(|a| super::agent_logo::has(*a));
        let has_agent = pane.tracker.agent.is_some();
        let since = pane.tracker.since;
        let unconnected = self.unconnected(pane).map(|a| (pane_id, a));
        let quiet = matches!(s, Status::Idle | Status::Unknown | Status::Ended);

        // From the right: the menu, inline active profile switcher, how long, hooks (or "not connected"), where it runs.
        let mut right = rect.right() - 8.0;
        let menu_rect = Rect::from_center_size(pos2(right - 9.0, cy), vec2(18.0, 18.0));
        right -= 18.0 + 6.0;
        let menu = ui
            .scope_builder(egui::UiBuilder::new().max_rect(menu_rect), |ui| kit::icon_button_ex(ui, icons::DOTS_THREE, t("Pane"), false, 18.0))
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("pane-menu:{pane_id}"), menu_rect);
        egui::Popup::menu(&menu).align(egui::RectAlign::BOTTOM_END).show(|ui| self.pane_menu(ui, pane_id));

        // Inline Active Profile Switcher + 1-click Switch Account button inside agent panes
        // that support isolated multi-account profiles (Claude Code & ChatGPT / Codex).
        let _ = has_agent;
        let show_profile = self
            .panes
            .get(&pane_id)
            .and_then(|x| x.tracker.agent.or(x.last_agent))
            .is_some_and(crate::profiles::supports_multi_profile);
        if show_profile && rect.width() > 300.0 {
            let left = (right - 270.0).max(rect.left() + 130.0).min(right);
            let switcher_area = Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom()));
            let used_left = ui
                .scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(switcher_area)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                    |ui| {
                        ui.set_clip_rect(switcher_area.intersect(ui.clip_rect()));
                        self.pane_profile_switcher(ui, pane_id);
                        ui.min_rect().left()
                    },
                )
                .inner;
            right = used_left - 8.0;
        }
        let painter = ui.painter_at(rect);
        let min_right = rect.left() + 140.0;
        // How long it has been running, or waiting: "1:42".
        if matches!(s, Status::Running | Status::NeedsInput) && right > min_right + 36.0 {
            let secs = since.elapsed().as_secs();
            let text = if secs >= 3600 { format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60) } else { format!("{}:{:02}", secs / 60, secs % 60) };
            let g = painter.layout_no_wrap(text, style::mono(10.5), p.text_muted);
            if right - g.size().x > min_right {
                right -= g.size().x;
                painter.galley(pos2(right, cy - g.size().y / 2.0), g, p.text_muted);
                right -= 10.0;
            }
            if s == Status::NeedsInput {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
            }
        }
        if hooks {
            let g = painter.layout_no_wrap(t("hooks").to_string(), style::mono(10.5), p.text_faint);
            let gw = g.size().x;
            let x = right - gw;
            if x - 16.0 > min_right {
                painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
                painter.text(pos2(x - 4.0, cy), Align2::RIGHT_CENTER, icons::PLUGS_CONNECTED, style::icon_font(11.0), p.text_faint);
                let r = Rect::from_min_max(pos2(x - 16.0, rect.top()), pos2(right, rect.bottom()));
                ui.interact(r, egui::Id::new(("hooks", pane_id)), Sense::hover())
                    .on_hover_text(crate::i18n::tf("Status from {} hooks", &[&hooks_label]));
                right = x - 16.0 - 10.0;
            }
        } else if let Some(agent) = unconnected {
            // An agent without the hooks: it can't say when it needs the user. Click: Connect.
            let g = painter.layout_no_wrap(t("not connected").to_string(), style::mono(10.5), p.warn);
            let gw = g.size().x;
            let b = Rect::from_min_max(pos2(right - gw - 16.0 - 8.0, cy - 10.0), pos2(right + 2.0, cy + 10.0));
            if b.left() > min_right {
                let r = ui.interact(b, egui::Id::new(("unconnected", pane_id)), Sense::click());
                #[cfg(debug_assertions)]
                crate::devtest::record(format!("unconnected:{pane_id}"), b);
                if r.hovered() {
                    painter.rect_filled(b, CornerRadius::same(5), style::tint(p.warn, ui.visuals().dark_mode));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                painter.galley(pos2(b.right() - 4.0 - gw, cy - g.size().y / 2.0), g, p.warn);
                painter.text(pos2(b.right() - 4.0 - gw - 4.0, cy), Align2::RIGHT_CENTER, icons::WARNING, style::icon_font(11.0), p.warn);
                if r.on_hover_text(self.unconnected_tip(&[agent])).clicked() {
                    self.actions.push(Action::PlanInstall(agent.1));
                }
                right = b.left() - 8.0;
            }
        }
        if let Some(place) = &place && right > min_right + 40.0 {
            let max_place = ((right - rect.left()) * 0.35).min((right - min_right - 12.0).max(0.0));
            let mut job = egui::text::LayoutJob::simple_singleline(place.clone(), style::mono(10.5), p.text_faint);
            job.wrap = egui::text::TextWrapping::truncate_at_width(max_place);
            let g = painter.layout_job(job);
            let w = g.size().x;
            if w > 8.0 && right - w > min_right {
                painter.galley(pos2(right - w, cy - g.size().y / 2.0), g, p.text_faint);
                right -= w + 10.0;
            }
        }

        // From the left: the mark, the name, the status, what it does.
        let clip = Rect::from_min_max(rect.min, pos2(right.max(rect.left()), rect.max.y));
        let painter = painter.with_clip_rect(clip);
        let mut x = rect.left() + 14.0;
        let mark = Rect::from_center_size(pos2(x + 7.0, cy), vec2(14.0, 14.0));
        match agent {
            Some(a) => {
                super::agent_logo::paint(&painter, mark, a, p.text, false);
            }
            None => {
                painter.text(mark.center(), Align2::CENTER_CENTER, icons::TERMINAL_WINDOW, style::icon_font(13.0), p.text_faint);
            }
        }
        x += 14.0 + 8.0;
        let label_c = if focused { p.text } else { p.text_muted };
        let g = painter.layout_no_wrap(label.clone(), style::medium(12.5), label_c);
        let lw = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, label_c);
        x += lw + 8.0;
        // The status: glyph and word for an agent or a pane that waits, failed or finished; the
        // glyph alone for a program running in a shell; nothing at its prompt.
        if !quiet || s == Status::Unknown {
            status::paint_glyph(&painter, pos2(x + 6.0, cy), s, &p, 12.0);
            x += 12.0 + 5.0;
            if has_agent || s != Status::Running {
                let g = painter.layout_no_wrap(status::text(s), style::medium(12.0), c);
                let w = g.size().x;
                painter.galley(pos2(x, cy - g.size().y / 2.0), g, c);
                x += w + 8.0;
            } else {
                x += 3.0;
            }
        }
        // What it asks, or says it is doing, quiet, after a dash.
        let task = message.or_else(|| doing.clone().filter(|d| d != &label));
        if let Some(task) = task {
            let room = clip.right() - x - 16.0;
            if room > 40.0 {
                painter.text(pos2(x, cy), Align2::LEFT_CENTER, "—", egui::FontId::proportional(12.0), p.text_ghost);
                x += 16.0;
                let mut job = egui::text::LayoutJob::simple_singleline(task, egui::FontId::proportional(12.0), p.text_faint);
                job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                let g = painter.layout_job(job);
                painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
            }
        }
        // The whole header (left of its controls) can be dragged to move the pane.
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
    }

    /// The banner over a restored pane whose agent can continue its last session (in Focus;
    /// a Grid tile says it in its foot): the agent's logo, what happened, Resume Session, ×.
    pub(super) fn resume_banner(&mut self, ui: &mut Ui, pane_id: PaneId, rect: Rect, tile: bool) {
        let _ = tile;
        let p = style::pal(ui);
        let Some((agent, _)) = self.panes.get(&pane_id).and_then(|x| x.resume.clone()) else { return };
        let painter = ui.painter_at(rect.expand(1.0));
        painter.rect(rect, CornerRadius::same(9), style::soft(p.accent, 0.09), Stroke::new(1.0, style::soft(p.accent, 0.28)), egui::StrokeKind::Inside);
        let cy = rect.center().y;
        let mark = Rect::from_center_size(pos2(rect.left() + 11.0 + 6.5, cy), vec2(13.0, 13.0));
        if !(super::agent_logo::has(agent) && super::agent_logo::paint(&painter, mark, agent, p.text, false)) {
            painter.text(mark.center(), Align2::CENTER_CENTER, icons::CLOCK_COUNTER_CLOCKWISE, style::icon_font(14.0), p.accent_text);
        }
        // Buttons from the right.
        let inner = Rect::from_min_max(pos2(rect.left() + 11.0, rect.top()), pos2(rect.right() - 4.0, rect.bottom()));
        let (resume, dismiss, left) = ui
            .scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let d = kit::icon_button_ex(ui, icons::X, t("Dismiss"), false, 22.0);
                let r = kit::button_ex(ui, kit::Kind::Primary, None, t("Resume Session"), true);
                (r, d, ui.min_rect().left())
            })
            .inner;
        #[cfg(debug_assertions)]
        {
            let at = if tile { "tile-" } else { "" };
            crate::devtest::record(format!("{at}resume:{pane_id}"), resume.rect);
            crate::devtest::record(format!("{at}dismiss:{pane_id}"), dismiss.rect);
        }
        let text_x = rect.left() + 11.0 + 13.0 + 8.0;
        let mut job = egui::text::LayoutJob::simple_singleline(
            crate::i18n::tf("{} ran here before the restart.", &[agent.label()]),
            egui::FontId::proportional(12.5),
            p.text,
        );
        // Too narrow: the logo and the buttons only.
        let room = left - 8.0 - text_x;
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

    /// The bar at the bottom of a pane whose shell ended (or could not start), a card inside
    /// its sheet: what happened, and a new shell (Enter) or closing the pane.
    fn exit_bar(&mut self, ui: &mut Ui, pane_id: PaneId, rect: Rect) {
        let p = style::pal(ui);
        let Some(pane) = self.panes.get(&pane_id) else { return };
        let failed = pane.start_error.clone();
        let text = match (&failed, pane.exit) {
            (Some(why), _) => tf("The shell could not start: {}", &[why]),
            (None, Some(tc_term::pty::ExitInfo::Code(c))) => tf("Exited with code {}", &[&c.to_string()]),
            (None, Some(tc_term::pty::ExitInfo::Signal(s))) => tf("Ended by signal {}", &[&s.to_string()]),
            _ => t("Shell ended").to_string(),
        };
        let painter = ui.painter_at(rect.expand(1.0));
        if failed.is_some() {
            painter.rect(rect, CornerRadius::same(9), style::soft(p.err, 0.09), Stroke::new(1.0, style::soft(p.err, 0.26)), egui::StrokeKind::Inside);
        } else {
            painter.rect(rect, CornerRadius::same(9), p.surface, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
            kit::paint_highlight(&painter, rect, 9, &p);
        }
        let cy = rect.center().y;
        if failed.is_some() {
            painter.text(pos2(rect.left() + 12.0 + 7.0, cy), Align2::CENTER_CENTER, icons::X_CIRCLE, style::icon_fill(14.0), p.err);
        } else {
            painter.text(pos2(rect.left() + 12.0 + 7.0, cy), Align2::CENTER_CENTER, icons::POWER, style::icon_font(14.0), p.text_faint);
        }
        let inner = Rect::from_min_max(pos2(rect.left() + 12.0, rect.top()), pos2(rect.right() - 6.0, rect.bottom()));
        let (restart, close, left) = ui
            .scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                ui.set_clip_rect(inner.intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 6.0;
                if failed.is_some() {
                    let r = kit::button_keys(ui, kit::Kind::Secondary, None, t("Try Again"), Some("↵"), true);
                    (r, None, ui.min_rect().left())
                } else {
                    let c = kit::button_keys(ui, kit::Kind::Ghost, None, t("Close Pane"), kit::keys(&Action::ClosePane), true);
                    let r = kit::button_keys(ui, kit::Kind::Secondary, None, t("New Shell"), Some("↵"), true);
                    (r, Some(c), ui.min_rect().left())
                }
            })
            .inner;
        let text_x = rect.left() + 12.0 + 14.0 + 8.0;
        let room = left - 10.0 - text_x;
        if room >= 32.0 {
            let mut job = egui::text::LayoutJob::simple_singleline(text, egui::FontId::proportional(12.5), p.text_muted);
            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
            let g = painter.layout_job(job);
            painter.galley(pos2(text_x, cy - g.size().y / 2.0), g, p.text_muted);
        }
        if restart.clicked() {
            self.actions.push(Action::RestartShell(pane_id));
        }
        if close.is_some_and(|c| c.clicked()) {
            self.actions.push(Action::ClosePaneId(pane_id));
        }
    }

    /// Inline approval / quick-action strip rendered at the bottom of an agent pane waiting for
    /// permission or input (`Status::NeedsInput`).
    fn approval_bar(&mut self, ui: &mut Ui, pane_id: PaneId, rect: Rect) {
        let p = style::pal(ui);
        let msg = self
            .panes
            .get(&pane_id)
            .and_then(|x| x.tracker.message.clone())
            .unwrap_or_else(|| t("Agent needs your confirmation").into());
        let painter = ui.painter_at(rect.expand(1.0));
        painter.rect(
            rect,
            CornerRadius::same(7),
            style::soft(p.warn, 0.10),
            Stroke::new(1.0, style::soft(p.warn, 0.32)),
            egui::StrokeKind::Inside,
        );
        let cy = rect.center().y;
        painter.text(pos2(rect.left() + 12.0 + 6.0, cy), Align2::CENTER_CENTER, icons::BELL_RINGING, style::icon_fill(12.5), p.warn);

        let inner = Rect::from_min_max(pos2(rect.left() + 28.0, rect.top()), pos2(rect.right() - 6.0, rect.bottom()));
        let right_edge = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.set_clip_rect(inner.intersect(ui.clip_rect()));
                    ui.spacing_mut().item_spacing.x = 5.0;
                    if kit::button_ex(ui, kit::Kind::Danger, Some(icons::X), t("Reject"), true).clicked()
                        && let Some(pane) = self.panes.get_mut(&pane_id)
                    {
                        pane.input(vec![0x1b]);
                    }
                    if kit::button_keys(ui, kit::Kind::Primary, Some(icons::CHECK), t("Approve"), Some("↵"), true).clicked()
                        && let Some(pane) = self.panes.get_mut(&pane_id)
                    {
                        pane.entered(Instant::now());
                        pane.input(b"\r".to_vec());
                    }
                    ui.min_rect().left()
                },
            )
            .inner;

        let tx = rect.left() + 28.0;
        let room = right_edge - 8.0 - tx;
        if room >= 32.0 {
            let mut job = egui::text::LayoutJob::simple_singleline(msg, egui::FontId::proportional(12.0), p.text);
            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
            let g = painter.layout_job(job);
            painter.galley(pos2(tx, cy - g.size().y / 2.0), g, p.text);
        }
    }

    /// Crisp 32 px header bar for the collapsible Secondary Drawer (`Terminals | Agent Logs | Problems`,
    /// pane split controls, Bottom/Right dock toggle, Maximize, Collapse).
    fn secondary_drawer_bar(&mut self, ui: &mut Ui, rect: Rect, pid: tc_core::ProjectId, has_editor: bool) {
        let p = style::pal(ui);
        let tab = self.drawer_tab;
        let logs_count = self.agent_logs.iter().filter(|e| e.project.is_none() || e.project == Some(pid)).count();
        let (diag_errs, diag_warns) = self.project_diagnostic_counts(pid);
        let diag_total = diag_errs + diag_warns;

        // Lay out right-side controls first so the left tab strip never overlaps them in narrow right-docked drawers.
        let right_used = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    if has_editor
                        && kit::icon_button_ex(ui, icons::X, &kit::tip(t("Hide Terminal Panel"), Action::ToggleDrawer), false, 22.0).clicked()
                    {
                        self.actions.push(Action::ToggleDrawer);
                    }
                    if has_editor {
                        let max_icon = if self.drawer_maximized { icons::ARROWS_IN } else { icons::ARROWS_OUT };
                        let max_tip = if self.drawer_maximized { t("Restore Terminal Panel") } else { t("Maximize Terminal Panel") };
                        let mr = kit::icon_button_ex(ui, max_icon, max_tip, self.drawer_maximized, 22.0);
                        #[cfg(debug_assertions)]
                        crate::devtest::record("drawer-maximize", mr.rect);
                        if mr.clicked() {
                            self.drawer_maximized = !self.drawer_maximized;
                        }
                        let is_right = self.is_dock_right();
                        let dock_icon = if is_right { icons::ROWS } else { icons::COLUMNS };
                        let dock_tip = if is_right { t("Dock Panel Bottom") } else { t("Dock Panel Right") };
                        if kit::icon_button_ex(ui, dock_icon, dock_tip, false, 22.0).clicked() {
                            self.actions.push(Action::SetDrawerDock(if is_right {
                                tc_core::model::DrawerDock::Bottom
                            } else {
                                tc_core::model::DrawerDock::Right
                            }));
                        }
                    }
                    if tab == tc_core::model::DrawerTab::Terminals && rect.width() >= 280.0 {
                        kit::vsep(ui);
                        if kit::icon_button_ex(ui, icons::SQUARE_SPLIT_VERTICAL, &kit::tip(t("Split Down"), Action::SplitDown), false, 22.0).clicked() {
                            self.actions.push(Action::SplitDown);
                        }
                        if kit::icon_button_ex(ui, icons::SQUARE_SPLIT_HORIZONTAL, &kit::tip(t("Split Right"), Action::SplitRight), false, 22.0).clicked() {
                            self.actions.push(Action::SplitRight);
                        }
                    }
                    ui.min_rect()
                },
            )
            .inner;

        let left_max = (right_used.left() - 6.0).max(rect.left());
        let left_rect = Rect::from_min_max(rect.min, pos2(left_max, rect.bottom()));
        let compact = left_rect.width() < 340.0;
        let diag_c = if diag_errs > 0 { p.err } else { p.warn };
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(left_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.set_clip_rect(left_rect.intersect(ui.clip_rect()));
                ui.spacing_mut().item_spacing.x = 3.0;
                if compact {
                    if kit::icon_button_ex(ui, icons::TERMINAL_WINDOW, t("Terminals"), tab == tc_core::model::DrawerTab::Terminals, 24.0).clicked() {
                        self.actions.push(Action::SetDrawerTab(tc_core::model::DrawerTab::Terminals));
                    }
                    if kit::icon_button_ex(ui, icons::LIST, t("Agent Logs"), tab == tc_core::model::DrawerTab::Logs, 24.0).clicked() {
                        self.actions.push(Action::SetDrawerTab(tc_core::model::DrawerTab::Logs));
                    }
                    if kit::icon_button_ex(ui, icons::WARNING_CIRCLE, t("Problems"), tab == tc_core::model::DrawerTab::Problems, 24.0).clicked() {
                        self.actions.push(Action::SetDrawerTab(tc_core::model::DrawerTab::Problems));
                    }
                } else {
                    if kit::drawer_tab(ui, icons::TERMINAL_WINDOW, t("Terminals"), None, tab == tc_core::model::DrawerTab::Terminals).clicked() {
                        self.actions.push(Action::SetDrawerTab(tc_core::model::DrawerTab::Terminals));
                    }
                    if kit::drawer_tab(ui, icons::LIST, t("Agent Logs"), (logs_count > 0).then_some((logs_count, p.info)), tab == tc_core::model::DrawerTab::Logs).clicked() {
                        self.actions.push(Action::SetDrawerTab(tc_core::model::DrawerTab::Logs));
                    }
                    if kit::drawer_tab(ui, icons::WARNING_CIRCLE, t("Problems"), (diag_total > 0).then_some((diag_total, diag_c)), tab == tc_core::model::DrawerTab::Problems).clicked() {
                        self.actions.push(Action::SetDrawerTab(tc_core::model::DrawerTab::Problems));
                    }
                }
            },
        );
    }

    /// Resizable 1 px splitter between the primary Code Editor stage and the Secondary Drawer.
    fn paint_workspace_splitter(&mut self, ui: &mut Ui, pid: tc_core::ProjectId, full: Rect, ed: Rect, dr: Rect, dock_right: bool) {
        let p = style::pal(ui);
        if dock_right {
            let split_x = (ed.right() + dr.left()) / 2.0;
            let divider = Rect::from_min_max(pos2(split_x - DIVIDER_GRAB, full.top()), pos2(split_x + DIVIDER_GRAB, full.bottom()));
            let resp = ui.interact(divider, egui::Id::new(("editor-divider", pid)), Sense::drag());
            #[cfg(debug_assertions)]
            crate::devtest::record("editor-divider", divider);
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                ui.painter().rect_filled(Rect::from_center_size(pos2(split_x, full.center().y), vec2(2.0, (full.height() - 16.0).max(0.0))), CornerRadius::same(1), p.accent);
            }
            if resp.dragged()
                && let Some(pos) = resp.interact_pointer_pos()
                && let Some(e) = self.editors.get_mut(&pid)
            {
                e.width = ((pos.x - full.left()) / full.width().max(1.0)).clamp(0.40, 0.85);
            }
            if resp.drag_stopped() {
                self.mark_dirty();
            }
        } else {
            let split_y = (ed.bottom() + dr.top()) / 2.0;
            let divider = Rect::from_min_max(pos2(full.left(), split_y - DIVIDER_GRAB), pos2(full.right(), split_y + DIVIDER_GRAB));
            let resp = ui.interact(divider, egui::Id::new(("drawer-divider", pid)), Sense::drag());
            #[cfg(debug_assertions)]
            crate::devtest::record("drawer-divider", divider);
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                ui.painter().rect_filled(Rect::from_center_size(pos2(full.center().x, split_y), vec2((full.width() - 16.0).max(0.0), 2.0)), CornerRadius::same(1), p.accent);
            }
            if resp.dragged()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                self.settings.drawer_size = ((full.bottom() - pos.y) / full.height().max(1.0)).clamp(0.18, 0.70);
            }
            if resp.drag_stopped() {
                self.settings_changed();
            }
        }
    }

    /// Raw & structured Agent Logs drawer tab (hooks, tool executions, rate limits, failover events).
    fn agent_logs_drawer(&mut self, ui: &mut Ui, pid: tc_core::ProjectId) {
        let p = style::pal(ui);
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let mut filter = self.logs_filter.clone();
                let filter_w = (ui.available_width() - 90.0).clamp(80.0, 240.0);
                kit::field(ui, egui::Id::new("logs-filter"), &mut filter, Some(icons::MAGNIFYING_GLASS), t("Filter agent logs…"), filter_w, true);
                self.logs_filter = filter;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::BROOM), t("Clear"), true).clicked() {
                        self.agent_logs.retain(|e| e.project.is_some() && e.project != Some(pid));
                    }
                });
            });
            ui.add_space(6.0);
            let q = self.logs_filter.trim().to_ascii_lowercase();
            let rows: Vec<_> = self
                .agent_logs
                .iter()
                .rev()
                .filter(|e| e.project.is_none() || e.project == Some(pid))
                .filter(|e| q.is_empty() || e.message.to_ascii_lowercase().contains(&q) || e.profile.as_deref().is_some_and(|pr| pr.to_ascii_lowercase().contains(&q)))
                .cloned()
                .collect();
            egui::ScrollArea::vertical()
                .id_salt(("agent-logs-scroll", pid))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if rows.is_empty() {
                        ui.label(RichText::new(t("No agent log events yet.")).size(12.0).color(p.text_faint));
                    } else {
                        for entry in rows {
                            let c = match entry.level {
                                crate::profiles::LogLevel::Failover => p.ok,
                                crate::profiles::LogLevel::Warn => p.warn,
                                crate::profiles::LogLevel::Err => p.err,
                                crate::profiles::LogLevel::Tool => p.info,
                                crate::profiles::LogLevel::Info => p.text_muted,
                            };
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(8.0, 2.0);
                                ui.label(RichText::new(entry.at.format("%H:%M:%S").to_string()).font(style::mono(10.5)).color(p.text_ghost));
                                if let Some(pr) = &entry.profile {
                                    ui.label(RichText::new(pr).font(style::mono_medium(10.5)).color(p.accent_text));
                                }
                                ui.add(egui::Label::new(RichText::new(&entry.message).font(style::mono(11.5)).color(c)).wrap());
                            });
                        }
                    }
                });
        });
    }

    fn project_diagnostic_counts(&self, pid: tc_core::ProjectId) -> (usize, usize) {
        let Some(root) = self.project(pid).map(|x| &x.path) else {
            return (0, 0);
        };
        let mut errs = 0usize;
        let mut warns = 0usize;
        for (path, (_, diags)) in &self.lsp.diagnostics {
            if path.starts_with(root) {
                for d in diags {
                    match d.severity {
                        Some(tc_lsp::types::DiagnosticSeverity::ERROR) => errs += 1,
                        Some(tc_lsp::types::DiagnosticSeverity::WARNING) => warns += 1,
                        _ => {}
                    }
                }
            }
        }
        (errs, warns)
    }

    /// The Code drawer's head: Files | Changes | History (`mode`: 0 = Files, 1 = Changes,
    /// 2 = Commit History), then at the right search (⌘O), full screen (the Review), and close.
    fn drawer_head(&mut self, ui: &mut Ui, rect: Rect, pid: tc_core::ProjectId, mode: usize) {
        let p = style::pal(ui);
        let review = mode == 1;
        let history = mode == 2;
        // From the right first so the segmented control never collides with close/expand/search on narrow panes.
        let mut x = rect.right() - 8.0;
        let mut button = |ui: &mut Ui, icon: &str, tip: &str| {
            let r = Rect::from_center_size(pos2(x - 11.0, rect.center().y), vec2(22.0, 22.0));
            x -= 24.0;
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::icon_button_ex(ui, icon, tip, false, 22.0)).inner
        };
        let close_tip = if review { kit::tip(t("Close Review"), Action::ToggleEditor) } else { kit::tip(t("Close Code"), Action::ToggleEditor) };
        let c = button(ui, icons::X, &close_tip);
        #[cfg(debug_assertions)]
        crate::devtest::record("editor-close", c.rect);
        if c.clicked() {
            if let Some(e) = self.editors.get_mut(&pid) {
                e.visible = false;
            }
            if review {
                self.review_close = true;
            }
            if history {
                self.history = None;
            }
            self.drawer_maximized = false;
            self.focus_request = self.current_pane();
            self.mark_dirty();
        }
        if button(ui, icons::MAGNIFYING_GLASS, &kit::tip(t("Open File…"), Action::QuickOpen)).clicked() {
            self.actions.push(Action::QuickOpen);
        }

        let changed = self.project(pid).and_then(|x| x.git.as_ref()).map_or(0, |g| g.changes.len());
        let avail = (x - 6.0 - (rect.left() + 8.0)).max(90.0);
        let mut options = vec![
            kit::Seg::new(t("Files"), "").icon(icons::FILES),
            kit::Seg::new(t("Changes"), "").icon(icons::GIT_DIFF),
            kit::Seg::new(t("History"), "").icon(icons::CLOCK_COUNTER_CLOCKWISE),
        ];
        if changed > 0 {
            options[1] = options[1].clone().count(changed, p.text_faint);
        }
        let mut w = kit::segmented_width(ui, &options);
        if w > avail {
            options = vec![
                kit::Seg::new(t("Files"), t("Files")),
                kit::Seg::new(t("Changes"), t("Changes")),
                kit::Seg::new(t("History"), t("History")),
            ];
            if changed > 0 {
                options[1] = options[1].clone().count(changed, p.text_faint);
            }
            w = kit::segmented_width(ui, &options).min(avail);
        }
        let seg = Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - kit::SEGMENTED_H / 2.0), vec2(w, kit::SEGMENTED_H));
        #[cfg(debug_assertions)]
        for (r, name) in kit::segment_rects(seg, 3).into_iter().zip(["files:files", "files:changes", "files:history"]) {
            crate::devtest::record(name, r);
        }
        match kit::segmented_in(ui, seg, egui::Id::new(("drawer-mode", pid)), &options, mode) {
            Some(0) => {
                if review {
                    self.review_close = true;
                }
                self.history = None;
            }
            Some(1) => {
                self.history = None;
                self.open_review(Some(super::review::Base::Head));
            }
            Some(2) => {
                self.open_history();
            }
            _ => {}
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
        if kit::menu_item(ui, Some(icons::SQUARE_SPLIT_VERTICAL), t("Split Down"), kit::keys(&Action::SplitDown), false).clicked() {
            self.actions.push(Action::FocusPane(pane_id));
            self.actions.push(Action::SplitDown);
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
        let filter = |app: &mut Self, ui: &mut Ui, rect: Rect| {
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
            |app, ui| {
                let left_w = ui.available_width();
                ui.add_space(4.0);
                if left_w > width + 110.0 {
                    ui.label(RichText::new(t("All projects")).font(style::semibold(14.0)).color(p.text));
                }
                // Worktrees are counted on their own ("7 projects · 2 worktrees · …"); drop on narrow bars so filters never clip.
                if left_w > width + 320.0 {
                    ui.add_space(4.0);
                    let worktrees = app_worktrees;
                    let mut parts = vec![crate::i18n::count(n - worktrees, t("1 project"), t("{} projects"))];
                    if worktrees > 0 {
                        parts.push(crate::i18n::count(worktrees, t("1 worktree"), t("{} worktrees")));
                    }
                    parts.push(tf("{} active", &[&active.to_string()]));
                    parts.push(tf("{} waiting", &[&waiting.to_string()]));
                    ui.label(RichText::new(parts.join(" · ")).font(style::mono(11.0)).color(p.text_faint));
                    ui.add_space(14.0);
                } else if left_w > width + 110.0 {
                    ui.add_space(8.0);
                }
                let seg_w = width.min((ui.available_width() - 4.0).max(120.0));
                let (rect, _) = ui.allocate_exact_size(vec2(seg_w, kit::SEGMENTED_H), Sense::hover());
                filter(app, ui, rect);
            },
            None,
        );
        // The same area as the Focus view's (one top bar above both): panes shown in the Grid
        // are sized against it.
        self.last_full_body = Some(sheets_area(body, self.settings.sidebar_hidden));
        body
    }

    /// The Grid's toolbar: Run Agent, and how many tiles a screen holds.
    fn grid_toolbar(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let compact = ui.available_width() < 760.0;
        let label = match self.grid_columns {
            0 => t("Auto").to_string(),
            n => format!("{n} × {n}"),
        };
        // Added right to left: the tiles menu, then Run Agent.
        let run_keys = (!compact).then(|| kit::keys(&Action::PickPreset)).flatten();
        let run = kit::button_keys(ui, kit::Kind::Primary, Some(icons::PLAY), t("Run Agent"), run_keys, false);
        if run.on_hover_text(kit::tip(t("Start an agent in a new pane"), Action::PickPreset)).clicked() {
            self.actions.push(Action::PickPreset);
        }
        kit::vsep(ui);
        let g = ui.painter().layout_no_wrap(label, style::mono(11.0), p.text);
        let (rect, r) = ui.allocate_exact_size(vec2(8.0 + 13.0 + 6.0 + g.size().x + 6.0 + 10.0 + 8.0, 26.0), Sense::click());
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&r));
        ui.painter().rect(rect, CornerRadius::same(style::RADIUS), if r.hovered() || open { p.hover } else { egui::Color32::TRANSPARENT }, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
        ui.painter().text(pos2(rect.left() + 8.0 + 6.5, rect.center().y), Align2::CENTER_CENTER, icons::SQUARES_FOUR, style::icon_font(13.0), p.text_faint);
        let gw = g.size().x;
        ui.painter().galley(pos2(rect.left() + 8.0 + 13.0 + 6.0, rect.center().y - g.size().y / 2.0), g, p.text);
        ui.painter().text(pos2(rect.left() + 8.0 + 13.0 + 6.0 + gw + 6.0 + 5.0, rect.center().y), Align2::CENTER_CENTER, icons::CARET_DOWN, style::icon_font(10.0), p.text_ghost);
        #[cfg(debug_assertions)]
        crate::devtest::record("grid-layout", rect);
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

/// A worktree's branch in its project header: "⑂ tc/generics · 2 ahead of main". A click
/// reviews it against its base.
fn worktree_chip(ui: &mut Ui, branch: &str, ahead: usize, base: &str, changed: usize) -> egui::Response {
    let p = style::pal(ui);
    let mut job = egui::text::LayoutJob::default();
    job.append(branch, 0.0, egui::TextFormat::simple(style::mono(11.5), p.text_muted));
    job.append("·", 6.0, egui::TextFormat::simple(style::mono(11.5), p.text_faint));
    job.append(&tf("{} ahead of {}", &[&ahead.to_string(), base]), 6.0, egui::TextFormat::simple(style::mono(11.5), p.text_faint));
    if changed > 0 {
        job.append("·", 6.0, egui::TextFormat::simple(style::mono(11.5), p.text_faint));
        job.append(&tf("{} changed", &[&changed.to_string()]), 6.0, egui::TextFormat::simple(style::mono(11.5), p.text_muted));
    }
    let g = ui.painter().layout_job(job);
    let w = 8.0 + 13.0 + 6.0 + g.size().x + 8.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    ui.painter().rect_stroke(rect, CornerRadius::same(style::RADIUS), Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(style::RADIUS), p.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(pos2(rect.left() + 8.0 + 6.5, rect.center().y), Align2::CENTER_CENTER, icons::GIT_FORK, style::icon_font(13.0), p.text_faint);
    ui.painter().galley(pos2(rect.left() + 8.0 + 13.0 + 6.0, rect.center().y - g.size().y / 2.0), g, p.text_muted);
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
/// A pull/merge request in the top bar: its number (`#N` on GitHub, `!N` on GitLab) and its checks.
fn pr_chip(ui: &mut Ui, pr: &tc_git::ship::Pr, forge: Option<tc_git::ship::Forge>) -> egui::Response {
    use tc_git::ship::{Checks, PrState};
    let p = style::pal(ui);
    let prefix = forge.map_or("#", |f| f.request_prefix());
    let number = format!("{prefix}{}", pr.number);
    let (check_text, check_c) = match pr.checks {
        Checks::Pending => (t("checks"), p.info),
        Checks::Passed => (t("passed"), p.ok),
        Checks::Failed => (t("failed"), p.err),
        Checks::None => (if pr.state == PrState::Draft { t("draft") } else { "" }, p.text_faint),
    };
    let ng = ui.painter().layout_no_wrap(number, style::mono(11.5), p.text);
    let cg = (!check_text.is_empty()).then(|| ui.painter().layout_no_wrap(check_text.to_string(), egui::FontId::proportional(11.5), check_c));
    let cw = cg.as_ref().map_or(0.0, |g| 6.0 + 12.0 + 4.0 + g.size().x);
    let (rect, resp) = ui.allocate_exact_size(vec2(8.0 + 13.0 + 6.0 + ng.size().x + cw + 8.0, 26.0), Sense::click());
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(7), p.hover);
    }
    painter.rect_stroke(rect, CornerRadius::same(7), Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
    let cy = rect.center().y;
    let mut x = rect.left() + 8.0;
    painter.text(pos2(x + 6.5, cy), Align2::CENTER_CENTER, icons::GIT_PULL_REQUEST, style::icon_font(13.0), p.accent_text);
    x += 13.0 + 6.0;
    let nw = ng.size().x;
    painter.galley(pos2(x, cy - ng.size().y / 2.0), ng, p.text);
    x += nw;
    if let Some(g) = cg {
        x += 6.0;
        match pr.checks {
            // Still (no frames while nothing changes): the lattice without its light running.
            Checks::Pending => kit::paint_lattice(painter, pos2(x + 6.0, cy), 10.0, check_c, false),
            Checks::Passed => {
                painter.text(pos2(x + 6.0, cy), Align2::CENTER_CENTER, icons::CHECK_CIRCLE, style::icon_fill(12.0), check_c);
            }
            Checks::Failed => {
                painter.text(pos2(x + 6.0, cy), Align2::CENTER_CENTER, icons::X_CIRCLE, style::icon_fill(12.0), check_c);
            }
            Checks::None => {}
        }
        if pr.checks != Checks::None {
            x += 12.0 + 4.0;
        } else {
            x -= 6.0;
        }
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, check_c);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn server_chip(ui: &mut Ui, servers: &[(String, String)]) -> Option<String> {
    let p = style::pal(ui);
    let (url, pane) = servers.first()?;
    let mut job = egui::text::LayoutJob::default();
    job.append(server_label(url), 0.0, egui::TextFormat::simple(style::mono(11.5), p.text_muted));
    if servers.len() > 1 {
        job.append(&format!("+{}", servers.len() - 1), 5.0, egui::TextFormat::simple(style::mono(11.5), p.text_faint));
    }
    let g = ui.painter().layout_job(job);
    let w = 8.0 + 6.0 + 6.0 + g.size().x + 6.0 + 11.0 + 8.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    #[cfg(debug_assertions)]
    crate::devtest::record("server-chip", rect);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(style::RADIUS), p.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let cy = rect.center().y;
    // A green dot in a soft ring of its colour: the server answers.
    let dot = pos2(rect.left() + 8.0 + 3.0, cy);
    ui.painter().circle_filled(dot, 6.0, style::soft(p.ok, 0.18));
    ui.painter().circle_filled(dot, 3.0, p.ok);
    let x = rect.left() + 8.0 + 6.0 + 6.0;
    let c = if resp.hovered() { p.text } else { p.text_muted };
    ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g.clone(), c);
    ui.painter().text(pos2(x + g.size().x + 6.0 + 5.5, cy), Align2::CENTER_CENTER, icons::ARROW_UP_RIGHT, style::icon_font(11.0), p.text_ghost);
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

/// A terminal tab in the project header: the agent's logo (or the terminal's icon, a globe for
/// a page), its label, a dot in the status colour when something in it waits for the user, a
/// close button on the tab in front (and on hover) when there are several tabs, and a thin
/// progress line when a program in it reports one. Returns the tab's response and whether
/// close was clicked.
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
    agent: Option<tc_core::agent::AgentKind>,
) -> (egui::Response, bool) {
    let p = style::pal(ui);
    let dark = ui.visuals().dark_mode;
    let font = style::medium(12.5);
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), font, egui::Color32::PLACEHOLDER);
    job.wrap = egui::text::TextWrapping::truncate_at_width(160.0);
    let galley = ui.painter().layout_job(job);
    // Something waiting, failed or finished: a dot after the label.
    let dot = waiting.filter(|s| matches!(s, Status::NeedsInput | Status::Failed(_) | Status::Done | Status::CommandDone));
    let dot_w = if dot.is_some() { 7.0 + 6.0 } else { 0.0 };
    let close_w = if closable { 4.0 + 14.0 } else { 0.0 };
    let w = 10.0 + 13.0 + 7.0 + galley.size().x + dot_w + close_w + if closable { 4.0 } else { 10.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click_and_drag());
    let close_rect = Rect::from_center_size(pos2(rect.right() - 4.0 - 7.0, rect.center().y), vec2(14.0, 14.0));
    let close_resp = closable.then(|| ui.interact(close_rect, resp.id.with("close"), Sense::click()));
    let painter = ui.painter_at(rect.expand(1.0));
    let radius = CornerRadius::same(8);
    if dragging {
        painter.rect_stroke(rect.shrink(0.5), radius, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
    } else if on {
        painter.rect(rect, radius, p.surface, Stroke::new(1.0, if dark { p.border } else { p.border_strong }), egui::StrokeKind::Inside);
        kit::paint_highlight(&painter, rect, 8, &p);
    } else if resp.hovered() {
        painter.rect_filled(rect, radius, p.hover);
    }
    let icon_at = pos2(rect.left() + 10.0 + 6.5, rect.center().y);
    let icon_color = if on { p.text_muted } else { p.text_faint };
    match (agent, web) {
        (Some(a), _) if web.is_none() => {
            super::agent_logo::paint(&painter, Rect::from_center_size(icon_at, vec2(13.0, 13.0)), a, if on { p.text } else { p.text_muted }, false);
        }
        _ => {
            let icon = match web {
                // A page let go to save memory: it loads again when shown.
                Some(true) => icons::MOON,
                Some(false) => icons::GLOBE,
                None => icons::TERMINAL_WINDOW,
            };
            painter.text(icon_at, Align2::CENTER_CENTER, icon, style::icon_font(13.0), icon_color);
        }
    }
    let text_color = if on || resp.hovered() { p.text } else { p.text_faint };
    let text_x = rect.left() + 10.0 + 13.0 + 7.0;
    let gw = galley.size().x;
    painter.galley(pos2(text_x, rect.center().y - galley.size().y / 2.0), galley, text_color);
    if let Some(s) = dot {
        painter.circle_filled(pos2(text_x + gw + 6.0 + 3.0, rect.center().y), 3.0, status::color(s, &p));
    }
    // What a program in the tab reports of its progress.
    if let Some((state, value)) = progress {
        let track = Rect::from_min_max(pos2(rect.left() + 10.0, rect.bottom() - 5.0), pos2(rect.right() - 10.0, rect.bottom() - 3.0));
        painter.rect_filled(track, CornerRadius::same(1), p.border_strong);
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
        if on || resp.hovered() || hot {
            painter.text(
                close_rect.center(),
                Align2::CENTER_CENTER,
                icons::X,
                style::icon_font(10.0),
                if hot { p.text } else { p.text_faint },
            );
        }
        closed = c.on_hover_text(t("Close Tab")).clicked();
    }
    (resp, closed)
}

/// Where a pane's terminal text goes inside its sheet: below its header, the terminal's own
/// padding (10) plus `TERM_PAD` from the sides. (`single`: kept for callers; every pane has a
/// header now.)
pub fn term_rect(pane: Rect, _single: bool) -> Rect {
    Rect::from_min_max(
        pos2(pane.left() + TERM_PAD, pane.top() + PANE_HEADER_H - 6.0),
        pos2(pane.right() - TERM_PAD, pane.bottom() - 2.0),
    )
}

/// The area the sheets take below the top bar: the window's edge kept at the right, and at the
/// left when the sidebar is hidden.
pub fn sheets_area(body: Rect, sidebar_hidden: bool) -> Rect {
    let left = if sidebar_hidden { style::EDGE } else { 0.0 };
    Rect::from_min_max(pos2(body.left() + left, body.top()), pos2(body.right() - style::EDGE, body.bottom()))
}

/// How a sheet stands out: plainly, as the pane with the keyboard among several, as one that
/// waits for the user (an amber ring with a soft glow), or as the page an agent uses (blue).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SheetLook {
    Plain,
    Focused,
    Waiting,
}

/// Paints a sheet (its fill the canvas colour).
pub fn paint_sheet(painter: &egui::Painter, rect: Rect, p: &style::Palette, look: SheetLook) {
    paint_sheet_fill(painter, rect, p.canvas, p, look);
}

/// Paints a sheet with its own fill (a terminal's background).
pub fn paint_sheet_fill(painter: &egui::Painter, rect: Rect, fill: egui::Color32, p: &style::Palette, look: SheetLook) {
    let dark = p.chrome.r() < 0x80;
    let radius = CornerRadius::same(style::RADIUS_LG);
    if look == SheetLook::Waiting {
        let glow = egui::epaint::Shadow { offset: [0, 8], blur: 30, spread: 0, color: style::soft(p.warn, 0.10) };
        painter.add(glow.as_shape(rect, radius));
        painter.rect_stroke(rect, CornerRadius::same(style::RADIUS_LG + 3), Stroke::new(3.0, style::soft(p.warn, if dark { 0.09 } else { 0.10 })), egui::StrokeKind::Outside);
    } else if !dark {
        // Light: a whisper of shadow under each sheet.
        let shadow = egui::epaint::Shadow { offset: [0, 1], blur: 2, spread: 0, color: egui::Color32::from_black_alpha(10) };
        painter.add(shadow.as_shape(rect, radius));
    }
    let line = match look {
        SheetLook::Waiting => style::soft(p.warn, 0.6),
        SheetLook::Focused => if dark { p.border_strong } else { p.border_heavy },
        SheetLook::Plain => if dark { p.border } else { p.border_strong },
    };
    painter.rect(rect, radius, fill, Stroke::new(1.0, line), egui::StrokeKind::Inside);
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
