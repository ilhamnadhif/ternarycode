//! The window: status bar, sidebar, the central view, and what floats over them.

use eframe::egui::{self, Ui};

use super::style;
use crate::app::App;

pub const STATUS_H: f32 = 26.0;

/// Empty space that moves the window (macOS, where the title bar is hidden and the content runs
/// under it); a double click zooms, like a title bar. Add it before the widgets on top of it.
pub fn window_drag_area(ui: &Ui, rect: egui::Rect, id: &str) {
    if cfg!(target_os = "macos") {
        title_drag_area(ui, rect, id);
    }
}

/// Empty space of a title bar the app draws: dragging it moves the window (snapping on
/// Windows), a double click zooms (maximises), and on Windows a right click shows the window's
/// system menu. Add it before the widgets on top of it.
pub fn title_drag_area(ui: &Ui, rect: egui::Rect, id: &str) {
    let r = ui.interact(rect, egui::Id::new(("window-drag", id)), egui::Sense::click_and_drag());
    if r.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    let zoomed = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    if r.double_clicked() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!zoomed));
    }
    if r.secondary_clicked() && crate::platform::custom_caption() {
        crate::platform::show_system_menu(zoomed);
    }
}

impl App {
    pub fn draw(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        self.editor_seen = false;
        self.monitor_pill_rect = None;
        let spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        // Windows: the title bar the app draws (the system's is off), across the window.
        if crate::platform::custom_caption() {
            egui::Panel::top("title-bar")
                .show_separator_line(false)
                .frame(egui::Frame::NONE.fill(p.chrome))
                .exact_size(super::caption::TITLE_H)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = spacing;
                    self.title_bar(ui)
                });
        }
        // Narrow: project tiles only, its width fixed.
        if !self.zen_mode && self.settings.sidebar_compact && !self.settings.sidebar_hidden {
            egui::Panel::left("sidebar-compact")
                .show_separator_line(false)
                .resizable(false)
                .exact_size(super::sidebar::COMPACT_W)
                .frame(egui::Frame::NONE.fill(p.chrome))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = spacing;
                    ui.expand_to_include_rect(ui.max_rect());
                    self.sidebar_compact(ui)
                });
        }
        if !self.zen_mode && !self.settings.sidebar_compact {
            let mut visible = !self.settings.sidebar_hidden;
            let response = egui::Panel::left("sidebar")
                .show_separator_line(false)
                .resizable(true)
                .default_size(self.settings.sidebar_width)
                .size_range(200.0..=360.0)
                .frame(egui::Frame::NONE.fill(p.chrome))
                .show_collapsible(ui, &mut visible, |ui| {
                    ui.spacing_mut().item_spacing = spacing;
                    // The panel keeps the size its content uses: claim all of it, or the sidebar
                    // shrinks a little every frame until it is at its narrowest.
                    ui.expand_to_include_rect(ui.max_rect());
                    self.sidebar(ui)
                });
            // Dragged shut past its narrowest (or open again from the edge): remember it, or the
            // panel would spring back next frame.
            if visible == self.settings.sidebar_hidden {
                self.settings.sidebar_hidden = !visible;
                self.settings_changed();
            }
            if let Some(r) = response
                && visible
            {
                #[cfg(debug_assertions)]
                crate::devtest::record("sidebar", r.response.rect);
                let w = r.response.rect.width();
                if (w - self.settings.sidebar_width).abs() > 1.0 {
                    self.settings.sidebar_width = w;
                    self.settings_changed();
                }
            }
        }
        // The status bar, under the main column (the sidebar runs the window's height).
        if !self.zen_mode {
            egui::Panel::bottom("status-bar")
                .show_separator_line(false)
                .frame(egui::Frame::NONE.fill(p.chrome).inner_margin(egui::Margin { left: 4, right: 12, top: 0, bottom: 0 }))
                .exact_size(STATUS_H)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = spacing;
                    self.status_bar(ui)
                });
        }
        // The window's base: the sheets (panes, panels, tiles) are drawn on it.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(p.chrome))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = spacing;
                self.central(ui)
            });

        self.editor_keyboard = self.editor_seen;
        let ctx = ui.ctx().clone();
        self.monitor_popover_ui(&ctx);
        self.attention_panel(&ctx);
        self.palette_ui(&ctx);
        self.dialogs(&ctx);
        self.updates(&ctx);
        self.toast_ui(&ctx);
    }

    /// Terminals take keys only while nothing floating over them does: a dialog, the palette,
    /// the Review, a management view, Zen Mode, a browser page, a menu popup, or the attention list.
    pub fn terminals_take_keys(&self) -> bool {
        self.dialog.is_none()
            && self.palette.is_none()
            && !self.review_full()
            && !self.attention_open
            && !self.monitor_popover
            && !self.updater.dialog_open
            && self.browser_focus.is_none()
            && self.pane_search.is_none()
            && !crate::ade::is_management_view(self.view)
            && !egui::Popup::is_any_open(&self.ctx)
    }

    fn central(&mut self, ui: &mut Ui) {
        // Accounts and the phone are Settings pages now: the views they had fall back to the
        // work (a state saved by an older version).
        if crate::ade::is_management_view(self.view) {
            self.view = if crate::ade::is_management_view(self.prev_view) { tc_core::model::View::Focus } else { self.prev_view };
        }
        if self.projects.is_empty() {
            let full = ui.max_rect();
            let body = self.top_bar(
                ui,
                full,
                |_, ui| {
                    ui.label(egui::RichText::new("TernaryCode").font(style::semibold(14.0)));
                },
                None,
            );
            // The Focus view's area (the same top bar above it): the first project's shell
            // starts at its final size.
            let body = super::focus::sheets_area(body, self.settings.sidebar_hidden);
            self.last_body = Some(body);
            self.last_full_body = Some(body);
            super::focus::paint_sheet(ui.painter(), body, &style::pal(ui), super::focus::SheetLook::Plain);
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                ui.set_clip_rect(body.shrink(1.0));
                egui::ScrollArea::vertical()
                    .id_salt("welcome-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let min_h = body.height().max(560.0);
                        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(500.0), min_h), egui::Sense::hover());
                        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| self.welcome(ui));
                    });
            });
            return;
        }
        match self.view {
            tc_core::model::View::Focus => self.focus_view(ui),
            tc_core::model::View::Grid => {
                let body = self.overview_bar(ui);
                ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| self.grid_view(ui));
            }
            tc_core::model::View::Profiles | tc_core::model::View::Monitor | tc_core::model::View::Pairing => {}
        }
    }

    /// No projects yet: add the first one (drop a folder, choose one, or tick the repositories
    /// found in the usual folders), open one from any terminal, see which agents are installed,
    /// and the shortcuts worth learning first.
    fn welcome(&mut self, ui: &mut Ui) {
        use super::{icons, kit};
        use crate::i18n::{t, tf};
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        // Look for repositories and agents once, while this shows.
        self.onboarding.poll();
        if self.onboarding.found.is_none() && !self.onboarding.scanning() {
            #[cfg(windows)]
            let path = crate::cli::pane_path();
            #[cfg(not(windows))]
            let path = self.lsp.path_env.clone();
            // The PATH a pane's shell gets is read once at launch: wait for it (four seconds at
            // most), or every agent would look missing.
            let asked = *self.onboarding.asked.get_or_insert_with(std::time::Instant::now);
            if path.is_some() || asked.elapsed() > std::time::Duration::from_secs(4) {
                let agents = self.presets().into_iter().filter_map(|x| Some((x.agent?, tc_core::agent::command_program(&x.command)?.to_string()))).filter(|(a, _)| *a != tc_core::agent::AgentKind::Gemini).collect();
                let ctx = ui.ctx().clone();
                self.onboarding.scan(&ctx, agents, path);
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
        let area = ui.max_rect();
        let w = (area.width() - 80.0).min(1000.0);
        let col = egui::Rect::from_min_max(egui::pos2(area.center().x - w / 2.0, area.top() + (area.height() * 0.06).clamp(24.0, 52.0)), egui::pos2(area.center().x + w / 2.0, area.bottom()));
        let painter = ui.painter().clone();
        let mut y = col.top();
        // Who we are, then what to do.
        let logo = egui::Rect::from_min_size(egui::pos2(col.left(), y), egui::vec2(18.0, 18.0));
        super::settings::paint_logo(&painter, logo, &p);
        painter.text(egui::pos2(logo.right() + 8.0, logo.center().y), egui::Align2::LEFT_CENTER, format!("TernaryCode {}", env!("CARGO_PKG_VERSION")), style::mono(11.0), p.text_faint);
        y = logo.bottom() + 12.0;
        let mut job = egui::text::LayoutJob::default();
        let mut f = egui::TextFormat::simple(style::semibold(26.0), p.text);
        f.extra_letter_spacing = -0.65;
        job.append(t("Add your first project"), 0.0, f);
        let g = painter.layout_job(job);
        painter.galley(egui::pos2(col.left(), y), g.clone(), p.text);
        y += g.size().y + 6.0;
        let mut job = egui::text::LayoutJob::simple(
            t("A project is a folder. Its terminals start there, its agents tell this window when they need you, and the Grid shows all of them at once.").to_string(),
            egui::FontId::proportional(13.5),
            p.text_muted,
            620.0_f32.min(w),
        );
        job.wrap.max_width = 620.0_f32.min(w);
        let g = painter.layout_job(job);
        painter.galley(egui::pos2(col.left(), y), g.clone(), p.text_muted);
        y += g.size().y + 24.0;

        // The drop zone, with Choose Folder…
        let narrow_drop = w < 580.0;
        let drop_h = if narrow_drop { 116.0 } else { 84.0 };
        let drop = egui::Rect::from_min_size(egui::pos2(col.left(), y), egui::vec2(w, drop_h));
        let r = drop.shrink(0.75);
        let dash = egui::Stroke::new(1.5, p.border_heavy);
        for (a, b) in [(r.left_top() + egui::vec2(12.0, 0.0), r.right_top() - egui::vec2(12.0, 0.0)), (r.right_top() + egui::vec2(0.0, 12.0), r.right_bottom() - egui::vec2(0.0, 12.0)), (r.right_bottom() - egui::vec2(12.0, 0.0), r.left_bottom() + egui::vec2(12.0, 0.0)), (r.left_bottom() - egui::vec2(0.0, 12.0), r.left_top() + egui::vec2(0.0, 12.0))] {
            painter.extend(egui::Shape::dashed_line(&[a, b], dash, 5.0, 4.0));
        }
        for (c, a0) in [(r.left_top() + egui::vec2(12.0, 12.0), 180.0_f32), (r.right_top() + egui::vec2(-12.0, 12.0), 270.0), (r.right_bottom() + egui::vec2(-12.0, -12.0), 0.0), (r.left_bottom() + egui::vec2(12.0, -12.0), 90.0)] {
            let arc: Vec<egui::Pos2> = (0..=6).map(|i| {
                let a = (a0 + i as f32 * 15.0).to_radians();
                c + egui::vec2(a.cos(), a.sin()) * 12.0
            }).collect();
            painter.add(egui::Shape::line(arc, dash));
        }
        let bw = kit::button_width(ui, Some(icons::FOLDER_OPEN), t("Choose Folder…"), kit::keys(&crate::actions::Action::AddProject), false);
        let top_cy = if narrow_drop { drop.top() + 38.0 } else { drop.center().y };
        let ic = egui::Rect::from_min_size(egui::pos2(drop.left() + 22.0, top_cy - 22.0), egui::vec2(44.0, 44.0));
        painter.rect(ic, egui::CornerRadius::same(11), p.accent_tint, egui::Stroke::new(1.0, style::soft(p.accent, 0.28)), egui::StrokeKind::Inside);
        painter.text(ic.center(), egui::Align2::CENTER_CENTER, icons::FOLDER_SIMPLE_PLUS, style::icon_font(22.0), p.accent_text);
        let text_max = if narrow_drop { (drop.right() - 20.0 - (ic.right() + 18.0)).max(60.0) } else { (drop.right() - 28.0 - bw - (ic.right() + 18.0)).max(60.0) };
        let mut tj = egui::text::LayoutJob::simple_singleline(t("Drop a folder anywhere on this window").to_string(), style::medium(14.0), p.text);
        tj.wrap = egui::text::TextWrapping::truncate_at_width(text_max);
        let tg = painter.layout_job(tj);
        painter.galley(egui::pos2(ic.right() + 18.0, top_cy - 9.0 - tg.size().y / 2.0), tg, p.text);
        let mut sj = egui::text::LayoutJob::simple_singleline(t("A file works too: its folder is added.").to_string(), egui::FontId::proportional(12.5), p.text_faint);
        sj.wrap = egui::text::TextWrapping::truncate_at_width(text_max);
        let sg = painter.layout_job(sj);
        painter.galley(egui::pos2(ic.right() + 18.0, top_cy + 11.0 - sg.size().y / 2.0), sg, p.text_faint);
        let br = if narrow_drop {
            egui::Rect::from_min_size(egui::pos2(drop.left() + 22.0, drop.bottom() - 40.0), egui::vec2(bw.min(drop.width() - 44.0), 28.0))
        } else {
            egui::Rect::from_min_size(egui::pos2(drop.right() - 20.0 - bw, drop.center().y - 14.0), egui::vec2(bw, 28.0))
        };
        if ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| kit::button_keys(ui, kit::Kind::Primary, Some(icons::FOLDER_OPEN), t("Choose Folder…"), kit::keys(&crate::actions::Action::AddProject), false)).inner.clicked() {
            self.actions.push(crate::actions::Action::AddProject);
        }
        y = drop.bottom() + 26.0;

        // Two columns when wide (`>= 680`), stacked vertically when narrow.
        let stacked = w < 680.0;
        let right_w = if stacked { w } else { 340.0_f32.min(w * 0.4) };
        let left = egui::Rect::from_min_max(
            egui::pos2(col.left(), y),
            egui::pos2(if stacked { col.right() } else { col.right() - right_w - 28.0 }, area.bottom() - 70.0),
        );
        let section = |painter: &egui::Painter, at: egui::Pos2, max_right: f32, title: &str, sub: Option<&str>| {
            let g = painter.layout_no_wrap(title.to_string(), style::semibold(13.5), p.text);
            let gw = g.size().x;
            painter.galley(at, g, p.text);
            if let Some(sub) = sub {
                let sub_room = (max_right - (at.x + gw + 14.0)).max(0.0);
                if sub_room >= 40.0 {
                    let mut job = egui::text::LayoutJob::simple_singleline(sub.to_string(), egui::FontId::proportional(12.0), p.text_faint);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(sub_room);
                    let sg = painter.layout_job(job);
                    painter.galley(egui::pos2(at.x + gw + 10.0, at.y + 9.0 - sg.size().y / 2.0), sg, p.text_faint);
                }
            }
        };
        let roots = self.onboarding.found.as_ref().map(|f| f.roots.iter().take(2).map(|r| crate::paths::contract_tilde(r)).collect::<Vec<_>>().join(&format!(" {} ", t("and")))).unwrap_or_default();
        let sub = if roots.is_empty() { None } else { Some(tf("git repositories in {}", &[&roots])) };
        let again_w = kit::button_width(ui, Some(icons::ARROW_CLOCKWISE), t("Look Again"), None, true);
        let again = egui::Rect::from_min_size(egui::pos2(left.right() - again_w, left.top() - 4.0), egui::vec2(again_w, 24.0));
        section(&painter, left.min, again.left() - 6.0, if cfg!(target_os = "macos") { t("Found on this Mac") } else { t("Found on this computer") }, sub.as_deref());
        if ui.scope_builder(egui::UiBuilder::new().max_rect(again), |ui| kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROW_CLOCKWISE), t("Look Again"), true)).inner.clicked() {
            self.onboarding.found = None;
        }
        let card_top = left.top() + 28.0;
        let repos = self.onboarding.found.as_ref().map(|f| f.repos.clone());
        let max_list_h = if stacked { 200.0 } else { (left.bottom() - card_top - 48.0).clamp(100.0, 320.0) };
        let list_h = repos.as_ref().map_or(50.0, |r| if r.is_empty() { 50.0 } else { (r.len() as f32 * 50.0).min(max_list_h) });
        let card = egui::Rect::from_min_size(
            egui::pos2(left.left(), card_top),
            egui::vec2(left.width(), list_h + if repos.as_ref().is_some_and(|r| !r.is_empty()) { 46.0 } else { 0.0 }),
        );
        painter.rect(card, egui::CornerRadius::same(10), p.surface, egui::Stroke::new(1.0, if dark { p.border } else { p.border_strong }), egui::StrokeKind::Inside);
        match &repos {
            None => {
                kit::paint_lattice(&painter, egui::pos2(card.left() + 14.0 + 7.0, card.top() + 25.0), 10.0, p.info, true);
                painter.text(egui::pos2(card.left() + 14.0 + 24.0, card.top() + 25.0), egui::Align2::LEFT_CENTER, t("Looking for git repositories…"), egui::FontId::proportional(12.5), p.text_muted);
            }
            Some(list) if list.is_empty() => {
                painter.text(egui::pos2(card.left() + 14.0, card.top() + 25.0), egui::Align2::LEFT_CENTER, t("No git repositories in the usual folders."), egui::FontId::proportional(12.5), p.text_muted);
            }
            Some(list) => {
                let scroll_rect = egui::Rect::from_min_size(card.min, egui::vec2(card.width(), list_h));
                ui.scope_builder(egui::UiBuilder::new().max_rect(scroll_rect), |ui| {
                    ui.set_clip_rect(scroll_rect.shrink(1.0));
                    egui::ScrollArea::vertical().id_salt("welcome-repos").auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, repo) in list.iter().enumerate() {
                            let (row, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 50.0), egui::Sense::click());
                            if !ui.is_rect_visible(row) {
                                continue;
                            }
                            let rp = ui.painter_at(row);
                            if i > 0 {
                                rp.hline(row.x_range(), row.top() + 0.5, egui::Stroke::new(1.0, p.border));
                            }
                            let on = self.onboarding.picked.contains(&repo.path);
                            if resp.clicked() {
                                if on {
                                    self.onboarding.picked.remove(&repo.path);
                                } else {
                                    self.onboarding.picked.insert(repo.path.clone());
                                }
                            }
                            if resp.hovered() {
                                rp.rect_filled(row.shrink(1.0), egui::CornerRadius::same(9), p.hover);
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            let cy = row.center().y;
                            let bx = egui::Rect::from_center_size(egui::pos2(row.left() + 14.0 + 8.0, cy), egui::vec2(16.0, 16.0));
                            kit::paint_checkbox(&rp, bx, on, resp.hovered(), &p);
                            let letters = crate::project::initial(&repo.name);
                            let (cr, cg, cb) = crate::project::COLORS[i % crate::project::COLORS.len()];
                            let tile = egui::Rect::from_min_size(egui::pos2(bx.right() + 12.0, cy - 12.0), egui::vec2(24.0, 24.0));
                            kit::paint_project_tile(&rp, tile, &letters, egui::Color32::from_rgb(cr, cg, cb).gamma_multiply(if on { 1.0 } else { 0.55 }), dark);
                            let x = tile.right() + 12.0;
                            let age = repo.touched.map(|tt| tf("{} ago", &[&super::dialogs::ago(tt)])).unwrap_or_default();
                            let ag = rp.layout_no_wrap(age, style::mono(10.5), p.text_ghost);
                            let bgal = rp.layout_no_wrap(repo.branch.clone(), style::mono(10.5), p.text_faint);
                            let rw = ag.size().x.max(bgal.size().x + 15.0);
                            rp.galley(egui::pos2(row.right() - 14.0 - bgal.size().x, cy - 9.0 - bgal.size().y / 2.0), bgal.clone(), p.text_faint);
                            rp.text(egui::pos2(row.right() - 14.0 - bgal.size().x - 4.0 - 5.5, cy - 9.0), egui::Align2::CENTER_CENTER, icons::GIT_BRANCH, style::icon_font(11.0), p.text_faint);
                            rp.galley(egui::pos2(row.right() - 14.0 - ag.size().x, cy + 8.0 - ag.size().y / 2.0), ag, p.text_ghost);
                            let room = (row.right() - 14.0 - rw - 12.0 - x).max(20.0);
                            let mut job = egui::text::LayoutJob::default();
                            job.append(&repo.name, 0.0, egui::TextFormat::simple(style::medium(13.0), if on { p.text } else { p.text_muted }));
                            if !repo.kind.label().is_empty() {
                                job.append(repo.kind.label(), 8.0, egui::TextFormat::simple(egui::FontId::proportional(12.0), p.text_faint));
                            }
                            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                            let g = rp.layout_job(job);
                            rp.galley(egui::pos2(x, cy - 8.0 - g.size().y / 2.0), g, p.text);
                            let mut job = egui::text::LayoutJob::simple_singleline(crate::paths::contract_tilde(&repo.path), style::mono(10.5), p.text_faint);
                            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                            let g = rp.layout_job(job);
                            rp.galley(egui::pos2(x, cy + 9.0 - g.size().y / 2.0), g, p.text_faint);
                        }
                    });
                });
                // The card's foot: how many are ticked, Select All, Add N Projects.
                let foot = egui::Rect::from_min_max(egui::pos2(card.left(), card.bottom() - 46.0), card.max);
                painter.hline(foot.x_range(), foot.top() + 0.5, egui::Stroke::new(1.0, p.border));
                let n = self.onboarding.picked.len();
                let add_label = crate::i18n::count(n, t("Add 1 Project"), t("Add {} Projects"));
                let aw = kit::button_width(ui, Some(icons::PLUS), &add_label, None, true);
                let ar = egui::Rect::from_min_size(egui::pos2(foot.right() - 10.0 - aw, foot.center().y - 12.0), egui::vec2(aw, 24.0));
                let add = ui.add_enabled_ui(n > 0, |ui| ui.scope_builder(egui::UiBuilder::new().max_rect(ar), |ui| kit::button_ex(ui, kit::Kind::Secondary, Some(icons::PLUS), &add_label, true)).inner).inner;
                if add.clicked() {
                    for repo in list.iter().filter(|r| self.onboarding.picked.contains(&r.path)) {
                        self.actions.push(crate::actions::Action::AddProjectPath(repo.path.clone()));
                    }
                }
                let sw = kit::button_width(ui, None, t("Select All"), None, true);
                let sr = egui::Rect::from_min_size(egui::pos2(ar.left() - 6.0 - sw, foot.center().y - 12.0), egui::vec2(sw, 24.0));
                if ui.scope_builder(egui::UiBuilder::new().max_rect(sr), |ui| kit::button_ex(ui, kit::Kind::Ghost, None, t("Select All"), true)).inner.clicked() {
                    self.onboarding.picked = list.iter().map(|r| r.path.clone()).collect();
                }
                let sel_room = (sr.left() - 8.0 - (foot.left() + 14.0)).max(24.0);
                let mut sel_job = egui::text::LayoutJob::simple_singleline(tf("{} of {} selected", &[&n.to_string(), &list.len().to_string()]), egui::FontId::proportional(12.0), p.text_faint);
                sel_job.wrap = egui::text::TextWrapping::truncate_at_width(sel_room);
                let sel_g = painter.layout_job(sel_job);
                painter.galley(egui::pos2(foot.left() + 14.0, foot.center().y - sel_g.size().y / 2.0), sel_g, p.text_faint);
            }
        }

        // Right column (or stacked below left column on narrow viewports).
        let right_top = if stacked { card.bottom() + 24.0 } else { y };
        let right = egui::Rect::from_min_max(
            egui::pos2(if stacked { col.left() } else { col.right() - right_w }, right_top),
            egui::pos2(col.right(), area.bottom() - 70.0),
        );
        // From any terminal.
        section(&painter, right.min, right.right(), t("From any terminal"), None);
        let code = egui::Rect::from_min_size(egui::pos2(right.left(), right.top() + 28.0), egui::vec2(right.width(), 38.0));
        if command_box(ui, code, "ternarycode .") {
            ui.ctx().copy_text("ternarycode .".into());
            self.toast_ok(t("Copied.").into());
        }
        let mut job = egui::text::LayoutJob::simple(t("Opens the folder you are in as a project, in this window.").to_string(), egui::FontId::proportional(12.0), p.text_faint, right.width());
        job.wrap.max_width = right.width();
        let g = painter.layout_job(job);
        painter.galley(egui::pos2(right.left(), code.bottom() + 8.0), g.clone(), p.text_faint);
        // The agents on this Mac.
        let ay = code.bottom() + 8.0 + g.size().y + 26.0;
        let os = if cfg!(target_os = "macos") { t("Agents on this Mac") } else { t("Agents on this computer") };
        section(&painter, egui::pos2(right.left(), ay), right.right(), os, None);
        let agents = self.onboarding.found.as_ref().map(|f| f.agents.clone()).unwrap_or_default();
        let mut cy = ay + 28.0;
        let card_top = cy;
        let mut rows: Vec<(egui::Rect, crate::onboarding::FoundAgent)> = Vec::new();
        let missing = agents.iter().filter(|a| !a.found).count() as f32;
        let commands = !stacked && card_top + agents.len() as f32 * 46.0 + missing * 42.0 <= area.bottom() - 70.0;
        for a in &agents {
            let h = if a.found || !commands { 46.0 } else { 46.0 + 42.0 };
            rows.push((egui::Rect::from_min_size(egui::pos2(right.left(), cy), egui::vec2(right.width(), h)), a.clone()));
            cy += h;
        }
        if !rows.is_empty() {
            let acard = egui::Rect::from_min_max(egui::pos2(right.left(), card_top), egui::pos2(right.right(), cy));
            painter.rect(acard, egui::CornerRadius::same(10), p.surface, egui::Stroke::new(1.0, if dark { p.border } else { p.border_strong }), egui::StrokeKind::Inside);
        }
        for (i, (row, a)) in rows.iter().enumerate() {
            if i > 0 {
                painter.hline(row.x_range(), row.top() + 0.5, egui::Stroke::new(1.0, p.border));
            }
            let ly = row.top() + 23.0;
            let tile = egui::Rect::from_min_size(egui::pos2(row.left() + 12.0, ly - 14.0), egui::vec2(28.0, 28.0));
            painter.rect(tile, egui::CornerRadius::same(8), p.surface, egui::Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
            super::agent_logo::paint(&painter, egui::Rect::from_center_size(tile.center(), egui::vec2(15.0, 15.0)), a.agent, p.text, false);
            let status_w = if a.found {
                let g = painter.layout_no_wrap(t("Found").to_string(), style::medium(11.5), p.ok);
                let fw = g.size().x;
                painter.galley(egui::pos2(row.right() - 12.0 - fw, ly - g.size().y / 2.0), g, p.ok);
                painter.text(egui::pos2(row.right() - 12.0 - fw - 4.0 - 6.0, ly), egui::Align2::CENTER_CENTER, icons::CHECK_CIRCLE, style::icon_fill(12.0), p.ok);
                fw + 22.0
            } else {
                let g = painter.layout_no_wrap(t("Not installed").to_string(), style::medium(11.5), p.warn);
                let nw = g.size().x;
                painter.galley(egui::pos2(row.right() - 12.0 - nw, ly - g.size().y / 2.0), g, p.warn);
                if let Some(cmd) = tc_core::agent::install_command(a.agent).filter(|_| commands) {
                    let code = egui::Rect::from_min_size(egui::pos2(row.left() + 12.0, row.top() + 46.0), egui::vec2(row.width() - 24.0, 32.0));
                    if command_box(ui, code, cmd) {
                        ui.ctx().copy_text(cmd.to_string());
                        self.toast_ok(t("Install command copied.").into());
                    }
                }
                nw + 8.0
            };
            let name_room = (row.right() - 12.0 - status_w - 8.0 - (tile.right() + 11.0)).max(24.0);
            let mut nj = egui::text::LayoutJob::default();
            nj.append(a.agent.label(), 0.0, egui::TextFormat::simple(style::medium(13.0), if a.found { p.text } else { p.text_muted }));
            if let Some(v) = &a.version {
                nj.append(v, 8.0, egui::TextFormat::simple(style::mono(10.5), p.text_faint));
            }
            nj.wrap = egui::text::TextWrapping::truncate_at_width(name_room);
            let ng = painter.layout_job(nj);
            painter.galley(egui::pos2(tile.right() + 11.0, ly - ng.size().y / 2.0), ng, p.text);
        }

        // The keys worth learning first: always positioned below both columns and wrapped cleanly.
        let keys_top = (area.bottom() - 60.0).max(card.bottom().max(cy) + 22.0);
        let keys = egui::Rect::from_min_max(egui::pos2(col.left(), keys_top), egui::pos2(col.right(), keys_top + 64.0));
        painter.hline(keys.x_range(), keys.top() + 0.5, egui::Stroke::new(1.0, p.border));
        let mut x = keys.left();
        let mut kcy = keys.top() + 24.0;
        use crate::actions::Action;
        for (action, what) in [(Action::PickPreset, t("Run an agent")), (Action::NextAttention, t("Go to the agent that waits")), (Action::ToggleGrid, t("Every project at once")), (Action::Palette, t("Every command"))] {
            if let Some(k) = kit::keys(&action) {
                let kw = kit::kbd_width(&painter, k);
                let g = painter.layout_no_wrap(what.to_string(), egui::FontId::proportional(12.0), p.text_faint);
                let item_w = kw + 8.0 + g.size().x;
                if x > keys.left() && x + item_w > keys.right() {
                    x = keys.left();
                    kcy += 24.0;
                }
                x += kit::paint_kbd_text(&painter, egui::pos2(x, kcy), k, &p) + 8.0;
                let gw = g.size().x;
                painter.galley(egui::pos2(x, kcy - g.size().y / 2.0), g, p.text_faint);
                x += gw + 22.0;
            }
        }
    }
}

