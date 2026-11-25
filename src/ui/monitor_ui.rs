//! The usage of the agents' accounts at a glance: the "Accounts & usage" popover the sidebar's
//! usage meter opens. Manage opens Settings → Accounts.

use std::time::Instant;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, pos2, vec2};
use tc_core::agent::AgentKind;
use tc_core::model::FailoverMode;

use super::profiles_ui::{
    Meter, paint_agent_tile, paint_plan_badge, paint_profile_pill, profile_meters, profile_pill_width, short_countdown,
};
use super::settings::{SettingsPage, galley};
use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};
use crate::profiles::{self, AuthStatus};

/// The popover's width.
const POPOVER_W: f32 = 360.0;

fn popover_meter_color(pct: f32, p: &style::Palette) -> Color32 {
    if pct >= 90.0 {
        p.rate_limited
    } else if pct >= 70.0 {
        p.warn
    } else {
        p.accent
    }
}

/// Paints one clean, right-aligned meter row inside a nested popover account card.
/// - Percentage meters (Claude, Codex): fixed left label (`5h`/`Wk`/`30d`), crisp progress track
///   filling exact remaining width, and flush-right `pct%` + `↻ reset` column.
/// - Local telemetry meters (Antigravity): fixed left label, tabular tokens on left-middle, and
///   flush-right turns count so every card shares the exact same visual grid.
fn paint_popover_meter(
    painter: &egui::Painter,
    rect: Rect,
    m: &Meter,
    p: &style::Palette,
) {
    let font = style::mono(style::DATA);
    let cy = rect.center().y;
    let label_col = 28.0;

    // Left window label (`5h`, `Wk`, `30d`).
    let lg = painter.layout_no_wrap(m.label.clone(), font.clone(), p.text_faint);
    painter.galley(pos2(rect.left(), cy - lg.size().y / 2.0), lg, p.text_faint);

    if let Some(pct) = m.pct {
        // Measure flush-right metadata (`10%  ↻ 38m`) so every bar ends cleanly on a consistent edge.
        let pct_color = if pct >= 70.0 { popover_meter_color(pct, p) } else { p.text };
        let pct_galley = painter.layout_no_wrap(format!("{pct:.0}%"), font.clone(), pct_color);
        let reset_galley = m.reset.as_ref().map(|r| painter.layout_no_wrap(r.clone(), font.clone(), p.text_faint));
        let reset_w = reset_galley.as_ref().map_or(0.0, |g| 8.0 + 11.0 + 3.0 + g.size().x);
        let right_w = pct_galley.size().x.max(24.0) + reset_w;

        let bar_left = rect.left() + label_col + 6.0;
        let bar_right = (rect.right() - right_w - 10.0).max(bar_left + 32.0);
        let bar_h = 5.0;
        let bar = Rect::from_min_max(pos2(bar_left, cy - bar_h / 2.0), pos2(bar_right, cy + bar_h / 2.0));
        let radius = CornerRadius::same(3);
        painter.rect_filled(bar, radius, p.subtle);
        let frac = (pct / 100.0).clamp(0.0, 1.0);
        if frac > 0.0 {
            let fw = (bar.width() * frac).max(bar_h);
            painter.rect_filled(
                Rect::from_min_size(bar.min, vec2(fw, bar_h)),
                radius,
                popover_meter_color(pct, p),
            );
        }

        // Paint right-aligned `pct%` and optional `↻ countdown`.
        let mut rx = rect.right();
        if let Some(rg) = reset_galley {
            rx -= rg.size().x;
            painter.galley(pos2(rx, cy - rg.size().y / 2.0), rg, p.text_faint);
            rx -= 3.0 + 11.0;
            painter.text(pos2(rx + 5.5, cy), Align2::CENTER_CENTER, icons::ARROW_COUNTER_CLOCKWISE, style::icon_font(10.5), p.text_faint);
            rx -= 8.0;
        }
        rx -= pct_galley.size().x;
        painter.galley(pos2(rx, cy - pct_galley.size().y / 2.0), pct_galley, pct_color);
    } else {
        // Local-telemetry-only account (e.g. Antigravity CLI): clean tabular tokens + flush-right turns.
        let tok_text = m.tokens.clone().unwrap_or_else(|| "—".to_string());
        let tg = painter.layout_no_wrap(tok_text, font.clone(), p.text_muted);
        painter.galley(pos2(rect.left() + label_col + 6.0, cy - tg.size().y / 2.0), tg, p.text_muted);

        if m.turns > 0 {
            let turns_text = tf("{} turns", &[&m.turns.to_string()]);
            let ng = painter.layout_no_wrap(turns_text, font, p.text_faint);
            painter.galley(pos2(rect.right() - ng.size().x, cy - ng.size().y / 2.0), ng, p.text_faint);
        }
    }
}

