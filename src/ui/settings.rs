//! Settings: a modal with its pages at the left (General, Appearance, Terminal, Agents, Editor,
//! Notifications, Updates, About), a search that narrows them, and each page as groups of rows
//! (what a setting is and a line about it at the left, its control at the right). Saved as they
//! change.

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use tc_core::agent::{AgentKind, Preset};
use tc_core::model::{CursorShape, Integration, Language, ThemeMode};

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPage {
    General,
    Appearance,
    Terminal,
    /// Presets and how each agent reports to TernaryCode (its hooks).
    Agents,
    Editor,
    Notifications,
    Updates,
    About,
    /// Opens Agents (kept for "Agent Integrations…").
    Integrations,
}

const W: f32 = 900.0;
const H: f32 = 640.0;
const NAV_W: f32 = 200.0;
const HEAD_H: f32 = 52.0;

/// The pages, their icons, and the rows on each (for the search).
fn pages() -> [(SettingsPage, &'static str, &'static str, &'static [&'static str]); 8] {
    [
        (SettingsPage::General, icons::GEAR_SIX, "General", &["Command line", "Data folder", "Version", "Check for updates automatically"]),
        (SettingsPage::Appearance, icons::PAINT_BRUSH, "Appearance", &["Theme", "Language", "With the dark theme", "With the light theme", "Font", "Size", "Cursor", "Blinking cursor"]),
        (SettingsPage::Terminal, icons::TERMINAL, "Terminal", &["Scrollback per pane", "Scrollback memory for all panes", "Option key as Meta", "Copy on select", "Programs may copy to the clipboard", "Shell integration"]),
        (SettingsPage::Agents, icons::PLUGS_CONNECTED, "Agents", &["Command", "Resume", "Status signals", "Add Preset", "Hooks"]),
        (SettingsPage::Editor, icons::CODE, "Editor", &["Language features", "Start servers only when needed", "Completion while typing", "Format on save"]),
        (SettingsPage::Notifications, icons::BELL, "Notifications", &["An agent needs input", "A process failed", "An agent finished its turn", "A long command finished", "Badge on the Dock icon"]),
        (SettingsPage::Updates, icons::ARROW_CIRCLE_UP, "Updates", &["Check for updates automatically", "Version"]),
        (SettingsPage::About, icons::INFO, "About", &["License"]),
    ]
}

/// Whether a row's label or note matches the search.
fn hit(search: &str, label: &str, note: &str) -> bool {
    let q = search.trim().to_lowercase();
    q.is_empty() || label.to_lowercase().contains(&q) || note.to_lowercase().contains(&q)
}

/// A page's title and the line under it.
fn page_head(ui: &mut Ui, title: &str, lead: &str) {
    let p = style::pal(ui);
    ui.label(RichText::new(title).font(style::semibold(17.0)).color(p.text));
    ui.add_space(4.0);
    ui.label(RichText::new(lead).size(12.5).color(p.text_muted));
    ui.add_space(16.0);
}

/// A small upper-case title over a group of rows.
fn group(ui: &mut Ui, text: &str) {
    let p = style::pal(ui);
    ui.add_space(18.0);
    let g = kit::section_galley(ui.painter(), text, p.text_faint);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), g.size().y), Sense::hover());
    ui.painter().galley(r.min, g, p.text_faint);
    ui.add_space(8.0);
}

/// Rows in a card: each the setting's name and a line about it at the left, its control at the
/// right, a line between rows.
struct Card<'a> {
    ui: &'a mut Ui,
    search: String,
    first: bool,
    rows: usize,
    /// An icon before the next row's name (a status, on Notifications).
    icon: Option<(&'static str, Color32)>,
}

impl Card<'_> {
    fn row(&mut self, label: &str, note: &str, control: impl FnOnce(&mut Ui)) {
        self.row_with(label, note, false, control);
    }

    fn row_icon(&mut self, icon: &'static str, color: Color32, label: &str, note: &str, control: impl FnOnce(&mut Ui)) {
        self.icon = Some((icon, color));
        self.row_with(label, note, false, control);
        self.icon = None;
    }

    /// `stacked`: the control under the text (a wide one, such as a grid of schemes).
    fn row_with(&mut self, label: &str, note: &str, stacked: bool, control: impl FnOnce(&mut Ui)) {
        if !hit(&self.search, label, note) {
            return;
        }
        let ui = &mut *self.ui;
        let p = style::pal(ui);
        if !self.first {
            let y = ui.cursor().top();
            let x = ui.max_rect().x_range();
            ui.painter().hline(x, y + 0.5, Stroke::new(1.0, p.border));
        }
        self.first = false;
        self.rows += 1;
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, 11)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let icon = self.icon;
            let text = |ui: &mut Ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    match icon {
                        Some((i, c)) => {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.label(RichText::new(i).font(style::icon_font(13.0)).color(c));
                                ui.label(RichText::new(label).font(style::medium(13.0)).color(p.text));
                            });
                        }
                        None => {
                            ui.label(RichText::new(label).font(style::medium(13.0)).color(p.text));
                        }
                    }
                    if !note.is_empty() {
                        ui.add(egui::Label::new(RichText::new(note).size(style::SMALL).color(p.text_faint).line_height(Some(16.0))).wrap());
                    }
                });
            };
            if stacked {
                text(ui);
                ui.add_space(10.0);
                control(ui);
            } else {
                egui::Sides::new().height(26.0).spacing(16.0).shrink_left().wrap().show(
                    ui,
                    |ui| text(ui),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        control(ui)
                    },
                );
            }
        });
    }
}

/// A card of rows: drawn only when one of them matches the search. Returns whether it was.
fn card(ui: &mut Ui, search: &str, labels: &[(&str, &str)], add: impl FnOnce(&mut Card)) -> bool {
    if !labels.iter().any(|(l, n)| hit(search, l, n)) {
        return false;
    }
    let p = style::pal(ui);
    egui::Frame::NONE
        .fill(p.canvas)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut c = Card { ui, search: search.to_string(), first: true, rows: 0, icon: None };
            add(&mut c);
        });
    true
}

