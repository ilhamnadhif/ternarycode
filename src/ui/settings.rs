//! Settings: a modal with its pages at the left (General, Appearance, Terminal, Agents, Editor,
//! Notifications, Updates, About), a search that narrows them, and each page as groups of rows
//! (what a setting is and a line about it at the left, its control at the right). Saved as they
//! change.

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use tc_agent::integrate::InstallRecord;
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
    /// Browser panes: where web pages open, sleeping, logins.
    Browser,
    /// Presets and how each agent reports to TernaryCode (its hooks).
    Agents,
    Editor,
    Notifications,
    Updates,
    About,
    /// Opens Agents (kept for "Agent Integrations…").
    Integrations,
}

const W: f32 = 1040.0;
const H: f32 = 768.0;
const NAV_W: f32 = 236.0;

/// The pages, their icons, and the rows on each (for the search).
fn pages() -> [(SettingsPage, &'static str, &'static str, &'static [&'static str]); 9] {
    [
        (SettingsPage::General, icons::GEAR_SIX, "General", &["Command line", "Data folder", "Version", "Check for updates automatically", "Group active projects at the top", "Worktrees"]),
        (SettingsPage::Appearance, icons::PAINT_BRUSH, "Appearance", &["Theme", "Language", "With the dark theme", "With the light theme", "Font", "Size", "Cursor", "Blinking cursor"]),
        (
            SettingsPage::Terminal,
            icons::TERMINAL,
            "Terminal",
            if cfg!(target_os = "macos") {
                &["Scrollback per pane", "Scrollback memory for all panes", "Option key as Meta", "Copy on select", "Programs may copy to the clipboard", "Shell integration"]
            } else {
                &["Scrollback per pane", "Scrollback memory for all panes", "Copy on select", "Programs may copy to the clipboard", "Shell integration"]
            },
        ),
        (SettingsPage::Browser, icons::GLOBE, "Browser", &["Open web pages in TernaryCode", "When an agent opens a page", "Put browsers to sleep after", "Pages in Grid tiles", "Logins & cookies", "Agents can use the browser", "Ask before agents open a site", "Sites agents may open without asking", "Agents may run JavaScript in pages"]),
        (SettingsPage::Agents, icons::PLUGS_CONNECTED, "Agents", &["Command", "Resume", "Status signals", "Add Preset", "Hooks"]),
        (SettingsPage::Editor, icons::CODE, "Editor", &["Language features", "Start servers only when needed", "Completion while typing", "Format on save"]),
        (
            SettingsPage::Notifications,
            icons::BELL,
            "Notifications",
            if cfg!(target_os = "macos") {
                &["An agent needs input", "A process failed", "An agent finished its turn", "A long command finished", "Badge on the Dock icon"]
            } else {
                &["An agent needs input", "A process failed", "An agent finished its turn", "A long command finished"]
            },
        ),
        (SettingsPage::Updates, icons::ARROW_CIRCLE_UP, "Updates", &["Check for updates automatically", "Version"]),
        (SettingsPage::About, icons::INFO, "About", &["License"]),
    ]
}

/// Whether a row's label or note matches the search.
fn hit(search: &str, label: &str, note: &str) -> bool {
    let q = search.trim().to_lowercase();
    q.is_empty() || label.to_lowercase().contains(&q) || note.to_lowercase().contains(&q)
}

/// A page's title and the line under it: drawn by the sheet's head (`settings_ui`), which
/// reads them from here.
fn page_head(ui: &mut Ui, title: &str, lead: &str) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("settings-head"), (title.to_string(), lead.to_string())));
}

