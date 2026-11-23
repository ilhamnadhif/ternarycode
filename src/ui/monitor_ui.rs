//! Live Telemetry Pill, Quick-Glance Usage Popover, and Shared Usage/Session Sections
//! embedded inside the unified Agent Profiles & Usage page (`View::Profiles`).

use std::time::Instant;

use eframe::egui::{self, Align2, CornerRadius, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use tc_core::status::Status;

use super::profiles_ui::{paint_window_meter, profile_state_color, profile_state_label};
use super::{agent_logo, icons, kit, status, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};
use crate::profiles::{self, AuthStatus, LogLevel};

impl App {
    /// Compact top-bar telemetry pill showing live aggregate agent/profile health & real token usage.
    pub fn telemetry_pill(&mut self, ui: &mut Ui) {
        self.telemetry_pill_sized(ui, false);
    }

    pub fn telemetry_pill_sized(&mut self, ui: &mut Ui, compact: bool) {
        let p = style::pal(ui);
        let (running, limited, errors, tokens) = self.telemetry_summary();
        let dot_color = if errors > 0 {
            p.err
        } else if limited > 0 {
            p.rate_limited
        } else if running > 0 {
            p.info
        } else {
            p.ok
        };
        let summary = if compact {
            if limited > 0 {
                format!("{limited}!")
            } else if running > 0 {
                if tokens > 0 {
                    format!("{running} · {}", profiles::format_tokens(tokens))
                } else {
                    format!("{running}")
                }
            } else if tokens > 0 {
                profiles::format_tokens(tokens)
            } else {
                t("Idle").to_string()
            }
        } else if limited > 0 {
            tf("{} limited · {} run", &[&limited.to_string(), &running.to_string()])
        } else if running > 0 {
            if tokens > 0 {
                format!("{} · {}", tf("{} running", &[&running.to_string()]), profiles::format_tokens(tokens))
            } else {
                tf("{} running", &[&running.to_string()])
            }
        } else if tokens > 0 {
            format!("{} · {}", t("Idle"), profiles::format_tokens(tokens))
        } else {
            t("Monitor").to_string()
        };

        let g = ui.painter().layout_no_wrap(summary, style::mono(11.0), p.text);
        let w = 10.0 + 6.0 + 6.0 + g.size().x + 6.0 + 10.0 + 8.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
        #[cfg(debug_assertions)]
        crate::devtest::record("telemetry-pill", rect);
        self.monitor_pill_rect = Some(rect);

        let open = self.monitor_popover || matches!(self.view, tc_core::model::View::Profiles | tc_core::model::View::Monitor);
        let fill = if open { p.surface } else if resp.hovered() { p.hover } else { egui::Color32::TRANSPARENT };
        let stroke = Stroke::new(1.0, if open || limited > 0 { style::soft(dot_color, 0.45) } else { p.border_strong });
        ui.painter().rect(rect, CornerRadius::same(style::RADIUS), fill, stroke, StrokeKind::Inside);
        let cy = rect.center().y;
        ui.painter().circle_filled(pos2(rect.left() + 10.0 + 3.0, cy), 3.0, dot_color);
        let gw = g.size().x;
        ui.painter().galley(pos2(rect.left() + 22.0, cy - g.size().y / 2.0), g, p.text);
        ui.painter().text(pos2(rect.left() + 22.0 + gw + 6.0 + 5.0, cy), Align2::CENTER_CENTER, icons::CARET_DOWN, style::icon_font(9.5), p.text_faint);

        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.on_hover_text(t("All-Agent Usage & Status Monitor")).clicked() {
            self.monitor_popover = !self.monitor_popover;
            if self.monitor_popover {
                self.attention_open = false;
                self.palette = None;
                self.dialog = None;
            }
        }
    }

    /// Expandable telemetry popover anchored under the top-bar pill.
    pub fn monitor_popover_ui(&mut self, ctx: &egui::Context) {
        if !self.monitor_popover {
            return;
        }
        let Some(anchor) = self.monitor_pill_rect else {
            self.monitor_popover = false;
            return;
        };
        let on_top = self.dialog.is_none() && self.palette.is_none() && !self.updater.dialog_open;
        if on_top && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.monitor_popover = false;
            return;
        }

        let screen = ctx.content_rect();
        let w = 390.0_f32.min(screen.width() - 24.0);
        let pos = pos2((anchor.right() - w).clamp(12.0, (screen.right() - w - 12.0).max(12.0)), anchor.bottom() + 6.0);
        let max_scroll_h = (screen.bottom() - pos.y - 48.0).clamp(180.0, 460.0);
        let now = Instant::now();
        let now_unix = profiles::now_unix_secs();
        let profiles = self.effective_profiles();
        let cur_agent_pane = self.current_pane().filter(|pid| {
            self.panes
                .get(pid)
                .is_some_and(|x| x.alive() && x.tracker.agent.is_some_and(profiles::supports_multi_profile))
        });
        let cur_pane_profile = cur_agent_pane.and_then(|pid| {
            let pane = self.panes.get(&pid)?;
            let agent = pane.tracker.agent?;
            self.resolved_pane_profile_id(pane, agent, &profiles)
        });

        let area_resp = egui::Area::new(egui::Id::new("monitor-popover"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                let p = style::pal(ui);
                let dark = ui.visuals().dark_mode;
                egui::Frame::NONE
                    .fill(p.overlay)
                    .stroke(Stroke::new(1.0, p.border_strong))
                    .corner_radius(CornerRadius::same(10))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 12],
                        blur: 32,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(if dark { 140 } else { 36 }),
                    })
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(w - 24.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(t("Agent Profiles & Usage")).font(style::semibold(13.0)).color(p.text));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROWS_OUT_SIMPLE), t("Manage"), true).clicked() {
                                    self.actions.push(Action::OpenProfiles);
                                    self.monitor_popover = false;
                                }
                            });
                        });
                        ui.add_space(6.0);
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                        ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, p.border));
                        ui.add_space(6.0);

                        let render_rows = |this: &mut Self, ui: &mut Ui| {
                            for (i, prof) in profiles.iter().enumerate() {
                                if i > 0 {
                                    ui.add_space(2.0);
                                    let (sep_r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                                    ui.painter().hline(sep_r.x_range(), sep_r.center().y, Stroke::new(1.0, p.border));
                                    ui.add_space(6.0);
                                }
                                let rt = this.profiles_rt.get(&prof.id).cloned().unwrap_or_default();
                                let sc = if prof.enabled { profile_state_color(rt.state, &p) } else { p.idle };
                                let badge_label = if prof.enabled { profile_state_label(rt.state) } else { t("Disabled") };
                                let auth = rt.auth_status(prof);
                                let used_5h = rt.quota_5h_used_pct(prof);
                                let used_wk = rt.quota_weekly_used_pct(prof);
                                let reset_5h = rt.window_5h_reset_secs(now_unix, now);
                                let reset_wk = rt.window_weekly_reset_secs(now_unix);
                                let l5 = rt.window_5h_label();
                                let lw = if rt.window_weekly_label() == "Weekly" { "Wk".to_string() } else { rt.window_weekly_label() };

                                egui::Sides::new().spacing(8.0).shrink_left().show(
                                    ui,
                                    |ui| {
                                        ui.spacing_mut().item_spacing.x = 7.0;
                                        let (mr, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                                        if !agent_logo::paint(ui.painter(), mr, prof.agent, if prof.enabled { p.text } else { p.text_faint }, false) {
                                            ui.painter().text(mr.center(), Align2::CENTER_CENTER, icons::TERMINAL_WINDOW, style::icon_font(12.0), p.text_muted);
                                        }
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&prof.id)
                                                    .font(style::mono_semibold(11.5))
                                                    .color(if prof.enabled { p.text } else { p.text_faint }),
                                            )
                                            .truncate(),
                                        );
                                        if let Some(plan) = &rt.usage.plan_type {
                                            kit::status_badge(ui, &plan.to_uppercase(), p.accent_text, true);
                                        }
                                        kit::status_badge(ui, badge_label, sc, false);
                                        if auth == AuthStatus::NotSignedIn {
                                            ui.label(RichText::new(t("Not signed in")).size(10.0).color(p.warn));
                                        }
                                    },
                                    |ui| {
                                        if let Some(pane_id) = cur_agent_pane
                                            && prof.enabled
                                            && cur_pane_profile.as_deref() != Some(prof.id.as_str())
                                            && auth != AuthStatus::NotSignedIn
                                            && rt.is_healthy(now)
                                            && kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROWS_CLOCKWISE), t("Switch"), true).clicked()
                                        {
                                            this.actions.push(Action::ResumeWithProfile { pane: pane_id, profile_id: prof.id.clone() });
                                            this.monitor_popover = false;
                                        }
                                    },
                                );
                                ui.add_space(2.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(20.0);
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        if rt.has_window_5h(prof) {
                                            paint_window_meter(ui, &l5, used_5h, rt.usage.window_5h.tokens.total(), reset_5h, 68.0, &p);
                                        }
                                        if rt.has_window_weekly(prof) {
                                            paint_window_meter(ui, &lw, used_wk, rt.usage.window_weekly.tokens.total(), reset_wk, 68.0, &p);
                                        }
                                    });
                                });
                                if let Some((pane_id, proj_id)) = rt.active_panes.first().copied() {
                                    let pname = this.project_label(proj_id);
                                    ui.horizontal(|ui| {
                                        ui.add_space(20.0);
                                        let link = ui.add(
                                            egui::Label::new(
                                                RichText::new(tf("Active in {}", &[&pname]))
                                                    .size(11.0)
                                                    .color(p.accent_text),
                                            )
                                            .sense(Sense::click()),
                                        );
                                        if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                            this.actions.push(Action::FocusPane(pane_id));
                                            this.monitor_popover = false;
                                        }
                                    });
                                }
                                ui.add_space(4.0);
                            }
                        };
                        egui::ScrollArea::vertical()
                            .id_salt("monitor-popover-scroll")
                            .min_scrolled_height(max_scroll_h)
                            .max_height(max_scroll_h)
                            .auto_shrink([false, true])
                            .show(ui, |ui| render_rows(self, ui));
                    });
            });

        #[cfg(debug_assertions)]
        crate::devtest::record("monitor-popover", area_resp.response.rect);
        // Close on click outside both the popover and its anchor pill.
        let outside = on_top
            && ctx.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer.interact_pos().is_some_and(|pt| !area_resp.response.rect.contains(pt) && !anchor.contains(pt))
            });
        if outside {
            self.monitor_popover = false;
        }
    }

    /// Compact tabular KPI summary strip (embedded at the top of `View::Profiles`).
    pub(super) fn usage_summary_strip(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let now = Instant::now();
        let profiles = self.effective_profiles();
        let (running, limited, _errors, _) = self.telemetry_summary();
        let (total_5h, total_weekly) = self.deduped_window_totals();
        let enabled_count = profiles.iter().filter(|pr| pr.enabled).count();
        let healthy = profiles
            .iter()
            .filter(|pr| pr.enabled && self.profiles_rt.get(&pr.id).is_none_or(|r| r.is_healthy(now)))
            .count();

        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, if dark { p.border } else { p.border_strong }))
            .corner_radius(CornerRadius::same(9))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::Sides::new().spacing(12.0).shrink_left().wrap().show(
                    ui,
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(26.0, 8.0);
                            metric_cell(ui, t("Running Agents"), &running.to_string(), if running > 0 { p.info } else { p.text });
                            metric_cell(ui, t("Healthy Profiles"), &format!("{healthy} / {enabled_count}"), p.ok);
                            metric_cell(ui, t("Rate-Limited"), &limited.to_string(), if limited > 0 { p.rate_limited } else { p.text_muted });
                            metric_cell(ui, t("5-Hour Tokens"), &profiles::format_tokens(total_5h), p.text);
                            metric_cell(ui, t("Weekly Tokens"), &profiles::format_tokens(total_weekly), p.text);
                        });
                    },
                    |ui| {
                        if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROW_CLOCKWISE), t("Refresh"), true).clicked() {
                            self.usage_scan_at = None;
                        }
                    },
                );
            });
    }

    /// Active agent sessions and recent failover/agent event log (embedded at the bottom of `View::Profiles`).
    pub(super) fn active_sessions_and_events(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;

        ui.label(RichText::new(t("Active Sessions")).font(style::semibold(13.0)).color(p.text));
        ui.add_space(6.0);
        let mut active_panes: Vec<_> = self
            .panes
            .values()
            .filter(|x| x.alive() && (x.tracker.agent.is_some() || x.active_profile.is_some() || !matches!(x.tracker.status, Status::Idle | Status::Ended)))
            .map(|x| {
                (
                    x.id,
                    self.project_label(x.project),
                    x.label(),
                    x.active_profile.clone(),
                    x.tracker.status,
                    status::age(x.tracker.since),
                    x.task.clone().or_else(|| x.tracker.message.clone()),
                )
            })
            .collect();
        active_panes.sort_by_key(|x| x.0);

        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, if dark { p.border } else { p.border_strong }))
            .corner_radius(CornerRadius::same(9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if active_panes.is_empty() {
                    egui::Frame::NONE.inner_margin(egui::Margin::same(14)).show(ui, |ui| {
                        ui.label(RichText::new(t("No active agent sessions right now.")).size(12.0).color(p.text_faint));
                    });
                } else {
                    for (i, (pane_id, project, label, prof, st, age, task)) in active_panes.into_iter().enumerate() {
                        if i > 0 {
                            let y = ui.cursor().top();
                            ui.painter().hline(ui.max_rect().x_range(), y + 0.5, Stroke::new(1.0, p.border));
                        }
                        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, 9)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            egui::Sides::new().spacing(10.0).shrink_left().wrap().show(
                                ui,
                                |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing = vec2(10.0, 4.0);
                                        ui.label(RichText::new(&project).font(style::medium(12.5)).color(p.text));
                                        ui.label(RichText::new(&label).size(12.0).color(p.text_muted));
                                        if let Some(pr) = prof {
                                            kit::status_badge(ui, &pr, p.accent_text, true);
                                        }
                                        kit::status_badge(ui, &status::text(st), status::color(st, &p), false);
                                        ui.label(RichText::new(age).font(style::mono(10.5)).color(p.text_faint));
                                        if let Some(ttext) = task {
                                            ui.add(egui::Label::new(RichText::new(ttext).size(11.5).color(p.text_faint)).wrap());
                                        }
                                    });
                                },
                                |ui| {
                                    if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::TERMINAL_WINDOW), t("Open"), true).clicked() {
                                        self.actions.push(Action::FocusPane(pane_id));
                                    }
                                },
                            );
                        });
                    }
                }
            });

        ui.add_space(18.0);
        ui.label(RichText::new(t("Recent Failover & Agent Events")).font(style::semibold(13.0)).color(p.text));
        ui.add_space(6.0);
        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, if dark { p.border } else { p.border_strong }))
            .corner_radius(CornerRadius::same(9))
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.agent_logs.is_empty() {
                    ui.label(RichText::new(t("No agent events recorded yet.")).size(12.0).color(p.text_faint));
                } else {
                    for entry in self.agent_logs.iter().rev().take(20) {
                        let c = match entry.level {
                            LogLevel::Failover => p.ok,
                            LogLevel::Warn => p.warn,
                            LogLevel::Err => p.err,
                            LogLevel::Tool => p.info,
                            LogLevel::Info => p.text_muted,
                        };
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(8.0, 2.0);
                            ui.label(RichText::new(entry.at.format("%H:%M:%S").to_string()).font(style::mono(10.5)).color(p.text_ghost));
                            if let Some(pr) = &entry.profile {
                                ui.label(RichText::new(pr).font(style::mono_medium(10.5)).color(p.accent_text));
                            }
                            ui.add(egui::Label::new(RichText::new(&entry.message).size(12.0).color(c)).wrap());
                        });
                    }
                }
            });
    }
}

fn metric_cell(ui: &mut Ui, label: &str, value: &str, color: egui::Color32) {
    let p = style::pal(ui);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(label).size(11.0).color(p.text_faint));
        ui.label(RichText::new(value).font(style::mono_semibold(16.0)).color(color));
    });
}