impl App {
    /// Draws the settings dialog; returns false when it should close.
    pub fn settings_ui(&mut self, ui: &mut Ui, page: &mut SettingsPage) -> bool {
        if *page == SettingsPage::Integrations {
            *page = SettingsPage::Agents;
        }
        let screen = ui.ctx().content_rect();
        let w = W.min(screen.width() - 40.0);
        let h = H.min(screen.height() - 60.0);
        ui.set_width(w);
        ui.set_height(h);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let mut keep = true;
        let p = style::pal(ui);
        let full = Rect::from_min_size(ui.max_rect().min, vec2(w, h));

        // ----- header: title, search, close -------------------------------------------------
        let head = Rect::from_min_size(full.min, vec2(w, HEAD_H));
        ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.border));
        let g = ui.painter().layout_no_wrap(t("Settings").to_string(), style::semibold(15.5), p.text);
        let tw = g.size().x;
        ui.painter().galley(pos2(head.left() + 20.0, head.center().y - g.size().y / 2.0), g, p.text);
        ui.painter().text(pos2(head.left() + 20.0 + tw + 12.0, head.center().y), Align2::LEFT_CENTER, t("Saved as you change them."), egui::FontId::proportional(style::SMALL), p.text_faint);
        let close = Rect::from_center_size(pos2(head.right() - 12.0 - 13.0, head.center().y), vec2(26.0, 26.0));
        if ui.scope_builder(egui::UiBuilder::new().max_rect(close), |ui| kit::icon_button_ex(ui, icons::X, t("Close"), false, 26.0)).inner.clicked() {
            keep = false;
        }
        let search_rect = Rect::from_min_size(pos2(close.left() - 12.0 - 220.0, head.center().y - 13.0), vec2(220.0, 26.0));
        let mut search = self.settings_search.clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(search_rect), |ui| {
            kit::field(ui, egui::Id::new("settings-search"), &mut search, Some(icons::MAGNIFYING_GLASS), t("Search settings"), 220.0, true)
        });
        self.settings_search = search.clone();

        // ----- the pages at the left ------------------------------------------------------------
        let nav = Rect::from_min_max(pos2(full.left(), head.bottom()), pos2(full.left() + NAV_W, full.bottom()));
        ui.painter().rect_filled(nav, CornerRadius { nw: 0, ne: 0, sw: style::RADIUS_LG + 2, se: 0 }, p.chrome);
        ui.painter().vline(nav.right() - 0.5, nav.y_range(), Stroke::new(1.0, p.border));
        let mut y = nav.top() + 14.0;
        for (i, (pg, icon, name, rows)) in pages().into_iter().enumerate() {
            if i == 6 {
                ui.painter().hline((nav.left() + 14.0)..=(nav.right() - 14.0), y + 10.0, Stroke::new(1.0, p.border));
                y += 21.0;
            }
            let label = t(name);
            let matches = !search.trim().is_empty() && rows.iter().any(|r| hit(&search, t(r), ""));
            let item = Rect::from_min_size(pos2(nav.left() + 8.0, y), vec2(NAV_W - 16.0, 30.0));
            y += 31.0;
            let r = ui.interact(item, egui::Id::new(("settings-page", i)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("settings:{name}"), item);
            let on = *page == pg;
            if on {
                ui.painter().rect_filled(item, CornerRadius::same(style::RADIUS), p.accent_soft);
            } else if r.hovered() {
                ui.painter().rect_filled(item, CornerRadius::same(style::RADIUS), p.faint_row());
            }
            let c = if on || r.hovered() { p.text } else { p.text_muted };
            ui.painter().text(pos2(item.left() + 10.0 + 7.5, item.center().y), Align2::CENTER_CENTER, icon, style::icon_font(15.0), c);
            ui.painter().text(pos2(item.left() + 10.0 + 15.0 + 9.0, item.center().y), Align2::LEFT_CENTER, label, style::medium(13.0), c);
            // At the right: what a page has for the user (an update, an agent without hooks).
            if pg == SettingsPage::Updates
                && let Some(v) = self.updater.available_version()
            {
                let w = kit::pill_width(ui.painter(), None, &v);
                kit::paint_pill(ui.painter(), pos2(item.right() - 8.0 - w, item.center().y), None, &v, p.accent_text, p.accent_soft);
            }
            if pg == SettingsPage::Agents {
                let n = self.agents_without_hooks();
                if n > 0 {
                    kit::paint_badge(ui.painter(), pos2(item.right() - 8.0, item.center().y), &n.to_string(), p.subtle, p.text_muted, 16.0);
                }
            }
            if matches {
                ui.painter().circle_filled(pos2(item.right() - 12.0, item.center().y), 3.0, p.accent);
            }
            if r.clicked() {
                *page = pg;
            }
        }

        // ----- the page -------------------------------------------------------------------------
        let main = Rect::from_min_max(pos2(nav.right(), head.bottom()), full.max);
        let before = self.settings.clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(main), |ui| {
            egui::ScrollArea::vertical().id_salt("settings-page").auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 26, right: 26, top: 20, bottom: 24 }).show(ui, |ui| {
                    ui.set_width(main.width() - 52.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    match *page {
                        SettingsPage::General => self.general_page(ui, &search),
                        SettingsPage::Appearance => self.appearance_page(ui, &search),
                        SettingsPage::Terminal => self.terminal_page(ui, &search),
                        SettingsPage::Agents | SettingsPage::Integrations => self.agents_page(ui, &search),
                        SettingsPage::Editor => self.editor_page(ui, &search),
                        SettingsPage::Notifications => self.notifications_page(ui, &search),
                        SettingsPage::Updates => self.updates_page(ui, &search),
                        SettingsPage::About => self.about_page(ui),
                    }
                });
            });
        });
        ui.allocate_rect(full, Sense::hover());
        if self.settings != before {
            if self.settings.language != before.language {
                crate::i18n::set_language(self.settings.language);
            }
            if self.settings.theme != before.theme {
                style::apply(ui.ctx(), self.settings.theme);
            }
            if self.settings.font_name != before.font_name {
                crate::app::apply_fonts(ui.ctx(), &self.settings);
            }
            if self.settings.lsp != before.lsp || self.settings.lsp_on_demand != before.lsp_on_demand {
                // On demand, open files wait for an edit or a language feature to start theirs.
                if self.settings.lsp && !self.settings.lsp_on_demand {
                    let ctx = ui.ctx().clone();
                    let open: Vec<std::path::PathBuf> = self.editors.values().flat_map(|e| e.files.iter().map(|f| f.path.clone())).collect();
                    for path in open {
                        self.lsp_open(&ctx, &path);
                    }
                } else if !self.settings.lsp {
                    self.lsp_stop_all();
                }
            }
            self.settings_changed();
        }
        keep
    }

    /// Agents whose status cannot come from hooks as things are set (they show "Active").
    pub fn agents_without_hooks(&self) -> usize {
        [AgentKind::Gemini, AgentKind::OpenCode]
            .into_iter()
            .filter(|a| self.installed(*a).is_none() && self.settings.integration(*a) != Integration::Off)
            .count()
    }

    fn general_page(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        page_head(ui, t("General"), t("The app itself."));
        let labels = [(t("Command line"), ""), (t("Data folder"), ""), (t("Version"), ""), (t("Check for updates automatically"), "")];
        card(ui, search, &labels, |c| {
            let cli_note = tf("{} in any terminal opens that folder here.", &["ternarycode ."]);
            c.row(t("Command line"), &cli_note, |ui| match crate::cli::installed() {
                Some(path) => {
                    ui.label(RichText::new(format!("→ {}", crate::paths::contract_tilde(path.parent().unwrap_or(&path)))).font(egui::FontId::monospace(11.5)).color(p.text_muted))
                        .on_hover_text(t("Type ternarycode <folder or project name> in any terminal to open it here."));
                }
                None => {
                    #[cfg(target_os = "macos")]
                    if kit::button_ex(ui, kit::Kind::Secondary, None, t("Install ternarycode Command…"), true).clicked() {
                        match crate::cli::install_with_admin() {
                            Ok(path) => self.toast_ok(tf("Installed: {}", &[&path.display().to_string()])),
                            Err(e) => self.toast_err(e),
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    ui.label(RichText::new(t("Not on PATH (works in TernaryCode's own panes)")).size(style::SMALL).color(p.text_muted));
                }
            });
            let dir = crate::paths::contract_tilde(crate::paths::data_dir());
            c.row(t("Data folder"), &dir, |ui| {
                if kit::button_ex(ui, kit::Kind::Secondary, None, t("Show"), true).clicked() {
                    crate::platform::reveal(crate::paths::data_dir());
                }
            });
            let version = format!("TernaryCode {}", crate::update::CURRENT);
            c.row(t("Version"), &version, |ui| {
                if kit::button_ex(ui, kit::Kind::Secondary, None, t("Check for Updates"), true).clicked() {
                    self.actions.push(Action::CheckForUpdates);
                }
            });
            c.row(t("Check for updates automatically"), t("The only network connection TernaryCode makes."), |ui| {
                kit::switch(ui, &mut self.settings.check_updates);
            });
        });
    }

    fn appearance_page(&mut self, ui: &mut Ui, search: &str) {
        page_head(ui, t("Appearance"), t("How TernaryCode and its terminals look."));
        let labels = [(t("Theme"), ""), (t("Language"), t("System follows the language of your computer."))];
        card(ui, search, &labels, |c| {
            c.row(t("Theme"), "", |ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for (mode, name) in [(ThemeMode::Dark, t("Dark")), (ThemeMode::Light, t("Light")), (ThemeMode::System, t("System"))] {
                    if theme_card(ui, mode, name, self.settings.theme == mode).clicked() {
                        self.settings.theme = mode;
                    }
                }
            });
            c.row(t("Language"), t("System follows the language of your computer."), |ui| {
                let system = if sys_locale::get_locales().next().is_some_and(|l| l.starts_with("id") || l.starts_with("in")) {
                    "Bahasa Indonesia"
                } else {
                    "English"
                };
                let options = [tf("System ({})", &[system]), "English".to_string(), "Bahasa Indonesia".to_string()];
                let current = match self.settings.language {
                    Language::System => 0,
                    Language::English => 1,
                    Language::Indonesian => 2,
                };
                let opts: Vec<&str> = options.iter().map(String::as_str).collect();
                if let Some(i) = kit::select(ui, "language", &options[current], &opts, 160.0) {
                    self.settings.language = [Language::System, Language::English, Language::Indonesian][i];
                }
            });
        });
        let labels = [(t("With the dark theme"), ""), (t("With the light theme"), "")];
        if labels.iter().any(|(l, n)| hit(search, l, n)) {
            group(ui, t("Terminal colors"));
        }
        card(ui, search, &labels, |c| {
            for dark in [true, false] {
                let label = if dark { t("With the dark theme") } else { t("With the light theme") };
                c.row_with(label, "", true, |ui| {
                    let current = self.scheme(dark).name;
                    let schemes: Vec<&'static tc_term::scheme::Scheme> = tc_term::scheme::SCHEMES.iter().filter(|s| s.dark == dark).collect();
                    let cols = 5;
                    let gap = 10.0;
                    let w = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).floor();
                    for chunk in schemes.chunks(cols) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for s in chunk {
                                if scheme_tile(ui, s, w, s.name == current).clicked() {
                                    if dark {
                                        self.settings.scheme_dark = s.name.to_string();
                                    } else {
                                        self.settings.scheme_light = s.name.to_string();
                                    }
                                }
                            }
                        });
                        ui.add_space(gap);
                    }
                });
            }
        });
        let labels = [(t("Font"), t("Monospace fonts installed on this computer.")), (t("Size"), ""), (t("Cursor"), ""), (t("Blinking cursor"), "")];
        if labels.iter().any(|(l, n)| hit(search, l, n)) {
            group(ui, t("Terminal text"));
        }
        card(ui, search, &labels, |c| {
            c.row(t("Font"), t("Monospace fonts installed on this computer."), |ui| {
                let fonts: Vec<&'static str> = tc_term::fonts::PRESETS.iter().filter(|f| tc_term::fonts::is_available(f)).map(|f| f.name).collect();
                let current = tc_term::fonts::effective_choice(&tc_term::prefs::FontChoice { name: self.settings.font_name.clone(), path: self.settings.font_path.clone(), index: 0 }).name;
                if let Some(i) = kit::select(ui, "font", &current, &fonts, 160.0) {
                    self.settings.font_name = fonts[i].to_string();
                    self.settings.font_path = None;
                }
            });
            c.row(t("Size"), &tf("{} and {} change it too.", &[kit::keys(&Action::FontBigger).unwrap_or(""), kit::keys(&Action::FontSmaller).unwrap_or("")]), |ui| {
                let size = self.settings.font_size;
                let text = if size.fract() == 0.0 { format!("{size:.0}") } else { format!("{size:.1}") };
                match kit::stepper(ui, &text) {
                    -1 => self.settings.font_size = (size - 0.5).max(9.0),
                    1 => self.settings.font_size = (size + 0.5).min(24.0),
                    _ => {}
                }
            });
            c.row(t("Cursor"), t("New panes take it."), |ui| {
                let options = [kit::Seg::new(t("Block"), ""), kit::Seg::new(t("Beam"), ""), kit::Seg::new(t("Underline"), "")];
                let on = self.settings.cursor_shape as usize;
                if let Some(i) = kit::segmented(ui, &options, on) {
                    self.settings.cursor_shape = [CursorShape::Block, CursorShape::Beam, CursorShape::Underline][i];
                }
            });
            c.row(t("Blinking cursor"), "", |ui| {
                kit::switch(ui, &mut self.settings.cursor_blink);
            });
        });
    }

    fn terminal_page(&mut self, ui: &mut Ui, search: &str) {
        page_head(ui, t("Terminal"), t("How panes behave."));
        let labels = [
            (t("Scrollback per pane"), t("Applies to new panes.")),
            (t("Scrollback memory for all panes"), ""),
            (t("Option key as Meta"), ""),
            (t("Copy on select"), ""),
            (t("Programs may copy to the clipboard"), ""),
            (t("Shell integration"), ""),
        ];
        card(ui, search, &labels, |c| {
            c.row(t("Scrollback per pane"), t("Applies to new panes."), |ui| {
                let n = self.settings.scrollback;
                match kit::stepper(ui, &tf("{} lines", &[&group_digits(n)])) {
                    -1 => self.settings.scrollback = n.saturating_sub(1000).max(1000),
                    1 => self.settings.scrollback = (n + 1000).min(100_000),
                    _ => {}
                }
            });
            c.row(t("Scrollback memory for all panes"), t("Older lines are dropped first when it fills."), |ui| {
                let mb = self.settings.scrollback_budget_mb;
                match kit::stepper(ui, &format!("{mb} MB")) {
                    -1 => self.settings.scrollback_budget_mb = mb.saturating_sub(64).max(64),
                    1 => self.settings.scrollback_budget_mb = (mb + 64).min(16_384),
                    _ => {}
                }
            });
            if cfg!(target_os = "macos") {
                c.row(t("Option key as Meta"), "", |ui| {
                    kit::switch(ui, &mut self.settings.option_as_meta);
                });
            }
            c.row(t("Copy on select"), "", |ui| {
                kit::switch(ui, &mut self.settings.copy_on_select);
            });
            c.row(t("Programs may copy to the clipboard"), t("vim, tmux and agents copy through OSC 52. They can never read it."), |ui| {
                kit::switch(ui, &mut self.settings.program_copy);
            });
            c.row(t("Shell integration"), t("Marks prompts and commands (zsh, bash, fish, PowerShell) so failures and long commands are noticed."), |ui| {
                kit::switch(ui, &mut self.settings.shell_integration);
            });
        });
    }

    fn editor_page(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        page_head(ui, t("Editor"), t("For reading and small edits."));
        let labels = [
            (t("Language features"), ""),
            (t("Start servers only when needed"), ""),
            (t("Completion while typing"), ""),
            (t("Format on save"), ""),
        ];
        card(ui, search, &labels, |c| {
            c.row(t("Language features"), t("Uses the language servers installed on this computer."), |ui| {
                kit::switch(ui, &mut self.settings.lsp);
            });
            let on = self.settings.lsp;
            c.row(t("Start servers only when needed"), t("A file you only read costs nothing."), |ui| {
                ui.add_enabled_ui(on, |ui| kit::switch(ui, &mut self.settings.lsp_on_demand));
            });
            c.row(t("Completion while typing"), "", |ui| {
                ui.add_enabled_ui(on, |ui| kit::switch(ui, &mut self.settings.auto_complete));
            });
            c.row(t("Format on save"), "", |ui| {
                ui.add_enabled_ui(on, |ui| kit::switch(ui, &mut self.settings.format_on_save));
            });
        });
        if !search.trim().is_empty() {
            return;
        }
        group(ui, t("Found on this computer"));
        let path = self.lsp.path_env.clone().or_else(|| std::env::var_os("PATH")).unwrap_or_default();
        let servers: Vec<(&'static tc_lsp::servers::Spec, bool, Option<bool>)> = tc_lsp::servers::SERVERS
            .iter()
            .map(|spec| {
                let found = tc_lsp::servers::resolve(spec.program, &path).is_some();
                let running = self.lsp.servers.values().find(|s| s.spec.name == spec.name).map(|s| s.state == crate::lsp::ServerState::Ready);
                (spec, found, running)
            })
            .collect();
        egui::Frame::NONE
            .fill(p.canvas)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, (spec, found, running)) in servers.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
                    if i > 0 {
                        ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border));
                    }
                    let (dot, note) = match (found, running) {
                        (_, Some(true)) => (Some(p.ok), t("running").to_string()),
                        (true, _) => (None, t("ready").to_string()),
                        (false, _) => (Some(p.warn), t("not installed").to_string()),
                    };
                    let cy = r.center().y;
                    match dot {
                        Some(c) => {
                            ui.painter().circle_filled(pos2(r.left() + 14.0 + 3.0, cy), 3.0, c);
                        }
                        None => {
                            ui.painter().circle_stroke(pos2(r.left() + 14.0 + 3.0, cy), 2.5, Stroke::new(1.0, p.text_faint));
                        }
                    }
                    ui.painter().text(pos2(r.left() + 14.0 + 6.0 + 10.0, cy), Align2::LEFT_CENTER, spec.name, style::medium(12.5), p.text);
                    let mut right = r.right() - 14.0;
                    if !found {
                        let g = ui.painter().layout_no_wrap(t("Copy install command").to_string(), egui::FontId::proportional(style::SMALL), p.accent_text);
                        right -= g.size().x;
                        let lr = Rect::from_min_size(pos2(right, cy - g.size().y / 2.0), g.size());
                        ui.painter().galley(lr.min, g, p.accent_text);
                        let resp = ui.interact(lr, egui::Id::new(("copy-install", spec.name)), Sense::click()).on_hover_text(spec.install);
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() {
                            ui.ctx().copy_text(spec.install.to_string());
                            self.toast_ok(t("Install command copied.").into());
                        }
                        right -= 10.0;
                    }
                    ui.painter().text(pos2(right, cy), Align2::RIGHT_CENTER, note, egui::FontId::proportional(style::SMALL), p.text_faint);
                }
            });
    }

    fn notifications_page(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        page_head(ui, t("Notifications"), t("Only when you are not already looking. Keyboard focus never moves by itself."));
        let labels = [
            (t("An agent needs input"), ""),
            (t("A process failed"), ""),
            (t("An agent finished its turn"), ""),
            (t("A long command finished"), ""),
            (t("Badge on the Dock icon"), ""),
        ];
        card(ui, search, &labels, |c| {
            let icon_row = |c: &mut Card, icon: &'static str, color: Color32, label: &str, note: &str, on: &mut bool| {
                c.row_icon(icon, color, label, note, |ui| {
                    kit::switch(ui, on);
                });
            };
            let n = &mut self.settings.notifications;
            icon_row(c, icons::BELL, p.warn, t("An agent needs input"), "", &mut n.needs_input);
            icon_row(c, icons::X_CIRCLE, p.err, t("A process failed"), "", &mut n.failed);
            icon_row(c, icons::CHECK_CIRCLE, p.ok, t("An agent finished its turn"), "", &mut n.done);
            let secs = self.settings.long_command_secs;
            let mut long_on = n.command_done;
            c.row_icon(icons::CHECK_CIRCLE, p.ok, t("A long command finished"), &tf("“Long” means at least {} s", &[&secs.to_string()]), |ui| {
                kit::switch(ui, &mut long_on);
                ui.add_space(10.0);
                match kit::stepper(ui, &format!("{secs} s")) {
                    -1 => self.settings.long_command_secs = secs.saturating_sub(1).max(3),
                    1 => self.settings.long_command_secs = (secs + 1).min(600),
                    _ => {}
                }
            });
            self.settings.notifications.command_done = long_on;
            if cfg!(target_os = "macos") {
                c.row(t("Badge on the Dock icon"), t("Counts agents waiting for input."), |ui| {
                    kit::switch(ui, &mut self.settings.notifications.dock_badge);
                });
            }
        });
    }

    fn updates_page(&mut self, ui: &mut Ui, search: &str) {
        page_head(ui, t("Updates"), t("TernaryCode updates itself from its GitHub releases."));
        let labels = [(t("Version"), ""), (t("Check for updates automatically"), "")];
        card(ui, search, &labels, |c| {
            let version = format!("TernaryCode {}", crate::update::CURRENT);
            let available = self.updater.available_version();
            let note = match &available {
                Some(v) => tf("Version {} is available.", &[v]),
                None => version.clone(),
            };
            c.row(t("Version"), &note, |ui| {
                if available.is_some() {
                    if kit::button_ex(ui, kit::Kind::Primary, None, t("Install Update…"), true).clicked() {
                        self.actions.push(Action::ShowUpdate);
                    }
                } else if kit::button_ex(ui, kit::Kind::Secondary, None, t("Check for Updates"), true).clicked() {
                    self.actions.push(Action::CheckForUpdates);
                }
            });
            c.row(t("Check for updates automatically"), t("At launch and every few hours. The only network connection TernaryCode makes."), |ui| {
                kit::switch(ui, &mut self.settings.check_updates);
            });
        });
    }

    fn about_page(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
            paint_logo(ui.painter(), r, &p);
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.label(RichText::new("TernaryCode").font(style::semibold(17.0)).color(p.text));
                ui.label(RichText::new(tf("Version {}", &[crate::update::CURRENT])).size(12.5).color(p.text_muted));
            });
        });
        ui.add_space(16.0);
        ui.label(RichText::new(t("Every project and its agents, in one window.")).size(13.0).color(p.text));
        ui.add_space(6.0);
        ui.label(RichText::new(t("MIT license. Fonts: Inter (SIL OFL 1.1) and Phosphor icons (MIT).")).size(style::SMALL).color(p.text_faint));
        ui.add_space(14.0);
        if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), t("Open Releases Page"), true).clicked() {
            crate::update::open_page(&format!("https://github.com/{}/releases/latest", crate::update::REPO));
        }
    }

    fn agents_page(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        page_head(ui, t("Agents"), &tf("Presets you can run with {}, and how each agent tells TernaryCode that it needs you.", &[kit::keys(&Action::PickPreset).unwrap_or("")]));
        let mut presets = self.settings.presets();
        let mut changed = false;
        let mut remove = None;
        let open_id = egui::Id::new("settings-agent-open");
        let mut open: Option<String> = ui.data(|d| d.get_temp(open_id)).unwrap_or(Some("claude".to_string()));
        for (i, preset) in presets.iter_mut().enumerate() {
            if !hit(search, &preset.name, &preset.command) && !hit(search, t("Command"), "") {
                continue;
            }
            let agent = preset.agent.unwrap_or(AgentKind::Custom);
            let expanded = open.as_deref() == Some(preset.id.as_str());
            let hooks = self.agent_hooks(agent);
            egui::Frame::NONE
                .fill(p.canvas)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(9))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    // Header: mark, name, how it reports; a click opens it.
                    let (head, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("agent-card:{}", preset.id), head);
                    let cy = head.center().y;
                    let tile = Rect::from_center_size(pos2(head.left() + 14.0 + 16.0, cy), vec2(32.0, 32.0));
                    ui.painter().rect_filled(tile, CornerRadius::same(8), p.subtle);
                    ui.painter().text(tile.center(), Align2::CENTER_CENTER, agent_mark(preset), style::semibold(11.5), p.text_muted);
                    let x = tile.right() + 12.0;
                    let g = ui.painter().layout_no_wrap(preset.name.clone(), style::medium(13.5), p.text);
                    let nw = g.size().x;
                    ui.painter().galley(pos2(x, cy - 9.0 - g.size().y / 2.0), g, p.text);
                    let (icon, text, fg, bg) = hooks.pill(&p, ui.visuals().dark_mode);
                    kit::paint_pill(ui.painter(), pos2(x + nw + 8.0, cy - 9.0), icon, &text, fg, bg);
                    let sub = if expanded && hooks.active > 0 {
                        tf("Active in {} panes", &[&hooks.active.to_string()])
                    } else {
                        match &preset.resume {
                            Some(r) if !r.is_empty() => format!("{}  ·  {} {r}", preset.command, t("resume:")),
                            _ => preset.command.clone(),
                        }
                    };
                    let font = if expanded && hooks.active > 0 { egui::FontId::proportional(style::SMALL) } else { egui::FontId::monospace(11.0) };
                    ui.painter().text(pos2(x, cy + 10.0), Align2::LEFT_CENTER, sub, font, p.text_faint);
                    ui.painter().text(pos2(head.right() - 14.0 - 6.0, cy), Align2::CENTER_CENTER, if expanded { icons::CARET_UP } else { icons::CARET_DOWN }, style::icon_font(12.0), p.text_faint);
                    if !hooks.hooked && hooks.can_install && self.installed(agent).is_none() {
                        let w = kit::button_width(ui, None, t("Install…"), None, true);
                        let br = Rect::from_center_size(pos2(head.right() - 14.0 - 22.0 - w / 2.0, cy), vec2(w, 24.0));
                        if ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| kit::button_ex(ui, kit::Kind::Primary, None, t("Install…"), true)).inner.clicked() {
                            self.actions.push(Action::PlanInstall(agent));
                        }
                    }
                    if resp.clicked() {
                        open = if expanded { None } else { Some(preset.id.clone()) };
                    }
                    if !expanded {
                        return;
                    }
                    // Opened: the commands, the hooks, a note.
                    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, p.border));
                    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 10.0;
                        let field_row = |ui: &mut Ui, label: &str, add: &mut dyn FnMut(&mut Ui)| {
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(vec2(110.0, 26.0), Sense::hover());
                                ui.painter().text(r.left_center(), Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), p.text_muted);
                                add(ui);
                            });
                        };
                        if !preset.builtin {
                            field_row(ui, t("Name"), &mut |ui| {
                                let id = egui::Id::new(("preset-name", i));
                                changed |= kit::field(ui, id, &mut preset.name, None, "", ui.available_width(), true).changed();
                            });
                        }
                        field_row(ui, t("Command"), &mut |ui| {
                            let id = egui::Id::new(("preset-command", i));
                            changed |= mono_field(ui, id, &mut preset.command).changed();
                        });
                        field_row(ui, t("Resume"), &mut |ui| {
                            let id = egui::Id::new(("preset-resume", i));
                            let mut resume = preset.resume.clone().unwrap_or_default();
                            if mono_field(ui, id, &mut resume).changed() {
                                preset.resume = Some(resume).filter(|r| !r.trim().is_empty());
                                changed = true;
                            }
                        });
                        if !preset.builtin {
                            field_row(ui, t("Environment"), &mut |ui| {
                                let id = egui::Id::new(("preset-env", i));
                                let mut text: String = preset.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ");
                                if mono_field(ui, id, &mut text).changed() {
                                    preset.env = text
                                        .split_whitespace()
                                        .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                                        .collect();
                                    changed = true;
                                }
                            });
                        }
                        if agent != AgentKind::Custom {
                            field_row(ui, t("Status signals"), &mut |ui| {
                                self.hooks_controls(ui, agent);
                            });
                            ui.horizontal(|ui| {
                                ui.add_space(110.0 + 8.0);
                                ui.add(egui::Label::new(RichText::new(hooks_note(agent)).size(style::SMALL).color(p.text_faint).line_height(Some(17.0))).wrap());
                            });
                            if hooks.unconnected > 0 {
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(110.0 + 8.0);
                                    let text = crate::i18n::count(
                                        hooks.unconnected,
                                        t("Typed by hand in 1 pane, it is not connected: Install connects it (then restart it there)."),
                                        t("Typed by hand in {} panes, it is not connected: Install connects it (then restart it there)."),
                                    );
                                    ui.add(egui::Label::new(RichText::new(text).size(style::SMALL).color(p.warn).line_height(Some(17.0))).wrap());
                                });
                            }
                        } else {
                            ui.horizontal(|ui| {
                                ui.add_space(110.0 + 8.0);
                                ui.add(egui::Label::new(RichText::new(t("A custom agent reports through ternarycode-hook with start, working, needs-input, done and end; without it, its status comes from the terminal.")).size(style::SMALL).color(p.text_faint)).wrap());
                            });
                            ui.horizontal(|ui| {
                                ui.add_space(110.0 + 8.0);
                                if kit::button_ex(ui, kit::Kind::Danger, Some(icons::TRASH), t("Delete Preset"), true).clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                    });
                });
            ui.add_space(10.0);
        }
        ui.data_mut(|d| d.insert_temp(open_id, open));
        if let Some(i) = remove {
            presets.remove(i);
            changed = true;
        }
        ui.horizontal(|ui| {
            if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::PLUS), t("Add Preset"), true).clicked() {
                let n = presets.iter().filter(|p| !p.builtin).count() + 1;
                // An id no preset has (counting would repeat one after a deletion).
                let mut k = n;
                while presets.iter().any(|p| p.id == format!("custom-{k}")) {
                    k += 1;
                }
                let id = format!("custom-{k}");
                presets.push(Preset {
                    id: id.clone(),
                    name: tf("My agent {}", &[&n.to_string()]),
                    agent: Some(AgentKind::Custom),
                    command: String::new(),
                    resume: None,
                    env: Default::default(),
                    builtin: false,
                });
                ui.data_mut(|d| d.insert_temp(open_id, Some(id)));
                changed = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let exists = self.hook_bin.bin.exists();
                ui.add(
                    egui::Label::new(
                        RichText::new(tf("Hook program: {}{}", &[&crate::paths::contract_tilde(&self.hook_bin.bin), if exists { "" } else { t(" (missing)") }]))
                            .size(style::SMALL)
                            .color(if exists { p.text_faint } else { p.err }),
                    )
                    .truncate(),
                );
                ui.label(RichText::new(if exists { icons::CHECK_CIRCLE } else { icons::WARNING }).font(style::icon_font(12.0)).color(if exists { p.ok } else { p.err }));
            });
        });
        if changed {
            // Saved: every user preset, and built-ins only where they differ from the default.
            let defaults = tc_core::agent::builtin_presets();
            self.settings.presets = presets.into_iter().filter(|p| !p.builtin || !defaults.contains(p)).collect();
        }
    }

    /// Per launch | Installed | Off, and Install… or Uninstall.
    fn hooks_controls(&mut self, ui: &mut Ui, agent: AgentKind) {
        let per_launch = matches!(agent, AgentKind::Claude | AgentKind::Codex);
        let installed = self.installed(agent);
        // Per launch is the default (and what Uninstall leaves), but an agent without it gets no
        // hooks then: that is Off. Nothing is saved unless a choice is made.
        let shown = match self.settings.integration(agent) {
            Integration::PerLaunch if !per_launch => Integration::Off,
            m => m,
        };
        let mut modes = Vec::new();
        if per_launch {
            modes.push((Integration::PerLaunch, t("Per launch")));
        }
        modes.push((Integration::Installed, t("Installed")));
        modes.push((Integration::Off, t("Off")));
        let options: Vec<kit::Seg> = modes.iter().map(|(_, l)| kit::Seg::new(l, "")).collect();
        let on = modes.iter().position(|(m, _)| *m == shown).unwrap_or(0);
        if let Some(i) = kit::segmented(ui, &options, on) {
            let m = modes[i].0;
            if m == Integration::Installed && installed.is_none() {
                self.actions.push(Action::PlanInstall(agent));
            } else {
                self.settings.integrations.insert(agent.slug().into(), m);
            }
        }
        ui.add_space(10.0);
        if installed.is_some() {
            if kit::button_ex(ui, kit::Kind::Secondary, None, t("Uninstall"), true).clicked() {
                self.actions.push(Action::Uninstall(agent));
            }
        } else {
            let r = kit::button_ex(ui, kit::Kind::Secondary, None, t("Install…"), true);
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("install:{}", agent.slug()), r.rect);
            if r.clicked() {
                self.actions.push(Action::PlanInstall(agent));
            }
        }
    }

    /// How an agent reports to TernaryCode as things are set, and in how many panes it runs with
    /// its hooks now.
    fn agent_hooks(&self, agent: AgentKind) -> Hooks {
        let installed = self.installed(agent).is_some();
        let per_launch = matches!(agent, AgentKind::Claude | AgentKind::Codex) && self.settings.integration(agent) == Integration::PerLaunch;
        let active = self.panes.values().filter(|p| p.tracker.hooks_active && p.tracker.agent == Some(agent)).count();
        let unconnected = self.unconnected_agents().iter().filter(|x| x.1 == agent).count();
        Hooks {
            agent,
            hooked: installed || per_launch,
            installed,
            can_install: !matches!(agent, AgentKind::Custom | AgentKind::Antigravity),
            off: self.settings.integration(agent) == Integration::Off,
            active,
            unconnected,
        }
    }
}

