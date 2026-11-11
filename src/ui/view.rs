//! The window: status bar, sidebar, the central view, and what floats over them.

use eframe::egui::{self, Ui};

use super::style;
use crate::app::App;

pub const STATUS_H: f32 = 26.0;

/// Empty space that moves the window (macOS, where the title bar is hidden and the content runs
/// under it); a double click zooms, like a title bar. Add it before the widgets on top of it.
pub fn window_drag_area(ui: &Ui, rect: egui::Rect, id: &str) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let r = ui.interact(rect, egui::Id::new(("window-drag", id)), egui::Sense::click_and_drag());
    if r.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if r.double_clicked() {
        let zoomed = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!zoomed));
    }
}

impl App {
    pub fn draw(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        self.editor_seen = false;
        let spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        egui::Panel::bottom("status-bar")
            .frame(
                egui::Frame::NONE
                    .fill(p.chrome)
                    .inner_margin(egui::Margin::symmetric(10, 0)),
            )
            .exact_size(STATUS_H)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = spacing;
                self.status_bar(ui)
            });
        let mut visible = !self.settings.sidebar_hidden;
        let response = egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(self.settings.sidebar_width)
            .size_range(180.0..=420.0)
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
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(p.canvas))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = spacing;
                self.central(ui)
            });

        self.editor_keyboard = self.editor_seen;
        let ctx = ui.ctx().clone();
        self.attention_panel(&ctx);
        self.palette_ui(&ctx);
        self.dialogs(&ctx);
        self.updates(&ctx);
        self.toast_ui(&ctx);
    }

    /// Terminals take keys only while nothing floating over them does: a dialog, the palette,
    /// the Review or the attention list. The panes are drawn first, so they would read an Esc
    /// meant for those (and send it to the agent) before it closes them.
    pub fn terminals_take_keys(&self) -> bool {
        self.dialog.is_none() && self.palette.is_none() && self.review.is_none() && !self.attention_open
    }

    fn central(&mut self, ui: &mut Ui) {
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
            self.last_body = Some(body);
            self.last_full_body = Some(body);
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| self.welcome(ui));
            return;
        }
        // Review takes the place of the panes (the sidebar stays).
        if self.review.is_some() {
            self.review_view(ui);
            return;
        }
        match self.view {
            tc_core::model::View::Focus => self.focus_view(ui),
            tc_core::model::View::Grid => {
                let body = self.overview_bar(ui);
                ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| self.grid_view(ui));
            }
        }
    }

    /// No projects yet: what TernaryCode is, the three ways to add a project, and the shortcuts
    /// worth learning first.
    fn welcome(&mut self, ui: &mut Ui) {
        use super::{icons, kit};
        use crate::i18n::{t, tf};
        let p = style::pal(ui);
        let area = ui.max_rect();
        let col = egui::Rect::from_min_size(egui::pos2(area.center().x - 310.0, area.top() + (area.height() * 0.11).clamp(24.0, 96.0)), egui::vec2(620.0, area.height()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(col).layout(egui::Layout::top_down(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let (r, _) = ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::hover());
            super::settings::paint_logo(ui.painter(), r, &p);
            ui.add_space(18.0);
            ui.label(egui::RichText::new(t("Every project and its agents, in one window")).font(style::semibold(22.0)).color(p.text));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(t("Add a project folder to open a terminal in it and start agents.")).size(style::BODY).color(p.text_muted));
            ui.add_space(20.0);
            if kit::button_keys(ui, kit::Kind::Primary, Some(icons::FOLDER_PLUS), t("Add Project…"), kit::keys(&crate::actions::Action::AddProject), false).clicked() {
                self.actions.push(crate::actions::Action::AddProject);
            }
            ui.add_space(30.0);
            // Three ways in.
            let (row, _) = ui.allocate_exact_size(egui::vec2(620.0, 138.0), egui::Sense::hover());
            let w = (620.0 - 24.0) / 3.0;
            let ways = [
                (icons::FOLDER_PLUS, t("Choose a folder"), t("Its terminals start there; files and git changes show beside them.").to_string()),
                (icons::ARROW_LINE_DOWN, t("Drop a folder here"), t("Anywhere on this window. A file adds the folder it is in.").to_string()),
                (icons::TERMINAL, t("From any terminal"), tf("{} opens the current folder here.", &["ternarycode ."])),
            ];
            for (i, (icon, title, body)) in ways.iter().enumerate() {
                let r = egui::Rect::from_min_size(row.min + egui::vec2(i as f32 * (w + 12.0), 0.0), egui::vec2(w, row.height()));
                let painter = ui.painter();
                painter.rect(r, egui::CornerRadius::same(9), p.surface, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                painter.text(r.min + egui::vec2(14.0, 14.0), egui::Align2::LEFT_TOP, *icon, style::icon_font(20.0), p.accent_text);
                painter.text(r.min + egui::vec2(14.0, 14.0 + 20.0 + 10.0), egui::Align2::LEFT_TOP, *title, style::medium(13.5), p.text);
                // The command in the third, in monospace.
                let mut job = egui::text::LayoutJob::default();
                let small = egui::FontId::proportional(style::SMALL);
                match (i, body.split_once("ternarycode .")) {
                    (2, Some((before, after))) => {
                        job.append(before, 0.0, egui::text::TextFormat::simple(small.clone(), p.text_muted));
                        job.append("ternarycode .", 0.0, egui::text::TextFormat::simple(egui::FontId::monospace(11.5), p.text));
                        job.append(after, 0.0, egui::text::TextFormat::simple(small, p.text_muted));
                    }
                    _ => job.append(body, 0.0, egui::text::TextFormat::simple(small, p.text_muted)),
                }
                job.wrap.max_width = w - 28.0;
                let g = painter.layout_job(job);
                painter.galley(r.min + egui::vec2(14.0, 14.0 + 20.0 + 10.0 + 20.0), g, p.text_muted);
            }
            ui.add_space(22.0);
            shortcuts_card(ui);
        });
    }
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
    let (rect, _) = ui.allocate_exact_size(egui::vec2(620.0, 12.0 + rows.len() as f32 * 34.0), egui::Sense::hover());
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
