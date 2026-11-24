//! The browser pane: its toolbar (back, forward, reload, the address, the console, ⋯) over its
//! page, with what asks about the page as notices between them; opening pages in it; and
//! keeping each page's web view where its pane is drawn, hidden when the pane is out of view or
//! something is drawn over it.

use std::time::Duration;

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::PaneId;

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::browser::{Answer, Ask, Browser, Dialog, DialogKind, Driver, Event, PageFind};
use crate::i18n::{t, tf};

/// The toolbar at the top of a browser pane (its header, also in a split).
pub const BAR_H: f32 = 44.0;
/// How round a page's frame is on macOS (in the Browser panel and in a Grid tile).
pub const PAGE_ROUND: f32 = 10.0;
/// Corner radius of the frame directly enclosing the native web view: rounded on macOS (where
/// `CALayer` clips `WKWebView` with anti-aliased corners), 0 on Windows and Linux (where native
/// child web views — WebView2 and WebKitGTK X11 — are strictly rectangular so a flush 0-radius
/// border aligns pixel-for-pixel on every edge and corner).
pub const WEB_FRAME_ROUND: f32 = if cfg!(target_os = "macos") { PAGE_ROUND } else { 0.0 };

/// Snaps `rect` to exact physical pixel boundaries so the border painted by egui and the native
/// web view placed inside it share the exact same pixel edges at every DPI scale (100%–250%).
pub(super) fn snap_rect_px(rect: Rect, ppp: f32) -> Rect {
    let ppp = ppp.max(0.25);
    let snap = |v: f32| (v * ppp).round() / ppp;
    let min = pos2(snap(rect.left()), snap(rect.top()));
    let max = pos2(snap(rect.right()).max(min.x), snap(rect.bottom()).max(min.y));
    Rect::from_min_max(min, max)
}

/// Snaps a border stroke thickness (in points) to an exact whole number of physical pixels
/// (at least 1 px) so `frame.shrink(stroke)` lands cleanly on integer physical pixels.
pub(super) fn snap_stroke_pt(width_pt: f32, ppp: f32) -> f32 {
    let ppp = ppp.max(0.25);
    (width_pt * ppp).round().max(1.0) / ppp
}
/// The room around the page's frame, its notices and its console.
const EDGE: f32 = 8.0;
/// A find bar's height (pages and documents).
pub const FIND_H: f32 = 36.0;
/// The toolbar's small icon buttons (back, forward, reload, ⋯) and chips.
const ICON_S: f32 = 24.0;
/// What the toolbar keeps between its controls.
const BAR_GAP: f32 = 4.0;
/// A console line's height.
const CONSOLE_ROW: f32 = 26.0;
/// The console drawer's header.
const CONSOLE_HEAD: f32 = 36.0;

impl App {
    /// What a pane is called (a tab, a tile): a terminal's label, or a browser's page.
    pub fn leaf_label(&self, id: PaneId) -> String {
        match (self.panes.get(&id), self.browsers.get(&id)) {
            (Some(p), _) => p.name(),
            (None, Some(b)) => b.label(),
            _ => String::new(),
        }
    }