/// The title over a group of rows.
fn group(ui: &mut Ui, text: &str) {
    let p = style::pal(ui);
    ui.add_space(16.0);
    let g = ui.painter().layout_no_wrap(text.to_string(), style::medium(12.0), p.text_muted);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), g.size().y), Sense::hover());
    ui.painter().galley(r.min + vec2(2.0, 0.0), g, p.text_muted);
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

    /// A row of only its control (the label is for the search), with the row's padding.
    fn bare(&mut self, label: &str, control: impl FnOnce(&mut Ui)) {
        self.bare_hit(label, "", control);
    }

    fn bare_hit(&mut self, label: &str, note: &str, control: impl FnOnce(&mut Ui)) {
        if !hit(&self.search, label, note) {
            return;
        }
        self.divider();
        egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 14, top: 14, bottom: 12 }).show(self.ui, |ui| {
            ui.set_width(ui.available_width());
            control(ui);
        });
    }

    /// A row without padding, shown when any of `labels` matches (rows of its own inside).
    fn bare_hit_flush(&mut self, labels: &[(&str, &str)], control: impl FnOnce(&mut Ui)) {
        if !labels.iter().any(|(l, n)| hit(&self.search, l, n)) {
            return;
        }
        self.divider();
        self.ui.scope(|ui| {
            ui.set_width(ui.available_width());
            control(ui);
        });
    }

    /// The line between rows (none before the first).
    fn divider(&mut self) {
        let ui = &mut *self.ui;
        let p = style::pal(ui);
        if !self.first {
            let y = ui.cursor().top();
            let x = ui.max_rect().x_range();
            ui.painter().hline(x, y + 0.5, Stroke::new(1.0, p.border));
        }
        self.first = false;
        self.rows += 1;
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
        egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 14, top: 12, bottom: 12 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(26.0);
            let icon = self.icon;
            let text = |ui: &mut Ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    match icon {
                        Some((i, c)) => {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.label(RichText::new(i).font(style::icon_fill(13.0)).color(c));
                                ui.label(RichText::new(label).size(13.0).color(p.text));
                            });
                        }
                        None => {
                            ui.label(RichText::new(label).size(13.0).color(p.text));
                        }
                    }
                    if !note.is_empty() {
                        ui.set_max_width(600.0);
                        ui.add(egui::Label::new(RichText::new(note).size(12.0).color(p.text_faint).line_height(Some(17.4))).wrap());
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

/// A site in the list agents may open without asking, with × to take it out. Returns whether ×
/// was clicked.
fn site_chip(ui: &mut Ui, site: &str) -> bool {
    let p = style::pal(ui);
    let g = ui.painter().layout_no_wrap(site.to_string(), egui::FontId::monospace(11.5), p.text);
    let (rect, _) = ui.allocate_exact_size(vec2(g.size().x + 10.0 + 18.0, 24.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(style::RADIUS), p.subtle, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    ui.painter().galley(pos2(rect.left() + 8.0, rect.center().y - g.size().y / 2.0), g, p.text);
    let x = Rect::from_center_size(pos2(rect.right() - 11.0, rect.center().y), vec2(16.0, 16.0));
    let r = ui.interact(x, ui.id().with(("site-x", site)), Sense::click()).on_hover_text(t("Remove"));
    let c = if r.hovered() { p.text } else { p.text_faint };
    ui.painter().text(x.center(), Align2::CENTER_CENTER, icons::X, style::icon_font(11.0), c);
    r.clicked()
}

/// A card of rows: drawn only when one of them matches the search. Returns whether it was.
fn card(ui: &mut Ui, search: &str, labels: &[(&str, &str)], add: impl FnOnce(&mut Card)) -> bool {
    if !labels.iter().any(|(l, n)| hit(search, l, n)) {
        return false;
    }
    let p = style::pal(ui);
    let dark = ui.visuals().dark_mode;
    let r = egui::Frame::NONE
        .fill(p.surface)
        .stroke(Stroke::new(1.0, if dark { p.border } else { p.border_strong }))
        .corner_radius(CornerRadius::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut c = Card { ui, search: search.to_string(), first: true, rows: 0, icon: None };
            add(&mut c);
        });
    kit::paint_highlight(ui.painter(), Rect::from_min_size(r.response.rect.min, vec2(r.response.rect.width(), 1.0)), 10, &p);
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
        let dark = ui.visuals().dark_mode;
        let full = Rect::from_min_size(ui.max_rect().min, vec2(w, h));
        kit::paint_highlight(ui.painter(), Rect::from_min_size(full.min, vec2(w, 1.0)), style::RADIUS_DIALOG, &p);

        // ----- the pages at the left: "Settings", the search, the pages, the version ----------
        let nav = Rect::from_min_max(full.min, pos2(full.left() + NAV_W, full.bottom()));
        let nav_fill = if dark { style::over(p.overlay, p.chrome, 0.45) } else { Color32::from_rgb(0xf6, 0xf6, 0xf5) };
        ui.painter().rect_filled(nav, CornerRadius { nw: style::RADIUS_DIALOG, ne: 0, sw: style::RADIUS_DIALOG, se: 0 }, nav_fill);
        ui.painter().vline(nav.right() - 0.5, nav.y_range(), Stroke::new(1.0, p.border));
        let mut job = egui::text::LayoutJob::default();
        let mut f = egui::TextFormat::simple(style::semibold(15.0), p.text);
        f.extra_letter_spacing = -0.15;
        job.append(t("Settings"), 0.0, f);
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(nav.left() + 18.0, nav.top() + 18.0), g, p.text);
        let search_rect = Rect::from_min_size(pos2(nav.left() + 10.0, nav.top() + 18.0 + 22.0 + 10.0), vec2(NAV_W - 20.0, 30.0));
        let mut search = self.settings_search.clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(search_rect), |ui| {
            kit::field(ui, egui::Id::new("settings-search"), &mut search, Some(icons::MAGNIFYING_GLASS), t("Search settings"), NAV_W - 20.0, false)
        });
        self.settings_search = search.clone();
        // ⌘F, at the right of the empty field.
        let sid = egui::Id::new("settings-search");
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            ui.memory_mut(|m| m.request_focus(sid));
        }
        if search.is_empty() && !ui.memory(|m| m.has_focus(sid)) {
            let keys = kit::keys(&Action::Find).unwrap_or("");
            if !keys.is_empty() {
                let w = kit::kbd_width(ui.painter(), keys);
                kit::paint_kbd_text(ui.painter(), pos2(search_rect.right() - 8.0 - w, search_rect.center().y), keys, &p);
            }
        }
        let mut y = search_rect.bottom() + 14.0;
        for (i, (pg, icon, name, rows)) in pages().into_iter().enumerate() {
            if i == 7 {
                ui.painter().hline((nav.left() + 18.0)..=(nav.right() - 18.0), y + 7.0, Stroke::new(1.0, p.border));
                y += 15.0;
            }
            let label = t(name);
            let matches = !search.trim().is_empty() && rows.iter().any(|r| hit(&search, t(r), ""));
            let item = Rect::from_min_size(pos2(nav.left() + 10.0, y), vec2(NAV_W - 20.0, 31.0));
            y += 32.0;
            let r = ui.interact(item, egui::Id::new(("settings-page", i)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("settings:{name}"), item);
            let on = *page == pg;
            if on {
                ui.painter().rect(item, CornerRadius::same(7), p.surface, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
                kit::paint_highlight(ui.painter(), item, 7, &p);
            } else if r.hovered() {
                ui.painter().rect_filled(item, CornerRadius::same(7), p.hover);
            }
            let (ic, tc) = if on || r.hovered() { (p.text, p.text) } else { (p.text_faint, p.text_muted) };
            ui.painter().text(pos2(item.left() + 10.0 + 8.0, item.center().y), Align2::CENTER_CENTER, icon, style::icon_font(15.0), ic);
            ui.painter().text(pos2(item.left() + 10.0 + 16.0 + 10.0, item.center().y), Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), tc);
            // At the right: what a page has for the user (an update, agents without hooks).
            if pg == SettingsPage::Updates && self.updater.available_version().is_some() {
                ui.painter().circle_filled(pos2(item.right() - 10.0 - 3.0, item.center().y), 3.0, p.accent_text);
            }
            if pg == SettingsPage::Agents {
                let n = self.agents_without_hooks();
                if n > 0 {
                    kit::paint_badge(ui.painter(), pos2(item.right() - 8.0, item.center().y), &n.to_string(), style::soft(p.warn, 0.15), p.warn, 16.0);
                }
            }
            if matches {
                ui.painter().circle_filled(pos2(item.right() - 12.0, item.center().y), 3.0, p.accent);
            }
            if r.clicked() {
                *page = pg;
            }
        }
        // The version at the bottom.
        let ver = Rect::from_min_max(pos2(nav.left() + 10.0, nav.bottom() - 12.0 - 38.0), pos2(nav.right() - 10.0, nav.bottom() - 12.0));
        ui.painter().hline(ver.x_range(), ver.top() + 0.5, Stroke::new(1.0, p.border));
        let logo = Rect::from_min_size(pos2(ver.left() + 8.0, ver.top() + 10.0), vec2(26.0, 26.0));
        paint_logo(ui.painter(), logo, &p);
        ui.painter().text(pos2(logo.right() + 10.0, logo.top() + 1.0), Align2::LEFT_TOP, "TernaryCode", style::medium(12.0), p.text_muted);
        let version = match self.updater.available_version() {
            Some(v) => format!("{} · {}", env!("CARGO_PKG_VERSION"), tf("{} is out", &[&v])),
            None => env!("CARGO_PKG_VERSION").to_string(),
        };
        ui.painter().text(pos2(logo.right() + 10.0, logo.bottom() + 1.0), Align2::LEFT_BOTTOM, version, style::mono(10.5), p.text_faint);

        // ----- the page's head: its title and line, "Saved as you change them", × -------------
        let main_full = Rect::from_min_max(pos2(nav.right(), full.top()), full.max);
        let head = Rect::from_min_size(main_full.min, vec2(main_full.width(), 24.0 + 26.0 + 5.0 + 18.0 + 6.0));
        let close = Rect::from_center_size(pos2(head.right() - 18.0 - 11.0, head.top() + 24.0 + 12.0), vec2(22.0, 22.0));
        if ui.scope_builder(egui::UiBuilder::new().max_rect(close), |ui| kit::icon_button_ex(ui, icons::X, t("Close"), false, 22.0)).inner.clicked() {
            keep = false;
        }
        let saved = ui.painter().layout_no_wrap(t("Saved as you change them").to_string(), egui::FontId::proportional(11.5), p.text_faint);
        let sx = close.left() - 12.0 - saved.size().x;
        ui.painter().galley(pos2(sx, close.center().y - saved.size().y / 2.0), saved, p.text_faint);
        ui.painter().text(pos2(sx - 5.0, close.center().y), Align2::RIGHT_CENTER, icons::CHECK, style::icon_font(12.0), p.ok);
        // ----- the page -------------------------------------------------------------------------
        let main = Rect::from_min_max(pos2(nav.right(), head.bottom()), full.max);
        #[cfg(debug_assertions)]
        crate::devtest::record("settings-body", main);
        let before = self.settings.clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(main), |ui| {
            egui::ScrollArea::vertical().id_salt("settings-page").auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 34, right: 34, top: 4, bottom: 30 }).show(ui, |ui| {
                    ui.set_width(main.width() - 68.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    match *page {
                        SettingsPage::General => self.general_page(ui, &search),
                        SettingsPage::Appearance => self.appearance_page(ui, &search),
                        SettingsPage::Terminal => self.terminal_page(ui, &search),
                        SettingsPage::Browser => self.browser_page(ui, &search),
                        SettingsPage::Agents | SettingsPage::Integrations => self.agents_page(ui, &search),
                        SettingsPage::Editor => self.editor_page(ui, &search),
                        SettingsPage::Notifications => self.notifications_page(ui, &search),
                        SettingsPage::Updates => self.updates_page(ui, &search),
                        SettingsPage::About => self.about_page(ui),
                    }
                });
            });
        });
        // The head's title and line, given by the page just drawn.
        if let Some((title, lead)) = ui.ctx().data(|d| d.get_temp::<(String, String)>(egui::Id::new("settings-head"))) {
            let mut job = egui::text::LayoutJob::default();
            let mut f = egui::TextFormat::simple(style::semibold(21.0), p.text);
            f.extra_letter_spacing = -0.42;
            job.append(&title, 0.0, f);
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(head.left() + 34.0, head.top() + 24.0), g, p.text);
            let mut job = egui::text::LayoutJob::simple_singleline(lead, egui::FontId::proportional(13.0), p.text_muted);
            job.wrap = egui::text::TextWrapping::truncate_at_width(sx - 20.0 - (head.left() + 34.0));
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(head.left() + 34.0, head.top() + 24.0 + 26.0 + 5.0), g, p.text_muted);
        }

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
        [AgentKind::OpenCode]
            .into_iter()
            .filter(|a| self.installed(*a).is_none() && self.settings.integration(*a) != Integration::Off)
            .count()
    }

    fn general_page(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        page_head(ui, t("General"), t("The app itself."));
        let labels = [(t("Command line"), ""), (t("Data folder"), ""), (t("Version"), ""), (t("Check for updates automatically"), ""), (t("Group active projects at the top"), "")];
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
            c.row(t("Group active projects at the top"), t("In the sidebar, projects where something runs or waits come first, under Active."), |ui| {
                kit::switch(ui, &mut self.settings.group_active);
            });
        });

        // Worktrees: where Run Agent → New worktree puts them, what their branches are called.
        let beside = tf("Beside the repository: {}", &["‹repo›.worktrees/‹name›"]);
        let prefix = self.settings.worktrees.prefix.clone();
        let examples = format!("{prefix}generics, {prefix}lexer-fix…");
        let labels = [
            (t("Where new worktrees go"), beside.as_str()),
            (t("Branch names start with"), examples.as_str()),
            (t("When a branch is merged"), t("Offer to remove its worktree.")),
        ];
        if labels.iter().any(|(l, n)| hit(search, l, n)) {
            group(ui, t("Worktrees"));
        }
        let place_note = match self.settings.worktrees.place {
            tc_core::model::WorktreePlace::Beside => beside.clone(),
            tc_core::model::WorktreePlace::DataFolder => tf("In TernaryCode's data folder: {}", &[&crate::paths::contract_tilde(&crate::paths::data_dir().join("worktrees"))]),
        };
        card(ui, search, &labels, |c| {
            c.row(t("Where new worktrees go"), &place_note, |ui| {
                let options = [t("Beside the repository"), t("In TernaryCode's data folder")];
                let current = match self.settings.worktrees.place {
                    tc_core::model::WorktreePlace::Beside => 0,
                    tc_core::model::WorktreePlace::DataFolder => 1,
                };
                if let Some(i) = kit::select(ui, "worktree-place", options[current], &options, 200.0) {
                    self.settings.worktrees.place = if i == 0 { tc_core::model::WorktreePlace::Beside } else { tc_core::model::WorktreePlace::DataFolder };
                }
            });
            c.row(t("Branch names start with"), &examples, |ui| {
                let mut text = self.settings.worktrees.prefix.clone();
                let r = kit::field(ui, egui::Id::new("worktree-prefix"), &mut text, None, "tc/", 90.0, true);
                // Only what a branch name can hold (empty: no prefix).
                if r.changed() && (text.is_empty() || tc_git::worktree::valid_branch(&format!("{text}x"))) {
                    self.settings.worktrees.prefix = text;
                }
            });
            c.row(t("When a branch is merged"), t("Offer to remove its worktree."), |ui| {
                kit::switch(ui, &mut self.settings.worktrees.offer_remove);
            });
        });
    }

    fn appearance_page(&mut self, ui: &mut Ui, search: &str) {
        page_head(ui, t("Appearance"), t("How TernaryCode and its terminals look."));
        let labels = [(t("Theme"), ""), (t("Language"), t("System follows the language of your computer."))];
        if labels.iter().any(|(l, n)| hit(search, l, n)) {
            group(ui, t("Theme"));
        }
        card(ui, search, &labels, |c| {
            c.bare(t("Theme"), |ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let w = ((ui.available_width() - 28.0) / 3.0).floor();
                ui.horizontal(|ui| {
                    for (mode, name) in [(ThemeMode::System, t("System")), (ThemeMode::Light, t("Light")), (ThemeMode::Dark, t("Dark"))] {
                        if theme_card(ui, mode, name, self.settings.theme == mode, w).clicked() {
                            self.settings.theme = mode;
                        }
                    }
                });
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
                if let Some(i) = kit::select(ui, "language", &options[current], &opts, 200.0) {
                    self.settings.language = [Language::System, Language::English, Language::Indonesian][i];
                }
            });
        });
        let labels = [(t("With the dark theme"), t("Also used in Grid tiles and previews.")), (t("With the light theme"), "")];
        if labels.iter().any(|(l, n)| hit(search, l, n)) {
            group(ui, t("Terminal colors"));
        }
        card(ui, search, &labels, |c| {
            for dark in [true, false] {
                let (label, note) = if dark { (t("With the dark theme"), t("Also used in Grid tiles and previews.")) } else { (t("With the light theme"), "") };
                c.bare_hit(label, note, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let p = style::pal(ui);
                        ui.vertical(|ui| {
                            ui.set_width(190.0);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(RichText::new(label).size(13.0).color(p.text));
                            if !note.is_empty() {
                                ui.add(egui::Label::new(RichText::new(note).size(12.0).color(p.text_faint).line_height(Some(17.4))).wrap());
                            }
                        });
                        // Four schemes (the chosen one among them), the rest behind "N more".
                        let current = self.scheme(dark).name;
                        let schemes: Vec<&'static tc_term::scheme::Scheme> = tc_term::scheme::SCHEMES.iter().filter(|s| s.dark == dark).collect();
                        let open_id = egui::Id::new(("schemes-open", dark));
                        let open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
                        let mut shown: Vec<&'static tc_term::scheme::Scheme> = if open { schemes.clone() } else { schemes.iter().take(4).copied().collect() };
                        if !open && !shown.iter().any(|s| s.name == current)
                            && let Some(s) = schemes.iter().find(|s| s.name == current)
                        {
                            shown.pop();
                            shown.push(s);
                        }
                        let more = schemes.len().saturating_sub(4);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(6.0, 10.0);
                            for s in &shown {
                                if scheme_tile(ui, s, s.name == current).clicked() {
                                    if dark {
                                        self.settings.scheme_dark = s.name.to_string();
                                    } else {
                                        self.settings.scheme_light = s.name.to_string();
                                    }
                                }
                            }
                            if more > 0 {
                                let label = if open { t("Fewer").to_string() } else { tf("{} more", &[&more.to_string()]) };
                                if more_tile(ui, &label).clicked() {
                                    ui.data_mut(|d| d.insert_temp(open_id, !open));
                                }
                            }
                        });
                    });
                });
            }
        });
        let labels = [(t("Font"), t("Monospace fonts installed on this computer.")), (t("Size"), ""), (t("Cursor"), t("New panes take it.")), (t("Blinking cursor"), "")];
        if labels.iter().any(|(l, n)| hit(search, l, n)) {
            group(ui, t("Terminal text"));
        }
        card(ui, search, &labels, |c| {
            // The settings at the left, a live preview at the right (in the terminal's font and
            // colours).
            c.bare_hit_flush(&labels, |ui| {
                let full = ui.available_width();
                let preview_w = if full > 640.0 { 294.0 } else { 0.0 };
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let left = ui
                        .vertical(|ui| {
                            ui.set_width(full - preview_w - if preview_w > 0.0 { 8.0 } else { 0.0 });
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let mut c = Card { ui, search: search.to_string(), first: true, rows: 0, icon: None };
                            c.row(t("Font"), "", |ui| {
                                let fonts: Vec<&'static str> = tc_term::fonts::PRESETS.iter().filter(|f| tc_term::fonts::is_available(f)).map(|f| f.name).collect();
                                let current = tc_term::fonts::effective_choice(&tc_term::prefs::FontChoice { name: self.settings.font_name.clone(), path: self.settings.font_path.clone(), index: 0 }).name;
                                if let Some(i) = kit::select(ui, "font", &current, &fonts, 200.0) {
                                    self.settings.font_name = fonts[i].to_string();
                                    self.settings.font_path = None;
                                }
                            });
                            c.row(t("Size"), "", |ui| {
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
                        })
                        .response
                        .rect;
                    if preview_w > 0.0 {
                        let r = Rect::from_min_size(pos2(left.right() + 0.0, left.top() + 8.0), vec2(preview_w, (left.height() - 16.0).max(150.0)));
                        ui.allocate_rect(Rect::from_min_size(left.right_top(), vec2(preview_w + 8.0, left.height())), Sense::hover());
                        let dark = ui.visuals().dark_mode;
                        terminal_preview(ui, r, self.scheme(dark), self.settings.font_size, self.settings.cursor_shape);
                    }
                });
            });
        });
    }

    fn browser_page(&mut self, ui: &mut Ui, search: &str) {
        page_head(ui, t("Browser"), t("Web pages beside your terminals, for you and your agents."));
        let labels = [
            (t("Open web pages in TernaryCode"), ""),
            (t("When an agent opens a page"), ""),
            (t("Put browsers to sleep after"), ""),
            (t("Pages in Grid tiles"), ""),
            (t("Logins & cookies"), ""),
        ];
        card(ui, search, &labels, |c| {
            c.row(t("Open web pages in TernaryCode"), t("Server chips, ⌘-clicked links, HTML files and programs that open a browser. Hold ⌥ for your default browser."), |ui| {
                kit::switch(ui, &mut self.settings.browser_here);
            });
            c.row(t("When an agent opens a page"), t("Its page goes in the project's Browser panel, at the right of the terminals. The keyboard never moves."), |ui| {
                let options = [t("Show the panel"), t("Light the Browser button")];
                let current = options[usize::from(!self.settings.browser_agent_beside)];
                if let Some(i) = kit::select(ui, "browser-agent-place", current, &options, 150.0) {
                    self.settings.browser_agent_beside = i == 0;
                }
            });
            c.row(t("Put browsers to sleep after"), t("Out of view and unused. Never while it plays sound, uses the camera or microphone, has a form being filled in, or an agent uses it."), |ui| {
                const STEPS: [u32; 7] = [1, 5, 10, 15, 30, 60, 120];
                let n = self.settings.browser_sleep_minutes;
                let at = STEPS.iter().position(|s| *s >= n).unwrap_or(STEPS.len() - 1);
                match kit::stepper(ui, &tf("{} min", &[&n.to_string()])) {
                    -1 => self.settings.browser_sleep_minutes = STEPS[at.saturating_sub(1)],
                    1 => self.settings.browser_sleep_minutes = STEPS[(at + 1).min(STEPS.len() - 1)],
                    _ => {}
                }
            });
            c.row(t("Pages in Grid tiles"), t("Smaller, a page keeps the layout it has in Focus. Fitting the tile, a site may switch to its phone layout."), |ui| {
                let options = [t("Keep their layout, smaller"), t("Fit the tile")];
                let current = options[usize::from(!self.settings.browser_grid_zoom)];
                if let Some(i) = kit::select(ui, "browser-grid-zoom", current, &options, 190.0) {
                    self.settings.browser_grid_zoom = i == 0;
                }
            });
            c.row(t("Logins & cookies"), t("One profile for every project: log in once and stay logged in, through restarts and updates."), |ui| {
                if kit::button_ex(ui, kit::Kind::Secondary, None, t("Clear…"), true).clicked() {
                    self.dialog = Some(crate::app::Dialog::ConfirmClearBrowsing);
                }
            });
        });

        // Agents: whether they get the tools, whether they ask before a site (and which sites
        // they open without asking then), scripts.
        let labels = [
            (t("Agents can use the browser"), t("Only agents in TernaryCode panes, each in its own project’s browser. Agents in other terminals don’t see the tools.")),
            (t("Sites agents may open without asking"), t("Every project. A project’s own sites are added from the question (“Always for …”).")),
            (t("Agents may run JavaScript in pages"), ""),
            (t("Ask before agents open a site"), t("Off: agents open any site at once. On: a site not listed below waits for Allow Once, Always or Block.")),
        ];
        if labels.iter().any(|(l, n)| hit(search, l, n)) {
            group(ui, t("Agents"));
        }
        let mut tools_changed = false;
        let mut remove_site = None;
        let mut add_site = None;
        let mut install = None;
        let statuses: Vec<(AgentKind, bool, String)> = [AgentKind::Claude, AgentKind::Codex, AgentKind::OpenCode, AgentKind::Antigravity].into_iter().map(|a| self.browser_tools_status(a)).collect();
        card(ui, search, &labels, |c| {
            c.row(labels[0].0, labels[0].1, |ui| {
                tools_changed = kit::switch(ui, &mut self.settings.browser_agents).changed();
            });
            c.row(labels[3].0, labels[3].1, |ui| {
                kit::switch(ui, &mut self.settings.browser_agent_ask);
            });
            if self.settings.browser_agent_ask {
                c.row_with(labels[1].0, labels[1].1, true, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        for site in &self.settings.browser_agent_sites {
                            if site_chip(ui, site) {
                                remove_site = Some(site.clone());
                            }
                        }
                        let id = egui::Id::new("browser-add-site");
                        let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_default();
                        let r = kit::field(ui, id.with("field"), &mut text, Some(icons::PLUS), t("Add Site"), 150.0, false);
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if enter && !text.trim().is_empty() {
                            add_site = Some(text.trim().to_string());
                            text.clear();
                        }
                        ui.data_mut(|d| d.insert_temp(id, text));
                    });
                });
            }
            c.row(labels[2].0, t("browser_evaluate: anything a page’s own scripts can do, cookies included."), |ui| {
                kit::switch(ui, &mut self.settings.browser_agent_js);
            });
            for (agent, ok, status) in &statuses {
                c.row_icon(icons::PLUGS_CONNECTED, if *ok { style::pal(c.ui).ok } else { style::pal(c.ui).text_faint }, agent.label(), status, |ui| {
                    if !ok && !matches!(*agent, AgentKind::Claude | AgentKind::Antigravity) && kit::button_ex(ui, kit::Kind::Secondary, None, t("Install…"), true).clicked() {
                        install = Some(*agent);
                    }
                });
            }
        });
        if tools_changed {
            // The next agent run gets (or no longer gets) the tools.
            let codex_config = std::fs::read_to_string(crate::paths::home_dir().join(".codex/config.toml")).ok();
            let _ = self.hook_bin.pane_env(&crate::paths::integrations_dir(), self.settings.browser_agents, codex_config.as_deref());
        }
        if let Some(site) = remove_site {
            self.settings.browser_agent_sites.retain(|s| *s != site);
        }
        if let Some(site) = add_site {
            let site = crate::browser::host_of(&site).trim_start_matches("*.").to_string();
            let site = if site.is_empty() { String::new() } else { site };
            if !site.is_empty() && !self.settings.browser_agent_sites.contains(&site) {
                self.settings.browser_agent_sites.push(site);
            }
        }
        if let Some(agent) = install {
            self.actions.push(Action::PlanInstall(agent));
        }

        // The sites one project's agents may open (added from the question in its browser).
        let sites: Vec<(tc_core::ProjectId, String, String)> =
            self.projects.iter().flat_map(|p| p.sites.iter().map(|s| (p.id, p.name.clone(), s.clone()))).collect();
        if sites.is_empty() || !self.settings.browser_agent_ask {
            return;
        }
        let mut labels: Vec<(&str, &str)> = vec![(t("Sites added for a project"), "")];
        for (_, project, site) in &sites {
            labels.push((site.as_str(), project.as_str()));
        }
        if labels.iter().any(|(l, n)| hit(search, l, n)) {
            group(ui, t("Sites added for a project"));
        }
        let mut remove = None;
        card(ui, search, &labels, |c| {
            for (id, project, site) in &sites {
                c.row(site, project, |ui| {
                    if kit::button_ex(ui, kit::Kind::Ghost, None, t("Remove"), true).clicked() {
                        remove = Some((*id, site.clone()));
                    }
                });
            }
        });
        if let Some((id, site)) = remove
            && let Some(p) = self.projects.iter_mut().find(|p| p.id == id)
        {
            p.sites.retain(|s| *s != site);
            self.mark_dirty();
        }
    }

    /// Whether `agent` gets the browser tools, and how that reads in Settings → Browser.
    fn browser_tools_status(&self, agent: AgentKind) -> (AgentKind, bool, String) {
        let mode = self.settings.integration(agent);
        let installed = self.installed(agent);
        let has_server = |rec: &Option<InstallRecord>| {
            rec.as_ref().and_then(|r| std::fs::read_to_string(&r.path).ok()).is_some_and(|text| {
                text.contains("ternarycode-hook")
                    && (text.contains("\"mcpServers\"")
                        // Codex asks before each call of a server not approved.
                        || (text.contains("mcp_servers.ternarycode") && text.contains("default_tools_approval_mode")))
            })
        };
        let (ok, text) = match (agent, mode) {
            (AgentKind::Claude | AgentKind::Codex, Integration::PerLaunch) => (true, t("Added to each run it starts from a preset")),
            (AgentKind::Claude, _) => (false, t("Only with Per launch (Settings → Agents)")),
            // In every pane (`OPENCODE_CONFIG_CONTENT`), whatever its hooks.
            (AgentKind::OpenCode, _) => (true, t("In every TernaryCode pane, typed by hand too")),
            // Its MCP servers are in its own files only; it has a browser of its own.
            (AgentKind::Antigravity, _) => (false, t("Not added for Antigravity CLI yet")),
            (_, Integration::Installed) if has_server(&installed) => (true, t("Browser tools installed")),
            (_, Integration::Installed) => (false, t("Installed before the browser tools: install again")),
            _ => (false, t("Hooks not installed")),
        };
        (agent, ok, text.to_string())
    }

    fn terminal_page(&mut self, ui: &mut Ui, search: &str) {
        page_head(ui, t("Terminal"), t("How panes behave."));
        let mut labels = vec![
            (t("Scrollback per pane"), t("Applies to new panes.")),
            (t("Scrollback memory for all panes"), ""),
            (t("Copy on select"), ""),
            (t("Programs may copy to the clipboard"), ""),
            (t("Shell integration"), ""),
        ];
        if cfg!(target_os = "macos") {
            labels.push((t("Option key as Meta"), ""));
        }
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
        let mut labels = vec![
            (t("An agent needs input"), ""),
            (t("A process failed"), ""),
            (t("An agent finished its turn"), ""),
            (t("A long command finished"), ""),
        ];
        if cfg!(target_os = "macos") {
            labels.push((t("Badge on the Dock icon"), ""));
        }
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
        page_head(ui, t("About"), "");
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
            let dark = ui.visuals().dark_mode;
            egui::Frame::NONE
                .fill(p.canvas)
                .stroke(Stroke::new(1.0, p.border_strong))
                .corner_radius(CornerRadius::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    // Header: mark, name, how it reports, the command; at the right how many
                    // panes run it (or Install / Uninstall), and the chevron. A click opens it.
                    let (head, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 57.0), Sense::click());
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("agent-card:{}", preset.id), head);
                    let cy = head.center().y;
                    let tile = Rect::from_center_size(pos2(head.left() + 13.0 + 17.0, cy), vec2(34.0, 34.0));
                    ui.painter().rect(tile, CornerRadius::same(9), if dark { p.inset } else { Color32::WHITE }, Stroke::new(1.0, p.border), StrokeKind::Inside);
                    let logo = Rect::from_center_size(tile.center(), vec2(18.0, 18.0));
                    if !preset.agent.is_some_and(|a| super::agent_logo::paint(ui.painter(), logo, a, p.text, false)) {
                        ui.painter().text(tile.center(), Align2::CENTER_CENTER, agent_mark(preset), style::mono_semibold(11.0), p.text_muted);
                    }
                    let x = tile.right() + 12.0;
                    let g = ui.painter().layout_no_wrap(preset.name.clone(), style::medium(13.5), p.text);
                    let nw = g.size().x;
                    ui.painter().galley(pos2(x, cy - 9.0 - g.size().y / 2.0), g, p.text);
                    let (icon, text, fg, bg) = hooks.pill(&p, dark);
                    kit::paint_pill(ui.painter(), pos2(x + nw + 8.0, cy - 9.0), icon, &text, fg, bg);
                    let sub = match &preset.resume {
                        Some(r) if !r.is_empty() => format!("{}  ·  {} {r}", preset.command, t("resume:")),
                        _ => format!("{}  ·  {} —", preset.command, t("resume:")),
                    };
                    ui.painter().text(pos2(x, cy + 10.0), Align2::LEFT_CENTER, sub, style::mono(11.0), p.text_faint);
                    let chevron_x = head.right() - 14.0 - 8.0;
                    ui.painter().text(pos2(chevron_x, cy), Align2::CENTER_CENTER, if expanded { icons::CARET_UP } else { icons::CARET_DOWN }, style::icon_font(12.0), p.text_faint);
                    let right = chevron_x - 8.0 - 14.0;
                    if !hooks.hooked && hooks.can_install && self.installed(agent).is_none() {
                        let w = kit::button_width(ui, None, t("Install…"), None, true);
                        let br = Rect::from_min_size(pos2(right - w, cy - 12.0), vec2(w, 24.0));
                        if ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| kit::button_ex(ui, kit::Kind::Primary, None, t("Install…"), true)).inner.clicked() {
                            self.actions.push(Action::PlanInstall(agent));
                        }
                    } else if hooks.installed && !matches!(agent, AgentKind::Claude | AgentKind::Codex) && !expanded {
                        // An agent whose hooks are only ever installed: Uninstall right here.
                        let w = kit::button_width(ui, None, t("Uninstall"), None, true);
                        let br = Rect::from_min_size(pos2(right - w, cy - 12.0), vec2(w, 24.0));
                        if ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| kit::button_ex(ui, kit::Kind::Ghost, None, t("Uninstall"), true)).inner.clicked() {
                            self.actions.push(Action::Uninstall(agent));
                        }
                    } else if hooks.active > 0 {
                        let text = crate::i18n::count(hooks.active, t("Active in 1 pane"), t("Active in {} panes"));
                        ui.painter().text(pos2(right, cy), Align2::RIGHT_CENTER, text, egui::FontId::proportional(12.0), p.text_faint);
                    }
                    if resp.clicked() {
                        open = if expanded { None } else { Some(preset.id.clone()) };
                    }
                    if !expanded {
                        return;
                    }
                    // Opened: the commands, the environment, the hooks and what they report, the
                    // browser tools.
                    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, p.border));
                    egui::Frame::NONE.inner_margin(egui::Margin { left: 59, right: 15, top: 12, bottom: 14 }).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        const LABEL_W: f32 = 122.0;
                        let field_row = |ui: &mut Ui, label: &str, add: &mut dyn FnMut(&mut Ui)| {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, 30.0), Sense::hover());
                                ui.painter().text(r.left_center(), Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), p.text_muted);
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 8.0;
                                    add(ui);
                                });
                            });
                        };
                        if !preset.builtin {
                            field_row(ui, t("Name"), &mut |ui| {
                                let id = egui::Id::new(("preset-name", i));
                                changed |= well_field(ui, id, &mut preset.name, "", false).changed();
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
                        field_row(ui, t("Environment"), &mut |ui| {
                            // KEY=value, one per line (kept as typed while the field has the
                            // keyboard; read into the preset as it changes).
                            let id = egui::Id::new(("preset-env", i));
                            let typed_id = id.with("typed");
                            let focused = ui.memory(|m| m.has_focus(id));
                            let mut text: String = match ui.data(|d| d.get_temp::<String>(typed_id)).filter(|_| focused) {
                                Some(t) => t,
                                None => preset.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n"),
                            };
                            if well_field(ui, id, &mut text, t("KEY=value, one per line"), true).changed() {
                                preset.env = text
                                    .lines()
                                    .filter_map(|kv| kv.trim().split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                                    .filter(|(k, _)| !k.is_empty())
                                    .collect();
                                changed = true;
                            }
                            ui.data_mut(|d| d.insert_temp(typed_id, text));
                        });
                        if agent == AgentKind::Antigravity {
                            field_row(ui, t("Status signals"), &mut |ui| {
                                ui.add_space(6.0);
                                ui.add(egui::Label::new(RichText::new(hooks_note(agent)).size(11.5).color(p.text_faint).line_height(Some(17.0))).wrap());
                            });
                        } else if agent != AgentKind::Custom {
                            field_row(ui, t("Status signals"), &mut |ui| {
                                ui.horizontal(|ui| {
                                    self.hooks_controls(ui, agent);
                                });
                                ui.add(egui::Label::new(RichText::new(hooks_note(agent)).size(11.5).color(p.text_faint).line_height(Some(17.0))).wrap());
                                // What the hooks report, and the version they were checked with.
                                health_strip(ui, &hooks, agent, &p);
                                if hooks.unconnected > 0 {
                                    let text = crate::i18n::count(
                                        hooks.unconnected,
                                        t("Typed by hand in 1 pane, it is not connected: Install connects it (then restart it there)."),
                                        t("Typed by hand in {} panes, it is not connected: Install connects it (then restart it there)."),
                                    );
                                    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(p.warn).line_height(Some(17.0))).wrap());
                                }
                            });
                            // The browser tools: added to each run started with Per launch.
                            if matches!(agent, AgentKind::Claude | AgentKind::Codex) {
                                let per_launch = self.settings.integration(agent) == Integration::PerLaunch;
                                field_row(ui, t("Browser tools"), &mut |ui| {
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 8.0;
                                        let (fg, bg, label) = if per_launch { (p.accent_text, p.accent_tint, t("Added to each run")) } else { (p.text_muted, p.subtle, t("Not added")) };
                                        kit::pill(ui, Some(icons::GLOBE_SIMPLE), label, fg, bg);
                                        let n = tc_agent::mcp::tools().as_array().map_or(0, |a| a.len());
                                        let note = if per_launch { tf("{} tools, from presets started with Per launch", &[&n.to_string()]) } else { t("Browser tools come with Per launch.").to_string() };
                                        ui.label(RichText::new(note).size(11.5).color(p.text_faint));
                                    });
                                });
                            }
                        } else {
                            field_row(ui, "", &mut |ui| {
                                ui.add(egui::Label::new(RichText::new(t("A custom agent reports through ternarycode-hook with start, working, needs-input, done and end; without it, its status comes from the terminal.")).size(11.5).color(p.text_faint).line_height(Some(17.0))).wrap());
                                if kit::button_ex(ui, kit::Kind::Danger, Some(icons::TRASH), t("Delete Preset"), true).clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                    });
                });
            ui.add_space(8.0);
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
                ui.spacing_mut().item_spacing.x = 8.0;
                let exists = self.hook_bin.bin.exists();
                let path = format!("{}{}", crate::paths::contract_tilde(&self.hook_bin.bin), if exists { "" } else { t(" (missing)") });
                ui.add(egui::Label::new(RichText::new(path).font(style::mono(10.5)).color(if exists { p.text_faint } else { p.err })).truncate());
                ui.label(RichText::new(t("Hook program")).size(12.0).color(p.text_muted));
                ui.label(RichText::new(if exists { icons::CHECK_CIRCLE } else { icons::WARNING }).font(style::icon_fill(12.0)).color(if exists { p.ok } else { p.err }));
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
        });
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
        if self.agent == AgentKind::Antigravity {
            return (Some(icons::CIRCLE_HALF), t("No hooks yet · status shows Active").to_string(), p.text_muted, p.subtle);
        }
        if self.hooked && !self.installed && self.unconnected > 0 {
            return (Some(icons::WARNING), t("Hooks · per launch only").to_string(), p.warn, style::tint(p.warn, dark));
        }
        if self.hooked && self.installed && self.agent == AgentKind::OpenCode {
            return (Some(icons::CHECK_CIRCLE), t("Plugin installed").to_string(), p.ok, style::tint(p.ok, dark));
        }
        if self.hooked {
            let how = if self.installed { t("installed") } else { t("per launch") };
            return (Some(icons::CHECK_CIRCLE), tf("Hooks · {}", &[how]), p.ok, style::tint(p.ok, dark));
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
        AgentKind::Claude => t("Per launch adds the hooks (--settings) and browser tools (--mcp-config) to each run, without touching your files. Installed writes the hooks into ~/.claude/settings.json, with a backup."),
        AgentKind::Codex => t("Per launch adds the hooks, notify and browser tools to each run. Codex asks you once to trust TernaryCode's hooks (its own security check)."),
        AgentKind::Gemini => t("Install adds hooks to ~/.gemini/settings.json (tool permissions, finished turns). Gemini CLI has no per-launch option."),
        AgentKind::Antigravity => t("Antigravity CLI replaced Gemini CLI. TernaryCode does not add hooks to it yet: while it runs, its status reads “Active”."),
        _ => t("Install adds a small plugin file to OpenCode's plugins folder (permission requests, finished turns). OpenCode has no per-launch option."),
    }
}