/// A command in a box (`$ ternarycode .`) with a copy button; returns whether copy was
/// clicked.
fn command_box(ui: &mut Ui, rect: egui::Rect, cmd: &str) -> bool {
    use super::{icons, kit};
    let p = style::pal(ui);
    let dark = ui.visuals().dark_mode;
    let painter = ui.painter();
    painter.rect(rect, egui::CornerRadius::same(9), if dark { p.inset } else { style::over(p.canvas, p.chrome, 0.35) }, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    painter.text(egui::pos2(rect.left() + 12.0, rect.center().y), egui::Align2::LEFT_CENTER, "$", style::mono(12.0), p.text_faint);
    let mut job = egui::text::LayoutJob::simple_singleline(cmd.to_string(), style::mono(if rect.height() < 36.0 { 11.0 } else { 12.0 }), p.text);
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 12.0 - 10.0 - 10.0 - 30.0);
    let g = painter.layout_job(job);
    painter.galley(egui::pos2(rect.left() + 12.0 + 10.0 + 6.0, rect.center().y - g.size().y / 2.0), g, p.text);
    let b = egui::Rect::from_center_size(egui::pos2(rect.right() - 6.0 - 11.0, rect.center().y), egui::vec2(22.0, 22.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(b), |ui| kit::icon_button_ex(ui, icons::COPY, crate::i18n::t("Copy"), false, 22.0)).inner.clicked()
}

/// The shortcuts worth learning first, in the order of the work: what waits, then agents, the
/// Grid, everything else, the code, the review.
pub fn shortcuts_card(ui: &mut Ui) {
    use super::kit;
    use crate::actions::Action;
    use crate::i18n::t;
    let p = style::pal(ui);
    let rows = [
        (Action::NextAttention, t("Jump to the agent that waits for you")),
        (Action::PickPreset, t("Run an agent")),
        (Action::ToggleGrid, t("See every project at once")),
        (Action::Palette, t("Command palette")),
        (Action::ToggleEditor, t("Show code beside the terminal")),
        (Action::Review, t("Review what the agent changed")),
    ];
    let card_w = ui.available_width().clamp(280.0, 620.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(card_w, 12.0 + rows.len() as f32 * 34.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect(rect, egui::CornerRadius::same(9), p.surface, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    for (i, (action, what)) in rows.iter().enumerate() {
        let y = rect.top() + 6.0 + i as f32 * 34.0;
        if i > 0 {
            painter.hline((rect.left() + 16.0)..=(rect.right() - 16.0), y + 0.5, egui::Stroke::new(1.0, p.border));
        }
        let cy = y + 17.0;
        kit::paint_kbd_text(painter, egui::pos2(rect.left() + 16.0, cy), kit::keys(action).unwrap_or_default(), &p);
        painter.text(egui::pos2(rect.left() + 16.0 + 52.0 + 14.0, cy), egui::Align2::LEFT_CENTER, *what, egui::FontId::proportional(13.0), p.text_muted);
    }
}
