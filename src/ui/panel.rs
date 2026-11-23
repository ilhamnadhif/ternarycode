//! The right panel of the Focus view (Slate): one place beside the terminals for Changes,
//! Files, History and the Browser, switched from the project bar above, with a quiet header
//! holding the active view's title, Expand (⇧⌘E) and Close.
//! Terminals stay the stage; the panel never pushes them into a drawer. It is as wide as the
//! project's panel share (dragged at its left edge), or fills the body when expanded.

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::ProjectId;

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::t;

/// Height of the panel's header (aligned with the terminal tab strip beside it).
pub const HEAD_H: f32 = 40.0;
/// The least the panel is wide, and the least the terminals keep beside it.
const MIN_W: f32 = 440.0;
const MIN_STAGE: f32 = 360.0;
/// The least Files is wide: its tree and the code beside it.
const FILES_MIN_W: f32 = 600.0;

/// What the right panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelTab {
    Changes,
    Files,
    History,
    Browser,
}

impl PanelTab {
    pub const ALL: [PanelTab; 4] = [PanelTab::Changes, PanelTab::Files, PanelTab::History, PanelTab::Browser];

    pub fn icon(self) -> &'static str {
        match self {
            PanelTab::Changes => icons::GIT_DIFF,
            PanelTab::Files => icons::FOLDER_SIMPLE,
            PanelTab::History => icons::CLOCK_COUNTER_CLOCKWISE,
            PanelTab::Browser => icons::GLOBE_SIMPLE,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PanelTab::Changes => t("Changes"),
            PanelTab::Files => t("Files"),
            PanelTab::History => t("History"),
            PanelTab::Browser => t("Browser"),
        }
    }

    pub fn action(self) -> Action {
        match self {
            PanelTab::Changes => Action::Review,
            PanelTab::Files => Action::ToggleEditor,
            PanelTab::History => Action::History,
            PanelTab::Browser => Action::ShowBrowser,
        }
    }
}

impl App {
    /// What the right panel of `project` shows now, if it is open: Changes, then History, then
    /// Files take the place before the Browser (which waits behind them).
    pub fn panel_tab(&self, project: ProjectId) -> Option<PanelTab> {
        if self.review.as_ref().is_some_and(|r| r.project == project && !r.expanded) {
            return Some(PanelTab::Changes);
        }
        if self.history.as_ref().is_some_and(|h| h.project == project) {
            return Some(PanelTab::History);
        }
        if self.editors.get(&project).is_some_and(|e| e.visible) {
            return Some(PanelTab::Files);
        }
        self.panel_of(project).filter(|p| p.open && !p.pages.is_empty()).map(|_| PanelTab::Browser)
    }

    /// Whether the panel of `project` fills the body (the terminals out of view).
    pub fn panel_expanded(&self, project: ProjectId) -> bool {
        self.panel_tab(project).is_some() && self.panel_of(project).is_some_and(|p| p.max)
    }

    /// Shows `tab` in the panel of the project in view (or closes the panel when it shows it
    /// already and `toggle`).
    pub fn switch_panel(&mut self, tab: PanelTab, toggle: bool) {
        let Some(pid) = self.active else { return };
        if toggle && self.panel_tab(pid) == Some(tab) {
            // Changes, Files or History put away: the Browser waiting behind them comes back.
            if tab != PanelTab::Browser && self.panel_of(pid).is_some_and(|p| p.open && !p.pages.is_empty()) {
                self.hide_drawer(pid);
                self.focus_request = self.current_pane();
                self.mark_dirty();
            } else {
                self.close_panel(pid);
            }
            return;
        }
        if tab != PanelTab::Browser {
            self.release_page_keyboard();
        }
        match tab {
            PanelTab::Browser => {
                self.hide_drawer(pid);
                self.show_browser(false);
            }
            PanelTab::Files => {
                self.review = None;
                self.history = None;
                self.ensure_editor_for_project(pid, 0.0);
                if let Some(e) = self.editors.get_mut(&pid) {
                    e.visible = true;
                }
                self.show_focus();
            }
            PanelTab::Changes => {
                if self.panel_tab(pid) != Some(PanelTab::Changes) {
                    self.history = None;
                    self.open_review(None);
                }
            }
            PanelTab::History => {
                if self.panel_tab(pid) != Some(PanelTab::History) {
                    self.open_history();
                }
            }
        }
        self.mark_dirty();
    }