/// Two letters for the tile of an agent with no logo.
fn agent_mark(preset: &Preset) -> String {
    match preset.agent {
        Some(a) if a != AgentKind::Custom => a.mark().into(),
        _ => preset.name.chars().filter(|c| c.is_alphanumeric()).take(2).collect::<String>(),
    }
}

/// A monospace text field the width left in the row.
fn mono_field(ui: &mut Ui, id: egui::Id, text: &mut String) -> egui::Response {
    well_field(ui, id, text, "", false)
}

/// A field in a well (30 high, the data font): one line, or `multi`: as many lines as it has.
fn well_field(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, multi: bool) -> egui::Response {
    let p = style::pal(ui);
    let dark = ui.visuals().dark_mode;
    let w = ui.available_width();
    let lines = if multi { text.lines().count().max(1) + usize::from(text.ends_with('\n')) } else { 1 };
    let h = 30.0 + (lines.saturating_sub(1)) as f32 * 17.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    if focused {
        ui.painter().rect_stroke(rect.expand(3.0), CornerRadius::same(10), Stroke::new(3.0, p.accent_tint), StrokeKind::Inside);
    }
    let fill = if dark { p.inset } else { Color32::WHITE };
    ui.painter().rect(rect, CornerRadius::same(7), fill, Stroke::new(1.0, if focused { p.accent } else { p.border }), StrokeKind::Inside);
    let inner = Rect::from_min_max(pos2(rect.left() + 10.0, rect.top() + 6.5), pos2(rect.right() - 10.0, rect.bottom() - 6.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Min)));
    let edit = if multi { egui::TextEdit::multiline(text).desired_rows(1) } else { egui::TextEdit::singleline(text) };
    child.add_sized(
        inner.size(),
        edit.id(id)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(style::mono(12.0))
            .hint_text(RichText::new(hint).font(style::mono(12.0)).color(p.text_ghost)),
    )
}

