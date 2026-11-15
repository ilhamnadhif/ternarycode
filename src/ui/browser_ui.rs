//! The browser pane: its toolbar (back, forward, reload, the address, the terminal that serves
//! the page, open in the default browser) over its page; opening pages in it; and keeping each
//! page's web view where its pane is drawn, hidden when the pane is out of view or something is
//! drawn over it.

use std::time::Duration;

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::PaneId;
use tc_term::layout::{Layout, SplitDir};

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::browser::{Answer, Ask, Browser, Driver, Event};
use crate::i18n::{t, tf};
use crate::project::Tab;

/// The toolbar at the top of a browser pane (its header, also in a split).
pub const BAR_H: f32 = 36.0;
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

        // Right side first: what is left goes to the address.
        let inner = bar.shrink2(vec2(6.0, 0.0));
        let mut right = inner.right();
        let more = Rect::from_center_size(pos2(right - 11.0, inner.center().y), vec2(22.0, 22.0));
        let ext = Rect::from_center_size(pos2(more.left() - 13.0, inner.center().y), vec2(22.0, 22.0));
        right = ext.left() - 4.0;
        // The console's warnings and errors, when there are any (a click opens its drawer).
        let (errors, warnings) = self.browsers.get(&id).map_or((0, 0), |b| {
            (b.console.iter().filter(|l| l.is_error()).count(), b.console.iter().filter(|l| l.is_warning()).count())
        });
        let console_open = self.browsers.get(&id).is_some_and(|b| b.console_open);
        let cons_rect = (errors + warnings > 0 || console_open).then(|| {
            let n = if errors + warnings > 0 { (errors + warnings).to_string() } else { String::new() };
            let g = ui.painter().layout_no_wrap(n, egui::FontId::proportional(style::SMALL), p.text);
            let w = if g.size().x > 0.0 { g.size().x + 30.0 } else { 26.0 };
            let r = Rect::from_min_max(pos2(right - w, inner.center().y - 11.0), pos2(right, inner.center().y + 11.0));
            right = r.left() - 4.0;
            r
        });
        let from_rect = from.as_ref().map(|(_, label)| {
            let g = ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(style::SMALL), p.text_muted);
            let w = (g.size().x + 26.0).min(160.0);
            let r = Rect::from_min_max(pos2(right - w, inner.center().y - 11.0), pos2(right, inner.center().y + 11.0));
            right = r.left() - 4.0;
            r
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
        if self.browser_edit == Some(id) {
            self.browser_edit = None;
            ui.memory_mut(|m| m.request_focus(fid));
        }
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
        let open_ext = ui
            .scope_builder(egui::UiBuilder::new().max_rect(ext), |ui| kit::icon_button_ex(ui, icons::ARROW_SQUARE_OUT, t("Open in Default Browser"), false, 22.0))
            .inner
            .clicked();
        let more = ui.scope_builder(egui::UiBuilder::new().max_rect(more), |ui| kit::icon_button_ex(ui, icons::DOTS_THREE, t("More"), false, 22.0)).inner;
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("browser-more:{id}"), more.rect);
        egui::Popup::menu(&more).align(egui::RectAlign::BOTTOM_END).width(230.0).show(|ui| self.browser_menu(ui, id));
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
        self.browser_pages.insert(id, page);
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
        let shared = self.browsers.get(&id).and_then(|b| self.project(b.project)).and_then(|p| p.tab_of(id).map(|i| p.tabs[i].layout.panes().len() > 1)).unwrap_or(false);
        if ui.add_enabled_ui(!url.is_empty(), |ui| kit::menu_item(ui, Some(icons::COPY), t("Copy Address"), None, false)).inner.clicked() {
            ui.ctx().copy_text(url.clone());
            ui.close();
        }
        if ui.add_enabled_ui(!url.is_empty(), |ui| kit::menu_item(ui, Some(icons::ARROW_SQUARE_OUT), t("Open in Default Browser"), None, false)).inner.clicked() {
            crate::platform::open_url(&url);
            ui.close();
        }
        if shared && kit::menu_item(ui, Some(icons::ARROWS_OUT_SIMPLE), t("Move to New Tab"), None, false).clicked() {
            self.actions.push(Action::PaneToTab(id));
            ui.close();
        }
        kit::menu_sep(ui);
        if kit::menu_item(ui, Some(icons::X), t("Close Pane"), kit::keys(&Action::ClosePane), true).clicked() {
            self.actions.push(Action::ClosePaneId(id));
            ui.close();
        }
    }

    /// A new tab with a browser pane, at the project's local server when it has one (else its
    /// address field takes the keyboard).
    pub fn new_browser_tab(&mut self) {
        let Some(project) = self.command_project() else { return };
        let url = self.project_servers(project).first().map(|(_, s)| s.url.clone()).unwrap_or_default();
        self.browser_tab_at(project, &url);
    }

    /// A new tab of `project` with a browser pane at `url` ("": its address field takes the
    /// keyboard), in view.
    pub fn browser_tab_at(&mut self, project: tc_core::ProjectId, url: &str) {
        let url = url.to_string();
        let id = self.new_id();
        self.browsers.insert(id, Browser::new(id, project, &url));
        let Some(p) = self.project_mut(project) else { return };
        p.tabs.push(Tab { layout: Layout::Pane(id), focused: id, title: None });
        p.active_tab = p.tabs.len() - 1;
        if url.is_empty() {
            self.browser_edit = Some(id);
        }
        self.show_focus();
        self.mark_dirty();
    }

    /// Opens `url` in the project's browser pane: the one in the tab in view, else another of
    /// the project's; with none, a new one beside the pane in view (its tab holding one pane,
    /// and room), else in a tab of its own. The keyboard stays where it is.
    pub fn open_web_here(&mut self, url: &str) {
        let Some(project) = self.command_project().or(self.active) else {
            crate::platform::open_url(url);
            return;
        };
        self.open_web_in(project, url, true);
    }

    /// Opens `url` in `project`'s browser pane, as [`App::open_web_here`]; `show`: the project
    /// comes into view (else its page waits there, the sidebar showing it has one).
    pub fn open_web_in(&mut self, project: tc_core::ProjectId, url: &str, show: bool) {
        let in_view: Vec<PaneId> = self.project(project).and_then(|p| p.tab()).map(|t| t.layout.panes()).unwrap_or_default();
        let all: Vec<PaneId> = self.project(project).map(|p| p.panes()).unwrap_or_default();
        let existing = in_view.iter().chain(all.iter()).find(|id| self.browsers.contains_key(id)).copied();
        if let Some(id) = existing {
            if let Some(b) = self.browsers.get_mut(&id) {
                b.navigate(url);
            }
            if let Some(p) = self.project_mut(project)
                && let Some(tab) = p.tab_of(id)
            {
                p.active_tab = tab;
            }
        } else {
            let id = self.new_id();
            let mut b = Browser::new(id, project, url);
            b.address = url.to_string();
            self.browsers.insert(id, b);
            let wide = self.last_body.is_some_and(|r| r.width() >= 900.0);
            let Some(p) = self.project_mut(project) else { return };
            match p.tab_mut() {
                Some(tab) if tab.layout.panes().len() == 1 && wide => {
                    let current = tab.focused;
                    tab.layout.split(current, id, SplitDir::Horizontal);
                }
                _ => {
                    p.tabs.push(Tab { layout: Layout::Pane(id), focused: id, title: None });
                    p.active_tab = p.tabs.len() - 1;
                }
            }
        }
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
        for b in self.browsers.values_mut() {
            let before = b.url.clone();
            for e in b.take_events() {
                match e {
                    Event::Focused => focused = Some(b.id),
                    // A link to a new window: a new browser tab of the project; an agent's page
                    // keeps it (the agent follows it there).
                    Event::OpenHere(url) if b.driven() => b.navigate(&url),
                    Event::OpenHere(url) => new_tabs.push((b.project, url)),
                    Event::Url(_) => moved = true,
                    Event::Loading(true) if b.driven() => began.push((b.id, b.url.clone(), before.clone())),
                    _ => {}
                }
            }
        }
        for (project, url) in new_tabs {
            self.browser_tab_at(project, &url);
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
            if let Some(project) = self.browsers.get(&id).map(|b| b.project)
                && let Some(p) = self.project_mut(project)
                && let Some(tab) = p.tab_of(id)
            {
                p.tabs[tab].focused = id;
            }
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
            let dim = b.ask.is_some();
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
