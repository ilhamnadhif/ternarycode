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

const TILE_HEADER: f32 = 40.0;
/// The foot of a tile that asks for something (Needs input, Done, Failed, Resume): a notice,
/// 8 from the tile's sides and bottom.
const TILE_FOOT_H: f32 = kit::NOTICE_H;
const FOOT_PAD: f32 = 8.0;
/// A tile's terminal text against the Focus view's: the data font (11 at the default 13).
pub const TILE_FONT: f32 = 11.0 / 13.0;
const GAP: f32 = style::GAP;
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
        // The tiles are sheets on the window's base, as the Focus view's panes.
        let area = super::focus::sheets_area(ui.max_rect(), self.settings.sidebar_hidden);
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
        // A quiet slot: a dashed outline, the folder icon, "Add project", the drop hint, its keys.
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            painter.rect_filled(rect, CornerRadius::same(style::RADIUS_LG), p.hover);
        }
        painter.add(egui::Shape::dashed_line(&rounded_outline(rect.shrink(0.5), style::RADIUS_LG as f32), Stroke::new(1.0, p.border_strong), 4.0, 4.0));
        let keys = kit::keys(&Action::AddProject).unwrap_or("");
        let cx = rect.center().x;
        // 20 + 6 + 18 + 6 + 16 (+ 6 + 18): centred as one block.
        let block = 20.0 + 6.0 + 18.0 + 6.0 + 16.0 + if keys.is_empty() { 0.0 } else { 6.0 + kit::KBD_H };
        let mut y = rect.center().y - block / 2.0;
        painter.text(pos2(cx, y + 10.0), Align2::CENTER_CENTER, icons::FOLDER_SIMPLE_PLUS, style::icon_font(20.0), p.text_faint);
        y += 20.0 + 6.0;
        painter.text(pos2(cx, y + 9.0), Align2::CENTER_CENTER, t("Add project"), style::medium(style::BODY), p.text_muted);
        y += 18.0 + 6.0;
        painter.text(pos2(cx, y + 8.0), Align2::CENTER_CENTER, t("or drop a folder here"), egui::FontId::proportional(style::SMALL), p.text_faint);
        y += 16.0 + 6.0;
        if !keys.is_empty() {
            let w = kit::kbd_width(&painter, keys);
            kit::paint_kbd_text(&painter, pos2(cx - w / 2.0, y + kit::KBD_H / 2.0), keys, &p);
        }
        if r.clicked() {
            self.actions.push(Action::AddProject);
        }
    }

    /// "Active" or "Waiting" with nothing there: the empty state, and Show all projects.
    fn nothing_active(&mut self, ui: &mut Ui, area: Rect) {
        let (icon, title, body) = if self.grid_filter == GridFilter::Waiting {
            (icons::CHECK_CIRCLE, t("Nothing is waiting"), t("Every agent is working or done.").to_string())
        } else {
            (icons::SQUARES_FOUR, t("Nothing is running"), tf("Open a project, or press {} to start an agent.", &[kit::keys(&Action::PickPreset).unwrap_or("")]))
        };
        if kit::empty_state_at(ui, area, icon, title, &body, Some((None, t("Show all projects"), None))).is_some_and(|r| r.clicked()) {
            self.actions.push(Action::GridFilter(GridFilter::All));
        }
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
        let letters = project.letters();
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

        // The card: a sheet in the terminal's own background, header included; ringed in amber
        // (1.5) while an agent in it waits for the user.
        let painter = ui.painter_at(rect.expand(8.0));
        let waiting_ring = attention && status_now == Some(Status::NeedsInput);
        let look = if hovered && !waiting_ring { super::focus::SheetLook::Focused } else { super::focus::SheetLook::Plain };
        super::focus::paint_sheet_fill(&painter, rect, scheme.background, &p, look);
        if waiting_ring {
            painter.rect_stroke(rect, CornerRadius::same(style::RADIUS_LG), Stroke::new(1.5, p.warn), egui::StrokeKind::Inside);
        }
        let painter = ui.painter_at(rect.expand(2.0));
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), TILE_HEADER));
        // The tile typed into: an accent ring over the edges, drawn last.
        let border = if keyboard_here { Stroke::new(1.5, p.accent) } else { Stroke::NONE };

        // Header.
        let hr = ui.interact(header, egui::Id::new(("tile-header", project_id)), Sense::click());
        // To Focus: the tab this tile shows.
        let open_action = open_pane.map_or(Action::SelectProject(project_id), Action::FocusPane);
        if hr.double_clicked() {
            self.actions.push(open_action.clone());
        }
        let cy = header.center().y;
        let small = egui::FontId::proportional(style::SMALL);
        // Right side first (open, the status, the server, the zoom, the browser); the left side
        // (tile, name, branch, tabs) is cut before it.
        let open_rect = Rect::from_center_size(pos2(header.right() - 8.0 - 12.0, cy), vec2(24.0, 24.0));
        let open = ui
            .scope_builder(egui::UiBuilder::new().max_rect(open_rect), |ui| {
                kit::icon_button_ex(ui, icons::ARROW_UP_RIGHT, &kit::tip(t("Open in Focus view"), Action::OpenGridTile), false, 24.0)
            })
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("tile-open:{name}"), open.rect);
        if open.clicked() {
            self.actions.push(open_action.clone());
        }
        let mut right = open_rect.left() - 8.0;
        // The status as a pill (the tabs' chips say it when there are several tabs).
        if strip.is_empty()
            && let Some((s, _, _, _, said, doing)) = &pane_status
            && !matches!(s, Status::Idle | Status::Ended)
        {
            let text = match s {
                Status::Running | Status::Unknown => match progress {
                    Some((_, Some(v))) => format!("{} · {v}%", status::text(*s)),
                    _ => status::text(*s),
                },
                Status::Failed(_) => t("Failed").to_string(),
                Status::CommandDone => t("Finished").to_string(),
                _ => status::text(*s),
            };
            let w = kit::state_pill_width(&painter, *s, &text);
            if right - w > header.left() + 150.0 {
                let r = kit::paint_state_pill(&painter, pos2(right - w, cy), *s, &text, &p, dark);
                right = r.left() - 6.0;
                let tip = match doing {
                    Some(doing) => format!("{doing}\n{said}"),
                    None => said.clone(),
                };
                ui.interact(r, egui::Id::new(("tile-status", project_id)), Sense::hover()).on_hover_text(tip);
            }
        }
        if let Some(agent) = unconnected && right - 22.0 > header.left() + 120.0 {
            let r = Rect::from_center_size(pos2(right - 11.0, cy), vec2(22.0, 22.0));
            right -= 22.0 + 6.0;
            let resp = ui.interact(r, egui::Id::new(("tile-unconnected", project_id)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("tile-unconnected:{}", agent.0), r);
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(style::RADIUS), style::tint(p.warn, dark));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            ui.painter().text(r.center(), Align2::CENTER_CENTER, icons::WARNING, style::icon_font(13.0), p.warn);
            if resp.on_hover_text(self.unconnected_tip(&[agent])).clicked() {
                self.actions.push(Action::PlanInstall(agent.1));
            }
        }
        // A local server its panes started: "● :5173" opens it in the browser.
        let servers = self.project_servers(project_id);
        if let Some((_, first)) = servers.first() {
            let port = format!(":{}", first.port);
            let more = if servers.len() > 1 { format!(" +{}", servers.len() - 1) } else { String::new() };
            let g = ui.painter().layout_no_wrap(format!("{port}{more}"), style::mono(10.5), p.text_muted);
            let w = 6.0 + 5.0 + 6.0 + g.size().x + 6.0;
            if right - w > header.left() + 120.0 {
                let r = Rect::from_center_size(pos2(right - w / 2.0, cy), vec2(w, 22.0));
                right -= w + 6.0;
                let resp = ui.interact(r, egui::Id::new(("tile-server", project_id)), Sense::click());
                #[cfg(debug_assertions)]
                crate::devtest::record(format!("tile-server:{name}"), r);
                if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(style::RADIUS), p.hover);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                ui.painter().circle_filled(pos2(r.left() + 6.0 + 2.5, cy), 5.0, style::soft(p.ok, 0.18));
                ui.painter().circle_filled(pos2(r.left() + 6.0 + 2.5, cy), 2.5, p.ok);
                ui.painter().galley(pos2(r.left() + 6.0 + 5.0 + 6.0, cy - g.size().y / 2.0), g, p.text_muted);
                let tip = servers.iter().map(|(_, s)| tf("Open {} in the browser", &[&s.url])).collect::<Vec<_>>().join("\n");
                if resp.on_hover_text(tip).clicked() {
                    self.actions.push(Action::OpenWeb(first.url.clone()));
                }
            }
        }
        // The browser an agent uses, on a tile too small for its picture: shows it in the tile.
        if let Some(w) = web.as_ref().filter(|_| web_small) {
            let g = ui.painter().layout_no_wrap(t("browser").to_string(), small.clone(), p.info);
            let wd = 6.0 + 13.0 + 4.0 + g.size().x + 6.0;
            if right - wd > header.left() + 120.0 {
                let r = Rect::from_center_size(pos2(right - wd / 2.0, cy), vec2(wd, 22.0));
                right -= wd + 6.0;
                let resp = ui.interact(r, egui::Id::new(("tile-web", project_id)), Sense::click());
                #[cfg(debug_assertions)]
                crate::devtest::record(format!("tile-web-chip:{name}"), r);
                let fill = style::tint(p.info, dark);
                ui.painter().rect_filled(r, CornerRadius::same(style::RADIUS), if resp.hovered() { fill.gamma_multiply(1.6) } else { fill });
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                ui.painter().text(pos2(r.left() + 6.0 + 6.5, cy), Align2::CENTER_CENTER, icons::GLOBE, style::icon_font(12.0), p.info);
                ui.painter().galley(pos2(r.left() + 6.0 + 13.0 + 4.0, cy - g.size().y / 2.0), g, p.info);
                let click_hint = if cfg!(target_os = "macos") {
                    t("Click: show the browser in this tile · ⌥-click: in Focus")
                } else {
                    t("Click: show the browser in this tile · Alt-click: in Focus")
                };
                let tip = format!("{} · {}\n{click_hint}", w.address, w.who);
                if resp.on_hover_text(tip).clicked() {
                    self.show_tile_web(project_id, w, ui.input(|i| i.modifiers.alt) || !panel_fits);
                }
            }
        }
        let left_limit = right - 6.0;
        let left = painter.with_clip_rect(Rect::from_min_max(header.min, pos2(left_limit, header.max.y)));
        // Left: the project's tile and name, its branch and what changed, then its tabs.
        let mut x = header.left() + 12.0;
        let tile = Rect::from_min_size(pos2(x, cy - 10.0), vec2(20.0, 20.0));
        kit::paint_project_tile(&left, tile, &letters, color, dark);
        x = tile.right() + 8.0;
        // A worktree goes by its project's name; its branch says which one it is.
        let g = left.layout_no_wrap(parent_name.clone().unwrap_or_else(|| name.clone()), style::medium(style::BODY), p.text);
        let gw = g.size().x;
        left.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
        x += gw + 8.0;
        // "main · 5 changed" (a worktree's branch with how far ahead), panes when split.
        let with_strip = !strip.is_empty();
        if let Some(b) = &branch {
            let mut text = match ahead {
                Some(n) if n > 0 => format!("{b} ↑{n}"),
                _ => b.clone(),
            };
            if changed > 0 {
                text = format!("{text} · {}", tf("{} changed", &[&changed.to_string()]));
            }
            let room = (left_limit - x - if with_strip { 90.0 } else { 0.0 }).max(0.0);
            if room > 40.0 {
                let mut job = egui::text::LayoutJob::simple_singleline(text, style::mono(style::DATA), p.text_faint);
                job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                let bg = left.layout_job(job);
                let bw = bg.size().x;
                left.galley(pos2(x, cy - bg.size().y / 2.0), bg, p.text_faint);
                x += bw + 8.0;
            }
        }
        if count > 1 && !with_strip {
            let g = left.layout_no_wrap(format!("{} {count}", icons::SQUARE_SPLIT_HORIZONTAL), style::mono(style::DATA), p.text_faint);
            let gw = g.size().x;
            if x + gw < left_limit {
                left.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
                x += gw + 8.0;
            }
        }
        // The strip of tabs, after the branch: the chips say what each tab holds.
        if with_strip {
            self.tab_strip(ui, &left, project_id, &name, &strip, shown, held, x, cy, left_limit, scheme.background);
        }
        // Progress a program reports, under the header.
        let mut body_top = header.bottom() - 6.0;
        if let Some((state, value)) = progress {
            let line = Rect::from_min_size(pos2(rect.left() + 10.0, header.bottom() - 2.0), vec2(rect.width() - 20.0, 2.0));
            let c = if state == 2 { p.err } else if state == 4 { p.warn } else { p.info };
            painter.rect_filled(line, CornerRadius::same(1), p.border_strong);
            let f = value.map_or(0.3, |v| v.min(100) as f32 / 100.0);
            painter.rect_filled(Rect::from_min_size(line.min, vec2(line.width() * f, 2.0)), CornerRadius::same(1), c);
            body_top += 4.0;
        }
        // What the tile asks of the user, in its foot: the question, what an agent finished,
        // what failed, or the agent that can be resumed.
        let mut body_bottom = rect.bottom() - 5.0;
        let resume_pane = layout.as_ref().map(|l| l.panes()).unwrap_or_default().into_iter().find(|id| self.panes.get(id).is_some_and(|x| x.resume.is_some()));
        let foot_kind = match (attention, status_now) {
            (true, Some(Status::NeedsInput)) => Some(Foot::Wait),
            (true, Some(Status::Done | Status::CommandDone)) => Some(Foot::Done),
            (true, Some(Status::Failed(_))) => Some(Foot::Fail),
            _ => resume_pane.map(Foot::Resume),
        };
        if let Some(kind) = foot_kind.filter(|_| rect.height() > TILE_HEADER + TILE_FOOT_H + 60.0) {
            let area = Rect::from_min_max(pos2(rect.left() + FOOT_PAD, rect.top()), pos2(rect.right() - FOOT_PAD, rect.bottom() - FOOT_PAD));
            let since = pane_id.and_then(|id| self.panes.get(&id)).map(|x| x.tracker.since);
            let top = self.tile_foot(ui, area, kind, pane_id, pane_status.as_ref().map(|x| (x.2.clone(), x.3.clone(), x.0)), since, review, &open_action, &name);
            body_bottom = top;
        }

        // Body: inset so the terminal's square corners stay inside the card's round ones.
        let body = Rect::from_min_max(pos2(rect.left() + 4.0, body_top), pos2(rect.right() - 4.0, body_bottom));
        let Some(pane_id) = pane_id else {
            if border != Stroke::NONE {
                painter.rect_stroke(rect, radius, border, egui::StrokeKind::Inside);
            }
            return;
        };
        let style = tile_style(self.term_style(ui), ui.ctx());
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
            if border != Stroke::NONE {
                ui.painter_at(rect.expand(2.0)).rect_stroke(rect, radius, border, egui::StrokeKind::Inside);
            }
            return;
        }
        // The panel at the right as in Focus (its share of the width), the terminals beside it as
        // Focus lays them (stacked when Focus stacks them).
        let (panes_body, panel_rect) = match tile_page {
            Some((_, zoom)) => {
                let share = self.panel_of(project_id).map_or(crate::project::PANEL_SHARE, crate::project::Panel::share);
                let w = (body.width() * share).round();
                // The page in its own rounded frame, inside the tile's right side; above it, how
                // much smaller than in Focus it is shown (hover says why).
                let column = Rect::from_min_max(pos2(body.right() - w, body.top() + 6.0), pos2(body.right() - 4.0, body.bottom() - 1.0));
                let mut page = column;
                if zoom < 0.995 {
                    let pct = (zoom * 100.0).round() as i32;
                    let label = Rect::from_min_size(column.min, vec2(40.0, 14.0));
                    let resp = ui.interact(label, egui::Id::new(("tile-zoom", project_id)), Sense::hover());
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("tile-zoom:{name}"), label);
                    painter.text(label.left_center(), Align2::LEFT_CENTER, format!("{pct}%"), style::mono(style::DATA), if resp.hovered() { p.text_muted } else { p.text_faint });
                    resp.on_hover_text(tf("Shown at {}%, laid out as in Focus. Open in Focus to see it at 100%.", &[&pct.to_string()]));
                    page.min.y += 14.0 + 4.0;
                }
                (Rect::from_min_max(body.min, pos2(body.right() - w - 6.0, body.bottom())), Some(page))
            }
            None => (body, None),
        };
        let stacked = tile_page.is_some() && self.panel_of(project_id).is_some_and(|x| x.stacked);
        let (rects, dividers) = match &layout {
            Some(l) if stacked => l.stacked().arrange(panes_body),
            Some(l) => l.arrange(panes_body),
            None => (vec![(pane_id, panes_body)], Vec::new()),
        };
        // (An agent that can be resumed says so in the tile's foot.)
        // The terminals take the tile's size (before they are drawn, so a sleeping shell starts
        // at it).
        let sizes = rects.iter().map(|(id, r)| (*id, widget::grid_for(*r, cell))).collect();
        self.resize_panes(&ui.ctx().clone(), sizes);
        #[cfg(debug_assertions)]
        for (id, r) in &rects {
            crate::devtest::record(format!("tile-pane:{id}"), *r);
        }
        // Split terminals in a tile: a hairline down the middle of the gap between them.
        for d in &dividers {
            match d.dir {
                tc_term::layout::SplitDir::Horizontal => painter.vline(d.rect.center().x, d.rect.y_range(), Stroke::new(1.0, p.border)),
                tc_term::layout::SplitDir::Vertical => painter.hline(d.rect.x_range(), d.rect.center().y, Stroke::new(1.0, p.border)),
            };
        }
        for (id, r) in rects {
            self.grid_pane(ui, id, r, &style, min_scale, input_enabled, wheel_free || keyboard_here);
        }
        if let (Some((page, zoom)), Some(r)) = (tile_page, panel_rect) {
            self.tile_browser(ui, page, r, zoom);
        }
        if let Some(w) = web.filter(|_| !web_small) {
            self.tile_pip(ui, project_id, body, &w);
        }
        // The ring of the tile typed into last, over the terminals' edges.
        if border != Stroke::NONE {
            ui.painter_at(rect.expand(2.0)).rect_stroke(rect, radius, border, egui::StrokeKind::Inside);
        }
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
        let size = if body.width() >= 520.0 && body.height() >= 300.0 { vec2(230.0, 146.0) } else { vec2(170.0, 112.0) };
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
        // A small window: who does what over the page's picture, ringed in the accent as an
        // agent's page is, with a menu's shadow.
        let radius = CornerRadius::same(style::RADIUS_CARD);
        let shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: egui::Color32::from_black_alpha(if ui.visuals().dark_mode { 90 } else { 40 }) };
        ui.painter().add(shadow.as_shape(r, radius));
        let painter = ui.painter_at(r.expand(2.0));
        painter.rect_filled(r, radius, egui::Color32::WHITE);
        let cap = Rect::from_min_size(r.min, vec2(r.width(), 26.0));
        let shot = Rect::from_min_max(pos2(r.left(), cap.bottom()), r.max);
        if let Some((tex, _)) = self.browsers.get(&web.id).and_then(|b| b.still.as_ref()) {
            let s = tex.size_vec2();
            let h = shot.width() * s.y / s.x.max(1.0);
            ui.painter_at(shot.shrink(1.0)).image(tex.id(), Rect::from_min_size(shot.min, vec2(shot.width(), h)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        painter.rect_filled(cap, CornerRadius { nw: style::RADIUS_CARD, ne: style::RADIUS_CARD, sw: 0, se: 0 }, p.menu);
        status::paint_glyph(&painter, pos2(cap.left() + 8.0 + 6.0, cap.center().y), Status::Running, &p, 12.0);
        let what = if web.who.is_empty() { web.doing.clone() } else { format!("{} · {}", web.who, web.doing) };
        let mut job = egui::text::LayoutJob::simple_singleline(what, egui::FontId::proportional(style::SMALL), p.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((cap.width() - 8.0 - 12.0 - 6.0 - 6.0 - 12.0 - 8.0).max(10.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(cap.left() + 8.0 + 12.0 + 6.0, cap.center().y - g.size().y / 2.0), g, p.text);
        painter.text(pos2(cap.right() - 8.0 - 6.0, cap.center().y), Align2::CENTER_CENTER, icons::ARROWS_OUT_SIMPLE, style::icon_font(12.0), if resp.hovered() { p.text } else { p.text_faint });
        painter.rect_stroke(r, radius, Stroke::new(if resp.hovered() { 2.0 } else { 1.5 }, p.accent), egui::StrokeKind::Outside);
        let click_hint = if cfg!(target_os = "macos") {
            t("Click: show the browser in this tile · ⌥-click: in Focus")
        } else {
            t("Click: show the browser in this tile · Alt-click: in Focus")
        };
        let tip = format!("{} · {}\n{click_hint}", web.address, web.who);
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
        // The page in its own rounded frame, ringed in the accent while an agent uses it (as in
        // the Browser panel).
        let round = super::browser_ui::PAGE_ROUND;
        if driven {
            ui.painter().rect_stroke(rect, CornerRadius::same(round as u8), Stroke::new(3.0, style::soft(p.accent, 0.16)), egui::StrokeKind::Outside);
        }
        ui.painter().rect(rect, CornerRadius::same(round as u8), p.canvas, Stroke::new(if driven { 2.0 } else { 1.0 }, if driven { p.accent } else { p.border }), egui::StrokeKind::Inside);
        let page = rect.shrink(if driven { 2.0 } else { 1.0 });
        // Its corners within the frame's rounded border (`browser::round_corners`).
        let corner = round - if driven { 2.0 } else { 1.0 };
        let whole = ui.clip_rect().contains_rect(page) && page.width() >= 40.0 && page.height() >= 40.0;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("tile-browser:{id}"), page);
        if whole && !b.url.is_empty() {
            if let Some(b) = self.browsers.get_mut(&id) {
                b.corner_want = corner;
            }
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
        // The terminal is to take the keyboard: the page the user typed in lets go of it.
        if request && self.browser_focus.is_some() {
            self.keyboard_to_app();
        }
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
        if self.view != View::Grid {
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
        // The tab shown in the medium weight, the others regular.
        let labels: Vec<std::sync::Arc<egui::Galley>> = tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let font = if i == shown { style::medium(style::SMALL) } else { egui::FontId::proportional(style::SMALL) };
                let mut job = egui::text::LayoutJob::simple_singleline(t.label.clone(), font, p.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width(160.0);
                painter.layout_job(job)
            })
            .collect();
        // The pin: 4 px after the label, 10.5 px wide.
        let pin_w = 14.5;
        let label_w = |i: usize| 7.0 + 12.0 + 5.0 + labels[i].size().x + if held && i == shown { pin_w } else { 0.0 } + 7.0;
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
        let icon_of = |t: &StripTab| match (t.status, &t.resume) {
            (Some(s), _) => (status::icon(s), status::color(s, &p)),
            (None, Some(_)) => (icons::CLOCK_COUNTER_CLOCKWISE, p.accent_text),
            (None, None) => (
                match t.web {
                    Some(true) => icons::MOON,
                    Some(false) => icons::GLOBE,
                    None => icons::TERMINAL_WINDOW,
                },
                p.text_faint,
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
            // The tab shown raised; one not shown where something waits tinted, with a dot.
            let waits = !on && tab.waits;
            if on {
                painter.rect(rect, CornerRadius::same(6), p.surface, Stroke::new(1.0, if ui.visuals().dark_mode { p.border } else { p.border_strong }), egui::StrokeKind::Inside);
            } else if waits {
                painter.rect_filled(rect, CornerRadius::same(6), style::tint(p.warn, ui.visuals().dark_mode));
            } else if resp.hovered() {
                painter.rect_filled(rect, CornerRadius::same(6), p.hover);
            }
            // Its status's glyph (the running one turns), else what it holds.
            let icon_x = if mode == StripMode::Labels { rect.left() + 7.0 + 6.0 } else { rect.center().x };
            match tab.status {
                Some(s) if status::has_glyph(s) => status::paint_glyph(painter, pos2(icon_x, cy), s, &p, 12.0),
                _ => {
                    let (icon, ic) = icon_of(tab);
                    painter.text(pos2(icon_x, cy), Align2::CENTER_CENTER, icon, style::icon_font(12.0), ic);
                }
            }
            if mode == StripMode::Labels {
                let tc = if waits {
                    p.warn
                } else if on {
                    p.text
                } else {
                    p.text_faint
                };
                let g = labels[i].clone();
                let lx = rect.left() + 7.0 + 12.0 + 5.0;
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

/// The outline of `rect` with corners of `radius`, as points (a dashed line follows it).
fn rounded_outline(rect: Rect, radius: f32) -> Vec<egui::Pos2> {
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (pos2(rect.right() - r, rect.top() + r), -90.0_f32),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), 90.0),
        (pos2(rect.left() + r, rect.top() + r), 180.0),
    ];
    let mut pts = vec![pos2(rect.left() + r, rect.top())];
    for (c, start) in corners {
        for i in 0..=8 {
            let a = (start + 90.0 * i as f32 / 8.0).to_radians();
            pts.push(pos2(c.x + r * a.cos(), c.y + r * a.sin()));
        }
    }
    pts.push(pos2(rect.left() + r, rect.top()));
    pts
}

/// The dot of a chip whose tab waits for the user: at its top right corner, ringed by the tile.
fn chip_dot(painter: &egui::Painter, rect: Rect, color: egui::Color32, ring: egui::Color32) {
    let c = pos2(rect.right() - 1.0, rect.top() + 1.0);
    painter.circle_filled(c, 4.5, ring);
    painter.circle_filled(c, 3.0, color);
}

/// What a tile's foot says.
#[derive(Clone, Copy, PartialEq)]
enum Foot {
    Wait,
    Done,
    Fail,
    /// The pane whose agent ran there before a restart.
    Resume(PaneId),
}

impl App {
    /// What a finished agent's run changed (files, added, removed): worked out once per finish,
    /// off the UI thread, from the snapshot taken when it started. `None` until it is known.
    pub(super) fn done_stats(&mut self, pane: PaneId) -> Option<(usize, u32, u32)> {
        let x = self.panes.get(&pane)?;
        let since = x.tracker.since;
        if let Some((at, stats)) = &x.done_stats
            && *at == since
        {
            return *stats;
        }
        let (tree, _, _) = x.run_snapshot.clone()?;
        let root = self.projects.iter().find(|p| p.panes().contains(&pane))?.path.clone();
        if let Some(x) = self.panes.get_mut(&pane) {
            x.done_stats = Some((since, None));
        }
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let stats = tc_git::repo_root(&root).and_then(|top| {
                let (_, now) = tc_git::worktree_vs_head(&top).ok()?;
                let files = tc_git::changes(&top, &tree, &now).ok()?;
                let (a, d) = files.iter().fold((0, 0), |(a, d), f| (a + f.additions.unwrap_or(0), d + f.deletions.unwrap_or(0)));
                Some((files.len(), a, d))
            });
            let _ = tx.send(crate::app::AppMsg::DoneStats(pane, since, stats));
            ctx.request_repaint();
        });
        None
    }

    /// A tile's foot: the notice that says what the tile asks of the user, with what to do about
    /// it, at the bottom of `area` (the tile less its padding). `said`: the pane's message, its
    /// name, its status. Returns where the notice starts (the terminals end there).
    #[allow(clippy::too_many_arguments)]
    fn tile_foot(
        &mut self,
        ui: &mut Ui,
        area: Rect,
        kind: Foot,
        pane: Option<PaneId>,
        said: Option<(Option<String>, String, Status)>,
        since: Option<Instant>,
        review: Option<PaneId>,
        open: &Action,
        name: &str,
    ) -> f32 {
        let _ = name;
        // An agent to resume: the pane's own banner (`resume_banner`).
        if let Foot::Resume(id) = kind {
            let rect = Rect::from_min_max(pos2(area.left(), area.bottom() - kit::NOTICE_H), area.max);
            self.resume_banner(ui, id, rect, true);
            #[cfg(debug_assertions)]
            {
                let (dismiss, resume, _) = notice_buttons(ui, rect, Some((t("Resume session"), Some("↵"))), None, true);
                resume.into_iter().for_each(|r| crate::devtest::record(format!("tile-resume:{id}"), r));
                dismiss.into_iter().for_each(|r| crate::devtest::record(format!("tile-dismiss:{id}"), r));
            }
            return rect.top();
        }
        let (message, label, status) = said.unwrap_or((None, String::new(), Status::Idle));
        let who = pane.and_then(|id| self.panes.get(&id)).and_then(|x| x.tracker.agent.or(x.last_agent)).map_or_else(|| label.clone(), |a| a.label().to_string());
        // What it says: tone, title, second line, the main button (and its key), the other one.
        let mut approvable = false;
        let (tone, title, body, primary, secondary): (kit::Tone, String, Option<String>, (&str, Option<&str>), Option<&str>) = match kind {
            Foot::Wait => {
                use super::focus::Ask;
                let ask = super::focus::ask_of(message.as_deref());
                approvable = !matches!(ask, Ask::Question(_));
                let (title, body) = match ask {
                    Ask::Permission { tool, target } if tool == "Bash" => (tf("{} wants to run a command", &[&who]), Some(format!("Bash · {target}"))),
                    Ask::Permission { tool, target } => (tf("{} wants to use {}", &[&who, &tool]), Some(format!("{tool} · {target}"))),
                    Ask::Plan(plan) => (tf("{} wants to apply its plan", &[&who]), Some(plan)),
                    Ask::Question(q) => (tf("{} has a question", &[&who]), Some(q)),
                    Ask::Confirm(said) => (tf("{} needs your confirmation", &[&who]), said),
                };
                if approvable {
                    (kit::Tone::Input, title, body, (t("Approve"), Some("↵")), Some(t("Open")))
                } else {
                    (kit::Tone::Input, title, body, (t("Open"), None), None)
                }
            }
            Foot::Done => {
                let title = if status == Status::CommandDone { tf("{} finished", &[&label]) } else { tf("{} finished its turn", &[&who]) };
                // What its run changed ("2 files · +41"), else what it said last.
                let stats = pane.filter(|_| status == Status::Done).and_then(|id| self.done_stats(id)).filter(|s| s.0 > 0);
                let body = match stats {
                    Some((n, a, d)) => {
                        let mut s = format!("{} · +{a}", crate::i18n::count(n, t("1 file"), t("{} files")));
                        if d > 0 {
                            s.push_str(&format!(" −{d}"));
                        }
                        Some(s)
                    }
                    None => message.filter(|_| status == Status::Done),
                };
                (kit::Tone::Done, title, body, (t("Commit…"), None), Some(t("Review")))
            }
            Foot::Fail => {
                let what = message.or_else(|| pane.and_then(|id| self.panes.get(&id)).and_then(|x| x.program_title.clone())).unwrap_or_else(|| label.clone());
                let code = match status {
                    Status::Failed(code) => code,
                    _ => 1,
                };
                let ago = since.map(|s| tf("{} ago", &[&status::age(s)]));
                (kit::Tone::Failed, tf("{} exited with code {}", &[&what, &code.to_string()]), ago, (t("Open"), None), None)
            }
            Foot::Resume(_) => return area.bottom(),
        };
        let h = if body.is_some() { kit::NOTICE_H } else { kit::NOTICE_H_SHORT };
        let rect = Rect::from_min_max(pos2(area.left(), area.bottom() - h), area.max);
        let out = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                kit::notice(ui, rect.width(), tone, None, &title, body.as_deref(), Some(primary), secondary, false)
            })
            .inner;
        #[cfg(debug_assertions)]
        {
            crate::devtest::record(format!("tile-foot:{name}"), out.rect);
            let (_, first, second) = notice_buttons(ui, out.rect, Some(primary), secondary, false);
            if let Some(id) = review.filter(|_| kind == Foot::Done) {
                first.into_iter().for_each(|r| crate::devtest::record(format!("tile-commit:{id}"), r));
                second.into_iter().for_each(|r| crate::devtest::record(format!("tile-review:{id}"), r));
            }
            if kind == Foot::Wait {
                first.into_iter().for_each(|r| crate::devtest::record(format!("tile-foot-primary:{name}"), r));
                second.into_iter().for_each(|r| crate::devtest::record(format!("tile-foot-open:{name}"), r));
            }
        }
        match kind {
            Foot::Wait if approvable => {
                // As in its pane: Enter answers yes.
                if out.primary
                    && let Some(x) = pane.and_then(|id| self.panes.get_mut(&id))
                {
                    x.entered(Instant::now());
                    x.input(b"\r".to_vec());
                }
                if out.secondary {
                    self.actions.push(open.clone());
                }
            }
            Foot::Wait | Foot::Fail => {
                if out.primary {
                    self.actions.push(open.clone());
                }
            }
            Foot::Done => {
                if let Some(id) = review.or(pane) {
                    if out.primary {
                        self.actions.push(Action::FocusPane(id));
                        self.actions.push(Action::SetView(View::Focus));
                        self.actions.push(Action::Commit);
                    }
                    if out.secondary {
                        self.actions.push(Action::FocusPane(id));
                        self.actions.push(Action::ReviewAgent);
                    }
                }
            }
            Foot::Resume(_) => {}
        }
        rect.top()
    }
}

/// Where `kit::segmented` puts its segments of words only (`labels`) in `rect`, each as wide
/// as its word, for the development scripts' clicks.
#[cfg(debug_assertions)]
pub(super) fn word_segments(ui: &Ui, rect: Rect, labels: &[String]) -> Vec<Rect> {
    let mut x = rect.left() + 2.0;
    labels
        .iter()
        .map(|l| {
            let w = ui.painter().layout_no_wrap(l.clone(), style::medium(12.0), egui::Color32::PLACEHOLDER).size().x + 20.0;
            let r = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(w, rect.height() - 4.0));
            x += w + 2.0;
            r
        })
        .collect()
}

