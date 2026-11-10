//! Settings: general, terminal, notifications, agent presets and agent integrations.

use eframe::egui::{self, RichText, Ui};
use tc_core::agent::{AgentKind, Preset};
use tc_core::model::{Integration, Language, ThemeMode};

use super::{kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPage {
    General,
    Terminal,
    Editor,
    Notifications,
    Agents,
    Integrations,
}

fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let p = style::pal(ui);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(160.0, 24.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(160.0);
                ui.add(egui::Label::new(RichText::new(label).color(p.text_muted)).truncate());
            },
        );
        add(ui);
    });
    ui.add_space(4.0);
}

impl App {
    /// Draws the settings dialog; returns false when it should close.
    pub fn settings_ui(&mut self, ui: &mut Ui, page: &mut SettingsPage) -> bool {
        ui.set_width(660.0);
        ui.set_max_width(660.0);
        let mut keep = true;
        let p = style::pal(ui);
        if super::dialogs::header(
            ui,
            super::icons::GEAR_SIX,
            p.accent,
            t("Settings"),
            Some(t("Saved as you change them.")),
        ) {
            keep = false;
        }
        egui::Frame::NONE
            .inner_margin(egui::Margin { left: 22, right: 22, top: 12, bottom: 22 })
            .show(ui, |ui| {
        ui.set_min_height(420.0);
        kit::tab_strip(
            ui,
            page,
            &[
                (SettingsPage::General, t("General")),
                (SettingsPage::Terminal, t("Terminal")),
                (SettingsPage::Editor, t("Editor")),
                (SettingsPage::Notifications, t("Notifications")),
                (SettingsPage::Agents, t("Agents")),
                (SettingsPage::Integrations, t("Integrations")),
            ],
        );
        ui.add_space(12.0);
        let before = self.settings.clone();
        match page {
            SettingsPage::General => {
                row(ui, t("Language"), |ui| {
                    for (v, label) in [
                        (Language::System, t("System")),
                        (Language::English, "English"),
                        (Language::Indonesian, "Bahasa Indonesia"),
                    ] {
                        ui.radio_value(&mut self.settings.language, v, label);
                    }
                });
                row(ui, t("Theme"), |ui| {
                    for (v, label) in [
                        (ThemeMode::System, t("System")),
                        (ThemeMode::Light, t("Light")),
                        (ThemeMode::Dark, t("Dark")),
                    ] {
                        ui.radio_value(&mut self.settings.theme, v, label);
                    }
                });
                row(ui, t("Updates"), |ui| {
                    ui.checkbox(&mut self.settings.check_updates, t("Check automatically"));
                    if kit::secondary(ui, None, t("Check Now")).clicked() {
                        self.actions.push(crate::actions::Action::CheckForUpdates);
                    }
                });
                row(ui, t("Command line"), |ui| match crate::cli::installed() {
                    Some(path) => {
                        ui.label(RichText::new(format!("ternarycode → {}", crate::paths::contract_tilde(&path))).color(p.text_muted))
                            .on_hover_text(t("Type ternarycode <folder or project name> in any terminal to open it here."));
                    }
                    None => {
                        #[cfg(target_os = "macos")]
                        if kit::secondary(ui, None, t("Install ternarycode Command…")).clicked() {
                            match crate::cli::install_with_admin() {
                                Ok(path) => self.toast(tf("Installed: {}", &[&path.display().to_string()])),
                                Err(e) => self.toast(e),
                            }
                        }
                        #[cfg(not(target_os = "macos"))]
                        ui.label(RichText::new(t("Not on PATH (works in TernaryCode's own panes)")).color(p.text_muted));
                    }
                });
                row(ui, t("Version"), |ui| {
                    ui.label(RichText::new(format!("TernaryCode {}", crate::update::CURRENT)).color(p.text_muted));
                });
                row(ui, t("Data folder"), |ui| {
                    let dir = crate::paths::contract_tilde(crate::paths::data_dir());
                    ui.add(egui::Label::new(RichText::new(&dir).color(p.text_muted)).truncate())
                        .on_hover_text(dir);
                });
            }
            SettingsPage::Terminal => {
                row(ui, t("Font size"), |ui| {
                    ui.add(egui::Slider::new(&mut self.settings.font_size, 9.0..=24.0).step_by(0.5));
                });
                row(ui, t("Scrollback lines per pane"), |ui| {
                    ui.add(egui::DragValue::new(&mut self.settings.scrollback).range(1000..=100_000).speed(500));
                });
                row(ui, t("Blinking cursor"), |ui| {
                    ui.checkbox(&mut self.settings.cursor_blink, "");
                });
                row(ui, t("Option key as Meta"), |ui| {
                    ui.checkbox(&mut self.settings.option_as_meta, "");
                });
                row(ui, t("Copy on select"), |ui| {
                    ui.checkbox(&mut self.settings.copy_on_select, "");
                });
                ui.label(
                    RichText::new(t("Scrollback changes apply to new panes."))
                        .size(style::SMALL)
                        .color(p.text_muted),
                );
            }
            SettingsPage::Editor => {
                row(ui, t("Language features (LSP)"), |ui| {
                    ui.checkbox(&mut self.settings.lsp, "");
                });
                row(ui, t("Completion while typing"), |ui| {
                    ui.add_enabled(self.settings.lsp, egui::Checkbox::new(&mut self.settings.auto_complete, ""));
                });
                row(ui, t("Format on save"), |ui| {
                    ui.add_enabled(self.settings.lsp, egui::Checkbox::new(&mut self.settings.format_on_save, ""));
                });
                ui.label(
                    RichText::new(t("Language features use the language servers installed on this computer (rust-analyzer, gopls, typescript-language-server, dart…). A server starts when a file it serves is opened in the editor and stops 10 minutes after the last one is closed; with this off, none runs."))
                        .size(style::SMALL)
                        .color(p.text_muted),
                );
            }
            SettingsPage::Notifications => {
                let n = &mut self.settings.notifications;
                row(ui, t("An agent needs input"), |ui| {
                    ui.checkbox(&mut n.needs_input, "");
                });
                row(ui, t("A process failed"), |ui| {
                    ui.checkbox(&mut n.failed, "");
                });
                row(ui, t("An agent finished its turn"), |ui| {
                    ui.checkbox(&mut n.done, "");
                });
                row(ui, t("A long command finished"), |ui| {
                    ui.checkbox(&mut n.command_done, "");
                });
                row(ui, t("Badge on the Dock icon"), |ui| {
                    ui.checkbox(&mut n.dock_badge, "");
                });
                row(ui, t("\"Long\" means at least"), |ui| {
                    ui.add(egui::DragValue::new(&mut self.settings.long_command_secs).range(3..=600).suffix(" s"));
                });
                ui.label(
                    RichText::new(t("Notifications appear when the window is in the background or the pane is in another project. The keyboard focus never moves by itself."))
                        .size(style::SMALL)
                        .color(p.text_muted),
                );
            }
            SettingsPage::Agents => self.presets_ui(ui),
            SettingsPage::Integrations => self.integrations_ui(ui),
        }
        if self.settings != before {
            if self.settings.language != before.language {
                crate::i18n::set_language(self.settings.language);
            }
            if self.settings.theme != before.theme {
                style::apply(ui.ctx(), self.settings.theme);
            }
            if self.settings.lsp != before.lsp {
                if self.settings.lsp {
                    let ctx = ui.ctx().clone();
                    let open: Vec<std::path::PathBuf> = self.editors.values().flat_map(|e| e.files.iter().map(|f| f.path.clone())).collect();
                    for path in open {
                        self.lsp_open(&ctx, &path);
                    }
                } else {
                    self.lsp_stop_all();
                }
            }
            self.settings_changed();
        }
            });
        keep
    }