fn meter_hover_tip(
    agent: AgentKind,
    m: &Meter,
    fetched_at_unix: Option<i64>,
    now_unix: i64,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(pct) = m.pct {
        parts.push(format!("{} {pct:.0}%", m.label));
    }
    if let Some(tok) = &m.tokens {
        parts.push(tok.clone());
    }
    if m.turns > 0 {
        parts.push(tf("{} turns", &[&m.turns.to_string()]));
    }
    if agent == AgentKind::Claude && m.pct.is_some() {
        if let Some(ts) = fetched_at_unix {
            let age = now_unix.saturating_sub(ts).max(0) as u64;
            let age_label = if age < 60 { format!("{age}s") } else { short_countdown(age) };
            parts.push(tf("Server quota synced {} ago · run /usage in Claude to refresh", &[&age_label]));
        }
    } else if m.pct.is_none() {
        parts.push(t("Live local usage from logs (server quota % not exposed by CLI)").to_string());
    }
    parts.join(" · ")
}

impl App {
    /// The "Accounts & usage" popover, next to `monitor_pill_rect`: each account inside a clean
    /// Slate card with its plan, consistent status pill, and aligned usage meters, followed by
    /// the failover switch.
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
        let h = ctx.data(|d| d.get_temp::<f32>(size_id)).unwrap_or(280.0);
        let x = if anchor.center().x < screen.center().x { anchor.left() } else { anchor.right() - w };
        let x = x.clamp(12.0, (screen.right() - w - 12.0).max(12.0));
        let below = anchor.bottom() + 6.0;
        let y = if below + h <= screen.bottom() - 12.0 || anchor.top() - 6.0 - h < 12.0 { below } else { anchor.top() - 6.0 - h };
        let y = y.clamp(screen.top() + 12.0, (screen.bottom() - h - 12.0).max(screen.top() + 12.0));
        let max_scroll_h = (screen.height() - 140.0).clamp(160.0, 540.0);
        let now = Instant::now();
        let now_unix = profiles::now_unix_secs();
        let profiles = self.effective_profiles();
        let cur_agent_pane = self.current_pane().filter(|pid| self.panes.get(pid).is_some_and(|x| x.alive() && x.tracker.agent.is_some_and(profiles::supports_profile)));
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
                    .inner_margin(egui::Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(w - 28.0);
                        ui.spacing_mut().item_spacing.y = 12.0;

                        // Header: title on left, subtle refresh + Manage on right.
                        ui.horizontal(|ui| {
                            let g = galley(ui, t("Accounts & usage"), style::semibold(style::HEADING), p.text, None, 20.0);
                            let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                            ui.painter().galley(r.min, g, p.text);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let m = kit::button_ex(ui, kit::Kind::Ghost, None, t("Manage"), true);
                                #[cfg(debug_assertions)]
                                crate::devtest::record("usage-manage", m.rect);
                                if m.clicked() {
                                    self.open_settings_at(SettingsPage::Accounts);
                                }
                                let rf = kit::icon_button_ex(ui, icons::ARROWS_CLOCKWISE, t("Refresh usage"), false, 24.0);
                                if rf.clicked() {
                                    self.usage_scan_at = None;
                                }
                            });
                        });

                        // Nested Slate account cards (clean separation, aligned meters, zero "Active in" clutter).
                        egui::ScrollArea::vertical()
                            .id_salt("monitor-popover-scroll")
                            .max_height(max_scroll_h)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 8.0;
                                for prof in &profiles {
                                    let rt = self.profiles_rt.get(&prof.id).cloned().unwrap_or_default();
                                    let auth = rt.auth_status(prof);
                                    let meters = profile_meters(prof, &rt, now_unix, now, true);
                                    let card_fill = if dark { p.hover } else { p.surface };
                                    egui::Frame::NONE
                                        .fill(card_fill)
                                        .stroke(Stroke::new(1.0, p.border))
                                        .corner_radius(CornerRadius::same(9))
                                        .inner_margin(egui::Margin::symmetric(11, 9))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.spacing_mut().item_spacing.y = 7.0;

                                            // Row 1: Agent logo, account id, plan pill … Switch + consistent state pill.
                                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
                                            let cy = r.center().y;
                                            let pw = profile_pill_width(ui.painter(), rt.state, prof.enabled, &p);
                                            let pill = paint_profile_pill(ui.painter(), pos2(r.right() - pw, cy), rt.state, prof.enabled, &p, dark);
                                            let mut right_limit = pill.left() - 6.0;
                                            if let Some(pane_id) = cur_agent_pane
                                                && prof.enabled
                                                && cur_pane_profile.as_deref() != Some(prof.id.as_str())
                                                && auth != AuthStatus::NotSignedIn
                                                && rt.is_healthy(now)
                                            {
                                                let bw = kit::button_width(ui, Some(icons::ARROWS_CLOCKWISE), t("Switch"), None, true);
                                                let br = Rect::from_min_size(pos2(pill.left() - 6.0 - bw, cy - 11.0), vec2(bw, 22.0));
                                                right_limit = br.left() - 6.0;
                                                let b = ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| {
                                                    kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROWS_CLOCKWISE), t("Switch"), true)
                                                }).inner;
                                                if b.on_hover_text(tf("Resume the pane in view on {}", &[&prof.id])).clicked() {
                                                    self.actions.push(Action::ResumeWithProfile { pane: pane_id, profile_id: prof.id.clone() });
                                                    self.monitor_popover = false;
                                                }
                                            }
                                            let left_clip = Rect::from_min_max(r.min, pos2(right_limit.max(r.left()), r.bottom())).intersect(ui.clip_rect());
                                            let lp = ui.painter().with_clip_rect(left_clip);
                                            let av = Rect::from_min_size(pos2(r.left(), cy - 9.0), vec2(18.0, 18.0));
                                            paint_agent_tile(&lp, av, Some(prof.agent), "", &p);
                                            let idc = if prof.enabled { p.text } else { p.text_faint };
                                            let ig = galley(ui, &prof.id, style::mono_semibold(style::DATA), idc, None, 14.0);
                                            let mut lx = av.right() + 8.0;
                                            let iw = ig.size().x;
                                            lp.galley(pos2(lx, cy - 7.0), ig, idc);
                                            lx += iw + 7.0;
                                            if let Some(plan) = rt.effective_plan() {
                                                lx = paint_plan_badge(&lp, pos2(lx, cy), plan, 16.0, &p).right() + 7.0;
                                            }
                                            if auth == AuthStatus::NotSignedIn {
                                                let g = galley(ui, t("Not signed in"), FontId::proportional(style::SMALL), p.warn, None, 16.0);
                                                lp.galley(pos2(lx, cy - 8.0), g, p.warn);
                                            }

                                            // Row 2..N: Aligned usage windows with rich real-time tooltips on hover.
                                            if meters.is_empty() {
                                                let g = galley(ui, t("No usage reported yet"), style::mono(style::DATA), p.text_ghost, None, 14.0);
                                                let (mr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
                                                ui.painter().galley(pos2(mr.left(), mr.top()), g, p.text_ghost);
                                            } else {
                                                for m in &meters {
                                                    let (mr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 15.0), Sense::hover());
                                                    let painter = ui.painter().with_clip_rect(mr.expand(2.0).intersect(ui.clip_rect()));
                                                    paint_popover_meter(&painter, mr, m, &p);
                                                    let tip = meter_hover_tip(prof.agent, m, rt.usage.quota_fetched_at_unix, now_unix);
                                                    if !tip.is_empty() {
                                                        resp.on_hover_text(tip);
                                                    }
                                                }
                                            }
                                        });
                                }
                            });

                        // Subtle divider + Failover switch footer.
                        let (div_r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                        ui.painter().rect_filled(div_r, CornerRadius::ZERO, p.border);

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
