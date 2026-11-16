//! The Browser panel: a project's pages at the right of its terminals in Focus, like the Code
//! drawer (the two take turns there). Each agent has a page of its own in it and so does the
//! user, under a strip of their tabs; the page in front follows the terminal the user picks.
//! Terminals are never squeezed for it: side by side they show stacked while it is open, or
//! it floats over them. A Grid tile big enough shows it too, its page zoomed out so that it
//! keeps the layout it has in Focus.

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::{PaneId, ProjectId};

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};
use crate::project::{Panel, PanelPlace};

/// The strip of page tabs at the top of the panel.
pub const TABS_H: f32 = 34.0;
/// The least a terminal gets beside the panel (else they show stacked, or it floats).
pub const MIN_COLS: usize = 60;
pub const MIN_ROWS: usize = 12;
/// A tile shows its project's panel from this width of its body on.
const TILE_PANEL_MIN_W: f32 = 560.0;
const TILE_PANEL_MIN_H: f32 = 200.0;
/// The least a page in a tile is zoomed to (smaller text is not read).
const MIN_ZOOM: f32 = 0.5;
/// A page tab's width.
const TAB_MIN: f32 = 96.0;
const TAB_MAX: f32 = 210.0;
const TAB_GAP: f32 = 3.0;

impl App {
    pub fn panel_of(&self, project: ProjectId) -> Option<&Panel> {
        self.project(project).map(|p| &p.panel)
    }

    pub fn panel_mut(&mut self, project: ProjectId) -> Option<&mut Panel> {
        self.project_mut(project).map(|p| &mut p.panel)
    }

    /// The Code drawer shows for the project (the panel waits behind it).
    pub fn drawer_shown(&self, project: ProjectId) -> bool {
        self.editors.get(&project).is_some_and(|e| e.visible)
    }

    /// The page the project's panel shows in Focus now: open, with a page in front, the Code
    /// drawer closed.
    pub fn panel_page(&self, project: ProjectId) -> Option<PaneId> {
        if self.drawer_shown(project) {
            return None;
        }
        self.panel_of(project)?.shown()
    }

    /// A new page in `project`'s panel at `url` ("" for none yet), for an agent (`owner`) or
    /// the user; beside the page in front. Not shown by this.
    pub fn add_page(&mut self, project: ProjectId, url: &str, owner: Option<PaneId>) -> PaneId {
        let id = self.new_id();
        let mut b = crate::browser::Browser::new(id, project, url);
        b.owner = owner;
        self.browsers.insert(id, b);
        if let Some(panel) = self.panel_mut(project) {
            let at = panel.front.and_then(|f| panel.pages.iter().position(|x| *x == f)).map_or(panel.pages.len(), |i| i + 1);
            panel.pages.insert(at, id);
        }
        self.mark_dirty();
        id
    }

    /// Shows `project`'s panel with `page` in front; the Code drawer (and Review in it) makes
    /// way.
    pub fn show_page(&mut self, project: ProjectId, page: PaneId) {
        if let Some(panel) = self.panel_mut(project) {
            panel.front = Some(page);
            panel.open = true;
        }
        self.hide_drawer(project);
        self.mark_dirty();
    }

    /// The Code drawer of `project` closes (the panel takes its place).
    fn hide_drawer(&mut self, project: ProjectId) {
        if let Some(e) = self.editors.get_mut(&project) {
            e.visible = false;
        }
        if self.review.as_ref().is_some_and(|r| r.project == project && !r.expanded) {
            self.review = None;
        }
    }

    /// Hides `project`'s panel; its pages keep running (an agent's too).
    pub fn hide_panel(&mut self, project: ProjectId) {
        let pages = self.panel_of(project).map(|p| p.pages.clone()).unwrap_or_default();
        if self.browser_focus.is_some_and(|f| pages.contains(&f)) {
            self.release_page_keyboard();
        }
        if let Some(panel) = self.panel_mut(project) {
            panel.open = false;
            panel.max = false;
        }
        self.focus_request = self.current_pane();
        self.mark_dirty();
    }

    /// Closes a page: its web view goes. An agent's next call opens it another.
    pub fn close_page(&mut self, id: PaneId) {
        let Some(project) = self.browsers.get(&id).map(|b| b.project) else { return };
        if let Some(panel) = self.panel_mut(project) {
            panel.remove(id);
        }
        self.drop_pane(id);
        if self.panel_of(project).is_some_and(|p| p.pages.is_empty()) {
            self.focus_request = self.current_pane();
        }
        self.mark_dirty();
    }

