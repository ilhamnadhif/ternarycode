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
use crate::app::{App, GridPick};
use crate::i18n::{t, tf};
use crate::project::{Project, Tab};

const TILE_HEADER: f32 = 34.0;
/// The resume banner in a tile (lower than Focus's).
const TILE_BANNER_H: f32 = 36.0;
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
    /// else one running an agent, else the tab's focused pane. Other tabs speak through the
    /// tile's strip: the header never speaks of a pane not in view.
    pub fn tile_pane(&self, project: ProjectId, tab: usize) -> Option<PaneId> {
        let tab = self.project(project)?.tabs.get(tab)?;
        let panes = tab.layout.panes();
        // A browser focused: the tab's terminal tells (the page is not in the tile).
        let terminal = || Some(tab.focused).filter(|id| self.panes.contains_key(id)).or_else(|| panes.iter().copied().find(|id| self.panes.contains_key(id)));
        self.telling_pane(&panes).or_else(terminal).or(Some(tab.focused))
    }

    /// How much a tab asks to be seen in a Grid tile, lower first: something waits (by its
    /// priority, the oldest first), an agent at work, a program running (or an agent at its
    /// prompt, or one that ran there before a restart and can be resumed), an idle shell.
    fn tab_rank(&self, tab: &Tab) -> (u8, Option<Instant>) {
        let panes: Vec<&crate::pane::Pane> = tab.layout.panes().iter().filter_map(|id| self.panes.get(id)).collect();
        if let Some(x) = panes.iter().filter(|x| x.tracker.wants_attention()).min_by_key(|x| (x.tracker.status.priority(), x.tracker.since)) {
            return (x.tracker.status.priority().unwrap_or(3), Some(x.tracker.since));
        }
        if panes.iter().any(|x| x.tracker.agent.is_some() && matches!(x.tracker.status, Status::Running | Status::Unknown)) {
            return (4, None);
        }
        if panes.iter().any(|x| x.tracker.agent.is_some() || x.resume.is_some() || matches!(x.tracker.fg, Foreground::Program { .. })) {
            return (5, None);
        }
        (6, None)
    }

    /// The tab that asks most to be seen; on a tie the one open in Focus, then the first.
    fn best_tab(&self, p: &Project) -> usize {
        (0..p.tabs.len())
            .min_by(|a, b| {
                self.tab_rank(&p.tabs[*a])
                    .cmp(&self.tab_rank(&p.tabs[*b]))
                    .then((*a != p.active_tab).cmp(&(*b != p.active_tab)))
                    .then(a.cmp(b))
            })
            .unwrap_or(0)
    }

    /// The tab a project's tile shows, remembered for the next frame. The tab the keyboard is
    /// in (or is going to) never changes by itself; a tab picked from the strip is held until
    /// something new waits in another tab; otherwise the tab that asks most to be seen, changing
    /// only when another asks strictly more (each change resizes its terminals to the tile, and
    /// an agent there redraws).
    pub fn grid_tab(&mut self, project: ProjectId) -> usize {
        let Some(p) = self.project(project) else { return 0 };
        if p.tabs.len() <= 1 {
            self.grid_pick.remove(&project);
            return 0;
        }
        let pick = self.grid_pick.get(&project).copied().and_then(|g| p.tab_of(g.pane).map(|i| (i, g)));
        let keyboard = self.grid_focus.or(self.focus_request).and_then(|f| p.tab_of(f));
        let (shown, manual, asks) = match (keyboard, pick) {
            // The keyboard's tab (held as picked when it was).
            (Some(i), Some((j, g))) if i == j => (i, g.manual, g.asks),
            (Some(i), _) => (i, false, 0),
            (None, Some((i, g))) if g.manual => {
                let fresh = p
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| *k != i)
                    .flat_map(|(_, t)| t.layout.panes())
                    .filter_map(|id| self.panes.get(&id))
                    .any(|x| x.tracker.wants_attention() && x.tracker.ask > g.asks);
                if fresh { (self.best_tab(p), false, 0) } else { (i, true, g.asks) }
            }
            (None, Some((i, _))) => {
                let best = self.best_tab(p);
                if self.tab_rank(&p.tabs[best]).0 < self.tab_rank(&p.tabs[i]).0 { (best, false, 0) } else { (i, false, 0) }
            }
            (None, None) => (self.best_tab(p), false, 0),
        };
        let pane = p.tabs[shown].focused;
        self.grid_pick.insert(project, GridPick { pane, manual, asks });
        shown
    }

    /// The same over all of a project's tabs: the pane its sidebar row speaks of.
    pub fn project_pane(&self, project: ProjectId) -> Option<PaneId> {
        let p = self.project(project)?;
        self.telling_pane(&p.panes()).or_else(|| p.current_pane())
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
        // Projects only (the "Add Project…" tile comes last).
        let tiles_of_projects = ids.len();
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
                            let shown = self.grid_tab(id);
                            drawn.extend(self.project(id).and_then(|p| p.tabs.get(shown)).map(|t| t.layout.panes()).unwrap_or_default());
                            self.tile(ui, id, rect, !scrolls, shown);
                        }
                    }
                    // More below: how many, at the bottom of the view; a click brings the next row.
                    let row_rect = |r: usize| {
                        Rect::from_min_size(content.min + vec2(0.0, r as f32 * (h + GAP)), vec2(width, h))
                    };
                    let next_row = (0..rows).find(|r| row_rect(*r).top() >= visible.bottom() - 1.0);
                    if let Some(next) = next_row {
                        let below = tiles_of_projects - (next * cols).min(tiles_of_projects);
                        if below > 0 {
                            self.more_below(ui, visible, below, row_rect(next));
                        }
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

    /// One project: header (name, tabs, status, branch, open in Focus) over its tab's panes (one
    /// whose agent can be resumed with Focus's banner on top). `wheel_free`: the mouse wheel may scroll the terminals (the Grid does not scroll);
    /// else only the one taking the keyboard gets it.
    fn tile(&mut self, ui: &mut Ui, project_id: ProjectId, rect: Rect, wheel_free: bool, shown: usize) {
        let p = style::pal(ui);
        let Some(project) = self.project(project_id) else {
            return;
        };
        let pane_id = self.tile_pane(project_id, shown);
        // Where Focus opens from this tile: the tab it shows.
        let open_pane = project.tabs.get(shown).map(|t| t.focused);
        // Several tabs: a strip of them (each one's label, what it holds, whether something there
        // waits for the user), and whether the tab shown was picked from it.
        let strip: Vec<StripTab> = if project.tabs.len() > 1 {
            project
                .tabs
                .iter()
                .map(|tab| {
                    let panes = tab.layout.panes();
                    let label = tab.title.clone().unwrap_or_else(|| self.leaf_label(tab.focused));
                    let teller = self.telling_pane(&panes).or(Some(tab.focused)).and_then(|id| self.panes.get(&id));
                    let status = teller.map(|x| x.tracker.status).filter(|s| !matches!(s, Status::Idle | Status::Unknown));
                    let waits = panes.iter().filter_map(|id| self.panes.get(id)).any(|x| x.tracker.wants_attention());
                    let resume = panes.iter().filter_map(|id| self.panes.get(id)).find_map(|x| x.resume.as_ref().map(|(a, _)| a.label().to_string()));
                    let web = self.browsers.get(&tab.focused).map(|b| b.asleep && !b.url.is_empty());
                    StripTab { label, status, waits, resume, web }
                })
                .collect()
        } else {
            Vec::new()
        };
        let held = !strip.is_empty() && self.grid_pick.get(&project_id).is_some_and(|g| g.manual);
        let name = project.name.clone();
        let color = project.color32();
        let branch = project.git.as_ref().map(|g| g.branch.clone());
        // A worktree: "ternary-engine ›" before its name, its branch with commits ahead.
        let parent_name = project.parent.and_then(|x| self.project(x)).map(|x| x.name.clone());
        let ahead = project.worktree.as_ref().map(|_| project.git.as_ref().map_or(0, |g| g.ahead));
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
        // The tab shown, with all its panes (a split project shows its split).
        let layout = project.tabs.get(shown).map(|t| t.layout.clone());
        let count = layout.as_ref().map_or(0, |l| l.panes().len());
        // A browser an agent uses, in another tab of the project: a picture of it in a corner of
        // the tile (a chip in its header when the tile is small).
        let mut in_tile: Vec<PaneId> = layout.as_ref().map_or_else(|| pane_id.into_iter().collect(), |l| l.panes());
        // Its Browser panel, as Focus shows it, when the tile has room: its page zoomed out to
        // keep its layout (the body is the tile less its header and a 4-pt inset).
        let tile_page = self.tile_panel(project_id, rect.width() - 8.0, rect.height() - TILE_HEADER - 8.0);
        in_tile.extend(tile_page.map(|(page, _)| page));
        let panel_fits = crate::ui::browser_panel::tile_fits_panel(rect.width() - 8.0, rect.height() - TILE_HEADER - 8.0);
        let web = self.tile_web(project_id, &in_tile);
        let web_small = rect.width() < 380.0 || rect.height() < 260.0;
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
        // To Focus: the tab this tile shows.
        let open_action = open_pane.map_or(Action::SelectProject(project_id), Action::FocusPane);
        if hr.double_clicked() {
            self.actions.push(open_action.clone());
        }
        let cy = header.center().y;
        let small = egui::FontId::proportional(style::SMALL);
        // Right side first (open button, review, panes, branch); the left side is cut
        // before it.
        let open_rect = Rect::from_center_size(pos2(header.right() - 6.0 - 11.0, cy), vec2(22.0, 22.0));
        let open = ui
            .scope_builder(egui::UiBuilder::new().max_rect(open_rect), |ui| {
                kit::icon_button_ex(ui, icons::ARROW_SQUARE_OUT, &kit::tip(t("Open in Focus view"), Action::OpenGridTile), false, 22.0)
            })
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("tile-open:{name}"), open.rect);
        if open.clicked() {
            self.actions.push(open_action.clone());
        }
        let mut right = open_rect.left() - 8.0;
        let button = |ui: &mut Ui, icon: Option<&str>, text: &str, right: &mut f32| {
            let w = kit::button_width(ui, icon, text, None, true);
            let r = Rect::from_center_size(pos2(*right - w / 2.0, cy), vec2(w, 24.0));
            *right -= w + 8.0;
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::button_ex(ui, kit::Kind::Secondary, icon, text, true)).inner
        };
        if let Some(id) = review {
            let b = button(ui, Some(icons::GIT_DIFF), t("Review"), &mut right);
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("tile-review:{id}"), b.rect);
            if b.on_hover_text(kit::tip(t("Review what the agent changed"), Action::Review)).clicked() {
                self.actions.push(Action::FocusPane(id));
                self.actions.push(Action::ReviewAgent);
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
        // The browser an agent uses, on a tile too small for its picture: shows it in the tile.
        if let Some(w) = web.as_ref().filter(|_| web_small) {
            let g = ui.painter().layout_no_wrap(t("browser").to_string(), small.clone(), p.info);
            let wd = 6.0 + 13.0 + 4.0 + g.size().x + 6.0;
            let r = Rect::from_center_size(pos2(right - wd / 2.0, cy), vec2(wd, 22.0));
            right -= wd + 6.0;
            let resp = ui.interact(r, egui::Id::new(("tile-web", project_id)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("tile-web-chip:{name}"), r);
            let fill = style::tint(p.info, dark);
            ui.painter().rect_filled(r, CornerRadius::same(style::RADIUS), if resp.hovered() { fill.gamma_multiply(1.6) } else { fill });
            ui.painter().text(pos2(r.left() + 6.0 + 6.5, cy), Align2::CENTER_CENTER, icons::GLOBE, style::icon_font(12.0), p.info);
            ui.painter().galley(pos2(r.left() + 6.0 + 13.0 + 4.0, cy - g.size().y / 2.0), g, p.info);
            let tip = format!("{} · {}\n{}", w.address, w.who, t("Click: show the browser in this tile · ⌥-click: in Focus"));
            if resp.on_hover_text(tip).clicked() {
                self.show_tile_web(project_id, w, ui.input(|i| i.modifiers.alt) || !panel_fits);
            }
        }
        // Its page shown smaller than in Focus: by how much (hover says why).
        if let Some((_, zoom)) = tile_page.filter(|(_, z)| *z < 0.995) {
            let g = ui.painter().layout_no_wrap(format!("{} {}%", icons::MAGNIFYING_GLASS_MINUS, (zoom * 100.0).round() as i32), small.clone(), p.text_faint);
            let wd = g.size().x + 10.0;
            let r = Rect::from_center_size(pos2(right - wd / 2.0, cy), vec2(wd, 22.0));
            right -= wd + 6.0;
            let resp = ui.interact(r, egui::Id::new(("tile-zoom", project_id)), Sense::hover());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("tile-zoom:{name}"), r);
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(style::RADIUS), p.hover);
            }
            ui.painter().galley(pos2(r.left() + 5.0, cy - g.size().y / 2.0), g, p.text_faint);
            resp.on_hover_text(tf("Shown at {}%, laid out as in Focus. Open in Focus to see it at 100%.", &[&((zoom * 100.0).round() as i32).to_string()]));
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
                self.actions.push(Action::OpenWeb(first.url.clone()));
            }
        }
        // Branch, how many files changed, panes (when split): only what fits.
        let mut meta: Vec<(String, egui::FontId, egui::Color32)> = Vec::new();
        if let Some(b) = &branch {
            match ahead {
                Some(n) => {
                    meta.push((format!("{} {b}", icons::GIT_FORK), small.clone(), p.text_faint));
                    meta.push((format!("↑{n}"), small.clone(), p.text_faint));
                }
                None => meta.push((b.clone(), small.clone(), p.text_faint)),
            }
            if changed > 0 {
                meta.push((format!("+{changed}"), small.clone(), p.warn));
            }
        }
        if count > 1 {
            meta.push((format!("{} {count}", icons::SQUARE_SPLIT_HORIZONTAL), small.clone(), p.text_faint));
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
        if let Some(parent) = &parent_name {
            let g = left.layout_no_wrap(format!("{parent} ›"), style::medium(12.0), p.text_faint);
            let gw = g.size().x;
            left.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
            x += gw + 5.0;
        }
        let g = left.layout_no_wrap(name.clone(), style::semibold(13.0), p.text);
        let gw = g.size().x;
        left.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
        x += gw + 8.0;
        // The strip of tabs, after the name: the chips say what each tab holds, so the status
        // that follows needs no label.
        let with_strip = !strip.is_empty();
        if with_strip {
            x = self.tab_strip(ui, &left, project_id, &name, &strip, shown, held, x, cy, left_limit, scheme.background);
        }
        if let Some((s, attention, message, label, said, doing)) = &pane_status {
            // Quiet: the terminal icon and what runs there (with a strip: nothing more). Else the
            // status as a pill, then what it asks (or what runs, and what the program says it is
            // doing).
            let quiet = matches!(s, Status::Idle | Status::Unknown);
            let c = status::color(*s, &p);
            let status_left = x;
            let mut job;
            if quiet {
                if with_strip {
                    job = egui::text::LayoutJob::default();
                } else {
                    left.text(pos2(x + 6.0, cy), Align2::CENTER_CENTER, icons::TERMINAL, style::icon_font(12.5), p.text_faint);
                    x += 17.0;
                    job = egui::text::LayoutJob::simple_singleline(label.clone(), small.clone(), p.text_faint);
                }
            } else {
                let pill_text = match s {
                    Status::Failed(c) => tf("Failed · code {}", &[&c.to_string()]),
                    _ => status::text(*s),
                };
                // The pill whole, or (no room for it) its icon.
                if x + kit::pill_width(&left, Some(status::icon(*s)), &pill_text) <= left_limit {
                    let r = kit::paint_pill(&left, pos2(x, cy), Some(status::icon(*s)), &pill_text, c, style::tint(c, dark));
                    x = r.right() + 8.0;
                } else {
                    left.text(pos2(x + 6.0, cy), Align2::CENTER_CENTER, status::icon(*s), style::icon_font(12.5), c);
                    x += 17.0;
                }
                let (text, col) = match (s, message, with_strip) {
                    (Status::NeedsInput, Some(m), true) => (m.clone(), p.text),
                    (Status::NeedsInput, Some(m), false) => (format!("{label} · {m}"), p.text),
                    (_, _, true) => (String::new(), p.text_muted),
                    _ => (label.clone(), p.text_muted),
                };
                job = egui::text::LayoutJob::simple_singleline(text, small.clone(), col);
                if let Some((_, Some(v))) = progress {
                    let sep = if job.text.is_empty() { "" } else { " · " };
                    job.append(&format!("{sep}{v}%"), 0.0, egui::TextFormat::simple(small.clone(), p.text_muted));
                }
            }
            if !(quiet && with_strip)
                && let Some(doing) = doing.as_ref().filter(|_| *s != Status::NeedsInput)
            {
                let sep = if job.text.is_empty() { "" } else { " · " };
                job.append(&format!("{sep}{doing}"), 0.0, egui::TextFormat::simple(small.clone(), p.text_faint));
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
                self.actions.push(open_action);
            }
            ui.painter_at(rect.expand(2.0)).rect_stroke(rect, radius, border, egui::StrokeKind::Inside);
            return;
        }
        // The panel at the right as in Focus (its share of the width), the terminals beside it as
        // Focus lays them (stacked when Focus stacks them).
        let (panes_body, panel_rect) = match tile_page {
            Some(_) => {
                let share = self.panel_of(project_id).map_or(crate::project::PANEL_SHARE, crate::project::Panel::share);
                let w = (body.width() * share).round();
                (Rect::from_min_max(body.min, pos2(body.right() - w - 1.0, body.bottom())), Some(Rect::from_min_max(pos2(body.right() - w, body.top()), body.max)))
            }
            None => (body, None),
        };
        let stacked = tile_page.is_some() && self.panel_of(project_id).is_some_and(|x| x.stacked);
        let (rects, dividers) = match &layout {
            Some(l) if stacked => l.stacked().arrange(panes_body),
            Some(l) => l.arrange(panes_body),
            None => (vec![(pane_id, panes_body)], Vec::new()),
        };
        // A pane whose agent ran there before a restart: Focus's banner at its top (it is why
        // the project counts as active), the terminal under it.
        let mut banners = Vec::new();
        let rects: Vec<(PaneId, Rect)> = rects
            .into_iter()
            .map(|(id, r)| {
                if self.panes.get(&id).is_some_and(|x| x.resume.is_some()) && r.height() > 2.0 * TILE_BANNER_H {
                    banners.push((id, Rect::from_min_size(r.min, vec2(r.width(), TILE_BANNER_H))));
                    (id, Rect::from_min_max(pos2(r.left(), r.top() + TILE_BANNER_H), r.max))
                } else {
                    (id, r)
                }
            })
            .collect();
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
        for (id, r) in banners {
            self.resume_banner(ui, id, r, true);
        }
        if let (Some((page, zoom)), Some(r)) = (tile_page, panel_rect) {
            ui.painter().vline(r.left() - 0.5, r.y_range(), Stroke::new(1.0, p.border.gamma_multiply(0.8)));
            self.tile_browser(ui, page, r, zoom);
        }
        if let Some(w) = web.filter(|_| !web_small) {
            self.tile_pip(ui, project_id, body, &w);
        }
        // The border last, over the terminals' edges.
        ui.painter_at(rect.expand(2.0)).rect_stroke(rect, radius, border, egui::StrokeKind::Inside);
    }

    /// A project's browser an agent uses (or used in the last 10 s), out of the tile's view
    /// (`in_tile`): what the tile's corner shows of it.
    fn tile_web(&self, project: ProjectId, in_tile: &[PaneId]) -> Option<TileWeb> {
        self.browsers.values().filter(|b| b.project == project && !in_tile.contains(&b.id)).find_map(|b| {
            let d = b.driver.as_ref().filter(|d| !d.paused)?;
            let working = self.browser_jobs.iter().any(|j| j.browser == b.id);
            if !working && d.last.elapsed() > std::time::Duration::from_secs(10) {
                return None;
            }
            let (host, path) = crate::browser::split_url(&b.url);
            Some(TileWeb { id: b.id, doing: d.doing.clone(), who: d.who.clone(), address: format!("{host}{path}") })
        })
    }

    /// Shows a tile's web page: in the tile (its project's panel, in front), or in Focus (⌥, or
    /// a tile with no room for the panel).
    fn show_tile_web(&mut self, project: ProjectId, web: &TileWeb, focus: bool) {
        if focus {
            self.actions.push(Action::FocusPane(web.id));
        } else {
            self.show_page(project, web.id);
        }
    }

    /// The picture of an agent's page in the corner of its tile: the page (two pictures a second
    /// at most), what the agent does, and a click to show it in the tile.
    fn tile_pip(&mut self, ui: &mut Ui, project: ProjectId, body: Rect, web: &TileWeb) {
        let p = style::pal(ui);
        let size = if body.width() >= 520.0 && body.height() >= 300.0 { vec2(216.0, 146.0) } else { vec2(160.0, 108.0) };
        if body.width() < size.x + 40.0 || body.height() < size.y + 30.0 {
            return;
        }
        let r = Rect::from_min_size(body.max - size - vec2(10.0, 10.0), size);
        let ctx = ui.ctx().clone();
        #[cfg(target_os = "macos")]
        if let Some(b) = self.browsers.get_mut(&web.id)
            && b.still.as_ref().is_none_or(|(_, at)| at.elapsed() > std::time::Duration::from_millis(500))
        {
            b.ask_picture(&ctx, Some(f64::from(size.x)));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        let resp = ui.interact(r, egui::Id::new(("tile-pip", project)), Sense::click());
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("tile-pip:{project}"), r);
        let painter = ui.painter_at(r.expand(2.0));
        painter.rect_filled(r, CornerRadius::same(style::RADIUS_LG), egui::Color32::WHITE);
        if let Some((tex, _)) = self.browsers.get(&web.id).and_then(|b| b.still.as_ref()) {
            let s = tex.size_vec2();
            let h = r.width() * s.y / s.x.max(1.0);
            ui.painter_at(r.shrink(1.0)).image(tex.id(), Rect::from_min_size(r.min, vec2(r.width(), h)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        let cap = Rect::from_min_max(pos2(r.left(), r.bottom() - 24.0), r.max);
        painter.rect_filled(cap, CornerRadius { nw: 0, ne: 0, sw: style::RADIUS_LG, se: style::RADIUS_LG }, style::soft(p.surface, 0.94));
        painter.hline(cap.x_range(), cap.top() + 0.5, Stroke::new(1.0, p.border));
        painter.text(pos2(cap.left() + 13.0, cap.center().y), Align2::CENTER_CENTER, icons::CURSOR_CLICK, style::icon_font(12.5), p.info);
        let mut job = egui::text::LayoutJob::simple_singleline(web.doing.clone(), egui::FontId::proportional(11.0), p.text_muted);
        job.wrap = egui::text::TextWrapping::truncate_at_width((cap.width() - 22.0 - 24.0).max(10.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(cap.left() + 22.0, cap.center().y - g.size().y / 2.0), g, p.text_muted);
        painter.text(pos2(cap.right() - 12.0, cap.center().y), Align2::CENTER_CENTER, icons::ARROWS_OUT_SIMPLE, style::icon_font(12.0), if resp.hovered() { p.text } else { p.text_faint });
        let ring = if resp.hovered() { p.info } else { style::over(p.border, p.info, 0.6) };
        painter.rect_stroke(r, CornerRadius::same(style::RADIUS_LG), Stroke::new(if resp.hovered() { 1.5 } else { 1.0 }, ring), egui::StrokeKind::Outside);
        let tip = format!("{} · {}\n{}", web.address, web.who, t("Click: show the browser in this tile · ⌥-click: in Focus"));
        if resp.on_hover_text(tip).clicked() {
            let alt = ui.input(|i| i.modifiers.alt);
            let fits = crate::ui::browser_panel::tile_fits_panel(body.width(), body.height());
            self.show_tile_web(project, web, alt || !fits);
        }
    }

    /// A page in a tile (`zoom`: drawn smaller, keeping its layout): live when the whole of it
    /// is in view, else its picture.
    fn tile_browser(&mut self, ui: &mut Ui, id: PaneId, rect: Rect, zoom: f32) {
        let p = style::pal(ui);
        if let Some(b) = self.browsers.get_mut(&id) {
            b.zoom_want = zoom;
        }
        let Some(b) = self.browsers.get(&id) else { return };
        let driven = b.driven();
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.canvas);
        let mut page = rect;
        if driven {
            ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(2.0, style::soft(p.info, 0.65)), egui::StrokeKind::Inside);
            page = rect.shrink(2.0);
        }
        let whole = ui.clip_rect().contains_rect(page) && page.width() >= 40.0 && page.height() >= 40.0;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("tile-browser:{id}"), page);
        if whole && !b.url.is_empty() {
            self.browser_pages.insert(id, page);
            return;
        }
        if let Some((tex, _)) = &b.still {
            let s = tex.size_vec2();
            let h = page.width() * s.y / s.x.max(1.0);
            ui.painter().with_clip_rect(page.intersect(ui.clip_rect())).image(tex.id(), Rect::from_min_size(page.min, vec2(page.width(), h)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
            return;
        }
        let (host, _) = crate::browser::split_url(&b.url);
        let c = page.center();
        ui.painter().text(c - vec2(0.0, 16.0), Align2::CENTER_CENTER, icons::GLOBE, style::icon_font(22.0), p.text_faint);
        ui.painter().text(c + vec2(0.0, 8.0), Align2::CENTER_CENTER, b.label(), egui::FontId::proportional(12.5), p.text_muted);
        ui.painter().text(c + vec2(0.0, 26.0), Align2::CENTER_CENTER, host, egui::FontId::proportional(style::SMALL), p.text_faint);
        #[cfg(target_os = "macos")]
        {
            let ctx = ui.ctx().clone();
            if let Some(b) = self.browsers.get_mut(&id) {
                b.ask_picture(&ctx, Some(f64::from(page.width())));
            }
        }
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
        // A browser pane: its page, live when the whole pane is in view (a web view can't be
        // cut to the Grid's scrolling), else its picture, else what page it holds.
        if self.browsers.contains_key(&pane_id) {
            self.tile_browser(ui, pane_id, rect, 1.0);
            return;
        }
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
        } else if self.grid_focus == Some(pane_id)
            // The palette or a dialog has the keyboard for a while: their commands act on this
            // tile's pane, and the keyboard comes back here after them.
            && self.palette.is_none()
            && self.dialog.is_none()
            && !ui.memory(|m| m.has_focus(widget::pane_widget_id(pane_id)))
        {
            self.grid_focus = None;
        }
    }

    /// Status bar note in the Grid: which project the keyboard types into.
    pub fn keyboard_target(&self) -> Option<String> {
        if self.view == View::Focus {
            return None;
        }
        let pane = self.grid_focus.and_then(|id| self.panes.get(&id))?;
        self.project(pane.project)?;
        Some(tf("Typing into {}", &[&format!("{} · {}", self.project_label(pane.project), pane.label())]))
    }
}

/// A tab of a tile's strip: its label, what it holds (none: a quiet shell), whether something
/// there waits for the user, and the agent that ran there before a restart (to resume).
struct StripTab {
    label: String,
    status: Option<Status>,
    waits: bool,
    resume: Option<String>,
    /// A browser tab: whether its page is asleep.
    web: Option<bool>,
}

/// How a tile's strip of tabs is drawn, from the room there is: each tab with its icon and
/// label; icons only (labels in tooltips); or one chip with their number and a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StripMode {
    Labels,
    Icons,
    Menu,
}

