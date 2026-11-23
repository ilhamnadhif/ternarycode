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
                        let min_h = body.height().max(640.0);
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
        use crate::actions::Action;
        use crate::i18n::{t, tf};
        use egui::{Align2, CornerRadius, Rect, Stroke, StrokeKind, pos2, vec2};
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
        // The sheet's padding (36 · 48 · 24 · 48), the content at most 1088 wide.
        let area = ui.max_rect();
        let w = (area.width() - 96.0).clamp(300.0, 1088.0);
        let col = Rect::from_min_max(pos2(area.center().x - w / 2.0, area.top() + 36.0), pos2(area.center().x + w / 2.0, area.bottom() - 24.0));
        let painter = ui.painter().clone();
        let mut y = col.top();
        // Who we are, then what to do.
        let logo = Rect::from_min_size(pos2(col.left(), y), vec2(22.0, 22.0));
        super::settings::paint_logo(&painter, logo, &p);
        painter.text(pos2(logo.right() + 8.0, logo.center().y), Align2::LEFT_CENTER, format!("TernaryCode {}", env!("CARGO_PKG_VERSION")), style::mono(style::DATA), p.text_faint);
        y = logo.bottom() + 18.0;
        let mut job = egui::text::LayoutJob::default();
        let mut f = egui::TextFormat::simple(style::semibold(style::DISPLAY), p.text);
        f.extra_letter_spacing = -0.4;
        f.line_height = Some(30.0);
        job.append(t("Add your first project"), 0.0, f);
        let g = painter.layout_job(job);
        painter.galley(pos2(col.left(), y), g.clone(), p.text);
        y += g.size().y + 18.0;
        let mut job = egui::text::LayoutJob::simple(
            t("A project is a folder. Its terminals start there, its agents tell this window when they need you, and the Overview shows all of them at once.").to_string(),
            egui::FontId::proportional(style::BODY),
            p.text_muted,
            640.0_f32.min(w),
        );
        job.sections[0].format.line_height = Some(18.0);
        let g = painter.layout_job(job);
        painter.galley(pos2(col.left(), y), g.clone(), p.text_muted);
        y += g.size().y + 18.0;

        // The drop zone: a dashed line, the folder tile, what to do, Choose folder….
        let keys = kit::keys(&Action::AddProject);
        let bw = kit::button_width(ui, Some(icons::FOLDER_OPEN), t("Choose folder…"), keys, false);
        let narrow_drop = w < 560.0;
        let drop = Rect::from_min_size(pos2(col.left(), y), vec2(w, if narrow_drop { 84.0 + 28.0 + 12.0 } else { 84.0 }));
        dashed_round_rect(&painter, drop.shrink(0.5), 12.0, Stroke::new(1.0, p.border_heavy));
        let tile = Rect::from_min_size(pos2(drop.left() + 18.0, drop.top() + 18.0), vec2(48.0, 48.0));
        painter.rect_filled(tile, CornerRadius::same(12), p.accent_tint);
        painter.text(tile.center(), Align2::CENTER_CENTER, icons::FOLDER_SIMPLE_PLUS, style::icon_font(22.0), p.accent_text);
        let tx = tile.right() + 16.0;
        let text_max = if narrow_drop { drop.right() - 18.0 - tx } else { drop.right() - 18.0 - bw - 16.0 - tx }.max(60.0);
        let mut tj = egui::text::LayoutJob::simple_singleline(t("Drop a folder anywhere on this window").to_string(), style::semibold(style::HEADING), p.text);
        tj.wrap = egui::text::TextWrapping::truncate_at_width(text_max);
        let tg = painter.layout_job(tj);
        let mut sj = egui::text::LayoutJob::simple_singleline(t("A file works too: its folder is added.").to_string(), egui::FontId::proportional(style::SMALL), p.text_faint);
        sj.wrap = egui::text::TextWrapping::truncate_at_width(text_max);
        let sg = painter.layout_job(sj);
        let text_h = 20.0 + 2.0 + 16.0;
        let ty = tile.center().y - text_h / 2.0;
        painter.galley(pos2(tx, ty + 10.0 - tg.size().y / 2.0), tg, p.text);
        painter.galley(pos2(tx, ty + 22.0 + 8.0 - sg.size().y / 2.0), sg, p.text_faint);
        let br = if narrow_drop {
            Rect::from_min_size(pos2(tx, tile.bottom() + 12.0), vec2(bw.min(drop.right() - 18.0 - tx), style::CONTROL_H))
        } else {
            Rect::from_min_size(pos2(drop.right() - 18.0 - bw, tile.center().y - style::CONTROL_H / 2.0), vec2(bw, style::CONTROL_H))
        };
        let r = ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| kit::button_keys(ui, kit::Kind::Primary, Some(icons::FOLDER_OPEN), t("Choose folder…"), keys, false)).inner;
        #[cfg(debug_assertions)]
        crate::devtest::record("welcome-choose", r.rect);
        if r.clicked() {
            self.actions.push(Action::AddProject);
        }
        y = drop.bottom() + 18.0;

        // The keys worth learning first, at the foot under a line (18 · 1 · 18 · keys 18).
        let hints: Vec<(&str, &str)> = [(Action::PickPreset, t("run an agent")), (Action::NextAttention, t("go to the agent that waits")), (Action::ToggleGrid, t("every project at once")), (Action::Palette, t("every command"))]
            .into_iter()
            .filter_map(|(a, what)| Some((kit::keys(&a)?, what)))
            .collect();
        let hint_rows = if kit::hints_width(&painter, &hints) <= w { 1.0 } else { 2.0 };
        let foot_h = 18.0 + 1.0 + 18.0 + hint_rows * 18.0 + (hint_rows - 1.0) * 8.0;

        // Two columns when wide (the right one 380), stacked when narrow.
        let stacked = w < 760.0;
        let right_w = if stacked { w } else { 380.0 };
        let left = Rect::from_min_max(pos2(col.left(), y), pos2(if stacked { col.right() } else { col.right() - right_w - 32.0 }, col.bottom() - foot_h));
        let heading = |at: egui::Pos2, title: &str| {
            painter.text(at, Align2::LEFT_TOP, title, style::semibold(style::HEADING), p.text);
        };
        // Found on this Mac: a heading, where it looked, Look again.
        heading(left.min, if cfg!(target_os = "macos") { t("Found on this Mac") } else { t("Found on this computer") });
        let roots = self.onboarding.found.as_ref().map(|f| f.roots.iter().take(2).map(|r| crate::paths::contract_tilde(r)).collect::<Vec<_>>().join(&format!(" {} ", t("and")))).unwrap_or_default();
        if !roots.is_empty() {
            painter.text(pos2(left.left(), left.top() + 22.0), Align2::LEFT_TOP, tf("git repositories in {}", &[&roots]), egui::FontId::proportional(style::SMALL), p.text_faint);
        }
        let again_w = kit::button_width(ui, Some(icons::ARROWS_CLOCKWISE), t("Look again"), None, true);
        let again = Rect::from_min_size(pos2(left.right() - again_w, left.top() + 19.0 - 12.0), vec2(again_w, 24.0));
        let r = ui.scope_builder(egui::UiBuilder::new().max_rect(again), |ui| kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROWS_CLOCKWISE), t("Look again"), true)).inner;
        #[cfg(debug_assertions)]
        crate::devtest::record("welcome-look-again", r.rect);
        if r.clicked() {
            self.onboarding.found = None;
        }
        // The repositories: a card of rows 52 high (hairlines between), the foot 48.
        let card_top = left.top() + 38.0 + 12.0;
        let repos = self.onboarding.found.as_ref().map(|f| f.repos.clone());
        let listed = repos.as_ref().is_some_and(|r| !r.is_empty());
        let foot = if listed { 48.0 + 1.0 } else { 0.0 };
        let max_list = if stacked { 5.0 * 53.0 } else { (left.bottom() - 18.0 - card_top - foot).max(2.0 * 53.0) };
        let list_h = match &repos {
            Some(r) if !r.is_empty() => (r.len() as f32 * 53.0 - 1.0).min(max_list),
            _ => 52.0,
        };
        let card = Rect::from_min_size(pos2(left.left(), card_top), vec2(left.width(), list_h + foot));
        painter.rect(card, CornerRadius::same(style::RADIUS_CARD), p.surface, Stroke::new(1.0, p.border), StrokeKind::Inside);
        match &repos {
            None => {
                kit::paint_lattice(&painter, pos2(card.left() + 12.0 + 7.0, card.top() + 26.0), 10.0, p.info, true);
                painter.text(pos2(card.left() + 12.0 + 22.0, card.top() + 26.0), Align2::LEFT_CENTER, t("Looking for git repositories…"), egui::FontId::proportional(style::SMALL), p.text_muted);
            }
            Some(list) if list.is_empty() => {
                painter.text(pos2(card.left() + 12.0, card.top() + 26.0), Align2::LEFT_CENTER, t("No git repositories in the usual folders."), egui::FontId::proportional(style::SMALL), p.text_muted);
            }
            Some(list) => {
                let scroll_rect = Rect::from_min_size(card.min, vec2(card.width(), list_h));
                ui.scope_builder(egui::UiBuilder::new().max_rect(scroll_rect), |ui| {
                    ui.set_clip_rect(scroll_rect.shrink(1.0).intersect(ui.clip_rect()));
                    egui::ScrollArea::vertical().id_salt("welcome-repos").auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, repo) in list.iter().enumerate() {
                            if i > 0 {
                                let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
                                ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, p.border));
                            }
                            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), egui::Sense::click());
                            if !ui.is_rect_visible(row) {
                                continue;
                            }
                            #[cfg(debug_assertions)]
                            crate::devtest::record(format!("welcome-repo:{i}"), row);
                            let rp = ui.painter_at(row);
                            let on = self.onboarding.picked.contains(&repo.path);
                            if resp.clicked() {
                                if on {
                                    self.onboarding.picked.remove(&repo.path);
                                } else {
                                    self.onboarding.picked.insert(repo.path.clone());
                                }
                            }
                            if resp.hovered() {
                                rp.rect_filled(row, CornerRadius::ZERO, p.hover);
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            // Checkbox 14, the project's avatar (M), name and kind over the path; at
                            // the right its branch and how long ago (60, right-aligned).
                            let cy = row.center().y;
                            let bx = Rect::from_min_size(pos2(row.left() + 12.0, cy - 7.0), vec2(14.0, 14.0));
                            kit::paint_checkbox(&rp, bx, on, resp.hovered(), &p);
                            let (cr, cg, cb) = crate::project::COLORS[i % crate::project::COLORS.len()];
                            let avatar = Rect::from_min_size(pos2(bx.right() + 12.0, cy - 10.0), vec2(20.0, 20.0));
                            kit::paint_project_tile(&rp, avatar, &crate::project::initial(&repo.name), egui::Color32::from_rgb(cr, cg, cb), dark);
                            let x = avatar.right() + 12.0;
                            let age = repo.touched.map(|tt| tf("{} ago", &[&super::dialogs::ago(tt)])).unwrap_or_default();
                            let ag = rp.layout_no_wrap(age, egui::FontId::proportional(style::SMALL), p.text_faint);
                            let age_left = row.right() - 12.0 - ag.size().x.max(60.0);
                            rp.galley(pos2(row.right() - 12.0 - ag.size().x, cy - ag.size().y / 2.0), ag, p.text_faint);
                            let bg = rp.layout_no_wrap(repo.branch.clone(), style::mono(style::DATA), p.text_muted);
                            let bgx = age_left - 12.0 - bg.size().x;
                            rp.galley(pos2(bgx, cy - bg.size().y / 2.0), bg, p.text_muted);
                            rp.text(pos2(bgx - 4.0 - 6.0, cy), Align2::CENTER_CENTER, icons::GIT_BRANCH, style::icon_font(12.0), p.text_faint);
                            let room = (bgx - 4.0 - 12.0 - 12.0 - x).max(20.0);
                            let mut job = egui::text::LayoutJob::default();
                            job.append(&repo.name, 0.0, egui::TextFormat::simple(style::medium(style::BODY), p.text));
                            if !repo.kind.label().is_empty() {
                                job.append(repo.kind.label(), 8.0, egui::TextFormat::simple(egui::FontId::proportional(style::SMALL), p.text_faint));
                            }
                            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                            let g = rp.layout_job(job);
                            rp.galley(pos2(x, cy - 17.0 + 9.0 - g.size().y / 2.0), g, p.text);
                            let mut job = egui::text::LayoutJob::simple_singleline(crate::paths::contract_tilde(&repo.path), style::mono(style::DATA), p.text_faint);
                            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                            let g = rp.layout_job(job);
                            rp.galley(pos2(x, cy - 17.0 + 20.0 + 7.0 - g.size().y / 2.0), g, p.text_faint);
                        }
                    });
                });
                // The card's foot: how many are ticked, Select all, Add N projects.
                let foot = Rect::from_min_max(pos2(card.left(), card.bottom() - 48.0), card.max);
                painter.hline(foot.x_range(), foot.top() - 0.5, Stroke::new(1.0, p.border));
                let n = self.onboarding.picked.len();
                let add_label = crate::i18n::count(n, t("Add 1 project"), t("Add {} projects"));
                let aw = kit::button_width(ui, Some(icons::PLUS), &add_label, None, true);
                let ar = Rect::from_min_size(pos2(foot.right() - 12.0 - aw, foot.center().y - 12.0), vec2(aw, 24.0));
                let add = ui.add_enabled_ui(n > 0, |ui| ui.scope_builder(egui::UiBuilder::new().max_rect(ar), |ui| kit::button_ex(ui, kit::Kind::Primary, Some(icons::PLUS), &add_label, true)).inner).inner;
                #[cfg(debug_assertions)]
                crate::devtest::record("welcome-add", add.rect);
                if add.clicked() {
                    for repo in list.iter().filter(|r| self.onboarding.picked.contains(&r.path)) {
                        self.actions.push(Action::AddProjectPath(repo.path.clone()));
                    }
                }
                let sw = kit::button_width(ui, None, t("Select all"), None, true);
                let sr = Rect::from_min_size(pos2(ar.left() - 8.0 - sw, foot.center().y - 12.0), vec2(sw, 24.0));
                let r = ui.scope_builder(egui::UiBuilder::new().max_rect(sr), |ui| kit::button_ex(ui, kit::Kind::Ghost, None, t("Select all"), true)).inner;
                #[cfg(debug_assertions)]
                crate::devtest::record("welcome-select-all", r.rect);
                if r.clicked() {
                    self.onboarding.picked = list.iter().map(|r| r.path.clone()).collect();
                }
                let mut sel_job = egui::text::LayoutJob::simple_singleline(tf("{} of {} selected", &[&n.to_string(), &list.len().to_string()]), egui::FontId::proportional(style::SMALL), p.text_faint);
                sel_job.wrap = egui::text::TextWrapping::truncate_at_width((sr.left() - 8.0 - (foot.left() + 12.0)).max(24.0));
                let sel_g = painter.layout_job(sel_job);
                painter.galley(pos2(foot.left() + 12.0, foot.center().y - sel_g.size().y / 2.0), sel_g, p.text_faint);
            }
        }

        // The right column (below the repositories when narrow).
        let right_top = if stacked { card.bottom() + 28.0 } else { y };
        let right = Rect::from_min_max(pos2(if stacked { col.left() } else { col.right() - right_w }, right_top), pos2(col.right(), col.bottom() - foot_h));
        // From any terminal: the command in a box, what it does.
        heading(right.min, t("From any terminal"));
        let code = Rect::from_min_size(pos2(right.left(), right.top() + 20.0 + 10.0), vec2(right.width(), 40.0));
        if command_box(ui, code, "$ ternarycode .", true) {
            ui.ctx().copy_text("ternarycode .".into());
            self.toast_ok(t("Copied.").into());
        }
        let mut job = egui::text::LayoutJob::simple(t("Opens the folder you are in as a project, in this window.").to_string(), egui::FontId::proportional(style::SMALL), p.text_faint, right.width());
        job.wrap.max_width = right.width();
        let g = painter.layout_job(job);
        painter.galley(pos2(right.left(), code.bottom() + 10.0), g.clone(), p.text_faint);
        // The agents on this Mac: a card of rows 52 high, then how to install what is missing.
        let ay = code.bottom() + 10.0 + g.size().y + 28.0;
        heading(pos2(right.left(), ay), if cfg!(target_os = "macos") { t("Agents on this Mac") } else { t("Agents on this computer") });
        let agents = self.onboarding.found.as_ref().map(|f| f.agents.clone()).unwrap_or_default();
        let installs: Vec<&'static str> = agents.iter().filter(|a| !a.found).filter_map(|a| tc_core::agent::install_command(a.agent)).collect();
        let card_top = ay + 20.0 + 12.0;
        let rows_h = agents.len() as f32 * 53.0 + installs.len() as f32 * 41.0 - 1.0;
        if !agents.is_empty() {
            let acard = Rect::from_min_size(pos2(right.left(), card_top), vec2(right.width(), rows_h));
            painter.rect(acard, CornerRadius::same(style::RADIUS_CARD), p.surface, Stroke::new(1.0, p.border), StrokeKind::Inside);
        }
        let mut cy = card_top;
        for (i, a) in agents.iter().enumerate() {
            if i > 0 {
                painter.hline(right.x_range(), cy - 0.5, Stroke::new(1.0, p.border));
            }
            let row = Rect::from_min_size(pos2(right.left(), cy), vec2(right.width(), 52.0));
            let ly = row.center().y;
            let tile = Rect::from_min_size(pos2(row.left() + 12.0, ly - 14.0), vec2(28.0, 28.0));
            kit::paint_agent_tile(&painter, tile, Some(a.agent), &p);
            let (pill_icon, pill_text, pill_c) = if a.found { (icons::CHECK_CIRCLE, t("Found"), p.ok) } else { (icons::CHAT_CIRCLE_DOTS, t("Not installed"), p.warn) };
            let pw = kit::pill_width(&painter, Some(pill_icon), pill_text);
            kit::paint_pill(&painter, pos2(row.right() - 12.0 - pw, ly), Some(pill_icon), pill_text, pill_c, style::tint(pill_c, dark));
            let room = (row.right() - 12.0 - pw - 12.0 - (tile.right() + 12.0)).max(24.0);
            let mut nj = egui::text::LayoutJob::simple_singleline(a.agent.label().to_string(), style::medium(style::BODY), p.text);
            nj.wrap = egui::text::TextWrapping::truncate_at_width(room);
            let ng = painter.layout_job(nj);
            painter.galley(pos2(tile.right() + 12.0, ly - 17.0 + 9.0 - ng.size().y / 2.0), ng, p.text);
            let sub = if a.found { a.version.clone().unwrap_or_default() } else { a.program.clone() };
            let mut vj = egui::text::LayoutJob::simple_singleline(sub, style::mono(style::DATA), p.text_faint);
            vj.wrap = egui::text::TextWrapping::truncate_at_width(room);
            let vg = painter.layout_job(vj);
            painter.galley(pos2(tile.right() + 12.0, ly - 17.0 + 20.0 + 7.0 - vg.size().y / 2.0), vg, p.text_faint);
            cy += 53.0;
        }
        for cmd in installs {
            painter.hline(right.x_range(), cy - 0.5, Stroke::new(1.0, p.border));
            let row = Rect::from_min_size(pos2(right.left(), cy), vec2(right.width(), 40.0));
            if command_box(ui, row, cmd, false) {
                ui.ctx().copy_text(cmd.to_string());
                self.toast_ok(t("Install command copied.").into());
            }
            cy += 41.0;
        }

        // The keys, under a line.
        let line_y = col.bottom() - foot_h + 18.0;
        painter.hline(col.x_range(), line_y + 0.5, Stroke::new(1.0, p.border));
        let mut x = col.left();
        let mut ky = line_y + 1.0 + 18.0 + 9.0;
        for h in &hints {
            let hw = kit::hints_width(&painter, std::slice::from_ref(h));
            if x > col.left() && x + hw > col.right() {
                x = col.left();
                ky += 26.0;
            }
            x += kit::paint_hints(&painter, pos2(x, ky), std::slice::from_ref(h), &p) + 14.0;
        }
    }
}

