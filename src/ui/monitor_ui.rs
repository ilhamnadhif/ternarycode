//! The usage of the agents' accounts at a glance: the "Accounts & usage" popover the sidebar's
//! usage meter opens. Manage opens Settings → Accounts.

use std::time::Instant;

use eframe::egui::{self, Align2, CornerRadius, FontId, Rect, Sense, Stroke, pos2, vec2};
use tc_core::model::FailoverMode;

use super::profiles_ui::{paint_agent_tile, paint_meter, paint_plan_badge, paint_profile_pill, profile_meters, profile_pill_width};
use super::settings::{SettingsPage, galley};
use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};
use crate::profiles::{self, AuthStatus};

/// The popover's width.
const POPOVER_W: f32 = 340.0;

impl App {
    /// The "Accounts & usage" popover, next to `monitor_pill_rect`: each account with its plan,
    /// its state and its 5-hour and weekly meters, then the failover switch. Below the anchor
    /// when there is room, else above it; aligned to its left edge in the left half of the
    /// window, to its right edge in the right half.
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
        // Countdowns tick while it is open.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        let screen = ctx.content_rect();
        let w = POPOVER_W.min(screen.width() - 24.0);
        let size_id = egui::Id::new("monitor-popover-height");
        let h = ctx.data(|d| d.get_temp::<f32>(size_id)).unwrap_or(260.0);
        let x = if anchor.center().x < screen.center().x { anchor.left() } else { anchor.right() - w };
        let x = x.clamp(12.0, (screen.right() - w - 12.0).max(12.0));
        let below = anchor.bottom() + 6.0;
        let y = if below + h <= screen.bottom() - 12.0 || anchor.top() - 6.0 - h < 12.0 { below } else { anchor.top() - 6.0 - h };
        let max_scroll_h = (screen.height() - 140.0).clamp(160.0, 520.0);
        let now = Instant::now();
        let now_unix = profiles::now_unix_secs();
        let profiles = self.effective_profiles();
        let cur_agent_pane = self.current_pane().filter(|pid| self.panes.get(pid).is_some_and(|x| x.alive() && x.tracker.agent.is_some_and(profiles::supports_multi_profile)));
        let cur_pane_profile = cur_agent_pane.and_then(|pid| {
            let pane = self.panes.get(&pid)?;
            let agent = pane.tracker.agent?;
            self.resolved_pane_profile_id(pane, agent, &profiles)
        });

