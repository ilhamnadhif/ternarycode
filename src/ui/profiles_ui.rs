//! Multi-Account Agent Profiles & Failover Management view (`View::Profiles`), plus the
//! inline Active Profile Switcher popover and Rate-Limit Recovery Banner inside agent panes.

use std::time::Instant;

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use tc_core::agent::AgentKind;
use tc_core::model::{AgentProfile, FailoverMode};
use tc_core::PaneId;

use super::{agent_logo, icons, kit, style};
use crate::actions::Action;
use crate::ade::ProfileDraft;
use crate::app::App;
use crate::i18n::{t, tf};
use crate::profiles::{self, AuthStatus, ConnectionTest, ProfileState};

pub fn profile_state_color(state: ProfileState, p: &style::Palette) -> Color32 {
    match state {
        ProfileState::Running => p.info,
        ProfileState::Idle => p.ok,
        ProfileState::RateLimited => p.rate_limited,
        ProfileState::Cooldown => p.cooldown,
        ProfileState::Error => p.err,
    }
}

pub fn profile_state_label(state: ProfileState) -> &'static str {
    match state {
        ProfileState::Running => t("Running"),
        ProfileState::Idle => t("Idle"),
        ProfileState::RateLimited => t("Rate-Limited"),
        ProfileState::Cooldown => t("Cooldown"),
        ProfileState::Error => t("Error"),
    }
}

pub fn usage_pct_color(used_pct: f32, p: &style::Palette) -> Color32 {
    if used_pct >= 90.0 {
        p.rate_limited
    } else if used_pct >= 70.0 {
        p.warn
    } else {
        p.ok
    }
}

/// Renders one compact, disciplined usage-limit meter (`5h` or `Weekly` / `30d`) with progress bar,
/// percentage or token count, and reset countdown.
pub fn paint_window_meter(
    ui: &mut Ui,
    label: &str,
    used_pct: Option<f32>,
    tokens: u64,
    reset_secs: Option<u64>,
    bar_w: f32,
    p: &style::Palette,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(label).font(style::mono_semibold(10.5)).color(p.text_muted));
        if let Some(pct) = used_pct {
            let bc = usage_pct_color(pct, p);
            let (bar_r, _) = ui.allocate_exact_size(vec2(bar_w, 5.0), Sense::hover());
            kit::paint_quota_bar(ui.painter(), bar_r, pct, bc, p);
            ui.label(RichText::new(format!("{pct:.0}%")).font(style::mono_semibold(10.5)).color(bc));
            if tokens > 0 {
                ui.label(RichText::new(profiles::format_tokens(tokens)).font(style::mono(10.0)).color(p.text_faint));
            }
        } else {
            let (bar_r, _) = ui.allocate_exact_size(vec2(bar_w, 5.0), Sense::hover());
            kit::paint_quota_bar(ui.painter(), bar_r, 0.0, p.idle, p);
            let tok_str = if tokens > 0 {
                tf("{} tok", &[&profiles::format_tokens(tokens)])
            } else {
                "0".into()
            };
            ui.label(RichText::new(tok_str).font(style::mono(10.5)).color(p.text_faint));
        }
        if let Some(secs) = reset_secs {
            let rc = if used_pct.is_some_and(|v| v >= 90.0) { p.rate_limited } else { p.text_ghost };
            ui.label(RichText::new(icons::CLOCK_COUNTER_CLOCKWISE).font(style::icon_font(10.5)).color(rc));
            ui.label(RichText::new(profiles::format_countdown(secs)).font(style::mono(10.0)).color(rc));
        }
    });
}

fn compact_window_summary(
    prof: &AgentProfile,
    rt: Option<&profiles::ProfileRuntime>,
    now_unix: i64,
    now: Instant,
) -> String {
    let Some(rt) = rt else {
        return t("Idle").into();
    };
    if let Some(secs) = rt.reset_remaining_secs(now) {
        return profiles::format_countdown(secs);
    }
    let mut parts = Vec::new();
    let l5 = rt.window_5h_label();
    if let Some(p5) = rt.quota_5h_used_pct(prof) {
        parts.push(format!("{l5} {p5:.0}%"));
    } else if rt.usage.window_5h.tokens.total() > 0 {
        parts.push(format!("{l5} {}", profiles::format_tokens(rt.usage.window_5h.tokens.total())));
    }
    let lw = if rt.window_weekly_label() == "Weekly" { "Wk".to_string() } else { rt.window_weekly_label() };
    if let Some(pw) = rt.quota_weekly_used_pct(prof) {
        parts.push(format!("{lw} {pw:.0}%"));
    } else if rt.usage.window_weekly.tokens.total() > 0 && parts.is_empty() {
        parts.push(format!("{lw} {}", profiles::format_tokens(rt.usage.window_weekly.tokens.total())));
    }
    if parts.is_empty() {
        if let Some(secs) = rt.window_5h_reset_secs(now_unix, now) {
            profiles::format_countdown(secs)
        } else {
            profile_state_label(rt.state).into()
        }
    } else {
        parts.join(" · ")
    }
}

