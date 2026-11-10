//! Grid (FR-V2–V5, V7): every project's terminal at once, two columns
//! and two rows to a screen, scrolling for more ("Active" keeps only projects where something
//! runs or waits). Each tile shows the project's most relevant pane at its real grid size, scaled
//! down: the PTY is never resized, so agents' full-screen interfaces don't redraw when the view
//! changes. A tile is drawn only when it is in view, and a sleeping pane's shell starts when its
//! tile is first drawn (as opening the project would).

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::ProjectId;
use tc_core::status::{Foreground, Status, Tracker};
use tc_core::{PaneId, model::View};
use tc_term::widget;

use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};

const TILE_HEADER: f32 = 30.0;
const GAP: f32 = 8.0;
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
    /// The pane a tile shows: one that needs attention, else one running an agent, else the
    /// project's focused pane.
    pub fn tile_pane(&self, project: ProjectId) -> Option<PaneId> {
        let p = self.project(project)?;
        let panes = p.panes();
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
        wants.or_else(agent).or_else(|| p.current_pane())
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

    /// The projects the Grid shows: all of them, or with "Active" those where
    /// something runs or waits. A frame is asked for when a program is about to make a project
    /// active, since nothing else may draw one.
    fn shown_projects(&self, ctx: &egui::Context) -> Vec<ProjectId> {
        if !self.grid_active_only {
            return self.projects.iter().map(|p| p.id).collect();
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
        let n = ids.len();
        let (cols, rows_in_view) = grid_shape(n, self.grid_columns);
        let rows = n.div_ceil(cols);
        let h = ((area.height() - GAP * (rows_in_view as f32 - 1.0)) / rows_in_view as f32).max(TILE_HEADER + 80.0);
        let total = rows as f32 * h + GAP * (rows as f32 - 1.0);
        let scrolls = total > area.height() + 0.5;
        // Room for the scroll bar beside the tiles.
        let width = area.width() - if scrolls { 10.0 } else { 0.0 };
        let w = (width - GAP * (cols as f32 - 1.0)) / cols as f32;
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            // A thin bar always in view when there is more below the 2 × 2.
            ui.spacing_mut().scroll = egui::style::ScrollStyle::thin();
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
                    let (content, _) = ui.allocate_exact_size(vec2(width, total), Sense::hover());
                    let visible = ui.clip_rect();
                    let mut drawn = Vec::new();
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
                    // The keyboard was meant for a pane out of view (the project open before):
                    // it would jump there when scrolled to, after the user chose another tile.
                    if self.focus_request.is_some_and(|f| !drawn.contains(&f)) {
                        self.focus_request = None;
                    }
                });
        });
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
                let hint = tf(
                    "Nothing is running. Open a project, or press {} to start an agent.",
                    &[kit::keys(&Action::PickPreset).unwrap_or("")],
                );
                ui.label(egui::RichText::new(hint).size(style::BODY).color(p.text_muted));
                ui.add_space(10.0);
                if kit::secondary(ui, None, t("Show All Projects")).clicked() {
                    self.actions.push(Action::GridActiveOnly(false));
                }
            },
        );
    }

    /// One project: header (name, status, branch, a resume button, open in Focus) over its tab's
    /// panes. `wheel_free`: the mouse wheel may scroll the terminals (the Grid does not scroll);
    /// else only the one taking the keyboard gets it.
    fn tile(&mut self, ui: &mut Ui, project_id: ProjectId, rect: Rect, wheel_free: bool) {
        let p = style::pal(ui);
        // Its panes at their Focus size (as the project would open now).
        self.fit_to_focus(&ui.ctx().clone(), project_id, true);
        let Some(project) = self.project(project_id) else {
            return;
        };
        let pane_id = self.tile_pane(project_id);
        let name = project.name.clone();
        let color = project.color32();
        let branch = project.git.as_ref().map(|g| g.branch.clone());
        let count = project.panes().len();
        let pane_status = pane_id.and_then(|id| self.panes.get(&id)).map(|x| {
            (
                x.tracker.status,
                x.tracker.wants_attention(),
                x.tracker.message.clone(),
                x.label(),
            )
        });
        // The tab in view, with all its panes (a split project shows its split).
        let layout = project.tab().map(|t| t.layout.clone());
        // An agent that ran in one of them and can be continued.
        let resume = layout.as_ref().and_then(|l| {
            l.panes().into_iter().find_map(|id| {
                self.panes
                    .get(&id)
                    .and_then(|x| x.resume.as_ref())
                    .map(|(a, _)| (id, a.label()))
            })
        });
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

        // The card: the terminal's own background, header included, on the darker Grid.
        let painter = ui.painter_at(rect.expand(2.0));
        painter.rect_filled(rect, radius, scheme.background);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), TILE_HEADER));
        painter.hline(
            (rect.left() + 1.0)..=(rect.right() - 1.0),
            header.bottom(),
            Stroke::new(1.0, p.border.gamma_multiply(0.7)),
        );
        let border = if keyboard_here {
            Stroke::new(1.5, p.accent)
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
        // Right side first (open button, resume, panes, branch); the left side is cut before it.
        let open_rect = Rect::from_center_size(pos2(header.right() - 18.0, cy), vec2(22.0, 22.0));
        let open = ui
            .scope_builder(egui::UiBuilder::new().max_rect(open_rect), |ui| {
                kit::icon_button_ex(ui, icons::ARROW_SQUARE_OUT, t("Open in Focus view"), false, 22.0)
            })
            .inner;
        if open.clicked() {
            self.actions.push(Action::SelectProject(project_id));
        }
        let mut right = header.right() - 36.0;
        if let Some((id, agent)) = resume {
            let text = tf("Resume {}", &[agent]);
            let w = style::text_width(ui, &text) + 34.0;
            let r = Rect::from_center_size(pos2(right - w / 2.0, cy), vec2(w, 22.0));
            right -= w + 8.0;
            let b = ui
                .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                    kit::button_ex(ui, kit::Kind::Secondary, Some(icons::PLAY), &text, true)
                })
                .inner;
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("tile-resume:{id}"), b.rect);
            if b.on_hover_text(tf("Continue {}'s last session here", &[agent])).clicked() {
                self.actions.push(Action::Resume(id));
            }
        }
        // Panes (when split), branch, and how many files changed: only what fits.
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
        let mut meta_left = right;
        let mut placed = Vec::new();
        for (text, font, color) in meta.into_iter().rev() {
            let g = painter.layout_no_wrap(text, font, color);
            if meta_left - g.size().x < header.left() + 140.0 {
                break;
            }
            meta_left -= g.size().x;
            placed.push((meta_left, g, color));
            meta_left -= 8.0;
        }
        for (x, g, color) in placed {
            painter.galley(pos2(x, cy - g.size().y / 2.0), g, color);
        }
        let left_limit = meta_left - 6.0;
        let left = painter.with_clip_rect(Rect::from_min_max(header.min, pos2(left_limit, header.max.y)));
        let mut x = header.left() + 12.0;
        left.circle_filled(pos2(x + 4.0, cy), 4.0, color);
        x += 14.0;
        let g = left.layout_no_wrap(name, style::semibold(13.0), p.text);
        let gw = g.size().x;
        left.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
        x += gw + 10.0;
        if let Some((s, attention, message, label)) = &pane_status {
            // What runs there, and its status when it says something (not idle).
            let quiet = matches!(s, Status::Idle | Status::Unknown);
            let (icon, c) = match s {
                Status::Idle => (icons::TERMINAL, p.text_faint),
                _ => (status::icon(*s), status::color(*s, &p)),
            };
            left.text(pos2(x + 6.0, cy), Align2::CENTER_CENTER, icon, style::icon_font(12.5), c);
            x += 17.0;
            let text = match (s, message) {
                (Status::NeedsInput, Some(m)) => format!("{label} · {m}"),
                _ if quiet => label.clone(),
                _ => format!("{label} · {}", status::text(*s)),
            };
            let mut job = egui::text::LayoutJob::simple_singleline(
                text,
                small.clone(),
                if *attention || !quiet { c } else { p.text_muted },
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width((left_limit - x).max(0.0));
            let sg = left.layout_job(job);
            left.galley(pos2(x, cy - sg.size().y / 2.0), sg, p.text_muted);
            if *attention {
                painter.rect_filled(
                    Rect::from_min_size(rect.min + vec2(0.0, 10.0), vec2(3.0, rect.height() - 20.0)),
                    CornerRadius::same(2),
                    c,
                );
            }
        }

        // Body: inset so the terminal's square corners stay inside the card's round ones.
        let body = Rect::from_min_max(pos2(rect.left() + 4.0, header.bottom() + 2.0), rect.max - vec2(4.0, 5.0));
        let Some(pane_id) = pane_id else {
            painter.rect_stroke(rect, radius, border, egui::StrokeKind::Inside);
            return;
        };
        let cell = widget::cell_size(ui.ctx(), self.settings.font_size);
        let style = widget::TermStyle {
            font_size: self.settings.font_size,
            cell,
            scheme,
            real_bold: false,
            real_italic: false,
            option_as_meta: self.settings.option_as_meta,
            copy_on_select: self.settings.copy_on_select,
            cursor_blink: self.settings.cursor_blink,
        };
        let input_enabled = self.dialog.is_none() && self.palette.is_none();
        let min_scale = MIN_SCALE;
        // Too low for even a few lines at a readable size: a compact summary.
        if body.height() < cell.y * min_scale * 3.0 + 12.0 {
            let Some(pane) = self.panes.get(&pane_id) else { return };
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
        for d in &dividers {
            painter.rect_filled(d.rect, CornerRadius::ZERO, p.border.gamma_multiply(0.8));
        }
        for (id, r) in rects {
            self.grid_pane(ui, id, r, &style, min_scale, input_enabled, wheel_free || keyboard_here);
        }
        // The border last, over the terminals' edges.
        ui.painter_at(rect.expand(2.0)).rect_stroke(rect, radius, border, egui::StrokeKind::Inside);
    }

    /// One pane inside a tile: drawn at its real grid size (never resized), not smaller than
    /// `min_scale`, typing goes to it once clicked. A sleeping pane starts its shell now, at the
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
        if resp.reconnect {
            self.actions.push(Action::RestartShell(pane_id));
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