const CHIP_H: f32 = 22.0;
const CHIP_GAP: f32 = 2.0;

impl App {
    /// The strip of a tile's tabs (several tabs only), from `x` on the header's line `cy`: a chip
    /// per tab, the one shown filled, one where something waits in the warning colour with a
    /// dot; a click shows that tab (held). Narrow, the chips lose their labels, then become one
    /// chip with a menu. Returns where what follows starts.
    #[allow(clippy::too_many_arguments)]
    fn tab_strip(
        &mut self,
        ui: &mut Ui,
        painter: &egui::Painter,
        project: ProjectId,
        name: &str,
        tabs: &[StripTab],
        shown: usize,
        held: bool,
        x: f32,
        cy: f32,
        limit: f32,
        bg: egui::Color32,
    ) -> f32 {
        let p = style::pal(ui);
        let font = style::medium(11.5);
        let labels: Vec<std::sync::Arc<egui::Galley>> = tabs
            .iter()
            .map(|t| {
                let mut job = egui::text::LayoutJob::simple_singleline(t.label.clone(), font.clone(), p.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width(140.0);
                painter.layout_job(job)
            })
            .collect();
        // The pin: 4 px after the label, 10.5 px wide.
        let pin_w = 14.5;
        let label_w = |i: usize| 6.0 + 12.0 + 5.0 + labels[i].size().x + if held && i == shown { pin_w } else { 0.0 } + 7.0;
        let all_labels: f32 = (0..tabs.len()).map(label_w).sum::<f32>() + CHIP_GAP * (tabs.len() as f32 - 1.0);
        let all_icons = 24.0 * tabs.len() as f32 + CHIP_GAP * (tabs.len() as f32 - 1.0);
        let room = limit - x;
        let mode = if all_labels <= room - 110.0 {
            StripMode::Labels
        } else if all_icons <= room - 70.0 {
            StripMode::Icons
        } else {
            StripMode::Menu
        };
        let icon_of = |t: &StripTab, on: bool| match (t.status, &t.resume) {
            (Some(s), _) => (status::icon(s), status::color(s, &p)),
            (None, Some(_)) => (icons::CLOCK_COUNTER_CLOCKWISE, p.accent_text),
            (None, None) => (
                match t.web {
                    Some(true) => icons::MOON,
                    Some(false) => icons::GLOBE,
                    None => icons::TERMINAL,
                },
                if on { p.accent_text } else { p.text_faint },
            ),
        };
        if mode == StripMode::Menu {
            // One chip: the number of tabs (warning when one not shown waits), a menu of them.
            let count = painter.layout_no_wrap(tabs.len().to_string(), style::medium(11.0), p.text_muted);
            let w = 6.0 + 13.0 + 3.0 + count.size().x + 3.0 + 10.0 + 6.0;
            let rect = Rect::from_min_size(pos2(x, cy - CHIP_H / 2.0), vec2(w, CHIP_H));
            let resp = ui.interact(rect, egui::Id::new(("tile-tabs", project)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("tile-tabs:{name}"), rect);
            let waits = tabs.iter().enumerate().any(|(i, t)| i != shown && t.waits);
            let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
            let fg = if waits { p.warn } else { p.text_muted };
            let fill = if waits {
                style::tint(p.warn, ui.visuals().dark_mode)
            } else if resp.hovered() || open {
                p.hover
            } else {
                egui::Color32::TRANSPARENT
            };
            painter.rect_filled(rect, CornerRadius::same(5), fill);
            painter.text(pos2(rect.left() + 6.0 + 6.5, cy), Align2::CENTER_CENTER, icons::BROWSERS, style::icon_font(12.0), fg);
            let cx = rect.left() + 6.0 + 13.0 + 3.0;
            painter.galley(pos2(cx, cy - count.size().y / 2.0), count.clone(), fg);
            painter.text(pos2(cx + count.size().x + 3.0 + 5.0, cy), Align2::CENTER_CENTER, icons::CARET_DOWN, style::icon_font(10.0), fg);
            if waits {
                chip_dot(painter, rect, p.warn, bg);
            }
            let resp = resp.on_hover_text(crate::i18n::count(tabs.len(), t("1 tab"), t("{} tabs")));
            egui::Popup::menu(&resp).align(egui::RectAlign::BOTTOM_START).show(|ui| {
                ui.set_min_width(230.0);
                ui.set_max_width(260.0);
                kit::menu_body(ui);
                kit::menu_header(ui, &tf("Tabs in {}", &[name]));
                for (i, tab) in tabs.iter().enumerate() {
                    let quiet = if tab.resume.is_some() { t("Can resume") } else { t("Idle") };
                    let state = tab.status.map_or_else(|| quiet.to_string(), status::text);
                    let icon = (i == shown).then_some(icons::CHECK);
                    if kit::menu_item(ui, icon, &tab.label, Some(&state), false).clicked() {
                        self.actions.push(Action::GridShowTab(project, i));
                        ui.close();
                    }
                }
            });
            return rect.right() + 8.0;
        }
        let mut cx = x;
        for (i, tab) in tabs.iter().enumerate() {
            let on = i == shown;
            let w = if mode == StripMode::Labels { label_w(i) } else { 24.0 };
            let rect = Rect::from_min_size(pos2(cx, cy - CHIP_H / 2.0), vec2(w, CHIP_H));
            cx = rect.right() + CHIP_GAP;
            let resp = ui.interact(rect, egui::Id::new(("tile-tab", project, i)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("tile-tab:{name}:{i}"), rect);
            let waits = !on && tab.waits;
            if on {
                painter.rect(rect, CornerRadius::same(5), p.surface, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
            } else if waits {
                painter.rect_filled(rect, CornerRadius::same(5), style::tint(p.warn, ui.visuals().dark_mode));
            } else if resp.hovered() {
                painter.rect_filled(rect, CornerRadius::same(5), p.hover);
            }
            let (icon, mut ic) = icon_of(tab, on);
            if waits {
                ic = p.warn;
            }
            let icon_x = if mode == StripMode::Labels { rect.left() + 6.0 + 6.0 } else { rect.center().x };
            painter.text(pos2(icon_x, cy), Align2::CENTER_CENTER, icon, style::icon_font(12.0), ic);
            if mode == StripMode::Labels {
                let tc = if on {
                    p.text
                } else if waits {
                    p.warn
                } else {
                    p.text_muted
                };
                let g = labels[i].clone();
                let lx = rect.left() + 6.0 + 12.0 + 5.0;
                painter.galley(pos2(lx, cy - g.size().y / 2.0), g.clone(), tc);
                if held && on {
                    painter.text(pos2(lx + g.size().x + 4.0 + 5.25, cy), Align2::CENTER_CENTER, icons::PUSH_PIN_SIMPLE, style::icon_font(10.5), p.text_faint);
                }
            }
            if waits {
                chip_dot(painter, rect, p.warn, bg);
            }
            // A quiet tab with an agent to resume says so.
            let why = tab.resume.as_ref().filter(|_| tab.status.is_none()).map(|a| tf("{} ran here before the restart.", &[a]));
            let tip = match (mode, held && on, why) {
                (_, true, _) => format!("{}\n{}", tab.label, t("Picked here: kept until something new waits in another tab")),
                (_, false, Some(why)) => format!("{} · {why}", tab.label),
                (StripMode::Icons, false, None) => tab.label.clone(),
                _ => String::new(),
            };
            let resp = if tip.is_empty() { resp } else { resp.on_hover_text(tip) };
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                self.actions.push(Action::GridShowTab(project, i));
            }
        }
        cx - CHIP_GAP + 8.0
    }
}

/// The dot of a chip whose tab waits for the user: at its top right corner, ringed by the tile.
fn chip_dot(painter: &egui::Painter, rect: Rect, color: egui::Color32, ring: egui::Color32) {
    let c = pos2(rect.right() - 1.0, rect.top() + 1.0);
    painter.circle_filled(c, 4.5, ring);
    painter.circle_filled(c, 3.0, color);
}

/// An agent's page a tile shows in its corner (`App::tile_web`).
struct TileWeb {
    id: PaneId,
    doing: String,
    who: String,
    address: String,
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