/// Under an agent's status signals: what its hooks report (green when they are in use), and
/// the version they were checked with.
fn health_strip(ui: &mut Ui, hooks: &Hooks, agent: AgentKind, p: &style::Palette) {
    let (icon, color, text) = if hooks.hooked {
        (icons::CHECK_CIRCLE, p.ok, t("Reports permission requests, questions and finished turns"))
    } else if hooks.off {
        (icons::MINUS_CIRCLE, p.text_faint, t("Off: its status comes from the terminal"))
    } else {
        (icons::WARNING, p.warn, t("Not connected: its status reads “Active” only"))
    };
    let checked = match agent {
        AgentKind::Claude => Some("2.1.319"),
        AgentKind::Codex => Some("0.162.1"),
        AgentKind::OpenCode => Some("1.19.5"),
        AgentKind::Gemini => Some("0.43.0"),
        _ => None,
    };
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
    let painter = ui.painter();
    let (fill, line) = if hooks.hooked { (style::soft(p.ok, 0.07), style::soft(p.ok, 0.28)) } else { (p.hover, p.border) };
    painter.rect(rect, CornerRadius::same(7), fill, Stroke::new(1.0, line), StrokeKind::Inside);
    painter.text(pos2(rect.left() + 10.0 + 6.5, rect.center().y), Align2::CENTER_CENTER, icon, style::icon_fill(13.0), color);
    let mut right = rect.right() - 10.0;
    if let Some(v) = checked {
        let g = painter.layout_no_wrap(tf("checked with {}", &[v]), style::mono(10.5), p.text_faint);
        right -= g.size().x;
        painter.galley(pos2(right, rect.center().y - g.size().y / 2.0), g, p.text_faint);
        right -= 12.0;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), egui::FontId::proportional(12.0), if hooks.hooked { p.text } else { p.text_muted });
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - rect.left() - 32.0).max(20.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(rect.left() + 32.0, rect.center().y - g.size().y / 2.0), g, p.text);
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
fn theme_card(ui: &mut Ui, mode: ThemeMode, name: &str, on: bool, w: f32) -> egui::Response {
    let p = style::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 74.0 + 10.0 + 18.0), Sense::click());
    let card = Rect::from_min_size(rect.min, vec2(w, 74.0));
    let painter = ui.painter();
    // A tiny drawing of the window in that theme: its base, the sidebar with the queue and a row
    // picked, two sheets with lines of text.
    let mini = |painter: &egui::Painter, dark: bool| {
        let pal = style::palette(dark);
        painter.rect_filled(card, CornerRadius::same(9), pal.chrome);
        let side = Rect::from_min_size(card.min + vec2(5.0, 6.0), vec2(card.width() * 0.31, card.height() - 12.0));
        for (i, c) in [Color32::from_rgb(0xff, 0x5f, 0x57), Color32::from_rgb(0xfe, 0xbc, 0x2e), Color32::from_rgb(0x28, 0xc8, 0x40)].iter().enumerate() {
            painter.circle_filled(pos2(side.left() + 4.0 + i as f32 * 6.5, side.top() + 3.0), 2.0, *c);
        }
        let q = Rect::from_min_size(side.min + vec2(0.0, 9.0), vec2(side.width(), 11.0));
        painter.rect(q, CornerRadius::same(3), style::soft(pal.warn, 0.18), Stroke::new(1.0, style::soft(pal.warn, 0.35)), StrokeKind::Inside);
        painter.circle_filled(pos2(q.left() + 6.0, q.center().y), 2.0, pal.warn);
        painter.rect_filled(Rect::from_min_size(pos2(q.left() + 11.0, q.center().y - 1.5), vec2(q.width() * 0.5, 3.0)), CornerRadius::same(2), style::soft(pal.text_muted, 0.4));
        for (i, (cr, cg, cb)) in [(0x9d, 0x8c, 0xf0), (0x9c, 0xbf, 0x5a), (0xe0, 0x7a, 0xa0), (0xe5, 0x8a, 0x6b)].iter().enumerate() {
            let row = Rect::from_min_size(pos2(side.left(), q.bottom() + 3.0 + i as f32 * 12.0), vec2(side.width(), 11.0));
            if i == 2 {
                painter.rect(row, CornerRadius::same(3), pal.surface, Stroke::new(1.0, pal.border), StrokeKind::Inside);
            }
            painter.rect_filled(Rect::from_center_size(pos2(row.left() + 6.0, row.center().y), vec2(6.0, 6.0)), CornerRadius::same(2), Color32::from_rgb(*cr, *cg, *cb).gamma_multiply(0.8));
            painter.rect_filled(Rect::from_min_size(pos2(row.left() + 12.0, row.center().y - 1.5), vec2(row.width() * [0.7, 0.55, 0.8, 0.6][i] - 14.0, 3.0)), CornerRadius::same(2), style::soft(pal.text_muted, 0.4));
        }
        let main = Rect::from_min_max(pos2(side.right() + 4.0, side.top()), pos2(card.right() - 6.0, side.bottom()));
        painter.rect_filled(Rect::from_min_size(main.min + vec2(2.0, 3.0), vec2(main.width() * 0.22, 3.0)), CornerRadius::same(2), style::soft(pal.text_muted, 0.5));
        painter.rect_filled(Rect::from_min_size(pos2(main.right() - 22.0, main.top() + 1.5), vec2(20.0, 6.0)), CornerRadius::same(2), pal.accent);
        let sheets = Rect::from_min_max(pos2(main.left(), main.top() + 12.0), main.max);
        let w1 = sheets.width() * 0.53;
        for (k, r) in [Rect::from_min_size(sheets.min, vec2(w1, sheets.height())), Rect::from_min_max(pos2(sheets.left() + w1 + 3.0, sheets.top()), sheets.max)].into_iter().enumerate() {
            painter.rect(r, CornerRadius::same(4), pal.canvas, Stroke::new(1.0, pal.border), StrokeKind::Inside);
            for j in 0..(5 - k) {
                let lw = r.width() * [0.72, 0.5, 0.86, 0.64, 0.4][j] - 10.0;
                let c = if j == 1 { pal.info } else { style::soft(pal.text_muted, 0.45) };
                painter.rect_filled(Rect::from_min_size(pos2(r.left() + 5.0, r.top() + 6.0 + j as f32 * 6.5), vec2(lw.max(4.0), 2.5)), CornerRadius::same(1), c);
            }
        }
    };
    match mode {
        ThemeMode::Dark => mini(painter, true),
        ThemeMode::Light => mini(painter, false),
        ThemeMode::System => {
            mini(painter, false);
            let (_, right) = card.split_left_right_at_fraction(0.5);
            mini(&painter.with_clip_rect(right), true);
        }
    }
    if on {
        painter.rect_stroke(card.expand(2.0), CornerRadius::same(11), Stroke::new(2.0, p.accent), StrokeKind::Outside);
    } else {
        painter.rect_stroke(card, CornerRadius::same(9), Stroke::new(1.0, if resp.hovered() { p.border_heavy } else { p.border_strong }), StrokeKind::Inside);
    }
    // A radio and the name under it ("follows macOS" for System).
    let ry = card.bottom() + 10.0 + 9.0;
    let rc = pos2(card.left() + 8.0, ry);
    if on {
        painter.circle_filled(rc, 8.0, p.accent);
        painter.circle_filled(rc, 3.0, Color32::WHITE);
    } else {
        painter.circle_stroke(rc, 7.5, Stroke::new(1.0, p.border_heavy));
    }
    let g = painter.layout_no_wrap(name.to_string(), if on { style::medium(12.5) } else { egui::FontId::proportional(12.5) }, if on { p.text } else { p.text_muted });
    let gw = g.size().x;
    painter.galley(pos2(card.left() + 8.0 + 8.0 + 8.0, ry - g.size().y / 2.0), g, if on { p.text } else { p.text_muted });
    if mode == ThemeMode::System {
        let os = if cfg!(target_os = "macos") { t("follows macOS") } else if cfg!(windows) { t("follows Windows") } else { t("follows the system") };
        painter.text(pos2(card.left() + 24.0 + gw + 8.0, ry), Align2::LEFT_CENTER, os, egui::FontId::proportional(11.5), p.text_faint);
    }
    resp
}