    /// Closes the panel of `project` (its pages keep running).
    pub fn close_panel(&mut self, project: ProjectId) {
        self.hide_drawer(project);
        self.hide_panel(project);
        if let Some(panel) = self.panel_mut(project) {
            panel.max = false;
        }
        self.focus_request = self.current_pane();
        self.mark_dirty();
    }

    /// ⇧⌘E: the panel fills the body, or goes back beside the terminals.
    pub fn toggle_panel_expanded(&mut self) {
        let Some(pid) = self.active else { return };
        if self.panel_tab(pid).is_none() {
            self.switch_panel(PanelTab::Changes, false);
        }
        if let Some(panel) = self.panel_mut(pid) {
            panel.max = !panel.max;
        }
        self.mark_dirty();
    }

    /// Splits the Focus body into the terminals' stage and the panel (None when closed); the
    /// stage is None when the panel fills the body.
    pub fn panel_split(&self, project: ProjectId, body: Rect) -> (Option<Rect>, Option<(Rect, PanelTab)>) {
        let Some(tab) = self.panel_tab(project) else { return (Some(body), None) };
        if self.panel_expanded(project) || self.zen_mode {
            return if self.zen_mode { (Some(body), None) } else { (None, Some((body, tab))) };
        }
        let share = self.panel_of(project).map_or(crate::project::PANEL_SHARE, |p| p.share()).clamp(0.25, 0.75);
        let max_w = (body.width() - MIN_STAGE - style::GAP).max(MIN_W.min(body.width() * 0.5));
        // Files keeps room for its tree beside the code.
        let least = if tab == PanelTab::Files { FILES_MIN_W } else { MIN_W };
        let w = (body.width() * share).max(least).min(max_w.max(MIN_W)).round();
        let panel = Rect::from_min_max(pos2(body.right() - w, body.top()), body.max);
        let stage = Rect::from_min_max(body.min, pos2(panel.left() - style::GAP, body.bottom()));
        (Some(stage), Some((panel, tab)))
    }

