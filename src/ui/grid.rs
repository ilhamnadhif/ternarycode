//! Grid (FR-V2–V5, V7): every project's terminal at once, two columns
//! and two rows to a screen, scrolling for more ("Active" keeps only projects where something
//! runs or waits). Each tile shows the project's tab in view (the one Focus has open), its header
//! speaking of that tab only (other tabs that wait get a hint), with its terminals sized to
//! the tile, so they fill it (user decision, 10 November 2026: a terminal kept at its Focus size
//! and scaled left empty space or cut columns). Going back to Focus gives them their Focus size
//! again; an agent redraws once per switch, as in any terminal that is resized. A tile is drawn
//! only when it is in view, and a sleeping pane's shell starts when its tile is first drawn (as
//! opening the project would), at the tile's size.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::ProjectId;
use tc_core::status::{Foreground, Status, Tracker};
use tc_core::{PaneId, model::{GridFilter, View}};
use tc_term::widget;

use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};

const TILE_HEADER: f32 = 34.0;
const GAP: f32 = 8.0;
/// The Grid's scroll bar, and the room between it and the tiles.
const SCROLL_BAR: f32 = 8.0;
const SCROLL_GAP: f32 = 4.0;
/// Tiles never draw terminal text smaller than this share of the font size (so ⌘= and ⌘−
/// work here too); a terminal too big to fit at that size is cut around its cursor instead of
/// shrunk further.
const MIN_SCALE: f32 = 0.85;
/// A program this long in the foreground makes its project active (a quick `ls` would make
/// tiles come and go).
pub const ACTIVE_AFTER: Duration = Duration::from_secs(2);

/// How a pane counts for the Grid's "Active" filter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Activity {
    /// Wants attention, runs an agent, or has had a program in the foreground a while.
    Active,
    /// A program started less than `ACTIVE_AFTER` ago: active in this long.
    Soon(Duration),
    Idle,
}

pub fn activity(t: &Tracker, now: Instant) -> Activity {
    if t.wants_attention() || t.agent.is_some() {
        return Activity::Active;
    }
    if matches!(t.fg, Foreground::Program { .. }) {
        let ran = t.fg_for(now);
        return if ran >= ACTIVE_AFTER {
            Activity::Active
        } else {
            Activity::Soon(ACTIVE_AFTER - ran)
        };
    }
    Activity::Idle
}

/// Columns of the Grid and the rows one screen holds for `n` tiles: 2 × 2 (a tile alone fills
/// the view, two sit side by side); more projects scroll. A saved column count `fixed` makes it
/// `fixed` × `fixed`.
pub fn grid_shape(n: usize, fixed: u32) -> (usize, usize) {
    let side = if fixed > 0 { fixed as usize } else { 2 };
    let n = n.max(1);
    let cols = side.min(n);
    (cols, n.div_ceil(cols).min(side))
}

impl App {
    /// The pane that speaks for `panes`: one that needs attention, else one running an agent.
    fn telling_pane(&self, panes: &[PaneId]) -> Option<PaneId> {
        let wants = panes
            .iter()
            .filter_map(|id| self.panes.get(id))
            .filter(|x| x.tracker.wants_attention())
            .min_by_key(|x| (x.tracker.status.priority(), x.tracker.since))
            .map(|x| x.id);
        let agent = || {
            panes
                .iter()
                .filter_map(|id| self.panes.get(id))
                .find(|x| x.tracker.agent.is_some())
                .map(|x| x.id)
        };
        wants.or_else(agent)
    }

    /// The pane a tile's header describes: of the tab the tile shows, one that needs attention,
    /// else one running an agent, else the tab's focused pane. Other tabs that wait are told
    /// apart (`waiting_elsewhere`): the header never speaks of a pane not in view.
    pub fn tile_pane(&self, project: ProjectId) -> Option<PaneId> {
        let tab = self.project(project)?.tab()?;
        self.telling_pane(&tab.layout.panes()).or(Some(tab.focused))
    }

    /// The same over all of a project's tabs: the pane its sidebar row speaks of.
    pub fn project_pane(&self, project: ProjectId) -> Option<PaneId> {
        let p = self.project(project)?;
        self.telling_pane(&p.panes()).or_else(|| p.current_pane())
    }