/// A terminal scheme's tile: its background with a prompt and its first six colours, its name.
fn scheme_tile(ui: &mut Ui, s: &tc_term::scheme::Scheme, on: bool) -> egui::Response {
    let p = style::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(104.0, 52.0 + 6.0 + 16.0), Sense::click());
    let painter = ui.painter();
    let top = Rect::from_min_size(rect.min, vec2(104.0, 52.0));
    painter.rect_filled(top, CornerRadius::same(8), s.background);
    let stroke = if on { Stroke::new(2.0, p.accent) } else if resp.hovered() { Stroke::new(1.0, p.text_faint) } else { Stroke::new(1.0, p.border_strong) };
    painter.rect_stroke(top, CornerRadius::same(8), stroke, if on { StrokeKind::Outside } else { StrokeKind::Inside });
    let mono = style::mono(10.0);
    let mut job = egui::text::LayoutJob::default();
    job.append("❯", 0.0, egui::TextFormat::simple(mono.clone(), s.ansi[2]));
    job.append("ls", 6.0, egui::TextFormat::simple(mono.clone(), s.foreground));
    job.append("src", 6.0, egui::TextFormat::simple(mono, s.ansi[4]));
    let g = painter.layout_job(job);
    painter.galley(top.min + vec2(10.0, 9.0), g, s.foreground);
    for (i, c) in [1, 2, 3, 4, 5, 6, 7].iter().map(|k| s.ansi[*k]).enumerate() {
        let r = Rect::from_min_size(top.min + vec2(10.0 + i as f32 * 10.0, 33.0), vec2(7.0, 7.0));
        painter.rect_filled(r, CornerRadius::same(2), c);
    }
    if on {
        let c = pos2(top.right() - 11.0, top.top() + 11.0);
        painter.circle_filled(c, 7.0, p.accent);
        painter.text(c, Align2::CENTER_CENTER, icons::CHECK, style::icon_font(9.0), Color32::WHITE);
    }
    // Its name without " Dark" / " Light" (the row says which).
    let name = s.name.trim_end_matches(" Dark").trim_end_matches(" Light");
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), egui::FontId::proportional(12.0), if on { p.text } else { p.text_faint });
    job.wrap = egui::text::TextWrapping::truncate_at_width(104.0);
    let g = painter.layout_job(job);
    painter.galley(pos2(top.left() + 1.0, top.bottom() + 6.0), g, p.text);
    resp
}