    /// A browser pane in `rect`: its toolbar, the notices about the page (an agent using it, a
    /// site to allow, its dialog, the find bar), then its page in a rounded frame (the web view
    /// is laid over it once the frame is drawn; meanwhile, or asleep, a placeholder) and its
    /// console under it.
    pub fn browser_pane(&mut self, ui: &mut Ui, id: PaneId, rect: Rect, focused: bool, single: bool) {
        let p = style::pal(ui);
        let Some(b) = self.browsers.get(&id) else { return };
        let project = b.project;
        let driver = b.driver.clone();
        let ask = b.ask.clone();
        let dialog = b.dialog.clone();
        let url = b.url.clone();
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), BAR_H));
        let _ = single;
        // A click on the bar makes the pane the focused one (⌘W closes it, splits go beside it).
        let bar_resp = ui.interact(bar, egui::Id::new(("browser-bar", id)), Sense::click());
        if bar_resp.clicked() && !focused {
            self.actions.push(Action::FocusPane(id));
        }
        // The terminal whose server this page comes from (same port on this machine).
        let (host, _) = crate::browser::split_url(&url);
        let port = crate::browser::is_local_host(&host)
            .then(|| host.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok()))
            .flatten();
        let from = port.and_then(|port| self.project_servers(project).into_iter().find(|(_, s)| s.port == port)).map(|(pane, _)| pane);
        if let Some(pane) = from
            && let Some(b) = self.browsers.get_mut(&id)
        {
            b.served_by = Some(pane);
        }
        let moved = self.page_toolbar(ui, id, bar, &bar_resp);
        // The user moving the page while an agent uses it takes it over.
        if moved && driver.as_ref().is_some_and(|d| !d.paused) {
            self.take_over_browser(id, true);
        }

        // Above the page, one notice each: who uses it, a site to allow, its dialog, then the
        // find bar.
        let mut top = bar.bottom();
        let at = |top: f32| pos2(rect.left() + EDGE, top);
        let width = (rect.width() - 2.0 * EDGE).max(40.0);
        if let Some(d) = &driver {
            top += self.agent_notice(ui, id, at(top), width, d) + EDGE;
        }
        if let Some(a) = &ask {
            let who = driver.as_ref().map_or_else(|| t("The agent").to_string(), |d| d.who.clone());
            let name = self.project(project).map(|x| x.name.clone()).unwrap_or_default();
            top += self.ask_notice(ui, id, at(top), width, a, &who, &name) + EDGE;
        }
        // A dialog of the page (alert, confirm, prompt): the page waits for its answer.
        if let Some(d) = &dialog {
            top += self.dialog_notice(ui, id, at(top), width, d) + EDGE;
        }
        // The find bar (⌘F while the page has the keyboard).
        if let Some(f) = self.browsers.get(&id).and_then(|b| b.find.clone()) {
            top += self.page_find_bar(ui, id, at(top), width, &f) + EDGE;
        }
        // The console's drawer at the foot of the pane, a card under the page's.
        let mut bottom = rect.bottom() - EDGE;
        if self.browsers.get(&id).is_some_and(|b| b.console_open) {
            let h = (rect.height() * 0.32).clamp(110.0, 260.0).min((bottom - top - 60.0 - EDGE).max(0.0));
            if h >= 60.0 {
                let r = Rect::from_min_max(pos2(rect.left() + EDGE, bottom - h), pos2(rect.right() - EDGE, bottom));
                self.console_drawer(ui, id, r);
                bottom = r.top() - EDGE;
            }
        }
        // The page: inset in the sheet, snapped to exact physical pixels, a crisp border round it;
        // ringed in the accent with a soft halo while an agent has it (the web view sits flush
        // inside on all four edges on every OS).
        let ppp = ui.ctx().pixels_per_point();
        let raw_frame = Rect::from_min_max(pos2(rect.left() + EDGE, top.min(bottom)), pos2(rect.right() - EDGE, bottom.max(top.min(bottom))));
        let frame = snap_rect_px(raw_frame, ppp);
        let agent_on = driver.as_ref().is_some_and(|d| !d.paused);
        let round = CornerRadius::same(WEB_FRAME_ROUND as u8);
        let border_w = snap_stroke_pt(if agent_on { 2.0 } else { 1.0 }, ppp);
        if agent_on {
            ui.painter().rect_stroke(frame, round, Stroke::new(3.0, style::soft(p.accent, 0.2)), egui::StrokeKind::Outside);
        }
        ui.painter().rect(frame, round, p.canvas, Stroke::new(border_w, if agent_on { p.accent } else { p.border }), egui::StrokeKind::Inside);
        let page = frame.shrink(border_w);
        // Its server does not answer: which terminal served it, and Reload.
        if self.browsers.get(&id).is_some_and(|b| b.down) {
            self.down_card(ui, id, page, &host);
        } else if let Some(b) = self.browsers.get(&id)
            && b.covered_since.is_some()
            && !b.shown
            && let Some((tex, _)) = &b.still
        {
            // Covered (the palette, a dialog, a menu): the page's picture in its place.
            let size = tex.size_vec2();
            let h = page.width() * size.y / size.x.max(1.0);
            let shown = Rect::from_min_size(page.min, vec2(page.width(), h));
            ui.painter().with_clip_rect(page).image(tex.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else if !self.browsers.get(&id).is_some_and(Browser::live) {
            self.page_placeholder(ui, page, &url, &host);
        }
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser:{id}"), page);
        // `EDGE` (8 pt) already keeps the frame clear of the panel's divider grab strip. Inside
        // the frame, the web view fills `page` flush on all four edges; on macOS its corners are
        // rounded to `WEB_FRAME_ROUND - border_w` via `CALayer`, while on Windows and Linux
        // `WEB_FRAME_ROUND` is 0 so the border matches the rectangular child web view exactly.
        if let Some(b) = self.browsers.get_mut(&id) {
            b.corner_want = (WEB_FRAME_ROUND - border_w).max(0.0);
        }
        self.browser_pages.insert(id, page);
        if self.focus_request == Some(id) {
            self.focus_request = None;
            self.browser_focus_request = Some(id);
        }
    }

    /// The page's toolbar in `bar`: back, forward, reload, the address (Enter goes there), the
    /// console's chip while it has errors or warnings, what the page downloaded, its zoom when
    /// not 100%, and ⋯ (also a right-click on the bar). Returns whether the user moved the page.
    fn page_toolbar(&mut self, ui: &mut Ui, id: PaneId, bar: Rect, bar_resp: &egui::Response) -> bool {
        let p = style::pal(ui);
        let Some(b) = self.browsers.get(&id) else { return false };
        let url = b.url.clone();
        let loading = b.loading;
        let errors = b.console.iter().filter(|l| l.is_error()).count();
        let warnings = b.console.iter().filter(|l| l.is_warning()).count();
        let console_open = b.console_open;
        let user_zoom = b.user_zoom;
        let (dl_count, dl_running, dl_failed) = (b.downloads.len(), b.downloads.iter().any(|d| d.done.is_none()), b.downloads.last().is_some_and(|d| d.done == Some(false)));
        let cy = bar.center().y;
        let inner = bar.shrink2(vec2(EDGE, 0.0));
        // Right side first: what is left goes to the address, which keeps its room in a narrow
        // pane (the chips go first, the zoom's, then the downloads', then the console's).
        const MIN_FIELD: f32 = 90.0;
        let field_left = inner.left() + 3.0 * ICON_S + 3.0 * BAR_GAP;
        let mut right = inner.right();
        let more_r = Rect::from_min_max(pos2(right - ICON_S, cy - ICON_S / 2.0), pos2(right, cy + ICON_S / 2.0));
        right = more_r.left() - BAR_GAP;
        let mut chip = |w: f32| {
            (right - w - BAR_GAP - field_left >= MIN_FIELD).then(|| {
                let r = Rect::from_min_max(pos2(right - w, cy - ICON_S / 2.0), pos2(right, cy + ICON_S / 2.0));
                right = r.left() - BAR_GAP;
                r
            })
        };
        let count_w = |n: usize| ui.painter().layout_no_wrap(n.to_string(), style::mono(style::DATA), p.text).size().x;
        // The console's warnings and errors, when there are any (a click opens its drawer).
        let issues = errors + warnings;
        let cons_rect = (issues > 0 || console_open).then(|| chip(7.0 + 12.0 + if issues > 0 { 5.0 + count_w(issues) } else { 0.0 } + 7.0)).flatten();
        // What the page downloaded (a click lists it): blue while one runs, red when the last
        // failed, green when all came.
        let dl_rect = (dl_count > 0).then(|| chip(7.0 + 12.0 + 5.0 + count_w(dl_count) + 7.0)).flatten();
        // The page's zoom, when it is not 100% (a click: back to 100%).
        let zoom_text = format!("{:.0}%", user_zoom * 100.0);
        let zoom_w = ui.painter().layout_no_wrap(zoom_text.clone(), style::mono(style::DATA), p.text).size().x;
        let zoom_rect = ((user_zoom - 1.0).abs() > 0.001).then(|| chip(7.0 + zoom_w + 7.0)).flatten();

        let mut x = inner.left();
        let mut nav = |icon: &str, tip: &str| {
            let r = Rect::from_min_size(pos2(x, cy - ICON_S / 2.0), vec2(ICON_S, ICON_S));
            x += ICON_S + BAR_GAP;
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::icon_button_ex(ui, icon, tip, false, ICON_S)).inner.clicked()
        };
        let back = nav(icons::ARROW_LEFT, t("Back"));
        let forward = nav(icons::ARROW_RIGHT, t("Forward"));
        let reload = nav(icons::ARROW_CLOCKWISE, t("Reload"));
        let field = Rect::from_min_max(pos2(x, cy - 15.0), pos2(right.max(x + 60.0), cy + 15.0));
        let fid = egui::Id::new(("browser-address", id));
        let typing = ui.memory(|m| m.has_focus(fid));
        // How the address is typed: without http://, with https:// (Enter keeps the scheme).
        let editable = |url: &str| url.trim_end_matches('/').trim_start_matches("http://").to_string();
        // Asked for (⌘L, the Browser button, a new empty browser): the address as typed, all of
        // it selected, so that what is typed replaces it.
        if self.browser_edit == Some(id) {
            self.browser_edit = None;
            let shown = editable(&url);
            kit::select_text(ui.ctx(), fid, 0, shown.chars().count());
            if let Some(b) = self.browsers.get_mut(&id) {
                b.address = shown;
            }
            ui.memory_mut(|m| m.request_focus(fid));
        }
        let typing = typing || ui.memory(|m| m.has_focus(fid));
        // At rest the address shows without its scheme (the lock says https).
        let mut text = match self.browsers.get(&id) {
            Some(b) if typing => b.address.clone(),
            Some(_) => editable(&url).trim_start_matches("https://").to_string(),
            None => String::new(),
        };
        let (host, _) = crate::browser::split_url(&url);
        let icon = if url.starts_with("https://") { icons::LOCK_SIMPLE } else { icons::GLOBE_SIMPLE };
        let insecure = url.starts_with("http://") && !crate::browser::is_local_host(&host);
        let (resp, well_clicked) = address_field(ui, fid, &mut text, icon, field, typing, insecure);
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser-address:{id}"), field);
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if loading {
            // A 2 pt segment of the accent runs along the field's foot while the page loads.
            let w = 120.0_f32.min(field.width() * 0.4);
            let t0 = ui.input(|i| i.time) as f32;
            let x0 = field.left() + (t0 * 420.0) % (field.width() + w) - w;
            let seg = Rect::from_min_max(pos2(x0, field.bottom() - 3.0), pos2(x0 + w, field.bottom() - 1.0));
            ui.painter().with_clip_rect(field.shrink2(vec2(6.0, 0.0))).rect_filled(seg, CornerRadius::same(1), p.accent);
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
        let more = ui.scope_builder(egui::UiBuilder::new().max_rect(more_r), |ui| kit::icon_button_ex(ui, icons::DOTS_THREE, t("More"), false, ICON_S)).inner;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser-more:{id}"), more.rect);
        egui::Popup::menu(&more).align(egui::RectAlign::BOTTOM_END).width(250.0).show(|ui| self.browser_menu(ui, id));
        bar_resp.context_menu(|ui| self.browser_menu(ui, id));
        // The chips.
        if let Some(r) = cons_rect {
            let resp = ui.interact(r, egui::Id::new(("browser-console", id)), Sense::click());
            let (icon, c) = if errors > 0 {
                (icons::WARNING_CIRCLE, p.err)
            } else if warnings > 0 {
                (icons::WARNING, p.warn)
            } else {
                (icons::TERMINAL_WINDOW, p.text_muted)
            };
            paint_chip(ui, r, resp.hovered() || console_open, icon, issues > 0, c, (issues > 0).then(|| issues.to_string()).as_deref());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("browser-console:{id}"), r);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t("Console")).clicked()
                && let Some(b) = self.browsers.get_mut(&id)
            {
                b.console_open = !b.console_open;
            }
        }
        // The list of downloads opens under its chip (or ⋯: from the menu).
        let dl_pop = egui::Id::new(("browser-downloads-list", id));
        let mut anchor = more.clone();
        if let Some(r) = dl_rect {
            let resp = ui.interact(r, egui::Id::new(("browser-downloads", id)), Sense::click());
            let c = if dl_running { p.info } else if dl_failed { p.err } else { p.ok };
            paint_chip(ui, r, resp.hovered(), icons::TRAY_ARROW_DOWN, false, c, Some(&dl_count.to_string()));
            if dl_running {
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("browser-downloads:{id}"), r);
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t("Downloads"));
            if resp.clicked() {
                egui::Popup::toggle_id(ui.ctx(), dl_pop);
            }
            anchor = resp;
        }
        if ui.data_mut(|d| d.remove_temp::<bool>(dl_pop.with("ask")).unwrap_or(false)) {
            egui::Popup::open_id(ui.ctx(), dl_pop);
        }
        let frame = egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(14)).corner_radius(CornerRadius::same(style::RADIUS_LG));
        egui::Popup::from_response(&anchor)
            .id(dl_pop)
            .open_memory(None)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .align(egui::RectAlign::BOTTOM_END)
            .gap(4.0)
            .frame(frame)
            .show(|ui| self.downloads_list(ui, id));
        if let Some(r) = zoom_rect {
            let resp = ui.interact(r, egui::Id::new(("browser-zoom", id)), Sense::click());
            let fill = if resp.hovered() { p.hover } else { Color32::TRANSPARENT };
            ui.painter().rect(r, CornerRadius::same(style::RADIUS), fill, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, zoom_text, style::mono(style::DATA), if resp.hovered() { p.text } else { p.text_muted });
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("browser-zoom:{id}"), r);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t("Actual size")).clicked()
                && let Some(b) = self.browsers.get_mut(&id)
            {
                b.user_zoom = 1.0;
            }
        }
        let mut navigated = false;
        if let Some(b) = self.browsers.get_mut(&id) {
            if (resp.gained_focus() || well_clicked) && !typing {
                // A click in the field: the whole address, as typed, selected.
                b.address = editable(&url);
                kit::select_text(ui.ctx(), fid, 0, b.address.chars().count());
            } else if typing || resp.changed() {
                b.address = text;
            }
            if enter {
                let address = b.address.clone();
                b.navigate(&address);
                navigated = true;
            }
            if back {
                b.back();
            }
            if forward {
                b.forward();
            }
            if reload {
                b.reload();
            }
        }
        if navigated {
            self.mark_dirty();
        }
        navigated || back || forward || reload
    }

    /// The page's server (on this machine) does not answer: said in the page's place, with the
    /// terminal that served it (Show Terminal) and Reload. The page comes back by itself.
    fn down_card(&mut self, ui: &mut Ui, id: PaneId, page: Rect, host: &str) {
        let served = self.browsers.get(&id).and_then(|b| b.served_by).filter(|pane| self.panes.contains_key(pane));
        let why = match served.and_then(|pane| self.panes.get(&pane)) {
            Some(pane) => match pane.tracker.status {
                tc_core::status::Status::Failed(code) => tf("The pane that served it, {}, exited with code {}.", &[&pane.label(), &code.to_string()]),
                _ => tf("The pane that served it, {}, is not serving it now.", &[&pane.label()]),
            },
            None => t("Nothing is serving it on this machine.").to_string(),
        };
        let body = format!("{why} {}", t("The page loads again once it answers."));
        let mut go = false;
        let mut reload = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(page), |ui| {
            ui.set_clip_rect(page.intersect(ui.clip_rect()));
            kit::empty_state(ui, icons::PLUGS, &tf("{} isn’t answering", &[host]), &body, |ui| {
                let go_w = if served.is_some() { kit::button_width(ui, Some(icons::TERMINAL_WINDOW), t("Show terminal"), None, true) + 8.0 } else { 0.0 };
                let w = go_w + kit::button_width(ui, Some(icons::ARROW_CLOCKWISE), t("Reload"), None, true);
                ui.allocate_ui_with_layout(vec2(w, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if served.is_some() {
                        go = kit::button_ex(ui, kit::Kind::Secondary, Some(icons::TERMINAL_WINDOW), t("Show terminal"), true).clicked();
                    }
                    reload = kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROW_CLOCKWISE), t("Reload"), true).clicked();
                });
            });
        });
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser-down:{id}"), page);
        if go && let Some(pane) = served {
            self.actions.push(Action::FocusPane(pane));
        }
        if reload && let Some(b) = self.browsers.get_mut(&id) {
            b.reload();
        }
    }

    /// In the frame of a page with no web view: no address yet, pages that can't show here, or
    /// the page loading (asleep, or not built yet).
    fn page_placeholder(&mut self, ui: &mut Ui, page: Rect, url: &str, host: &str) {
        let p = style::pal(ui);
        if !crate::browser::supported() || url.is_empty() {
            ui.scope_builder(egui::UiBuilder::new().max_rect(page), |ui| {
                ui.set_clip_rect(page.intersect(ui.clip_rect()));
                if !crate::browser::supported() {
                    kit::empty_state(ui, icons::GLOBE_SIMPLE, t("Pages can't show in TernaryCode here"), t("They open in your default browser."), |ui| {
                        if !url.is_empty() && kit::button_ex(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), t("Open in default browser"), true).clicked() {
                            crate::platform::open_url(url);
                        }
                    });
                } else {
                    kit::empty_state(ui, icons::GLOBE_SIMPLE, t("No page yet"), t("Type an address above, or a port like 5173."), |ui| {
                        if let Some(k) = kit::keys(&Action::BrowserAddress) {
                            kit::kbd(ui, k);
                        }
                    });
                }
            });
            return;
        }
        let c = page.center();
        let g = ui.painter().layout_no_wrap(tf("Loading {}…", &[host]), egui::FontId::proportional(style::SMALL), p.text_muted);
        let w = 16.0 + 8.0 + g.size().x;
        let x = c.x - w / 2.0;
        super::status::paint_spinner(ui.painter(), pos2(x + 8.0, c.y), 16.0, p.text_faint);
        ui.painter().galley(pos2(x + 24.0, c.y - g.size().y / 2.0), g, p.text_muted);
    }

    /// The page's console, a card under the page: its counts, All or only the errors, Clear and
    /// Close; the lines (errors first, then the rest as they came), one each.
    fn console_drawer(&mut self, ui: &mut Ui, id: PaneId, r: Rect) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let Some(b) = self.browsers.get(&id) else { return };
        let errors = b.console.iter().filter(|l| l.is_error()).count();
        let warnings = b.console.iter().filter(|l| l.is_warning()).count();
        let filter_id = egui::Id::new(("browser-console-errors", id));
        let only_errors = ui.data(|d| d.get_temp::<bool>(filter_id)).unwrap_or(false);
        let mut lines: Vec<crate::browser::ConsoleLine> = b.console.iter().filter(|l| l.is_error()).cloned().collect();
        if !only_errors {
            lines.extend(b.console.iter().filter(|l| !l.is_error()).cloned());
        }
        let painter = ui.painter_at(r.expand(1.0));
        painter.rect(r, CornerRadius::same(WEB_FRAME_ROUND as u8), p.canvas, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        let head = Rect::from_min_size(r.min, vec2(r.width(), CONSOLE_HEAD));
        painter.hline(head.x_range().shrink(1.0), head.bottom() - 0.5, Stroke::new(1.0, p.border));
        let cy = head.center().y;
        let mut x = head.left() + 12.0;
        painter.text(pos2(x + 6.5, cy), Align2::CENTER_CENTER, icons::TERMINAL_WINDOW, style::icon_font(13.0), p.text_muted);
        x += 13.0 + 6.0;
        let g = painter.layout_no_wrap(t("Console").to_string(), style::medium(style::SMALL), p.text);
        let gw = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
        x += gw + 6.0;
        // The counts: red for errors, amber for warnings.
        for (n, c) in [(errors, p.err), (warnings, p.warn)] {
            if n > 0 {
                let g = painter.layout_no_wrap(n.to_string(), style::mono(style::DATA), c);
                let pill = Rect::from_min_size(pos2(x, cy - 8.0), vec2((g.size().x + 10.0).max(16.0), 16.0));
                painter.rect_filled(pill, CornerRadius::same(8), style::tint(c, dark));
                painter.galley(pill.center() - g.size() / 2.0, g, c);
                x += pill.width() + 6.0;
            }
        }
        // Close, Clear, then All / Errors, at the right.
        let close_r = Rect::from_min_size(pos2(head.right() - 8.0 - ICON_S, cy - ICON_S / 2.0), vec2(ICON_S, ICON_S));
        let clear_r = close_r.translate(vec2(-(ICON_S + 6.0), 0.0));
        let mut clear = false;
        let mut close = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(clear_r), |ui| clear = kit::icon_button_ex(ui, icons::TRASH, t("Clear"), false, ICON_S).clicked());
        ui.scope_builder(egui::UiBuilder::new().max_rect(close_r), |ui| close = kit::icon_button_ex(ui, icons::X, t("Close"), false, ICON_S).clicked());
        let mut seg_right = clear_r.left() - 6.0;
        let mut pick = None;
        for (i, label) in [(1, t("Errors")), (0, t("All"))] {
            let w = segment_width(ui, None, label);
            if seg_right - w < x + 8.0 {
                break;
            }
            let sr = Rect::from_min_size(pos2(seg_right - w, cy - 12.0), vec2(w, 24.0));
            seg_right = sr.left() - 2.0;
            if segment(ui, sr, filter_id.with(i), None, label, only_errors == (i == 1)).clicked() {
                pick = Some(i == 1);
            }
        }
        if let Some(e) = pick {
            ui.data_mut(|d| d.insert_temp(filter_id, e));
        }
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser-console-drawer:{id}"), r);
        // The lines.
        let list = Rect::from_min_max(pos2(r.left() + 1.0, head.bottom()), pos2(r.right() - 1.0, r.bottom() - 4.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            ui.set_clip_rect(list.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical().id_salt(("browser-console-lines", id)).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if lines.is_empty() {
                    let empty = if only_errors { t("No errors in the console.") } else { t("Nothing in the console yet.") };
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), CONSOLE_ROW + 6.0), Sense::hover());
                    ui.painter().text(pos2(row.left() + 12.0, row.center().y), Align2::LEFT_CENTER, empty, egui::FontId::proportional(style::SMALL), p.text_faint);
                }
                for l in &lines {
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), CONSOLE_ROW), Sense::hover());
                    if !ui.is_rect_visible(row) {
                        continue;
                    }
                    let painter = ui.painter();
                    let level = match l.level.as_str() {
                        "error" => Some((icons::WARNING_CIRCLE, p.err)),
                        "warn" => Some((icons::WARNING, p.warn)),
                        _ => None,
                    };
                    if let Some((icon, c)) = level {
                        painter.text(pos2(row.left() + 12.0 + 6.0, row.center().y), Align2::CENTER_CENTER, icon, style::icon_fill(12.0), c);
                    }
                    let font = style::mono(style::DATA);
                    let src_w = if l.src.is_empty() || row.width() < 220.0 {
                        0.0
                    } else {
                        let max_src = (row.width() * 0.35).clamp(40.0, 220.0);
                        let mut sjob = egui::text::LayoutJob::simple_singleline(l.src.clone(), font.clone(), p.text_faint);
                        sjob.wrap = egui::text::TextWrapping::truncate_at_width(max_src);
                        let g = painter.layout_job(sjob);
                        let w = g.size().x;
                        painter.galley(pos2(row.right() - 12.0 - w, row.center().y - g.size().y / 2.0), g, p.text_faint);
                        w + 12.0
                    };
                    let text_x = row.left() + 12.0 + 12.0 + 8.0;
                    let mut job = egui::text::LayoutJob::simple_singleline(l.text.lines().next().unwrap_or("").to_string(), font, p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((row.right() - 12.0 - src_w - text_x).max(20.0));
                    let g = painter.layout_job(job);
                    painter.galley(pos2(text_x, row.center().y - g.size().y / 2.0), g, p.text);
                }
            });
        });
        if let Some(b) = self.browsers.get_mut(&id) {
            if clear {
                b.console.clear();
            }
            if close {
                b.console_open = false;
            }
        }
    }

    /// The notice of the agent using the page (what it does, Take Over); once the user took
    /// the page over, the quiet one with Hand Back. Returns its height.
    fn agent_notice(&mut self, ui: &mut Ui, id: PaneId, at: egui::Pos2, width: f32, d: &Driver) -> f32 {
        let rect = Rect::from_min_size(at, vec2(width, kit::NOTICE_H));
        let (tone, glyph, title, body) = if d.paused {
            (kit::Tone::Neutral, icons::HAND_POINTING, t("You have this page").to_string(), tf("{} waits until you hand it back", &[&d.who]))
        } else {
            let body = if d.doing.is_empty() { t("Click or type in the page to take over").to_string() } else { tf("{} · click or type in the page to take over", &[&d.doing]) };
            (kit::Tone::Info, icons::CURSOR_CLICK, tf("{} is using this page", &[&d.who]), body)
        };
        let primary = d.paused.then_some((t("Hand back"), Some("↵")));
        let secondary = (!d.paused).then_some(t("Take over"));
        // A click on the notice gives it the keyboard: Enter hands the page back.
        let keys = notice_keys(ui, rect, egui::Id::new(("browser-agent-keys", id)));
        let out = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| kit::notice(ui, width, tone, Some(glyph), &title, Some(&body), primary, secondary, false)).inner;
        #[cfg_attr(not(debug_assertions), allow(unused_variables))]
        let (pr, sr) = notice_buttons(ui, rect, primary, secondary);
        if let Some(r) = pr {
            ui.interact(r, egui::Id::new(("browser-hand-back-tip", id)), Sense::hover()).on_hover_text(tf("Hand back to {}", &[&d.who]));
        }
        #[cfg(debug_assertions)]
        {
            if let Some(r) = pr.or(sr) {
                crate::devtest::record(format!("browser-take-over:{id}"), r);
            }
            crate::devtest::record(format!("browser-agent:{id}"), rect);
        }
        let enter = d.paused && keys && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        if out.primary || out.secondary || enter {
            self.take_over_browser(id, !d.paused);
        }
        kit::NOTICE_H
    }

    /// The question about a site an agent's step would open: Allow Once (⌘: always for the
    /// project), Block. Returns its height.
    #[allow(clippy::too_many_arguments)]
    fn ask_notice(&mut self, ui: &mut Ui, id: PaneId, at: egui::Pos2, width: f32, a: &Ask, who: &str, project: &str) -> f32 {
        let p = style::pal(ui);
        let rect = Rect::from_min_size(at, vec2(width, kit::NOTICE_H));
        let host = match crate::browser::split_url(&a.url).0 {
            h if h.is_empty() => a.host.clone(),
            h => h,
        };
        let title = if a.click { tf("{}’s click goes to {}", &[who, &host]) } else { tf("{} wants to open {}", &[who, &host]) };
        let (cmd, always_keys) = if cfg!(target_os = "macos") { ("⌘", "⌘↵") } else { ("Ctrl", "Ctrl+Enter") };
        let primary = Some((t("Allow once"), Some("↵")));
        let secondary = Some(t("Block"));
        // Where it is too narrow for the whole line, its end ("⌘↵ always") stays.
        #[cfg_attr(not(debug_assertions), allow(unused_variables))]
        let (pr, sr) = notice_buttons(ui, rect, primary, secondary);
        let body = tf("{} · not on your list of sites · {} always", &[project, always_keys]);
        let room = sr.or(pr).map_or(rect.right() - 8.0, |b| b.left() - 6.0) - (rect.left() + 38.0) - 8.0;
        let body = if ui.painter().layout_no_wrap(body.clone(), style::mono(style::DATA), p.text).size().x <= room { body } else { tf("{} · {} always", &[project, always_keys]) };
        // A click on the notice gives it the keyboard: Enter allows once, ⌘↵ always, Esc blocks.
        let kid = egui::Id::new(("browser-ask-keys", id));
        let keys = notice_keys(ui, rect, kid);
        let out = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| kit::notice(ui, width, kit::Tone::Input, Some(icons::GLOBE_SIMPLE), &title, Some(&body), primary, secondary, false)).inner;
        if let Some(r) = pr {
            ui.interact(r, kid.with("tip"), Sense::hover()).on_hover_text(tf("Always for {}: {}-click or {}", &[project, cmd, always_keys]));
        }
        // "⌘↵ always" at the end of the line can be clicked too.
        let mut always = false;
        if let Some(link) = always_link(ui, rect, &title, &body, always_keys, sr.or(pr)) {
            let resp = ui.interact(link, kid.with("always"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tf("Always for {}", &[project]));
            if resp.hovered() {
                ui.painter().hline(link.x_range(), link.bottom() - 1.0, Stroke::new(1.0, p.text_muted));
            }
            always = resp.clicked();
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("browser-always:{id}"), link);
        }
        #[cfg(debug_assertions)]
        {
            if let Some(r) = pr {
                crate::devtest::record(format!("browser-allow:{id}"), r);
            }
            if let Some(r) = sr {
                crate::devtest::record(format!("browser-block:{id}"), r);
            }
            crate::devtest::record(format!("browser-ask:{id}"), rect);
        }
        let held = ui.input(|i| i.modifiers.command);
        let mut answer = None;
        if out.primary {
            answer = Some(if held { Answer::Always } else { Answer::Once });
        }
        if out.secondary {
            answer = Some(Answer::Block);
        }
        if always {
            answer = Some(Answer::Always);
        }
        if keys && !self.overlay_open() {
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter) {
                    answer = Some(Answer::Always);
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                    answer = Some(Answer::Once);
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    answer = Some(Answer::Block);
                }
            });
        }
        if let Some(ans) = answer {
            self.answer_site(id, ans);
        }
        kit::NOTICE_H
    }

    /// The find bar of a page, above it: the field (Enter: next, ⇧Enter: the one before, Esc:
    /// done), how many matches, ↑ ↓ and ×. Returns its height.
    fn page_find_bar(&mut self, ui: &mut Ui, id: PaneId, at: egui::Pos2, width: f32, f: &PageFind) -> f32 {
        let p = style::pal(ui);
        let fid = egui::Id::new(("browser-find", id));
        let count = match f.count {
            _ if f.query.is_empty() => String::new(),
            None => String::new(),
            Some(0) => t("No results").to_string(),
            Some(n) => tf("{} of {}", &[&f.index.to_string(), &n.to_string()]),
        };
        let mut query = f.query.clone();
        let out = find_card(
            ui,
            at,
            width,
            FindCard { id: fid, query: &mut query, hint: t("Find in page"), count, count_color: if f.count == Some(0) { p.err } else { p.text_muted }, field_w: 160.0, focus: f.focus, shadow: false },
            !self.overlay_open(),
        );
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser-find:{id}"), out.field);
        if f.focus
            && let Some(ff) = self.browsers.get_mut(&id).and_then(|b| b.find.as_mut())
        {
            ff.focus = false;
        }
        let mut step = out.step;
        if out.changed {
            if let Some(ff) = self.browsers.get_mut(&id).and_then(|b| b.find.as_mut()) {
                ff.query = query;
            }
            step = Some(0);
        }
        if out.close {
            self.close_page_find(id);
        } else if let Some(step) = step {
            self.find_in_page_step(id, step);
        }
        FIND_H
    }

    /// A key of the app's pressed while a page had the keyboard (Windows, Linux, where the page's
    /// window gets the keys): a browser's Ctrl+F, Ctrl+L, Ctrl+R and zoom keys act on the page;
    /// Ctrl+Shift keys are the app's, as anywhere in it.
    pub fn page_key(&mut self, id: PaneId, key: egui::Key, mods: egui::Modifiers) {
        use egui::Key;
        if mods.ctrl && !mods.shift && !mods.alt {
            match key {
                Key::F => self.open_page_find(id),
                Key::L => {
                    self.release_page_keyboard();
                    self.browser_edit = Some(id);
                }
                Key::R => {
                    if let Some(b) = self.browsers.get(&id) {
                        b.reload();
                    }
                }
                Key::Equals | Key::Plus => self.zoom_page(id, 1),
                Key::Minus => self.zoom_page(id, -1),
                Key::Num0 => self.zoom_page(id, 0),
                _ => {}
            }
            return;
        }
        if let Some(action) = crate::app::shortcut_for(mods, key) {
            self.actions.push(action);
        }
    }

    /// Opens a page's find bar (or gives it the keyboard again).
    pub fn open_page_find(&mut self, id: PaneId) {
        if let Some(b) = self.browsers.get_mut(&id) {
            let f = b.find.get_or_insert_with(PageFind::default);
            f.focus = true;
            if let Some(v) = &b.view {
                let _ = v.focus_parent();
            }
        }
        if self.browser_focus == Some(id) {
            self.browser_focus = None;
        }
    }

    /// Closes a page's find bar (its selection goes).
    pub fn close_page_find(&mut self, id: PaneId) {
        if let Some(b) = self.browsers.get_mut(&id)
            && b.find.take().is_some()
            && let Some(v) = &b.view
        {
            crate::browser::run_js(v, "getSelection().removeAllRanges(); return '';", |_| {});
        }
    }

    /// Looks for the find bar's text in the page: from the top (`step` 0), the next match (1),
    /// or the one before (-1). The page answers in a while (`find_result`).
    pub fn find_in_page_step(&mut self, id: PaneId, step: i32) {
        let Some(b) = self.browsers.get_mut(&id) else { return };
        let Some(f) = b.find.as_mut() else { return };
        f.step = step;
        if f.query.is_empty() {
            f.count = None;
            f.index = 0;
        }
        if let Some(v) = &b.view {
            let slot = b.find_result.clone();
            let ctx = self.ctx.clone();
            crate::browser::run_js(v, &crate::browser::find_script(&f.query, step < 0, step == 0), move |r| {
                let got: Option<(usize, bool)> = r.ok().and_then(|s| serde_json::from_str(&s).ok());
                if let Some(got) = got {
                    *slot.lock() = Some(got);
                    ctx.request_repaint();
                }
            });
        }
    }

    /// Zooms the page one step in (1) or out (-1), or back to 100% (0).
    pub fn zoom_page(&mut self, id: PaneId, dir: i32) {
        if let Some(b) = self.browsers.get_mut(&id) {
            b.user_zoom = if dir == 0 { 1.0 } else { crate::browser::zoom_step(b.user_zoom, dir) };
        }
    }

    /// The page with the keyboard (keys meant for a page act there: ⌘F, ⌘=, ⌘−).
    pub fn page_with_keys(&self) -> Option<PaneId> {
        self.browser_focus.filter(|id| self.browsers.contains_key(id))
    }

    /// A page's dialog above it, in the Notice's look: who asks and what (all of it, up to 8
    /// lines), a field for a prompt, OK (and Cancel). Enter answers OK, Esc Cancel. Returns its
    /// height.
    fn dialog_notice(&mut self, ui: &mut Ui, id: PaneId, at: egui::Pos2, width: f32, d: &Dialog) -> f32 {
        let p = style::pal(ui);
        let prompt = d.kind == DialogKind::Prompt;
        let ok_w = kit::button_width(ui, None, t("OK"), Some("↵"), true);
        let cancel_w = if d.kind != DialogKind::Alert { kit::button_width(ui, None, t("Cancel"), None, true) + 6.0 } else { 0.0 };
        let tx = at.x + 38.0;
        let text_w = (at.x + width - 8.0 - ok_w - cancel_w - 10.0 - tx).max(60.0);
        let who = if prompt { tf("{} asks", &[&d.host]) } else { tf("{} says", &[&d.host]) };
        let mut who_job = egui::text::LayoutJob::simple_singleline(who, style::medium(style::SMALL), p.text);
        who_job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
        let who_g = ui.painter().layout_job(who_job);
        let msg_text = clip_text(&d.message, 4_096);
        let mut job = egui::text::LayoutJob::simple(msg_text.into_owned(), style::mono(style::DATA), p.text_muted, text_w);
        job.wrap.max_rows = 8;
        let msg = ui.painter().layout_job(job);
        let field_h = 26.0;
        let content_h = who_g.size().y + 2.0 + msg.size().y + if prompt { 8.0 + field_h } else { 0.0 };
        let h = (content_h + 16.0).max(kit::NOTICE_H);
        let r = Rect::from_min_size(at, vec2(width, h));
        // The Notice, Neutral: the raised surface on a hairline.
        let painter = ui.painter_at(r.expand(1.0));
        painter.rect(r, CornerRadius::same(9), p.surface, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
        let top = r.center().y - content_h / 2.0;
        let who_h = who_g.size().y;
        painter.text(pos2(r.left() + 20.0, top + who_h / 2.0 + if h > kit::NOTICE_H { 0.0 } else { (content_h - who_h) / 2.0 }), Align2::CENTER_CENTER, icons::CHAT_CENTERED_TEXT, style::icon_font(16.0), kit::Tone::Neutral.color(&p));
        painter.galley(pos2(tx, top), who_g, p.text);
        painter.galley(pos2(tx, top + who_h + 2.0), msg.clone(), p.text_muted);
        let mut answer = None;
        let mut has_keys = false;
        let fid = egui::Id::new(("browser-dialog-field", id));
        let focus = std::mem::take(&mut self.dialog_focus) == Some(id);
        if prompt {
            let field = Rect::from_min_size(pos2(tx, top + who_h + 2.0 + msg.size().y + 8.0), vec2(text_w.min(360.0), field_h));
            let mut text = clip_text(&d.text, 4_096).into_owned();
            let resp = ui.scope_builder(egui::UiBuilder::new().max_rect(field), |ui| kit::field(ui, fid, &mut text, None, "", field.width(), true)).inner;
            if focus {
                resp.request_focus();
            }
            if resp.has_focus() {
                ui.memory_mut(|m| m.set_focus_lock_filter(fid, egui::EventFilter { escape: true, horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
            }
            has_keys |= resp.has_focus() || ui.memory(|m| m.had_focus_last_frame(fid));
            if resp.changed()
                && let Some(b) = self.browsers.get_mut(&id)
                && let Some(dd) = b.dialog.as_mut()
            {
                dd.text = text;
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                answer = Some(true);
            }
        }
        let row = Rect::from_min_max(pos2(r.right() - 8.0 - ok_w - cancel_w - 2.0, r.center().y - 12.0), pos2(r.right() - 8.0, r.center().y + 12.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let ok = kit::button_keys(ui, kit::Kind::Primary, None, t("OK"), Some("↵"), true);
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("browser-dialog-ok:{id}"), ok.rect);
            if focus && !prompt {
                ok.request_focus();
            }
            if ok.has_focus() {
                ui.memory_mut(|m| m.set_focus_lock_filter(ok.id, egui::EventFilter { escape: true, ..Default::default() }));
            }
            has_keys |= ok.has_focus() || ui.memory(|m| m.had_focus_last_frame(ok.id));
            if ok.clicked() {
                answer = Some(true);
            }
            if d.kind != DialogKind::Alert {
                let cancel = kit::button_ex(ui, kit::Kind::Secondary, None, t("Cancel"), true);
                #[cfg(debug_assertions)]
                crate::devtest::record(format!("browser-dialog-cancel:{id}"), cancel.rect);
                if cancel.has_focus() {
                    ui.memory_mut(|m| m.set_focus_lock_filter(cancel.id, egui::EventFilter { escape: true, ..Default::default() }));
                }
                has_keys |= cancel.has_focus() || ui.memory(|m| m.had_focus_last_frame(cancel.id));
                if cancel.clicked() {
                    answer = Some(false);
                }
            }
        });
        // Esc while the notice has the keyboard: Cancel (OK for an alert).
        if has_keys && !self.overlay_open() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            answer = Some(d.kind == DialogKind::Alert);
        }
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser-dialog:{id}"), r);
        if let Some(accept) = answer {
            self.answer_dialog(id, accept);
        }
        h
    }

    /// Answers a page's dialog (OK or Cancel; a prompt's field with OK). Returns whether one
    /// was waiting.
    pub fn answer_dialog(&mut self, id: PaneId, accept: bool) -> bool {
        let Some(d) = self.browsers.get_mut(&id).and_then(|b| b.dialog.take()) else { return false };
        crate::browser_dialog::answer(d.id, accept, &d.text);
        true
    }

    /// The list of what a page downloaded (a popover 320 wide): newest first, each with its
    /// file's icon, its name (a click opens it once it came), how it went, and Show in Finder
    /// (came) or Retry (failed); Clear List.
    fn downloads_list(&mut self, ui: &mut Ui, id: PaneId) {
        let p = style::pal(ui);
        let w = 320.0 - 28.0;
        ui.set_width(w);
        ui.spacing_mut().item_spacing = vec2(8.0, 14.0);
        let Some(b) = self.browsers.get(&id) else { return };
        let list: Vec<crate::browser::Download> = b.downloads.iter().rev().cloned().collect();
        let mut clear = false;
        let mut retry = None;
        let (head, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
        ui.painter().text(head.left_center(), Align2::LEFT_CENTER, t("Downloads"), style::semibold(style::HEADING), p.text);
        let cg = ui.painter().layout_no_wrap(t("Clear list").to_string(), egui::FontId::proportional(style::SMALL), p.text_faint);
        let cr = Rect::from_min_size(pos2(head.right() - cg.size().x, head.center().y - cg.size().y / 2.0), cg.size());
        let cresp = ui.interact(cr.expand(2.0), egui::Id::new(("browser-downloads-clear", id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        ui.painter().galley(cr.min, cg, if cresp.hovered() { p.text } else { p.text_faint });
        if cresp.clicked() {
            clear = true;
        }
        let reveal_tip = if cfg!(target_os = "macos") { t("Show in Finder") } else { t("Show in Folder") };
        let max_list_h = (ui.ctx().content_rect().height() * 0.55).clamp(160.0, 420.0);
        egui::ScrollArea::vertical()
            .id_salt(("browser-downloads-scroll", id))
            .max_height(max_list_h)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.set_width(w);
                ui.spacing_mut().item_spacing = vec2(8.0, 14.0);
                for (i, d) in list.iter().enumerate() {
                    let name = d.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let running = d.done.is_none();
                    let h = if running { 18.0 + 6.0 + 4.0 + 6.0 + 14.0 } else { 18.0 + 2.0 + 16.0 };
                    let (row, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                    let (icon, ic) = file_icon(&name, &p);
                    let name_y = row.top() + 9.0;
                    ui.painter().text(pos2(row.left() + 8.0, name_y), Align2::CENTER_CENTER, icon, style::icon_font(16.0), ic);
                    let tx = row.left() + 16.0 + 8.0;
                    let button = Rect::from_min_size(pos2(row.right() - ICON_S, if running { row.top() - 3.0 } else { row.center().y - ICON_S / 2.0 }), vec2(ICON_S, ICON_S));
                    let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), egui::FontId::proportional(style::BODY), p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((button.left() - 8.0 - tx).max(20.0));
                    let g = ui.painter().layout_job(job);
                    let nr = Rect::from_min_size(pos2(tx, name_y - g.size().y / 2.0), g.size());
                    let open = ui.interact(nr, egui::Id::new(("browser-download-name", id, i)), Sense::click());
                    let open = if d.done == Some(true) { open.on_hover_cursor(egui::CursorIcon::PointingHand) } else { open };
                    open.clone().on_hover_text(d.path.display().to_string());
                    ui.painter().galley(nr.min, g, p.text);
                    if open.clicked() && d.done == Some(true) {
                        crate::platform::reveal(&d.path);
                        ui.close();
                    }
                    match d.done {
                        None => {
                            // How far it is, as the system does not say: a segment running along.
                            let track = Rect::from_min_size(pos2(tx, row.top() + 18.0 + 6.0), vec2(row.right() - tx, 4.0));
                            ui.painter().rect_filled(track, CornerRadius::same(2), p.subtle);
                            let seg = track.width() * 0.3;
                            let x0 = track.left() + ((ui.input(|i| i.time) as f32 * 160.0) % (track.width() + seg)) - seg;
                            ui.painter().with_clip_rect(track.intersect(ui.clip_rect())).rect_filled(Rect::from_min_size(pos2(x0, track.top()), vec2(seg, 4.0)), CornerRadius::same(2), p.accent);
                            ui.ctx().request_repaint_after(Duration::from_millis(33));
                            ui.painter().text(pos2(tx, track.bottom() + 6.0 + 7.0), Align2::LEFT_CENTER, t("Downloading…"), style::mono(style::DATA), p.text_faint);
                        }
                        Some(false) => {
                            ui.painter().text(pos2(tx, row.bottom() - 8.0), Align2::LEFT_CENTER, t("Download failed"), egui::FontId::proportional(style::SMALL), p.err);
                            let r = ui.scope_builder(egui::UiBuilder::new().max_rect(button), |ui| kit::icon_button_ex(ui, icons::ARROW_CLOCKWISE, t("Retry"), false, ICON_S)).inner;
                            if r.clicked() {
                                retry = Some(d.url.clone());
                            }
                        }
                        Some(true) => {
                            let at = std::time::SystemTime::now().checked_sub(d.at.elapsed()).unwrap_or_else(std::time::SystemTime::now);
                            let meta = format!("{} · {}", super::update_ui::human_size(d.size.unwrap_or(0)), super::dialogs::ago(at));
                            ui.painter().text(pos2(tx, row.bottom() - 8.0), Align2::LEFT_CENTER, meta, style::mono(style::DATA), p.text_faint);
                            let r = ui.scope_builder(egui::UiBuilder::new().max_rect(button), |ui| kit::icon_button_ex(ui, icons::FOLDER_OPEN, reveal_tip, false, ICON_S)).inner;
                            if r.clicked() {
                                crate::platform::reveal_in_folder(&d.path);
                                ui.close();
                            }
                        }
                    }
                }
            });
        if list.is_empty() {
            ui.label(egui::RichText::new(t("Nothing downloaded yet.")).size(style::SMALL).color(p.text_faint));
        }
        if clear && let Some(b) = self.browsers.get_mut(&id) {
            b.downloads.retain(|d| d.done.is_none());
        }
        // Retry: the page downloads it again (with its cookies, as a link with `download`).
        if let Some(url) = retry
            && let Some(v) = self.browsers.get(&id).and_then(|b| b.view.as_ref())
        {
            let js = format!("const a = document.createElement('a'); a.href = {}; a.download = ''; document.body.appendChild(a); a.click(); a.remove(); return '';", serde_json::to_string(&url).unwrap_or_default());
            crate::browser::run_js(v, &js, |_| {});
        }
    }

    /// A browser pane's menu (⋯ in its toolbar, or a right-click on it).
    fn browser_menu(&mut self, ui: &mut Ui, id: PaneId) {
        kit::menu_body(ui);
        ui.set_min_width(238.0);
        let mac = cfg!(target_os = "macos");
        let Some(b) = self.browsers.get(&id) else { return };
        let url = b.url.clone();
        let has_downloads = !b.downloads.is_empty();
        let served = b.served_by.filter(|pane| self.panes.contains_key(pane)).map(|pane| (pane, self.leaf_label(pane)));
        if ui.add_enabled_ui(!url.is_empty(), |ui| kit::menu_item(ui, Some(icons::COPY), t("Copy address"), None, false)).inner.clicked() {
            ui.ctx().copy_text(url.clone());
            ui.close();
        }
        if ui.add_enabled_ui(!url.is_empty(), |ui| kit::menu_item(ui, Some(icons::ARROW_SQUARE_OUT), t("Open in default browser"), None, false)).inner.clicked() {
            crate::platform::open_url(&url);
            ui.close();
        }
        // The terminal whose dev server serves the page.
        if let Some((pane, label)) = served
            && kit::menu_item(ui, Some(icons::TERMINAL_WINDOW), &tf("Show terminal: {}", &[&label]), None, false).clicked()
        {
            self.actions.push(Action::FocusPane(pane));
            ui.close();
        }
        kit::menu_sep(ui);
        if kit::menu_item(ui, Some(icons::MAGNIFYING_GLASS), t("Find…"), Some(if mac { "⌘F" } else { "Ctrl+F" }), false).clicked() {
            self.open_page_find(id);
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::MAGNIFYING_GLASS_PLUS), t("Zoom in"), Some(if mac { "⌘=" } else { "Ctrl+=" }), false).clicked() {
            self.zoom_page(id, 1);
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::MAGNIFYING_GLASS_MINUS), t("Zoom out"), Some(if mac { "⌘-" } else { "Ctrl+-" }), false).clicked() {
            self.zoom_page(id, -1);
            ui.close();
        }
        // ⌘0 is the Grid on macOS.
        if kit::menu_item(ui, None, t("Actual size"), (!mac).then_some("Ctrl+0"), false).clicked() {
            self.zoom_page(id, 0);
            ui.close();
        }
        kit::menu_sep(ui);
        if ui.add_enabled_ui(has_downloads, |ui| kit::menu_item(ui, Some(icons::TRAY_ARROW_DOWN), t("Downloads"), None, false)).inner.clicked() {
            ui.data_mut(|d| d.insert_temp(egui::Id::new(("browser-downloads-list", id)).with("ask"), true));
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::CODE), t("Inspect element"), None, false).clicked() {
            if let Some(v) = self.browsers.get(&id).and_then(|b| b.view.as_ref()) {
                v.open_devtools();
            }
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::PRINTER), t("Print…"), None, false).clicked() {
            if let Some(v) = self.browsers.get(&id).and_then(|b| b.view.as_ref()) {
                crate::browser::print(v);
            }
            ui.close();
        }
        kit::menu_sep(ui);
        if kit::menu_item(ui, Some(icons::X), t("Close page"), kit::keys(&Action::ClosePane), true).clicked() {
            self.actions.push(Action::ClosePaneId(id));
            ui.close();
        }
    }

    /// Web pages open in TernaryCode (else in the default browser): where browser panes can show
    /// pages (`browser::supported`), and Settings → Browser says so.
    pub fn pages_here(&self) -> bool {
        crate::browser::supported() && self.settings.browser_here
    }

    /// What the Browser button in the top bar says for the project in view.
    pub fn browser_button_state(&self) -> Option<WebButton> {
        let project = self.active?;
        let panel = self.panel_of(project)?;
        let shown = self.panel_page(project).is_some();
        if let Some(d) = panel.pages.iter().filter_map(|id| self.browsers.get(id)).find_map(|b| b.driver.as_ref().filter(|d| !d.paused)) {
            return Some(WebButton::Agent(d.who.clone(), shown));
        }
        if shown {
            return Some(WebButton::InView);
        }
        if let Some(b) = panel.front.or(panel.pages.last().copied()).and_then(|id| self.browsers.get(&id)) {
            return Some(WebButton::Elsewhere(b.label()));
        }
        let server = self.project_servers(project).first().map(|(_, s)| crate::browser::split_url(&s.url).0);
        Some(WebButton::None(server))
    }

    /// The Browser button (in the top bar, beside Code): shows the project's browser or makes
    /// one; ⌥-click, always a new one.
    pub fn browser_button(&mut self, ui: &mut Ui) {
        let Some(state) = self.browser_button_state() else { return };
        let p = style::pal(ui);
        let keys = kit::keys(&Action::ShowBrowser).unwrap_or_default();
        let head = format!("{}   {keys}", t("Browser"));
        let in_view_hint = if cfg!(target_os = "macos") {
            t("Hides the Browser panel · ⌘L: type an address")
        } else {
            t("Hides the Browser panel · Ctrl+L: type an address")
        };
        let tip = match &state {
            WebButton::None(Some(host)) => format!("{head}\n{}", tf("Opens {} beside the terminals", &[host])),
            WebButton::None(None) => format!("{head}\n{}", t("Opens an empty page beside the terminals")),
            WebButton::Elsewhere(label) => format!("{head}\n{}", tf("Shows the Browser panel (“{}”)", &[label])),
            WebButton::InView => format!("{head}\n{in_view_hint}"),
            WebButton::Agent(who, false) => format!("{}\n{}", tf("{} is using the browser", &[who]), t("Click to watch it · clicking or typing in it takes it over")),
            WebButton::Agent(who, true) => format!("{}\n{}", tf("{} is using the browser", &[who]), t("Hides the Browser panel · the agent keeps working")),
        };
        let new_page_hint = if cfg!(target_os = "macos") { t("⌥-click: a new page") } else { t("Alt-click: a new page") };
        let tip = if self.pages_here() { format!("{tip}\n{new_page_hint}") } else { tip };
        let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
        let resp = resp.on_hover_text(tip);
        #[cfg(debug_assertions)]
        crate::devtest::record("browser-button", rect);
        let painter = ui.painter();
        let radius = CornerRadius::same(style::RADIUS);
        let (fill, color) = match state {
            WebButton::Agent(..) => (Some(style::tint(p.info, ui.visuals().dark_mode)), p.info),
            WebButton::InView => (Some(p.accent_soft), p.accent_text),
            _ if resp.hovered() => (Some(p.hover), p.text),
            _ => (None, p.text_muted),
        };
        if let Some(f) = fill {
            painter.rect_filled(rect, radius, if resp.hovered() { f.gamma_multiply(1.4) } else { f });
        }
        painter.text(rect.center(), Align2::CENTER_CENTER, icons::GLOBE, style::icon_font(15.0), color);
        if matches!(state, WebButton::Agent(..)) {
            let dot = pos2(rect.right() - 6.0, rect.top() + 6.0);
            painter.circle_filled(dot, 3.5, p.chrome);
            painter.circle_filled(dot, 2.5, p.info);
        }
        if resp.clicked() {
            let alt = ui.input(|i| i.modifiers.alt);
            self.actions.push(if alt && self.pages_here() { Action::NewBrowserTab } else { Action::ShowBrowser });
        }
    }

    /// The Browser button and ⇧⌘B: shows the project's Browser panel, or hides it when it shows;
    /// `address` (⌘L): its page's address field (the panel shown first). With no page yet, one
    /// at the project's dev server, else empty with its address field taking the keyboard.
    pub fn show_browser(&mut self, address: bool) {
        let Some(project) = self.command_project() else { return };
        let in_focus = self.view == tc_core::model::View::Focus && self.active == Some(project);
        if let Some(front) = self.panel_page(project).filter(|_| in_focus) {
            if address {
                self.release_page_keyboard();
                self.browser_edit = Some(front);
            } else {
                self.hide_panel(project);
            }
            return;
        }
        self.open_browser_panel(project, address);
    }

    /// Opens or brings forward `project`'s Browser panel without toggling it closed.
    pub(crate) fn open_browser_panel(&mut self, project: tc_core::ProjectId, address: bool) {
        let server = self.project_servers(project).first().map(|(_, s)| s.url.clone());
        if !crate::browser::supported() {
            // Pages can't show in TernaryCode here: the default browser has the page.
            match server {
                Some(url) => crate::platform::open_url(&url),
                None => self.toast(t("Pages can't show in TernaryCode here. They open in your default browser.").to_string()),
            }
            return;
        }
        let page = self.panel_of(project).and_then(|p| p.front.filter(|f| p.pages.contains(f)).or(p.pages.last().copied()));
        let page = match page {
            Some(id) => id,
            None => self.add_page(project, if address { "" } else { server.as_deref().unwrap_or_default() }, None),
        };
        self.active = Some(project);
        self.show_page(project, page);
        let empty = self.browsers.get(&page).is_some_and(|b| b.url.is_empty());
        if address || empty {
            self.release_page_keyboard();
            self.browser_edit = Some(page);
        }
        self.show_focus();
    }

    /// ⌘R: the page in view loads again (the one with the keyboard first).
    pub fn reload_browser(&mut self) {
        let shown = self.active.and_then(|p| self.panel_page(p));
        let id = self.browser_focus.filter(|id| self.browsers.contains_key(id)).or(shown);
        if let Some(b) = id.and_then(|id| self.browsers.get(&id)) {
            b.reload();
        }
    }

    /// A new page of the user's in the project's panel, at its local server when it has one
    /// (else its address field takes the keyboard).
    pub fn new_browser_tab(&mut self) {
        let Some(project) = self.command_project() else { return };
        let url = self.project_servers(project).first().map(|(_, s)| s.url.clone()).unwrap_or_default();
        self.browser_tab_at(project, &url);
    }

    /// A new page of the user's in `project`'s panel at `url` ("": its address field takes the
    /// keyboard), in view.
    pub fn browser_tab_at(&mut self, project: tc_core::ProjectId, url: &str) {
        let id = self.add_page(project, url, None);
        self.active = Some(project);
        self.show_page(project, id);
        if url.is_empty() {
            self.release_page_keyboard();
            self.browser_edit = Some(id);
        }
        self.show_focus();
    }

    /// Opens `url` in the project's Browser panel: in the user's page of the same site (host
    /// and port) when there is one, else in a new page. Agents' pages are theirs. The keyboard
    /// stays where it is.
    pub fn open_web_here(&mut self, url: &str) {
        let Some(project) = self.command_project().or(self.active) else {
            crate::platform::open_url(url);
            return;
        };
        self.open_web_in(project, url, true);
    }

    /// Opens `url` in `project`'s panel, as [`App::open_web_here`]; `show`: the project comes
    /// into view (else its page waits there, the sidebar showing it has one).
    pub fn open_web_in(&mut self, project: tc_core::ProjectId, url: &str, show: bool) {
        let site = crate::browser::split_url(&crate::browser::normalize(url)).0;
        let pages = self.panel_of(project).map(|p| p.pages.clone()).unwrap_or_default();
        let same = pages.iter().copied().find(|id| {
            self.browsers.get(id).is_some_and(|b| b.owner.is_none() && !site.is_empty() && crate::browser::split_url(&b.url).0 == site)
        });
        let id = match same {
            Some(id) => {
                if let Some(b) = self.browsers.get_mut(&id) {
                    b.navigate(url);
                }
                id
            }
            None => self.add_page(project, &crate::browser::normalize(url), None),
        };
        self.show_page(project, id);
        if show {
            if self.active != Some(project) {
                self.actions.push(Action::SelectProject(project));
            }
            self.show_focus();
        }
        self.mark_dirty();
    }

    /// Forgets every page's cookies, logins, cache and history (one profile for all).
    pub fn clear_browsing_data(&mut self) {
        let views: Vec<&crate::browser::View> = self.browsers.values().filter_map(|b| b.view.as_ref()).collect();
        crate::browser::clear_data(&views);
        self.toast_ok(t("Logins, cookies and history were cleared.").to_string());
    }

    /// The page that has the keyboard gives it back to the app.
    pub fn release_page_keyboard(&mut self) {
        if let Some(id) = self.browser_focus.take()
            && let Some(v) = self.browsers.get(&id).and_then(|b| b.view.as_ref())
        {
            let _ = v.focus_parent();
        }
    }

    /// Memory the pages take: their WebKit processes (each counted once).
    pub fn browsers_resident_bytes(&self) -> Option<u64> {
        #[cfg(target_os = "macos")]
        {
            let pids: std::collections::BTreeSet<i32> = self.browsers.values().filter_map(|b| b.view.as_ref()).filter_map(crate::browser::web_pid).collect();
            if pids.is_empty() {
                return None;
            }
            Some(pids.into_iter().filter_map(tc_agent::proc::resident_bytes).sum())
        }
        // The system's web view shares its processes (WebView2, WebKitGTK's helper).
        #[cfg(not(target_os = "macos"))]
        None
    }

    /// ⌘C, ⌘V, ⌘A while a page has the keyboard: sent to the page. Returns whether one has it.
    pub fn page_has_keys(&self, command: &str) -> bool {
        if self.overlay_open() {
            return false;
        }
        if let Some(v) = self.browser_focus.and_then(|id| self.browsers.get(&id)).filter(|b| b.shown).and_then(|b| b.view.as_ref()) {
            crate::browser::edit_command(v, command);
            return true;
        }
        false
    }

    /// Takes in what the pages reported: a click in a page gives it the keyboard (and makes
    /// its pane the focused one), a new window opens in the same pane, a new address is saved.
    pub fn browser_events(&mut self, ctx: &egui::Context) {
        // What the page helper said since the last frame (Linux).
        #[cfg(target_os = "linux")]
        crate::browser_linux::pump();
        let mut moved = false;
        let mut focused = None;
        // Loads that began in pages an agent uses: (page, address, the address before).
        let mut began = Vec::new();
        let mut new_tabs = Vec::new();
        let mut dialogs = Vec::new();
        let mut keys = Vec::new();
        for b in self.browsers.values_mut() {
            // The page answered a search of its find bar.
            if let Some((n, found)) = b.find_result.lock().take()
                && let Some(f) = b.find.as_mut()
            {
                f.answered(n, found);
            }
            let before = b.url.clone();
            for e in b.take_events() {
                match e {
                    // Believed after a real click on the page only: a script in it can post
                    // that message too, to take the keyboard from a terminal.
                    Event::Focused => {
                        let clicked = b.view.as_ref().is_some_and(crate::browser::clicked_in);
                        if clicked {
                            focused = Some(b.id);
                        }
                    }
                    // A link to a new window: a new page of the user's in the panel; an agent's
                    // page keeps it (the agent follows it there).
                    Event::OpenHere(url) if b.driven() => b.navigate(&url),
                    Event::OpenHere(url) => new_tabs.push((b.project, url)),
                    Event::Url(_) => {
                        if !ctx.memory(|m| m.has_focus(egui::Id::new(("browser-address", b.id)))) {
                            b.address = b.url.clone();
                        }
                        moved = true;
                    }
                    Event::Dialog(d) => {
                        b.dialog = Some(d);
                        dialogs.push(b.id);
                    }
                    e @ (Event::DownloadStarted(..) | Event::DownloadDone(..)) => b.note_download(&e),
                    Event::Loading(true) if b.driven() => began.push((b.id, b.url.clone(), before.clone())),
                    Event::Key(key, mods) => keys.push((b.id, key, mods)),
                    _ => {}
                }
            }
        }
        // macOS: the user clicked in a page (seen as it happened, `browser::note_user_click`),
        // whether or not the page says it took the keyboard (it had it already for an agent).
        #[cfg(target_os = "macos")]
        if let Some(key) = crate::browser::take_clicked_page()
            && let Some(id) = self
                .browsers
                .values()
                .find(|b| b.view.as_ref().is_some_and(|v| crate::browser::page_key(&crate::browser::web_view_of(v)) == key))
                .map(|b| b.id)
        {
            focused = Some(id);
        }
        for (project, url) in new_tabs {
            self.browser_tab_at(project, &url);
        }
        // A dialog came up: the keyboard leaves its page (blocked till it is answered) for the
        // dialog's bar, when the page had it.
        for id in dialogs {
            if let Some(v) = self.browsers.get(&id).and_then(|b| b.view.as_ref())
                && crate::browser::has_keyboard(v)
            {
                let _ = v.focus_parent();
                self.dialog_focus = Some(id);
            }
            if self.browser_focus == Some(id) {
                self.browser_focus = None;
                self.dialog_focus = Some(id);
            }
            if focused == Some(id) {
                focused = None;
            }
        }
        // Dialogs of pages that went away are answered (WebKit wants each answered).
        #[cfg(target_os = "macos")]
        {
            let live: Vec<objc2::rc::Retained<objc2_web_kit::WKWebView>> = self.browsers.values().filter_map(|b| b.view.as_ref()).map(crate::browser::web_view_of).collect();
            crate::browser_dialog::tidy(&live.iter().map(|w| &**w).collect::<Vec<_>>());
        }
        #[cfg(not(target_os = "macos"))]
        {
            let live: Vec<PaneId> = self.browsers.values().filter(|b| b.view.is_some()).map(|b| b.id).collect();
            crate::browser_dialog::tidy(&live);
        }
        for (id, url, before) in began {
            self.stop_unasked_load(id, &url, &before);
        }
        // Keys of the app's pressed in a page (Windows, Linux): as in the app.
        for (id, key, mods) in keys {
            self.page_key(id, key, mods);
        }
        // A click in a page an agent uses: the user takes it over (and the page keeps the
        // keyboard, which it may have lost in between, see `sync_browsers`).
        if let Some(id) = focused
            && self.browsers.get(&id).is_some_and(Browser::driven)
        {
            self.take_over_browser(id, true);
            self.browser_focus_request = Some(id);
        }
        if let Some(id) = focused {
            self.browser_focus = Some(id);
            // The app's own keyboard focus (a terminal) lets go: its keys go to the page now.
            ctx.memory_mut(|m| {
                if let Some(f) = m.focused() {
                    m.surrender_focus(f);
                }
            });
        }
        if moved {
            self.mark_dirty();
        }
    }

    /// Lays each page's web view where its pane was drawn this frame, hidden when the pane was
    /// not drawn or something covers it (a dialog, the update dialog, the palette, a menu,
    /// Review full screen);
    /// lets go of pages out of view and unused a while (asleep). Hands the keyboard between
    /// the pages and the app.
    pub fn sync_browsers(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        #[cfg(target_os = "linux")]
        crate::browser_linux::note_window(frame, ctx);
        #[cfg(windows)]
        crate::browser_win::note_window(ctx);
        if self.browsers.is_empty() {
            self.browser_pages.clear();
            self.run_browser_jobs(ctx);
            self.browser_focus = None;
            self.tell_key_watch();
            return;
        }
        // Pages agents use stay up out of view (outside the window, where they keep drawing).
        let in_use = self.browsers_in_use();
        let now = std::time::Instant::now();
        let covered = self.dialog.is_some()
            || self.updater.dialog_open
            || self.palette.is_some()
            || self.review_full()
            || self.attention_open
            || self.monitor_popover
            || egui::Popup::is_any_open(ctx);
        let zoom = ctx.zoom_factor();
        let ppp_bits = ctx.pixels_per_point().to_bits();
        let sleep = crate::browser::sleep_after(self.settings.browser_sleep_minutes);
        // Memory runs low (the system says so): pages out of view sleep sooner, the oldest first.
        let pressure = self.memory_pressure(now);
        // A page with a pop-up open (a sign-in) keeps it: it stays awake.
        let popups: std::collections::HashSet<PaneId> = self.browsers.keys().copied().filter(|id| crate::browser_popup::popup_of(*id).is_some()).collect();
        let pages = std::mem::take(&mut self.browser_pages);
        let mut wake: Option<Duration> = None;
        for b in self.browsers.values_mut() {
            b.pick_up_picture(ctx);
            // The page helper went away (Linux): the page loads again in a new one.
            #[cfg(target_os = "linux")]
            if b.view.as_ref().is_some_and(crate::browser::View::dead) {
                b.sleep();
            }
            match pages.get(&b.id) {
                Some(rect) if rect.width() >= 8.0 && rect.height() >= 8.0 && (!covered || b.view.is_some()) => {
                    b.last_seen = now;
                    // Its server does not answer: the pane says so (the page would be blank).
                    if b.down {
                        if b.shown
                            && let Some(v) = &b.view
                        {
                            let _ = v.set_visible(false);
                        }
                        b.shown = false;
                        continue;
                    }
                    if covered {
                        // Something is drawn over the page, which the page would hide: its
                        // picture shows in its place (once it comes, or after a moment).
                        let since = *b.covered_since.get_or_insert(now);
                        if b.shown {
                            let pictured = b.still.as_ref().is_some_and(|(_, at)| *at >= since);
                            if !pictured {
                                b.ask_picture(ctx, None);
                            }
                            if pictured || now.duration_since(since) > Duration::from_millis(250) {
                                if let Some(v) = &b.view {
                                    if in_use.contains(&b.id) {
                                        // An agent works in it: it keeps drawing, out of the window.
                                        let off = crate::browser::bounds(Rect::from_min_size(pos2(-30000.0, 0.0), rect.size()), zoom);
                                        let _ = v.set_bounds(off);
                                        b.placed = None;
                                        b.placed_ppp = 0;
                                        b.offscreen = true;
                                    } else {
                                        let _ = v.set_visible(false);
                                    }
                                }
                                b.shown = false;
                            } else {
                                ctx.request_repaint_after(Duration::from_millis(30));
                            }
                        }
                        continue;
                    }
                    if b.covered_since.take().is_some() {
                        // Uncovered: the page itself again; its picture is let go.
                        b.still = None;
                    }
                    b.offscreen = false;
                    // A page with no address yet has no web view (the pane says so).
                    if b.view.is_none() && !b.url.is_empty() {
                        b.view = crate::browser::build_view(frame, &b.url, crate::browser::bounds(*rect, zoom), b.sink(ctx));
                        b.asleep = b.view.is_none();
                        if b.view.is_some() {
                            b.asleep_since = None;
                        }
                        b.placed = Some(*rect);
                        b.placed_ppp = ppp_bits;
                        b.shown = b.view.is_some();
                        b.dimmed = false;
                        b.zoom = 1.0;
                        b.corner = 0.0;
                    } else if b.view.is_some() {
                        if (b.placed != Some(*rect) || b.placed_ppp != ppp_bits)
                            && let Some(v) = &b.view
                        {
                            let _ = v.set_bounds(crate::browser::bounds(*rect, zoom));
                            b.placed = Some(*rect);
                            b.placed_ppp = ppp_bits;
                        }
                        if !b.shown
                            && let Some(v) = &b.view
                        {
                            let _ = v.set_visible(true);
                            b.shown = true;
                        }
                    }
                    // The page's corners within its frame's rounded border.
                    let corner = b.corner_want * zoom;
                    if (b.corner - corner).abs() > 0.01
                        && let Some(v) = &b.view
                    {
                        crate::browser::round_corners(v, f64::from(corner));
                        b.corner = corner;
                    }
                    // Zoomed out in a Grid tile (the layout of Focus, smaller), as is in Focus.
                    // And the zoom the user chose for the page (⌘= / ⌘−).
                    let want = b.zoom_want * b.user_zoom;
                    if (b.zoom - want).abs() > 0.004
                        && let Some(v) = &b.view
                    {
                        let _ = v.zoom(f64::from(want));
                        b.zoom = want;
                    }
                }
                _ if in_use.contains(&b.id) => {
                    let size = b.placed.map_or(vec2(1200.0, 800.0), |r| r.size());
                    let off = crate::browser::bounds(Rect::from_min_size(pos2(-30000.0, 0.0), size), zoom);
                    if b.view.is_none() && !b.url.is_empty() {
                        b.view = crate::browser::build_view(frame, &b.url, off, b.sink(ctx));
                        b.asleep = b.view.is_none();
                        if b.view.is_some() {
                            b.asleep_since = None;
                        }
                        b.dimmed = false;
                    } else if !b.offscreen
                        && let Some(v) = &b.view
                    {
                        let _ = v.set_bounds(off);
                        let _ = v.set_visible(true);
                    }
                    b.offscreen = b.view.is_some();
                    b.placed = None;
                    b.placed_ppp = 0;
                    b.shown = false;
                    b.last_seen = now;
                }
                _ => {
                    if (b.shown || b.offscreen)
                        && let Some(v) = &b.view
                    {
                        let _ = v.set_visible(false);
                    }
                    b.shown = false;
                    b.offscreen = false;
                    if b.view.is_some() {
                        // Busy (sound, the camera, a form being filled in, a dialog, a download,
                        // a pop-up): it stays awake, and its time unused counts from when that
                        // ends.
                        if b.busy() || popups.contains(&b.id) {
                            b.last_seen = now;
                        }
                        let limit = if pressure { sleep.min(crate::browser::PRESSURE_SLEEP) } else { sleep };
                        let idle = now.duration_since(b.last_seen);
                        if idle >= limit {
                            b.sleep();
                        } else {
                            // Looked at again within a minute: memory may run low meanwhile.
                            let left = (limit - idle).min(crate::browser::PRESSURE_SLEEP);
                            wake = Some(wake.map_or(left, |w| w.min(left)));
                        }
                    }
                }
            }
        }
        // A page under a question is dimmed; a page loading says so (from its request on). The
        // keyboard stays where the user types (`browser_focus`): a page that has it otherwise
        // (an agent focused something in it, or the page took it by itself) gives it back once
        // no agent acts in it. On macOS a page has the user's keyboard only when its window is
        // the key window (`browser_host`), which an agent's clicks and keys never make it. On
        // Windows and Linux an agent's keys need no keyboard, and a page the user clicks says so
        // only a moment after it took the keyboard: there only agents' pages give it back.
        let mut typing_in = self.browser_focus;
        let acting: std::collections::HashSet<PaneId> = if cfg!(target_os = "macos") { self.browser_jobs.iter().map(|j| j.browser).collect() } else { Default::default() };
        let mut back = Vec::new();
        for b in self.browsers.values_mut() {
            // A page out of sight that has the user's keyboard gives it back (else keys go
            // nowhere). Not one an agent types in out of sight (macOS: its own window's keyboard).
            if !b.shown {
                if let Some(v) = &b.view
                    && crate::browser::has_user_keyboard(v)
                {
                    let _ = v.focus_parent();
                }
                if typing_in == Some(b.id) {
                    typing_in = None;
                }
            }
            if let Some(v) = &b.view {
                b.loading = crate::browser::is_loading(v);
                if typing_in != Some(b.id)
                    && (cfg!(target_os = "macos") || b.driven())
                    && !acting.contains(&b.id)
                    && crate::browser::has_keyboard(v)
                {
                    back.push(b.id);
                }
            }
            if b.view.is_some() {
                let loading = b.loading;
                b.watch_server(loading, ctx);
            }
            let dim = b.ask.is_some() || b.dialog.is_some();
            if dim != b.dimmed
                && let Some(v) = &b.view
            {
                crate::browser::dim(v, dim);
                b.dimmed = dim;
            }
        }
        if typing_in != self.browser_focus {
            self.browser_focus = typing_in;
        }
        for id in back {
            self.keyboard_back_from(id);
        }
        // An agent's page asleep a while after its agent ended (it left, or its terminal
        // closed): it closes (Reopen Closed Page brings it back). One whose agent still runs
        // in its terminal stays: the agent may come back to it.
        let keep = crate::browser::agent_page_keep();
        let mut ended = Vec::new();
        for b in self.browsers.values() {
            let Some(owner) = b.owner else { continue };
            let Some(since) = b.asleep_since.filter(|_| b.view.is_none() && !in_use.contains(&b.id)) else { continue };
            if self.panes.get(&owner).is_some_and(|p| p.tracker.agent.is_some()) {
                continue;
            }
            let asleep = now.duration_since(since);
            if asleep >= keep {
                ended.push(b.id);
            } else {
                let left = keep - asleep;
                wake = Some(wake.map_or(left, |w| w.min(left)));
            }
        }
        for id in ended {
            self.close_page(id);
        }
        if let Some(d) = wake {
            ctx.request_repaint_after(d);
        }
        self.run_browser_jobs(ctx);
        // Windows, Linux: the window comes back to the front (Alt+Tab, a click on its title):
        // the system gives the keyboard to the app's window, not to the page inside it that had
        // it. The page takes it again.
        #[cfg(not(target_os = "macos"))]
        {
            let front = ctx.input(|i| i.raw.focused);
            if front && !self.window_in_front
                && let Some(id) = self.browser_focus
                && self.browser_focus_request.is_none()
            {
                self.browser_focus_request = Some(id);
            }
            self.window_in_front = front;
        }
        // A pane asked to be focused: its page takes the keyboard.
        if let Some(id) = self.browser_focus_request.take()
            && let Some(v) = self.browsers.get(&id).and_then(|b| b.view.as_ref())
        {
            let _ = v.focus();
            self.browser_focus = Some(id);
        }
        // A click in the app (not on a page) gives it the keyboard back.
        if let Some(id) = self.browser_focus
            && ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pos| !pages.values().any(|r| r.contains(pos))))
        {
            if let Some(v) = self.browsers.get(&id).and_then(|b| b.view.as_ref()) {
                let _ = v.focus_parent();
            }
            self.browser_focus = None;
        }
        self.tell_key_watch();
    }

    /// The click watch (macOS, `browser::note_user_click`) learns where the user types.
    fn tell_key_watch(&self) {
        #[cfg(target_os = "macos")]
        {
            crate::browser_host::tell_webkit();
            let user = self.browser_focus.and_then(|id| self.browsers.get(&id)).and_then(|b| b.view.as_ref());
            crate::browser::set_user_page(user.map(|v| crate::browser::page_key(&crate::browser::web_view_of(v))));
        }
    }

    /// A page has the keyboard though the user does not type in it: the keyboard goes back
    /// where they type, the app or another page (Windows, Linux: its caret where it was, which
    /// the page lost with the keyboard; on macOS each page keeps its own window's keyboard, and
    /// its caret with it).
    fn keyboard_back_from(&mut self, id: PaneId) {
        if let Some(v) = self.browsers.get(&id).and_then(|b| b.view.as_ref()) {
            let _ = v.focus_parent();
        }
        if let Some(v) = self.browser_focus.filter(|u| *u != id).and_then(|u| self.browsers.get(&u)).and_then(|b| b.view.as_ref()) {
            let _ = v.focus();
            #[cfg(not(target_os = "macos"))]
            crate::browser_agent::restore_caret(v);
        }
    }

    /// The user types in the app from now on (a terminal or a field of the app took the
    /// keyboard): the page they typed in lets go of it.
    pub fn keyboard_to_app(&mut self) {
        self.release_page_keyboard();
        self.tell_key_watch();
    }
}