    /// The page in front follows the terminal the user picks: one whose agent has a page brings
    /// it to the front (once, as it is picked: a page chosen after stays).
    pub fn panel_follow(&mut self, project: ProjectId) {
        let Some(focused) = self.project(project).and_then(|p| p.current_pane()) else { return };
        if self.panel_of(project).is_none_or(|p| p.seen_focus == Some(focused)) {
            return;
        }
        let owned = self.panel_of(project).and_then(|p| p.pages.iter().copied().find(|id| self.browsers.get(id).is_some_and(|b| b.owner == Some(focused))));
        if let Some(panel) = self.panel_mut(project) {
            panel.seen_focus = Some(focused);
            if let Some(page) = owned {
                panel.front = Some(page);
            }
        }
    }

    /// Where Focus lays out `layout` (a tab of `project`) and the project's panel in `body` (the
    /// area beside the sidebar, the Code drawer taken out).
    pub fn focus_place(&self, project: ProjectId, layout: &tc_term::layout::Layout, body: Rect, cell: egui::Vec2) -> PanelPlace {
        let unplaced = PanelPlace::without(body);
        if self.drawer_shown(project) {
            return unplaced;
        }
        let Some(panel) = self.panel_of(project) else { return unplaced };
        crate::project::place_panel(body, panel, layout, |rects| {
            let single = rects.len() == 1;
            rects.iter().all(|(_, r)| {
                let g = tc_term::widget::grid_for(super::focus::term_rect(*r, single), cell);
                g.cols >= MIN_COLS && g.rows >= MIN_ROWS
            })
        })
    }