/// "N more" (or "Fewer"): opens the rest of the schemes.
fn more_tile(ui: &mut Ui, label: &str) -> egui::Response {
    let p = style::pal(ui);
    // As tall as a scheme's tile with its name, the box level with the pictures.
    let (full, _) = ui.allocate_exact_size(vec2(72.0, 52.0 + 6.0 + 16.0), Sense::hover());
    let rect = Rect::from_min_size(full.min, vec2(72.0, 52.0));
    let resp = ui.interact(rect, ui.id().with(("more-schemes", label)), Sense::click());
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(8), if resp.hovered() { p.hover } else { Color32::TRANSPARENT }, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    painter.text(rect.center() - vec2(0.0, 8.0), Align2::CENTER_CENTER, icons::DOTS_THREE, style::icon_font(12.0), p.text_ghost);
    painter.text(rect.center() + vec2(0.0, 7.0), Align2::CENTER_CENTER, label, egui::FontId::proportional(12.0), p.text_faint);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A few lines of a terminal in `scheme`, in the terminal's own font at `size`, with its cursor:
/// what the settings beside it look like.
fn terminal_preview(ui: &mut Ui, r: Rect, scheme: &tc_term::scheme::Scheme, size: f32, cursor: CursorShape) {
    let p = style::pal(ui);
    let painter = ui.painter_at(r.expand(1.0));
    painter.rect(r, CornerRadius::same(8), scheme.background, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    let head = Rect::from_min_size(r.min, vec2(r.width(), 28.0));
    painter.hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, style::soft(scheme.foreground, 0.12)));
    let faint = style::soft(scheme.foreground, 0.55);
    painter.text(pos2(head.left() + 10.0 + 6.0, head.center().y), Align2::CENTER_CENTER, icons::EYE, style::icon_font(12.0), faint);
    painter.text(pos2(head.left() + 10.0 + 12.0 + 6.0, head.center().y), Align2::LEFT_CENTER, t("Preview"), egui::FontId::proportional(11.5), faint);
    let size_text = if size.fract() == 0.0 { format!("{size:.0}") } else { format!("{size:.1}") };
    painter.text(pos2(head.right() - 10.0, head.center().y), Align2::RIGHT_CENTER, format!("{} · {size_text} pt", scheme.name), style::mono(10.5), faint);
    let font = egui::FontId::new(size, egui::FontFamily::Name(tc_term::fonts::TERM_REGULAR.into()));
    let a = scheme.ansi;
    let fg = scheme.foreground;
    let lines: [Vec<(&str, Color32)>; 8] = [
        vec![("storefront", a[6]), (" on ", a[8]), (" main", a[5]), (" ❯ ", a[2]), ("git status -sb", fg)],
        vec![("## ", a[2]), ("main", a[2]), ("...", fg), ("origin/main", a[1]), (" [ahead 2]", fg)],
        vec![(" M ", a[1]), ("src/components/Header.tsx", fg)],
        vec![("?? ", a[1]), ("src/components/CartBadge.tsx", fg)],
        vec![("storefront", a[6]), (" ❯ ", a[2]), ("npm test -- --run", fg)],
        vec![(" ✓ ", a[2]), ("CartBadge renders the count ", fg), ("12 ms", a[8])],
        vec![(" ✕ ", a[1]), ("Header shows the badge ", fg), ("31 ms", a[8])],
        vec![("storefront", a[6]), (" ❯ ", a[2])],
    ];
    let line_h = (size * 1.35).round();
    let mut y = head.bottom() + 10.0;
    let x0 = r.left() + 12.0;
    let mut end = pos2(x0, y);
    for line in &lines {
        if y + line_h > r.bottom() - 6.0 {
            break;
        }
        let mut job = egui::text::LayoutJob::default();
        for (text, c) in line {
            job.append(text, 0.0, egui::TextFormat::simple(font.clone(), *c));
        }
        let g = painter.layout_job(job);
        end = pos2(x0 + g.size().x, y);
        painter.galley(pos2(x0, y + (line_h - g.size().y) / 2.0), g, fg);
        y += line_h;
    }
    // The cursor after the last prompt, as it is set.
    let cw = ui.fonts_mut(|f| f.glyph_width(&font, 'M'));
    let cell = Rect::from_min_size(end, vec2(cw, line_h));
    match cursor {
        CursorShape::Block => painter.rect_filled(cell, CornerRadius::ZERO, scheme.cursor),
        CursorShape::Beam => painter.rect_filled(Rect::from_min_size(cell.min, vec2(2.0, line_h)), CornerRadius::ZERO, scheme.cursor),
        CursorShape::Underline => painter.rect_filled(Rect::from_min_max(pos2(cell.left(), cell.bottom() - 2.0), cell.max), CornerRadius::ZERO, scheme.cursor),
    };
}