/// What the Browser button in the top bar shows.
pub enum WebButton {
    /// No browser yet: a click opens one (at this dev server, when there is one).
    None(Option<String>),
    /// Pages, the panel hidden (the page in front's label): a click shows it.
    Elsewhere(String),
    /// The panel shows: a click hides it.
    InView,
    /// An agent uses it (who; whether the panel shows).
    Agent(String, bool),
}

/// The address field of a page's bar: a well (inset, 30 high) with the lock or globe, the
/// address in the data font, its path quieter than its host while it is not being typed in;
/// ringed in the accent while it is; "Not secure" at its end for plain http off this machine.
/// Returns the text field's response and whether the well around it was clicked.
#[allow(clippy::too_many_arguments)]
fn address_field(ui: &mut Ui, id: egui::Id, text: &mut String, icon: &str, rect: Rect, typing: bool, insecure: bool) -> (egui::Response, bool) {
    let p = style::pal(ui);
    let dark = ui.visuals().dark_mode;
    let outer = ui.interact(rect, id.with("well"), Sense::click());
    if outer.clicked() {
        ui.ctx().memory_mut(|m| m.request_focus(id));
    }
    let painter = ui.painter();
    if typing {
        painter.rect_stroke(rect.expand(3.0), CornerRadius::same(11), Stroke::new(3.0, style::soft(p.accent, 0.25)), egui::StrokeKind::Inside);
    }
    painter.rect(rect, CornerRadius::same(8), p.inset, Stroke::new(1.0, if typing { p.accent } else { p.border_strong }), egui::StrokeKind::Inside);
    painter.text(pos2(rect.left() + 10.0 + 6.5, rect.center().y), Align2::CENTER_CENTER, icon, style::icon_font(13.0), p.text_faint);
    let mut right = rect.right() - 10.0;
    if insecure && !typing {
        let g = painter.layout_no_wrap(t("Not secure").to_string(), style::medium(style::SMALL), p.warn);
        let pill = Rect::from_min_max(pos2(rect.right() - 6.0 - g.size().x - 12.0, rect.center().y - 9.0), pos2(rect.right() - 6.0, rect.center().y + 9.0));
        painter.rect_filled(pill, CornerRadius::same(5), style::tint(p.warn, dark));
        painter.galley(pill.center() - g.size() / 2.0, g, p.warn);
        right = pill.left() - 8.0;
    }
    let inner = Rect::from_min_max(pos2(rect.left() + 10.0 + 13.0 + 8.0, rect.top() + 1.0), pos2(right.max(rect.left() + 60.0), rect.bottom() - 1.0));
    let (host_c, path_c) = (p.text, p.text_faint);
    let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let s = buf.as_str();
        let font = style::mono(style::MONO);
        let mut job = egui::text::LayoutJob::default();
        // Host, then path (from the first "/" after the scheme) in a quieter ink.
        let start = s.find("://").map_or(0, |i| i + 3);
        let cut = if typing { s.len() } else { s[start..].find('/').map_or(s.len(), |i| start + i) };
        job.append(&s[..cut], 0.0, egui::TextFormat::simple(font.clone(), host_c));
        if cut < s.len() {
            job.append(&s[cut..], 0.0, egui::TextFormat::simple(font, path_c));
        }
        job.wrap.max_width = wrap;
        job.wrap.max_rows = 1;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let resp = child.add_sized(
        inner.size(),
        egui::TextEdit::singleline(text)
            .id(id)
            .char_limit(8_192)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(style::mono(style::MONO))
            .hint_text(egui::RichText::new(t("Search or enter an address")).font(egui::FontId::proportional(style::SMALL)).color(p.text_faint))
            .layouter(&mut layouter)
            .vertical_align(egui::Align::Center),
    );
    (resp, outer.clicked())
}