/// A dashed rounded rectangle (dashes 5, gaps 4): the drop zone's edge.
fn dashed_round_rect(painter: &egui::Painter, r: egui::Rect, radius: f32, stroke: egui::Stroke) {
    use egui::{pos2, vec2};
    let mut pts: Vec<egui::Pos2> = Vec::new();
    // Around the corners (8 steps each), clockwise from the top-left one's end.
    let corners = [(pos2(r.right() - radius, r.top() + radius), 270.0_f32), (pos2(r.right() - radius, r.bottom() - radius), 0.0), (pos2(r.left() + radius, r.bottom() - radius), 90.0), (pos2(r.left() + radius, r.top() + radius), 180.0)];
    pts.push(pos2(r.left() + radius, r.top()));
    for (c, a0) in corners {
        for i in 0..=8 {
            let a = (a0 + i as f32 * 90.0 / 8.0).to_radians();
            pts.push(c + vec2(a.cos(), a.sin()) * radius);
        }
    }
    pts.push(pos2(r.left() + radius, r.top()));
    painter.extend(egui::Shape::dashed_line(&pts, stroke, 5.0, 4.0));
}

/// A command in a box with a copy button (Icon button S): `framed`, its own inset box 40 high
/// (`$ ternarycode .`); else a row of a card, the `$` quiet before it. Returns whether copy was
/// clicked.
fn command_box(ui: &mut Ui, rect: egui::Rect, cmd: &str, framed: bool) -> bool {
    use super::{icons, kit};
    let p = style::pal(ui);
    let painter = ui.painter();
    let mut x = rect.left() + 12.0;
    if framed {
        painter.rect(rect, egui::CornerRadius::same(8), p.inset, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    } else {
        let g = painter.layout_no_wrap("$".into(), style::mono(style::MONO), p.text_faint);
        let gw = g.size().x;
        painter.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, p.text_faint);
        x += gw + 8.0;
    }
    let font = if framed { style::mono(style::MONO) } else { style::mono(style::DATA) };
    let mut job = egui::text::LayoutJob::simple_singleline(cmd.to_string(), font, p.text);
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - 6.0 - 24.0 - 8.0 - x).max(20.0));
    let g = painter.layout_job(job);
    painter.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, p.text);
    let b = egui::Rect::from_center_size(egui::pos2(rect.right() - 6.0 - 12.0, rect.center().y), egui::vec2(24.0, 24.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(b), |ui| kit::icon_button_ex(ui, icons::COPY, crate::i18n::t("Copy"), false, 24.0)).inner.clicked()
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