    /// The panel in `rect`: its header, then what `tab` shows. Browser puts its page tabs in the
    /// header row itself (with Expand and Close at the right) so the panel never stacks two chrome
    /// bars above the address bar.
    pub fn right_panel(&mut self, ui: &mut Ui, project: ProjectId, rect: Rect, tab: PanelTab) {
        let p = style::pal(ui);
        super::focus::paint_sheet(ui.painter(), rect, &p, super::focus::SheetLook::Plain);
        #[cfg(debug_assertions)]
        crate::devtest::record("panel", rect);
        if tab == PanelTab::Browser {
            let body = Rect::from_min_max(pos2(rect.left() + 1.0, rect.top() + 1.0), pos2(rect.right() - 1.0, rect.bottom() - 1.0));
            let strip = Rect::from_min_size(body.min, vec2(body.width(), super::browser_panel::TABS_H));
            let right_left = self.panel_head_controls(ui, project, strip, 640.0);
            if self.panel_tab(project) != Some(PanelTab::Browser) {
                return;
            }
            let reserve = (strip.right() - right_left).max(0.0);
            self.browser_panel_body(ui, project, body, reserve);
            return;
        }
        let head = Rect::from_min_size(rect.min, vec2(rect.width(), HEAD_H));
        self.panel_head(ui, project, head, tab);
        let body = Rect::from_min_max(pos2(rect.left() + 1.0, head.bottom()), pos2(rect.right() - 1.0, rect.bottom() - 1.0));
        ui.painter().hline(body.x_range(), body.top() + 0.5, Stroke::new(1.0, p.border));
        let body = Rect::from_min_max(pos2(body.left(), body.top() + 1.0), body.max);
        // The tab may have changed in the header: draw what is current now.
        let Some(tab) = self.panel_tab(project) else { return };
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            ui.set_clip_rect(body.intersect(ui.clip_rect()));
            match tab {
                PanelTab::Changes => self.review_drawer(ui),
                PanelTab::History => self.history_drawer(ui),
                PanelTab::Files => {
                    let root = self.project(project).map(|x| x.path.clone()).unwrap_or_default();
                    self.code_drawer(ui, project, root);
                }
                PanelTab::Browser => {}
            }
        });
    }

    /// Right-side panel header controls (expanded hint, Expand / Back beside the terminals, Close),
    /// returning the leftmost x they occupy.
    fn panel_head_controls(&mut self, ui: &mut Ui, project: ProjectId, rect: Rect, hint_min_w: f32) -> f32 {
        let p = style::pal(ui);
        let expanded = self.panel_expanded(project);
        let inner = Rect::from_min_max(pos2(rect.left() + 8.0, rect.top()), pos2(rect.right() - 8.0, rect.bottom()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let close = kit::icon_button_ex(ui, icons::X, t("Close Panel"), false, 24.0);
            #[cfg(debug_assertions)]
            crate::devtest::record("panel-close", close.rect);
            if close.clicked() {
                self.close_panel(project);
            }
            let (icon, tip) = if expanded {
                (icons::ARROWS_IN_SIMPLE, t("Back beside the terminals"))
            } else {
                (icons::ARROWS_OUT_SIMPLE, t("Expand"))
            };
            let ex = kit::icon_button_ex(ui, icon, &kit::tip(tip, Action::TogglePanelExpanded), false, 24.0);
            #[cfg(debug_assertions)]
            crate::devtest::record("panel-expand", ex.rect);
            if ex.clicked() {
                self.actions.push(Action::TogglePanelExpanded);
            }
            if expanded && rect.width() > hint_min_w {
                let keys = kit::keys(&Action::TogglePanelExpanded).unwrap_or_default();
                ui.label(egui::RichText::new(format!("{keys} {}", t("back beside the terminals"))).font(egui::FontId::proportional(style::SMALL)).color(p.text_faint));
            }
            ui.min_rect().left()
        })
        .inner
    }

    /// The header: the active panel's title (with its count when relevant), then at the right a
    /// note while it fills the body, Expand / Back beside the terminals, and Close. Switching
    /// between panels lives solely in the project toolbar above (no duplicate tab buttons here).
    fn panel_head(&mut self, ui: &mut Ui, project: ProjectId, rect: Rect, tab: PanelTab) {
        let p = style::pal(ui);
        let changed = self.project(project).and_then(|x| x.git.as_ref()).map_or(0, |g| g.changed);
        let inner = Rect::from_min_max(pos2(rect.left() + 8.0, rect.top()), pos2(rect.right() - 8.0, rect.bottom()));
        // Right side first.
        let right_left = self.panel_head_controls(ui, project, rect, 520.0);
        let title_rect = Rect::from_min_max(inner.min, pos2((right_left - 8.0).max(inner.left()), inner.bottom()));
        let painter = ui.painter().with_clip_rect(title_rect.intersect(ui.clip_rect()));
        let cy = title_rect.center().y;
        let mut x = title_rect.left() + 6.0;
        painter.text(pos2(x + 7.0, cy), Align2::CENTER_CENTER, tab.icon(), style::icon_font(14.0), p.text_muted);
        x += 14.0 + 7.0;
        let g = painter.layout_no_wrap(tab.label().to_string(), style::medium(style::BODY), p.text);
        let gw = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
        x += gw;
        let count = match tab {
            PanelTab::Changes if changed > 0 => Some(changed),
            _ => None,
        };
        if let Some(n) = count {
            painter.text(pos2(x + 7.0, cy), Align2::LEFT_CENTER, n.to_string(), style::mono(style::DATA), p.text_faint);
        }
    }

    /// The edge between the terminals and the panel, dragged to size it.
    pub fn panel_resize_edge(&mut self, ui: &mut Ui, project: ProjectId, panel: Rect, body: Rect) {
        let p = style::pal(ui);
        let mid = panel.left() - style::GAP / 2.0;
        let grab = Rect::from_min_max(pos2(mid - super::focus::DIVIDER_GRAB, body.top()), pos2(mid + super::focus::DIVIDER_GRAB, body.bottom()));
        let r = ui.interact(grab, egui::Id::new(("panel-edge", project)), Sense::drag());
        #[cfg(debug_assertions)]
        crate::devtest::record("panel-edge", grab);
        if r.hovered() || r.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            ui.painter().rect_filled(Rect::from_center_size(pos2(mid, body.center().y), vec2(2.0, body.height() - 16.0)), CornerRadius::same(1), p.accent);
        }
        if r.dragged()
            && let Some(pos) = r.interact_pointer_pos()
            && let Some(panel) = self.panel_mut(project)
        {
            panel.width = ((body.right() - pos.x) / body.width().max(1.0)).clamp(0.25, 0.75);
        }
        if r.drag_stopped() {
            self.mark_dirty();
        }
    }
}