        let area_resp = egui::Area::new(egui::Id::new("monitor-popover"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(x, y))
            .show(ctx, |ui| {
                let p = style::pal(ui);
                let dark = ui.visuals().dark_mode;
                egui::Frame::NONE
                    .fill(p.menu)
                    .stroke(Stroke::new(1.0, p.border_strong))
                    .corner_radius(CornerRadius::same(12))
                    .shadow(ui.visuals().popup_shadow)
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width(w - 32.0);
                        ui.spacing_mut().item_spacing.y = 14.0;
                        // Title and Manage.
                        ui.horizontal(|ui| {
                            let g = galley(ui, t("Accounts & usage"), style::semibold(style::HEADING), p.text, None, 20.0);
                            let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                            ui.painter().galley(r.min, g, p.text);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let m = kit::button_ex(ui, kit::Kind::Ghost, None, t("Manage"), true);
                                #[cfg(debug_assertions)]
                                crate::devtest::record("usage-manage", m.rect);
                                if m.clicked() {
                                    self.open_settings_at(SettingsPage::Accounts);
                                }
                            });
                        });
                        egui::ScrollArea::vertical()
                            .id_salt("monitor-popover-scroll")
                            .max_height(max_scroll_h)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 14.0;
                                for prof in &profiles {
                                    let rt = self.profiles_rt.get(&prof.id).cloned().unwrap_or_default();
                                    let auth = rt.auth_status(prof);
                                    let meters = profile_meters(prof, &rt, now_unix, now, false);
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 6.0;
                                        // Avatar, id, plan … state (and Switch, for the pane in view).
                                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
                                        let cy = r.center().y;
                                        let av = Rect::from_min_size(pos2(r.left(), cy - 8.0), vec2(16.0, 16.0));
                                        paint_agent_tile(ui.painter(), av, Some(prof.agent), "", &p);
                                        let idc = if prof.enabled { p.text } else { p.text_faint };
                                        let ig = galley(ui, &prof.id, style::mono_semibold(style::DATA), idc, None, 14.0);
                                        let mut lx = av.right() + 8.0;
                                        let iw = ig.size().x;
                                        ui.painter().galley(pos2(lx, cy - 7.0), ig, idc);
                                        lx += iw + 8.0;
                                        if let Some(plan) = rt.usage.plan_type.as_deref().filter(|s| !s.is_empty()) {
                                            lx = paint_plan_badge(ui.painter(), pos2(lx, cy), plan, 16.0, &p).right() + 8.0;
                                        }
                                        if auth == AuthStatus::NotSignedIn {
                                            let g = galley(ui, t("Not signed in"), FontId::proportional(style::SMALL), p.warn, None, 16.0);
                                            ui.painter().galley(pos2(lx, cy - 8.0), g, p.warn);
                                        }
                                        let pw = profile_pill_width(ui.painter(), rt.state, prof.enabled, &p);
                                        let pill = paint_profile_pill(ui.painter(), pos2(r.right() - pw, cy), rt.state, prof.enabled, &p, dark);
                                        if let Some(pane_id) = cur_agent_pane
                                            && prof.enabled
                                            && cur_pane_profile.as_deref() != Some(prof.id.as_str())
                                            && auth != AuthStatus::NotSignedIn
                                            && rt.is_healthy(now)
                                        {
                                            let bw = kit::button_width(ui, Some(icons::ARROWS_CLOCKWISE), t("Switch"), None, true);
                                            let br = Rect::from_min_size(pos2(pill.left() - 6.0 - bw, cy - 12.0), vec2(bw, 24.0));
                                            let b = ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROWS_CLOCKWISE), t("Switch"), true)).inner;
                                            if b.on_hover_text(tf("Resume the pane in view on {}", &[&prof.id])).clicked() {
                                                self.actions.push(Action::ResumeWithProfile { pane: pane_id, profile_id: prof.id.clone() });
                                                self.monitor_popover = false;
                                            }
                                        }
                                        // The meters: 5h and the week.
                                        if meters.is_empty() {
                                            let g = galley(ui, t("No usage reported yet"), style::mono(style::DATA), p.text_ghost, None, 14.0);
                                            let (mr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
                                            ui.painter().galley(pos2(mr.left() + 24.0, mr.top()), g, p.text_ghost);
                                        }
                                        for m in &meters {
                                            let (mr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
                                            let painter = ui.painter().with_clip_rect(mr.expand(2.0).intersect(ui.clip_rect()));
                                            let bar_w = (mr.width() - 24.0 - 8.0 - 8.0 - 36.0 - if m.reset.is_some() { 8.0 + 15.0 + 44.0 } else { 0.0 }).clamp(60.0, 190.0);
                                            paint_meter(&painter, pos2(mr.left(), mr.center().y), m, bar_w, 4.0, 0.0, false, &p);
                                        }
                                        // Where it runs now: a click goes there.
                                        if let Some((pane_id, proj_id)) = rt.active_panes.first().copied() {
                                            let g = galley(ui, &tf("Active in {}", &[&self.project_label(proj_id)]), FontId::proportional(style::SMALL), p.accent_text, None, 16.0);
                                            let (lr, link) = ui.allocate_exact_size(g.size(), Sense::click());
                                            ui.painter().galley(lr.min, g, p.accent_text);
                                            if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                                self.actions.push(Action::FocusPane(pane_id));
                                                self.monitor_popover = false;
                                            }
                                        }
                                    });
                                }
                            });
                        // Failover: switch automatically.
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let (ir, _) = ui.allocate_exact_size(vec2(14.0, 16.0), Sense::hover());
                            ui.painter().text(ir.center(), Align2::CENTER_CENTER, icons::LIGHTNING, style::icon_font(14.0), p.text_muted);
                            let g = galley(ui, t("Failover: switch automatically"), FontId::proportional(style::SMALL), p.text_muted, None, 16.0);
                            let (gr, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                            ui.painter().galley(gr.min, g, p.text_muted);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let mut auto = self.settings.failover.mode == FailoverMode::Auto;
                                if kit::switch(ui, &mut auto).changed() {
                                    self.settings.failover.mode = if auto { FailoverMode::Auto } else { FailoverMode::Manual };
                                    self.settings_changed();
                                    self.push_companion_now();
                                }
                            });
                        });
                    });
            });

        let rect = area_resp.response.rect;
        if (rect.height() - h).abs() > 0.5 {
            ctx.data_mut(|d| d.insert_temp(size_id, rect.height()));
            ctx.request_repaint();
        }
        #[cfg(debug_assertions)]
        crate::devtest::record("monitor-popover", rect);
        // Close on click outside both the popover and its anchor.
        let outside = on_top
            && ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pt| !rect.contains(pt) && !anchor.contains(pt)));
        if outside {
            self.monitor_popover = false;
        }
    }
}
