//! Agent accounts (profiles): Settings → Accounts (their usage, the failover policy, the
//! accounts in failover order, the account editor), plus the account switcher in agent panes'
//! headers and the rate-limit banner inside a paused pane.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use tc_core::agent::AgentKind;
use tc_core::model::{AgentProfile, FailoverMode};
use tc_core::status::Status;
use tc_core::PaneId;

use super::settings::{any_hit, card, galley, gap, group_head, hit, mono_field, raised, section, text};
use super::{agent_logo, icons, kit, status, style};
use crate::actions::Action;
use crate::ade::ProfileDraft;
use crate::app::App;
use crate::i18n::{t, tf};
use crate::profiles::{self, AuthStatus, ConnectionTest, LogLevel, ProfileState};

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

/// An agent's avatar (16, 20 or 28 square): its logo on the active fill, or `letters` (a
/// terminal when there are none).
pub(super) fn paint_agent_tile(painter: &egui::Painter, rect: Rect, agent: Option<AgentKind>, letters: &str, p: &style::Palette) {
    let s = rect.width();
    let r = if s >= 26.0 { 7 } else if s >= 19.0 { 5 } else { 4 };
    painter.rect_filled(rect, CornerRadius::same(r), p.subtle);
    let m = (s * 0.68).round();
    let logo = Rect::from_center_size(rect.center(), vec2(m, m));
    if agent.is_some_and(|a| agent_logo::paint(painter, logo, a, p.text, false)) {
        return;
    }
    if letters.is_empty() {
        painter.text(rect.center(), Align2::CENTER_CENTER, icons::TERMINAL, style::icon_font((s * 0.72).round()), p.text_muted);
    } else {
        painter.text(rect.center(), Align2::CENTER_CENTER, letters, style::mono_semibold((s * 0.4).round()), p.text_muted);
    }
}

/// An account's state as a pill: In use (running), Ready, Limited, Cooling down, Error, or Off
/// when it is left out of the failover order.
enum Pill {
    State(Status, &'static str),
    Icon(&'static str, &'static str, Color32),
}

fn profile_pill(state: ProfileState, enabled: bool, p: &style::Palette) -> Pill {
    if !enabled {
        return Pill::State(Status::Idle, t("Off"));
    }
    match state {
        ProfileState::Running => Pill::State(Status::Running, t("In use")),
        ProfileState::Idle => Pill::State(Status::Done, t("Ready")),
        ProfileState::RateLimited => Pill::Icon(icons::HOURGLASS_MEDIUM, t("Limited"), p.rate_limited),
        ProfileState::Cooldown => Pill::Icon(icons::HOURGLASS_MEDIUM, t("Cooling down"), p.cooldown),
        ProfileState::Error => Pill::State(Status::Failed(1), t("Error")),
    }
}

/// How wide [`paint_profile_pill`] draws.
pub(super) fn profile_pill_width(painter: &egui::Painter, state: ProfileState, enabled: bool, p: &style::Palette) -> f32 {
    match profile_pill(state, enabled, p) {
        Pill::State(s, text) => kit::state_pill_width(painter, s, text),
        Pill::Icon(i, text, _) => kit::pill_width(painter, Some(i), text),
    }
}

/// Paints an account's state pill with its left edge at `left_center`.
pub(super) fn paint_profile_pill(painter: &egui::Painter, left_center: egui::Pos2, state: ProfileState, enabled: bool, p: &style::Palette, dark: bool) -> Rect {
    match profile_pill(state, enabled, p) {
        Pill::State(s, text) => kit::paint_state_pill(painter, left_center, s, text, p, dark),
        Pill::Icon(i, text, c) => kit::paint_pill(painter, left_center, Some(i), text, c, style::tint(c, dark)),
    }
}

/// A plan as a small accent badge ("MAX", "PRO"), `h` high; returns its rectangle.
pub(super) fn paint_plan_badge(painter: &egui::Painter, left_center: egui::Pos2, plan: &str, h: f32, p: &style::Palette) -> Rect {
    let g = painter.layout_no_wrap(plan.to_uppercase(), style::mono(style::DATA), p.accent_text);
    let pad = if h >= 18.0 { 6.0 } else { 5.0 };
    let r = Rect::from_min_size(pos2(left_center.x, left_center.y - h / 2.0), vec2(g.size().x + pad * 2.0, h));
    painter.rect_filled(r, CornerRadius::same(if h >= 18.0 { 5 } else { 4 }), p.accent_tint);
    painter.galley(r.center() - g.size() / 2.0, g, p.accent_text);
    r
}

/// A reset countdown as the meters show it: "3d 14h", "4h 2m", "14m", "40s".
pub(super) fn short_countdown(secs: u64) -> String {
    if secs >= 86_400 {
        format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3600)
    } else if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// One usage window as a meter: its label, a bar (when a quota % is known), the share used,
/// real tokens/turns and the reset countdown.
pub(super) struct Meter {
    pub label: String,
    pub pct: Option<f32>,
    pub tokens: Option<String>,
    pub turns: u32,
    pub reset: Option<String>,
}

/// The bar's colour: the rate-limit colour from 90 %, amber from 70 % (`amber`), else the accent.
fn meter_color(pct: f32, amber: bool, p: &style::Palette) -> Color32 {
    if pct >= 90.0 {
        p.rate_limited
    } else if amber && pct >= 70.0 {
        p.warn
    } else {
        p.accent
    }
}

/// Paints a meter with its left edge at `left_center`: when a quota percentage is known, draws
/// label + bar + `%` + optional tokens + reset; when only real token/turn usage is known (e.g.
/// Antigravity CLI), draws label + tokens/turns + reset without a misleading empty bar.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_meter(painter: &egui::Painter, left_center: egui::Pos2, m: &Meter, bar_w: f32, bar_h: f32, pct_w: f32, amber: bool, p: &style::Palette) -> f32 {
    let font = style::mono(style::DATA);
    let cy = left_center.y;
    let mut x = left_center.x;
    let g = painter.layout_no_wrap(m.label.clone(), font.clone(), p.text_faint);
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
    x += 24.0 + 8.0;
    if let Some(pct) = m.pct {
        let bar = Rect::from_min_size(pos2(x, cy - bar_h / 2.0), vec2(bar_w, bar_h));
        let r = CornerRadius::same((bar_h / 2.0).ceil() as u8);
        painter.rect_filled(bar, r, p.subtle);
        let f = (pct / 100.0).clamp(0.0, 1.0);
        if f > 0.0 {
            painter.rect_filled(Rect::from_min_size(bar.min, vec2((bar_w * f).max(bar_h), bar_h)), r, meter_color(pct, amber, p));
        }
        x += bar_w + 8.0;
        let g = painter.layout_no_wrap(format!("{pct:.0}%"), font.clone(), p.text_muted);
        let gw = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_muted);
        x += if pct_w > 0.0 { pct_w.max(gw) } else { gw } + 8.0;
        if let Some(tok) = &m.tokens {
            let g = painter.layout_no_wrap(tok.clone(), font.clone(), p.text_faint);
            let gw = g.size().x;
            painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
            x += gw.max(60.0) + 8.0;
        }
    } else {
        let summary = match (&m.tokens, m.turns) {
            (Some(tok), n) if n > 0 => format!("{tok} · {}", tf("{} turns", &[&n.to_string()])),
            (Some(tok), _) => tok.clone(),
            (None, n) if n > 0 => tf("{} turns", &[&n.to_string()]),
            (None, _) => "—".to_string(),
        };
        let g = painter.layout_no_wrap(summary, font.clone(), p.text_muted);
        let gw = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_muted);
        x += gw + 10.0;
    }
    if let Some(reset) = &m.reset {
        painter.text(pos2(x + 5.5, cy), Align2::CENTER_CENTER, icons::ARROW_COUNTER_CLOCKWISE, style::icon_font(11.0), p.text_faint);
        x += 11.0 + 4.0;
        let g = painter.layout_no_wrap(reset.clone(), font, p.text_faint);
        let gw = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
        x += gw;
    } else {
        x -= if m.pct.is_some() { 8.0 } else { 10.0 };
    }
    x - left_center.x
}

