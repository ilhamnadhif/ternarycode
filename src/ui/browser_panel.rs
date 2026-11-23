//! The Browser panel: a project's pages at the right of its terminals in Focus, like the Code
//! drawer (the two take turns there). Each agent has a page of its own in it and so does the
//! user, under a strip of their tabs; the page in front follows the terminal the user picks.
//! Terminals are never squeezed for it: side by side they show stacked while it is open, or
//! it floats over them. A Grid tile big enough shows it too, its page zoomed out so that it
//! keeps the layout it has in Focus.

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use tc_core::agent::AgentKind;
use tc_core::{PaneId, ProjectId};

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};
use crate::project::{Panel, PanelPlace};

/// The strip of page tabs at the top of the panel.
pub const TABS_H: f32 = 40.0;
/// The least a terminal gets beside the panel (else they show stacked, or it floats).
pub const MIN_COLS: usize = 60;
pub const MIN_ROWS: usize = 12;
/// A tile shows its project's panel from this width of its body on.
const TILE_PANEL_MIN_W: f32 = 560.0;
const TILE_PANEL_MIN_H: f32 = 200.0;
/// The least a page in a tile is zoomed to (smaller text is not read).
const MIN_ZOOM: f32 = 0.5;
/// A page tab: its height, the least and most it is wide, the room between tabs, inside its
/// sides and between what it shows.
const TAB_H: f32 = 28.0;
const TAB_MIN: f32 = 96.0;
const TAB_MAX: f32 = 220.0;
const TAB_GAP: f32 = 2.0;
const TAB_PAD: f32 = 9.0;
const TAB_GAP_IN: f32 = 6.0;
/// New Page (and ⋯) after the tabs.
const NEW_W: f32 = 24.0;

impl App {
    pub fn panel_of(&self, project: ProjectId) -> Option<&Panel> {
        self.project(project).map(|p| &p.panel)
    }

    pub fn panel_mut(&mut self, project: ProjectId) -> Option<&mut Panel> {
        self.project_mut(project).map(|p| &mut p.panel)
    }

    /// The Code drawer shows for the project (the panel waits behind it).
    pub fn drawer_shown(&self, project: ProjectId) -> bool {
        self.zen_mode
            || self.review.as_ref().is_some_and(|r| r.project == project && !r.expanded)
            || self.history.as_ref().is_some_and(|h| h.project == project)
            || self.editors.get(&project).is_some_and(|e| e.visible)
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
        self.exit_zen_mode();
        self.drawer_maximized = false;
        self.drawer_tab = tc_core::model::DrawerTab::Terminals;
        self.settings.drawer_open = true;
        if let Some(panel) = self.panel_mut(project) {
            panel.front = Some(page);
            panel.open = true;
        }
        self.hide_drawer(project);
        self.mark_dirty();
    }

