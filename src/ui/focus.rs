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
/// A pane's header: what runs there, its status pill, what it does, since when, the account.
pub const PANE_HEADER_H: f32 = 36.0;
/// The strip of tabs at the top of the stage.
pub const TAB_STRIP_H: f32 = 40.0;
/// The secondary drawer's tab bar (`Terminals | Agent Logs | Problems`, dock controls).
pub const DRAWER_BAR_H: f32 = 32.0;
/// How far either side of a divider (or the Code editor's edge) a drag takes it.
pub(crate) const DIVIDER_GRAB: f32 = 5.0;
/// Space between a sheet's edge and where the terminal's own padding starts (16 px to the text
/// at the sides, as the design has it).
const TERM_PAD: f32 = 6.0;

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
                left(self, ui);
            },
        );
        Rect::from_min_max(pos2(full.left(), header.bottom()), full.max)
    }


    /// Right side of the project bar (added right to left): the panel's toggles, then Run
    /// agent and Commit. Zen shows only its way out; the Overview has its own bar.
    fn toolbar(&mut self, ui: &mut Ui) {
        if self.zen_mode {
            let exit = kit::button_keys(ui, kit::Kind::Secondary, Some(icons::ARROWS_IN), t("Exit Zen"), kit::keys(&Action::ToggleZenMode), true);
            #[cfg(debug_assertions)]
            crate::devtest::record("toggle-zen", exit.rect);
            if exit.clicked() {
                self.actions.push(Action::ToggleZenMode);
            }
            return;
        }
        if self.view == View::Grid {
            self.grid_toolbar(ui);
            return;
        }
        if self.view == View::Inbox {
            self.inbox_toolbar(ui);
            return;
        }
        let Some(pid) = self.active else { return };
        let compact = ui.available_width() < 760.0;
        // The panel's four toggles, in an inset group (the open one tinted).
        let open = self.panel_tab(pid);
        let agent_page = matches!(self.browser_button_state(), Some(super::browser_ui::WebButton::Agent(..)));
        let group = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let (rect, _) = ui.allocate_exact_size(vec2(4.0 * 28.0 + 3.0 * 2.0 + 4.0, 32.0), Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(8), style::pal(ui).hover);
                let mut x = rect.left() + 2.0;
                for tab in super::panel::PanelTab::ALL {
                    let r = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(28.0, 28.0));
                    x += 30.0;
                    let resp = ui
                        .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                            kit::icon_button_ex(ui, tab.icon(), &kit::tip(tab.label(), tab.action()), open == Some(tab), 28.0)
                        })
                        .inner;
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("panel-toggle:{tab:?}"), resp.rect);
                    if tab == super::panel::PanelTab::Browser && agent_page {
                        // An agent is using a page: a dot on the globe.
                        let p = style::pal(ui);
                        let c = pos2(r.right() - 6.0, r.top() + 6.0);
                        ui.painter().circle_filled(c, 4.0, p.chrome);
                        ui.painter().circle_filled(c, 2.5, p.info);
                    }
                    if resp.clicked() {
                        if tab == super::panel::PanelTab::Browser && ui.input(|i| i.modifiers.alt) {
                            self.actions.push(Action::NewBrowserTab);
                        } else {
                            self.switch_panel(tab, true);
                        }
                    }
                }
                rect
            })
            .inner;
        let _ = group;
        ui.add_space(6.0);
        kit::vsep(ui);
        ui.add_space(4.0);
        let run_keys = (!compact).then(|| kit::keys(&Action::PickPreset)).flatten();
        let run = kit::button_keys(ui, kit::Kind::Primary, Some(icons::PLAY), t("Run agent"), run_keys, false);
        #[cfg(debug_assertions)]
        crate::devtest::record("run-agent", run.rect);
        if run.on_hover_text(kit::tip(t("Start an agent in a new pane"), Action::PickPreset)).clicked() {
            self.actions.push(Action::PickPreset);
        }
        ui.add_space(6.0);
        let commit_keys = (!compact).then(|| kit::keys(&Action::Commit)).flatten();
        let commit = kit::button_keys(ui, kit::Kind::Secondary, Some(icons::GIT_COMMIT), t("Commit"), commit_keys, false);
        #[cfg(debug_assertions)]
        crate::devtest::record("commit", commit.rect);
        if commit.on_hover_text(kit::tip(t("Commit, push and open a pull request"), Action::Commit)).clicked() {
            self.actions.push(Action::Commit);
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
            ui.spacing_mut().item_spacing.x = 8.0;
            // The project switcher: its letter tile, its name, a caret; a click opens its menu.
            let title_text = match (&parent, &worktree) {
                (Some(_), Some(_)) => None,
                _ => Some(name.clone()),
            };
            if let Some(title_text) = title_text {
                let mut job = egui::text::LayoutJob::simple_singleline(title_text.clone(), style::semibold(style::HEADING), p.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width((bar_w * 0.30).clamp(72.0, 220.0));
                let g = ui.painter().layout_job(job);
                let (rect, resp) = ui.allocate_exact_size(vec2(4.0 + 20.0 + 8.0 + g.size().x + 8.0 + 10.0 + 8.0, 30.0), Sense::click());
                let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
                if resp.hovered() || open {
                    ui.painter().rect_filled(rect, CornerRadius::same(style::RADIUS_ROW), p.hover);
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
                // "storefront › fix-oauth": the project opens it; the worktree's name opens its menu.
                let (r, _) = ui.allocate_exact_size(vec2(24.0, 30.0), Sense::hover());
                let tile = Rect::from_center_size(r.center() + vec2(2.0, 0.0), vec2(20.0, 20.0));
                kit::paint_project_tile(ui.painter(), tile, &letters, color, dark);
                let r = ui.add(egui::Label::new(RichText::new(parent_name).font(style::semibold(style::HEADING)).color(p.text_muted)).sense(Sense::click()));
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tf("Open {}", &[parent_name])).clicked() {
                    app.actions.push(Action::SelectProject(*parent_id));
                }
                ui.label(RichText::new(icons::CARET_RIGHT).font(style::icon_font(10.0)).color(p.text_faint));
                let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), style::semibold(style::HEADING), p.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width((bar_w * 0.26).clamp(64.0, 180.0));
                let g = ui.painter().layout_job(job);
                let (rect, resp) = ui.allocate_exact_size(vec2(8.0 + 14.0 + 6.0 + g.size().x + 8.0 + 10.0 + 6.0, 30.0), Sense::click());
                let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
                if resp.hovered() || open {
                    ui.painter().rect_filled(rect, CornerRadius::same(style::RADIUS_ROW), p.hover);
                }
                ui.painter().text(pos2(rect.left() + 8.0 + 7.0, rect.center().y), Align2::CENTER_CENTER, icons::GIT_FORK, style::icon_font(14.0), p.text_muted);
                ui.painter().galley(pos2(rect.left() + 8.0 + 14.0 + 6.0, rect.center().y - g.size().y / 2.0), g.clone(), p.text);
                ui.painter().text(pos2(rect.left() + 8.0 + 14.0 + 6.0 + g.size().x + 8.0 + 5.0, rect.center().y), Align2::CENTER_CENTER, icons::CARET_DOWN, style::icon_font(10.0), p.text_faint);
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
                && bar_right - ui.cursor().min.x > 200.0
            {
                let label = if changed > 0 {
                    format!("{} · {}", g.branch, tf("{} changed", &[&changed.to_string()]))
                } else {
                    g.branch.clone()
                };
                let r = bar_chip(ui, icons::GIT_BRANCH, &label);
                #[cfg(debug_assertions)]
                crate::devtest::record("branch-chip", r.rect);
                if r.on_hover_text(kit::tip(t("Review Changes…"), Action::Review)).clicked() {
                    app.actions.push(Action::Review);
                }
            }
            if let Some(pr) = &pr
                && bar_right - ui.cursor().min.x > 220.0
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
                && bar_right - ui.cursor().min.x > 200.0
                && let Some(url) = server_chip(ui, &servers)
            {
                app.actions.push(Action::OpenWeb(url));
            }
        }, None);

        // The sheets sit on the base below the bar, the window's edge beside them (and a margin
        // at the left too when the sidebar is hidden).
        let body = sheets_area(body, self.settings.sidebar_hidden || self.zen_mode);
        self.last_full_body = Some(body);

        // ----- the stage (terminals) and the right panel (Changes, Files, History, Browser) -----
        // The page in front follows the terminal the user picks.
        self.panel_follow(pid);
        let (stage, panel) = self.panel_split(pid, body);
        if let Some((rect, tab)) = panel {
            self.right_panel(ui, pid, rect, tab);
            if stage.is_some() {
                self.panel_resize_edge(ui, pid, rect, body);
            }
        }
        let Some(stage) = stage else {
            self.tab_drop(ui, &[], None, &[], active_tab);
            return;
        };

        let style = self.term_style(ui);
        let scheme = style.scheme;
        self.cell = Some(style.cell);
        // The stage is one sheet in the terminal's own colour: its tabs at the top, its panes
        // below them, hairlines between them.
        paint_sheet_fill(ui.painter(), stage, scheme.background, &p, SheetLook::Plain);
        #[cfg(debug_assertions)]
        crate::devtest::record("stage", stage);
        let strip_row = Rect::from_min_max(pos2(stage.left() + 6.0, stage.top()), pos2(stage.right() - 8.0, stage.top() + TAB_STRIP_H));
        // Right end of the strip: Split Right, Split Down.
        let right_used = ui
            .scope_builder(egui::UiBuilder::new().max_rect(strip_row).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if kit::icon_button_ex(ui, icons::SQUARE_SPLIT_VERTICAL, &kit::tip(t("Split Down"), Action::SplitDown), false, 24.0).clicked() {
                    self.actions.push(Action::SplitDown);
                }
                if kit::icon_button_ex(ui, icons::SQUARE_SPLIT_HORIZONTAL, &kit::tip(t("Split Right"), Action::SplitRight), false, 24.0).clicked() {
                    self.actions.push(Action::SplitRight);
                }
                ui.min_rect().left()
            })
            .inner;
        let tabs_row = Rect::from_min_max(strip_row.min, pos2((right_used - 8.0).max(strip_row.left()), strip_row.bottom()));
        let mut tab_rects: Vec<Rect> = Vec::with_capacity(tabs.len());
        // The part of the strip in view (tabs scrolled out of it are no place to drop on).
        let mut strip_shown: Option<Rect> = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(tabs_row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.set_clip_rect(tabs_row.intersect(ui.clip_rect()));
            let plus_w = 26.0 + 2.0;
            let strip = Rect::from_min_max(pos2(tabs_row.left(), tabs_row.center().y - 14.0), pos2((tabs_row.right() - plus_w).max(tabs_row.left()), tabs_row.center().y + 14.0));
            let seen_id = egui::Id::new(("tab-strip-active", pid));
            let scroll_to_active = ui.data(|d| d.get_temp::<(usize, usize)>(seen_id)) != Some((active_tab, tabs.len()));
            ui.data_mut(|d| d.insert_temp(seen_id, (active_tab, tabs.len())));
            let strip_used = ui
                .scope_builder(egui::UiBuilder::new().max_rect(strip).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                    ui.style_mut().always_scroll_the_only_direction = true;
                    ui.spacing_mut().scroll.fade.strength = 0.0;
                    let out = egui::ScrollArea::horizontal()
                        .id_salt(("tab-strip", pid))
                        .auto_shrink([true, true])
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .scroll_source(egui::scroll_area::ScrollSource::MOUSE_WHEEL)
                        .show(ui, |ui| {
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                for (i, label, waiting, doing, focused, progress, web, agent) in &tabs {
                                    let dragging = self.tab_drag == Some(*i);
                                    let on = *i == active_tab;
                                    let (r, close) = terminal_tab(ui, label, waiting.as_ref().map(|w| w.0), on, dragging, tabs.len() > 1, *progress, *web, *agent);
                                    if close {
                                        self.actions.push(Action::CloseTab(*i));
                                    }
                                    if on && scroll_to_active {
                                        r.scroll_to_me(None);
                                    }
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record(format!("tab:{i}"), r.rect);
                                    tab_rects.push(r.rect);
                                    if r.hovered() || r.is_pointer_button_down_on() {
                                        self.tabs_hot = true;
                                    }
                                    if r.clicked() && !close {
                                        self.actions.push(Action::SelectTab(*i));
                                    }
                                    if r.middle_clicked() && tabs.len() > 1 {
                                        self.actions.push(Action::CloseTab(*i));
                                    }
                                    // Dragged: along the strip to reorder, onto a pane to split it.
                                    if r.drag_started() && tabs.len() > 1 {
                                        self.tab_drag = Some(*i);
                                    }
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
                                        let current = self.panes.get(focused).and_then(|p| p.title.clone()).unwrap_or_default();
                                        self.dialog = Some(crate::app::Dialog::RenamePane(*focused, current));
                                    }
                                }
                            });
                        });
                    let shown = out.inner_rect;
                    strip_shown = Some(shown);
                    let offset = out.state.offset.x;
                    let fade_w = 24.0_f32.min(shown.width() / 2.0);
                    if offset > 0.5 {
                        kit::fade(ui.painter(), Rect::from_min_max(shown.min, pos2(shown.left() + fade_w, shown.bottom())), scheme.background, false);
                    }
                    if offset + shown.width() < out.content_size.x - 0.5 {
                        kit::fade(ui.painter(), Rect::from_min_max(pos2(shown.right() - fade_w, shown.top()), shown.max), scheme.background, true);
                    }
                    // Just after the last tab (the strip is only as wide as the tabs need).
                    tab_rects.last().map_or(shown.left(), |r| r.right()).min(shown.right())
                })
                .inner;
            let plus = Rect::from_center_size(pos2(strip_used + 4.0 + 13.0, tabs_row.center().y), vec2(26.0, 26.0));
            let r = ui.scope_builder(egui::UiBuilder::new().max_rect(plus), |ui| kit::icon_button_ex(ui, icons::PLUS, &kit::tip(t("New Tab"), Action::NewTab), false, 26.0)).inner;
            #[cfg(debug_assertions)]
            crate::devtest::record("new-tab", r.rect);
            if r.clicked() {
                self.actions.push(Action::NewTab);
            }
        });
        let line_y = stage.top() + TAB_STRIP_H;
        ui.painter().hline((stage.left() + 1.0)..=(stage.right() - 1.0), line_y + 0.5, Stroke::new(1.0, p.border));
        let panes_area = Rect::from_min_max(pos2(stage.left() + 1.0, line_y + 1.0), pos2(stage.right() - 1.0, stage.bottom() - 1.0));

        // ----- panes -------------------------------------------------------------------------
        let Some(tab) = self.project(pid).and_then(|p| p.tab()) else {
            return;
        };
        let focused = tab.focused;
        // Beside the panel, panes side by side too narrow for a terminal show stacked instead.
        let cell = style.cell;
        let (mut rects, mut dividers) = tab.layout.arrange(panes_area);
        let kept_side = self.panel_of(pid).is_some_and(|x| x.side_by_side);
        let too_narrow = rects.len() > 1
            && panel.is_some()
            && !kept_side
            && rects.iter().any(|(_, r)| widget::grid_for(term_rect(*r, false), cell).cols < super::browser_panel::MIN_COLS);
        if too_narrow {
            (rects, dividers) = tab.layout.stacked().arrange(panes_area);
        }
        // Told once per run (a tile lays its panes the same way).
        let told = self.panel_of(pid).is_some_and(|x| x.told_stacked);
        let browser_beside = panel.is_some_and(|(_, t)| t == super::panel::PanelTab::Browser);
        if let Some(x) = self.panel_mut(pid) {
            x.stacked = too_narrow && browser_beside;
            x.floating = false;
            x.told_stacked |= too_narrow && browser_beside;
        }
        if too_narrow && browser_beside && !told {
            self.toast_with(
                crate::app::ToastKind::Info,
                t("Terminals stacked while the browser is open").to_string(),
                Some((t("Keep side by side").to_string(), Action::PanelSideBySide(pid))),
            );
        }
        let single = rects.len() == 1;
        // Where panes go, for sizing new ones before they start (no resize right after).
        self.last_body = Some(panes_area);
        let pane_areas = rects.clone();
        // Hairlines between panes, in the middle of the gaps the layout leaves.
        for d in &dividers {
            let c = d.rect.center();
            match d.dir {
                SplitDir::Horizontal => ui.painter().vline(c.x, d.rect.y_range(), Stroke::new(1.0, p.border)),
                SplitDir::Vertical => ui.painter().hline(d.rect.x_range(), c.y, Stroke::new(1.0, p.border)),
            };
        }

        let input_enabled = self.terminals_take_keys();
        let now = Instant::now();
        for (pane_id, rect) in rects {
            let is_focused = pane_id == focused;
            // A browser pane: its toolbar is its header, its page the rest.
            if self.browsers.contains_key(&pane_id) {
                self.browser_pane(ui, pane_id, rect, is_focused, single);
                continue;
            }
            // A pane that waits for the user is ringed in amber.
            let waiting = self.panes.get(&pane_id).is_some_and(|x| x.tracker.status == Status::NeedsInput);
            if waiting {
                ui.painter().rect_stroke(rect.shrink(1.0), CornerRadius::same(10), Stroke::new(1.5, p.warn), egui::StrokeKind::Inside);
            }
            let header = Rect::from_min_size(rect.min, vec2(rect.width(), PANE_HEADER_H));
            let mut term_rect = term_rect(rect, single);
            self.pane_header(ui, pane_id, header, is_focused && !single);
            // Notices: a session to resume under the header; at the foot an agent's question,
            // a usage limit, a failed command or an ended shell.
            let inset = |h: f32, top: f32| Rect::from_min_size(pos2(rect.left() + 8.0, top), vec2((rect.width() - 16.0).max(40.0), h));
            if self.panes.get(&pane_id).is_some_and(|x| x.resume.is_some() && x.rate_limit.is_none()) {
                let n = inset(kit::NOTICE_H, header.bottom());
                self.resume_banner(ui, pane_id, n, false);
                term_rect.min.y = n.bottom() + 4.0;
            }
            let foot = |h: f32| Rect::from_min_max(pos2(rect.left() + 8.0, rect.bottom() - 8.0 - h), pos2(rect.right() - 8.0, rect.bottom() - 8.0));
            if self.panes.get(&pane_id).is_some_and(|x| x.rate_limit.is_some()) {
                let n = foot(kit::NOTICE_H);
                self.rate_limit_notice(ui, pane_id, n);
                term_rect.max.y = n.top() - 4.0;
            } else if self.panes.get(&pane_id).is_some_and(|x| x.exit.is_some() && !x.alive()) {
                let n = foot(kit::NOTICE_H);
                self.exit_bar(ui, pane_id, n);
                term_rect.max.y = n.top() - 4.0;
            } else if self.panes.get(&pane_id).is_some_and(|x| x.tracker.status == Status::NeedsInput && x.tracker.agent.is_some())
                && term_rect.height() > 140.0
            {
                let n = foot(kit::NOTICE_H);
                self.approval_bar(ui, pane_id, n);
                term_rect.max.y = n.top() - 4.0;
            } else if self.panes.get(&pane_id).is_some_and(|x| matches!(x.tracker.status, Status::Failed(_)) && x.tracker.unseen && x.alive())
                && term_rect.height() > 140.0
            {
                let n = foot(kit::NOTICE_H);
                self.failed_notice(ui, pane_id, n);
                term_rect.max.y = n.top() - 4.0;
            } else if self.panes.get(&pane_id).is_some_and(crate::pane::Pane::turn_finished)
                && term_rect.height() > 140.0
            {
                let n = foot(kit::NOTICE_H);
                self.done_notice(ui, pane_id, n);
                term_rect.max.y = n.top() - 4.0;
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
            let resp = widget::show(ui, term_rect, &mut pane.view, &pane.shared, &style, is_focused, input_enabled, alive);
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
            let scrolled = widget::display_offset(&pane.shared);
            if scrolled > 0 {
                self.scrolled_pill(ui, pane_id, term_rect, scrolled);
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
                    SplitDir::Horizontal => egui::CursorIcon::ResizeHorizontal,
                    SplitDir::Vertical => egui::CursorIcon::ResizeVertical,
                });
                // A 2 px accent line down the middle of the gap.
                let line = match d.dir {
                    SplitDir::Horizontal => Rect::from_center_size(d.rect.center(), vec2(2.0, d.rect.height() - 16.0)),
                    SplitDir::Vertical => Rect::from_center_size(d.rect.center(), vec2(d.rect.width() - 16.0, 2.0)),
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
        let field_w = (rect.width() - 230.0).clamp(60.0, 150.0);
        let dark = ui.visuals().dark_mode;
        egui::Area::new(egui::Id::new("find-bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2((rect.right() - 340.0).max(rect.left() + 6.0), rect.top() + 8.0))
            .constrain_to(rect)
            .show(ui.ctx(), |ui| {
                egui::Frame::NONE
                    .fill(p.menu)
                    .stroke(Stroke::new(1.0, p.border_strong))
                    .corner_radius(CornerRadius::same(9))
                    .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: egui::Color32::from_black_alpha(if dark { 89 } else { 26 }) })
                    .inner_margin(egui::Margin { left: 10, right: 4, top: 0, bottom: 0 })
                    .show(ui, |ui| {
                        ui.set_height(36.0);
                        ui.horizontal_centered(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(RichText::new(icons::MAGNIFYING_GLASS).font(style::icon_font(14.0)).color(p.text_faint));
                            let fid = egui::Id::new("pane-find-input");
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut search.query)
                                    .id(fid)
                                    .frame(egui::Frame::NONE)
                                    .hint_text(RichText::new(t("Find")).color(p.text_faint))
                                    .font(egui::FontId::proportional(style::BODY))
                                    .text_color(p.text)
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
                            // "3 of 12", or that nothing matches.
                            let (count, c) = match (search.count, search.missing) {
                                (_, true) => (t("No results").to_string(), p.err),
                                (Some((at, n)), _) => (tf("{} of {}", &[&at.to_string(), &n.to_string()]), p.text_muted),
                                (None, _) => (String::new(), p.text_muted),
                            };
                            ui.label(RichText::new(count).font(style::mono(style::DATA)).color(c));
                            // Capitals in the query make it match case.
                            let cased = search.query.chars().any(char::is_uppercase);
                            let (ar, _) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::hover());
                            if cased {
                                ui.painter().rect_filled(ar, CornerRadius::same(5), p.accent_soft);
                            }
                            ui.painter().text(ar.center(), Align2::CENTER_CENTER, "Aa", style::medium(style::SMALL), if cased { p.text } else { p.text_muted });
                            ui.interact(ar, fid.with("case"), Sense::hover()).on_hover_text(t("Matches case when the query has capitals"));
                            let (sep, _) = ui.allocate_exact_size(vec2(1.0, 16.0), Sense::hover());
                            ui.painter().vline(sep.center().x, sep.y_range(), Stroke::new(1.0, p.border_strong));
                            if kit::icon_button_ex(ui, icons::CARET_UP, t("Older"), false, 24.0).clicked() {
                                go = Some(true);
                            }
                            if kit::icon_button_ex(ui, icons::CARET_DOWN, t("Newer"), false, 24.0).clicked() {
                                go = Some(false);
                            }
                            if let Some(backward) = go {
                                match tc_term::search::find(&shared, &search.query, search.current.as_ref(), backward) {
                                    Some(m) => {
                                        tc_term::search::reveal(&shared, &m);
                                        let all = tc_term::search::find_all(&shared, &search.query, 999);
                                        search.count = all.iter().position(|(x, _)| x == &m).map(|i| (i + 1, all.len()));
                                        search.current = Some(m);
                                        search.missing = false;
                                    }
                                    None => {
                                        search.missing = !search.query.is_empty();
                                        search.count = None;
                                    }
                                }
                            }
                            if kit::icon_button_ex(ui, icons::X, t("Close"), false, 24.0).clicked() {
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
        let dark = ui.visuals().dark_mode;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("pane-header:{pane_id}"), rect);
        let Some(pane) = self.panes.get(&pane_id) else {
            return;
        };
        let root = self.project(pane.project).map(|x| x.path.clone()).unwrap_or_default();
        let s = pane.tracker.status;
        let cy = rect.center().y;
        let label = pane.name();
        let place = pane.place(&root);
        let message = pane.tracker.message.clone().filter(|_| s == Status::NeedsInput).map(|m| m.replacen(": ", " · ", 1));
        let doing = pane.program_title.clone();
        let tip = status::described(&pane.tracker);
        let agent = pane.tracker.agent;
        let has_agent = agent.is_some();
        let since = pane.tracker.since;
        let unconnected = self.unconnected(pane).map(|a| (pane_id, a));
        let quiet = matches!(s, Status::Idle | Status::Ended) || (s == Status::Unknown && !has_agent);

        // From the right: the menu, the account, how long, where it runs.
        let mut right = rect.right() - 8.0;
        let menu_rect = Rect::from_center_size(pos2(right - 12.0, cy), vec2(24.0, 24.0));
        right -= 24.0 + 6.0;
        let menu = ui
            .scope_builder(egui::UiBuilder::new().max_rect(menu_rect), |ui| kit::icon_button_ex(ui, icons::DOTS_THREE, t("Pane"), false, 24.0))
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("pane-menu:{pane_id}"), menu_rect);
        egui::Popup::menu(&menu).align(egui::RectAlign::BOTTOM_END).show(|ui| self.pane_menu(ui, pane_id));
        // The agent's account (Claude Code, Codex, Antigravity): a chip, and a switch when another is ready.
        let show_profile = self
            .panes
            .get(&pane_id)
            .and_then(|x| x.tracker.agent.or(x.last_agent))
            .is_some_and(crate::profiles::supports_profile);
        if show_profile && rect.width() > 340.0 {
            let left = (right - 270.0).max(rect.left() + 160.0).min(right);
            let switcher_area = Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom()));
            let used_left = ui
                .scope_builder(egui::UiBuilder::new().max_rect(switcher_area).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                    ui.set_clip_rect(switcher_area.intersect(ui.clip_rect()));
                    self.pane_profile_switcher(ui, pane_id);
                    ui.min_rect().left()
                })
                .inner;
            right = used_left - 8.0;
        }
        let painter = ui.painter_at(rect);
        let min_right = rect.left() + 160.0;
        // How long it has been running, or waiting: "1:42".
        if matches!(s, Status::Running | Status::NeedsInput) && right > min_right + 36.0 {
            let secs = since.elapsed().as_secs();
            let text = if secs >= 3600 { format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60) } else { format!("{}:{:02}", secs / 60, secs % 60) };
            let g = painter.layout_no_wrap(text, style::mono(style::DATA), p.text_muted);
            if right - g.size().x > min_right {
                right -= g.size().x;
                painter.galley(pos2(right, cy - g.size().y / 2.0), g, p.text_muted);
                right -= 10.0;
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
        }
        if let Some(agent) = unconnected {
            // An agent without the hooks: it can't say when it needs the user. Click: Connect.
            let g = painter.layout_no_wrap(t("Not connected · Connect").to_string(), style::medium(style::SMALL), p.warn);
            let gw = g.size().x;
            let b = Rect::from_min_max(pos2(right - gw - 8.0 - 13.0 - 6.0 - 8.0, cy - 11.0), pos2(right, cy + 11.0));
            if b.left() > min_right {
                let r = ui.interact(b, egui::Id::new(("unconnected", pane_id)), Sense::click());
                #[cfg(debug_assertions)]
                crate::devtest::record(format!("unconnected:{pane_id}"), b);
                painter.rect_filled(b, CornerRadius::same(6), style::tint(p.warn, dark));
                if r.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                painter.text(pos2(b.left() + 8.0 + 6.5, cy), Align2::CENTER_CENTER, icons::PLUGS, style::icon_font(13.0), p.warn);
                painter.galley(pos2(b.left() + 8.0 + 13.0 + 6.0, cy - g.size().y / 2.0), g, p.warn);
                if r.on_hover_text(self.unconnected_tip(&[agent])).clicked() {
                    self.actions.push(Action::PlanInstall(agent.1));
                }
                right = b.left() - 8.0;
            }
        }
        if let Some(place) = &place
            && right > min_right + 40.0
        {
            let max_place = ((right - rect.left()) * 0.35).min((right - min_right - 12.0).max(0.0));
            let mut job = egui::text::LayoutJob::simple_singleline(place.clone(), style::mono(style::DATA), p.text_faint);
            job.wrap = egui::text::TextWrapping::truncate_at_width(max_place);
            let g = painter.layout_job(job);
            let w = g.size().x;
            if w > 8.0 && right - w > min_right {
                painter.galley(pos2(right - w, cy - g.size().y / 2.0), g, p.text_faint);
                right -= w + 10.0;
            }
        }

        // From the left: the agent's tile (or the terminal's icon), the name, the status pill,
        // what it does or asks.
        let clip = Rect::from_min_max(rect.min, pos2(right.max(rect.left()), rect.max.y));
        let painter = painter.with_clip_rect(clip);
        let mut x = rect.left() + 12.0;
        let mark = Rect::from_center_size(pos2(x + 8.0, cy), vec2(16.0, 16.0));
        kit::paint_agent_avatar(&painter, mark, agent, &p);
        x += 16.0 + 8.0;
        let label_c = if focused { p.text } else { p.text_muted };
        let mut job = egui::text::LayoutJob::simple_singleline(label.clone(), style::medium(style::SMALL), label_c);
        job.wrap = egui::text::TextWrapping::truncate_at_width((clip.right() - x - 60.0).max(40.0));
        let g = painter.layout_job(job);
        let lw = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, label_c);
        x += lw + 8.0;
        if !quiet || has_agent {
            let word = if s == Status::Running && !has_agent { t("Running").to_string() } else { status::text(s) };
            let word = match s {
                Status::Failed(_) => t("Failed").to_string(),
                _ => word,
            };
            if clip.right() - x > kit::state_pill_width(&painter, s, &word) + 8.0 {
                let r = kit::paint_state_pill(&painter, pos2(x, cy), s, &word, &p, dark);
                x = r.right() + 8.0;
            }
        }
        // What it asks, or says it is doing, quiet.
        let task = message.or_else(|| doing.clone().filter(|d| d != &label));
        if let Some(task) = task {
            let room = clip.right() - x - 8.0;
            if room > 40.0 {
                let mut job = egui::text::LayoutJob::simple_singleline(task, egui::FontId::proportional(style::SMALL), p.text_faint);
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
        let Some((agent, _)) = self.panes.get(&pane_id).and_then(|x| x.resume.clone()) else { return };
        let out = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                kit::notice(
                    ui,
                    rect.width(),
                    kit::Tone::Info,
                    Some(icons::CLOCK_COUNTER_CLOCKWISE),
                    &tf("{} ran here before the restart", &[agent.label()]),
                    Some(&tf("Resume continues {}'s last session in this pane", &[agent.label()])),
                    Some((t("Resume session"), Some("↵"))),
                    None,
                    true,
                )
            })
            .inner;
        #[cfg(debug_assertions)]
        {
            let at = if tile { "tile-" } else { "" };
            crate::devtest::record(format!("{at}resume:{pane_id}"), out.rect);
            crate::devtest::record(format!("{at}dismiss:{pane_id}"), out.rect);
        }
        let _ = tile;
        if out.primary {
            self.actions.push(Action::Resume(pane_id));
        }
        if out.dismissed {
            self.actions.push(Action::DismissResume(pane_id));
        }
    }

    /// The bar at the bottom of a pane whose shell ended (or could not start), a card inside
    /// its sheet: what happened, and a new shell (Enter) or closing the pane.
    fn exit_bar(&mut self, ui: &mut Ui, pane_id: PaneId, rect: Rect) {
        let Some(pane) = self.panes.get(&pane_id) else { return };
        let failed = pane.start_error.clone();
        let out = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| match (&failed, pane.exit) {
                (Some(why), _) => kit::notice(ui, rect.width(), kit::Tone::Failed, None, t("The shell could not start"), Some(why), Some((t("Try again"), Some("↵"))), None, false),
                (None, exit) => {
                    let title = match exit {
                        Some(tc_term::pty::ExitInfo::Code(c)) => tf("Exited with code {}", &[&c.to_string()]),
                        Some(tc_term::pty::ExitInfo::Signal(s)) => tf("Ended by signal {}", &[&s.to_string()]),
                        _ => t("Shell ended").to_string(),
                    };
                    kit::notice(ui, rect.width(), kit::Tone::Neutral, None, &title, Some(t("Press ↵ for a new shell")), Some((t("New shell"), Some("↵"))), Some(t("Close pane")), false)
                }
            })
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("exit-bar:{pane_id}"), out.rect);
        if out.primary {
            self.actions.push(Action::RestartShell(pane_id));
        }
        if out.secondary {
            self.actions.push(Action::ClosePaneId(pane_id));
        }
    }

    /// Notice strip rendered at the bottom of an agent pane waiting for permission or input
    /// (`Status::NeedsInput`). Shows what the agent is asking without Reject/Approve buttons so
    /// the user handles the prompt directly inside the agent's own terminal UI.
    fn approval_bar(&mut self, ui: &mut Ui, pane_id: PaneId, rect: Rect) {
        let Some(pane) = self.panes.get(&pane_id) else { return };
        let who = pane.tracker.agent.map_or_else(|| t("The agent").to_string(), |a| a.label().to_string());
        let ask = ask_of(pane.tracker.message.as_deref());
        let (title, body) = match &ask {
            Ask::Permission { tool, target } if tool == "Bash" => (tf("{} wants to run a command", &[&who]), Some(format!("Bash · {target}"))),
            Ask::Permission { tool, target } => (tf("{} wants to use {}", &[&who, tool]), Some(format!("{tool} · {target}"))),
            Ask::Plan(plan) => (tf("{} wants to apply its plan", &[&who]), Some(plan.clone())),
            Ask::Question(q) => (tf("{} has a question", &[&who]), Some(q.clone())),
            Ask::Confirm(said) => (tf("{} needs your confirmation", &[&who]), said.clone()),
        };
        let out = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                kit::notice(ui, rect.width(), kit::Tone::Input, None, &title, body.as_deref(), None, None, false)
            })
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("approval:{pane_id}"), out.rect);
        if ui.interact(out.rect, egui::Id::new(("approval-focus", pane_id)), Sense::click()).clicked() {
            self.actions.push(Action::FocusPane(pane_id));
        }
    }

    /// An agent that finished its turn, at the foot of its pane: what its run changed (or its
    /// last words), then Review and Commit…. Typing in the pane puts it away.
    fn done_notice(&mut self, ui: &mut Ui, pane_id: PaneId, rect: Rect) {
        let Some(pane) = self.panes.get(&pane_id) else { return };
        let who = pane.tracker.agent.map_or_else(|| t("The agent").to_string(), |a| a.label().to_string());
        let body = match pane.done_stats.as_ref().and_then(|(_, s)| *s) {
            Some((files, add, del)) if files > 0 => format!("{} · +{add} −{del}", crate::i18n::count(files, t("1 file"), t("{} files"))),
            _ => pane.last_words.clone().unwrap_or_default(),
        };
        let out = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                kit::notice(
                    ui,
                    rect.width(),
                    kit::Tone::Done,
                    None,
                    &tf("{} finished its turn", &[&who]),
                    (!body.is_empty()).then_some(body.as_str()),
                    Some((t("Commit…"), None)),
                    Some(t("Review")),
                    false,
                )
            })
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("done:{pane_id}"), out.rect);
        if out.primary {
            self.actions.push(Action::Commit);
        }
        if out.secondary {
            self.actions.push(Action::Review);
        }
    }

    /// A long command that failed (not yet seen), at the foot of its pane: what failed, and
    /// Dismiss (marks it seen).
    fn failed_notice(&mut self, ui: &mut Ui, pane_id: PaneId, rect: Rect) {
        let Some(pane) = self.panes.get(&pane_id) else { return };
        let Status::Failed(code) = pane.tracker.status else { return };
        let what = pane.tracker.message.clone().or_else(|| pane.program_title.clone()).unwrap_or_else(|| pane.label());
        let body = format!("{what} · {}", status::age(pane.tracker.since));
        let out = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                kit::notice(ui, rect.width(), kit::Tone::Failed, None, &tf("Exited with code {}", &[&code.to_string()]), Some(&body), None, None, true)
            })
            .inner;
        if out.dismissed {
            self.actions.push(Action::MarkSeen(pane_id));
        }
    }

    /// A usage limit in an agent's pane: who hit it and when it resets, then Resume on the next
    /// healthy account (two when there is room); after an automatic switch, where it went.
    fn rate_limit_notice(&mut self, ui: &mut Ui, pane_id: PaneId, rect: Rect) {
        let now = Instant::now();
        let Some(banner) = self.panes.get(&pane_id).and_then(|x| x.rate_limit.clone()) else { return };
        let profiles = self.effective_profiles();
        let cross = self.settings.failover.cross_provider;
        let mut candidates: Vec<&tc_core::model::AgentProfile> = profiles
            .iter()
            .filter(|pr| {
                pr.enabled
                    && pr.id != banner.from_profile
                    && (cross || pr.agent == banner.agent)
                    && self.profiles_rt.get(&pr.id).is_none_or(|rt| rt.is_healthy(now))
            })
            .collect();
        candidates.sort_by_key(|pr| (pr.agent != banner.agent, pr.fallback_order, pr.id.as_str()));
        let next: Vec<String> = candidates.into_iter().take(2).map(|pr| pr.id.clone()).collect();
        let out = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                if let Some(to) = &banner.auto_switched_to {
                    kit::notice(
                        ui,
                        rect.width(),
                        kit::Tone::Done,
                        Some(icons::LIGHTNING),
                        &tf("{} hit its limit — resumed on {}", &[&banner.from_profile, to]),
                        Some(t("Switched automatically")),
                        None,
                        None,
                        true,
                    )
                } else {
                    let rem_dur = banner.reset_at.saturating_duration_since(now);
                    let rem = rem_dur.as_secs() + u64::from(rem_dur.subsec_nanos() > 0);
                    let mut body = if rem > 0 { tf("Resets in {}", &[&crate::profiles::format_countdown(rem)]) } else { String::new() };
                    if let Some(first) = next.first() {
                        if !body.is_empty() {
                            body.push_str(" · ");
                        }
                        body.push_str(&tf("{} is ready", &[first]));
                    }
                    let primary = next.first().map(|id| tf("Resume on {}", &[id]));
                    let secondary = next.get(1).map(|id| tf("Resume on {}", &[id]));
                    kit::notice(
                        ui,
                        rect.width(),
                        kit::Tone::Limited,
                        None,
                        &tf("{} hit its usage limit", &[&banner.from_profile]),
                        (!body.is_empty()).then_some(body.as_str()),
                        primary.as_deref().map(|p| (p, Some("↵"))),
                        secondary.as_deref(),
                        true,
                    )
                }
            })
            .inner;
        #[cfg(debug_assertions)]
        {
            crate::devtest::record(format!("rate-limit-banner:{pane_id}"), out.rect);
            if let Some(id) = next.first() {
                crate::devtest::record(format!("failover-resume:{id}"), out.rect);
            }
        }
        if out.dismissed {
            self.actions.push(Action::DismissRateLimit(pane_id));
        }
        if out.primary
            && let Some(id) = next.first()
        {
            self.actions.push(Action::ResumeWithProfile { pane: pane_id, profile_id: id.clone() });
        }
        if out.secondary
            && let Some(id) = next.get(1)
        {
            self.actions.push(Action::ResumeWithProfile { pane: pane_id, profile_id: id.clone() });
        }
    }

    /// Scrolled back into the history: a pill at the foot of the terminal says how far, and a
    /// click goes back down to the newest output.
    fn scrolled_pill(&mut self, ui: &mut Ui, pane_id: PaneId, term: Rect, lines: usize) {
        let p = style::pal(ui);
        let text = if lines == 1 { t("1 line below").to_string() } else { tf("{} lines below", &[&lines.to_string()]) };
        let g = ui.painter().layout_no_wrap(text, style::medium(style::SMALL), p.text);
        let keys = "⇧End";
        let kw = kit::kbd_width(ui.painter(), keys);
        let w = 10.0 + 12.0 + 8.0 + g.size().x + 8.0 + kw + 6.0;
        let rect = Rect::from_center_size(pos2(term.center().x, term.bottom() - 20.0), vec2(w, 28.0));
        if rect.width() > term.width() - 16.0 {
            return;
        }
        let r = ui.interact(rect, egui::Id::new(("scrolled-pill", pane_id)), Sense::click());
        let painter = ui.painter();
        let shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: egui::Color32::from_black_alpha(if ui.visuals().dark_mode { 89 } else { 26 }) };
        painter.add(shadow.as_shape(rect, CornerRadius::same(14)));
        painter.rect(rect, CornerRadius::same(14), if r.hovered() { style::over(p.menu, p.text, 0.04) } else { p.menu }, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
        let cy = rect.center().y;
        painter.text(pos2(rect.left() + 10.0 + 6.0, cy), Align2::CENTER_CENTER, icons::ARROW_DOWN, style::icon_font(12.0), p.text);
        let gx = rect.left() + 10.0 + 12.0 + 8.0;
        let gw = g.size().x;
        painter.galley(pos2(gx, cy - g.size().y / 2.0), g, p.text);
        kit::paint_kbd_text(painter, pos2(gx + gw + 8.0, cy), keys, &p);
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if r.clicked()
            && let Some(pane) = self.panes.get(&pane_id)
        {
            widget::scroll_to_bottom(&pane.shared);
            self.focus_request = Some(pane_id);
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

    /// The Overview's bar: its title and counts at the left; the filter, how many tiles a
    /// screen holds and Run agent at the right (`grid_toolbar`).
    pub fn overview_bar(&mut self, ui: &mut Ui) -> Rect {
        let p = style::pal(ui);
        let full = ui.max_rect();
        let n = self.projects.len();
        let worktrees = self.projects.iter().filter(|x| x.is_worktree()).count();
        let waiting = self.attention_queue().len();
        let active = self.active_projects();
        let body = self.top_bar(
            ui,
            full,
            |_, ui| {
                ui.add(egui::Label::new(RichText::new(t("Overview")).font(style::semibold(style::TITLE)).color(p.text)).selectable(false));
                // "6 projects · 1 worktree · 4 active · 2 waiting", when there is room for it.
                let mut parts = vec![crate::i18n::count(n - worktrees, t("1 project"), t("{} projects"))];
                if worktrees > 0 {
                    parts.push(crate::i18n::count(worktrees, t("1 worktree"), t("{} worktrees")));
                }
                parts.push(tf("{} active", &[&active.to_string()]));
                parts.push(tf("{} waiting", &[&waiting.to_string()]));
                let counts = parts.join(" · ");
                let w = ui.painter().layout_no_wrap(counts.clone(), style::mono(style::DATA), p.text_faint).size().x;
                if ui.available_width() > w + 12.0 {
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(RichText::new(counts).font(style::mono(style::DATA)).color(p.text_faint)).selectable(false));
                }
            },
            None,
        );
        // The same area as the Focus view's (one top bar above both): panes shown in the Grid
        // are sized against it.
        self.last_full_body = Some(sheets_area(body, self.settings.sidebar_hidden));
        body
    }

    /// The Overview's toolbar (laid out right to left): Run agent, how many tiles a screen
    /// holds, and the filter (All, Active, Waiting, each with its count).
    fn grid_toolbar(&mut self, ui: &mut Ui) {
        use tc_core::model::GridFilter;
        let p = style::pal(ui);
        let compact = ui.available_width() < 760.0;
        let run_keys = (!compact).then(|| kit::keys(&Action::PickPreset)).flatten();
        let run = kit::button_keys(ui, kit::Kind::Primary, Some(icons::PLAY), t("Run agent"), run_keys, false);
        #[cfg(debug_assertions)]
        crate::devtest::record("grid-run-agent", run.rect);
        if run.on_hover_text(kit::tip(t("Start an agent in a new pane"), Action::PickPreset)).clicked() {
            self.actions.push(Action::PickPreset);
        }
        ui.add_space(10.0);

        // Tiles in view: "2 × 2" (Automatic is two by two too), a menu of the others.
        let side = if self.grid_columns == 0 { 2 } else { self.grid_columns };
        let g = ui.painter().layout_no_wrap(format!("{side} × {side}"), style::mono(style::DATA), p.text_muted);
        let (rect, r) = ui.allocate_exact_size(vec2(10.0 + 14.0 + 6.0 + g.size().x + 6.0 + 10.0 + 10.0, 28.0), Sense::click());
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&r));
        let painter = ui.painter();
        painter.rect(rect, CornerRadius::same(style::RADIUS_ROW), if r.hovered() || open { p.hover } else { egui::Color32::TRANSPARENT }, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
        let cy = rect.center().y;
        painter.text(pos2(rect.left() + 10.0 + 7.0, cy), Align2::CENTER_CENTER, icons::SQUARES_FOUR, style::icon_font(14.0), p.text_muted);
        let gw = g.size().x;
        painter.galley(pos2(rect.left() + 10.0 + 14.0 + 6.0, cy - g.size().y / 2.0), g, p.text_muted);
        painter.text(pos2(rect.left() + 10.0 + 14.0 + 6.0 + gw + 6.0 + 5.0, cy), Align2::CENTER_CENTER, icons::CARET_DOWN, style::icon_font(10.0), p.text_faint);
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
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
        ui.add_space(10.0);

        // The filter: each word with its count ("Waiting 2").
        let n = self.projects.len();
        let active = self.active_projects();
        let waiting = self.waiting_projects().len();
        let labels = [format!("{} {n}", t("All")), format!("{} {active}", t("Active")), format!("{} {waiting}", t("Waiting"))];
        let options = [
            kit::Seg::new(&labels[0], t("Every project's terminal; scroll for more")),
            kit::Seg::new(&labels[1], t("Only projects where something runs or waits, or an agent can be resumed")),
            kit::Seg::new(&labels[2], t("Only projects with something waiting for you")),
        ];
        let filters = [GridFilter::All, GridFilter::Active, GridFilter::Waiting];
        let on = filters.iter().position(|f| *f == self.grid_filter).unwrap_or(0);
        let r = ui.push_id("grid-filter", |ui| kit::segmented(ui, &options, on));
        #[cfg(debug_assertions)]
        for (rect, name) in super::grid::word_segments(ui, r.response.rect, &labels).into_iter().zip(["grid:all", "grid:active", "grid:waiting"]) {
            crate::devtest::record(name, rect);
        }
        if let Some(i) = r.inner {
            self.actions.push(Action::GridFilter(filters[i]));
        }
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
/// What an agent that waits for the user asks, read from its hook's message: a tool to allow
/// ("Bash: npm test"), a plan ("Plan: …"), a question, or a plain confirmation.
#[derive(Debug, PartialEq)]
pub(super) enum Ask {
    Permission { tool: String, target: String },
    Plan(String),
    Question(String),
    Confirm(Option<String>),
}

pub(super) fn ask_of(message: Option<&str>) -> Ask {
    let Some(m) = message.map(str::trim).filter(|m| !m.is_empty()) else { return Ask::Confirm(None) };
    if let Some(plan) = m.strip_prefix("Plan: ") {
        return Ask::Plan(plan.to_string());
    }
    if let Some((tool, target)) = m.split_once(": ")
        && !tool.is_empty()
        && tool.len() <= 40
        && tool.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && tool.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Ask::Permission { tool: tool.to_string(), target: target.to_string() };
    }
    if m.ends_with('?') && !m.starts_with("Allow ") {
        return Ask::Question(m.to_string());
    }
    Ask::Confirm(Some(m.to_string()))
}