/// Where `kit::notice` puts its buttons in `rect` (dismiss, primary, secondary), for the
/// development scripts' clicks.
#[cfg(debug_assertions)]
fn notice_buttons(ui: &Ui, rect: Rect, primary: Option<(&str, Option<&str>)>, secondary: Option<&str>, dismiss: bool) -> (Option<Rect>, Option<Rect>, Option<Rect>) {
    let cy = rect.center().y;
    let mut right = rect.right() - 8.0;
    let mut at = |w: f32| {
        let r = Rect::from_min_max(pos2(right - w, cy - 12.0), pos2(right, cy + 12.0));
        right -= w + 6.0;
        r
    };
    let d = dismiss.then(|| at(24.0));
    let p = primary.map(|(label, keys)| at(kit::button_width(ui, None, label, keys, true)));
    let s = secondary.map(|label| at(kit::button_width(ui, None, label, None, true)));
    (d, p, s)
}

/// The terminal style of a tile: the Focus view's, in the data font's size (`TILE_FONT`).
fn tile_style(base: widget::TermStyle<'static>, ctx: &egui::Context) -> widget::TermStyle<'static> {
    let font_size = base.font_size * TILE_FONT;
    widget::TermStyle { font_size, cell: widget::cell_size(ctx, font_size), ..base }
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