    /// The Code drawer of `project` closes (the panel takes its place).
    pub(crate) fn hide_drawer(&mut self, project: ProjectId) {
        if let Some(e) = self.editors.get_mut(&project) {
            e.visible = false;
        }
        if self.review.as_ref().is_some_and(|r| r.project == project && !r.expanded) {
            self.review = None;
        }
        if self.history.as_ref().is_some_and(|h| h.project == project) {
            self.history = None;
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

    /// Closes a page: its web view goes (Reopen Closed Page brings it back). An agent's next
    /// call opens it another.
    pub fn close_page(&mut self, id: PaneId) {
        let Some(b) = self.browsers.get(&id) else { return };
        let project = b.project;
        if !b.url.is_empty() && b.url != "about:blank" {
            self.closed_pages.push(crate::browser::ClosedPage { project, url: b.url.clone(), title: b.title.clone() });
            if self.closed_pages.len() > crate::browser::CLOSED_PAGES {
                self.closed_pages.remove(0);
            }
        }
        self.agent_pages.retain(|_, page| *page != id);
        if let Some(panel) = self.panel_mut(project) {
            panel.remove(id);
        }
        self.drop_pane(id);
        if self.panel_of(project).is_some_and(|p| p.pages.is_empty()) {
            self.focus_request = self.current_pane();
        }
        self.mark_dirty();
    }

    /// Opens again the page closed last (of the project in view, else of any): a page of the
    /// user's, in front.
    pub fn reopen_closed_page(&mut self) {
        let at = self.closed_pages.iter().rposition(|c| Some(c.project) == self.active).or_else(|| self.closed_pages.iter().rposition(|c| self.project(c.project).is_some()));
        let Some(closed) = at.map(|i| self.closed_pages.remove(i)) else {
            self.toast(t("No closed page to reopen").to_string());
            return;
        };
        let id = self.add_page(closed.project, &closed.url, None);
        if let Some(b) = self.browsers.get_mut(&id) {
            b.title = closed.title;
        }
        self.active = Some(closed.project);
        self.show_page(closed.project, id);
        self.show_focus();
    }

    /// An agent's terminal closed: its pages out of view and silent close with it; one in
    /// view, or playing sound, stays as the user's (the music goes on).
    pub fn agent_pane_closed(&mut self, pane: PaneId) {
        self.agent_pages.remove(&pane);
        self.agent_page_sessions.remove(&pane);
        let owned: Vec<PaneId> = self.browsers.values().filter(|b| b.owner == Some(pane)).map(|b| b.id).collect();
        for id in owned {
            let Some((project, keep)) = self.browsers.get(&id).map(|b| (b.project, b.shown || b.busy())) else { continue };
            // Its project going away takes its pages anyway.
            if self.project(project).is_none() {
                continue;
            }
            if keep {
                if let Some(b) = self.browsers.get_mut(&id) {
                    b.owner = None;
                }
            } else {
                self.close_page(id);
            }
        }
    }

    /// Whether memory runs low (asked of the system at most every half minute; tests can say
    /// so).
    pub fn memory_pressure(&mut self, now: std::time::Instant) -> bool {
        #[cfg(debug_assertions)]
        if std::env::var_os("TERNARYCODE_MEMORY_LOW").is_some() {
            return true;
        }
        match self.mem_low {
            Some((at, low)) if now.duration_since(at) < std::time::Duration::from_secs(30) => low,
            _ => {
                let low = tc_agent::proc::memory_low();
                self.mem_low = Some((now, low));
                low
            }
        }
    }

    /// The page in front follows the terminal the user picks: one whose agent has a page brings
    /// it to the front (once, as it is picked: a page chosen after stays).
    pub fn panel_follow(&mut self, project: ProjectId) {
        let Some(focused) = self.project(project).and_then(|p| p.current_pane()) else { return };
        if self.panel_of(project).is_none_or(|p| p.seen_focus == Some(focused)) {
            return;
        }
        // The page its agent acts on (its own, or one of the user's it uses), else its own.
        let current = self.agent_pages.get(&focused).copied().filter(|id| self.panel_of(project).is_some_and(|p| p.pages.contains(id)));
        let owned = current.or_else(|| self.panel_of(project).and_then(|p| p.pages.iter().copied().find(|id| self.browsers.get(id).is_some_and(|b| b.owner == Some(focused)))));
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

    /// The Browser panel from its strip of page tabs down, in `rect` (no sheet, no header): the
    /// tabs and New Page, then the page in front with its toolbar, its notices, its frame and
    /// its console.
    pub fn browser_panel_body(&mut self, ui: &mut Ui, project: ProjectId, rect: Rect) {
        self.panel_body(ui, project, rect, 0.0);
    }

    /// [`App::browser_panel_body`], `reserve` points kept free at the strip's right.
    fn panel_body(&mut self, ui: &mut Ui, project: ProjectId, rect: Rect, reserve: f32) {
        let p = style::pal(ui);
        let strip = Rect::from_min_size(rect.min, vec2(rect.width(), TABS_H));
        self.page_tabs(ui, project, strip, reserve);
        ui.painter().hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, p.border));
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
        // Filling the window: Esc puts it back beside the terminals (unless a page has the keys).
        if self.panel_of(project).is_some_and(|x| x.max)
            && self.browser_focus.is_none()
            && self.terminals_take_keys()
            && ui.input(|i| i.key_pressed(egui::Key::Escape))
            && let Some(panel) = self.panel_mut(project)
        {
            panel.max = false;
        }
    }

    /// The edge between the terminals and the panel, dragged to size it (after the panes, so
    /// that a terminal beside it does not take the strip along it).
    pub fn panel_edge(&mut self, ui: &mut Ui, project: ProjectId, x: f32, full: Rect, float: bool) {
        let p = style::pal(ui);
        // Beside the terminals the edge is the gap before the panel; floating over them, its
        // own left side.
        let mid = if float { x } else { x - style::GAP / 2.0 };
        let grab = Rect::from_min_max(pos2(mid - super::focus::DIVIDER_GRAB, full.top()), pos2(mid + super::focus::DIVIDER_GRAB, full.bottom()));
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
            panel.width = ((full.right() - pos.x) / full.width().max(1.0)).clamp(0.25, 0.75);
        }
        if r.drag_stopped() {
            self.mark_dirty();
        }
        // The gap before the panel is the edge: the accent shows on it while it is grabbed.
        if r.hovered() || r.dragged() {
            ui.painter().rect_filled(Rect::from_center_size(pos2(mid, full.center().y), vec2(2.0, full.height() - 16.0)), CornerRadius::same(1), p.accent);
        }
    }