/// How wide a meter is with a bar of no width (what the rest takes).
fn meter_rest(painter: &egui::Painter, m: &Meter, pct_w: f32) -> f32 {
    let font = style::mono(style::DATA);
    let w = |s: &str| painter.layout_no_wrap(s.to_string(), font.clone(), Color32::PLACEHOLDER).size().x;
    let mut x = 24.0 + 8.0;
    if let Some(pct) = m.pct {
        let pw = w(&format!("{pct:.0}%"));
        x += 8.0 + if pct_w > 0.0 { pct_w.max(pw) } else { pw };
        if let Some(tok) = &m.tokens {
            x += 8.0 + w(tok).max(60.0);
        }
    } else {
        let summary = match (&m.tokens, m.turns) {
            (Some(tok), n) if n > 0 => format!("{tok} · {}", tf("{} turns", &[&n.to_string()])),
            (Some(tok), _) => tok.clone(),
            (None, n) if n > 0 => tf("{} turns", &[&n.to_string()]),
            (None, _) => "—".to_string(),
        };
        x += w(&summary);
    }
    if let Some(reset) = &m.reset {
        x += (if m.pct.is_some() { 8.0 } else { 10.0 }) + 11.0 + 4.0 + w(reset);
    }
    x
}

/// An account's usage windows (5h and the week, or what the provider has), as meters.
pub(super) fn profile_meters(prof: &AgentProfile, rt: &profiles::ProfileRuntime, now_unix: i64, now: Instant, with_tokens: bool) -> Vec<Meter> {
    let mut out = Vec::new();
    if rt.has_window_5h(prof) {
        let pct = rt.quota_5h_used_pct(prof);
        let tok = rt.usage.window_5h.tokens.total();
        out.push(Meter {
            label: rt.window_5h_label(),
            pct,
            tokens: ((with_tokens || pct.is_none()) && tok > 0).then(|| tf("{} tok", &[&profiles::format_tokens(tok)])),
            turns: rt.usage.window_5h.turns,
            reset: rt.window_5h_reset_secs(now_unix, now).map(short_countdown),
        });
    }
    if rt.has_window_weekly(prof) {
        let pct = rt.quota_weekly_used_pct(prof);
        let tok = rt.usage.window_weekly.tokens.total();
        let label = if rt.window_weekly_label() == "Weekly" { "Wk".to_string() } else { rt.window_weekly_label() };
        out.push(Meter {
            label,
            pct,
            tokens: ((with_tokens || pct.is_none()) && tok > 0).then(|| tf("{} tok", &[&profiles::format_tokens(tok)])),
            turns: rt.usage.window_weekly.turns,
            reset: rt.window_weekly_reset_secs(now_unix).map(short_countdown),
        });
    }
    out
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

/// What the ⋯ menu of an account asked for.
enum RowAction {
    Clear,
    Test,
    SignIn,
    Edit,
    Delete,
}

impl App {
    /// Settings → Accounts: the usage summary, what happens at a limit, the accounts in
    /// failover order (drag to reorder), the account editor, then the sessions running now and
    /// the recent failover events.
    pub(super) fn accounts_page(&mut self, ui: &mut Ui, search: &str) {
        // Countdowns tick while the page is open.
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        // Esc cancels an active drag before closing Settings.
        let drag_id = egui::Id::new("profiles-drag-idx");
        if ui.data(|d| d.get_temp::<usize>(drag_id).is_some())
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            ui.data_mut(|d| d.remove::<usize>(drag_id));
        }
        let summary = [(t("Agents running"), ""), (t("Healthy accounts"), ""), (t("Rate-limited"), ""), (t("Tokens · 5h"), ""), (t("Tokens · week"), "")];
        if any_hit(search, &summary) {
            self.usage_summary(ui);
        }
        self.failover_policy(ui, search);
        self.accounts_list(ui, search);
        if hit(search, t("Add account"), "") {
            gap(ui);
            ui.horizontal(|ui| {
                let add = kit::button_ex(ui, kit::Kind::Secondary, Some(icons::PLUS), t("Add account"), true);
                #[cfg(debug_assertions)]
                crate::devtest::record("profiles-add", add.rect);
                if add.clicked() {
                    let existing = self.effective_profiles();
                    self.profile_draft = Some(ProfileDraft::new_for(AgentKind::Claude, &existing));
                }
            });
        }
        self.sessions_and_events(ui, search);
    }

    /// The numbers at the top: agents running, healthy accounts, rate-limited ones, tokens in
    /// the 5-hour window and in the week.
    fn usage_summary(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let now = Instant::now();
        let profiles = self.effective_profiles();
        let (running, limited, _errors, _) = self.telemetry_summary();
        let (total_5h, total_weekly) = self.deduped_window_totals();
        let enabled = profiles.iter().filter(|pr| pr.enabled).count();
        let healthy = profiles.iter().filter(|pr| pr.enabled && self.profiles_rt.get(&pr.id).is_none_or(|r| r.is_healthy(now))).count();
        let cells = [
            (t("Agents running"), running.to_string(), p.text),
            (t("Healthy accounts"), format!("{healthy} / {enabled}"), if healthy > 0 { p.ok } else { p.text }),
            (t("Rate-limited"), limited.to_string(), if limited > 0 { p.rate_limited } else { p.text }),
            (t("Tokens · 5h"), profiles::format_tokens(total_5h), p.text),
            (t("Tokens · week"), profiles::format_tokens(total_weekly), p.text),
        ];
        raised(ui, 16, |ui| {
            let w = ui.available_width();
            let cell_w = (w - 12.0 * 4.0) / 5.0;
            let (r, _) = ui.allocate_exact_size(vec2(w, 16.0 + 4.0 + 24.0), Sense::hover());
            for (i, (label, value, c)) in cells.iter().enumerate() {
                let x = r.left() + i as f32 * (cell_w + 12.0);
                let lg = galley(ui, label, FontId::proportional(style::SMALL), p.text_faint, None, 16.0);
                let cell_clip = Rect::from_min_size(pos2(x, r.top()), vec2(cell_w, r.height())).intersect(ui.clip_rect());
                ui.painter().with_clip_rect(cell_clip).galley(pos2(x, r.top()), lg, p.text_faint);
                let vg = galley(ui, value, style::semibold(style::TITLE), *c, None, 24.0);
                ui.painter().with_clip_rect(cell_clip).galley(pos2(x, r.top() + 20.0), vg, *c);
            }
        });
    }

    /// "When an account hits its limit": switch automatically or ask, fall back to another
    /// provider, and how long a limited account rests.
    fn failover_policy(&mut self, ui: &mut Ui, search: &str) {
        let labels = [
            (t("Switch"), t("Auto resumes the session on the next healthy account; Ask pauses with a Notice.")),
            (t("Fall back to another provider"), t("Only when every account of this agent is limited.")),
            (t("Retry a limited account after"), ""),
        ];
        let mut changed = false;
        section(ui, search, Some(t("When an account hits its limit")), None, &labels, |c| {
            c.row(labels[0].0, labels[0].1, |ui| {
                let modes = [kit::Seg::new(t("Automatically"), "").icon(icons::LIGHTNING), kit::Seg::new(t("Ask me"), "").icon(icons::HAND)];
                let current = usize::from(self.settings.failover.mode == FailoverMode::Manual);
                if let Some(i) = kit::segmented(ui, &modes, current) {
                    self.settings.failover.mode = if i == 0 { FailoverMode::Auto } else { FailoverMode::Manual };
                    changed = true;
                }
            });
            c.row(labels[1].0, labels[1].1, |ui| {
                changed |= kit::switch(ui, &mut self.settings.failover.cross_provider).changed();
            });
            c.row(labels[2].0, "", |ui| {
                const MINS: [u32; 4] = [15, 30, 60, 120];
                let names = [t("15 minutes"), t("30 minutes"), t("1 hour"), t("2 hours")];
                let now = self.settings.failover.default_cooldown_mins;
                let current = match MINS.iter().position(|m| *m == now) {
                    Some(i) => names[i].to_string(),
                    None => tf("{} minutes", &[&now.to_string()]),
                };
                if let Some(i) = kit::select(ui, "failover-cooldown", &current, &names, 140.0) {
                    self.settings.failover.default_cooldown_mins = MINS[i];
                    changed = true;
                }
            });
        });
        if changed {
            self.settings_changed();
            self.push_companion_now();
        }
    }

    /// "Accounts, in failover order": a row per account (drag its grip to reorder), its usage
    /// under it.
    fn accounts_list(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let now = Instant::now();
        let now_unix = profiles::now_unix_secs();
        let profiles = self.effective_profiles();
        let home = crate::paths::home_dir();
        let data = crate::paths::data_dir();
        let title = t("Accounts, in failover order");
        let title_hit = !search.trim().is_empty() && hit(search, title, "");
        let shown: Vec<usize> = (0..profiles.len())
            .filter(|i| {
                let pr = &profiles[*i];
                title_hit || hit(search, &pr.id, &pr.name) || pr.account_hint.as_deref().is_some_and(|h| hit(search, h, ""))
            })
            .collect();
        if shown.is_empty() {
            return;
        }

        let mut drag_reorder: Option<(usize, usize)> = None;
        let mut toggle_enabled: Option<String> = None;
        let mut row_action: Option<(String, RowAction)> = None;
        let mut delete_id: Option<String> = None;
        let drag_id = egui::Id::new("profiles-drag-idx");
        let confirm_id = egui::Id::new("profiles-delete-confirm");
        let mut confirm: Option<String> = ui.data(|d| d.get_temp(confirm_id));
        let mut dragging: Option<usize> = ui.data(|d| d.get_temp::<usize>(drag_id));
        if dragging.is_some_and(|i| i >= profiles.len()) || !search.trim().is_empty() {
            dragging = None;
            ui.data_mut(|d| d.remove::<usize>(drag_id));
        }
        let can_drag = profiles.len() > 1 && search.trim().is_empty();
        let mut row_rects: Vec<Rect> = Vec::with_capacity(profiles.len());

        gap(ui);
        group_head(ui, title, can_drag.then_some(t("Drag to reorder")));
        let table = card(ui, "", |c| {
            for &idx in &shown {
                let prof = &profiles[idx];
                let rt = self.profiles_rt.get(&prof.id).cloned().unwrap_or_default();
                let auth = rt.auth_status(prof);
                let is_dragged = dragging == Some(idx);
                let dir = crate::paths::contract_tilde(&profiles::resolved_config_dir(prof, &home, data));
                let meters = profile_meters(prof, &rt, now_unix, now, true);
                c.divider();
                let fill = if is_dragged { p.accent_tint } else { Color32::TRANSPARENT };
                let resp = egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(16, 12)).show(c.ui, |ui| {
                    let w = ui.available_width();
                    ui.set_width(w);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    // Row 1: grip, rank, avatar, id, hint, plan, state … sign-in, enabled, ⋯
                    let (r1, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
                    let cy = r1.center().y;
                    let handle = Rect::from_min_size(r1.min, vec2(14.0 + 10.0 + 22.0, 24.0));
                    let hr = ui.interact(handle, egui::Id::new(("profile-grip", &prof.id)), if can_drag { Sense::drag() } else { Sense::hover() });
                    let lit = is_dragged || (can_drag && hr.hovered());
                    ui.painter().text(pos2(r1.left() + 7.0, cy), Align2::CENTER_CENTER, icons::DOTS_SIX_VERTICAL, style::icon_font(14.0), if lit { p.text_muted } else { p.text_ghost });
                    let rank = galley(ui, &format!("#{}", idx + 1), style::mono(style::DATA), p.text_faint, None, 14.0);
                    ui.painter().galley(pos2(r1.left() + 24.0, cy - 7.0), rank, p.text_faint);
                    if can_drag {
                        if hr.drag_started() {
                            dragging = Some(idx);
                            ui.data_mut(|d| d.insert_temp(drag_id, idx));
                        }
                        if is_dragged {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        } else if hr.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                        }
                        hr.on_hover_text(t("Drag to reorder"));
                    }
                    let avatar = Rect::from_min_size(pos2(r1.left() + 56.0, cy - 10.0), vec2(20.0, 20.0));
                    paint_agent_tile(ui.painter(), avatar, Some(prof.agent), "", &p);
                    // The right side first, so the text at the left knows its room.
                    let mut right = r1.right();
                    let more = Rect::from_min_size(pos2(right - 24.0, cy - 12.0), vec2(24.0, 24.0));
                    let more_resp = ui.scope_builder(egui::UiBuilder::new().max_rect(more), |ui| kit::icon_button_ex(ui, icons::DOTS_THREE, t("More"), false, 24.0)).inner;
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("profile-more:{}", prof.id), more_resp.rect);
                    right = more.left() - 10.0;
                    let sw = Rect::from_min_size(pos2(right - 26.0, cy - 8.0), vec2(26.0, 16.0));
                    let mut en = prof.enabled;
                    let swr = ui.scope_builder(egui::UiBuilder::new().max_rect(sw), |ui| kit::switch(ui, &mut en)).inner;
                    if swr.changed() {
                        toggle_enabled = Some(prof.id.clone());
                    }
                    swr.on_hover_text(t("In the failover order"));
                    right = sw.left() - 10.0;
                    let (auth_text, auth_c) = match auth {
                        AuthStatus::SignedIn => (t("Signed in"), p.ok),
                        AuthStatus::ApiKey => (t("API key"), p.text_muted),
                        AuthStatus::NotSignedIn => (t("Not signed in"), p.warn),
                        AuthStatus::Unknown => ("", p.text_faint),
                    };
                    if !auth_text.is_empty() {
                        let g = galley(ui, auth_text, FontId::proportional(style::SMALL), auth_c, None, 16.0);
                        right -= g.size().x;
                        ui.painter().galley(pos2(right, cy - 8.0), g, auth_c);
                        right -= 10.0;
                    }
                    let mut x = avatar.right() + 10.0;
                    let id_c = if prof.enabled { p.text } else { p.text_faint };
                    let idg = galley(ui, &prof.id, style::mono_semibold(style::DATA), id_c, None, 14.0);
                    let id_rect = Rect::from_min_size(pos2(x, cy - 7.0), idg.size());
                    ui.painter().galley(id_rect.min, idg, id_c);
                    ui.interact(id_rect, egui::Id::new(("profile-id", &prof.id)), Sense::hover()).on_hover_text(tf("Sign-in folder: {}", &[&dir]));
                    x = id_rect.right() + 10.0;
                    let pill_w = profile_pill_width(ui.painter(), rt.state, prof.enabled, &p);
                    let plan = rt.effective_plan().map(str::to_string);
                    let plan_w = plan.as_ref().map_or(0.0, |s| ui.painter().layout_no_wrap(s.to_uppercase(), style::mono(style::DATA), p.accent_text).size().x + 12.0 + 10.0);
                    let hint = prof
                        .account_hint
                        .clone()
                        .or_else(|| rt.usage.account_email.clone())
                        .unwrap_or_else(|| prof.name.clone());
                    let hint_room = right - x - plan_w - pill_w - 10.0;
                    if hint_room > 24.0 && !hint.is_empty() {
                        let mut job = egui::text::LayoutJob::simple_singleline(hint, FontId::proportional(style::SMALL), p.text_faint);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(hint_room);
                        let g = ui.painter().layout_job(job);
                        let gw = g.size().x;
                        ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
                        x += gw + 10.0;
                    }
                    if let Some(plan) = &plan
                        && x + plan_w + pill_w <= right + 10.0
                    {
                        let b = paint_plan_badge(ui.painter(), pos2(x, cy), plan, 18.0, &p);
                        x = b.right() + 10.0;
                    }
                    if x + pill_w <= right + 10.0 {
                        paint_profile_pill(ui.painter(), pos2(x, cy), rt.state, prof.enabled, &p, dark);
                    }
                    // The ⋯ menu.
                    egui::Popup::menu(&more_resp).align(egui::RectAlign::BOTTOM_END).width(220.0).show(|ui| {
                        kit::menu_body(ui);
                        ui.set_min_width(200.0);
                        if matches!(rt.state, ProfileState::RateLimited | ProfileState::Cooldown)
                            && kit::menu_item(ui, Some(icons::ARROW_COUNTER_CLOCKWISE), t("Clear cooldown"), None, false).clicked()
                        {
                            row_action = Some((prof.id.clone(), RowAction::Clear));
                        }
                        if kit::menu_item(ui, Some(icons::PLUGS_CONNECTED), t("Test connection"), None, false).clicked() {
                            row_action = Some((prof.id.clone(), RowAction::Test));
                        }
                        let sign = if auth == AuthStatus::SignedIn { t("Sign in again") } else { t("Sign in") };
                        if kit::menu_item(ui, Some(icons::KEY), sign, None, false).clicked() {
                            row_action = Some((prof.id.clone(), RowAction::SignIn));
                        }
                        if kit::menu_item(ui, Some(icons::PENCIL_SIMPLE), t("Edit…"), None, false).clicked() {
                            row_action = Some((prof.id.clone(), RowAction::Edit));
                        }
                        kit::menu_sep(ui);
                        let can_delete = profiles.len() > 1;
                        let del = ui.add_enabled_ui(can_delete, |ui| kit::menu_item(ui, Some(icons::TRASH), t("Delete…"), None, can_delete)).inner;
                        let del = if can_delete { del } else { del.on_disabled_hover_text(t("At least one account is needed")) };
                        if del.clicked() {
                            row_action = Some((prof.id.clone(), RowAction::Delete));
                        }
                    });

                    // Row 2: the usage windows, under the avatar.
                    let indent = 56.0;
                    let room = w - indent;
                    if meters.is_empty() {
                        let (r2, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
                        let g = galley(ui, t("No usage reported yet"), style::mono(style::DATA), p.text_ghost, None, 14.0);
                        ui.painter().galley(pos2(r2.left() + indent, r2.top()), g, p.text_ghost);
                    } else {
                        let (r2, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
                        let rest: f32 = meters.iter().map(|m| meter_rest(ui.painter(), m, 36.0)).sum::<f32>() + 24.0 * (meters.len() - 1) as f32;
                        let bars = meters.iter().filter(|m| m.pct.is_some()).count().max(1);
                        let bar_w = ((room - rest) / bars as f32).clamp(40.0, 120.0);
                        let mut x = r2.left() + indent;
                        let painter = ui.painter().with_clip_rect(r2.intersect(ui.clip_rect()));
                        for m in &meters {
                            x += paint_meter(&painter, pos2(x, r2.center().y), m, bar_w, 5.0, 36.0, true, &p) + 24.0;
                        }
                    }
                    // A connection test's answer.
                    if let Some(conn) = &rt.connection {
                        let (msg, c) = match conn {
                            ConnectionTest::Testing => (t("Testing connection…").to_string(), p.info),
                            ConnectionTest::Ok(m) => (m.clone(), p.ok),
                            ConnectionTest::Err(e) => (e.clone(), p.err),
                        };
                        ui.horizontal(|ui| {
                            ui.add_space(indent);
                            text(ui, &msg, style::mono(style::DATA), c, 14.0, true);
                        });
                    }
                    // Delete asks first, here in the row.
                    if confirm.as_deref() == Some(prof.id.as_str()) {
                        ui.horizontal(|ui| {
                            ui.add_space(indent);
                            ui.spacing_mut().item_spacing.x = 8.0;
                            text(ui, &tf("Delete {}? Its sign-in folder stays on this computer.", &[&prof.id]), FontId::proportional(style::SMALL), p.text, 16.0, false);
                            if kit::button_ex(ui, kit::Kind::Ghost, None, t("Cancel"), true).clicked() {
                                confirm = None;
                            }
                            let d = kit::button_ex(ui, kit::Kind::DangerFilled, Some(icons::TRASH), t("Delete"), true);
                            #[cfg(debug_assertions)]
                            crate::devtest::record(format!("profile-delete:{}", prof.id), d.rect);
                            if d.clicked() {
                                delete_id = Some(prof.id.clone());
                                confirm = None;
                            }
                        });
                    }
                });
                row_rects.push(resp.response.rect);
            }
        });
        let table_rect = table.rect;

        // Dragging: a line where the account would land and the account under the pointer.
        if let Some(from) = dragging {
            let cancel = !ui.input(|i| i.focused);
            if cancel {
                ui.data_mut(|d| d.remove::<usize>(drag_id));
            } else {
                let pointer_pos = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
                let still_down = ui.input(|i| i.pointer.primary_down());
                if let Some(pos) = pointer_pos {
                    let in_zone = table_rect.expand(40.0).contains(pos);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    let slot = row_rects.iter().position(|r| pos.y < r.center().y).unwrap_or(row_rects.len());
                    let line_y = if slot == 0 {
                        row_rects.first().map_or(table_rect.top(), |r| r.top())
                    } else if slot >= row_rects.len() {
                        row_rects.last().map_or(table_rect.bottom(), |r| r.bottom())
                    } else {
                        row_rects[slot].top()
                    };
                    if in_zone && slot != from && slot != from + 1 {
                        let x0 = table_rect.left() + 10.0;
                        let x1 = table_rect.right() - 10.0;
                        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("profiles-drop-line")));
                        painter.hline(x0..=x1, line_y, Stroke::new(2.0, p.accent));
                        painter.circle_filled(pos2(x0, line_y), 3.5, p.accent);
                        painter.circle_filled(pos2(x1, line_y), 3.5, p.accent);
                    }
                    if let Some(prof) = profiles.get(from) {
                        egui::Area::new(egui::Id::new("profiles-drag-ghost"))
                            .order(egui::Order::Tooltip)
                            .interactable(false)
                            .fixed_pos(pos + vec2(14.0, -14.0))
                            .show(ui.ctx(), |ui| {
                                egui::Frame::NONE
                                    .fill(p.menu)
                                    .stroke(Stroke::new(1.0, p.accent))
                                    .corner_radius(CornerRadius::same(7))
                                    .inner_margin(egui::Margin::symmetric(10, 5))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 6.0;
                                            let (mr, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                                            paint_agent_tile(ui.painter(), mr, Some(prof.agent), "", &p);
                                            ui.label(RichText::new(&prof.id).font(style::mono_semibold(style::DATA)).color(p.text));
                                        });
                                    });
                            });
                    }
                    if !still_down {
                        if in_zone {
                            let target = if slot > from { slot - 1 } else { slot };
                            if target != from && target < profiles.len() {
                                drag_reorder = Some((from, target));
                            }
                        }
                        ui.data_mut(|d| d.remove::<usize>(drag_id));
                    }
                } else if !still_down {
                    ui.data_mut(|d| d.remove::<usize>(drag_id));
                }
            }
        }

        if let Some((id, action)) = row_action {
            match action {
                RowAction::Clear => self.actions.push(Action::ClearProfileCooldown(id)),
                RowAction::Test => self.actions.push(Action::TestProfile(id)),
                RowAction::SignIn => self.actions.push(Action::LoginProfile(id)),
                RowAction::Edit => {
                    if let Some(prof) = profiles.iter().find(|x| x.id == id) {
                        self.profile_draft = Some(ProfileDraft::from_profile(prof));
                        ui.data_mut(|d| d.insert_temp(egui::Id::new("profile-editor-scroll"), true));
                    }
                }
                RowAction::Delete => confirm = Some(id),
            }
        }
        ui.data_mut(|d| match &confirm {
            Some(c) => {
                d.insert_temp(confirm_id, c.clone());
            }
            None => d.remove::<String>(confirm_id),
        });
        if let Some((from, to)) = drag_reorder {
            self.ensure_mutable_profiles();
            self.settings.profiles.sort_by_key(|x| (x.fallback_order, x.id.clone()));
            if from < self.settings.profiles.len() && to < self.settings.profiles.len() && from != to {
                let item = self.settings.profiles.remove(from);
                self.settings.profiles.insert(to, item);
                for (i, p_item) in self.settings.profiles.iter_mut().enumerate() {
                    p_item.fallback_order = (i + 1) as u32;
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
                self.recompute_usage_totals();
                self.push_companion_now();
            }
        }
        if let Some(id) = delete_id {
            self.delete_profile(&id);
        }
    }

    /// Removes an account (at least one stays), and what pointed to it.
    fn delete_profile(&mut self, id: &str) {
        self.ensure_mutable_profiles();
        if self.settings.profiles.len() <= 1 {
            return;
        }
        self.settings.profiles.retain(|x| x.id != id);
        self.profiles_rt.remove(id);
        for pane in self.panes.values_mut() {
            if pane.active_profile.as_deref() == Some(id) {
                pane.active_profile = None;
            }
            if pane.rate_limit.as_ref().is_some_and(|b| b.from_profile == id) {
                pane.rate_limit = None;
            }
        }
        for proj in &mut self.projects {
            if proj.active_profile.as_deref() == Some(id) {
                proj.active_profile = None;
            }
        }
        for list in self.project_sessions.values_mut() {
            for item in list {
                if item.profile_id.as_deref() == Some(id) {
                    item.profile_id = None;
                }
            }
        }
        self.sessions_scan_at.clear();
        if self.profile_draft.as_ref().and_then(|d| d.editing_id.as_deref()) == Some(id) {
            self.profile_draft = None;
        }
        self.settings_changed();
        self.recompute_usage_totals();
        self.push_companion_now();
        self.toast(tf("Removed account {}.", &[id]));
    }

    /// Centered Slate pop-up modal for adding or editing an account (rendered over Settings).
    pub(super) fn profile_editor_modal(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.profile_draft.clone() else { return };
        let dark = ctx.global_style().visuals.dark_mode;
        let p = style::palette(dark);
        let mut save = false;
        let mut cancel = false;
        let existing = self.effective_profiles();
        let sanitized_id = profiles::sanitize_id(draft.id.trim());
        let id_empty = draft.id.trim().is_empty();
        let id_dup = !id_empty && existing.iter().any(|x| x.id == sanitized_id && draft.editing_id.as_deref() != Some(x.id.as_str()));
        let agy_exists = existing.iter().any(|x| x.agent == AgentKind::Antigravity && draft.editing_id.as_deref() != Some(x.id.as_str()));
        let agy_dup = draft.agent == AgentKind::Antigravity && agy_exists;
        let single_account = !profiles::supports_multi_profile(draft.agent);
        if single_account {
            draft.use_default_auth = true;
            draft.config_dir.clear();
        }
        let title = match &draft.editing_id {
            Some(id) => tf("Edit {}", &[id]),
            None => t("New account").to_string(),
        };
        let sub = if draft.editing_id.is_some() {
            t("Update this account's name, limits, or credentials.")
        } else {
            t("Sign in and track usage separately for this agent.")
        };
        let ok = !id_empty && !id_dup && !agy_dup;
        let body_h = (ctx.content_rect().height() - 190.0).clamp(260.0, 560.0);

        let modal = egui::Modal::new(egui::Id::new("profile-editor-modal"))
            .frame(super::dialogs::frame(ctx))
            .backdrop_color(super::dialogs::scrim(dark))
            .show(ctx, |ui| {
                super::dialogs::begin(ui, 580.0);
                if super::dialogs::header(ui, icons::USER_CIRCLE, p.accent, &title, Some(sub)) {
                    cancel = true;
                }
                super::dialogs::body(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("profile-editor-scroll")
                        .min_scrolled_height(body_h)
                        .max_height(body_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            card(ui, "", |c| {
                                let agent_picker = |ui: &mut Ui, draft: &mut ProfileDraft| {
                                    let providers = [
                                        (AgentKind::Claude, "Claude Code"),
                                        (AgentKind::Codex, "Codex"),
                                        (AgentKind::Antigravity, "Antigravity CLI"),
                                    ];
                                    let names: Vec<&str> = providers.iter().map(|(_, n)| *n).collect();
                                    let cur = providers.iter().find(|(k, _)| *k == draft.agent).map_or("Claude Code", |(_, n)| *n);
                                    if let Some(i) = kit::select(ui, "prof-agent", cur, &names, 165.0) {
                                        let new_agent = providers[i].0;
                                        if new_agent != draft.agent {
                                            // A new account's suggested id and name follow the agent (claude2 → chatgpt2 → agy1).
                                            if draft.editing_id.is_none() {
                                                let (old_id, old_name) = ProfileDraft::suggest_id_and_name(draft.agent, &existing);
                                                let (nid, nname) = ProfileDraft::suggest_id_and_name(new_agent, &existing);
                                                if draft.id == old_id || draft.id.trim().is_empty() {
                                                    draft.id = nid;
                                                }
                                                if draft.name == old_name || draft.name.trim().is_empty() {
                                                    draft.name = nname;
                                                }
                                                draft.use_default_auth = !profiles::supports_multi_profile(new_agent);
                                            }
                                            draft.agent = new_agent;
                                            if !profiles::supports_multi_profile(new_agent) {
                                                draft.use_default_auth = true;
                                                draft.config_dir.clear();
                                            }
                                        }
                                    }
                                };
                                if agy_dup {
                                    c.row_note(t("Agent"), t("Antigravity CLI supports 1 account only (Google policy)."), p.err, |ui| {
                                        agent_picker(ui, &mut draft);
                                    });
                                } else {
                                    c.row(t("Agent"), t("Which agent signs in with this account."), |ui| {
                                        agent_picker(ui, &mut draft);
                                    });
                                }
                                if id_dup {
                                    c.row_note(t("Account id"), t("Another account has this id."), p.err, |ui| {
                                        mono_field(ui, egui::Id::new("prof-id"), &mut draft.id, "claude2", 140.0, false);
                                    });
                                } else {
                                    c.row(t("Account id"), t("A short name for panes and menus."), |ui| {
                                        mono_field(ui, egui::Id::new("prof-id"), &mut draft.id, "claude2", 140.0, false);
                                    });
                                }
                                c.row(t("Name"), "", |ui| {
                                    kit::field(ui, egui::Id::new("prof-name"), &mut draft.name, None, "Claude 2", 200.0, false);
                                });
                                c.row(t("Email or note"), t("Optional, to tell accounts apart."), |ui| {
                                    kit::field(ui, egui::Id::new("prof-hint"), &mut draft.account_hint, None, "work@acme.dev", 220.0, false);
                                });
                                c.row(t("Token limits"), t("Claude and Codex report their own limits. Set caps only for other plans: 500k, 2M."), |ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    text(ui, "5h", style::mono(style::DATA), p.text_faint, 14.0, false);
                                    mono_field(ui, egui::Id::new("prof-lim-5h"), &mut draft.limit_5h_text, "500k", 76.0, false);
                                    ui.add_space(4.0);
                                    text(ui, "Wk", style::mono(style::DATA), p.text_faint, 14.0, false);
                                    mono_field(ui, egui::Id::new("prof-lim-wk"), &mut draft.limit_weekly_text, "5M", 76.0, false);
                                });
                                if !single_account && !draft.use_default_auth {
                                    let hint = crate::paths::contract_tilde(&crate::paths::data_dir().join("profiles").join(&sanitized_id));
                                    c.row(t("Sign-in folder"), t("Where this account keeps its sign-in."), |ui| {
                                        kit::field(ui, egui::Id::new("prof-dir"), &mut draft.config_dir, Some(icons::FOLDER), &hint, 236.0, false);
                                    });
                                }
                                c.row(t("API key"), t("Optional: used instead of signing in."), |ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    key_field(ui, egui::Id::new("prof-key"), &mut draft.api_key, draft.reveal_key, 200.0);
                                    if kit::icon_button_ex(ui, icons::EYE, t("Show/Hide Key"), draft.reveal_key, 28.0).clicked() {
                                        draft.reveal_key = !draft.reveal_key;
                                    }
                                });
                                c.row(t("Extra environment"), t("KEY=value, one per line."), |ui| {
                                    mono_field(ui, egui::Id::new("prof-extra-env"), &mut draft.extra_env_text, "ANTHROPIC_BASE_URL=https://…", 236.0, true);
                                });
                                c.row(t("In the failover order"), t("Off: the account is kept but never picked."), |ui| {
                                    kit::switch(ui, &mut draft.enabled);
                                });
                            });
                        });
                });
                super::dialogs::footer_left(
                    ui,
                    super::dialogs::esc_hint,
                    |ui| {
                        let label = if draft.editing_id.is_some() { t("Save account") } else { t("Add account") };
                        let s = ui.add_enabled_ui(ok, |ui| kit::button_ex(ui, kit::Kind::Primary, None, label, false)).inner;
                        #[cfg(debug_assertions)]
                        crate::devtest::record("profile-save", s.rect);
                        if s.clicked() {
                            save = true;
                        }
                        if kit::secondary(ui, None, t("Cancel")).clicked() {
                            cancel = true;
                        }
                    },
                );
            });
        #[cfg(debug_assertions)]
        crate::devtest::record("profile-editor-modal", modal.response.rect);
        if modal.should_close() {
            cancel = true;
        }

        if cancel {
            self.profile_draft = None;
        } else if save && ok {
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
                    self.rename_profile_refs(&old_id, &new_id);
                }
            } else {
                let new_prof = draft.into_profile(next_order);
                self.settings.profiles.push(new_prof);
            }
            self.settings_changed();
            self.profile_draft = None;
            self.recompute_usage_totals();
            self.usage_scan_at = None;
            self.push_companion_now();
            self.toast_ok(t("Account saved.").into());
        } else {
            self.profile_draft = Some(draft);
        }
    }

    /// An account's id changed: every pane, project and session that pointed to the old one
    /// follows.
    fn rename_profile_refs(&mut self, old_id: &str, new_id: &str) {
        if let Some(rt) = self.profiles_rt.remove(old_id) {
            self.profiles_rt.insert(new_id.to_string(), rt);
        }
        for pane in self.panes.values_mut() {
            if pane.active_profile.as_deref() == Some(old_id) {
                pane.active_profile = Some(new_id.to_string());
            }
            if let Some(b) = &mut pane.rate_limit
                && b.from_profile == old_id
            {
                b.from_profile = new_id.to_string();
            }
        }
        for proj in &mut self.projects {
            if proj.active_profile.as_deref() == Some(old_id) {
                proj.active_profile = Some(new_id.to_string());
            }
        }
        for list in self.project_sessions.values_mut() {
            for item in list {
                if item.profile_id.as_deref() == Some(old_id) {
                    item.profile_id = Some(new_id.to_string());
                }
            }
        }
        self.sessions_scan_at.clear();
    }

    /// "Active sessions" (agents running now, with their account) and "Recent events" (the
    /// failover log).
    fn sessions_and_events(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let mut sessions: Vec<_> = self
            .panes
            .values()
            .filter(|x| x.alive() && (x.tracker.agent.is_some() || x.active_profile.is_some()))
            .map(|x| {
                let task = x.task.clone().or_else(|| x.tracker.message.clone());
                (x.id, self.project_label(x.project), x.label(), x.active_profile.clone(), x.tracker.status, status::age(x.tracker.since), task)
            })
            .collect();
        sessions.sort_by_key(|x| x.0);
        let labels: Vec<(String, String)> = sessions.iter().map(|s| (s.1.clone(), s.2.clone())).collect();
        let mut wanted: Vec<(&str, &str)> = labels.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        wanted.push((t("Active sessions"), ""));
        let mut focus = None;
        section(ui, search, Some(t("Active sessions")), None, &wanted, |c| {
            if sessions.is_empty() {
                c.block(&[], egui::Margin::symmetric(16, 12), |ui| {
                    text(ui, t("No agents are running now."), FontId::proportional(style::SMALL), p.text_faint, 16.0, false);
                });
            }
            for (pane, project, label, prof, st, age, task) in &sessions {
                let mut note = format!("{label} · {age}");
                if let Some(tk) = task {
                    note.push_str(&format!(" · {tk}"));
                }
                c.row(project, &note, |ui| {
                    if let Some(pr) = prof {
                        let g = ui.painter().layout_no_wrap(pr.clone(), style::mono(style::DATA), p.accent_text);
                        let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 12.0, 18.0), Sense::hover());
                        paint_plan_badge(ui.painter(), r.left_center(), pr, 18.0, &p);
                    }
                    let st_text = status::text(*st);
                    let w = kit::state_pill_width(ui.painter(), *st, &st_text);
                    let (r, _) = ui.allocate_exact_size(vec2(w, kit::PILL_H), Sense::hover());
                    kit::paint_state_pill(ui.painter(), r.left_center(), *st, &st_text, &p, dark);
                    if kit::button_ex(ui, kit::Kind::Secondary, None, t("Open"), true).clicked() {
                        focus = Some(*pane);
                    }
                });
            }
        });
        if let Some(pane) = focus {
            self.actions.push(Action::FocusPane(pane));
        }

        let wanted = [(t("Recent events"), "")];
        section(ui, search, Some(t("Recent events")), None, &wanted, |c| {
            c.block(&[], egui::Margin::symmetric(16, 12), |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                if self.agent_logs.is_empty() {
                    text(ui, t("Nothing happened yet."), FontId::proportional(style::SMALL), p.text_faint, 16.0, false);
                }
                for entry in self.agent_logs.iter().rev().take(20) {
                    let c = match entry.level {
                        LogLevel::Failover => p.ok,
                        LogLevel::Warn => p.warn,
                        LogLevel::Err => p.err,
                        LogLevel::Tool => p.info,
                        LogLevel::Info => p.text_muted,
                    };
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        text(ui, &entry.at.format("%H:%M:%S").to_string(), style::mono(style::DATA), p.text_faint, 16.0, false);
                        if let Some(pr) = &entry.profile {
                            text(ui, pr, style::mono_semibold(style::DATA), p.accent_text, 16.0, false);
                        }
                        text(ui, &entry.message, FontId::proportional(style::SMALL), c, 16.0, true);
                    });
                }
            });
        });
    }
}