    /// Panes of a project's other tabs (not the one in view) that need attention: how many, and
    /// the most urgent status among them.
    fn waiting_elsewhere(&self, project: ProjectId) -> Option<(usize, Status)> {
        let p = self.project(project)?;
        let waiting: Vec<Status> = p
            .tabs
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != p.active_tab)
            .flat_map(|(_, tab)| tab.layout.panes())
            .filter_map(|id| self.panes.get(&id))
            .filter(|x| x.tracker.wants_attention())
            .map(|x| x.tracker.status)
            .collect();
        let most = waiting.iter().copied().min_by_key(|s| s.priority())?;
        Some((waiting.len(), most))
    }

    /// The project's most active pane counts for it; an agent to resume makes it active.
    pub fn project_activity(&self, project: ProjectId, now: Instant) -> Activity {
        let Some(p) = self.project(project) else {
            return Activity::Idle;
        };
        let mut out = Activity::Idle;
        for pane in p.panes().iter().filter_map(|id| self.panes.get(id)) {
            // An agent session that ran here before a restart, waiting to be resumed.
            if pane.resume.is_some() {
                return Activity::Active;
            }
            match (activity(&pane.tracker, now), out) {
                (Activity::Active, _) => return Activity::Active,
                (Activity::Soon(d), Activity::Soon(e)) => out = Activity::Soon(d.min(e)),
                (Activity::Soon(d), _) => out = Activity::Soon(d),
                _ => {}
            }
        }
        out
    }

    /// Projects where something runs or waits.
    pub fn active_projects(&self) -> usize {
        let now = Instant::now();
        self.projects
            .iter()
            .filter(|p| self.project_activity(p.id, now) == Activity::Active)
            .count()
    }

    /// Projects with something the user has not seen yet.
    pub fn waiting_projects(&self) -> Vec<ProjectId> {
        self.projects
            .iter()
            .filter(|p| p.panes().iter().filter_map(|id| self.panes.get(id)).any(|x| x.tracker.wants_attention()))
            .map(|p| p.id)
            .collect()
    }

    /// The projects the Grid shows: all of them; with "Active" those where something runs or
    /// waits; with "Waiting" those with something not seen yet. A frame is asked for when a
    /// program is about to make a project active, since nothing else may draw one.
    fn shown_projects(&self, ctx: &egui::Context) -> Vec<ProjectId> {
        match self.grid_filter {
            GridFilter::All => return self.projects.iter().map(|p| p.id).collect(),
            GridFilter::Waiting => return self.waiting_projects(),
            GridFilter::Active => {}
        }
        let now = Instant::now();
        let mut soon: Option<Duration> = None;
        let mut out = Vec::new();
        for p in &self.projects {
            match self.project_activity(p.id, now) {
                Activity::Active => out.push(p.id),
                Activity::Soon(d) => soon = Some(soon.map_or(d, |s| s.min(d))),
                Activity::Idle => {}
            }
        }
        if let Some(d) = soon {
            ctx.request_repaint_after(d + Duration::from_millis(20));
        }
        out
    }

    pub fn grid_view(&mut self, ui: &mut Ui) {
        // Darker than the terminals, so each tile reads as a card.
        ui.painter().rect_filled(ui.max_rect(), CornerRadius::ZERO, style::pal(ui).chrome);
        let area = ui.max_rect().shrink(GAP);
        let ids = self.shown_projects(ui.ctx());
        if ids.is_empty() {
            self.nothing_active(ui, area);
            return;
        }
        // Showing every project, the place after the last adds one (when the row has room).
        let (cols0, _) = grid_shape(ids.len() + 1, self.grid_columns);
        let add_tile = self.grid_filter == GridFilter::All && !ids.len().is_multiple_of(cols0) && ids.len() > 1;
        let n = ids.len() + add_tile as usize;
        let (cols, rows_in_view) = grid_shape(n, self.grid_columns);
        let rows = n.div_ceil(cols);
        let h = ((area.height() - GAP * (rows_in_view as f32 - 1.0)) / rows_in_view as f32).max(TILE_HEADER + 80.0);
        let total = rows as f32 * h + GAP * (rows as f32 - 1.0);
        let scrolls = total > area.height() + 0.5;
        // Room for the scroll bar beside the tiles.
        let width = area.width() - if scrolls { SCROLL_BAR + SCROLL_GAP } else { 0.0 };
        let w = (width - GAP * (cols as f32 - 1.0)) / cols as f32;
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            // A bar always in view when there is more below the 2 × 2: a track, and a handle
            // that stands out from the dark Grid (the plain widget fill would not).
            let p = style::pal(ui);
            let tiles_widgets = ui.visuals().widgets.clone();
            ui.spacing_mut().scroll = egui::style::ScrollStyle {
                bar_width: SCROLL_BAR,
                bar_inner_margin: SCROLL_GAP,
                bar_outer_margin: 0.0,
                handle_min_length: 48.0,
                ..egui::style::ScrollStyle::solid()
            };
            let handle = &mut ui.visuals_mut().widgets;
            handle.inactive.bg_fill = p.text_faint;
            handle.hovered.bg_fill = p.text_muted;
            handle.active.bg_fill = p.text_muted;
            let bar = if scrolls {
                egui::scroll_area::ScrollBarVisibility::AlwaysVisible
            } else {
                egui::scroll_area::ScrollBarVisibility::AlwaysHidden
            };
            egui::ScrollArea::vertical()
                .id_salt("grid")
                .auto_shrink([false, false])
                .scroll_bar_visibility(bar)
                .show(ui, |ui| {
                    ui.visuals_mut().widgets = tiles_widgets;
                    let (content, _) = ui.allocate_exact_size(vec2(width, total), Sense::hover());
                    let visible = ui.clip_rect();
                    let mut drawn = Vec::new();
                    if add_tile {
                        let i = ids.len();
                        let (c, r) = (i % cols, i / cols);
                        let rect = Rect::from_min_size(content.min + vec2(c as f32 * (w + GAP), r as f32 * (h + GAP)), vec2(w, h));
                        if rect.intersects(visible) {
                            self.add_tile(ui, rect);
                        }
                    }
                    for (i, id) in ids.into_iter().enumerate() {
                        let (c, r) = (i % cols, i / cols);
                        let rect = Rect::from_min_size(
                            content.min + vec2(c as f32 * (w + GAP), r as f32 * (h + GAP)),
                            vec2(w, h),
                        );
                        // Out of view: not drawn (and its shell not started for it).
                        if rect.intersects(visible) {
                            drawn.extend(self.project(id).and_then(|p| p.tab()).map(|t| t.layout.panes()).unwrap_or_default());
                            self.tile(ui, id, rect, !scrolls);
                        }
                    }
                    // More below: how many, at the bottom of the view; a click brings the next row.
                    let row_rect = |r: usize| {
                        Rect::from_min_size(content.min + vec2(0.0, r as f32 * (h + GAP)), vec2(width, h))
                    };
                    let next_row = (0..rows).find(|r| row_rect(*r).top() >= visible.bottom() - 1.0);
                    if let Some(next) = next_row {
                        let below = n - (next * cols).min(n);
                        self.more_below(ui, visible, below, row_rect(next));
                    }
                    // The keyboard was meant for a pane out of view (the project open before):
                    // it would jump there when scrolled to, after the user chose another tile.
                    if self.focus_request.is_some_and(|f| !drawn.contains(&f)) {
                        self.focus_request = None;
                    }
                });
        });
    }

    /// The note at the bottom of a scrolled Grid: how many projects are further down. Clicked, it
    /// scrolls the next row into view.
    fn more_below(&mut self, ui: &mut Ui, visible: Rect, below: usize, next_row: Rect) {
        let p = style::pal(ui);
        let text = tf("{} more below", &[&below.to_string()]);
        let g = ui.painter().layout_no_wrap(text, egui::FontId::proportional(style::SMALL), p.text);
        let w = 14.0 + 11.0 + 6.0 + g.size().x + 14.0;
        let pill = Rect::from_center_size(pos2(visible.center().x, visible.bottom() - 18.0 - 13.0), vec2(w, 26.0));
        let r = ui.interact(pill, egui::Id::new("grid-more-below"), Sense::click());
        #[cfg(debug_assertions)]
        crate::devtest::record("grid-more", pill);
        let fill = if r.hovered() { p.hover } else { p.surface };
        let painter = ui.painter();
        let shadow = egui::epaint::Shadow { offset: [0, 4], blur: 14, spread: 0, color: egui::Color32::from_black_alpha(if ui.visuals().dark_mode { 100 } else { 28 }) };
        painter.add(shadow.as_shape(pill, CornerRadius::same(13)));
        painter.rect(pill, CornerRadius::same(13), fill, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
        painter.text(pos2(pill.left() + 14.0 + 5.5, pill.center().y), Align2::CENTER_CENTER, icons::CARET_DOWN, style::icon_font(11.0), p.text);
        painter.galley(pos2(pill.left() + 14.0 + 11.0 + 6.0, pill.center().y - g.size().y / 2.0), g, p.text);
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if r.clicked() {
            ui.scroll_to_rect(next_row, Some(egui::Align::Max));
        }
    }

    /// The place after the last project: adds one (or a folder dropped on the window does).
    fn add_tile(&mut self, ui: &mut Ui, rect: Rect) {
        let p = style::pal(ui);
        let r = ui.interact(rect, egui::Id::new("grid-add"), Sense::click());
        let painter = ui.painter_at(rect.expand(1.0));
        let color = if r.hovered() { p.text_muted } else { p.text_faint };
        if r.hovered() {
            painter.rect_filled(rect, CornerRadius::same(style::RADIUS_LG), p.faint_row());
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // A dashed border.
        let rr = rect.shrink(0.5);
        let pts = [rr.left_top() + vec2(10.0, 0.0), rr.right_top() - vec2(10.0, 0.0)];
        let stroke = Stroke::new(1.0, p.border_strong);
        for (a, b) in [
            (pts[0], pts[1]),
            (rr.right_top() + vec2(0.0, 10.0), rr.right_bottom() - vec2(0.0, 10.0)),
            (rr.right_bottom() - vec2(10.0, 0.0), rr.left_bottom() + vec2(10.0, 0.0)),
            (rr.left_bottom() - vec2(0.0, 10.0), rr.left_top() + vec2(0.0, 10.0)),
        ] {
            painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 5.0, 4.0));
        }
        for (c, a0) in [(rr.left_top() + vec2(10.0, 10.0), 180.0_f32), (rr.right_top() + vec2(-10.0, 10.0), 270.0), (rr.right_bottom() + vec2(-10.0, -10.0), 0.0), (rr.left_bottom() + vec2(10.0, -10.0), 90.0)] {
            let arc: Vec<egui::Pos2> = (0..=6).map(|i| {
                let a = (a0 + i as f32 * 15.0).to_radians();
                c + vec2(a.cos(), a.sin()) * 10.0
            }).collect();
            painter.add(egui::Shape::line(arc, stroke));
        }
        let cy = rect.center().y;
        painter.text(pos2(rect.center().x, cy - 18.0), Align2::CENTER_CENTER, icons::PLUS, style::icon_font(16.0), color);
        painter.text(pos2(rect.center().x, cy + 4.0), Align2::CENTER_CENTER, t("Add Project…"), egui::FontId::proportional(12.5), color);
        painter.text(pos2(rect.center().x, cy + 21.0), Align2::CENTER_CENTER, t("or drop a folder here"), egui::FontId::proportional(style::SMALL), p.text_faint);
        if r.clicked() {
            self.actions.push(Action::AddProject);
        }
    }

    /// "Active" with nothing running or waiting.
    fn nothing_active(&mut self, ui: &mut Ui, area: Rect) {
        let p = style::pal(ui);
        let inner = Rect::from_center_size(area.center(), vec2(area.width().min(560.0), 90.0));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(egui::Align::Center)),
            |ui| {
                let hint = if self.grid_filter == GridFilter::Waiting {
                    t("Nothing is waiting for you.").to_string()
                } else {
                    tf("Nothing is running. Open a project, or press {} to start an agent.", &[kit::keys(&Action::PickPreset).unwrap_or("")])
                };
                ui.label(egui::RichText::new(hint).size(style::BODY).color(p.text_muted));
                ui.add_space(10.0);
                if kit::secondary(ui, None, t("Show All Projects")).clicked() {
                    self.actions.push(Action::GridFilter(GridFilter::All));
                }
            },
        );
    }

    /// One project: header (name, status, branch, a resume button, open in Focus) over its tab's
    /// panes. `wheel_free`: the mouse wheel may scroll the terminals (the Grid does not scroll);
    /// else only the one taking the keyboard gets it.
    fn tile(&mut self, ui: &mut Ui, project_id: ProjectId, rect: Rect, wheel_free: bool) {
        let p = style::pal(ui);
        let Some(project) = self.project(project_id) else {
            return;
        };
        let pane_id = self.tile_pane(project_id);
        let name = project.name.clone();
        let color = project.color32();
        let branch = project.git.as_ref().map(|g| g.branch.clone());
        let pane_status = pane_id.and_then(|id| self.panes.get(&id)).map(|x| {
            (
                x.tracker.status,
                x.tracker.wants_attention(),
                x.tracker.message.clone(),
                x.label(),
                status::described(&x.tracker),
                x.program_title.clone(),
            )
        });
        let elsewhere = self.waiting_elsewhere(project_id);
        // The tab in view, with all its panes (a split project shows its split).
        let layout = project.tab().map(|t| t.layout.clone());
        let count = layout.as_ref().map_or(0, |l| l.panes().len());
        // An agent that ran in one of them and can be continued.
        let resume = layout.as_ref().and_then(|l| {
            l.panes().into_iter().find_map(|id| {
                self.panes
                    .get(&id)
                    .and_then(|x| x.resume.as_ref())
                    .map(|(a, _)| (id, a.label()))
            })
        });
        // An agent that finished its turn: what it changed is one click away.
        let review = pane_id.filter(|id| self.panes.get(id).is_some_and(|x| x.tracker.status == Status::Done));
        // An agent without the hooks (it can't say when it needs the user).
        let unconnected = pane_id.and_then(|id| self.panes.get(&id).and_then(|x| self.unconnected(x)).map(|a| (id, a)));
        let keyboard_here = self
            .grid_focus
            .is_some_and(|f| layout.as_ref().is_some_and(|l| l.contains(f)));
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("tile:{name}"), rect);

        let dark = ui.visuals().dark_mode;
        let scheme = self.scheme(dark);
        let changed = project.git.as_ref().map_or(0, |g| g.changed);
        let radius = CornerRadius::same(style::RADIUS_LG);
        let hovered = ui.rect_contains_pointer(rect);
        let progress = pane_id.and_then(|id| self.panes.get(&id)).and_then(|x| x.progress);
        let attention = pane_status.as_ref().is_some_and(|x| x.1);
        let status_now = pane_status.as_ref().map(|x| x.0);

        // The card: the terminal's own background, header included, on the darker Grid.
        let painter = ui.painter_at(rect.expand(2.0));
        painter.rect_filled(rect, radius, scheme.background);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), TILE_HEADER));
        painter.hline(
            (rect.left() + 1.0)..=(rect.right() - 1.0),
            header.bottom() - 0.5,
            Stroke::new(1.0, p.border.gamma_multiply(0.7)),
        );
        let border = if keyboard_here {
            Stroke::new(1.5, p.accent)
        } else if attention && status_now == Some(Status::NeedsInput) {
            Stroke::new(1.0, style::over(p.border, p.warn, 0.55))
        } else if hovered {
            Stroke::new(1.0, p.border_strong)
        } else {
            Stroke::new(1.0, p.border)
        };

        // Header.
        let hr = ui.interact(header, egui::Id::new(("tile-header", project_id)), Sense::click());
        if hr.double_clicked() {
            self.actions.push(Action::SelectProject(project_id));
        }
        let cy = header.center().y;
        let small = egui::FontId::proportional(style::SMALL);
        // Right side first (open button, resume, review, panes, branch); the left side is cut
        // before it.
        let open_rect = Rect::from_center_size(pos2(header.right() - 6.0 - 11.0, cy), vec2(22.0, 22.0));
        let open = ui
            .scope_builder(egui::UiBuilder::new().max_rect(open_rect), |ui| {
                kit::icon_button_ex(ui, icons::ARROW_SQUARE_OUT, &kit::tip(t("Open in Focus view"), Action::OpenGridTile), false, 22.0)
            })
            .inner;
        if open.clicked() {
            self.actions.push(Action::SelectProject(project_id));
        }
        let mut right = open_rect.left() - 8.0;
        let button = |ui: &mut Ui, icon: Option<&str>, text: &str, right: &mut f32| {
            let w = kit::button_width(ui, icon, text, None, true);
            let r = Rect::from_center_size(pos2(*right - w / 2.0, cy), vec2(w, 24.0));
            *right -= w + 8.0;
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::button_ex(ui, kit::Kind::Secondary, icon, text, true)).inner
        };
        if let Some((id, agent)) = resume {
            let b = button(ui, Some(icons::PLAY), &tf("Resume {}", &[agent]), &mut right);
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("tile-resume:{id}"), b.rect);
            if b.on_hover_text(tf("Continue {}'s last session here", &[agent])).clicked() {
                self.actions.push(Action::Resume(id));
            }
        }
        if let Some(id) = review {
            let b = button(ui, Some(icons::GIT_DIFF), t("Review"), &mut right);
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("tile-review:{id}"), b.rect);
            if b.on_hover_text(kit::tip(t("Review what the agent changed"), Action::Review)).clicked() {
                self.actions.push(Action::FocusPane(id));
                self.actions.push(Action::Review);
            }
        }
        if let Some(agent) = unconnected {
            let r = Rect::from_center_size(pos2(right - 11.0, cy), vec2(22.0, 22.0));
            right -= 22.0 + 6.0;
            let resp = ui.interact(r, egui::Id::new(("tile-unconnected", project_id)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("tile-unconnected:{}", agent.0), r);
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(style::RADIUS), style::tint(p.warn, dark));
            }
            ui.painter().text(r.center(), Align2::CENTER_CENTER, icons::WARNING, style::icon_font(13.0), p.warn);
            if resp.on_hover_text(self.unconnected_tip(&[agent])).clicked() {
                self.actions.push(Action::PlanInstall(agent.1));
            }
        }
        // A local server its panes started: ":5173" opens it in the browser.
        let servers = self.project_servers(project_id);
        if let Some((_, first)) = servers.first() {
            let port = format!(":{}", first.port);
            let more = if servers.len() > 1 { format!(" +{}", servers.len() - 1) } else { String::new() };
            let g = ui.painter().layout_no_wrap(format!("{port}{more}"), small.clone(), p.text_muted);
            let w = 6.0 + 6.0 + 5.0 + g.size().x + 6.0;
            let r = Rect::from_center_size(pos2(right - w / 2.0, cy), vec2(w, 22.0));
            right -= w + 6.0;
            let resp = ui.interact(r, egui::Id::new(("tile-server", project_id)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("tile-server:{name}"), r);
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(style::RADIUS), p.hover);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            ui.painter().circle_filled(pos2(r.left() + 6.0 + 3.0, cy), 3.0, p.ok);
            ui.painter().galley(pos2(r.left() + 6.0 + 6.0 + 5.0, cy - g.size().y / 2.0), g, p.text_muted);
            let tip = servers.iter().map(|(_, s)| tf("Open {} in the browser", &[&s.url])).collect::<Vec<_>>().join("\n");
            if resp.on_hover_text(tip).clicked() {
                crate::platform::open_url(&first.url);
            }
        }
        // Branch, how many files changed, panes (when split): only what fits.
        let mut meta: Vec<(String, egui::FontId, egui::Color32)> = Vec::new();
        if let Some(b) = &branch {
            meta.push((b.clone(), small.clone(), p.text_faint));
            if changed > 0 {
                meta.push((format!("+{changed}"), small.clone(), p.warn));
            }
        }
        if count > 1 {
            meta.push((format!("{} {count}", icons::SQUARE_SPLIT_HORIZONTAL), small.clone(), p.text_faint));
        }
        // Another tab waits: said here, nearest the buttons (the last to give way), without
        // switching the tile to it.
        if let Some((n, s)) = elsewhere {
            let text = format!("{} {}", status::icon(s), tf("+{} in other tabs", &[&n.to_string()]));
            meta.push((text, small.clone(), status::color(s, &p)));
        }
        let mut meta_left = right;
        let mut placed = Vec::new();
        for (i, (text, font, color)) in meta.into_iter().rev().enumerate() {
            let g = painter.layout_no_wrap(text, font, color);
            if meta_left - g.size().x < header.left() + 160.0 {
                break;
            }
            meta_left -= g.size().x;
            placed.push((meta_left, g, color));
            meta_left -= if i == 0 { 8.0 } else { 6.0 };
        }
        for (x, g, color) in placed {
            painter.galley(pos2(x, cy - g.size().y / 2.0), g, color);
        }
        let left_limit = meta_left - 8.0;
        let left = painter.with_clip_rect(Rect::from_min_max(header.min, pos2(left_limit, header.max.y)));
        let mut x = header.left() + 14.0;
        left.circle_filled(pos2(x + 4.0, cy), 4.0, color);
        x += 8.0 + 8.0;
        let g = left.layout_no_wrap(name, style::semibold(13.0), p.text);
        let gw = g.size().x;
        left.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
        x += gw + 8.0;
        if let Some((s, attention, message, label, said, doing)) = &pane_status {
            // Quiet: the terminal icon and what runs there. Else the status as a pill, then what
            // it asks (or what runs, and what the program says it is doing).
            let quiet = matches!(s, Status::Idle | Status::Unknown);
            let c = status::color(*s, &p);
            let status_left = x;
            let mut job;
            if quiet {
                left.text(pos2(x + 6.0, cy), Align2::CENTER_CENTER, icons::TERMINAL, style::icon_font(12.5), p.text_faint);
                x += 17.0;
                job = egui::text::LayoutJob::simple_singleline(label.clone(), small.clone(), p.text_faint);
            } else {
                let pill_text = match s {
                    Status::Failed(c) => tf("Failed · code {}", &[&c.to_string()]),
                    _ => status::text(*s),
                };
                let r = kit::paint_pill(&left, pos2(x, cy), Some(status::icon(*s)), &pill_text, c, style::tint(c, dark));
                x = r.right() + 8.0;
                let (text, col) = match (s, message) {
                    (Status::NeedsInput, Some(m)) => (format!("{label} · {m}"), p.text),
                    _ => (label.clone(), p.text_muted),
                };
                job = egui::text::LayoutJob::simple_singleline(text, small.clone(), col);
                if let Some((_, Some(v))) = progress {
                    job.append(&format!(" · {v}%"), 0.0, egui::TextFormat::simple(small.clone(), p.text_muted));
                }
            }
            if let Some(doing) = doing.as_ref().filter(|_| *s != Status::NeedsInput) {
                job.append(&format!(" · {doing}"), 0.0, egui::TextFormat::simple(small.clone(), p.text_faint));
            }
            job.wrap = egui::text::TextWrapping::truncate_at_width((left_limit - x).max(0.0));
            let sg = left.layout_job(job);
            let status_right = (x + sg.size().x).min(left_limit);
            left.galley(pos2(x, cy - sg.size().y / 2.0), sg, p.text_muted);
            // Where the status came from, on hover (the header's double click still opens).
            let status_rect = Rect::from_min_max(pos2(status_left, header.top()), pos2(status_right, header.bottom()));
            if status_rect.width() > 0.0 {
                let tip = match doing {
                    Some(doing) => format!("{doing}\n{said}"),
                    None => said.clone(),
                };
                ui.interact(status_rect, egui::Id::new(("tile-status", project_id)), Sense::hover())
                    .on_hover_text(tip);
            }
            if *attention {
                painter.rect_filled(
                    Rect::from_min_size(rect.min + vec2(0.0, 10.0), vec2(3.0, rect.height() - 20.0)),
                    CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 },
                    c,
                );
            }
        }
        // Progress a program reports, under the header.
        let mut body_top = header.bottom();
        if let Some((state, value)) = progress {
            let line = Rect::from_min_size(pos2(rect.left() + 1.0, header.bottom()), vec2(rect.width() - 2.0, 2.0));
            let c = if state == 2 { p.err } else if state == 4 { p.warn } else { p.info };
            painter.rect_filled(line, CornerRadius::ZERO, style::soft(c, 0.18));
            let f = value.map_or(0.3, |v| v.min(100) as f32 / 100.0);
            painter.rect_filled(Rect::from_min_size(line.min, vec2(line.width() * f, 2.0)), CornerRadius::ZERO, c);
            body_top += 2.0;
        }
        // An agent that finished, not seen yet: its last words (or that it finished) and Review.
        let mut body_bottom = rect.bottom();
        if attention && status_now == Some(Status::Done) {
            let said = pane_status.as_ref().and_then(|x| x.2.clone());
            let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 32.0), rect.max);
            let fill = style::over(scheme.background, p.ok, 0.07);
            painter.rect_filled(foot, CornerRadius { nw: 0, ne: 0, sw: style::RADIUS_LG, se: style::RADIUS_LG }, fill);
            painter.hline((foot.left() + 1.0)..=(foot.right() - 1.0), foot.top() + 0.5, Stroke::new(1.0, style::soft(p.ok, 0.22)));
            painter.text(pos2(foot.left() + 14.0 + 7.0, foot.center().y), Align2::CENTER_CENTER, icons::CHECK_CIRCLE, style::icon_font(14.0), p.ok);
            let text = said.map(|m| format!("“{m}”")).unwrap_or_else(|| t("Finished its turn. Review what it changed.").to_string());
            let mut job = egui::text::LayoutJob::simple_singleline(text, egui::FontId::proportional(12.0), p.text);
            job.wrap = egui::text::TextWrapping::truncate_at_width((foot.width() - 14.0 - 14.0 - 8.0 - 12.0).max(10.0));
            let g = painter.layout_job(job);
            painter.galley(pos2(foot.left() + 14.0 + 14.0 + 8.0, foot.center().y - g.size().y / 2.0), g, p.text);
            body_bottom = foot.top();
        }

        // Body: inset so the terminal's square corners stay inside the card's round ones.
        let body = Rect::from_min_max(pos2(rect.left() + 4.0, body_top + 2.0), pos2(rect.right() - 4.0, body_bottom - 5.0));
        let Some(pane_id) = pane_id else {
            painter.rect_stroke(rect, radius, border, egui::StrokeKind::Inside);
            return;
        };
        let style = self.term_style(ui);
        let cell = style.cell;
        let input_enabled = self.terminals_take_keys();
        let min_scale = MIN_SCALE;
        // Too low for even a few lines at a readable size: a compact summary.
        if body.height() < cell.y * min_scale * 3.0 + 12.0 {
            // On screen all the same: its shells start as in a full tile (at the size they have,
            // a grid this low being no size for an agent), and its output shows as a preview's.
            for id in layout.as_ref().map_or_else(|| vec![pane_id], |l| l.panes()) {
                if self.panes.get(&id).is_some_and(|x| x.dormant()) {
                    self.actions.push(Action::WakePane(id));
                    ui.ctx().request_repaint();
                }
            }
            let Some(pane) = self.panes.get(&pane_id) else { return };
            let how = crate::pane::Shown::of(ui, pane_id, &pane.view, crate::pane::Shown::Preview);
            self.shown_now.insert(pane_id, how);
            // Compact: the last lines at a readable size; a click opens the project.
            let bg = scheme.background;
            painter.rect_filled(body, CornerRadius::same(style::RADIUS_LG - 2), bg);
            let lines = tc_term::core::tail_text(&pane.shared, ((body.height() - 12.0) / 14.0).max(1.0) as usize);
            let painter = ui.painter_at(body.shrink(6.0));
            for (i, line) in lines.iter().enumerate() {
                painter.text(
                    body.left_top() + vec2(8.0, 6.0 + i as f32 * 14.0),
                    Align2::LEFT_TOP,
                    line,
                    egui::FontId::monospace(10.5),
                    scheme.foreground,
                );
            }
            let r = ui.interact(body, egui::Id::new(("tile-compact", project_id)), Sense::click());
            if r.on_hover_text(t("Open in Focus view")).clicked() {
                self.actions.push(Action::SelectProject(project_id));
            }
            ui.painter_at(rect.expand(2.0)).rect_stroke(rect, radius, border, egui::StrokeKind::Inside);
            return;
        }
        let (rects, dividers) = match &layout {
            Some(l) => l.arrange(body),
            None => (vec![(pane_id, body)], Vec::new()),
        };
        // The terminals take the tile's size (before they are drawn, so a sleeping shell starts
        // at it).
        let sizes = rects.iter().map(|(id, r)| (*id, widget::grid_for(*r, cell))).collect();
        self.resize_panes(&ui.ctx().clone(), sizes);
        #[cfg(debug_assertions)]
        for (id, r) in &rects {
            crate::devtest::record(format!("tile-pane:{id}"), *r);
        }
        for d in &dividers {
            painter.rect_filled(d.rect, CornerRadius::ZERO, p.border.gamma_multiply(0.8));
        }
        for (id, r) in rects {
            self.grid_pane(ui, id, r, &style, min_scale, input_enabled, wheel_free || keyboard_here);
        }
        // The border last, over the terminals' edges.
        ui.painter_at(rect.expand(2.0)).rect_stroke(rect, radius, border, egui::StrokeKind::Inside);
    }

    /// One pane inside a tile, sized to it (`tile`), typing goes to it once clicked; drawn
    /// scaled down only when the font is so large that the terminal is at its smallest grid. A sleeping pane starts its shell now, at the
    /// size its grid already has. `wheel`: the mouse wheel scrolls it (else the Grid).
    #[allow(clippy::too_many_arguments)]
    fn grid_pane(
        &mut self,
        ui: &mut Ui,
        pane_id: PaneId,
        rect: Rect,
        style: &widget::TermStyle,
        min_scale: f32,
        input_enabled: bool,
        wheel: bool,
    ) {
        if self.panes.get(&pane_id).is_some_and(|x| x.dormant()) {
            self.actions.push(Action::WakePane(pane_id));
            ui.ctx().request_repaint();
        }
        let request = self.focus_request == Some(pane_id);
        let Some(pane) = self.panes.get_mut(&pane_id) else { return };
        if request {
            ui.memory_mut(|m| m.request_focus(widget::pane_widget_id(pane_id)));
        }
        self.term_rects.push((pane_id, rect.intersect(ui.clip_rect())));
        let alive = pane.alive();
        let resp = widget::show_readable(ui, rect, &mut pane.view, &pane.shared, style, input_enabled, alive, min_scale, wheel);
        let how = crate::pane::Shown::of(ui, pane_id, &pane.view, crate::pane::Shown::Preview);
        self.shown_now.insert(pane_id, how);
        if !resp.input.is_empty() {
            if resp.input.contains(&b'\r') {
                pane.entered(std::time::Instant::now());
            }
            pane.input(resp.input);
        }
        if let Some(bytes) = resp.paste_without_text {
            pane.paste_without_text(bytes);
        }
        if resp.reconnect {
            self.actions.push(Action::RestartShell(pane_id));
        }
        // ⌘-click on a file: it opens beside this terminal, in its project's Focus view.
        if let Some(word) = resp.link
            && let Some(open) = super::focus::term_link(&word, &pane.cwd)
        {
            self.actions.push(Action::FocusPane(pane_id));
            self.actions.push(open);
        }
        // Clicked: the keyboard goes here, whatever was asked for before.
        if resp.activated && self.focus_request.is_some_and(|f| f != pane_id) {
            self.focus_request = None;
        }
        if resp.focused {
            self.grid_focus = Some(pane_id);
            if request {
                self.focus_request = None;
            }
        } else if self.grid_focus == Some(pane_id) && !ui.memory(|m| m.has_focus(widget::pane_widget_id(pane_id))) {
            self.grid_focus = None;
        }
    }

    /// Status bar note in the Grid: which project the keyboard types into.
    pub fn keyboard_target(&self) -> Option<String> {
        if self.view == View::Focus {
            return None;
        }
        let pane = self.grid_focus.and_then(|id| self.panes.get(&id))?;
        let project = self.project(pane.project)?;
        Some(tf("Typing into {}", &[&format!("{} · {}", project.name, pane.label())]))
    }
}