    /// The strip at the top of the panel: a tab per page (its agent's avatar, its title, a dot
    /// while an agent uses it or it asks the user something), then New Page. Tabs are as wide
    /// as what they show; too many, they share the room, then the page in front and those
    /// before it that fit stay, the rest in ⋯.
    fn page_tabs(&mut self, ui: &mut Ui, project: ProjectId, strip: Rect, reserve: f32) {
        let Some(panel) = self.panel_of(project).cloned() else { return };
        let cy = strip.center().y;
        let left = strip.left() + 8.0;
        let right = strip.right() - 8.0 - reserve;
        let room = (right - left - TAB_GAP - NEW_W).max(0.0);
        let marks = self.page_marks(&panel.pages);
        let looks: std::collections::HashMap<PaneId, TabLook> =
            panel.pages.iter().filter_map(|id| Some((*id, self.tab_look(*id, marks.get(id).map(String::as_str))?))).collect();
        let natural: Vec<f32> = panel.pages.iter().map(|id| looks.get(id).map_or(TAB_MIN, |l| tab_natural(ui, l, panel.front == Some(*id)))).collect();
        let n = panel.pages.len();
        let gaps = |k: usize| TAB_GAP * k.saturating_sub(1) as f32;
        let (shown, widths, more): (Vec<PaneId>, Vec<f32>, Vec<PaneId>) = if natural.iter().sum::<f32>() + gaps(n) <= room {
            (panel.pages.clone(), natural, Vec::new())
        } else if n as f32 * TAB_MIN + gaps(n) <= room {
            let w = ((room - gaps(n)) / n.max(1) as f32).min(TAB_MAX);
            (panel.pages.clone(), vec![w; n], Vec::new())
        } else {
            // The ⋯ button takes a place too.
            let room = room - TAB_GAP - NEW_W;
            let k = (((room + TAB_GAP) / (TAB_MIN + TAB_GAP)).floor() as usize).max(1);
            let mut shown: Vec<PaneId> = panel.pages.iter().copied().take(k).collect();
            if let Some(f) = panel.front.filter(|f| !shown.contains(f)) {
                shown.pop();
                shown.push(f);
            }
            let more = panel.pages.iter().copied().filter(|id| !shown.contains(id)).collect();
            let w = ((room - gaps(shown.len())) / shown.len().max(1) as f32).clamp(24.0_f32.min(room.max(0.0)), TAB_MAX);
            let widths = vec![w; shown.len()];
            (shown, widths, more)
        };
        let mut x = left;
        let mut front = None;
        let mut close = None;
        for (id, w) in shown.iter().zip(&widths) {
            let r = Rect::from_min_size(pos2(x, cy - TAB_H / 2.0), vec2(*w, TAB_H));
            x += w + TAB_GAP;
            let Some(look) = looks.get(id) else { continue };
            let (clicked, closed) = self.page_tab(ui, *id, r, panel.front == Some(*id), look);
            if clicked {
                front = Some(*id);
            }
            if closed {
                close = Some(*id);
            }
        }
        if !more.is_empty() {
            let r = Rect::from_min_size(pos2(x, cy - NEW_W / 2.0), Vec2::splat(NEW_W));
            x += NEW_W + TAB_GAP;
            let resp = ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::icon_button_ex(ui, icons::DOTS_THREE, &tf("{} more pages", &[&more.len().to_string()]), false, NEW_W)).inner;
            #[cfg(debug_assertions)]
            crate::devtest::record("page-more", r);
            egui::Popup::menu(&resp).align(egui::RectAlign::BOTTOM_START).width(240.0).show(|ui| {
                kit::menu_body(ui);
                for id in &more {
                    let Some(look) = looks.get(id) else { continue };
                    let icon = if look.asleep { icons::MOON } else { icons::GLOBE_SIMPLE };
                    if kit::menu_item(ui, Some(icon), &look.title, None, false).clicked() {
                        front = Some(*id);
                        ui.close();
                    }
                }
            });
        }
        let nr = Rect::from_min_size(pos2(x, cy - NEW_W / 2.0), Vec2::splat(NEW_W));
        #[cfg(debug_assertions)]
        crate::devtest::record("panel-new", nr);
        if ui.scope_builder(egui::UiBuilder::new().max_rect(nr), |ui| kit::icon_button_ex(ui, icons::PLUS, &kit::tip(t("New Browser Page"), Action::NewBrowserTab), false, NEW_W)).inner.clicked() {
            self.actions.push(Action::NewBrowserTab);
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

    /// What a page's tab shows (`mark`: its agent's, for an agent's page).
    fn tab_look(&self, id: PaneId, mark: Option<&str>) -> Option<TabLook> {
        let p_kind = |pane: PaneId| self.panes.get(&pane).and_then(|x| x.tracker.agent.or(x.last_agent));
        let b = self.browsers.get(&id)?;
        // Its agent (still in its terminal, or gone: then the page is the agent's last).
        let agent = b.owner.map(|o| {
            let ended = !self.panes.contains_key(&o);
            let kind = p_kind(o);
            let mark = mark.map_or_else(|| kind.map_or("AI", tc_core::agent::AgentKind::mark).to_string(), str::to_string);
            let letters = kind.map_or("AI", tc_core::agent::AgentKind::mark);
            let number = mark.strip_prefix(letters).unwrap_or("").to_string();
            TabAgent { kind, letters, number, ended }
        });
        // An agent at work in a page of the user's: its avatar beside the user's globe.
        let borrowed = match (&agent, b.driver.as_ref()) {
            (None, Some(d)) if !d.paused => {
                let kind = p_kind(d.pane);
                Some(TabAgent { kind, letters: kind.map_or("AI", tc_core::agent::AgentKind::mark), number: String::new(), ended: false })
            }
            _ => None,
        };
        Some(TabLook {
            agent,
            borrowed,
            asleep: b.asleep && !b.url.is_empty(),
            speaker: (b.state.audible || b.muted).then_some(b.muted),
            dot: if b.ask.is_some() || b.dialog.is_some() { Some(Dot::Asks) } else if b.driven() { Some(Dot::Agent) } else { None },
            title: b.label(),
        })
    }

    /// One page's tab; returns (clicked, close asked).
    fn page_tab(&mut self, ui: &mut Ui, id: PaneId, r: Rect, front: bool, look: &TabLook) -> (bool, bool) {
        let p = style::pal(ui);
        let Some(b) = self.browsers.get(&id) else { return (false, false) };
        let resp = ui.interact(r, egui::Id::new(("page-tab", id)), Sense::click());
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("page-tab:{id}"), r);
        let hovered = resp.hovered();
        if hovered && !front {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let painter = ui.painter_at(r);
        let radius = CornerRadius::same(style::RADIUS_ROW);
        if front {
            painter.rect(r, radius, p.surface, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        } else if hovered {
            painter.rect_filled(r, radius, p.hover);
        }
        let ended = look.agent.as_ref().is_some_and(|a| a.ended);
        let cy = r.center().y;
        let mut x = r.left() + TAB_PAD;
        // The lead: the agent's avatar (its page), the user's globe (or the moon: asleep) with
        // the avatar of an agent at work in it; the speaker while it plays sound.
        match &look.agent {
            Some(a) => {
                paint_avatar(&painter, Rect::from_min_size(pos2(x, cy - 8.0), Vec2::splat(16.0)), a, &p);
                x += 16.0;
                if !a.number.is_empty() {
                    let g = painter.layout_no_wrap(a.number.clone(), style::mono_semibold(9.5), p.text_faint);
                    painter.galley(pos2(x + 2.0, cy - g.size().y / 2.0), g.clone(), p.text_faint);
                    x += 2.0 + g.size().x;
                }
            }
            None => {
                let icon = if look.asleep { icons::MOON } else { icons::GLOBE_SIMPLE };
                painter.text(pos2(x + 6.5, cy), Align2::CENTER_CENTER, icon, style::icon_font(13.0), p.text_faint);
                x += 13.0;
                if let Some(a) = &look.borrowed {
                    paint_avatar(&painter, Rect::from_min_size(pos2(x + 3.0, cy - 8.0), Vec2::splat(16.0)), a, &p);
                    x += 3.0 + 16.0;
                }
            }
        }
        let speaker = look.speaker.map(|_| {
            let sr = Rect::from_center_size(pos2(x + 4.0 + 6.0, cy), vec2(16.0, 16.0));
            x += 4.0 + 12.0;
            sr
        });
        x += TAB_GAP_IN;
        let show_x = hovered || front;
        let x_rect = Rect::from_center_size(pos2(r.right() - TAB_PAD - 5.0, cy), vec2(16.0, 16.0));
        let mut text_right = if show_x { x_rect.center().x - 5.0 - TAB_GAP_IN } else { r.right() - TAB_PAD };
        // The dot: it needs the user (a site to allow, a dialog), or an agent works in it.
        if let Some(dot) = look.dot {
            let c = match dot {
                Dot::Asks => p.warn,
                Dot::Agent => p.info,
            };
            painter.circle_filled(pos2(text_right - 3.0, cy), 3.0, c);
            text_right -= 6.0 + TAB_GAP_IN;
        }
        let text_color = if ended { p.text_faint } else if front || hovered { p.text } else { p.text_muted };
        let font = if front { style::medium(style::SMALL) } else { egui::FontId::proportional(style::SMALL) };
        let mut job = egui::text::LayoutJob::simple_singleline(look.title.clone(), font, text_color);
        job.wrap = egui::text::TextWrapping::truncate_at_width((text_right - x).max(8.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, text_color);
        let mut toggled = false;
        if let (Some(sr), Some(muted)) = (speaker, look.speaker) {
            let mr = ui.interact(sr, egui::Id::new(("page-mute", id)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("page-mute:{id}"), sr);
            if mr.hovered() {
                painter.rect_filled(sr, CornerRadius::same(4), p.subtle);
            }
            let icon = if muted { icons::SPEAKER_SLASH } else { icons::SPEAKER_HIGH };
            let ink = if mr.hovered() { p.text } else if muted { p.text_faint } else { p.text_muted };
            painter.text(sr.center(), Align2::CENTER_CENTER, icon, style::icon_font(12.0), ink);
            toggled = mr.on_hover_text(if muted { t("Unmute Page") } else { t("Mute Page") }).clicked();
        }
        let mut closed = false;
        if show_x {
            let xr = ui.interact(x_rect, egui::Id::new(("page-close", id)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("page-close:{id}"), x_rect);
            if xr.hovered() {
                painter.rect_filled(x_rect, CornerRadius::same(4), p.subtle);
            }
            painter.text(x_rect.center(), Align2::CENTER_CENTER, icons::X, style::icon_font(10.0), if xr.hovered() { p.text } else { p.text_faint });
            closed = xr.on_hover_text(t("Close page")).clicked();
        }
        let who = look.agent.as_ref().and_then(|a| {
            let pane = b.owner.and_then(|o| self.panes.get(&o));
            a.kind.map(|k| k.label().to_string()).or_else(|| pane.map(|x| x.label()))
        });
        let (host, path) = crate::browser::split_url(&b.url);
        let mut tip = look.title.clone();
        if !host.is_empty() {
            tip = format!("{tip}\n{host}{path}");
        }
        if let Some(who) = &who {
            tip = format!("{tip}\n{}", if ended { tf("{}’s page (its terminal is closed)", &[who]) } else { tf("{}’s page", &[who]) });
        }
        if let Some(d) = b.driver.as_ref().filter(|d| !d.paused) {
            tip = format!("{tip}\n{} · {}", d.who, d.doing);
        }
        if b.state.audible && !b.muted {
            tip = format!("{tip}\n{}", t("Playing sound"));
        }
        let resp = resp.on_hover_text(tip);
        if resp.middle_clicked() {
            closed = true;
        }
        if toggled && let Some(b) = self.browsers.get_mut(&id) {
            let on = !b.muted;
            b.set_muted(on);
        }
        (resp.clicked() && !closed && !toggled, closed)
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

/// An agent in a page's tab: its kind (when known), its letters (without a logo), its number
/// when two agents of a kind have pages here, and whether its terminal is gone.
struct TabAgent {
    kind: Option<AgentKind>,
    letters: &'static str,
    number: String,
    ended: bool,
}

/// The dot of a page's tab.
#[derive(Clone, Copy)]
enum Dot {
    /// It needs the user (a site to allow, a dialog): amber.
    Asks,
    /// An agent works in it: blue.
    Agent,
}

/// What a page's tab shows.
struct TabLook {
    /// The page is an agent's.
    agent: Option<TabAgent>,
    /// An agent works in this page of the user's.
    borrowed: Option<TabAgent>,
    asleep: bool,
    /// It plays sound (`false`) or is muted (`true`).
    speaker: Option<bool>,
    dot: Option<Dot>,
    title: String,
}

/// How wide a tab is for what it shows (`front`: its title is stronger and it has ×).
fn tab_natural(ui: &Ui, l: &TabLook, front: bool) -> f32 {
    let w = |t: &str, f: egui::FontId| ui.painter().layout_no_wrap(t.to_string(), f, egui::Color32::PLACEHOLDER).size().x;
    let mut lead = match &l.agent {
        Some(a) => 16.0 + if a.number.is_empty() { 0.0 } else { 2.0 + w(&a.number, style::mono_semibold(9.5)) },
        None => 13.0 + if l.borrowed.is_some() { 3.0 + 16.0 } else { 0.0 },
    };
    if l.speaker.is_some() {
        lead += 4.0 + 12.0;
    }
    let font = if front { style::medium(style::SMALL) } else { egui::FontId::proportional(style::SMALL) };
    let fixed = TAB_PAD + lead + TAB_GAP_IN + if l.dot.is_some() { TAB_GAP_IN + 6.0 } else { 0.0 } + if front { TAB_GAP_IN + 10.0 } else { 0.0 } + TAB_PAD;
    (fixed + w(&l.title, font).max(24.0)).min(TAB_MAX)
}

/// An agent's avatar (16 square): its logo on the active fill, else its letters.
fn paint_avatar(painter: &egui::Painter, rect: Rect, a: &TabAgent, p: &style::Palette) {
    painter.rect_filled(rect, CornerRadius::same(4), p.subtle);
    let logo = Rect::from_center_size(rect.center(), Vec2::splat((rect.width() * 0.68).round()));
    if !a.kind.is_some_and(|k| super::agent_logo::paint(painter, logo, k, p.text, a.ended)) {
        let ink = if a.ended { p.text_faint } else { p.text_muted };
        painter.text(rect.center(), Align2::CENTER_CENTER, a.letters, style::mono_semibold(8.0), ink);
    }
}