struct Hooks {
    agent: AgentKind,
    hooked: bool,
    installed: bool,
    can_install: bool,
    off: bool,
    active: usize,
    /// Panes where it runs without the hooks (typed by hand).
    unconnected: usize,
}

impl Hooks {
    fn pill(&self, p: &style::Palette, dark: bool) -> (Option<&'static str>, String, Color32, Color32) {
        if self.agent == AgentKind::Custom {
            return (None, t("Custom").to_string(), p.text_muted, p.subtle);
        }
        // Per launch covers what Run Agent starts, not the agent typed by hand.
        if self.hooked && !self.installed && self.unconnected > 0 {
            return (Some(icons::WARNING), t("Hooks · Run Agent only").to_string(), p.warn, style::tint(p.warn, dark));
        }
        if self.hooked {
            let how = if self.installed { t("installed") } else { t("Run Agent only") };
            return (Some(icons::PLUGS_CONNECTED), tf("Hooks · {}", &[how]), p.ok, style::tint(p.ok, dark));
        }
        if self.off {
            return (None, t("Hooks off").to_string(), p.text_muted, p.subtle);
        }
        (Some(icons::WARNING), t("No hooks · status shows “Active”").to_string(), p.warn, style::tint(p.warn, dark))
    }
}

/// What each agent's hooks report, and how they are added.
fn hooks_note(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Claude => t("Per launch adds TernaryCode's hooks to each run (--settings) without changing your files. Installed writes them into ~/.claude/settings.json, with a backup. Hooks report permission requests, questions and finished turns. Checked with Claude Code 2.1.319."),
        AgentKind::Codex => t("Hooks and notify. Codex asks you once to trust TernaryCode's hooks (its own security check). Checked with Codex 0.162.1."),
        AgentKind::Gemini => t("Install adds hooks to ~/.gemini/settings.json (tool permissions, finished turns). Gemini CLI has no per-launch option. Checked with Gemini CLI 0.43.0."),
        _ => t("Install adds a small plugin file to OpenCode's plugins folder (permission requests, finished turns). OpenCode has no per-launch option. Checked with OpenCode 1.19.5."),
    }
}