fn clip_text(s: &str, max_chars: usize) -> std::borrow::Cow<'_, str> {
    if s.len() <= max_chars {
        return std::borrow::Cow::Borrowed(s);
    }
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => std::borrow::Cow::Owned(format!("{}…", &s[..idx])),
        None => std::borrow::Cow::Borrowed(s),
    }
}

/// A chip of the page's toolbar (24 high, on a hairline): an icon in `color` (`filled`), and a
/// count; `lit` while hovered or what it opens shows.
fn paint_chip(ui: &Ui, r: Rect, lit: bool, icon: &str, filled: bool, color: Color32, count: Option<&str>) {
    let p = style::pal(ui);
    let painter = ui.painter();
    painter.rect(r, CornerRadius::same(style::RADIUS), if lit { p.hover } else { Color32::TRANSPARENT }, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    let font = if filled { style::icon_fill(12.0) } else { style::icon_font(12.0) };
    painter.text(pos2(r.left() + 7.0 + 6.0, r.center().y), Align2::CENTER_CENTER, icon, font, color);
    if let Some(n) = count {
        painter.text(pos2(r.left() + 7.0 + 12.0 + 5.0, r.center().y), Align2::LEFT_CENTER, n, style::mono(style::DATA), if lit { p.text } else { p.text_muted });
    }
}

/// A notice that takes the keyboard when clicked (its buttons keep their own clicks: they are
/// added after it). Returns whether it has the keyboard.
fn notice_keys(ui: &mut Ui, rect: Rect, id: egui::Id) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    if resp.clicked() {
        resp.request_focus();
    }
    if resp.has_focus() {
        ui.memory_mut(|m| m.set_focus_lock_filter(id, egui::EventFilter { escape: true, ..Default::default() }));
    }
    resp.has_focus() || ui.memory(|m| m.had_focus_last_frame(id))
}