#[cfg(test)]
mod tests {
    use super::{ACTIVE_AFTER, Activity, activity, grid_shape};
    use std::time::{Duration, Instant};
    use tc_core::agent::AgentKind;
    use tc_core::status::{Foreground, Status, Tracker};

    #[test]
    fn a_project_is_active_when_something_runs_or_waits() {
        let t0 = Instant::now();
        // A shell at its prompt (or one not started yet): a card.
        let shell = Tracker::new(t0);
        assert_eq!(activity(&shell, t0 + Duration::from_secs(60)), Activity::Idle);
        // A program in the foreground: active after a moment (not for a quick `ls`).
        let mut build = Tracker::new(t0);
        build.fg = Foreground::Program { name: "cargo".into(), agent: None };
        assert_eq!(activity(&build, t0 + Duration::from_millis(500)), Activity::Soon(ACTIVE_AFTER - Duration::from_millis(500)));
        assert_eq!(activity(&build, t0 + ACTIVE_AFTER), Activity::Active);
        // An agent, even waiting at its own prompt: active at once.
        let mut agent = Tracker::new(t0);
        agent.fg = Foreground::Program { name: "claude".into(), agent: Some(AgentKind::Claude) };
        agent.agent = Some(AgentKind::Claude);
        assert_eq!(activity(&agent, t0), Activity::Active);
        // Something the user has not seen yet (a finished run) keeps the tile until they look.
        let mut done = Tracker::new(t0);
        done.status = Status::Done;
        done.unseen = true;
        assert_eq!(activity(&done, t0), Activity::Active);
        done.unseen = false;
        assert_eq!(activity(&done, t0), Activity::Idle);
    }

    #[test]
    fn the_grid_is_two_by_two_and_scrolls() {
        // One project fills the view; two sit side by side; then two rows to a screen.
        assert_eq!(grid_shape(1, 0), (1, 1));
        assert_eq!(grid_shape(2, 0), (2, 1));
        assert_eq!(grid_shape(3, 0), (2, 2));
        assert_eq!(grid_shape(4, 0), (2, 2));
        // Seven projects: still 2 × 2 in view, the rest below.
        assert_eq!(grid_shape(7, 0), (2, 2));
        // A saved column count: that many columns and rows to a screen.
        assert_eq!(grid_shape(5, 3), (3, 2));
        assert_eq!(grid_shape(12, 3), (3, 3));
        assert_eq!(grid_shape(0, 0), (1, 1));
    }
}