    /// The panel in Focus in `rect`: the strip of its pages, then the page in front (`float`:
    /// over the terminals).
    pub fn panel_view(&mut self, ui: &mut Ui, project: ProjectId, rect: Rect, float: bool) {
        let p = style::pal(ui);
        if float {
            let shadow = egui::epaint::Shadow { offset: [0, 6], blur: 22, spread: 0, color: egui::Color32::from_black_alpha(if ui.visuals().dark_mode { 120 } else { 40 }) };
            ui.painter().add(shadow.as_shape(rect, CornerRadius::ZERO));
        }
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.canvas);
        // Clicks on its empty parts stay with it (not the terminal under a floating panel).
        ui.interact(rect, egui::Id::new(("panel-back", project)), Sense::click());
        #[cfg(debug_assertions)]
        crate::devtest::record("panel", rect);
        let strip = Rect::from_min_size(rect.min, vec2(rect.width(), TABS_H));
        self.page_tabs(ui, project, strip);
        if let Some(front) = self.panel_of(project).and_then(Panel::shown) {
            let page = Rect::from_min_max(pos2(rect.left(), strip.bottom()), rect.max);
            if let Some(b) = self.browsers.get_mut(&front) {
                b.zoom_want = 1.0;
            }
            self.browser_pane(ui, front, page, false, true);
            if let Some(panel) = self.panel_mut(project) {
                panel.focus_width = page.width();
            }
        }
        if float {
            ui.painter().vline(rect.left(), rect.y_range(), Stroke::new(1.0, p.border_strong));
        }
        // Filling the window: Esc puts it back beside the terminals (unless a page has the keys).
        if self.panel_of(project).is_some_and(|x| x.max)
            && self.browser_focus.is_none()
            && ui.input(|i| i.key_pressed(egui::Key::Escape))
            && let Some(panel) = self.panel_mut(project)
        {
            panel.max = false;
        }
    }

    /// The edge between the terminals and the panel, dragged to size it (after the panes, so
    /// that a terminal beside it does not take the strip along it).
    pub fn panel_edge(&mut self, ui: &mut Ui, project: ProjectId, x: f32, full: Rect) {
        let p = style::pal(ui);
        let grab = Rect::from_min_max(pos2(x - super::focus::DIVIDER_GRAB, full.top()), pos2(x + super::focus::DIVIDER_GRAB, full.bottom()));
        let r = ui.interact(grab, egui::Id::new(("panel-edge", project)), Sense::drag());
        #[cfg(debug_assertions)]
        crate::devtest::record("panel-edge", grab);
        if r.hovered() || r.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if r.dragged()
            && let Some(pos) = r.interact_pointer_pos()
            && let Some(panel) = self.panel_mut(project)
        {
            panel.width = ((full.right() - pos.x) / full.width()).clamp(0.25, 0.75);
        }
        if r.drag_stopped() {
            self.mark_dirty();
        }
        let line = if r.hovered() || r.dragged() { p.accent } else { p.border };
        ui.painter().vline(x, full.y_range(), Stroke::new(1.0, line));
    }

    /// The strip at the top of the panel: a tab per page (its agent's mark, its title, a dot
    /// while an agent uses it or it asks the user something), then New Page, Fill the Window,
    /// Hide.
    fn page_tabs(&mut self, ui: &mut Ui, project: ProjectId, strip: Rect) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        ui.painter().rect_filled(strip, CornerRadius::ZERO, p.chrome);
        ui.painter().hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, p.border));
        let Some(panel) = self.panel_of(project).cloned() else { return };
        let cy = strip.center().y;
        let mut right = strip.right() - 6.0;
        let mut button = |ui: &mut Ui, icon: &str, tip: &str, on: bool, name: &str| {
            let r = Rect::from_center_size(pos2(right - 11.0, cy), vec2(22.0, 22.0));
            right -= 24.0;
            #[cfg(debug_assertions)]
            crate::devtest::record(name, r);
            let _ = name;
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::icon_button_ex(ui, icon, tip, on, 22.0)).inner.clicked()
        };
        let hide = button(ui, icons::X, &kit::tip(t("Hide Browser"), Action::ShowBrowser), false, "panel-hide");
        let fill_tip = if panel.max { t("Back beside the terminals") } else { t("Fill the window") };
        let max = button(ui, icons::ARROWS_OUT_SIMPLE, fill_tip, panel.max, "panel-max");
        let new = button(ui, icons::PLUS, &kit::tip(t("New Browser Page"), Action::NewBrowserTab), false, "panel-new");
        if hide {
            self.hide_panel(project);
        }
        if max && let Some(x) = self.panel_mut(project) {
            x.max = !x.max;
        }
        if new {
            self.actions.push(Action::NewBrowserTab);
        }

        // The tabs: as wide as there is room for (within bounds); too many, the page in front
        // and those before it that fit, the rest in ⋯.
        let left = strip.left() + 6.0;
        let room = (right - 6.0 - left).max(0.0);
        let n = panel.pages.len();
        let fits = |room: f32| (((room + TAB_GAP) / (TAB_MIN + TAB_GAP)).floor() as usize).max(1);
        let (shown, more): (Vec<PaneId>, Vec<PaneId>) = if fits(room) >= n {
            (panel.pages.clone(), Vec::new())
        } else {
            let k = fits(room - 30.0);
            let mut shown: Vec<PaneId> = panel.pages.iter().copied().take(k).collect();
            if let Some(f) = panel.front.filter(|f| !shown.contains(f)) {
                shown.pop();
                shown.push(f);
            }
            let more = panel.pages.iter().copied().filter(|id| !shown.contains(id)).collect();
            (shown, more)
        };
        let width = if shown.is_empty() { TAB_MIN } else { ((room - if more.is_empty() { 0.0 } else { 30.0 }) / shown.len() as f32 - TAB_GAP).clamp(TAB_MIN.min(room), TAB_MAX) };
        let marks = self.page_marks(&panel.pages);
        let mut x = left;
        let mut front = None;
        let mut close = None;
        for id in &shown {
            let r = Rect::from_min_size(pos2(x, cy - 12.0), vec2(width, 24.0));
            x += width + TAB_GAP;
            let mark = marks.get(id).map(String::as_str);
            let (clicked, closed) = self.page_tab(ui, *id, r, panel.front == Some(*id), mark, dark);
            if clicked {
                front = Some(*id);
            }
            if closed {
                close = Some(*id);
            }
        }
        if !more.is_empty() {
            let r = Rect::from_min_size(pos2(x, cy - 11.0), vec2(26.0, 22.0));
            let resp = ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::icon_button_ex(ui, icons::DOTS_THREE, &tf("{} more pages", &[&more.len().to_string()]), false, 22.0)).inner;
            #[cfg(debug_assertions)]
            crate::devtest::record("page-more", r);
            egui::Popup::menu(&resp).align(egui::RectAlign::BOTTOM_START).width(240.0).show(|ui| {
                kit::menu_body(ui);
                for id in &more {
                    let label = self.browsers.get(id).map(|b| b.label()).unwrap_or_default();
                    if kit::menu_item(ui, Some(icons::GLOBE), &label, None, false).clicked() {
                        front = Some(*id);
                        ui.close();
                    }
                }
            });
        }
        if let Some(id) = front
            && let Some(x) = self.panel_mut(project)
        {
            x.front = Some(id);
        }
        if let Some(id) = close {
            self.close_page(id);
        }
    }

    /// The mark of each agent's page: its agent's two letters, numbered when two agents of a
    /// kind have pages here ("CC1", "CC2", in the order of the pages).
    fn page_marks(&self, pages: &[PaneId]) -> std::collections::HashMap<PaneId, String> {
        let plain: Vec<(PaneId, &'static str)> = pages
            .iter()
            .filter_map(|id| {
                let owner = self.browsers.get(id)?.owner?;
                let kind = self.panes.get(&owner).and_then(|x| x.tracker.agent.or(x.last_agent));
                Some((*id, kind.map_or("AI", tc_core::agent::AgentKind::mark)))
            })
            .collect();
        let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        plain
            .iter()
            .map(|(id, mark)| {
                let twins = plain.iter().filter(|(_, m)| m == mark).count();
                let n = seen.entry(mark).or_insert(0);
                *n += 1;
                (*id, if twins > 1 { format!("{mark}{n}") } else { mark.to_string() })
            })
            .collect()
    }

    /// One page's tab (`mark`: its agent's, for an agent's page); returns (clicked, close asked).
    fn page_tab(&mut self, ui: &mut Ui, id: PaneId, r: Rect, front: bool, mark: Option<&str>, dark: bool) -> (bool, bool) {
        let p = style::pal(ui);
        let Some(b) = self.browsers.get(&id) else { return (false, false) };
        let resp = ui.interact(r, egui::Id::new(("page-tab", id)), Sense::click());
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("page-tab:{id}"), r);
        let hovered = resp.hovered();
        let painter = ui.painter();
        let radius = CornerRadius::same(style::RADIUS);
        if front {
            painter.rect_filled(r, radius, p.canvas);
            painter.rect_stroke(r, radius, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        } else if hovered {
            painter.rect_filled(r, radius, p.hover);
        }
        // Its agent (still in its terminal, or gone: then the page is the agent's last).
        let owner = b.owner.map(|o| (o, self.panes.get(&o)));
        let ended = owner.is_some_and(|(_, pane)| pane.is_none());
        let kind = owner.and_then(|(_, pane)| pane).and_then(|x| x.tracker.agent.or(x.last_agent));
        let who = kind.map(|k| k.label().to_string()).or_else(|| owner.and_then(|(_, pane)| pane).map(|x| x.label()));
        let text_color = if ended { p.text_faint } else if front || hovered { p.text } else { p.text_muted };
        let mut x = r.left() + 6.0;
        if owner.is_some() {
            let mark = mark.unwrap_or_else(|| kind.map_or("AI", tc_core::agent::AgentKind::mark));
            let ink = if ended { p.text_faint } else { p.text };
            match kind.filter(|k| super::agent_logo::has(*k)) {
                // Its agent's logo, and its number when two agents of a kind have pages here.
                Some(k) => {
                    let n = mark.strip_prefix(k.mark()).unwrap_or("");
                    let g = (!n.is_empty()).then(|| painter.layout_no_wrap(n.to_string(), style::semibold(9.5), ink));
                    let w = 11.0 + g.as_ref().map_or(0.0, |g| g.size().x + 2.0) + 5.0;
                    let m = Rect::from_min_size(pos2(x, r.center().y - 8.0), vec2(w.max(18.0), 16.0));
                    painter.rect_filled(m, CornerRadius::same(4), p.subtle);
                    let left = if g.is_some() { m.left() + 2.5 } else { m.center().x - 5.5 };
                    super::agent_logo::paint(painter, Rect::from_min_size(pos2(left, m.center().y - 5.5), vec2(11.0, 11.0)), k, p.text, ended);
                    if let Some(g) = g {
                        painter.galley(pos2(left + 13.0, m.center().y - g.size().y / 2.0), g, ink);
                    }
                    x = m.right() + 6.0;
                }
                None => {
                    let g = painter.layout_no_wrap(mark.to_string(), style::semibold(9.5), ink);
                    let m = Rect::from_min_size(pos2(x, r.center().y - 8.0), vec2(g.size().x.max(12.0) + 6.0, 16.0));
                    painter.rect_filled(m, CornerRadius::same(4), p.subtle);
                    painter.galley(pos2(m.center().x - g.size().x / 2.0, m.center().y - g.size().y / 2.0), g, p.text);
                    x = m.right() + 6.0;
                }
            }
        } else {
            let icon = if b.asleep && !b.url.is_empty() { icons::MOON } else { icons::GLOBE };
            painter.text(pos2(x + 6.5, r.center().y), Align2::CENTER_CENTER, icon, style::icon_font(13.0), p.text_faint);
            x += 19.0;
        }
        // The dot: it needs the user (a site to allow), or an agent works in it.
        let dot = if b.ask.is_some() || b.dialog.is_some() { Some(p.warn) } else if b.driven() { Some(p.info) } else { None };
        let show_x = hovered || front;
        let mut text_right = r.right() - 6.0;
        let x_rect = Rect::from_center_size(pos2(r.right() - 12.0, r.center().y), vec2(16.0, 16.0));
        if show_x {
            text_right = x_rect.left() - 2.0;
        }
        if let Some(c) = dot {
            painter.circle_filled(pos2(text_right - 3.0, r.center().y), 3.0, c);
            text_right -= 10.0;
        }
        let mut job = egui::text::LayoutJob::simple_singleline(b.label(), egui::FontId::proportional(12.0), text_color);
        job.wrap = egui::text::TextWrapping::truncate_at_width((text_right - x).max(8.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(x, r.center().y - g.size().y / 2.0), g, text_color);
        let mut closed = false;
        if show_x {
            let xr = ui.interact(x_rect, egui::Id::new(("page-close", id)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("page-close:{id}"), x_rect);
            if xr.hovered() {
                ui.painter().rect_filled(x_rect, CornerRadius::same(4), if front { p.hover } else { p.subtle });
            }
            ui.painter().text(x_rect.center(), Align2::CENTER_CENTER, icons::X, style::icon_font(11.0), if xr.hovered() { p.text } else { p.text_faint });
            closed = xr.on_hover_text(t("Close Page")).clicked();
        }
        let (host, path) = crate::browser::split_url(&b.url);
        let mut tip = b.label();
        if !host.is_empty() {
            tip = format!("{tip}\n{host}{path}");
        }
        if let Some(who) = &who {
            tip = format!("{tip}\n{}", if ended { tf("{}’s page (its terminal is closed)", &[who]) } else { tf("{}’s page", &[who]) });
        }
        if let Some(d) = b.driver.as_ref().filter(|d| !d.paused) {
            tip = format!("{tip}\n{} · {}", d.who, d.doing);
        }
        let _ = dark;
        let resp = resp.on_hover_text(tip);
        if resp.middle_clicked() {
            closed = true;
        }
        (resp.clicked() && !closed, closed)
    }

    /// The page a tile shows beside its terminals, and its zoom: when its project's panel shows
    /// in Focus and the tile (`body_w` × `body_h` inside) has room for it.
    pub fn tile_panel(&self, project: ProjectId, body_w: f32, body_h: f32) -> Option<(PaneId, f32)> {
        if !tile_fits_panel(body_w, body_h) {
            return None;
        }
        let page = self.panel_page(project)?;
        let panel = self.panel_of(project)?;
        // Settings: pages fit the tile (laid out at its width) instead.
        if !self.settings.browser_grid_zoom {
            return Some((page, 1.0));
        }
        let w = (body_w * panel.share()).round();
        // As wide as in Focus, smaller: the layout stays the same.
        let reference = if panel.focus_width > 0.0 { panel.focus_width } else { self.last_full_body.map_or(1280.0, |b| b.width()) * panel.share() };
        Some((page, (w / reference.max(1.0)).clamp(MIN_ZOOM, 1.0)))
    }
}

/// A tile (its body `w` × `h`) has room for its project's Browser panel.
pub fn tile_fits_panel(w: f32, h: f32) -> bool {
    w >= TILE_PANEL_MIN_W && h >= TILE_PANEL_MIN_H
}