/// Where `kit::notice` in `rect` put its primary and secondary buttons (as it lays them out:
/// from the right, a button dropped when the text would get too little room).
fn notice_buttons(ui: &Ui, rect: Rect, primary: Option<(&str, Option<&str>)>, secondary: Option<&str>) -> (Option<Rect>, Option<Rect>) {
    let cy = rect.center().y;
    let text_left = rect.left() + 40.0;
    let min_text = kit::NOTICE_MIN_TEXT;
    let mut right = rect.right() - 8.0;
    let at = |right: f32, w: f32| Rect::from_min_max(pos2(right - w, cy - 12.0), pos2(right, cy + 12.0));
    let mut pr = None;
    if let Some((label, keys)) = primary {
        let w = kit::button_width(ui, None, label, keys, true);
        if right - w - text_left > min_text * 0.75 {
            pr = Some(at(right, w));
            right -= w + 6.0;
        }
    }
    let mut sr = None;
    if let Some(label) = secondary {
        let w = kit::button_width(ui, None, label, None, true);
        if right - w - text_left > min_text {
            sr = Some(at(right, w));
        }
    }
    (pr, sr)
}

/// Where the end of a notice's second line, from `keys` on ("⌘↵ always"), is drawn: a notice
/// in `rect` with `title` and `body`, its leftmost button `button`. `None` while it is cut short.
fn always_link(ui: &Ui, rect: Rect, title: &str, body: &str, keys: &str, button: Option<Rect>) -> Option<Rect> {
    let w = |s: &str, f: egui::FontId| ui.painter().layout_no_wrap(s.to_string(), f, Color32::PLACEHOLDER).size();
    let tx = rect.left() + 38.0;
    let right = button.map_or(rect.right() - 8.0, |b| b.left() - 6.0);
    let max_w = (right - tx - 8.0).max(20.0);
    let font = style::mono(style::DATA);
    let full = w(body, font.clone());
    if full.x > max_w {
        return None;
    }
    let at = body.rfind(keys)?;
    let before = w(&body[..at], font.clone()).x;
    let after = w(&body[at..], font).x;
    let th = w(title, style::medium(style::SMALL)).y;
    let top = rect.center().y - (th + 2.0 + full.y) / 2.0 + th + 2.0;
    Some(Rect::from_min_size(pos2(tx + before, top), vec2(after, full.y)))
}