/// The app's mark: a 3 × 3 lattice on the accent.
pub fn paint_logo(painter: &egui::Painter, r: Rect, _p: &style::Palette) {
    // The app icon (assets/icon.svg), drawn: a blue square, the "</" path through a 3 × 3
    // lattice in cream, the lattice's other points faint.
    let s = r.width() / 1024.0;
    let at = |x: f32, y: f32| pos2(r.left() + x * s, r.top() + y * s);
    let square = Rect::from_min_max(at(72.0, 72.0), at(952.0, 952.0));
    painter.rect_filled(square, CornerRadius::same((196.0 * s).round().min(255.0) as u8), Color32::from_rgb(0x1f, 0x5f, 0xbf));
    let cream = Color32::from_rgb(0xff, 0xf6, 0xea);
    let stroke = Stroke::new((72.0 * s).max(1.0), cream);
    painter.line_segment([at(512.0, 312.0), at(312.0, 512.0)], stroke);
    painter.line_segment([at(312.0, 512.0), at(512.0, 712.0)], stroke);
    painter.line_segment([at(712.0, 312.0), at(512.0, 712.0)], stroke);
    for (x, y) in [(512.0, 312.0), (712.0, 312.0), (312.0, 512.0), (512.0, 712.0)] {
        painter.circle_filled(at(x, y), 52.0 * s, cream);
    }
    for (x, y) in [(312.0, 312.0), (512.0, 512.0), (712.0, 512.0), (312.0, 712.0), (712.0, 712.0)] {
        painter.circle_filled(at(x, y), 30.0 * s, cream.gamma_multiply(0.32));
    }
}
