//! The browser pane: its toolbar (back, forward, reload, the address, the terminal that serves
//! the page, open in the default browser) over its page; opening pages in it; and keeping each
//! page's web view where its pane is drawn, hidden when the pane is out of view or something is
//! drawn over it.

use std::time::Duration;

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::PaneId;

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::browser::{Answer, Ask, Browser, Dialog, DialogKind, Driver, Event};
use crate::i18n::{t, tf};

/// The toolbar at the top of a browser pane (its header, also in a split).
pub const BAR_H: f32 = 36.0;
/// How far a page's web view keeps from its pane's sides and foot (see `browser_pane`).
const WEB_INSET: f32 = 3.0;
/// The bar under it while an agent uses the page.
const AGENT_H: f32 = 34.0;
/// The question about a site an agent's step would open.
const ASK_H: f32 = 84.0;

impl App {
    /// What a pane is called (a tab, a tile): a terminal's label, or a browser's page.
    pub fn leaf_label(&self, id: PaneId) -> String {
        match (self.panes.get(&id), self.browsers.get(&id)) {
            (Some(p), _) => p.label(),
            (None, Some(b)) => b.label(),
            _ => String::new(),
        }
    }

    /// A browser pane in `rect`: its toolbar, then its page (the web view is laid over it once
    /// the frame is drawn; meanwhile, or asleep, a placeholder).
    pub fn browser_pane(&mut self, ui: &mut Ui, id: PaneId, rect: Rect, focused: bool, single: bool) {
        let p = style::pal(ui);
        let Some(b) = self.browsers.get(&id) else { return };
        let project = b.project;
        let driver = b.driver.clone();
        let ask = b.ask.clone();
        let dialog = b.dialog.clone();
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), BAR_H));
        let mut top = bar.bottom();
        let painter = ui.painter();
        painter.rect_filled(bar, CornerRadius::ZERO, if focused && !single { p.surface } else { p.chrome });
        painter.hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, p.border));
        if focused && !single && self.window_focused {
            painter.hline(bar.x_range(), bar.top() + 1.0, Stroke::new(2.0, p.accent));
        }
        // A click on the bar makes the pane the focused one (⌘W closes it, splits go beside it).
        let bar_resp = ui.interact(bar, egui::Id::new(("browser-bar", id)), Sense::click());
        if bar_resp.clicked() && !focused {
            self.actions.push(Action::FocusPane(id));
        }

        // The terminal whose server this page comes from (same port on this machine).
        let (host, _) = crate::browser::split_url(&b.url);
        let port = host.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok());
        let from = port.and_then(|port| self.project_servers(project).into_iter().find(|(_, s)| s.port == port)).map(|(pane, _)| (pane, self.leaf_label(pane)));
        let url = b.url.clone();
        let loading = b.loading;

        // Right side first: what is left goes to the address. In a narrow pane the address keeps
        // its room: the terminal serving the page, the console's chip and ↗ (also in ⋯) go first.
        let inner = bar.shrink2(vec2(6.0, 0.0));
        const MIN_FIELD: f32 = 90.0;
        let field_left = inner.left() + 3.0 * 24.0 + 4.0;
        let fits = |right: f32, w: f32| right - w - 4.0 - field_left >= MIN_FIELD;
        let mut right = inner.right();
        let more = Rect::from_center_size(pos2(right - 11.0, inner.center().y), vec2(22.0, 22.0));
        right = more.left() - 4.0;
        let ext = fits(right, 22.0).then(|| Rect::from_center_size(pos2(more.left() - 13.0, inner.center().y), vec2(22.0, 22.0)));
        if let Some(e) = ext {
            right = e.left() - 4.0;
        }
        // The console's warnings and errors, when there are any (a click opens its drawer).
        let (errors, warnings) = self.browsers.get(&id).map_or((0, 0), |b| {
            (b.console.iter().filter(|l| l.is_error()).count(), b.console.iter().filter(|l| l.is_warning()).count())
        });
        let console_open = self.browsers.get(&id).is_some_and(|b| b.console_open);
        let cons_rect = (errors + warnings > 0 || console_open).then(|| {
            let n = if errors + warnings > 0 { (errors + warnings).to_string() } else { String::new() };
            let g = ui.painter().layout_no_wrap(n, egui::FontId::proportional(style::SMALL), p.text);
            let w = if g.size().x > 0.0 { g.size().x + 30.0 } else { 26.0 };
            fits(right, w).then(|| {
                let r = Rect::from_min_max(pos2(right - w, inner.center().y - 11.0), pos2(right, inner.center().y + 11.0));
                right = r.left() - 4.0;
                r
            })
        }).flatten();
        // What the page downloaded (a click lists it): blue while one runs, red when the last
        // failed, green when all came.
        let (dl_count, dl_running, dl_failed) = self.browsers.get(&id).map_or((0, false, false), |b| {
            (b.downloads.len(), b.downloads.iter().any(|d| d.done.is_none()), b.downloads.last().is_some_and(|d| d.done == Some(false)))
        });
        let dl_rect = (dl_count > 0).then(|| {
            let g = ui.painter().layout_no_wrap(dl_count.to_string(), egui::FontId::proportional(style::SMALL), p.text);
            let w = g.size().x + 30.0;
            fits(right, w).then(|| {
                let r = Rect::from_min_max(pos2(right - w, inner.center().y - 11.0), pos2(right, inner.center().y + 11.0));
                right = r.left() - 4.0;
                r
            })
        }).flatten();
        let from_rect = from.as_ref().and_then(|(_, label)| {
            let g = ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(style::SMALL), p.text_muted);
            let w = (g.size().x + 26.0).min(160.0);
            fits(right, w).then(|| {
                let r = Rect::from_min_max(pos2(right - w, inner.center().y - 11.0), pos2(right, inner.center().y + 11.0));
                right = r.left() - 4.0;
                r
            })
        });
        let mut x = inner.left();
        let mut nav = |icon: &str, tip: &str| {
            let r = Rect::from_center_size(pos2(x + 11.0, inner.center().y), vec2(22.0, 22.0));
            x += 24.0;
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::icon_button_ex(ui, icon, tip, false, 22.0)).inner.clicked()
        };
        let back = nav(icons::ARROW_LEFT, t("Back"));
        let forward = nav(icons::ARROW_RIGHT, t("Forward"));
        let reload = nav(icons::ARROWS_CLOCKWISE, t("Reload"));
        let field = Rect::from_min_max(pos2(x + 4.0, inner.center().y - 13.0), pos2(right.max(x + 60.0), inner.center().y + 13.0));
        let fid = egui::Id::new(("browser-address", id));
        let typing = ui.memory(|m| m.has_focus(fid));
        // Asked for (⌘L, the Browser button, a new empty browser): the address as shown, all of
        // it selected, so that what is typed replaces it.
        if self.browser_edit == Some(id) {
            self.browser_edit = None;
            let shown = self.browsers.get(&id).map(|b| b.url.trim_end_matches('/').trim_start_matches("http://").to_string()).unwrap_or_default();
            kit::select_text(ui.ctx(), fid, 0, shown.chars().count());
            if let Some(b) = self.browsers.get_mut(&id) {
                b.address = shown;
            }
            ui.memory_mut(|m| m.request_focus(fid));
        }
        let typing = typing || ui.memory(|m| m.has_focus(fid));
        let mut text = match self.browsers.get(&id) {
            Some(b) if typing => b.address.clone(),
            Some(b) => b.url.trim_end_matches('/').trim_start_matches("http://").to_string(),
            None => String::new(),
        };
        let icon = if url.starts_with("https://") { icons::LOCK_SIMPLE } else { icons::GLOBE };
        let resp = ui
            .scope_builder(egui::UiBuilder::new().max_rect(field), |ui| {
                kit::field(ui, fid, &mut text, Some(icon), t("Search or enter an address"), field.width(), true)
            })
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser-address:{id}"), field);
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if loading {
            // A thin line under the bar while the page loads.
            let w = bar.width() * 0.35;
            let t0 = ui.input(|i| i.time) as f32;
            let x0 = bar.left() + (t0 * 420.0) % (bar.width() + w) - w;
            ui.painter().with_clip_rect(bar).hline(x0..=x0 + w, bar.bottom() - 1.0, Stroke::new(2.0, p.accent));
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
        if let Some((pane, _)) = &from
            && let Some(b) = self.browsers.get_mut(&id)
        {
            b.served_by = Some(*pane);
        }
        if let Some(((pane, label), r)) = from.zip(from_rect) {
            let resp = ui.interact(r, egui::Id::new(("browser-from", id)), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(style::RADIUS), p.hover);
            }
            ui.painter().text(pos2(r.left() + 7.0 + 6.0, r.center().y), Align2::CENTER_CENTER, icons::TERMINAL, style::icon_font(12.0), p.text_faint);
            let mut job = egui::text::LayoutJob::simple_singleline(label, egui::FontId::proportional(style::SMALL), p.text_muted);
            job.wrap = egui::text::TextWrapping::truncate_at_width(r.width() - 26.0);
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(r.left() + 21.0, r.center().y - g.size().y / 2.0), g, p.text_muted);
            if resp.on_hover_text(t("Served by this terminal: show it")).clicked() {
                self.actions.push(Action::FocusPane(pane));
            }
        }
        let open_ext = ext.is_some_and(|ext| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(ext), |ui| kit::icon_button_ex(ui, icons::ARROW_SQUARE_OUT, t("Open in Default Browser"), false, 22.0))
                .inner
                .clicked()
        });
        let more = ui.scope_builder(egui::UiBuilder::new().max_rect(more), |ui| kit::icon_button_ex(ui, icons::DOTS_THREE, t("More"), false, 22.0)).inner;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser-more:{id}"), more.rect);
        egui::Popup::menu(&more).align(egui::RectAlign::BOTTOM_END).width(230.0).show(|ui| self.browser_menu(ui, id));
        if let Some(r) = dl_rect {
            let resp = ui.interact(r, egui::Id::new(("browser-downloads", id)), Sense::click());
            let c = if dl_running { p.info } else if dl_failed { p.err } else { p.ok };
            let dark = ui.visuals().dark_mode;
            ui.painter().rect_filled(r, CornerRadius::same(style::RADIUS), if resp.hovered() { style::tint(c, dark).gamma_multiply(1.6) } else { style::tint(c, dark) });
            ui.painter().text(pos2(r.left() + 13.0, r.center().y), Align2::CENTER_CENTER, icons::DOWNLOAD_SIMPLE, style::icon_font(12.0), c);
            ui.painter().text(pos2(r.left() + 23.0, r.center().y), Align2::LEFT_CENTER, dl_count.to_string(), style::medium(style::SMALL), c);
            if dl_running {
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("browser-downloads:{id}"), r);
            let resp = resp.on_hover_text(t("Downloads"));
            egui::Popup::menu(&resp).align(egui::RectAlign::BOTTOM_END).width(300.0).show(|ui| self.downloads_menu(ui, id));
        }
        bar_resp.context_menu(|ui| self.browser_menu(ui, id));
        let mut navigated = false;
        if let Some(b) = self.browsers.get_mut(&id) {
            if typing || resp.changed() {
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
        // The user moving the page while an agent uses it takes it over.
        if (navigated || back || forward || reload) && driver.as_ref().is_some_and(|d| !d.paused) {
            self.take_over_browser(id, true);
        }
        if open_ext && !url.is_empty() {
            crate::platform::open_url(&url);
        }

        // Who uses the page, and a question about a site.
        if let Some(d) = &driver {
            let r = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), AGENT_H));
            self.agent_bar(ui, id, r, d);
            top = r.bottom();
        }
        if let Some(a) = &ask {
            let r = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), ASK_H));
            let who = driver.as_ref().map_or_else(|| t("The agent").to_string(), |d| d.who.clone());
            let name = self.project(project).map(|x| x.name.clone()).unwrap_or_default();
            self.ask_card(ui, id, r, a, &who, &name);
            top = r.bottom();
        }
        // A dialog of the page (alert, confirm, prompt): the page waits for its answer.
        if let Some(d) = &dialog {
            top += self.dialog_bar(ui, id, pos2(rect.left(), top), rect.width(), d);
        }
        // The console's chip in the toolbar, and its drawer at the foot of the pane.
        if let Some(r) = cons_rect {
            let resp = ui.interact(r, egui::Id::new(("browser-console", id)), Sense::click());
            let c = if errors > 0 { p.err } else if warnings > 0 { p.warn } else { p.text_muted };
            let fill = if resp.hovered() || console_open { style::tint(c, ui.visuals().dark_mode).gamma_multiply(1.6) } else { style::tint(c, ui.visuals().dark_mode) };
            ui.painter().rect_filled(r, CornerRadius::same(style::RADIUS), fill);
            let icon = if errors > 0 { icons::WARNING_CIRCLE } else if warnings > 0 { icons::WARNING } else { icons::TERMINAL_WINDOW };
            ui.painter().text(pos2(r.left() + 13.0, r.center().y), Align2::CENTER_CENTER, icon, style::icon_font(12.0), c);
            if errors + warnings > 0 {
                ui.painter().text(pos2(r.left() + 23.0, r.center().y), Align2::LEFT_CENTER, (errors + warnings).to_string(), style::medium(style::SMALL), c);
            }
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("browser-console:{id}"), r);
            if resp.on_hover_text(t("Console")).clicked()
                && let Some(b) = self.browsers.get_mut(&id)
            {
                b.console_open = !b.console_open;
            }
        }
        let mut bottom = rect.bottom();
        if self.browsers.get(&id).is_some_and(|b| b.console_open) {
            let h = (rect.height() * 0.32).clamp(110.0, 260.0).min((rect.bottom() - top - 60.0).max(0.0));
            if h >= 60.0 {
                let r = Rect::from_min_max(pos2(rect.left(), rect.bottom() - h), rect.max);
                self.console_drawer(ui, id, r);
                bottom = r.top();
            }
        }
        let mut page = Rect::from_min_max(pos2(rect.left(), top.min(bottom)), pos2(rect.right(), bottom));

        // The page: the web view goes here. Until then, or without one, what it is.
        ui.painter().rect_filled(page, CornerRadius::ZERO, p.canvas);
        // A 2px info frame while an agent has it (the web view sits inside).
        if driver.as_ref().is_some_and(|d| !d.paused) {
            ui.painter().rect_stroke(page, CornerRadius::ZERO, Stroke::new(2.0, style::soft(p.info, 0.65)), egui::StrokeKind::Inside);
            page = page.shrink(2.0);
        }
        // Its server does not answer: which terminal served it, and Reload.
        if self.browsers.get(&id).is_some_and(|b| b.down) {
            self.down_card(ui, id, page, &host);
        }
        // Covered (the palette, a dialog, a menu): the page's picture in its place.
        if let Some(b) = self.browsers.get(&id)
            && b.covered_since.is_some()
            && !b.shown
            && let Some((tex, _)) = &b.still
        {
            let size = tex.size_vec2();
            let h = page.width() * size.y / size.x.max(1.0);
            let shown = Rect::from_min_size(page.min, vec2(page.width(), h));
            ui.painter().with_clip_rect(page).image(tex.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        if !self.browsers.get(&id).is_some_and(Browser::live) {
            let label = if url.is_empty() { t("No page yet: type an address above.").to_string() } else { tf("Loading {}…", &[&host]) };
            ui.painter().text(page.center() - vec2(0.0, 14.0), Align2::CENTER_CENTER, icons::GLOBE, style::icon_font(26.0), p.text_faint);
            ui.painter().text(page.center() + vec2(0.0, 16.0), Align2::CENTER_CENTER, label, egui::FontId::proportional(style::SMALL), p.text_muted);
            #[cfg(not(target_os = "macos"))]
            ui.painter().text(page.center() + vec2(0.0, 36.0), Align2::CENTER_CENTER, t("Browser panes show pages on macOS for now: ↗ opens it in your browser."), egui::FontId::proportional(style::SMALL), p.text_faint);
        }
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser:{id}"), page);
        // The web view keeps clear of the pane's sides and foot: a divider (or the Code drawer's
        // edge, the sidebar's) is dragged by a strip either side of it, and the page, always
        // drawn over the app, would take its half.
        let web = Rect::from_min_max(pos2(page.left() + WEB_INSET, page.top()), pos2(page.right() - WEB_INSET, page.bottom() - WEB_INSET));
        self.browser_pages.insert(id, web);
        if self.focus_request == Some(id) {
            self.focus_request = None;
            self.browser_focus_request = Some(id);
        }
    }

    /// The page's server (on this machine) does not answer: said in the page's place, with the
    /// terminal that served it (Go to Pane) and Reload. The page comes back by itself.
    fn down_card(&mut self, ui: &mut Ui, id: PaneId, page: Rect, host: &str) {
        let p = style::pal(ui);
        let served = self.browsers.get(&id).and_then(|b| b.served_by).filter(|pane| self.panes.contains_key(pane));
        let why = match served.and_then(|pane| self.panes.get(&pane)) {
            Some(pane) => match pane.tracker.status {
                tc_core::status::Status::Failed(code) => tf("The pane that served it, {}, exited with code {}.", &[&pane.label(), &code.to_string()]),
                _ => tf("The pane that served it, {}, is not serving it now.", &[&pane.label()]),
            },
            None => t("Nothing is serving it on this machine.").to_string(),
        };
        let c = page.center();
        let painter = ui.painter();
        painter.text(c - vec2(0.0, 52.0), Align2::CENTER_CENTER, icons::PLUGS, style::icon_font(30.0), p.text_faint);
        painter.text(c - vec2(0.0, 16.0), Align2::CENTER_CENTER, tf("{} is not answering", &[host]), style::semibold(14.0), p.text);
        let mut job = egui::text::LayoutJob::simple(format!("{why} {}", t("The page loads again once it answers.")), egui::FontId::proportional(12.5), p.text_muted, (page.width() - 48.0).clamp(120.0, 380.0));
        job.halign = egui::Align::Center;
        let g = painter.layout_job(job);
        let text_h = g.size().y;
        painter.galley(pos2(c.x, c.y + 4.0), g, p.text_muted);
        // The buttons, centred: their row is exactly as wide as they are.
        let go_w = if served.is_some() { kit::button_width(ui, Some(icons::TERMINAL), t("Go to Pane"), None, true) + 6.0 } else { 0.0 };
        let w = go_w + kit::button_width(ui, None, t("Reload"), None, true);
        let row = Rect::from_center_size(pos2(c.x, c.y + 4.0 + text_h + 24.0), vec2(w, 28.0));
        let mut go = false;
        let mut reload = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if served.is_some() {
                go = kit::button_ex(ui, kit::Kind::Secondary, Some(icons::TERMINAL), t("Go to Pane"), true).clicked();
            }
            reload = kit::button_ex(ui, kit::Kind::Ghost, None, t("Reload"), true).clicked();
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

    /// The page's console at the foot of its pane: errors first, then the rest as they came.
    fn console_drawer(&mut self, ui: &mut Ui, id: PaneId, r: Rect) {
        let p = style::pal(ui);
        let Some(b) = self.browsers.get(&id) else { return };
        let errors = b.console.iter().filter(|l| l.is_error()).count();
        let warnings = b.console.iter().filter(|l| l.is_warning()).count();
        let mut lines: Vec<&crate::browser::ConsoleLine> = b.console.iter().filter(|l| l.is_error()).collect();
        lines.extend(b.console.iter().filter(|l| !l.is_error()));
        let lines: Vec<crate::browser::ConsoleLine> = lines.into_iter().cloned().collect();
        let painter = ui.painter_at(r);
        painter.rect_filled(r, CornerRadius::ZERO, p.chrome);
        painter.hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border));
        let head = Rect::from_min_size(r.min, vec2(r.width(), 28.0));
        painter.hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.border));
        painter.text(pos2(head.left() + 18.0, head.center().y), Align2::CENTER_CENTER, icons::TERMINAL_WINDOW, style::icon_font(13.0), p.text_muted);
        let g = painter.layout_no_wrap(t("Console").to_string(), style::medium(12.0), p.text_muted);
        let mut x = head.left() + 30.0;
        painter.galley(pos2(x, head.center().y - g.size().y / 2.0), g.clone(), p.text_muted);
        x += g.size().x + 8.0;
        for (n, c) in [(errors, p.err), (warnings, p.warn)] {
            if n > 0 {
                let g = ui.painter().layout_no_wrap(n.to_string(), style::semibold(10.0), p.on_accent);
                let w = (g.size().x + 10.0).max(16.0);
                kit::paint_badge(ui.painter(), pos2(x + w, head.center().y), &n.to_string(), c, p.on_accent, 15.0);
                x += w + 5.0;
            }
        }
        // Clear and close, at the right.
        let mut clear = false;
        let mut close = false;
        let close_r = Rect::from_center_size(pos2(head.right() - 17.0, head.center().y), vec2(22.0, 22.0));
        let clear_r = Rect::from_center_size(pos2(close_r.left() - 13.0, head.center().y), vec2(22.0, 22.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(clear_r), |ui| clear = kit::icon_button_ex(ui, icons::TRASH, t("Clear"), false, 22.0).clicked());
        ui.scope_builder(egui::UiBuilder::new().max_rect(close_r), |ui| close = kit::icon_button_ex(ui, icons::X, t("Close"), false, 22.0).clicked());
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser-console-drawer:{id}"), r);
        // The lines.
        let list = Rect::from_min_max(pos2(r.left(), head.bottom()), r.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            egui::ScrollArea::vertical().id_salt(("browser-console-lines", id)).auto_shrink([false, false]).show(ui, |ui| {
                if lines.is_empty() {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(t("Nothing in the console yet.")).size(style::SMALL).color(p.text_faint));
                    });
                }
                for l in &lines {
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
                    let painter = ui.painter();
                    painter.hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, p.border));
                    let (icon, c) = match l.level.as_str() {
                        "error" => (Some(icons::WARNING_CIRCLE), p.err),
                        "warn" => (Some(icons::WARNING), p.warn),
                        _ => (None, p.text_faint),
                    };
                    if let Some(i) = icon {
                        painter.text(pos2(row.left() + 18.0, row.center().y), Align2::CENTER_CENTER, i, style::icon_font(12.5), c);
                    }
                    let src_w = if l.src.is_empty() {
                        0.0
                    } else {
                        let g = painter.layout_no_wrap(l.src.clone(), egui::FontId::monospace(11.0), p.text_faint);
                        let w = g.size().x;
                        painter.galley(pos2(row.right() - 12.0 - w, row.center().y - g.size().y / 2.0), g, p.text_faint);
                        w + 16.0
                    };
                    let mut job = egui::text::LayoutJob::simple_singleline(l.text.lines().next().unwrap_or("").to_string(), egui::FontId::monospace(11.5), p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((row.width() - 30.0 - 12.0 - src_w).max(20.0));
                    let g = painter.layout_job(job);
                    painter.galley(pos2(row.left() + 30.0, row.center().y - g.size().y / 2.0), g, p.text);
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

    /// The bar saying which agent uses the page and what it does, with Take Over; once the
    /// user took it, Hand Back.
    fn agent_bar(&mut self, ui: &mut Ui, id: PaneId, r: Rect, d: &Driver) {
        let p = style::pal(ui);
        let painter = ui.painter_at(r);
        let (fill, line) = if d.paused { (p.surface, p.border) } else { (style::over(p.surface, p.info, 0.09), style::over(p.surface, p.info, 0.3)) };
        painter.rect_filled(r, CornerRadius::ZERO, fill);
        painter.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, line));
        let (icon, color) = if d.paused { (icons::HAND_POINTING, p.text_muted) } else { (icons::CURSOR_CLICK, p.info) };
        painter.text(pos2(r.left() + 20.0, r.center().y), Align2::CENTER_CENTER, icon, style::icon_font(15.0), color);
        // The button at the right, then the words in what is left.
        let label = if d.paused { tf("Hand Back to {}", &[&d.who]) } else { t("Take Over").to_string() };
        let bicon = if d.paused { icons::PLAY } else { icons::HAND_POINTING };
        let area = Rect::from_min_max(pos2(r.left() + 36.0, r.top()), pos2(r.right() - 8.0, r.bottom()));
        let clicked = ui
            .scope_builder(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                let b = kit::button_ex(ui, kit::Kind::Secondary, Some(bicon), &label, true);
                #[cfg(debug_assertions)]
                crate::devtest::record(format!("browser-take-over:{id}"), b.rect);
                (b.clicked(), b.rect.left())
            })
            .inner;
        let font = egui::FontId::proportional(12.5);
        let mut job = egui::text::LayoutJob::default();
        let mut add = |text: &str, font: egui::FontId, color: egui::Color32| {
            job.append(text, 0.0, egui::TextFormat { font_id: font, color, ..Default::default() });
        };
        if d.paused {
            add(t("You have the browser."), style::medium(12.5), p.text);
            add("  ", font.clone(), p.text);
            add(&tf("{} is paused.", &[&d.who]), font, p.text_muted);
        } else {
            add(&d.who, style::semibold(12.5), p.text);
            add(" ", font.clone(), p.text);
            add(t("is using this browser"), font.clone(), p.text_muted);
            add("  ·  ", font.clone(), p.text_faint);
            add(&d.doing, font, p.text_muted);
        }
        job.wrap = egui::text::TextWrapping::truncate_at_width((clicked.1 - 10.0 - (r.left() + 36.0)).max(20.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(r.left() + 36.0, r.center().y - g.size().y / 2.0), g, p.text);
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser-agent:{id}"), r);
        if clicked.0 {
            self.take_over_browser(id, !d.paused);
        }
    }

    /// A page's dialog above it: who asks, the message, a field for a prompt, OK (and Cancel).
    /// Enter answers OK, Esc Cancel. Returns its height.
    fn dialog_bar(&mut self, ui: &mut Ui, id: PaneId, at: egui::Pos2, width: f32, d: &Dialog) -> f32 {
        let p = style::pal(ui);
        let x = at.x + 40.0;
        let text_w = (width - 52.0).max(60.0);
        let mut job = egui::text::LayoutJob::simple(d.message.clone(), egui::FontId::proportional(12.5), p.text, text_w);
        job.wrap.max_rows = 8;
        let msg = ui.painter().layout_job(job);
        let prompt = d.kind == DialogKind::Prompt;
        let h = 10.0 + 18.0 + msg.size().y + 10.0 + if prompt { 34.0 } else { 0.0 } + 26.0 + 12.0;
        let r = Rect::from_min_size(at, vec2(width, h));
        let painter = ui.painter_at(r);
        painter.rect_filled(r, CornerRadius::ZERO, style::over(p.surface, p.info, 0.07));
        painter.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, style::over(p.surface, p.info, 0.3)));
        painter.text(pos2(r.left() + 22.0, r.top() + 19.0), Align2::CENTER_CENTER, icons::INFO, style::icon_font(17.0), p.info);
        let who = if prompt { tf("{} asks:", &[&d.host]) } else { tf("{} says:", &[&d.host]) };
        painter.text(pos2(x, r.top() + 10.0), Align2::LEFT_TOP, who, style::medium(12.5), p.text_muted);
        painter.galley(pos2(x, r.top() + 28.0), msg.clone(), p.text);
        let mut y = r.top() + 28.0 + msg.size().y + 10.0;
        let mut answer = None;
        let mut has_keys = false;
        let fid = egui::Id::new(("browser-dialog-field", id));
        let focus = std::mem::take(&mut self.dialog_focus) == Some(id);
        if prompt {
            let field = Rect::from_min_size(pos2(x, y), vec2((r.right() - 12.0 - x).min(360.0), 26.0));
            let mut text = d.text.clone();
            let resp = ui.scope_builder(egui::UiBuilder::new().max_rect(field), |ui| kit::field(ui, fid, &mut text, None, "", field.width(), false)).inner;
            if focus {
                resp.request_focus();
            }
            has_keys |= resp.has_focus();
            if resp.changed()
                && let Some(b) = self.browsers.get_mut(&id)
                && let Some(dd) = b.dialog.as_mut()
            {
                dd.text = text;
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                answer = Some(true);
            }
            y += 34.0;
        }
        let row = Rect::from_min_max(pos2(x, y), pos2(r.right() - 12.0, y + 26.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let ok = kit::button_ex(ui, kit::Kind::Primary, None, t("OK"), true);
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("browser-dialog-ok:{id}"), ok.rect);
            if focus && !prompt {
                ok.request_focus();
            }
            has_keys |= ok.has_focus();
            if ok.clicked() {
                answer = Some(true);
            }
            if d.kind != DialogKind::Alert {
                let cancel = kit::button_ex(ui, kit::Kind::Secondary, None, t("Cancel"), true);
                #[cfg(debug_assertions)]
                crate::devtest::record(format!("browser-dialog-cancel:{id}"), cancel.rect);
                has_keys |= cancel.has_focus();
                if cancel.clicked() {
                    answer = Some(false);
                }
            }
        });
        // Esc while the bar has the keyboard: Cancel (OK for an alert).
        if has_keys && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
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
        #[cfg(target_os = "macos")]
        crate::browser_dialog::answer(d.id, accept, &d.text);
        #[cfg(not(target_os = "macos"))]
        let _ = (d, accept);
        true
    }

    /// The list of what a page downloaded: newest first; a click opens the file, the magnifier
    /// shows it in the Finder.
    fn downloads_menu(&mut self, ui: &mut Ui, id: PaneId) {
        let p = style::pal(ui);
        let Some(b) = self.browsers.get(&id) else { return };
        let list: Vec<crate::browser::Download> = b.downloads.iter().rev().cloned().collect();
        ui.label(egui::RichText::new(t("Downloads")).font(style::semibold(style::SMALL)).color(p.text_faint));
        let mut clear = false;
        for d in &list {
            let name = d.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let meta = match d.done {
                None => t("Downloading…").to_string(),
                Some(false) => t("Download failed").to_string(),
                Some(true) => {
                    let size = std::fs::metadata(&d.path).map(|m| m.len()).unwrap_or(0);
                    format!("{} · {}", super::update_ui::human_size(size), super::dialogs::ago(std::time::SystemTime::now() - d.at.elapsed()))
                }
            };
            ui.horizontal(|ui| {
                let open = ui.add(egui::Button::new(egui::RichText::new(&name).font(style::medium(12.5))).frame(false)).on_hover_text(d.path.display().to_string());
                if open.clicked() && d.done == Some(true) {
                    crate::platform::reveal(&d.path);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if kit::icon_button_ex(ui, icons::MAGNIFYING_GLASS, t("Show in Finder"), false, 22.0).clicked() {
                        crate::platform::reveal_in_folder(&d.path);
                    }
                });
            });
            ui.label(egui::RichText::new(meta).font(egui::FontId::proportional(style::SMALL)).color(if d.done == Some(false) { p.err } else { p.text_muted }));
            ui.add_space(4.0);
        }
        ui.separator();
        if ui.button(t("Clear List")).clicked() {
            clear = true;
        }
        if clear && let Some(b) = self.browsers.get_mut(&id) {
            b.downloads.retain(|d| d.done.is_none());
        }
    }

    /// The question about a site an agent's step would open: Allow Once, Always for the
    /// project, Block.
    fn ask_card(&mut self, ui: &mut Ui, id: PaneId, r: Rect, a: &Ask, who: &str, project: &str) {
        let p = style::pal(ui);
        let painter = ui.painter_at(r);
        painter.rect_filled(r, CornerRadius::ZERO, style::over(p.surface, p.warn, 0.09));
        painter.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, style::over(p.surface, p.warn, 0.32)));
        painter.text(pos2(r.left() + 22.0, r.top() + 20.0), Align2::CENTER_CENTER, icons::SHIELD_WARNING, style::icon_font(17.0), p.warn);
        let x = r.left() + 40.0;
        let note = t("not on this project’s list");
        let note_w = if r.width() > 560.0 {
            let g = painter.layout_no_wrap(note.to_string(), egui::FontId::proportional(style::SMALL), p.text_faint);
            let w = g.size().x;
            painter.galley(pos2(r.right() - 12.0 - w, r.top() + 12.0), g, p.text_faint);
            w + 20.0
        } else {
            0.0
        };
        // The question, the site in bold.
        let q = if a.click { tf("{}’s click goes to {}", &[who, &a.host]) } else { tf("{} wants to open {}", &[who, &a.host]) };
        let mut job = egui::text::LayoutJob::default();
        let font = style::medium(12.5);
        match q.rfind(&a.host) {
            Some(at) => {
                job.append(&q[..at], 0.0, egui::TextFormat { font_id: font.clone(), color: p.text, ..Default::default() });
                job.append(&a.host, 0.0, egui::TextFormat { font_id: style::semibold(12.5), color: p.text, ..Default::default() });
                job.append(&q[at + a.host.len()..], 0.0, egui::TextFormat { font_id: font, color: p.text, ..Default::default() });
            }
            None => job.append(&q, 0.0, egui::TextFormat { font_id: font, color: p.text, ..Default::default() }),
        }
        job.wrap = egui::text::TextWrapping::truncate_at_width((r.right() - 12.0 - note_w - x).max(40.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(x, r.top() + 11.0), g, p.text);
        let mut url = egui::text::LayoutJob::simple_singleline(a.url.clone(), egui::FontId::monospace(11.5), p.text_muted);
        url.wrap = egui::text::TextWrapping::truncate_at_width((r.right() - 12.0 - x).max(40.0));
        let g = painter.layout_job(url);
        painter.galley(pos2(x, r.top() + 30.0), g, p.text_muted);
        let row = Rect::from_min_max(pos2(x, r.top() + 48.0), pos2(r.right() - 12.0, r.top() + 76.0));
        let mut answer = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let buttons = [
                (kit::Kind::Primary, t("Allow Once").to_string(), Answer::Once, "allow"),
                (kit::Kind::Secondary, tf("Always for {}", &[project]), Answer::Always, "always"),
                (kit::Kind::Ghost, t("Block").to_string(), Answer::Block, "block"),
            ];
            for (kind, label, ans, name) in buttons {
                let b = kit::button_ex(ui, kind, None, &label, true);
                #[cfg(debug_assertions)]
                crate::devtest::record(format!("browser-{name}:{id}"), b.rect);
                let _ = name;
                if b.clicked() {
                    answer = Some(ans);
                }
            }
        });
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser-ask:{id}"), r);
        if let Some(ans) = answer {
            self.answer_site(id, ans);
        }
    }

    /// A browser pane's menu (⋯ in its toolbar, or a right-click on it).
    fn browser_menu(&mut self, ui: &mut Ui, id: PaneId) {
        kit::menu_body(ui);
        let url = self.browsers.get(&id).map(|b| b.url.clone()).unwrap_or_default();
        if ui.add_enabled_ui(!url.is_empty(), |ui| kit::menu_item(ui, Some(icons::COPY), t("Copy Address"), None, false)).inner.clicked() {
            ui.ctx().copy_text(url.clone());
            ui.close();
        }
        if ui.add_enabled_ui(!url.is_empty(), |ui| kit::menu_item(ui, Some(icons::ARROW_SQUARE_OUT), t("Open in Default Browser"), None, false)).inner.clicked() {
            crate::platform::open_url(&url);
            ui.close();
        }
        kit::menu_sep(ui);
        if kit::menu_item(ui, Some(icons::X), t("Close Page"), kit::keys(&Action::ClosePane), true).clicked() {
            self.actions.push(Action::ClosePaneId(id));
            ui.close();
        }
    }

    /// Web pages open in TernaryCode (else in the default browser): browser panes show pages on
    /// macOS, and Settings → Browser says so.
    pub fn pages_here(&self) -> bool {
        cfg!(target_os = "macos") && self.settings.browser_here
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
        let tip = match &state {
            WebButton::None(Some(host)) => format!("{head}\n{}", tf("Opens {} beside the terminals", &[host])),
            WebButton::None(None) => format!("{head}\n{}", t("Opens an empty page beside the terminals")),
            WebButton::Elsewhere(label) => format!("{head}\n{}", tf("Shows the Browser panel (“{}”)", &[label])),
            WebButton::InView => format!("{head}\n{}", t("Hides the Browser panel · ⌘L: type an address")),
            WebButton::Agent(who, false) => format!("{}\n{}", tf("{} is using the browser", &[who]), t("Click to watch it · clicking or typing in it takes it over")),
            WebButton::Agent(who, true) => format!("{}\n{}", tf("{} is using the browser", &[who]), t("Hides the Browser panel · the agent keeps working")),
        };
        let tip = if self.pages_here() { format!("{tip}\n{}", t("⌥-click: a new page")) } else { tip };
        let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
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
        let server = self.project_servers(project).first().map(|(_, s)| s.url.clone());
        if !cfg!(target_os = "macos") {
            // Pages show in TernaryCode on macOS only: the default browser has the page.
            match server {
                Some(url) => crate::platform::open_url(&url),
                None => self.toast(t("Browser panes show pages on macOS for now.").to_string()),
            }
            return;
        }
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
        #[cfg(target_os = "macos")]
        crate::browser::clear_data();
        self.toast_ok(t("Logins, cookies and history were cleared.").to_string());
    }

    /// The page that has the keyboard gives it back to the app.
    pub fn release_page_keyboard(&mut self) {
        if let Some(id) = self.browser_focus.take() {
            #[cfg(target_os = "macos")]
            if let Some(v) = self.browsers.get(&id).and_then(|b| b.view.as_ref()) {
                let _ = v.focus_parent();
            }
            let _ = id;
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
        #[cfg(not(target_os = "macos"))]
        None
    }

    /// ⌘C, ⌘V, ⌘A while a page has the keyboard: sent to the page. Returns whether one has it.
    pub fn page_has_keys(&self, command: &str) -> bool {
        #[cfg(target_os = "macos")]
        if let Some(v) = self.browser_focus.and_then(|id| self.browsers.get(&id)).and_then(|b| b.view.as_ref()) {
            crate::browser::edit_command(v, command);
            return true;
        }
        let _ = command;
        false
    }

    /// Takes in what the pages reported: a click in a page gives it the keyboard (and makes
    /// its pane the focused one), a new window opens in the same pane, a new address is saved.
    pub fn browser_events(&mut self, ctx: &egui::Context) {
        let mut moved = false;
        let mut focused = None;
        // Loads that began in pages an agent uses: (page, address, the address before).
        let mut began = Vec::new();
        let mut new_tabs = Vec::new();
        let mut dialogs = Vec::new();
        for b in self.browsers.values_mut() {
            let before = b.url.clone();
            for e in b.take_events() {
                match e {
                    // Believed after a real click on the page only: a script in it can post
                    // that message too, to take the keyboard from a terminal.
                    Event::Focused => {
                        #[cfg(target_os = "macos")]
                        let clicked = b.view.as_ref().is_some_and(crate::browser::clicked_in);
                        #[cfg(not(target_os = "macos"))]
                        let clicked = true;
                        if clicked {
                            focused = Some(b.id);
                        }
                    }
                    // A link to a new window: a new page of the user's in the panel; an agent's
                    // page keeps it (the agent follows it there).
                    Event::OpenHere(url) if b.driven() => b.navigate(&url),
                    Event::OpenHere(url) => new_tabs.push((b.project, url)),
                    Event::Url(_) => moved = true,
                    Event::Dialog(d) => {
                        b.dialog = Some(d);
                        dialogs.push(b.id);
                    }
                    e @ (Event::DownloadStarted(..) | Event::DownloadDone(..)) => b.note_download(&e),
                    Event::Loading(true) if b.driven() => began.push((b.id, b.url.clone(), before.clone())),
                    _ => {}
                }
            }
        }
        for (project, url) in new_tabs {
            self.browser_tab_at(project, &url);
        }
        // A dialog came up: the keyboard leaves its page (blocked till it is answered) for the
        // dialog's bar, when the page had it.
        for id in dialogs {
            #[cfg(target_os = "macos")]
            if let Some(v) = self.browsers.get(&id).and_then(|b| b.view.as_ref())
                && crate::browser::has_keyboard(v)
            {
                let _ = v.focus_parent();
                self.dialog_focus = Some(id);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = id;
        }
        // Dialogs of pages that went away are answered (WebKit wants each answered).
        #[cfg(target_os = "macos")]
        {
            let live: Vec<objc2::rc::Retained<objc2_web_kit::WKWebView>> = self.browsers.values().filter_map(|b| b.view.as_ref()).map(crate::browser::web_view_of).collect();
            crate::browser_dialog::tidy(&live.iter().map(|w| &**w).collect::<Vec<_>>());
        }
        #[cfg(target_os = "macos")]
        for (id, url, before) in began {
            self.stop_unasked_load(id, &url, &before);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = began;
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
    /// not drawn or something covers it (a dialog, the palette, a menu, Review full screen);
    /// lets go of pages out of view and unused a while (asleep). Hands the keyboard between
    /// the pages and the app.
    #[cfg(target_os = "macos")]
    pub fn sync_browsers(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.browsers.is_empty() {
            self.browser_pages.clear();
            self.run_browser_jobs(ctx);
            return;
        }
        // Pages agents use stay up out of view (outside the window, where they keep drawing).
        let in_use = self.browsers_in_use();
        let now = std::time::Instant::now();
        let covered = self.dialog.is_some() || self.palette.is_some() || self.review_full() || self.attention_open || egui::Popup::is_any_open(ctx);
        let zoom = ctx.zoom_factor();
        let sleep = crate::browser::sleep_after(self.settings.browser_sleep_minutes);
        let pages = std::mem::take(&mut self.browser_pages);
        let mut wake: Option<Duration> = None;
        for b in self.browsers.values_mut() {
            b.pick_up_picture(ctx);
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
                        b.placed = Some(*rect);
                        b.shown = b.view.is_some();
                        b.dimmed = false;
                        b.zoom = 1.0;
                    } else if b.view.is_some() {
                        if b.placed != Some(*rect)
                            && let Some(v) = &b.view
                        {
                            let _ = v.set_bounds(crate::browser::bounds(*rect, zoom));
                            b.placed = Some(*rect);
                        }
                        if !b.shown
                            && let Some(v) = &b.view
                        {
                            let _ = v.set_visible(true);
                            b.shown = true;
                        }
                    }
                    // Zoomed out in a Grid tile (the layout of Focus, smaller), as is in Focus.
                    if (b.zoom - b.zoom_want).abs() > 0.004
                        && let Some(v) = &b.view
                    {
                        let _ = v.zoom(f64::from(b.zoom_want));
                        b.zoom = b.zoom_want;
                    }
                }
                _ if in_use.contains(&b.id) => {
                    let size = b.placed.map_or(vec2(1200.0, 800.0), |r| r.size());
                    let off = crate::browser::bounds(Rect::from_min_size(pos2(-30000.0, 0.0), size), zoom);
                    if b.view.is_none() && !b.url.is_empty() {
                        b.view = crate::browser::build_view(frame, &b.url, off, b.sink(ctx));
                        b.asleep = b.view.is_none();
                        b.dimmed = false;
                    } else if !b.offscreen
                        && let Some(v) = &b.view
                    {
                        let _ = v.set_bounds(off);
                        let _ = v.set_visible(true);
                    }
                    b.offscreen = b.view.is_some();
                    b.placed = None;
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
                        let idle = now.duration_since(b.last_seen);
                        if idle >= sleep {
                            b.sleep();
                        } else {
                            let left = sleep - idle;
                            wake = Some(wake.map_or(left, |w| w.min(left)));
                        }
                    }
                }
            }
        }
        // A page under a question is dimmed; a page loading says so (from its request on). A
        // page an agent acts in takes the keyboard when the agent focuses something in it: the
        // keyboard goes back where the user had it (the agent's terminal).
        let mut typing_in = self.browser_focus;
        for b in self.browsers.values_mut() {
            // A page out of sight that has the keyboard gives it back (else keys go nowhere).
            if !b.shown
                && let Some(v) = &b.view
                && crate::browser::has_keyboard(v)
            {
                let _ = v.focus_parent();
                if typing_in == Some(b.id) {
                    typing_in = None;
                }
            }
            if let Some(v) = &b.view {
                b.loading = crate::browser::is_loading(v);
                if b.driven() && typing_in != Some(b.id) && crate::browser::has_keyboard(v) {
                    let _ = v.focus_parent();
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
        if let Some(d) = wake {
            ctx.request_repaint_after(d);
        }
        self.run_browser_jobs(ctx);
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