/// A chip in the project bar: an icon and a short mono label (the branch), clicked to review.
fn bar_chip(ui: &mut Ui, icon: &str, label: &str) -> egui::Response {
    let p = style::pal(ui);
    let g = ui.painter().layout_no_wrap(label.to_string(), style::mono(style::DATA), p.text_muted);
    let w = 8.0 + 13.0 + 6.0 + g.size().x + 8.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    ui.painter().rect_stroke(rect, CornerRadius::same(style::RADIUS), Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(style::RADIUS), p.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(pos2(rect.left() + 8.0 + 6.5, rect.center().y), Align2::CENTER_CENTER, icon, style::icon_font(13.0), p.text_faint);
    ui.painter().galley(pos2(rect.left() + 8.0 + 13.0 + 6.0, rect.center().y - g.size().y / 2.0), g, p.text_muted);
    resp
}

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

#[cfg(test)]
mod ask_tests {
    use super::*;

    #[test]
    fn reads_what_the_agent_asks() {
        assert_eq!(ask_of(Some("Bash: npm test")), Ask::Permission { tool: "Bash".into(), target: "npm test".into() });
        assert_eq!(ask_of(Some("Plan: Add a cart badge")), Ask::Plan("Add a cart badge".into()));
        assert_eq!(ask_of(Some("Which database should I use?")), Ask::Question("Which database should I use?".into()));
        assert_eq!(ask_of(Some("Allow Edit(src/main.rs)?")), Ask::Confirm(Some("Allow Edit(src/main.rs)?".into())));
        assert_eq!(ask_of(Some("Claude needs your permission to use Bash")), Ask::Confirm(Some("Claude needs your permission to use Bash".into())));
        assert_eq!(ask_of(None), Ask::Confirm(None));
    }
}
