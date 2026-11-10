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
        self.review_ui(&ctx);
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
            let body = self.top_bar(ui, full, |_, ui| {
                ui.label(egui::RichText::new("TernaryCode").font(style::semibold(14.0)));
            });
            // The Focus view's area (the same top bar above it): the first project's shell
            // starts at its final size.
            self.last_body = Some(body);
            self.last_full_body = Some(body);
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| self.welcome(ui));
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

    fn welcome(&mut self, ui: &mut Ui) {
        use crate::i18n::t;
        use super::{icons, kit};
        let p = style::pal(ui);
        ui.add_space((ui.available_height() * 0.2).min(160.0));
        ui.vertical_centered(|ui| {
            kit::icon(ui, icons::TERMINAL_WINDOW, 40.0, p.accent_text);
            ui.add_space(10.0);
            ui.label(egui::RichText::new(t("No projects yet")).font(style::semibold(20.0)).color(p.text));
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(t("Add a project folder to open a terminal in it and start agents."))
                    .color(p.text_muted),
            );
            ui.add_space(16.0);
            if kit::primary_icon(ui, icons::FOLDER_PLUS, t("Add Project…")).clicked() {
                self.actions.push(crate::actions::Action::AddProject);
            }
            ui.add_space(28.0);
            shortcuts_card(ui);
        });
    }
}

/// The shortcuts worth learning first.
pub fn shortcuts_card(ui: &mut Ui) {
    use super::kit;
    use crate::i18n::t;
    let p = style::pal(ui);
    use crate::actions::Action;
    let rows = [
        (Action::NextAttention, t("Jump to the agent that waits for you")),
        (Action::Palette, t("Command palette")),
        (Action::PickPreset, t("Start an agent")),
        (Action::QuickOpen, t("Open a file of the project")),
        (Action::ToggleEditor, t("Show or hide the editor")),
        (Action::SplitRight, t("Split the pane")),
        (Action::Review, t("Review what the agent changed")),
    ];
    kit::card(ui).show(ui, |ui| {
        ui.set_width(340.0);
        egui::Grid::new("shortcuts").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            for (action, what) in rows {
                kit::kbd(ui, kit::keys(&action).unwrap_or_default());
                ui.label(egui::RichText::new(what).color(p.text_muted));
                ui.end_row();
            }
        });
    });
}