impl App {
    /// Dedicated Profiles & Accounts management page (`View::Profiles`).
    pub fn profiles_view(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let full = ui.max_rect();
        let body = self.top_bar(
            ui,
            full,
            |_, ui| {
                ui.add_space(4.0);
                ui.label(RichText::new(t("Agent Profiles & Failover")).font(style::semibold(14.0)).color(p.text));
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(t("Multi-account credentials, isolated auth sessions, and rate-limit recovery"))
                            .size(12.0)
                            .color(p.text_faint),
                    )
                    .truncate(),
                );
            },
            None,
        );
        let area = super::focus::sheets_area(body, self.settings.sidebar_hidden);
        super::focus::paint_sheet(ui.painter(), area, &p, super::focus::SheetLook::Plain);

        let inner = area.shrink(1.0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);
            egui::ScrollArea::vertical()
                .id_salt("profiles-view-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(24, 20))
                        .show(ui, |ui| {
                            ui.set_width((inner.width() - 48.0).max(320.0));
                            self.profiles_policy_bar(ui);
                            ui.add_space(16.0);
                            if self.profile_draft.is_some() {
                                self.profile_editor_panel(ui);
                                ui.add_space(16.0);
                            }
                            self.profiles_table(ui);
                        });
                });
        });
    }

    fn profiles_policy_bar(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, if dark { p.border } else { p.border_strong }))
            .corner_radius(CornerRadius::same(9))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(12.0, 8.0);
                    ui.label(RichText::new(t("Failover Mode")).font(style::medium(12.5)).color(p.text));
                    let modes = [
                        kit::Seg::new(t("Auto-Resume"), t("Automatically fail over to the next healthy profile and continue")).icon(icons::LIGHTNING),
                        kit::Seg::new(t("Manual Resume"), t("Pause with an inline banner to choose the fallback profile")).icon(icons::HAND_POINTING),
                    ];
                    let current = usize::from(self.settings.failover.mode == FailoverMode::Manual);
                    if let Some(i) = kit::segmented(ui, &modes, current) {
                        self.settings.failover.mode = if i == 0 { FailoverMode::Auto } else { FailoverMode::Manual };
                        self.settings_changed();
                        self.push_companion_now();
                    }

                    kit::vsep(ui);
                    let mut cross = self.settings.failover.cross_provider;
                    kit::switch(ui, &mut cross);
                    if cross != self.settings.failover.cross_provider {
                        self.settings.failover.cross_provider = cross;
                        self.settings_changed();
                    }
                    ui.label(RichText::new(t("Cross-provider fallback")).size(12.0).color(p.text_muted))
                        .on_hover_text(t("Fall back to another AI provider when all accounts of the active provider hit their limit"));

                    kit::vsep(ui);
                    ui.label(RichText::new(t("Default cooldown")).size(12.0).color(p.text_muted));
                    let opts = ["15m", "30m", "60m", "120m"];
                    let mins = [15u32, 30, 60, 120];
                    let cur_label = format!("{}m", self.settings.failover.default_cooldown_mins);
                    if let Some(i) = kit::select(ui, "failover-cooldown", &cur_label, &opts, 76.0) {
                        self.settings.failover.default_cooldown_mins = mins[i];
                        self.settings_changed();
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let add = kit::button_ex(ui, kit::Kind::Primary, Some(icons::PLUS), t("Add Profile"), false);
                        #[cfg(debug_assertions)]
                        crate::devtest::record("profiles-add", add.rect);
                        if add.clicked() {
                            let existing = self.effective_profiles();
                            self.profile_draft = Some(ProfileDraft::new_for(AgentKind::Claude, &existing));
                        }
                    });
                });
            });
    }

    fn profile_editor_panel(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let Some(mut draft) = self.profile_draft.clone() else { return };
        let mut save = false;
        let mut cancel = false;

        let existing = self.effective_profiles();
        let sanitized_id = profiles::sanitize_id(draft.id.trim());
        let id_empty = draft.id.trim().is_empty();
        let id_dup = !id_empty
            && existing
                .iter()
                .any(|x| x.id == sanitized_id && draft.editing_id.as_deref() != Some(x.id.as_str()));

        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, style::soft(p.accent, 0.45)))
            .corner_radius(CornerRadius::same(9))
            .inner_margin(egui::Margin::symmetric(16, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let heading = if draft.editing_id.is_some() { t("Edit Agent Profile") } else { t("New Agent Profile") };
                    ui.label(RichText::new(heading).font(style::semibold(13.5)).color(p.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if kit::icon_button_ex(ui, icons::X, t("Close"), false, 22.0).clicked() {
                            cancel = true;
                        }
                    });
                });
                ui.add_space(10.0);

                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(12.0, 8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(t("Provider")).size(11.5).color(p.text_muted));
                        let providers = [
                            (AgentKind::Claude, "Claude Code"),
                            (AgentKind::Codex, "ChatGPT / Codex"),
                            (AgentKind::Gemini, "Gemini CLI"),
                            (AgentKind::OpenCode, "OpenCode"),
                            (AgentKind::Antigravity, "Antigravity"),
                            (AgentKind::Custom, "Custom"),
                        ];
                        let names: Vec<&str> = providers.iter().map(|(_, n)| *n).collect();
                        let cur = providers.iter().find(|(k, _)| *k == draft.agent).map(|(_, n)| *n).unwrap_or("Claude Code");
                        if let Some(i) = kit::select(ui, "prof-agent", cur, &names, 165.0) {
                            let new_agent = providers[i].0;
                            if new_agent != draft.agent {
                                // Auto-update sequential ID/name (`chatgpt2`, `claude2`) if still creating a new profile.
                                if draft.editing_id.is_none() {
                                    let (old_id, old_name) = ProfileDraft::suggest_id_and_name(draft.agent, &existing);
                                    let (nid, nname) = ProfileDraft::suggest_id_and_name(new_agent, &existing);
                                    if draft.id == old_id || draft.id.trim().is_empty() {
                                        draft.id = nid;
                                    }
                                    if draft.name == old_name || draft.name.trim().is_empty() {
                                        draft.name = nname;
                                    }
                                }
                                draft.agent = new_agent;
                            }
                        }
                    });
                    ui.vertical(|ui| {
                        ui.label(RichText::new(t("Profile ID")).size(11.5).color(p.text_muted));
                        kit::field(ui, egui::Id::new("prof-id"), &mut draft.id, None, "chatgpt2", 130.0, false);
                        if id_dup {
                            ui.label(RichText::new(t("Profile ID already exists.")).size(11.0).color(p.err));
                        }
                    });
                    ui.vertical(|ui| {
                        ui.label(RichText::new(t("Display Name")).size(11.5).color(p.text_muted));
                        kit::field(ui, egui::Id::new("prof-name"), &mut draft.name, None, "ChatGPT Plus #2", 170.0, false);
                    });
                    ui.vertical(|ui| {
                        ui.label(RichText::new(t("Account Note / Email (optional)")).size(11.5).color(p.text_muted));
                        kit::field(ui, egui::Id::new("prof-hint"), &mut draft.account_hint, None, "work@gmail.com", 180.0, false);
                    });
                });

                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(12.0, 8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(t("5-Hour Token Limit (optional)")).size(11.5).color(p.text_muted));
                        kit::field(ui, egui::Id::new("prof-lim-5h"), &mut draft.limit_5h_text, None, "500k / 2M", 150.0, false);
                    });
                    ui.vertical(|ui| {
                        ui.label(RichText::new(t("Weekly Token Limit (optional)")).size(11.5).color(p.text_muted));
                        kit::field(ui, egui::Id::new("prof-lim-wk"), &mut draft.limit_weekly_text, None, "5M / 20M", 150.0, false);
                    });
                    ui.vertical(|ui| {
                        ui.add_space(19.0);
                        ui.label(
                            RichText::new(t("ChatGPT / Codex reports 5h & weekly % automatically; set token limits for Claude or Gemini."))
                                .size(11.0)
                                .color(p.text_faint),
                        );
                    });
                });

                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(12.0, 8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(t("Isolated Auth / Config Folder")).size(11.5).color(p.text_muted));
                        let hint = crate::paths::contract_tilde(
                            &crate::paths::data_dir().join("profiles").join(&sanitized_id),
                        );
                        ui.add_enabled_ui(!draft.use_default_auth, |ui| {
                            kit::field(ui, egui::Id::new("prof-dir"), &mut draft.config_dir, Some(icons::FOLDER), &hint, 310.0, false);
                        });
                    });
                    ui.vertical(|ui| {
                        ui.label(RichText::new(t("API Key (optional)")).size(11.5).color(p.text_muted));
                        ui.horizontal(|ui| {
                            let w = 250.0;
                            let id = egui::Id::new("prof-key");
                            let (rect, outer) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                            if outer.clicked() {
                                ui.ctx().memory_mut(|m| m.request_focus(id));
                            }
                            let focused = ui.ctx().memory(|m| m.has_focus(id));
                            let fill = if dark { p.inset } else { Color32::WHITE };
                            ui.painter().rect(rect, CornerRadius::same(style::RADIUS), fill, Stroke::new(1.0, if focused { p.accent } else { p.border_strong }), StrokeKind::Inside);
                            let inner = rect.shrink2(vec2(10.0, 1.0));
                            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
                            child.add_sized(
                                inner.size(),
                                egui::TextEdit::singleline(&mut draft.api_key)
                                    .id(id)
                                    .password(!draft.reveal_key)
                                    .frame(egui::Frame::NONE)
                                    .font(style::mono(12.0))
                                    .hint_text(RichText::new("sk-…").color(p.text_ghost)),
                            );
                            if kit::icon_button_ex(ui, icons::EYE, t("Show/Hide Key"), draft.reveal_key, 28.0).clicked() {
                                draft.reveal_key = !draft.reveal_key;
                            }
                        });
                    });
                });

                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(t("Extra Environment (KEY=VALUE per line, optional)")).size(11.5).color(p.text_muted));
                    let env_w = ui.available_width().min(580.0);
                    egui::Frame::NONE
                        .fill(if dark { p.inset } else { Color32::WHITE })
                        .stroke(Stroke::new(1.0, p.border_strong))
                        .corner_radius(CornerRadius::same(style::RADIUS))
                        .inner_margin(egui::Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut draft.extra_env_text)
                                    .id(egui::Id::new("prof-extra-env"))
                                    .desired_rows(2)
                                    .desired_width(env_w)
                                    .frame(egui::Frame::NONE)
                                    .font(style::mono(11.5))
                                    .hint_text(RichText::new("ANTHROPIC_BASE_URL=https://…").color(p.text_ghost)),
                            );
                        });
                });

                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    kit::switch(ui, &mut draft.use_default_auth);
                    ui.label(RichText::new(t("Use system default auth folder (~/.claude, ~/.codex, ~/.gemini)")).size(12.0).color(p.text_muted));
                    ui.add_space(14.0);
                    kit::switch(ui, &mut draft.enabled);
                    ui.label(RichText::new(t("Enabled in failover chain")).size(12.0).color(p.text_muted));
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ok_enabled = !id_empty && !id_dup;
                        if ui.add_enabled_ui(ok_enabled, |ui| kit::button_ex(ui, kit::Kind::Primary, Some(icons::CHECK), t("Save Profile"), false)).inner.clicked() {
                            save = true;
                        }
                        if kit::button_ex(ui, kit::Kind::Secondary, None, t("Cancel"), false).clicked() {
                            cancel = true;
                        }
                    });
                });
            });

        if cancel {
            self.profile_draft = None;
        } else if save && !id_empty && !id_dup {
            self.ensure_mutable_profiles();
            let editing = draft.editing_id.clone();
            let next_order = self.settings.profiles.iter().map(|x| x.fallback_order).max().unwrap_or(0) + 1;
            if let Some(old_id) = editing
                && let Some(slot) = self.settings.profiles.iter_mut().find(|x| x.id == old_id)
            {
                let order = slot.fallback_order;
                let updated = draft.into_profile(order);
                let new_id = updated.id.clone();
                *slot = updated;
                if old_id != new_id {
                    if let Some(rt) = self.profiles_rt.remove(&old_id) {
                        self.profiles_rt.insert(new_id.clone(), rt);
                    }
                    for pane in self.panes.values_mut() {
                        if pane.active_profile.as_deref() == Some(old_id.as_str()) {
                            pane.active_profile = Some(new_id.clone());
                        }
                        if let Some(b) = &mut pane.rate_limit
                            && b.from_profile == old_id
                        {
                            b.from_profile = new_id.clone();
                        }
                    }
                    for proj in &mut self.projects {
                        if proj.active_profile.as_deref() == Some(old_id.as_str()) {
                            proj.active_profile = Some(new_id.clone());
                        }
                    }
                }
            } else {
                let new_prof = draft.into_profile(next_order);
                self.settings.profiles.push(new_prof);
            }
            self.settings_changed();
            self.profile_draft = None;
            self.usage_scan_at = None;
            self.push_companion_now();
            self.toast_ok(t("Profile saved.").into());
        } else {
            self.profile_draft = Some(draft);
        }
    }

    fn profiles_table(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let now = Instant::now();
        let profiles = self.effective_profiles();
        let home = crate::paths::home_dir();
        let data = crate::paths::data_dir();

        let mut move_dir: Option<(usize, i32)> = None;
        let mut toggle_enabled: Option<String> = None;
        let mut delete_id: Option<String> = None;
        let mut edit_prof: Option<AgentProfile> = None;

        let now_unix = profiles::now_unix_secs();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            ui.label(RichText::new(t("Fallback Priority Chain")).font(style::semibold(13.0)).color(p.text));
            ui.label(
                RichText::new(tf("{} configured", &[&profiles.len().to_string()]))
                    .font(style::mono(11.0))
                    .color(p.text_faint),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::PLUS), t("Gemini"), true).clicked() {
                    self.profile_draft = Some(ProfileDraft::new_for(AgentKind::Gemini, &profiles));
                }
                if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::PLUS), t("Claude"), true).clicked() {
                    self.profile_draft = Some(ProfileDraft::new_for(AgentKind::Claude, &profiles));
                }
                if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::PLUS), t("ChatGPT"), true)
                    .on_hover_text(t("Pre-register another ChatGPT Plus / Codex account with its own isolated login"))
                    .clicked()
                {
                    self.profile_draft = Some(ProfileDraft::new_for(AgentKind::Codex, &profiles));
                }
            });
        });
        ui.add_space(8.0);

        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, if dark { p.border } else { p.border_strong }))
            .corner_radius(CornerRadius::same(9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;

                for (idx, prof) in profiles.iter().enumerate() {
                    if idx > 0 {
                        let y = ui.cursor().top();
                        ui.painter().hline(ui.max_rect().x_range(), y + 0.5, Stroke::new(1.0, p.border));
                    }
                    let rt = self.profiles_rt.get(&prof.id).cloned().unwrap_or_default();
                    let state_c = profile_state_color(rt.state, &p);
                    let dir = profiles::resolved_config_dir(prof, &home, data);
                    let dir_str = crate::paths::contract_tilde(&dir);
                    let auth = rt.auth_status(prof);
                    let used_5h = rt.quota_5h_used_pct(prof);
                    let used_wk = rt.quota_weekly_used_pct(prof);
                    let reset_5h = rt.window_5h_reset_secs(now_unix, now);
                    let reset_wk = rt.window_weekly_reset_secs(now_unix);
                    let l5 = rt.window_5h_label();
                    let lw = rt.window_weekly_label();
                    let can_delete = profiles.len() > 1;
                    let needs_login = auth == AuthStatus::NotSignedIn;

                    egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(14, 12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 5.0;
                            // Row 1: priority, logo, identity & status badges (left) + actions (right).
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                ui.label(RichText::new(format!("#{}", idx + 1)).font(style::mono_semibold(11.0)).color(p.text_faint));
                                if ui.add_enabled_ui(idx > 0, |ui| kit::icon_button_ex(ui, icons::ARROW_UP, t("Move Up"), false, 20.0)).inner.clicked() {
                                    move_dir = Some((idx, -1));
                                }
                                if ui.add_enabled_ui(idx + 1 < profiles.len(), |ui| kit::icon_button_ex(ui, icons::ARROW_DOWN, t("Move Down"), false, 20.0)).inner.clicked() {
                                    move_dir = Some((idx, 1));
                                }
                                let (mr, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                                if !agent_logo::paint(ui.painter(), mr, prof.agent, p.text, false) {
                                    ui.painter().text(mr.center(), Align2::CENTER_CENTER, icons::TERMINAL_WINDOW, style::icon_font(13.0), p.text_muted);
                                }
                                ui.label(RichText::new(&prof.id).font(style::mono_semibold(12.5)).color(if prof.enabled { p.text } else { p.text_faint }));
                                ui.label(RichText::new(&prof.name).size(12.0).color(p.text_muted));
                                if let Some(plan) = &rt.usage.plan_type {
                                    kit::status_badge(ui, &plan.to_uppercase(), p.accent_text, true);
                                }
                                kit::status_badge(ui, profile_state_label(rt.state), state_c, false);
                                match auth {
                                    AuthStatus::SignedIn => {
                                        kit::status_badge(ui, t("Signed in"), p.ok, false);
                                    }
                                    AuthStatus::ApiKey => {
                                        kit::status_badge(ui, t("API Key"), p.info, false);
                                    }
                                    AuthStatus::NotSignedIn => {
                                        kit::status_badge(ui, t("Not signed in"), p.warn, false);
                                    }
                                    AuthStatus::Unknown => {}
                                }

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    let del_tip = if can_delete { t("Delete Profile") } else { t("At least one profile is required") };
                                    if ui.add_enabled_ui(can_delete, |ui| kit::icon_button_ex(ui, icons::TRASH, del_tip, false, 24.0)).inner.clicked() {
                                        delete_id = Some(prof.id.clone());
                                    }
                                    if kit::icon_button_ex(ui, icons::PENCIL_SIMPLE, t("Edit Profile"), false, 24.0).clicked() {
                                        edit_prof = Some(prof.clone());
                                    }
                                    let login_kind = if needs_login { kit::Kind::Primary } else { kit::Kind::Secondary };
                                    if kit::button_ex(ui, login_kind, Some(icons::KEY), t("Auth Login"), true)
                                        .on_hover_text(t("Sign in interactively inside this profile's isolated auth folder"))
                                        .clicked()
                                    {
                                        self.actions.push(Action::LoginProfile(prof.id.clone()));
                                    }
                                    if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::PLUGS_CONNECTED), t("Test"), true).clicked() {
                                        self.actions.push(Action::TestProfile(prof.id.clone()));
                                    }
                                    if matches!(rt.state, ProfileState::RateLimited | ProfileState::Cooldown)
                                        && kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROW_CLOCKWISE), t("Clear"), true).clicked()
                                    {
                                        self.actions.push(Action::ClearProfileCooldown(prof.id.clone()));
                                    }
                                    let mut en = prof.enabled;
                                    kit::switch(ui, &mut en);
                                    if en != prof.enabled {
                                        toggle_enabled = Some(prof.id.clone());
                                    }
                                });
                            });

                            // Row 2 (full width, overlap-proof): 5-Hour & Weekly usage limit meters.
                            ui.horizontal(|ui| {
                                ui.add_space(76.0);
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = vec2(20.0, 4.0);
                                    paint_window_meter(ui, &l5, used_5h, rt.usage.window_5h.tokens.total(), reset_5h, 96.0, &p);
                                    paint_window_meter(ui, &lw, used_wk, rt.usage.window_weekly.tokens.total(), reset_wk, 96.0, &p);
                                });
                            });

                            // Row 3 (quiet truncated metadata): account hint + isolated config folder.
                            ui.horizontal(|ui| {
                                ui.add_space(76.0);
                                let meta = match &prof.account_hint {
                                    Some(hint) => format!("{hint}  ·  {dir_str}"),
                                    None => dir_str.clone(),
                                };
                                ui.add(egui::Label::new(RichText::new(meta).font(style::mono(10.0)).color(p.text_ghost)).truncate());
                            });

                            if let Some(conn) = &rt.connection {
                                ui.horizontal(|ui| {
                                    ui.add_space(76.0);
                                    match conn {
                                        ConnectionTest::Testing => {
                                            ui.label(RichText::new(t("Testing connection…")).size(11.0).color(p.info));
                                        }
                                        ConnectionTest::Ok(msg) => {
                                            ui.label(RichText::new(msg).font(style::mono(10.5)).color(p.ok));
                                        }
                                        ConnectionTest::Err(err) => {
                                            ui.label(RichText::new(err).font(style::mono(10.5)).color(p.err));
                                        }
                                    }
                                });
                            }
                        });
                }
            });

        if let Some((idx, delta)) = move_dir {
            self.ensure_mutable_profiles();
            self.settings.profiles.sort_by_key(|x| (x.fallback_order, x.id.clone()));
            let target = (idx as i32 + delta) as usize;
            if idx < self.settings.profiles.len() && target < self.settings.profiles.len() {
                self.settings.profiles.swap(idx, target);
                for (i, item) in self.settings.profiles.iter_mut().enumerate() {
                    item.fallback_order = (i + 1) as u32;
                }
                self.settings_changed();
                self.push_companion_now();
            }
        }
        if let Some(id) = toggle_enabled {
            self.ensure_mutable_profiles();
            if let Some(slot) = self.settings.profiles.iter_mut().find(|x| x.id == id) {
                slot.enabled = !slot.enabled;
                self.settings_changed();
                self.push_companion_now();
            }
        }
        if let Some(id) = delete_id {
            self.ensure_mutable_profiles();
            if self.settings.profiles.len() > 1 {
                self.settings.profiles.retain(|x| x.id != id);
                self.profiles_rt.remove(&id);
                for pane in self.panes.values_mut() {
                    if pane.active_profile.as_deref() == Some(id.as_str()) {
                        pane.active_profile = None;
                    }
                    if pane.rate_limit.as_ref().is_some_and(|b| b.from_profile == id) {
                        pane.rate_limit = None;
                    }
                }
                for proj in &mut self.projects {
                    if proj.active_profile.as_deref() == Some(id.as_str()) {
                        proj.active_profile = None;
                    }
                }
                if self.profile_draft.as_ref().and_then(|d| d.editing_id.as_deref()) == Some(id.as_str()) {
                    self.profile_draft = None;
                }
                self.settings_changed();
                self.push_companion_now();
                self.toast(tf("Removed profile {}.", &[&id]));
            }
        }
        if let Some(prof) = edit_prof {
            self.profile_draft = Some(ProfileDraft::from_profile(&prof));
        }
    }

    /// Compact interactive Active Profile Switcher + 1-Click Switch Account button rendered inside
    /// a pane's header bar (right-to-left layout).
    pub fn pane_profile_switcher(&mut self, ui: &mut Ui, pane_id: PaneId) {
        let p = style::pal(ui);
        let now = Instant::now();
        let now_unix = profiles::now_unix_secs();
        let profiles = self.effective_profiles();
        let Some((active_id, agent, has_live_agent)) = self.panes.get(&pane_id).map(|pane| {
            let agent = pane.tracker.agent.or(pane.last_agent);
            let id = pane.active_profile.clone().or_else(|| {
                agent.and_then(|a| profiles.iter().find(|x| x.agent == a && x.enabled).map(|x| x.id.clone()))
            });
            let live = pane.alive()
                && pane.tracker.agent.is_some()
                && !matches!(pane.tracker.fg, tc_core::status::Foreground::Shell | tc_core::status::Foreground::Gone);
            (id, agent, live)
        }) else {
            return;
        };
        let label = active_id.as_deref().unwrap_or("default");
        let rt = active_id.as_deref().and_then(|id| self.profiles_rt.get(id));
        let prof_cfg = active_id.as_deref().and_then(|id| profiles.iter().find(|x| x.id == id));
        let dot = if let Some((r, c)) = rt.zip(prof_cfg) {
            if r.state == ProfileState::RateLimited {
                p.rate_limited
            } else if let Some(used) = r.quota_5h_used_pct(c) {
                if used >= 90.0 { p.rate_limited } else if used >= 70.0 { p.warn } else { profile_state_color(r.state, &p) }
            } else {
                profile_state_color(r.state, &p)
            }
        } else {
            p.idle
        };
        let detail = rt.zip(prof_cfg).and_then(|(r, c)| {
            if let Some(secs) = r.reset_remaining_secs(now) {
                Some(profiles::format_countdown(secs))
            } else {
                r.quota_5h_used_pct(c).map(|used| format!("{} {:.0}%", r.window_5h_label(), used))
            }
        });
        let resp = kit::profile_chip(ui, label, detail.as_deref(), dot, false);
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("pane-profile:{pane_id}"), resp.rect);

        // One-click Switch Account button right beside the active profile chip when another
        // pre-registered account is ready (no logout / re-login required).
        if has_live_agent && let Some(next_prof) = self.next_switch_profile(pane_id) {
            let next_rt = self.profiles_rt.get(&next_prof.id);
            let next_sum = compact_window_summary(&next_prof, next_rt, now_unix, now);
            let tip = tf("Switch immediately to {} ({}) and resume session without logout", &[&next_prof.id, &next_sum]);
            let urgent = rt.zip(prof_cfg).is_some_and(|(r, c)| {
                r.state == ProfileState::RateLimited || r.quota_5h_used_pct(c).is_some_and(|u| u >= 85.0)
            });
            let sw = kit::button_ex(
                ui,
                if urgent { kit::Kind::Primary } else { kit::Kind::Ghost },
                Some(icons::ARROWS_CLOCKWISE),
                &next_prof.id,
                true,
            )
            .on_hover_text(tip);
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("pane-switch:{pane_id}"), sw.rect);
            if sw.clicked() {
                self.actions.push(Action::ResumeWithProfile {
                    pane: pane_id,
                    profile_id: next_prof.id.clone(),
                });
            }
        }

        egui::Popup::menu(&resp).align(egui::RectAlign::BOTTOM_END).show(|ui| {
            kit::menu_body(ui);
            ui.set_min_width(300.0);
            kit::menu_header(ui, t("Switch Account Profile"));
            let mut ordered: Vec<&AgentProfile> = profiles.iter().filter(|p| p.enabled).collect();
            ordered.sort_by_key(|p| (agent.is_some_and(|a| p.agent != a), p.fallback_order, p.id.as_str()));
            for prof in ordered {
                let prt = self.profiles_rt.get(&prof.id);
                let state = prt.map(|r| r.state).unwrap_or(ProfileState::Idle);
                let auth = prt.map(|r| r.auth_status(prof)).unwrap_or(AuthStatus::Unknown);
                let right = if auth == AuthStatus::NotSignedIn {
                    t("Not signed in").to_string()
                } else {
                    compact_window_summary(prof, prt, now_unix, now)
                };
                let is_cur = active_id.as_deref() == Some(prof.id.as_str());
                let mut row_label = prof.id.clone();
                if let Some(hint) = &prof.account_hint {
                    row_label.push_str(&format!(" · {hint}"));
                } else if agent.is_some_and(|a| a != prof.agent) {
                    row_label.push_str(&format!(" ({})", prof.agent.label()));
                }
                let warn_row = state == ProfileState::RateLimited || auth == AuthStatus::NotSignedIn;
                if kit::menu_item(ui, is_cur.then_some(icons::CHECK), &row_label, Some(&right), warn_row).clicked() {
                    if is_cur {
                        ui.close();
                    } else if auth == AuthStatus::NotSignedIn {
                        self.actions.push(Action::LoginProfile(prof.id.clone()));
                        ui.close();
                    } else {
                        self.actions.push(Action::SetPaneProfile(pane_id, prof.id.clone()));
                        ui.close();
                    }
                }
            }
            kit::menu_sep(ui);
            let mode_text = match self.settings.failover.mode {
                FailoverMode::Auto => t("Failover: Auto-Resume"),
                FailoverMode::Manual => t("Failover: Manual Resume"),
            };
            if kit::menu_item(ui, Some(icons::LIGHTNING), mode_text, None, false).clicked() {
                self.settings.failover.mode = match self.settings.failover.mode {
                    FailoverMode::Auto => FailoverMode::Manual,
                    FailoverMode::Manual => FailoverMode::Auto,
                };
                self.settings_changed();
                self.push_companion_now();
                ui.close();
            }
            if kit::menu_item(ui, Some(icons::USER_PLUS), t("Add / Manage Accounts…"), None, false).clicked() {
                self.actions.push(Action::OpenProfiles);
                ui.close();
            }
        });
    }

    /// Inline Rate-Limit Failover & Recovery Banner rendered directly inside a paused pane.
    pub fn rate_limit_banner(&mut self, ui: &mut Ui, pane_id: PaneId, rect: Rect) {
        let p = style::pal(ui);
        let now = Instant::now();
        let Some(banner) = self.panes.get(&pane_id).and_then(|x| x.rate_limit.clone()) else {
            return;
        };
        let painter = ui.painter_at(rect.expand(1.0));
        let accent = if banner.auto_switched_to.is_some() { p.ok } else { p.rate_limited };
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("rate-limit-banner:{pane_id}"), rect);
        painter.rect(
            rect,
            CornerRadius::same(7),
            style::soft(accent, 0.11),
            Stroke::new(1.0, style::soft(accent, 0.36)),
            StrokeKind::Inside,
        );
        let cy = rect.center().y;
        let icon = if banner.auto_switched_to.is_some() { icons::LIGHTNING } else { icons::WARNING };
        painter.text(pos2(rect.left() + 12.0 + 6.0, cy), Align2::CENTER_CENTER, icon, style::icon_font(13.5), accent);

        let profiles = self.effective_profiles();
        let cross = self.settings.failover.cross_provider;
        let max_btns = if rect.width() < 420.0 { 1 } else if rect.width() < 560.0 { 2 } else { 3 };
        let mut candidates: Vec<&AgentProfile> = profiles
            .iter()
            .filter(|pr| {
                pr.enabled
                    && pr.id != banner.from_profile
                    && (cross || pr.agent == banner.agent)
                    && self.profiles_rt.get(&pr.id).is_none_or(|rt| rt.is_healthy(now))
            })
            .collect();
        candidates.sort_by_key(|pr| (pr.agent != banner.agent, pr.fallback_order, pr.id.as_str()));
        let fallbacks: Vec<String> = candidates.into_iter().take(max_btns).map(|pr| pr.id.clone()).collect();

        let inner = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), pos2(rect.right() - 6.0, rect.bottom()));
        let right_edge = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.set_clip_rect(inner.intersect(ui.clip_rect()));
                    ui.spacing_mut().item_spacing.x = 5.0;
                    if kit::icon_button_ex(ui, icons::X, t("Dismiss"), false, 20.0).clicked() {
                        self.actions.push(Action::DismissRateLimit(pane_id));
                    }
                    if banner.auto_switched_to.is_none() {
                        for id in fallbacks.iter().rev() {
                            let label = tf("Resume on {}", &[id]);
                            let b = kit::button_ex(ui, kit::Kind::Primary, Some(icons::PLAY), &label, true);
                            #[cfg(debug_assertions)]
                            crate::devtest::record(format!("failover-resume:{id}"), b.rect);
                            if b.clicked() {
                                self.actions.push(Action::ResumeWithProfile {
                                    pane: pane_id,
                                    profile_id: id.clone(),
                                });
                            }
                        }
                    }
                    ui.min_rect().left()
                },
            )
            .inner;

        let text = if let Some(to) = &banner.auto_switched_to {
            tf("{} hit limit → auto-resumed on {}", &[&banner.from_profile, to])
        } else {
            let rem_dur = banner.reset_at.saturating_duration_since(now);
            let rem = rem_dur.as_secs() + u64::from(rem_dur.subsec_nanos() > 0);
            if rem > 0 {
                tf("{} hit usage limit · resets in {}", &[&banner.from_profile, &profiles::format_countdown(rem)])
            } else {
                tf("{} hit usage limit", &[&banner.from_profile])
            }
        };
        let tx = rect.left() + 30.0;
        let room = (right_edge - 8.0 - tx).max(20.0);
        let mut job = egui::text::LayoutJob::simple_singleline(text, style::medium(12.0), p.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let g = painter.layout_job(job);
        painter.galley(pos2(tx, cy - g.size().y / 2.0), g, p.text);
    }
}