/// A password field (the API key): dots unless shown.
fn key_field(ui: &mut Ui, id: egui::Id, key: &mut String, reveal: bool, width: f32) -> egui::Response {
    let p = style::pal(ui);
    let (rect, outer) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    if outer.clicked() {
        ui.memory_mut(|m| m.request_focus(id));
    }
    let focused = ui.memory(|m| m.has_focus(id));
    if focused {
        ui.painter().rect_stroke(rect.expand(3.0), CornerRadius::same(10), Stroke::new(3.0, style::soft(p.accent, 0.25)), StrokeKind::Inside);
    }
    ui.painter().rect(rect, CornerRadius::same(7), p.inset, Stroke::new(1.0, if focused { p.accent } else { p.border_strong }), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(10.0, 1.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.add_sized(
        inner.size(),
        egui::TextEdit::singleline(key)
            .id(id)
            .char_limit(4_096)
            .password(!reveal)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(style::mono(12.0))
            .vertical_align(egui::Align::Center)
            .hint_text(RichText::new("sk-…").color(p.text_faint)),
    )
}

impl App {
    /// Compact interactive Active Profile Switcher + 1-Click Switch Account button rendered inside
    /// a pane's header bar (right-to-left layout).
    pub fn pane_profile_switcher(&mut self, ui: &mut Ui, pane_id: PaneId) {
        let p = style::pal(ui);
        let now = Instant::now();
        let now_unix = profiles::now_unix_secs();
        let profiles = self.effective_profiles();
        let Some((active_id, agent, has_live_agent)) = self.panes.get(&pane_id).and_then(|pane| {
            let agent = pane.tracker.agent.or(pane.last_agent)?;
            if !profiles::supports_profile(agent) {
                return None;
            }
            let id = self.resolved_pane_profile_id(pane, agent, &profiles);
            let live = pane.alive()
                && pane.tracker.agent.is_some()
                && !matches!(pane.tracker.fg, tc_core::status::Foreground::Shell | tc_core::status::Foreground::Gone);
            Some((id, Some(agent), live))
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
            } else if let Some(used) = r.quota_5h_used_pct(c) {
                Some(format!("· {used:.0}%"))
            } else if r.usage.window_5h.tokens.total() > 0 {
                Some(format!("· {}", profiles::format_tokens(r.usage.window_5h.tokens.total())))
            } else {
                None
            }
        });
        let resp = kit::profile_chip(ui, label, detail.as_deref(), dot, false);
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("pane-profile:{pane_id}"), resp.rect);

        // One-click Switch Account button right beside the active profile chip when another
        // pre-registered account is ready (no logout / re-login required).
        // Shown only while the account runs out and the user has not dismissed failover for this pane.
        let dismissed = self.panes.get(&pane_id).is_some_and(|x| x.rate_limit_dismissed);
        let urgent = !dismissed
            && rt.zip(prof_cfg).is_some_and(|(r, c)| {
                r.state == ProfileState::RateLimited || r.quota_5h_used_pct(c).is_some_and(|u| u >= 85.0)
            });
        if has_live_agent
            && urgent
            && let Some(next_prof) = self.next_switch_profile(pane_id)
        {
            let next_rt = self.profiles_rt.get(&next_prof.id);
            let next_sum = compact_window_summary(&next_prof, next_rt, now_unix, now);
            let tip = tf("Switch immediately to {} ({}) and resume session without logout", &[&next_prof.id, &next_sum]);
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
        let room = right_edge - 8.0 - tx;
        if room >= 32.0 {
            let mut job = egui::text::LayoutJob::simple_singleline(text, style::medium(12.0), p.text);
            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
            let g = painter.layout_job(job);
            painter.galley(pos2(tx, cy - g.size().y / 2.0), g, p.text);
        }
    }
}