    fn presets_ui(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        ui.label(
            RichText::new(t("A preset is typed into a new pane's shell. Resume is offered after a restart."))
                .size(style::SMALL)
                .color(p.text_muted),
        );
        ui.add_space(8.0);
        let mut presets = self.settings.presets();
        let mut changed = false;
        let mut remove = None;
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for (i, preset) in presets.iter_mut().enumerate() {
                kit::card(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&preset.name).font(style::medium(13.0)));
                        if let Some(a) = preset.agent {
                            ui.label(RichText::new(a.label()).size(style::SMALL).color(p.text_faint));
                        }
                        if !preset.builtin {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if kit::icon_button(ui, super::icons::TRASH, t("Delete")).clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.add_sized([70.0, 22.0], egui::Label::new(t("Name")));
                        changed |= ui.add(egui::TextEdit::singleline(&mut preset.name).desired_width(380.0)).changed();
                    });
                    ui.horizontal(|ui| {
                        ui.add_sized([70.0, 22.0], egui::Label::new(t("Command")));
                        changed |= ui.add(egui::TextEdit::singleline(&mut preset.command).desired_width(380.0).font(egui::TextStyle::Monospace)).changed();
                    });
                    ui.horizontal(|ui| {
                        ui.add_sized([70.0, 22.0], egui::Label::new(t("Resume")));
                        let mut resume = preset.resume.clone().unwrap_or_default();
                        if ui.add(egui::TextEdit::singleline(&mut resume).desired_width(380.0).font(egui::TextStyle::Monospace)).changed() {
                            preset.resume = Some(resume).filter(|r| !r.trim().is_empty());
                            changed = true;
                        }
                    });
                });
                ui.add_space(4.0);
            }
        });
        if let Some(i) = remove {
            presets.remove(i);
            changed = true;
        }
        if kit::secondary(ui, Some(super::icons::PLUS), t("Add Preset")).clicked() {
            let n = presets.iter().filter(|p| !p.builtin).count() + 1;
            // An id no preset has (counting would repeat one after a deletion).
            let mut k = n;
            while presets.iter().any(|p| p.id == format!("custom-{k}")) {
                k += 1;
            }
            presets.push(Preset {
                id: format!("custom-{k}"),
                name: tf("My agent {}", &[&n.to_string()]),
                agent: Some(AgentKind::Custom),
                command: String::new(),
                resume: None,
                env: Default::default(),
                builtin: false,
            });
            changed = true;
        }
        if changed {
            // Saved: every user preset, and built-ins only where they differ from the default.
            let defaults = tc_core::agent::builtin_presets();
            self.settings.presets = presets
                .into_iter()
                .filter(|p| !p.builtin || !defaults.contains(p))
                .collect();
        }
    }

    fn integrations_ui(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        ui.label(
            RichText::new(t("How agents tell TernaryCode that they need you. Per launch adds hooks only when a preset starts the agent and changes no file. Installed writes the hooks into the agent's own settings, so they also work when you type the agent's name yourself."))
                .size(style::SMALL)
                .color(p.text_muted),
        );
        ui.add_space(10.0);
        for agent in [AgentKind::Claude, AgentKind::Codex, AgentKind::Gemini, AgentKind::OpenCode] {
            let per_launch = matches!(agent, AgentKind::Claude | AgentKind::Codex);
            let installed = self.installed(agent);
            // Per launch is the default (and what Uninstall leaves), but an agent without it
            // gets no hooks then: that is Off. Nothing is saved unless a choice is made.
            let shown = match self.settings.integration(agent) {
                Integration::PerLaunch if !per_launch => Integration::Off,
                m => m,
            };
            let mut mode = shown;
            kit::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(agent.label()).font(style::medium(13.5)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(rec) = &installed {
                            if kit::secondary(ui, None, t("Uninstall")).clicked() {
                                self.actions.push(Action::Uninstall(agent));
                            }
                            ui.label(
                                RichText::new(tf("in {}", &[&crate::paths::contract_tilde(&rec.path)]))
                                    .size(style::SMALL)
                                    .color(p.text_faint),
                            );
                        } else {
                            let r = kit::secondary(ui, None, t("Install…"));
                            #[cfg(debug_assertions)]
                            crate::devtest::record(format!("install:{}", agent.slug()), r.rect);
                            if r.clicked() {
                                self.actions.push(Action::PlanInstall(agent));
                            }
                        }
                    });
                });
                ui.horizontal(|ui| {
                    if per_launch {
                        ui.radio_value(&mut mode, Integration::PerLaunch, t("Per launch"));
                    }
                    if installed.is_some() {
                        ui.radio_value(&mut mode, Integration::Installed, t("Installed"));
                    }
                    ui.radio_value(&mut mode, Integration::Off, t("Off"));
                });
                let note = match agent {
                    AgentKind::Claude => t("Hooks: permission requests, questions, finished turns. Checked with Claude Code 2.1.319."),
                    AgentKind::Codex => t("Hooks and notify. Codex asks you once to trust TernaryCode's hooks (its own security check). Checked with Codex 0.162.1."),
                    AgentKind::Gemini => t("Install adds hooks to ~/.gemini/settings.json (tool permissions, finished turns). Gemini CLI has no per-launch option. Checked with Gemini CLI 0.43.0."),
                    _ => t("Install adds a small plugin file to OpenCode's plugins folder (permission requests, finished turns). OpenCode has no per-launch option. Checked with OpenCode 1.19.5."),
                };
                ui.label(RichText::new(note).size(style::SMALL).color(p.text_muted));
            });
            if mode != shown {
                self.settings.integrations.insert(agent.slug().into(), mode);
            }
            ui.add_space(6.0);
        }
        let exists = self.hook_bin.bin.exists();
        ui.label(
            RichText::new(tf(
                "Hook program: {}{}",
                &[
                    &crate::paths::contract_tilde(&self.hook_bin.bin),
                    if exists { "" } else { t(" (missing)") },
                ],
            ))
            .size(style::SMALL)
            .color(if exists { p.text_faint } else { p.err }),
        );
    }
}