/// Two letters for an agent's tile.
fn agent_mark(preset: &Preset) -> String {
    match preset.agent {
        Some(AgentKind::Claude) => "CC".into(),
        Some(AgentKind::Codex) => "Cx".into(),
        Some(AgentKind::Gemini) => "Gm".into(),
        Some(AgentKind::OpenCode) => "OC".into(),
        _ => preset.name.chars().filter(|c| c.is_alphanumeric()).take(2).collect::<String>(),
    }
}

/// A monospace text field the width left in the row.
fn mono_field(ui: &mut Ui, id: egui::Id, text: &mut String) -> egui::Response {
    let p = style::pal(ui);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    ui.painter().rect(rect, CornerRadius::same(style::RADIUS), p.canvas, Stroke::new(1.0, if focused { p.accent } else { p.border_strong }), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(8.0, 1.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.add_sized(
        inner.size(),
        egui::TextEdit::singleline(text).id(id).frame(egui::Frame::NONE).margin(egui::Margin::ZERO).font(egui::FontId::monospace(12.0)).vertical_align(egui::Align::Center),
    )
}

/// "10,000".
fn group_digits(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A theme's preview card: a miniature window (sidebar and content) in its colours.
fn theme_card(ui: &mut Ui, mode: ThemeMode, name: &str, on: bool) -> egui::Response {
    let p = style::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(92.0, 58.0 + 6.0 + 16.0), Sense::click());
    let card = Rect::from_min_size(rect.min, vec2(92.0, 58.0));
    let dark = style::palette(true);
    let light = style::palette(false);
    let painter = ui.painter();
    let paint_half = |r: Rect, pal: &style::Palette, side: bool| {
        if side {
            painter.rect_filled(Rect::from_min_size(r.min, vec2(26.0_f32.min(r.width()), r.height())), CornerRadius::ZERO, pal.chrome);
        }
        let content = if side { Rect::from_min_max(pos2(r.left() + 26.0, r.top()), r.max) } else { r };
        painter.rect_filled(content, CornerRadius::ZERO, pal.canvas);
        let l = Rect::from_min_size(content.min + vec2(6.0, 8.0), vec2(content.width() * 0.6, 4.0));
        painter.rect_filled(l, CornerRadius::same(2), style::soft(pal.text_muted, 0.5));
        let l2 = Rect::from_min_size(l.min + vec2(0.0, 8.0), vec2(content.width() * 0.4, 4.0));
        painter.rect_filled(l2, CornerRadius::same(2), style::soft(pal.text_muted, 0.35));
    };
    let clip = painter.with_clip_rect(card);
    let _ = clip;
    match mode {
        ThemeMode::Dark => paint_half(card, &dark, true),
        ThemeMode::Light => paint_half(card, &light, true),
        ThemeMode::System => {
            let (a, b) = card.split_left_right_at_fraction(0.5);
            painter.rect_filled(a, CornerRadius::ZERO, light.chrome);
            painter.rect_filled(b, CornerRadius::ZERO, dark.chrome);
        }
    }
    let stroke = if on { Stroke::new(2.0, p.accent) } else if resp.hovered() { Stroke::new(1.0, p.text_faint) } else { Stroke::new(1.0, p.border_strong) };
    painter.rect_stroke(card, CornerRadius::same(7), stroke, StrokeKind::Outside);
    painter.text(pos2(card.center().x, card.bottom() + 6.0 + 8.0), Align2::CENTER_CENTER, name, egui::FontId::proportional(style::SMALL), if on { p.text } else { p.text_muted });
    resp
}

/// A terminal scheme's tile: its background with a prompt and its first six colours, its name.
fn scheme_tile(ui: &mut Ui, s: &tc_term::scheme::Scheme, w: f32, on: bool) -> egui::Response {
    let p = style::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 44.0 + 26.0), Sense::click());
    let painter = ui.painter();
    let top = Rect::from_min_size(rect.min, vec2(w, 44.0));
    let bottom = Rect::from_min_max(pos2(rect.left(), top.bottom()), rect.max);
    painter.rect_filled(top, CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, s.background);
    painter.rect_filled(bottom, CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 }, p.surface);
    let mono = egui::FontId::monospace(10.0);
    let mut job = egui::text::LayoutJob::default();
    job.append("~/app", 0.0, egui::TextFormat::simple(mono.clone(), s.ansi[4]));
    job.append("❯", 5.0, egui::TextFormat::simple(mono.clone(), s.ansi[8]));
    job.append("ls", 5.0, egui::TextFormat::simple(mono, s.foreground));
    let g = painter.layout_job(job);
    painter.galley(top.min + vec2(8.0, 6.0), g, s.foreground);
    for (i, c) in s.ansi[1..7].iter().enumerate() {
        let r = Rect::from_min_size(top.min + vec2(8.0 + i as f32 * 12.0, 24.0), vec2(9.0, 9.0));
        painter.rect_filled(r, CornerRadius::same(2), *c);
    }
    let mut job = egui::text::LayoutJob::simple_singleline(s.name.to_string(), style::medium(11.0), p.text);
    job.wrap = egui::text::TextWrapping::truncate_at_width(w - 16.0);
    let g = painter.layout_job(job);
    painter.galley(pos2(bottom.left() + 8.0, bottom.center().y - g.size().y / 2.0), g, p.text);
    let stroke = if on { Stroke::new(2.0, p.accent) } else if resp.hovered() { Stroke::new(1.0, p.text_faint) } else { Stroke::new(1.0, p.border) };
    painter.rect_stroke(rect, CornerRadius::same(8), stroke, if on { StrokeKind::Outside } else { StrokeKind::Inside });
    resp
}

/// The app's mark: a 3 × 3 lattice on the accent.
pub fn paint_logo(painter: &egui::Painter, r: Rect, p: &style::Palette) {
    painter.rect_filled(r, CornerRadius::same((r.width() * 0.25) as u8), p.accent);
    let inner = r.shrink(r.width() * 0.2);
    let gap = r.width() * 0.055;
    let cell = (inner.width() - 2.0 * gap) / 3.0;
    for i in 0..9 {
        let (cx, cy) = ((i % 3) as f32, (i / 3) as f32);
        let c = Rect::from_min_size(inner.min + vec2(cx * (cell + gap), cy * (cell + gap)), vec2(cell, cell));
        let a = if i % 2 == 0 { 0.9 } else { 0.35 };
        painter.rect_filled(c, CornerRadius::same(2), Color32::from_white_alpha((a * 255.0) as u8));
    }
}