/// A find bar's parts: its field's id and text, the hint, how many matches (and in what
/// colour), how wide its field is, whether it takes the keyboard now, and a shadow (floating
/// over what it searches).
pub(crate) struct FindCard<'a> {
    pub id: egui::Id,
    pub query: &'a mut String,
    pub hint: &'a str,
    pub count: String,
    pub count_color: Color32,
    pub field_w: f32,
    pub focus: bool,
    pub shadow: bool,
}

/// What the user did with a find bar this frame.
pub(crate) struct FindOut {
    pub changed: bool,
    /// The next match (1) or the one before (-1).
    pub step: Option<i32>,
    pub close: bool,
    /// Its field has the keyboard.
    pub focused: bool,
    /// Where its field is (for the dev scripts' clicks).
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    pub field: Rect,
}

/// The find bar of pages and documents, as the terminals' (36 high, on the menu colour,
/// radius 9): the magnifier, the field, how many matches ("2 of 5"), ↑ ↓ and ×, at `at`, at
/// most `max_w` wide. Enter: the next match, ⇧Enter: the one before; Esc (`esc`): close.
pub(crate) fn find_card(ui: &mut Ui, at: egui::Pos2, max_w: f32, c: FindCard, esc: bool) -> FindOut {
    let p = style::pal(ui);
    let cg = ui.painter().layout_no_wrap(c.count.clone(), style::mono(style::DATA), c.count_color);
    let count_w = if c.count.is_empty() { 0.0 } else { cg.size().x.max(40.0) };
    let field_w = c.field_w.min(max_w - FIND_FIXED - count_w).max(40.0);
    let r = Rect::from_min_size(at, vec2((FIND_FIXED + field_w + count_w).min(max_w.max(120.0)), FIND_H));
    let radius = CornerRadius::same(9);
    if c.shadow {
        ui.painter().add(ui.visuals().popup_shadow.as_shape(r, radius));
    }
    ui.painter().rect(r, radius, p.menu, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
    // Clicks on it stay with it (not what it floats over).
    ui.interact(r, c.id.with("card"), Sense::click());
    let cy = r.center().y;
    let mut x = r.left() + 10.0;
    ui.painter().text(pos2(x + 7.0, cy), Align2::CENTER_CENTER, icons::MAGNIFYING_GLASS, style::icon_font(14.0), p.text_faint);
    x += 14.0 + 6.0;
    let field = Rect::from_min_max(pos2(x, r.top() + 4.0), pos2(x + field_w, r.bottom() - 4.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let resp = child.add_sized(
        field.size(),
        egui::TextEdit::singleline(c.query)
            .id(c.id)
            .char_limit(1_024)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(egui::FontId::proportional(style::BODY))
            .hint_text(egui::RichText::new(c.hint).color(p.text_faint))
            .vertical_align(egui::Align::Center),
    );
    x += field_w + 6.0;
    if count_w > 0.0 {
        ui.painter().galley(pos2(x + count_w - cg.size().x, cy - cg.size().y / 2.0), cg, c.count_color);
    }
    x += count_w + 6.0;
    ui.painter().vline(x + 0.5, (cy - 8.0)..=(cy + 8.0), Stroke::new(1.0, p.border));
    x += 1.0 + 6.0;
    let mut out = FindOut { changed: resp.changed(), step: None, close: false, focused: false, field };
    if c.focus {
        resp.request_focus();
        kit::select_text(ui.ctx(), c.id, 0, c.query.chars().count());
    }
    let focused = resp.has_focus() || c.focus;
    if resp.has_focus() {
        ui.memory_mut(|m| m.set_focus_lock_filter(c.id, egui::EventFilter { escape: true, horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
    }
    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        out.step = Some(if ui.input(|i| i.modifiers.shift) { -1 } else { 1 });
        resp.request_focus();
    }
    let mut button = |ui: &mut Ui, icon: &str, tip: &str| {
        let b = Rect::from_min_size(pos2(x, cy - ICON_S / 2.0), vec2(ICON_S, ICON_S));
        x += ICON_S + 6.0;
        ui.scope_builder(egui::UiBuilder::new().max_rect(b), |ui| kit::icon_button_ex(ui, icon, tip, false, ICON_S)).inner.clicked()
    };
    if button(ui, icons::CARET_UP, t("Previous")) {
        out.step = Some(-1);
    }
    if button(ui, icons::CARET_DOWN, t("Next")) {
        out.step = Some(1);
    }
    out.close = button(ui, icons::X, t("Close"))
        || (esc && (resp.has_focus() || ui.memory(|m| m.had_focus_last_frame(c.id))) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)));
    out.focused = focused || out.step.is_some();
    out
}

/// A find bar's width but its field's and its count's: the magnifier, the line, ↑ ↓ ×, with
/// their gaps and the sides.
const FIND_FIXED: f32 = 10.0 + 14.0 + 6.0 + 6.0 + 6.0 + 1.0 + 6.0 + 3.0 * ICON_S + 2.0 * 6.0 + 4.0;

/// How wide [`find_card`] is with `count` and its field `field_w` wide.
pub(crate) fn find_card_width(ui: &Ui, count: &str, field_w: f32) -> f32 {
    let count_w = if count.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(count.to_string(), style::mono(style::DATA), Color32::PLACEHOLDER).size().x.max(40.0) };
    FIND_FIXED + field_w + count_w
}

/// How wide [`segment`] draws `icon` and `text`.
pub(crate) fn segment_width(ui: &Ui, icon: Option<&str>, text: &str) -> f32 {
    let w = ui.painter().layout_no_wrap(text.to_string(), style::medium(style::SMALL), Color32::PLACEHOLDER).size().x;
    10.0 + icon.map_or(0.0, |_| 13.0 + 6.0) + w + 10.0
}

/// One segment standing alone (24 high): `on`, raised on a hairline; else quiet words that a
/// hover lights (the console's All / Errors, a workbook's sheets).
pub(crate) fn segment(ui: &mut Ui, rect: Rect, id: egui::Id, icon: Option<&str>, text: &str, on: bool) -> egui::Response {
    let p = style::pal(ui);
    let resp = ui.interact(rect, id, Sense::click());
    if resp.hovered() && !on {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, text));
    let painter = ui.painter();
    let radius = CornerRadius::same(style::RADIUS);
    if on {
        painter.rect_filled(rect.translate(vec2(0.0, 1.0)), radius, Color32::from_black_alpha(if ui.visuals().dark_mode { 64 } else { 18 }));
        painter.rect(rect, radius, p.surface, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        painter.rect_filled(rect, radius, p.hover);
    }
    let color = if on || resp.hovered() { p.text } else { p.text_muted };
    let mut x = rect.left() + 10.0;
    if let Some(i) = icon {
        painter.text(pos2(x + 6.5, rect.center().y), Align2::CENTER_CENTER, i, style::icon_font(13.0), color);
        x += 13.0 + 6.0;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), style::medium(style::SMALL), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - 10.0 - x).max(8.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, color);
    resp
}

/// A file's icon and its colour, by its name's extension (a download's row).
fn file_icon(name: &str, p: &style::Palette) -> (&'static str, Color32) {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "csv" | "tsv" => (icons::FILE_CSV, p.ok),
        "xls" | "xlsx" | "ods" => (icons::FILE_XLS, p.ok),
        "pdf" => (icons::FILE_PDF, p.err),
        "zip" | "gz" | "tgz" | "tar" | "7z" | "rar" | "dmg" => (icons::FILE_ZIP, p.text_faint),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" => (icons::FILE_IMAGE, p.accent_text),
        "doc" | "docx" | "pages" => (icons::FILE_DOC, p.accent_text),
        "ppt" | "pptx" | "key" => (icons::FILE_PPT, p.rate_limited),
        "txt" | "md" | "log" => (icons::FILE_TEXT, p.text_faint),
        _ => (icons::FILE, p.text_faint),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_frame_snaps_to_exact_physical_pixels_at_all_dpi_scales() {
        for ppp in [1.0_f32, 1.25, 1.5, 1.75, 2.0, 2.5] {
            let raw = Rect::from_min_max(pos2(650.33, 91.7), pos2(1271.4, 790.6));
            let frame = snap_rect_px(raw, ppp);
            let border = snap_stroke_pt(1.0, ppp);
            let page = frame.shrink(border);
            for edge in [frame.left(), frame.top(), frame.right(), frame.bottom(), page.left(), page.top(), page.right(), page.bottom()] {
                let px = edge * ppp;
                assert!((px - px.round()).abs() < 1e-3, "edge {edge} at ppp={ppp} should be integer px (got {px})");
            }
            let border_px = border * ppp;
            assert!((border_px - border_px.round()).abs() < 1e-3 && border_px >= 1.0);
            assert!(page.left() > frame.left() && page.right() < frame.right());
            assert!(page.top() > frame.top() && page.bottom() < frame.bottom());
        }
        #[cfg(target_os = "macos")]
        assert_eq!(WEB_FRAME_ROUND, PAGE_ROUND);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(WEB_FRAME_ROUND, 0.0);
    }
}
