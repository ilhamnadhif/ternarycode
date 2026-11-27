//! The sidebar (Slate): the places on top (Search, Inbox, Overview), then the projects in the
//! order the user chose (⌘1–9 never move), the selected one opening its sessions (its tabs) and
//! worktrees, then the account in use as one meter and Settings at the foot. Narrow (⌥⌘B), a
//! rail of tiles. Typing in Search filters the projects in place.

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::{App, Dialog};
use crate::i18n::{t, tf};
use tc_core::model::View;
use tc_core::status::Status;

/// Height of the project bar beside the title row (the Focus view's top bar).
pub const TOP_H: f32 = 44.0;
/// The title row (the window's buttons on macOS, Hide Sidebar).
const TITLE_H: f32 = 40.0;
/// The narrow sidebar (⌥⌘B): a rail of tiles.
pub const COMPACT_W: f32 = 76.0;
/// A section's header ("Projects").
pub const SECTION_H: f32 = 28.0;
const PAD: f32 = 8.0;
const GAP: f32 = 10.0;
const NAV_H: f32 = 30.0;
const SEARCH_H: f32 = 30.0;
const PROJECT_H: f32 = 32.0;
const GROUP_H: f32 = 26.0;
const SESSION_H: f32 = 28.0;
const FOOT_H: f32 = 30.0;
const USAGE_H: f32 = 68.0;

/// Sidebar sizes derived from `settings.sidebar_font_size` (default 14 pt vs base 13 pt).
#[derive(Clone, Copy)]
struct SidebarScale {
    body: f32,
    small: f32,
    data: f32,
    label: f32,
    icon: f32,
    tile: f32,
    nav_h: f32,
    search_h: f32,
    project_h: f32,
    group_h: f32,
    session_h: f32,
    foot_h: f32,
    usage_h: f32,
}

impl SidebarScale {
    fn of(body: f32) -> Self {
        let s = (body / style::BODY).clamp(0.84, 1.45);
        Self {
            body,
            small: (style::SMALL * s).round(),
            data: (style::DATA * s).round(),
            label: (style::LABEL * s).round(),
            icon: (16.0 * s).round(),
            tile: (17.0 * s).round(),
            nav_h: (NAV_H * s).round(),
            search_h: (SEARCH_H * s).round(),
            project_h: (PROJECT_H * s).round(),
            group_h: (GROUP_H * s).round(),
            session_h: (SESSION_H * s).round(),
            foot_h: (FOOT_H * s).round(),
            usage_h: (USAGE_H * s).round(),
        }
    }
}

/// How long a project stays under Active after it went quiet (so it does not jump away just as
/// you reach for it). End-to-end checks can shorten this via `TERNARYCODE_ACTIVE_LINGER_MS`.
fn active_linger() -> std::time::Duration {
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var("TERNARYCODE_ACTIVE_LINGER_MS").ok().and_then(|v| v.parse().ok()) {
        return std::time::Duration::from_millis(ms);
    }
    std::time::Duration::from_secs(30)
}

/// A line of the project list.
enum Line {
    /// A group's collapsible header (`true` = Active, `false` = Idle).
    Group(bool),
    /// A root project row.
    Project {
        idx: usize,
        marks: Vec<bool>,
        dim: bool,
        is_active: bool,
        /// Whether child sessions and/or worktrees are shown below this row.
        expanded: bool,
        /// One of this project's worktrees is currently selected (not the root itself).
        parent_of_selected: bool,
    },
    /// A pane inside a tab of an expanded root project or worktree.
    Session {
        project: tc_core::ProjectId,
        tab: usize,
        pane: tc_core::PaneId,
        first_in_tab: bool,
        /// Last child at its own nesting level.
        is_last: bool,
        /// Nested under a worktree (level 2) rather than a root project (level 1).
        in_worktree: bool,
        /// When `in_worktree`, whether another worktree follows its parent worktree under
        /// the same root project (so the outer level-1 vertical rail continues through).
        outer_continues: bool,
    },
    /// A worktree under its project.
    Worktree {
        idx: usize,
        marks: Vec<bool>,
        /// Last child under its parent project.
        is_last: bool,
        /// How many expandable sessions/panes belong to this worktree right now.
        session_count: usize,
        /// Whether its nested sessions are currently emitted right below it.
        children_expanded: bool,
        /// Whether this worktree has active/running/resumable panes.
        is_active: bool,
    },
}

/// What a session row shows.
struct SessionRow {
    title: String,
    agent: Option<tc_core::agent::AgentKind>,
    meta: String,
    status: Option<Status>,
    on: bool,
}

impl App {
    pub fn sidebar(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let sc = SidebarScale::of(self.settings.sidebar_font_size);
        let full = ui.max_rect();
        let inner_l = full.left() + PAD;
        let inner_r = full.right() - PAD;

        // ----- title row: the window's buttons, the drag area, Hide Sidebar -----------------
        let title = Rect::from_min_size(full.min, vec2(full.width(), TITLE_H));
        super::view::window_drag_area(ui, title, "sidebar-top");
        let toggle = Rect::from_center_size(pos2(full.right() - 6.0 - 13.0, title.center().y), vec2(26.0, 26.0));
        let r = ui
            .scope_builder(egui::UiBuilder::new().max_rect(toggle), |ui| {
                kit::icon_button_ex(ui, icons::SIDEBAR_SIMPLE, &kit::tip(t("Hide Sidebar"), Action::ToggleSidebar), false, 26.0)
            })
            .inner;
        #[cfg(debug_assertions)]
        crate::devtest::record("hide-sidebar", toggle);
        if r.clicked() {
            self.actions.push(Action::ToggleSidebar);
        }
        let mut y = title.bottom();

        // ----- Search: filters the projects here (⌘K searches everything) --------------------
        let search = Rect::from_min_size(pos2(inner_l, y), vec2(inner_r - inner_l, sc.search_h));
        let typing = self.sidebar_search(ui, search, sc);
        y = search.bottom() + GAP;
        let searching = typing || !self.project_filter.trim().is_empty();

        // ----- Inbox, Overview, Automations --------------------------------------------------
        if !searching {
            let waiting = self.attention_queue().len() + self.unreviewed_automation_alerts_count();
            let inbox = Rect::from_min_size(pos2(inner_l, y), vec2(inner_r - inner_l, sc.nav_h));
            let r = nav_item(ui, inbox, icons::TRAY, t("Inbox"), (waiting > 0).then_some(waiting), None, self.view == View::Inbox, sc, &p);
            #[cfg(debug_assertions)]
            crate::devtest::record("nav:inbox", inbox);
            if r.on_hover_text(kit::tip(t("Everything that waits for you"), Action::ToggleAttention)).clicked() {
                self.actions.push(Action::ToggleAttention);
            }
            let overview = inbox.translate(vec2(0.0, sc.nav_h + 2.0));
            let keys = kit::keys(&Action::ToggleGrid);
            let r = nav_item(ui, overview, icons::SQUARES_FOUR, t("Overview"), None, keys, self.view == View::Grid && !self.attention_open, sc, &p);
            #[cfg(debug_assertions)]
            crate::devtest::record("nav:overview", overview);
            if r.on_hover_text(t("Every project at once")).clicked() {
                self.actions.push(Action::SetView(if self.view == View::Grid { View::Focus } else { View::Grid }));
            }
            let automations = overview.translate(vec2(0.0, sc.nav_h + 2.0));
            let auto_running = self.running_automations_count();
            let auto_alerts = self.unreviewed_automation_alerts_count();
            let auto_keys = if auto_running > 0 { None } else { kit::keys(&Action::ToggleAutomations) };
            let auto_count = if auto_running == 0 && auto_alerts > 0 { Some(auto_alerts) } else { None };
            let r = nav_item(ui, automations, icons::CLOCK, t("Automations"), auto_count, auto_keys, self.view == View::Automations, sc, &p);
            if auto_running > 0 {
                let dark = ui.visuals().dark_mode;
                let txt = format!("● {auto_running}");
                let g = ui.painter().layout_no_wrap(txt, style::mono_semibold(style::DATA), p.info);
                let w = (g.size().x + 10.0).max(24.0);
                let pill = Rect::from_min_size(pos2(automations.right() - 8.0 - w, automations.center().y - 9.0), vec2(w, 18.0));
                ui.painter().rect_filled(pill, CornerRadius::same(9), style::tint(p.info, dark));
                ui.painter().galley(pill.center() - g.size() / 2.0, g, p.info);
            }
            #[cfg(debug_assertions)]
            crate::devtest::record("nav:automations", automations);
            if r.on_hover_text(kit::tip(t("Scheduled background runs"), Action::ToggleAutomations)).clicked() {
                self.actions.push(Action::ToggleAutomations);
            }
            y = automations.bottom() + GAP;
        }

        // ----- foot: the account in use, Settings (while searching: the keys) -------------------
        let bottom = full.bottom() - 10.0;
        let list_bottom = if searching {
            let hint = Rect::from_min_max(pos2(inner_l, bottom - 18.0), pos2(inner_r, bottom));
            ui.painter().text(pos2(hint.left() + 2.0, hint.center().y), Align2::LEFT_CENTER, t("↑↓ choose · ↵ open · esc clear"), egui::FontId::proportional(sc.small), p.text_faint);
            hint.top() - GAP
        } else {
            let foot = Rect::from_min_max(pos2(inner_l, bottom - sc.foot_h), pos2(inner_r, bottom));
            self.sidebar_footer(ui, foot, sc);
            let mut top = foot.top() - GAP;
            if let Some(meter) = self.usage_meter_data() {
                let rect = Rect::from_min_max(pos2(inner_l, top - sc.usage_h), pos2(inner_r, top));
                if rect.top() > y + SECTION_H + 3.0 * sc.project_h {
                    self.usage_meter(ui, rect, &meter, sc);
                    top = rect.top() - GAP;
                }
            }
            top
        };

        // ----- projects ---------------------------------------------------------------------
        let list = Rect::from_min_max(pos2(full.left(), y.min(list_bottom)), pos2(full.right(), list_bottom.max(y)));
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            ui.set_clip_rect(list.intersect(ui.clip_rect()));
            self.project_list(ui, searching, sc)
        });
    }

    /// The search field at the top: a magnifier, what is typed (or "Search…" and ⌘K), × to
    /// empty it. While it has the keyboard ↑ ↓ choose a project, ↵ opens it, Esc empties it and
    /// lets go. Returns whether it has the keyboard.
    fn sidebar_search(&mut self, ui: &mut Ui, rect: Rect, sc: SidebarScale) -> bool {
        let p = style::pal(ui);
        let fid = egui::Id::new("project-filter");
        let typing = ui.memory(|m| m.has_focus(fid));
        #[cfg(debug_assertions)]
        crate::devtest::record("project-filter", rect);
        let found = std::mem::take(&mut self.project_filter_rows);
        let n = found.len();
        let mut sel = self.project_filter_sel.min(n.saturating_sub(1));
        let mut open = None;
        if typing && !self.overlay_open() {
            use egui::{Key, Modifiers};
            let (down, up, enter, esc) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                )
            });
            if down && n > 0 {
                sel = (sel + 1) % n;
                self.project_filter_reveal = true;
            }
            if up && n > 0 {
                sel = (sel + n - 1) % n;
                self.project_filter_reveal = true;
            }
            if enter && let Some(&id) = found.get(sel) {
                open = Some(id);
            }
            if esc || open.is_some() {
                self.project_filter.clear();
                sel = 0;
                ui.memory_mut(|m| m.surrender_focus(fid));
            }
        }
        let typing = ui.memory(|m| m.has_focus(fid));
        // The field: inset with the accent ring while typed into, else a quiet fill.
        let painter = ui.painter();
        if typing {
            painter.rect(rect.expand(3.0), CornerRadius::same(11), egui::Color32::TRANSPARENT, Stroke::new(3.0, style::soft(p.ring, 0.25)), StrokeKind::Inside);
            painter.rect(rect, CornerRadius::same(8), p.inset, Stroke::new(1.0, p.accent), StrokeKind::Inside);
        } else {
            painter.rect(rect, CornerRadius::same(8), p.hover, Stroke::new(1.0, p.border), StrokeKind::Inside);
        }
        painter.text(pos2(rect.left() + 10.0 + 7.0, rect.center().y), Align2::CENTER_CENTER, icons::MAGNIFYING_GLASS, style::icon_font((sc.icon - 2.0).max(12.0)), if typing { p.text_muted } else { p.text_faint });
        let text_rect = Rect::from_min_max(pos2(rect.left() + 10.0 + 14.0 + 8.0, rect.top()), pos2(rect.right() - 30.0, rect.bottom()));
        let mut text = self.project_filter.clone();
        let edit = ui
            .scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .id(fid)
                        .char_limit(1_024)
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::proportional(sc.body))
                        .text_color(p.text)
                        .desired_width(text_rect.width())
                        .vertical_align(egui::Align::Center)
                        .min_size(vec2(text_rect.width(), text_rect.height())),
                )
            })
            .inner;
        if edit.changed() {
            self.project_filter = text;
            sel = 0;
            self.project_filter_reveal = true;
        }
        if edit.has_focus() {
            ui.memory_mut(|m| m.set_focus_lock_filter(fid, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, escape: true, ..Default::default() }));
        }
        if self.project_filter.is_empty() {
            ui.painter().text(pos2(text_rect.left(), rect.center().y), Align2::LEFT_CENTER, t("Search…"), egui::FontId::proportional(sc.body), p.text_faint);
            if !typing && let Some(k) = kit::keys(&Action::Palette) {
                let kw = kit::kbd_width(ui.painter(), k);
                kit::paint_kbd_text(ui.painter(), pos2(rect.right() - 6.0 - kw, rect.center().y), k, &p);
            }
        } else {
            let x = Rect::from_center_size(pos2(rect.right() - 8.0 - 9.0, rect.center().y), vec2(18.0, 18.0));
            let r = ui.interact(x, fid.with("clear"), Sense::click()).on_hover_text(t("Clear"));
            let c = if r.hovered() { p.text } else { p.text_faint };
            ui.painter().text(x.center(), Align2::CENTER_CENTER, icons::X_CIRCLE, style::icon_fill(14.0), c);
            if r.clicked() {
                self.project_filter.clear();
                sel = 0;
                ui.memory_mut(|m| m.request_focus(fid));
            }
        }
        self.project_filter_sel = sel;
        if let Some(id) = open {
            self.actions.push(Action::SelectProject(id));
        }
        ui.memory(|m| m.has_focus(fid))
    }

    /// Settings at the foot, ⌘, at its right.
    fn sidebar_footer(&mut self, ui: &mut Ui, rect: Rect, sc: SidebarScale) {
        let p = style::pal(ui);
        let label = t("Settings");
        let g = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(sc.body), p.text_muted);
        let b = Rect::from_min_size(rect.min, vec2(8.0 + sc.icon + 10.0 + g.size().x + 8.0, sc.foot_h));
        let r = ui.interact(b, egui::Id::new("sidebar-settings"), Sense::click());
        #[cfg(debug_assertions)]
        crate::devtest::record("sidebar-settings", b);
        let painter = ui.painter();
        let hot = r.hovered();
        if hot {
            painter.rect_filled(b, CornerRadius::same(style::RADIUS_ROW), p.hover);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let ink = if hot { p.text } else { p.text_muted };
        painter.text(pos2(b.left() + 8.0 + sc.icon / 2.0, b.center().y), Align2::CENTER_CENTER, icons::GEAR_SIX, style::icon_font(sc.icon), ink);
        painter.galley(pos2(b.left() + 8.0 + sc.icon + 10.0, b.center().y - g.size().y / 2.0), g, ink);
        if let Some(k) = kit::keys(&Action::Settings) {
            painter.text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, k, style::mono(sc.data), p.text_ghost);
        }
        if r.clicked() {
            self.actions.push(Action::Settings);
        }
    }

    /// The account the meter at the foot shows: the one the agent in view uses, else an active or
    /// highest-used account whose usage is known.
    fn usage_meter_data(&self) -> Option<UsageMeter> {
        let profiles = self.effective_profiles();
        let pick_id = self.current_pane().and_then(|id| {
            let pane = self.panes.get(&id)?;
            if let Some(agent) = pane.tracker.agent.or(pane.last_agent) {
                self.resolved_pane_profile_id(pane, agent, &profiles)
            } else {
                pane.active_profile.clone()
            }
        });
        let mut best: Option<(&tc_core::model::AgentProfile, &crate::profiles::ProfileRuntime, Option<f32>, bool, f32)> = None;
        for pr in profiles.iter().filter(|pr| pr.enabled) {
            let Some(rt) = self.profiles_rt.get(&pr.id) else { continue };
            let p5 = rt.quota_5h_used_pct(pr);
            let pw = rt.quota_weekly_used_pct(pr);
            let pct = p5.or(pw);
            let has_tokens = rt.usage.window_5h.tokens.total() > 0
                || rt.usage.window_weekly.tokens.total() > 0
                || rt.usage.window_5h.turns > 0
                || rt.usage.window_weekly.turns > 0;
            if pct.is_none() && !has_tokens {
                continue;
            }
            let running = matches!(
                rt.state,
                crate::profiles::ProfileState::Running | crate::profiles::ProfileState::RateLimited
            );
            let score = p5.unwrap_or(0.0).max(pw.unwrap_or(0.0));
            if pick_id.as_deref() == Some(pr.id.as_str()) {
                best = Some((pr, rt, pct, running, score));
                break;
            }
            let better = match best {
                None => true,
                Some((_, _, b_pct, b_run, b_score)) => {
                    (running, pct.is_some(), score) > (b_run, b_pct.is_some(), b_score)
                }
            };
            if better {
                best = Some((pr, rt, pct, running, score));
            }
        }
        let (pr, rt, pct, _, _) = best?;
        let now = std::time::Instant::now();
        let now_unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let short = rt.quota_5h_used_pct(pr).is_some()
            || (pct.is_none() && (rt.usage.window_5h.tokens.total() > 0 || rt.usage.window_5h.turns > 0));
        let (window, reset, tok, turns) = if short {
            (
                rt.window_5h_label(),
                rt.window_5h_reset_secs(now_unix, now),
                rt.usage.window_5h.tokens.total(),
                rt.usage.window_5h.turns,
            )
        } else {
            (
                rt.window_weekly_label(),
                rt.window_weekly_reset_secs(now_unix),
                rt.usage.window_weekly.tokens.total(),
                rt.usage.window_weekly.turns,
            )
        };
        let metric = if let Some(v) = pct {
            format!("{}%", v.round() as u32)
        } else if tok > 0 {
            tf("{} tok", &[&crate::profiles::format_tokens(tok)])
        } else {
            tf("{} turns", &[&turns.to_string()])
        };
        Some(UsageMeter {
            agent: pr.agent,
            label: format!("{} · {}", short_agent(pr.agent), pr.id),
            pct,
            metric,
            window,
            reset: reset.map(|s| tf("resets {}", &[&crate::profiles::format_countdown(s)])),
            limited: matches!(rt.state, crate::profiles::ProfileState::RateLimited | crate::profiles::ProfileState::Cooldown),
        })
    }

    /// The meter: the agent's mark, "Claude · claude1", how much is used, a bar, the window and
    /// when it resets. A click opens Accounts & usage.
    fn usage_meter(&mut self, ui: &mut Ui, rect: Rect, m: &UsageMeter, sc: SidebarScale) {
        let p = style::pal(ui);
        let r = ui.interact(rect, egui::Id::new("usage-meter"), Sense::click());
        #[cfg(debug_assertions)]
        crate::devtest::record("usage-meter", rect);
        let painter = ui.painter();
        let open = self.monitor_popover;
        painter.rect_filled(rect, CornerRadius::same(8), if r.hovered() || open { p.subtle } else { p.hover });
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let inner = rect.shrink(10.0);
        let av = sc.icon;
        let row1 = inner.top() + av / 2.0;
        kit::paint_agent_avatar(painter, Rect::from_center_size(pos2(inner.left() + av / 2.0, row1), vec2(av, av)), Some(m.agent), &p);
        let pg = painter.layout_no_wrap(m.metric.clone(), style::mono(sc.data), p.text_muted);
        let pw = pg.size().x;
        painter.galley(pos2(inner.right() - pw, row1 - pg.size().y / 2.0), pg, p.text_muted);
        let mut job = egui::text::LayoutJob::simple_singleline(m.label.clone(), style::medium(sc.small), p.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((inner.width() - (av + 8.0) - pw - 8.0).max(20.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(inner.left() + av + 8.0, row1 - g.size().y / 2.0), g, p.text);
        let track_y = inner.center().y;
        let row3 = inner.bottom() - sc.label / 2.0;
        if let Some(pct) = m.pct {
            let track = Rect::from_min_size(pos2(inner.left(), track_y - 2.0), vec2(inner.width(), 4.0));
            painter.rect_filled(track, CornerRadius::same(2), p.subtle);
            let fill_c = if m.limited || pct >= 90.0 { p.rate_limited } else { p.accent };
            let fw = (track.width() * (pct / 100.0).clamp(0.0, 1.0)).max(if pct > 0.0 { 4.0 } else { 0.0 });
            painter.rect_filled(Rect::from_min_size(track.min, vec2(fw, 4.0)), CornerRadius::same(2), fill_c);
            painter.text(pos2(inner.left(), row3), Align2::LEFT_CENTER, tf("{} window", &[&m.window]), egui::FontId::proportional(sc.label), p.text_faint);
            if let Some(reset) = &m.reset {
                painter.text(pos2(inner.right(), row3), Align2::RIGHT_CENTER, reset, egui::FontId::proportional(sc.label), p.text_faint);
            }
        } else {
            // Provider does not expose server quota % (e.g. Antigravity CLI): draw a clean divider
            // instead of a misleading empty 0% progress bar.
            painter.hline(inner.left()..=inner.right(), track_y, Stroke::new(1.0, p.border));
            painter.text(pos2(inner.left(), row3), Align2::LEFT_CENTER, tf("{} local", &[&m.window]), egui::FontId::proportional(sc.label), p.text_faint);
            let right = m.reset.as_deref().unwrap_or_else(|| t("Local only"));
            painter.text(pos2(inner.right(), row3), Align2::RIGHT_CENTER, right, egui::FontId::proportional(sc.label), p.text_faint);
        }
        if r.on_hover_text(t("Accounts & usage")).clicked() {
            self.monitor_popover = !self.monitor_popover;
            if self.monitor_popover {
                self.request_usage_refresh(std::time::Instant::now());
            }
        }
        self.monitor_pill_rect = Some(rect);
    }

    /// The projects: "Projects", how many, Search and Add; then each project in its place, the
    /// selected one with its sessions and worktrees under it. While searching: what was found,
    /// the keyboard's row with ↵.
    fn project_list(&mut self, ui: &mut Ui, searching: bool, sc: SidebarScale) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let area = ui.max_rect();
        let total = self.projects.iter().filter(|x| !x.is_worktree()).count();
        let fid = egui::Id::new("project-filter");
        let typing = ui.memory(|m| m.has_focus(fid));

        // Header.
        let head = Rect::from_min_size(pos2(area.left() + PAD, area.top()), vec2(area.width() - 2.0 * PAD, SECTION_H));
        let g = kit::section_galley(ui.painter(), t("Projects"), p.text_faint);
        let gw = g.size().x;
        ui.painter().galley(pos2(head.left() + 8.0, head.center().y - g.size().y / 2.0), g, p.text_faint);
        if !searching {
            ui.painter().text(pos2(head.left() + 8.0 + gw + 6.0, head.center().y), Align2::LEFT_CENTER, total.to_string(), style::mono(style::DATA), p.text_ghost);
            let add = Rect::from_center_size(pos2(head.right() - 6.0 - 11.0, head.center().y), vec2(22.0, 22.0));
            #[cfg(debug_assertions)]
            crate::devtest::record("add-project", add);
            let r = ui.scope_builder(egui::UiBuilder::new().max_rect(add), |ui| kit::icon_button_ex(ui, icons::PLUS, &kit::tip(t("Add Project…"), Action::AddProject), false, 22.0)).inner;
            if r.clicked() {
                self.actions.push(Action::AddProject);
            }
            if total > 0 {
                let find = add.translate(vec2(-24.0, 0.0));
                let r = ui.scope_builder(egui::UiBuilder::new().max_rect(find), |ui| kit::icon_button_ex(ui, icons::MAGNIFYING_GLASS, t("Search Projects"), false, 22.0)).inner;
                if r.clicked() {
                    ui.memory_mut(|m| m.request_focus(fid));
                }
            }
        }
        let list = Rect::from_min_max(pos2(area.left(), head.bottom() + 2.0), area.max);
        if total == 0 {
            self.no_projects_card(ui, Rect::from_min_size(pos2(area.left() + PAD, list.top() + 2.0), vec2(area.width() - 2.0 * PAD, 150.0)));
            self.project_filter_rows.clear();
            return;
        }

        // The lines: while searching, flat best-match results; otherwise grouped with Active
        // projects at the top and Idle projects below. While the pointer is over the list
        // nothing moves between groups so rows never jump under the cursor.
        let query = self.project_filter.trim().to_string();
        let filtering = !query.is_empty();
        let hold = ui.rect_contains_pointer(list) || typing;
        let active_ids = self.sidebar_active(hold);
        let now = std::time::Instant::now();
        let in_focus = !filtering && self.view == View::Focus && !self.attention_open;
        let mut lines: Vec<Line> = Vec::new();
        let mut counts = (0usize, 0usize);
        let mut summary = [0usize; 4];
        if filtering {
            let matches = self.project_matches();
            for (idx, (i, dim)) in matches.iter().copied().enumerate() {
                let pr = &self.projects[i];
                let marks = if dim { Vec::new() } else { super::palette::matched(&query, &pr.name) };
                if pr.is_worktree() {
                    let is_last = !matches.get(idx + 1).is_some_and(|(ni, _)| self.projects[*ni].is_worktree());
                    let wt_active = self.project_activity(pr.id, now) == super::grid::Activity::Active;
                    lines.push(Line::Worktree {
                        idx: i,
                        marks,
                        is_last,
                        session_count: 0,
                        children_expanded: false,
                        is_active: wt_active,
                    });
                } else {
                    lines.push(Line::Project {
                        idx: i,
                        marks,
                        dim,
                        is_active: active_ids.contains(&pr.id),
                        expanded: false,
                        parent_of_selected: false,
                    });
                }
            }
        } else {
            let active_root = self.active.and_then(|id| self.root_project(id)).map(|x| x.id);
            let roots: Vec<usize> = self
                .projects
                .iter()
                .enumerate()
                .filter(|(_, x)| !x.is_worktree() || x.parent.and_then(|id| self.project(id)).is_none())
                .map(|(i, _)| i)
                .collect();
            let (active_roots, idle_roots): (Vec<usize>, Vec<usize>) =
                roots.into_iter().partition(|&i| active_ids.contains(&self.projects[i].id));
            self.sidebar_order = active_roots
                .iter()
                .chain(idle_roots.iter())
                .map(|&i| self.projects[i].id)
                .collect();
            counts = (active_roots.len(), idle_roots.len());
            if !active_roots.is_empty() {
                summary = self.group_summary(&active_ids);
            }
            let is_live_pane = |pane_id: tc_core::PaneId| -> bool {
                self.panes.get(&pane_id).is_some_and(|x| {
                    x.tracker.agent.is_some()
                        || x.last_agent.is_some()
                        || x.resume.is_some()
                        || shows_status(x.tracker.status)
                        || matches!(x.tracker.fg, tc_core::status::Foreground::Program { .. })
                })
            };
            let collect_panes = |pr: &crate::project::Project| -> Vec<(usize, tc_core::PaneId, bool)> {
                let mut rows = Vec::new();
                for (k, tab) in pr.tabs.iter().enumerate() {
                    let tab_panes: Vec<tc_core::PaneId> = tab
                        .layout
                        .panes()
                        .into_iter()
                        .filter(|id| !self.browsers.contains_key(id))
                        .collect();
                    for (p_idx, pane_id) in tab_panes.into_iter().enumerate() {
                        rows.push((k, pane_id, p_idx == 0));
                    }
                }
                rows
            };
            let push_project = |lines: &mut Vec<Line>, i: usize, is_active: bool| {
                let pr = &self.projects[i];
                let root_selected = self.active == Some(pr.id);
                let in_subtree = active_root == Some(pr.id);
                // Expand ALL Active projects (so every running agent across every active project
                // or worktree is listed immediately), plus the currently selected subtree even when Idle.
                let expanded = is_active || in_subtree;
                let parent_of_selected = in_focus && in_subtree && !root_selected;
                lines.push(Line::Project {
                    idx: i,
                    marks: Vec::new(),
                    dim: false,
                    is_active,
                    expanded,
                    parent_of_selected,
                });
                if !expanded {
                    return;
                }
                let wts: Vec<usize> = self
                    .projects
                    .iter()
                    .enumerate()
                    .filter(|(_, x)| x.parent == Some(pr.id))
                    .map(|(j, _)| j)
                    .collect();
                let mut rows = collect_panes(pr);
                // When root itself isn't selected (either a worktree of this project is focused,
                // or another project is focused while this one is Active), list only root's live
                // / agent panes — hiding idle `zsh` noise above its worktrees.
                if !root_selected {
                    let agent_rows: Vec<(usize, tc_core::PaneId, bool)> =
                        rows.iter().copied().filter(|(_, pane_id, _)| is_live_pane(*pane_id)).collect();
                    if !agent_rows.is_empty() {
                        rows = agent_rows;
                    } else if !wts.is_empty() {
                        rows.clear();
                    } else {
                        rows.truncate(1);
                    }
                }
                let row_count = rows.len();
                for (idx, (k, pane_id, first_in_tab)) in rows.into_iter().enumerate() {
                    lines.push(Line::Session {
                        project: pr.id,
                        tab: k,
                        pane: pane_id,
                        first_in_tab,
                        is_last: wts.is_empty() && idx + 1 == row_count,
                        in_worktree: false,
                        outer_continues: false,
                    });
                }
                let wt_count = wts.len();
                for (pos, j) in wts.into_iter().enumerate() {
                    let wt = &self.projects[j];
                    let wt_is_last = pos + 1 == wt_count;
                    let wt_selected = self.active == Some(wt.id);
                    let wt_all_rows = collect_panes(wt);
                    let wt_live_rows: Vec<(usize, tc_core::PaneId, bool)> =
                        wt_all_rows.iter().copied().filter(|(_, pid, _)| is_live_pane(*pid)).collect();
                    let wt_active = !wt_live_rows.is_empty()
                        || self.project_activity(wt.id, now) == super::grid::Activity::Active;
                    let wt_rows = if wt_selected {
                        wt_all_rows
                    } else if wt_active {
                        wt_live_rows
                    } else {
                        Vec::new()
                    };
                    let session_count = wt_rows.len();
                    let children_expanded = session_count > 0 && !wt.folded;
                    lines.push(Line::Worktree {
                        idx: j,
                        marks: Vec::new(),
                        is_last: wt_is_last,
                        session_count,
                        children_expanded,
                        is_active: wt_active,
                    });
                    if children_expanded {
                        let s_count = wt_rows.len();
                        for (s_idx, (k, pane_id, first_in_tab)) in wt_rows.into_iter().enumerate() {
                            lines.push(Line::Session {
                                project: wt.id,
                                tab: k,
                                pane: pane_id,
                                first_in_tab,
                                is_last: s_idx + 1 == s_count,
                                in_worktree: true,
                                outer_continues: !wt_is_last,
                            });
                        }
                    }
                }
            };
            if !active_roots.is_empty() {
                lines.push(Line::Group(true));
                if !self.sidebar_folded.contains("active") {
                    for i in active_roots {
                        push_project(&mut lines, i, true);
                    }
                }
            }
            if !idle_roots.is_empty() {
                lines.push(Line::Group(false));
                if !self.sidebar_folded.contains("idle") {
                    for i in idle_roots {
                        push_project(&mut lines, i, false);
                    }
                }
            }
        }
        if filtering && lines.is_empty() {
            ui.painter().text(pos2(list.left() + PAD + 8.0, list.top() + 14.0), Align2::LEFT_CENTER, t("No matching projects."), egui::FontId::proportional(style::SMALL), p.text_faint);
            self.project_filter_rows.clear();
            return;
        }
        let sel = self.project_filter_sel;
        let reveal = std::mem::take(&mut self.project_filter_reveal);
        let top_indices: std::collections::HashMap<tc_core::ProjectId, usize> = self.top_order().into_iter().enumerate().map(|(idx, pid)| (pid, idx)).collect();
        let mut order = Vec::new();
        let mut pick: Option<Action> = None;
        let mut toggle_group: Option<&'static str> = None;
        let mut toggle_wt_fold: Option<tc_core::ProjectId> = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink2(vec2(PAD, 0.0))), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(if filtering { "project-list-found" } else { "project-list" })
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let mut found = 0usize;
                    let mut groups_seen = 0usize;
                    for line in &lines {
                        match line {
                            Line::Group(is_active) => {
                                if groups_seen > 0 {
                                    let (sep_r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
                                    if ui.is_rect_visible(sep_r) {
                                        ui.painter().hline(
                                            (sep_r.left() + 6.0)..=(sep_r.right() - 6.0),
                                            sep_r.center().y,
                                            Stroke::new(1.0, p.border),
                                        );
                                    }
                                }
                                groups_seen += 1;
                                let key = if *is_active { "active" } else { "idle" };
                                let folded = self.sidebar_folded.contains(key);
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), sc.group_h), Sense::click());
                                if ui.is_rect_visible(rect) {
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record(format!("sidebar-group:{key}"), rect.intersect(ui.clip_rect()));
                                    let (label, count) = if *is_active { (t("Active"), counts.0) } else { (t("Idle"), counts.1) };
                                    let group_sum: &[(Status, usize)] = if *is_active {
                                        &[(Status::Running, summary[0]), (Status::NeedsInput, summary[3]), (Status::Done, summary[1]), (Status::Failed(1), summary[2])]
                                    } else {
                                        &[]
                                    };
                                    paint_group_head(ui, rect, label, count, folded, resp.hovered(), *is_active, group_sum, dark, sc, &p);
                                }
                                let resp = if *is_active {
                                    resp.on_hover_text(t("Only projects where something runs or waits, or an agent can be resumed"))
                                } else {
                                    resp
                                };
                                if resp.clicked() {
                                    toggle_group = Some(key);
                                }
                            }
                            Line::Project { idx, marks, dim, is_active, expanded, parent_of_selected } => {
                                let pr = &self.projects[*idx];
                                let id = pr.id;
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), sc.project_h), Sense::click());
                                let chosen = typing && !dim && found == sel;
                                if !dim {
                                    found += 1;
                                    order.push(id);
                                }
                                if chosen && reveal {
                                    ui.scroll_to_rect(rect, None);
                                }
                                if !ui.is_rect_visible(rect) {
                                    continue;
                                }
                                #[cfg(debug_assertions)]
                                crate::devtest::record(format!("project:{}", pr.name), rect.intersect(ui.clip_rect()));
                                let selected = in_focus && self.active == Some(id);
                                let row = self.project_row(pr, &p);
                                paint_project_row(
                                    ui,
                                    rect,
                                    &row,
                                    marks,
                                    selected,
                                    *parent_of_selected,
                                    *expanded,
                                    resp.hovered() || chosen,
                                    *is_active,
                                    *dim,
                                    chosen,
                                    dark,
                                    sc,
                                    &p,
                                );
                                if resp.clicked() {
                                    pick = Some(Action::SelectProject(id));
                                }
                                let name = row.name.clone();
                                let index = top_indices.get(&id).copied().unwrap_or(usize::MAX);
                                let resp = resp.on_hover_ui(|ui| self.project_tooltip(ui, id, index));
                                resp.context_menu(|ui| self.project_menu(ui, id, &name));
                            }
                            Line::Worktree { idx, marks, is_last, session_count, children_expanded, is_active } => {
                                let pr = &self.projects[*idx];
                                let id = pr.id;
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), sc.session_h), Sense::click());
                                let chosen = typing && found == sel;
                                found += 1;
                                order.push(id);
                                if chosen && reveal {
                                    ui.scroll_to_rect(rect, None);
                                }
                                if !ui.is_rect_visible(rect) {
                                    continue;
                                }
                                #[cfg(debug_assertions)]
                                crate::devtest::record(format!("project:{}", pr.name), rect.intersect(ui.clip_rect()));
                                // Register the fold caret hitbox unconditionally whenever expandable
                                // so hovering never creates/destroys hitboxes (zero hover flicker).
                                let caret_resp = if *session_count > 0 {
                                    let caret_rect = Rect::from_min_max(pos2(rect.right() - 22.0, rect.top()), rect.max);
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record(format!("wt-fold:{}", pr.name), caret_rect.intersect(ui.clip_rect()));
                                    Some(ui.interact(caret_rect, ui.id().with(("wt-fold", id)), Sense::click()))
                                } else {
                                    None
                                };
                                let caret_hovered = caret_resp.as_ref().is_some_and(|r| r.hovered());
                                let row_hovered = resp.hovered() || caret_hovered || chosen;
                                let selected = in_focus && self.active == Some(id);
                                let st = self.project_status(pr).map(|(s, _)| s).filter(|s| shows_status(*s));
                                let asks = pr
                                    .panes()
                                    .iter()
                                    .filter_map(|pid| self.panes.get(pid))
                                    .filter(|pane| pane.tracker.status == Status::NeedsInput)
                                    .count();
                                let (branch, changed, ahead, merged) = pr
                                    .git
                                    .as_ref()
                                    .map_or((String::new(), 0, 0, false), |g| (g.branch.clone(), g.changed, g.ahead, g.merged));
                                let info = WorktreeRowInfo {
                                    name: &pr.name,
                                    branch: &branch,
                                    branch_prefix: &self.settings.worktrees.prefix,
                                    marks,
                                    status: st,
                                    asks,
                                    changed,
                                    ahead,
                                    merged,
                                    missing: pr.missing,
                                    selected,
                                    hovered: row_hovered,
                                    caret_hovered,
                                    is_active: *is_active,
                                    is_last: *is_last,
                                    session_count: *session_count,
                                    children_expanded: *children_expanded,
                                };
                                paint_worktree_row(ui, rect, &info, dark, sc, &p);
                                let caret_clicked = if let Some(cr) = caret_resp {
                                    let tip = if *children_expanded { t("Collapse sessions") } else { t("Expand sessions") };
                                    cr.on_hover_text(tip).clicked()
                                } else {
                                    false
                                };
                                if caret_clicked {
                                    toggle_wt_fold = Some(id);
                                } else if resp.clicked() {
                                    pick = Some(Action::SelectProject(id));
                                }
                                let resp = resp.on_hover_ui(|ui| self.worktree_tooltip(ui, id));
                                resp.context_menu(|ui| self.worktree_menu(ui, id));
                            }
                            Line::Session { project, tab, pane, first_in_tab, is_last, in_worktree, outer_continues } => {
                                let _ = first_in_tab;
                                let row_h = if *in_worktree { (sc.session_h - 3.0).max(23.0) } else { sc.session_h };
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                                if !ui.is_rect_visible(rect) {
                                    continue;
                                }
                                #[cfg(debug_assertions)]
                                {
                                    let vis = rect.intersect(ui.clip_rect());
                                    if *first_in_tab {
                                        crate::devtest::record(format!("session:{project}:{tab}"), vis);
                                    }
                                    crate::devtest::record(format!("session-pane:{pane}"), vis);
                                }
                                let Some(s) = self.session_row(*project, *tab, *pane) else { continue };
                                paint_session_row(ui, rect, &s, resp.hovered(), *is_last, *in_worktree, *outer_continues, dark, sc, &p);
                                if resp.on_hover_text(&s.title).clicked() {
                                    pick = Some(Action::FocusPane(*pane));
                                }
                            }
                        }
                    }
                });
        });
        self.project_filter_rows = order;
        if let Some(key) = toggle_group {
            if !self.sidebar_folded.remove(key) {
                self.sidebar_folded.insert(key.to_string());
            }
            self.mark_dirty();
        }
        if let Some(wt_id) = toggle_wt_fold {
            if let Some(pr) = self.project_mut(wt_id) {
                pr.folded = !pr.folded;
            }
            self.mark_dirty();
        }
        if let Some(a) = pick {
            if self.attention_open {
                self.attention_open = false;
            }
            self.actions.push(a);
        }
    }

    /// Root projects currently under the sidebar's Active group: something runs, waits, or can be
    /// resumed in them or their worktrees now (the Grid's rule), or did less than `active_linger`
    /// ago. While `hold` (pointer over the list, or typing in search) the set stays put so rows
    /// never jump under the cursor.
    fn sidebar_active(&mut self, hold: bool) -> std::collections::HashSet<tc_core::ProjectId> {
        use super::grid::Activity;
        let now = std::time::Instant::now();
        let linger = active_linger();
        let mut wake: Option<std::time::Duration> = None;
        for k in 0..self.projects.len() {
            let pr = &self.projects[k];
            let top = pr.parent.filter(|x| self.project(*x).is_some()).unwrap_or(pr.id);
            match self.project_activity(pr.id, now) {
                Activity::Active => {
                    self.sidebar_active_until.insert(top, now + linger);
                }
                Activity::Soon(d) => wake = Some(wake.map_or(d, |w| w.min(d))),
                Activity::Idle => {}
            }
        }
        let projects = &self.projects;
        self.sidebar_active_until
            .retain(|id, until| *until > now && projects.iter().any(|p| p.id == *id && !p.is_worktree()));
        if let Some(d) = self.sidebar_active_until.values().map(|u| *u - now).min() {
            wake = Some(wake.map_or(d, |w| w.min(d)));
        }
        if let Some(d) = wake {
            self.ctx.request_repaint_after(d);
        }
        let live: std::collections::HashSet<tc_core::ProjectId> = self.sidebar_active_until.keys().copied().collect();
        if hold {
            self.sidebar_hold.get_or_insert(live).clone()
        } else {
            self.sidebar_hold = None;
            live
        }
    }

    /// How many Active projects are, at their most pressing: running, done, failed, waiting
    /// for the user (in that order). Quiet/lingering projects contribute nothing.
    fn group_summary(&self, ids: &std::collections::HashSet<tc_core::ProjectId>) -> [usize; 4] {
        let mut out = [0; 4];
        for &id in ids {
            let mut rank: Option<usize> = None;
            for pid in std::iter::once(id).chain(self.worktrees_of(id)) {
                let Some(pr) = self.project(pid) else { continue };
                for pane in pr.panes().iter().filter_map(|x| self.panes.get(x)) {
                    let cand = match pane.tracker.status {
                        Status::NeedsInput => Some(3),
                        Status::Failed(_) if pane.tracker.wants_attention() => Some(2),
                        Status::Done | Status::CommandDone if pane.tracker.wants_attention() => Some(1),
                        Status::Running => Some(0),
                        _ => None,
                    };
                    if let Some(r) = cand {
                        rank = Some(rank.map_or(r, |cur| cur.max(r)));
                    }
                }
            }
            if let Some(r) = rank {
                out[r] += 1;
            }
        }
        out
    }

    /// A pane inside a project's tab as a sidebar session row: who works there, its title/task,
    /// how long it has been at it (or the port its server listens on), and its own live status.
    fn session_row(&self, project: tc_core::ProjectId, k: usize, pane_id: tc_core::PaneId) -> Option<SessionRow> {
        let pr = self.project(project)?;
        let tab = pr.tabs.get(k)?;
        let pane = self.panes.get(&pane_id);
        let single_in_tab = tab
            .layout
            .panes()
            .into_iter()
            .filter(|id| !self.browsers.contains_key(id))
            .count()
            == 1;
        let pane_title = |p: &crate::pane::Pane| -> String {
            if p.title.is_none()
                && p.tracker.agent.is_none()
                && !matches!(p.tracker.fg, tc_core::status::Foreground::Program { .. })
                && let Some(a) = p.resume.as_ref().map(|(a, _)| *a).or(p.last_agent)
            {
                return a.label().to_string();
            }
            p.name()
        };
        let title = if single_in_tab {
            tab.title
                .clone()
                .unwrap_or_else(|| pane.map(pane_title).unwrap_or_else(|| self.leaf_label(pane_id)))
        } else {
            pane.map(pane_title).unwrap_or_else(|| self.leaf_label(pane_id))
        };
        let status = pane.map(|x| x.tracker.status).filter(|s| shows_status(*s));
        let server = pane.and_then(|p| p.live_servers().min_by_key(|s| s.port).map(|s| s.url.as_str()));
        let meta = match (server, pane) {
            (Some(url), _) => super::focus::server_label(url).rsplit_once(':').map_or_else(|| super::focus::server_label(url).to_string(), |(_, port)| format!(":{port}")),
            (None, Some(x)) if x.tracker.agent.is_some() && !matches!(x.tracker.status, Status::Idle | Status::Unknown | Status::Ended) => short_age(x.tracker.since.elapsed().as_secs()),
            _ => String::new(),
        };
        Some(SessionRow {
            title,
            agent: pane.and_then(|x| x.tracker.agent.or(x.last_agent)),
            meta,
            status,
            on: self.view == View::Focus && self.active == Some(project) && pr.active_tab == k && tab.focused == pane_id && !self.attention_open,
        })
    }

    /// No projects yet: a dashed card with Add project.
    fn no_projects_card(&mut self, ui: &mut Ui, rect: Rect) {
        let p = style::pal(ui);
        let painter = ui.painter();
        kit::dashed_rect(painter, rect, 10.0, p.border_strong);
        let cx = rect.center().x;
        let mut y = rect.top() + 24.0;
        painter.text(pos2(cx, y + 10.0), Align2::CENTER_CENTER, icons::FOLDER_SIMPLE_PLUS, style::icon_font(20.0), p.text_faint);
        y += 20.0 + 8.0;
        painter.text(pos2(cx, y + 9.0), Align2::CENTER_CENTER, t("No projects yet"), style::medium(style::BODY), p.text);
        y += 18.0 + 8.0;
        let mut job = egui::text::LayoutJob::simple(t("Drop a folder here, or add one.").to_string(), egui::FontId::proportional(style::SMALL), p.text_muted, 180.0);
        job.halign = egui::Align::Center;
        let g = painter.layout_job(job);
        let gh = g.size().y;
        painter.galley(pos2(cx, y), g, p.text_muted);
        y += gh + 8.0;
        let keys = kit::keys(&Action::AddProject);
        let w = kit::button_width(ui, None, t("Add project"), keys, true);
        let b = Rect::from_min_size(pos2(cx - w / 2.0, y), vec2(w, kit::BUTTON_S));
        let r = ui.scope_builder(egui::UiBuilder::new().max_rect(b), |ui| kit::button_keys(ui, kit::Kind::Secondary, None, t("Add project"), keys, true)).inner;
        #[cfg(debug_assertions)]
        crate::devtest::record("add-project-card", r.rect);
        if r.clicked() {
            self.actions.push(Action::AddProject);
        }
    }

    /// The narrow sidebar: Search, Inbox (its count on it), Overview, then a tile per project
    /// (the selected one raised, a dot when it waits, failed or finished unseen), Settings at
    /// the foot. Hovering a tile names it.
    pub fn sidebar_compact(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let full = ui.max_rect();
        let title = Rect::from_min_size(full.min, vec2(full.width(), TITLE_H));
        super::view::window_drag_area(ui, title, "sidebar-top");
        let cx = full.center().x;
        let cell = |y: f32| Rect::from_min_size(pos2(cx - 18.0, y), vec2(36.0, 32.0));
        let mut y = title.bottom();
        // Search (the full sidebar comes back with the field in it).
        let r = rail_button(ui, cell(y), icons::MAGNIFYING_GLASS, &kit::tip(t("Search"), Action::Palette), false, true, &p);
        if r.clicked() {
            self.actions.push(Action::Palette);
        }
        y += 32.0 + 8.0;
        let waiting = self.attention_queue().len() + self.unreviewed_automation_alerts_count();
        let r = rail_button(ui, cell(y), icons::TRAY, &kit::tip(t("Inbox"), Action::ToggleAttention), self.view == View::Inbox, false, &p);
        #[cfg(debug_assertions)]
        crate::devtest::record("nav:inbox", r.rect);
        if waiting > 0 {
            let text = waiting.to_string();
            let g = ui.painter().layout_no_wrap(text, style::mono_semibold(style::DATA - 1.0), egui::Color32::from_rgb(0x11, 0x11, 0x13));
            let w = (g.size().x + 6.0).max(14.0);
            let badge = Rect::from_min_size(pos2(r.rect.left() + 22.0, r.rect.top() - 3.0), vec2(w, 14.0));
            ui.painter().rect_filled(badge, CornerRadius::same(7), p.warn);
            ui.painter().galley(badge.center() - g.size() / 2.0, g, egui::Color32::BLACK);
        }
        if r.clicked() {
            self.actions.push(Action::ToggleAttention);
        }
        y += 32.0 + 8.0;
        let r = rail_button(ui, cell(y), icons::SQUARES_FOUR, &kit::tip(t("Overview"), Action::ToggleGrid), self.view == View::Grid && !self.attention_open, false, &p);
        #[cfg(debug_assertions)]
        crate::devtest::record("nav:overview", r.rect);
        if r.clicked() {
            self.actions.push(Action::SetView(if self.view == View::Grid { View::Focus } else { View::Grid }));
        }
        y += 32.0 + 8.0;
        let r = rail_button(ui, cell(y), icons::CLOCK, &kit::tip(t("Automations"), Action::ToggleAutomations), self.view == View::Automations, false, &p);
        #[cfg(debug_assertions)]
        crate::devtest::record("nav:automations", r.rect);
        let auto_running = self.running_automations_count();
        let auto_alerts = self.unreviewed_automation_alerts_count();
        if auto_running > 0 || auto_alerts > 0 {
            let dot_col = if auto_running > 0 { p.info } else { p.warn };
            ui.painter().circle_filled(pos2(r.rect.right() - 6.0, r.rect.top() + 6.0), 4.0, dot_col);
        }
        if r.clicked() {
            self.actions.push(Action::ToggleAutomations);
        }
        y += 32.0 + 8.0;
        ui.painter().hline((cx - 14.0)..=(cx + 14.0), y + 0.5, Stroke::new(1.0, p.border));
        y += 1.0 + 8.0;
        let gear = Rect::from_min_size(pos2(cx - 18.0, full.bottom() - 10.0 - 32.0), vec2(36.0, 32.0));
        let r = rail_button(ui, gear, icons::GEAR_SIX, &kit::tip(t("Settings"), Action::Settings), false, false, &p);
        #[cfg(debug_assertions)]
        crate::devtest::record("sidebar-settings", gear);
        if r.clicked() {
            self.actions.push(Action::Settings);
        }
        // The projects (a worktree shows as its project), Active first, then Idle.
        let list = Rect::from_min_max(pos2(full.left(), y), pos2(full.right(), gear.top() - 8.0));
        let active_ids = self.sidebar_active(ui.rect_contains_pointer(list));
        let active_root = self.active.and_then(|id| self.root_project(id)).map(|x| x.id);
        let build_row = |x: &crate::project::Project| {
            // Its dot: the most urgent of it and its worktrees, when it asks for the user.
            let mut dot: Option<(u8, egui::Color32)> = None;
            let mut wt_count = 0usize;
            for pr in self.projects.iter().filter(|c| c.id == x.id || c.parent == Some(x.id)) {
                if pr.parent == Some(x.id) {
                    wt_count += 1;
                }
                if let Some((st, attention)) = self.project_status(pr) {
                    let cand = match st {
                        Status::NeedsInput => Some((3, p.warn)),
                        Status::Failed(_) => Some((2, p.err)),
                        Status::Done | Status::CommandDone if attention => Some((1, p.ok)),
                        _ => None,
                    };
                    if let Some((rank, c)) = cand
                        && dot.is_none_or(|(best, _)| rank > best)
                    {
                        dot = Some((rank, c));
                    }
                }
            }
            (x.id, x.name.clone(), dot.map(|(_, c)| c), active_ids.contains(&x.id), wt_count)
        };
        let (mut active_rows, idle_rows): (Vec<_>, Vec<_>) = self
            .projects
            .iter()
            .filter(|x| x.parent.is_none())
            .map(build_row)
            .partition(|r| r.3);
        let active_count = active_rows.len();
        active_rows.extend(idle_rows);
        self.sidebar_order = active_rows.iter().map(|r| r.0).collect();
        let rows = active_rows;
        let mut pick = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            ui.set_clip_rect(list.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt("sidebar-compact")
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    for (idx, (id, name, dot, is_active, wt_count)) in rows.iter().enumerate() {
                        if active_count > 0 && idx == active_count {
                            let (sep, _) = ui.allocate_exact_size(vec2(ui.available_width(), 5.0), Sense::hover());
                            ui.painter().hline((sep.center().x - 14.0)..=(sep.center().x + 14.0), sep.center().y, Stroke::new(1.0, p.border));
                        }
                        let (cell, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                        let on = active_root == Some(*id) && self.view == View::Focus && !self.attention_open;
                        let back = Rect::from_center_size(cell.center(), vec2(36.0, 36.0));
                        if on {
                            raised(ui.painter(), back, 8, &p);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(back, CornerRadius::same(8), p.hover);
                        }
                        let tile = Rect::from_center_size(cell.center(), vec2(19.0, 19.0));
                        let tone = if on {
                            kit::FolderTone::Selected
                        } else if resp.hovered() {
                            kit::FolderTone::Hover
                        } else if *is_active {
                            kit::FolderTone::Active
                        } else {
                            kit::FolderTone::Idle
                        };
                        kit::paint_project_folder(ui.painter(), tile, tone, dark);
                        if let Some(c) = dot {
                            let at = pos2(back.left() + 27.0 + 4.0, back.top() + 1.0 + 4.0);
                            ui.painter().circle_filled(at, 5.0, p.chrome);
                            ui.painter().circle_filled(at, 3.0, *c);
                        }
                        if *wt_count > 0 {
                            let at = pos2(back.right() - 6.0, back.bottom() - 6.0);
                            ui.painter().circle_filled(at, 5.5, if on { p.surface } else { p.chrome });
                            ui.painter().text(
                                at,
                                Align2::CENTER_CENTER,
                                icons::GIT_FORK,
                                style::icon_font(9.0),
                                if on || *is_active { p.accent_text } else { p.text_faint },
                            );
                        }
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("project:{name}"), cell);
                        let tip = if *wt_count > 0 {
                            format!("{name} · {}", crate::i18n::count(*wt_count, t("1 worktree"), t("{} worktrees")))
                        } else {
                            name.clone()
                        };
                        if resp.on_hover_text(tip).clicked() {
                            pick = Some(*id);
                        }
                    }
                });
        });
        if let Some(id) = pick {
            self.attention_open = false;
            self.actions.push(Action::SelectProject(id));
        }
    }

    /// The rows of the project list (indexes into `projects`, and whether a row is only there
    /// for a worktree found under it, dimmed). While the search is empty: every project, each
    /// followed by its worktrees unless they are folded away. While it holds letters: the
    /// projects whose name (or a worktree's) holds them in order, best first, each followed by
    /// its worktrees found.
    fn project_matches(&self) -> Vec<(usize, bool)> {
        let query = self.project_filter.trim();
        let top = |p: &crate::project::Project| p.parent.is_none_or(|x| self.project(x).is_none());
        if query.is_empty() {
            return self
                .projects
                .iter()
                .enumerate()
                .filter(|(_, p)| top(p) || !p.parent.and_then(|x| self.project(x)).is_some_and(|x| x.folded))
                .map(|(i, _)| (i, false))
                .collect();
        }
        let score = |i: usize| super::palette::score(query, &self.projects[i].name);
        let kids = |id: tc_core::ProjectId| self.projects.iter().enumerate().filter(move |(_, k)| k.parent == Some(id)).map(|(k, _)| k);
        let mut groups: Vec<(usize, i32, bool)> = Vec::new();
        for (i, pr) in self.projects.iter().enumerate().filter(|(_, p)| top(p)) {
            let own = score(i);
            let best = own.into_iter().chain(kids(pr.id).filter_map(score)).max();
            if let Some(b) = best {
                groups.push((i, b, own.is_some()));
            }
        }
        groups.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut out = Vec::new();
        for (i, _, own) in groups {
            out.push((i, !own));
            out.extend(kids(self.projects[i].id).filter(|k| score(*k).is_some()).map(|k| (k, false)));
        }
        out
    }

    /// A project's tooltip: where it is and what kind, its branch, and each pane's status with
    /// where that came from.
    fn project_tooltip(&self, ui: &mut Ui, id: u64, index: usize) {
        let p = style::pal(ui);
        let Some(pr) = self.project(id) else { return };
        ui.set_max_width(320.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(egui::RichText::new(&pr.name).font(style::semibold(12.5)).color(p.text));
        let mut place = crate::paths::contract_tilde(&pr.path);
        if !pr.kind.label().is_empty() {
            place = format!("{place} · {}", pr.kind.label());
        }
        ui.label(egui::RichText::new(place).size(style::SMALL).color(p.text_muted));
        if let Some(g) = &pr.git {
            ui.label(egui::RichText::new(format!("{} {}", icons::GIT_BRANCH, branch_text(g))).size(style::SMALL).color(p.text));
        }
        for pane in pr.panes().iter().filter_map(|x| self.panes.get(x)) {
            let s = pane.tracker.status;
            let quiet = matches!(s, Status::Idle | Status::Unknown | Status::Ended);
            let (icon, c) = if quiet { (icons::TERMINAL, p.text_faint) } else { (status::icon(s), status::color(s, &p)) };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(egui::RichText::new(icon).font(style::icon_font(12.0)).color(c));
                ui.label(egui::RichText::new(format!("{} · {}", pane.label(), status::text(s))).size(style::SMALL).color(p.text));
            });
            if pane.tracker.source != tc_core::status::Source::None && !quiet {
                ui.label(
                    egui::RichText::new(crate::i18n::tf("from {}, {} ago", &[&status::source(pane.tracker.source), &status::age(pane.tracker.since)]))
                        .size(style::SMALL)
                        .color(p.text_faint),
                );
            }
        }
        let asleep = pr.panes().iter().all(|x| self.panes.get(x).is_none_or(crate::pane::Pane::dormant));
        if asleep {
            ui.label(egui::RichText::new(t("Shell not started yet (starts when the project is shown)")).size(style::SMALL).color(p.text_faint));
        }
        let keys = kit::keys(&Action::SelectProjectIndex(index)).map(|k| format!("{k} · ")).unwrap_or_default();
        ui.label(egui::RichText::new(format!("{keys}{}", t("Right-click for more"))).size(style::SMALL).color(p.text_faint));
    }

    /// A worktree's tooltip: full branch (since the row deduplicates `tc/<name>`), base branch,
    /// path, ahead/changed/merged state, and each pane's status.
    fn worktree_tooltip(&self, ui: &mut Ui, id: u64) {
        let p = style::pal(ui);
        let Some(pr) = self.project(id) else { return };
        ui.set_max_width(320.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(egui::RichText::new(&pr.name).font(style::semibold(12.5)).color(p.text));
        ui.label(egui::RichText::new(crate::paths::contract_tilde(&pr.path)).size(style::SMALL).color(p.text_muted));
        if pr.missing {
            ui.label(egui::RichText::new(t("Folder not found")).size(style::SMALL).color(p.err));
        } else if let Some(g) = &pr.git {
            let mut line = format!("{} {}", icons::GIT_BRANCH, branch_text(g));
            if let Some(wt) = &pr.worktree
                && !wt.base.is_empty()
            {
                line = format!("{line} · {}", tf("from {}", &[&wt.base]));
            }
            if g.merged {
                line = format!("{line} · {}", t("Merged"));
            } else if g.ahead > 0 {
                line = format!("{line} · ↑{}", g.ahead);
            }
            ui.label(egui::RichText::new(line).size(style::SMALL).color(p.text));
        }
        for pane in pr.panes().iter().filter_map(|x| self.panes.get(x)) {
            let s = pane.tracker.status;
            let quiet = matches!(s, Status::Idle | Status::Unknown | Status::Ended);
            let (icon, c) = if quiet { (icons::TERMINAL, p.text_faint) } else { (status::icon(s), status::color(s, &p)) };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(egui::RichText::new(icon).font(style::icon_font(12.0)).color(c));
                ui.label(egui::RichText::new(format!("{} · {}", pane.label(), status::text(s))).size(style::SMALL).color(p.text));
            });
        }
        ui.label(egui::RichText::new(t("Right-click for more")).size(style::SMALL).color(p.text_faint));
    }

    pub(super) fn project_menu(&mut self, ui: &mut Ui, id: u64, name: &str) {
        kit::menu_body(ui);
        ui.set_min_width(250.0);
        if kit::menu_item(ui, Some(icons::PLAY), t("Run Agent…"), kit::keys(&Action::PickPreset), false).clicked() {
            self.actions.push(Action::SelectProject(id));
            self.actions.push(Action::PickPreset);
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::GIT_DIFF), t("Review Changes…"), kit::keys(&Action::Review), false).clicked() {
            self.actions.push(Action::SelectProject(id));
            self.actions.push(Action::Review);
            ui.close();
        }
        let git = self.project(id).is_some_and(|x| x.git.is_some());
        if git && kit::menu_item(ui, Some(icons::GIT_FORK), t("New Worktree…"), None, false).clicked() {
            if let Some(pr) = self.projects.iter_mut().find(|x| x.id == id) {
                pr.new_worktree = true;
            }
            self.actions.push(Action::SelectProject(id));
            self.actions.push(Action::PickPreset);
            ui.close();
        }
        kit::menu_sep(ui);
        if kit::menu_item(ui, Some(icons::PENCIL_SIMPLE), t("Rename…"), None, false).clicked() {
            self.dialog = Some(Dialog::Rename(id, name.to_string()));
            ui.close();
        }
        let reveal = if cfg!(target_os = "macos") {
            t("Show in Finder")
        } else {
            t("Show in Folder")
        };
        if kit::menu_item(ui, Some(icons::FOLDER_OPEN), reveal, None, false).clicked() {
            self.actions.push(Action::RevealProject(id));
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::COPY), t("Copy Path"), None, false).clicked() {
            if let Some(path) = self.project(id).map(|x| x.path.to_string_lossy().into_owned()) {
                ui.ctx().copy_text(path);
            }
            ui.close();
        }
        kit::menu_sep(ui);
        let in_active = self.is_sidebar_active(id);
        let group: Vec<tc_core::ProjectId> = self.top_order().into_iter().filter(|pid| self.is_sidebar_active(*pid) == in_active).collect();
        let at = group.iter().position(|x| *x == id);
        let first = at == Some(0);
        let last = at.is_some_and(|i| i + 1 == group.len());
        if ui.add_enabled_ui(!first, |ui| kit::menu_item(ui, Some(icons::ARROW_UP), t("Move Up"), kit::keys(&Action::MoveProject(id, true)), false)).inner.clicked() {
            self.actions.push(Action::MoveProject(id, true));
            ui.close();
        }
        if ui.add_enabled_ui(!last, |ui| kit::menu_item(ui, Some(icons::ARROW_DOWN), t("Move Down"), kit::keys(&Action::MoveProject(id, false)), false)).inner.clicked() {
            self.actions.push(Action::MoveProject(id, false));
            ui.close();
        }
        kit::menu_sep(ui);
        if kit::menu_item(ui, Some(icons::TRASH), t("Remove from Workspace"), None, true).clicked() {
            self.actions.push(Action::RemoveProject(id));
            ui.close();
        }
    }
}

/// What something waiting says or what happened: the agent's question (or its last words when
/// it finished), else its status.
/// The second line of a waiting row in the queue, short: what is asked ("Bash · flutter test"),
/// or what happened ("Finished", "go test ./... · exit 1").
pub fn attention_short(pane: &crate::pane::Pane) -> String {
    let s = pane.tracker.status;
    match s {
        Status::NeedsInput => pane.tracker.message.clone().map(|m| m.replacen(": ", " · ", 1)).unwrap_or_else(|| status::text(s)),
        Status::Done => match &pane.tracker.message {
            Some(m) => format!("{} · {m}", t("Finished")),
            None => t("Finished").to_string(),
        },
        Status::Failed(code) => format!("{} · {}", pane.label(), crate::i18n::tf("exit {}", &[&code.to_string()])),
        Status::CommandDone => format!("{} · {}", pane.label(), t("finished")),
        _ => status::text(s),
    }
}

/// The status bar's sentence about a waiting pane: who, then what, and the command it is about
/// (set in the data font): ("Claude Code", "wants to run", Some("flutter test")).
pub fn attention_sentence(pane: &crate::pane::Pane) -> (String, String, Option<String>) {
    let who = pane.label();
    let s = pane.tracker.status;
    match (s, pane.tracker.message.as_deref()) {
        (Status::NeedsInput, Some(m)) => match m.split_once(": ") {
            Some(("Bash", cmd)) => (who, t("wants to run").to_string(), Some(cmd.to_string())),
            Some((tool, what)) => (who, crate::i18n::tf("asks to use {}:", &[tool]), Some(what.to_string())),
            None => (who, m.to_string(), None),
        },
        (Status::NeedsInput, None) => (who, t("needs input").to_string(), None),
        (Status::Done, _) => (who, t("finished its turn").to_string(), None),
        (Status::Failed(code), _) => (who, crate::i18n::tf("exited with code {}", &[&code.to_string()]), None),
        (Status::CommandDone, _) => (who, t("finished").to_string(), None),
        _ => (who, status::text(s).to_lowercase(), None),
    }
}

pub fn attention_what(pane: &crate::pane::Pane) -> String {
    let s = pane.tracker.status;
    match s {
        Status::NeedsInput | Status::Done => pane.tracker.message.clone().unwrap_or_else(|| status::text(s)),
        _ => status::text(s),
    }
}

/// What a project's sidebar row shows.
pub struct ProjectRow {
    pub id: u64,
    pub name: String,
    pub color: u8,
    /// What happens there (none when every shell idles).
    pub status: Option<Status>,
    /// Panes with something not seen yet.
    pub waiting: usize,
    /// Panes whose agent asks the user something (the count the row shows).
    pub asks: usize,
    /// No shell started yet.
    pub asleep: bool,
    /// A dot per pane (when there are several), in its status colour.
    pub dots: Vec<egui::Color32>,
    /// Under the name: the branch and how many files changed, else where it lives.
    pub branch: Option<(String, usize)>,
    pub place: String,
    /// Under the name, at the right: what is at work or waits (its mark, text, colour).
    pub chip: Option<(RowMark, String, egui::Color32)>,
    /// How many worktrees are listed under it, and whether they are folded away.
    pub worktrees: usize,
    pub folded: bool,
    /// A worktree: the commits its branch has that its base has not.
    pub ahead: Option<usize>,
    /// It has a browser with its page open: whether an agent uses it (a globe after the dots,
    /// in the info colour then).
    pub web: Option<bool>,
}

/// What leads a row's status: the status's glyph, the agent at work's logo, or an icon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RowMark {
    Status(Status),
    Agent(tc_core::agent::AgentKind),
    Icon(&'static str),
}

impl RowMark {
    /// How wide [`RowMark::paint_pill`] draws `text`.
    pub fn pill_width(painter: &egui::Painter, text: &str) -> f32 {
        painter.layout_no_wrap(text.to_string(), style::medium(11.5), egui::Color32::PLACEHOLDER).size().x + 11.0 + 5.0 + 14.0
    }

    /// The mark and `text` as a pill on the colour's tint (the palette's rows); returns its
    /// rectangle.
    pub fn paint_pill(self, painter: &egui::Painter, left_center: egui::Pos2, text: &str, color: egui::Color32, p: &style::Palette, dark: bool) -> Rect {
        let w = Self::pill_width(painter, text);
        let rect = Rect::from_min_size(pos2(left_center.x, left_center.y - kit::PILL_H / 2.0), vec2(w, kit::PILL_H));
        let quiet = color == p.idle || color == p.text_faint;
        painter.rect_filled(rect, CornerRadius::same(10), if quiet { p.hover } else { style::tint(color, dark) });
        self.paint(painter, pos2(rect.left() + 7.0 + 5.5, rect.center().y), color, p);
        let g = painter.layout_no_wrap(text.to_string(), style::medium(11.5), color);
        painter.galley(pos2(rect.left() + 7.0 + 11.0 + 5.0, rect.center().y - g.size().y / 2.0), g, color);
        rect
    }

    /// Paints the mark centred at `center` (11 across).
    pub fn paint(self, painter: &egui::Painter, center: egui::Pos2, color: egui::Color32, p: &style::Palette) {
        match self {
            RowMark::Status(s) => status::paint_glyph(painter, center, s, p, 11.0),
            RowMark::Agent(a) => {
                super::agent_logo::paint(painter, Rect::from_center_size(center, vec2(11.0, 11.0)), a, p.text, false);
            }
            RowMark::Icon(i) => {
                painter.text(center, Align2::CENTER_CENTER, i, style::icon_font(11.0), color);
            }
        }
    }
}


/// The account in use, for the meter at the foot.
struct UsageMeter {
    agent: tc_core::agent::AgentKind,
    label: String,
    pct: Option<f32>,
    metric: String,
    window: String,
    reset: Option<String>,
    limited: bool,
}

/// An agent's short name in the meter ("Claude", "Codex").
fn short_agent(a: tc_core::agent::AgentKind) -> &'static str {
    let l = a.label();
    l.split_whitespace().next().unwrap_or(l)
}

/// "2m", "14m", "1h", "3d".
fn short_age(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

/// The statuses a row shows a glyph for (quiet ones show nothing).
fn shows_status(s: Status) -> bool {
    !matches!(s, Status::Idle | Status::Unknown | Status::Ended)
}

/// Something selected in the sidebar: the raised surface, its hairline and a soft shadow.
fn raised(painter: &egui::Painter, rect: Rect, radius: u8, p: &style::Palette) {
    let shadow = egui::epaint::Shadow { offset: [0, 1], blur: 2, spread: 0, color: egui::Color32::from_black_alpha(64) };
    painter.add(shadow.as_shape(rect, CornerRadius::same(radius)));
    painter.rect(rect, CornerRadius::same(radius), p.surface, Stroke::new(1.0, p.border), StrokeKind::Inside);
}

/// A place at the top (Inbox, Overview): its icon, its name, a count of what waits, its keys.
#[allow(clippy::too_many_arguments)]
fn nav_item(ui: &mut Ui, rect: Rect, icon: &str, label: &str, count: Option<usize>, keys: Option<&str>, on: bool, sc: SidebarScale, p: &style::Palette) -> egui::Response {
    let r = ui.interact(rect, egui::Id::new(("nav", label)), Sense::click());
    let painter = ui.painter();
    if on {
        raised(painter, rect, style::RADIUS_ROW, p);
    } else if r.hovered() {
        painter.rect_filled(rect, CornerRadius::same(style::RADIUS_ROW), p.hover);
    }
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let ink = if on || r.hovered() { p.text } else { p.text_muted };
    let cy = rect.center().y;
    painter.text(pos2(rect.left() + 8.0 + sc.icon / 2.0, cy), Align2::CENTER_CENTER, icon, style::icon_font(sc.icon), ink);
    let font = if on { style::medium(sc.body) } else { egui::FontId::proportional(sc.body) };
    painter.text(pos2(rect.left() + 8.0 + sc.icon + 10.0, cy), Align2::LEFT_CENTER, label, font, ink);
    let mut right = rect.right() - 8.0;
    if let Some(n) = count {
        right -= count_pill(painter, pos2(right, cy), n, p, ui.visuals().dark_mode);
    }
    if let Some(k) = keys
        && count.is_none()
    {
        painter.text(pos2(right, cy), Align2::RIGHT_CENTER, k, style::mono(sc.data), p.text_faint);
    }
    r
}

/// The amber count of what waits, ending at `right_center`; returns its width.
fn count_pill(painter: &egui::Painter, right_center: egui::Pos2, n: usize, p: &style::Palette, dark: bool) -> f32 {
    let g = painter.layout_no_wrap(n.to_string(), style::mono_semibold(style::DATA), p.warn);
    let w = (g.size().x + 10.0).max(18.0);
    let rect = Rect::from_min_size(pos2(right_center.x - w, right_center.y - 9.0), vec2(w, 18.0));
    painter.rect_filled(rect, CornerRadius::same(9), style::tint(p.warn, dark));
    painter.galley(rect.center() - g.size() / 2.0, g, p.warn);
    w
}

/// A button of the rail (36 × 32): raised when on, the hover fill under the pointer.
fn rail_button(ui: &mut Ui, rect: Rect, icon: &str, tip: &str, on: bool, filled: bool, p: &style::Palette) -> egui::Response {
    let r = ui.interact(rect, egui::Id::new(("rail", icon)), Sense::click()).on_hover_text(tip);
    let painter = ui.painter();
    if on {
        raised(painter, rect, style::RADIUS_ROW, p);
    } else if r.hovered() || filled {
        painter.rect_filled(rect, CornerRadius::same(style::RADIUS_ROW), if r.hovered() && filled { p.subtle } else { p.hover });
    }
    let ink = if on || r.hovered() { p.text } else { p.text_muted };
    painter.text(rect.center(), Align2::CENTER_CENTER, icon, style::icon_font(16.0), ink);
    r
}

/// A group's collapsible header line (`Active` / `Idle`): caret, name, count, and (for Active)
/// clean tinted status pills on the right (`● 1 running`, `● 1 waiting`, …).
#[allow(clippy::too_many_arguments)]
fn paint_group_head(
    ui: &Ui,
    rect: Rect,
    label: &str,
    count: usize,
    folded: bool,
    hovered: bool,
    is_active: bool,
    summary: &[(Status, usize)],
    dark: bool,
    sc: SidebarScale,
    p: &style::Palette,
) {
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, CornerRadius::same(6), p.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let cy = rect.center().y;
    let caret = if folded { icons::CARET_RIGHT } else { icons::CARET_DOWN };
    painter.text(pos2(rect.left() + 10.0, cy), Align2::CENTER_CENTER, caret, style::icon_font((sc.label - 1.5).max(9.0)), p.text_ghost);
    let label_c = if is_active { p.text_muted } else { p.text_faint };
    let g = painter.layout_no_wrap(label.to_string(), style::medium(sc.label), label_c);
    let gw = g.size().x;
    painter.galley(pos2(rect.left() + 20.0, cy - g.size().y / 2.0), g, label_c);
    let count_g = painter.layout_no_wrap(count.to_string(), style::mono(sc.data), p.text_ghost);
    let left_end = rect.left() + 20.0 + gw + 6.0 + count_g.size().x + 8.0;
    painter.galley(pos2(rect.left() + 20.0 + gw + 6.0, cy - count_g.size().y / 2.0), count_g, p.text_ghost);

    let mut x = rect.right() - 6.0;
    let pill_font = style::mono((sc.data - 0.5).max(9.5));
    for (s, n) in summary.iter().rev().filter(|s| s.1 > 0) {
        let color = status::color(*s, p);
        let text = match *s {
            Status::Running => tf("{} running", &[&n.to_string()]),
            Status::NeedsInput => tf("{} waiting", &[&n.to_string()]),
            Status::Done | Status::CommandDone => tf("{} finished", &[&n.to_string()]),
            Status::Failed(_) => format!("{n} {}", t("failed")),
            _ => n.to_string(),
        };
        let tg = painter.layout_no_wrap(text, pill_font.clone(), color);
        let pw = 7.0 + 5.5 + 5.0 + tg.size().x + 7.0;
        if x - pw < left_end {
            break;
        }
        let pill = Rect::from_min_size(pos2(x - pw, cy - 9.0), vec2(pw, 18.0));
        painter.rect_filled(pill, CornerRadius::same(9), style::tint(color, dark));
        let dot_c = pos2(pill.left() + 9.5, cy);
        if *s == Status::Running {
            painter.circle_filled(dot_c, 4.2, style::soft(color, 0.22));
        }
        painter.circle_filled(dot_c, 2.5, color);
        painter.galley(pos2(pill.left() + 17.0, cy - tg.size().y / 2.0), tg, color);
        x -= pw + 5.0;
    }
}

/// A project's row (Conductor × macOS): frameless sculpted metallic folder aligned on the nav-icon
/// grid, micro-dot status on the folder corner, hover/selected branch hint pill, subtle worktree
/// count badge, and live status (suppressed on expanded headers to avoid duplicate spinners).
#[allow(clippy::too_many_arguments)]
fn paint_project_row(
    ui: &Ui,
    rect: Rect,
    row: &ProjectRow,
    marks: &[bool],
    selected: bool,
    parent_of_selected: bool,
    expanded: bool,
    hovered: bool,
    is_active: bool,
    dim: bool,
    chosen: bool,
    dark: bool,
    sc: SidebarScale,
    p: &style::Palette,
) {
    let painter = ui.painter();
    if selected {
        raised(painter, rect, style::RADIUS_ROW, p);
    } else if hovered {
        painter.rect_filled(rect, CornerRadius::same(style::RADIUS_ROW), p.hover);
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let cy = rect.center().y;
    let icon_cx = rect.left() + 8.0 + sc.icon / 2.0;
    let tile = Rect::from_center_size(pos2(icon_cx, cy), vec2(sc.tile, sc.tile));
    let tone = if selected || parent_of_selected {
        kit::FolderTone::Selected
    } else if hovered {
        kit::FolderTone::Hover
    } else if is_active {
        kit::FolderTone::Active
    } else {
        kit::FolderTone::Idle
    };
    kit::paint_project_folder(painter, tile, tone, dark);

    // Subtle status micro-dot anchored on the bottom-right corner of the sculpted folder.
    if !dim
        && let Some(st) = row.status.filter(|s| {
            matches!(s, Status::Running | Status::NeedsInput | Status::Failed(_))
                || (matches!(s, Status::Done | Status::CommandDone) && row.waiting > 0)
        })
    {
        let dot_at = pos2(tile.right() - 1.2, tile.bottom() - 2.2);
        let halo = if selected { p.surface } else { p.chrome };
        painter.circle_filled(dot_at, 3.6, halo);
        painter.circle_filled(dot_at, 2.3, status::color(st, p));
    }

    let x = rect.left() + 8.0 + sc.icon + 10.0;
    let mut right = rect.right() - 8.0;
    if chosen {
        let kw = kit::kbd_width(painter, "↵");
        kit::paint_kbd_text(painter, pos2(right - kw, cy), "↵", p);
        right -= kw + 8.0;
    } else if !dim {
        // When collapsed, show the project's live status glyph on the right; when expanded,
        // the child sessions underneath already display individual status glyphs (preventing
        // duplicate spinners while keeping the subtle folder corner micro-dot).
        if !expanded {
            if let Some(s) = row.status.filter(|s| shows_status(*s)) {
                let gs = (sc.icon - 2.0).max(12.0);
                status::paint_glyph(painter, pos2(right - gs / 2.0, cy), s, p, gs);
                right -= gs + 8.0;
            } else if row.asleep {
                painter.circle_filled(pos2(right - 3.0, cy), 2.5, p.text_ghost);
                right -= 6.0 + 8.0;
            }
            if row.asks > 0 {
                right -= count_pill(painter, pos2(right, cy), row.asks, p, dark) + 8.0;
            }
        }
        // Conductor-style muted branch pill on hover or selection when room allows, else a quiet
        // `⑂ N` worktree badge when this project has worktrees under it.
        if (hovered || selected)
            && let Some((branch, _)) = &row.branch
            && !branch.is_empty()
            && right - x > 110.0
        {
            let max_bw = ((right - x) * 0.36).clamp(32.0, 74.0);
            let mut bjob = egui::text::LayoutJob::simple_singleline(branch.clone(), style::mono((sc.data - 0.5).max(9.5)), p.text_faint);
            bjob.wrap = egui::text::TextWrapping::truncate_at_width(max_bw);
            let bg = painter.layout_job(bjob);
            if bg.size().x > 8.0 {
                let pw = bg.size().x + 10.0;
                let pill = Rect::from_min_size(pos2(right - pw, cy - 8.5), vec2(pw, 17.0));
                painter.rect_filled(pill, CornerRadius::same(4), p.hover);
                painter.galley(pos2(pill.left() + 5.0, cy - bg.size().y / 2.0), bg, p.text_faint);
                right -= pw + 6.0;
            }
        } else if row.worktrees > 0 && right - x > 92.0 {
            let num_g = painter.layout_no_wrap(row.worktrees.to_string(), style::mono((sc.data - 0.5).max(9.5)), p.text_faint);
            let pw = 5.0 + 10.0 + 3.0 + num_g.size().x + 5.0;
            let pill = Rect::from_min_size(pos2(right - pw, cy - 8.0), vec2(pw, 16.0));
            painter.rect_filled(pill, CornerRadius::same(4), p.hover);
            painter.text(pos2(pill.left() + 5.0 + 5.0, cy), Align2::CENTER_CENTER, icons::GIT_FORK, style::icon_font(10.0), p.text_faint);
            painter.galley(pos2(pill.left() + 18.0, cy - num_g.size().y / 2.0), num_g, p.text_faint);
            right -= pw + 6.0;
        }
    }
    let ink = if dim { p.text_faint } else if selected || parent_of_selected || hovered { p.text } else { p.text_muted };
    let font = if selected { style::medium(sc.body) } else { egui::FontId::proportional(sc.body) };
    let mut job = name_job(&row.name, marks, font, ink, p);
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(16.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, ink);
}

/// Conductor-style rounded L-elbow tree guide (`└` for the last child, `├` for earlier children).
fn paint_tree_elbow(painter: &egui::Painter, rect: Rect, rail_x: f32, end_x: f32, is_last: bool, color: egui::Color32) {
    let stroke = Stroke::new(1.0, color);
    let cy = rect.center().y;
    let r = 5.0_f32.min((end_x - rail_x).max(2.0));
    let elbow = vec![
        pos2(rail_x, rect.top() - 1.0),
        pos2(rail_x, cy - r),
        pos2(rail_x + r * 0.25, cy - r * 0.45),
        pos2(rail_x + r * 0.55, cy - r * 0.15),
        pos2(rail_x + r, cy),
        pos2(end_x, cy),
    ];
    painter.add(egui::Shape::line(elbow, stroke));
    if !is_last {
        painter.vline(rail_x, (cy - r)..=(rect.bottom() + 1.0), stroke);
    }
}

/// Returns true when a worktree's git branch name is redundant with its sidebar display name
/// (e.g. `tc/update-readme-2` vs `update-readme-2`), so the sidebar can give full width to the name.
fn branch_is_redundant(branch: &str, name: &str, prefix: &str) -> bool {
    if branch.is_empty() || branch == name {
        return true;
    }
    if !prefix.is_empty() && branch.strip_prefix(prefix) == Some(name) {
        return true;
    }
    branch.rsplit_once('/').is_some_and(|(_, tail)| tail == name)
}

/// Display data for a worktree row in the sidebar.
struct WorktreeRowInfo<'a> {
    name: &'a str,
    branch: &'a str,
    branch_prefix: &'a str,
    marks: &'a [bool],
    status: Option<Status>,
    asks: usize,
    changed: usize,
    ahead: usize,
    merged: bool,
    missing: bool,
    selected: bool,
    hovered: bool,
    caret_hovered: bool,
    is_active: bool,
    is_last: bool,
    session_count: usize,
    children_expanded: bool,
}

/// A worktree under its project: monochrome curved tree guide, `⑂` fork icon, full worktree name
/// (suppressing redundant `tc/<name>` branch text), compact metadata pill (`missing` / `merged` /
/// `N chg` / `↑N` / distinct branch), folded status, and expand/collapse caret.
fn paint_worktree_row(ui: &Ui, rect: Rect, w: &WorktreeRowInfo<'_>, dark: bool, sc: SidebarScale, p: &style::Palette) {
    let painter = ui.painter();
    let rail_x = rect.left() + 8.0 + sc.icon / 2.0;
    let body = Rect::from_min_max(pos2(rail_x + 9.0, rect.top()), rect.max);
    if w.selected {
        raised(painter, body, 6, p);
    } else if w.hovered {
        painter.rect_filled(body, CornerRadius::same(6), p.hover);
    }
    if w.hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let cy = rect.center().y;
    paint_tree_elbow(painter, rect, rail_x, body.left() - 1.5, w.is_last, p.border_strong);
    let ic = (sc.icon - 2.0).max(12.0);
    let mut x = body.left() + 6.5;
    let icon_c = if w.missing {
        p.err
    } else if w.merged {
        p.ok
    } else if w.selected || w.is_active {
        p.accent_text
    } else {
        p.text_faint
    };
    painter.text(pos2(x + ic / 2.0, cy), Align2::CENTER_CENTER, icons::GIT_FORK, style::icon_font(ic), icon_c);
    x += ic + 6.0;

    let mut right = rect.right() - 6.0;
    // Fold caret (far right) when this worktree has expandable sessions.
    if w.session_count > 0 {
        let caret = if w.children_expanded { icons::CARET_DOWN } else { icons::CARET_RIGHT };
        let caret_c = if w.caret_hovered {
            p.text
        } else if w.hovered {
            p.text_muted
        } else {
            p.text_ghost
        };
        painter.text(pos2(right - 5.0, cy), Align2::CENTER_CENTER, caret, style::icon_font((sc.label - 1.5).max(9.0)), caret_c);
        right -= 14.0;
    }

    // When collapsed (folded or idle), show status glyph, asks count, and folded session count.
    if !w.children_expanded {
        if let Some(s) = w.status {
            status::paint_glyph(painter, pos2(right - 6.0, cy), s, p, 12.0);
            right -= 12.0 + 6.0;
        }
        if w.asks > 0 {
            right -= count_pill(painter, pos2(right, cy), w.asks, p, dark) + 6.0;
        }
        if w.session_count > 0 && right - x > 74.0 {
            let cg = painter.layout_no_wrap(w.session_count.to_string(), style::mono((sc.data - 0.5).max(9.5)), p.text_faint);
            let cw = (cg.size().x + 8.0).max(16.0);
            let pill = Rect::from_min_size(pos2(right - cw, cy - 8.0), vec2(cw, 16.0));
            painter.rect_filled(pill, CornerRadius::same(4), p.hover);
            painter.galley(pill.center() - cg.size() / 2.0, cg, p.text_faint);
            right -= cw + 5.0;
        }
    }

    // Highest-priority compact worktree metadata pill (only if enough room stays for the name).
    let meta_pill: Option<(String, egui::Color32, egui::Color32, f32)> = if w.missing {
        Some((t("missing").to_string(), style::tint(p.err, dark), p.err, 64.0))
    } else if w.merged {
        Some((t("merged").to_string(), style::tint(p.ok, dark), p.ok, 64.0))
    } else if w.changed > 0 {
        Some((tf("{} chg", &[&w.changed.to_string()]), style::tint(p.accent, dark), p.accent_text, 64.0))
    } else if w.ahead > 0 {
        Some((format!("↑{}", w.ahead), p.hover, p.text_faint, 48.0))
    } else if !branch_is_redundant(w.branch, w.name, w.branch_prefix) {
        Some((w.branch.to_string(), p.hover, p.text_faint, 60.0))
    } else {
        None
    };
    if let Some((text, bg_c, fg_c, max_w)) = meta_pill {
        let mut pjob = egui::text::LayoutJob::simple_singleline(text, style::mono((sc.data - 0.5).max(9.5)), fg_c);
        pjob.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
        let pg = painter.layout_job(pjob);
        let pw = pg.size().x + 8.0;
        if right - pw - 6.0 - x >= 56.0 {
            let pill = Rect::from_min_size(pos2(right - pw, cy - 8.0), vec2(pw, 16.0));
            painter.rect_filled(pill, CornerRadius::same(4), bg_c);
            painter.galley(pos2(pill.left() + 4.0, cy - pg.size().y / 2.0), pg, fg_c);
            right -= pw + 6.0;
        }
    }

    let ink = if w.missing {
        p.text_faint
    } else if w.selected || w.hovered || w.is_active {
        p.text
    } else {
        p.text_muted
    };
    let font = if w.selected { style::medium(sc.small) } else { egui::FontId::proportional(sc.small) };
    let mut job = name_job(w.name, w.marks, font, ink, p);
    if w.missing {
        for sec in &mut job.sections {
            sec.format.strikethrough = Stroke::new(1.0, p.text_faint);
        }
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(16.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, ink);
}

/// A session (pane) under an expanded root project (level 1) or nested under a worktree (level 2):
/// monochrome curved tree guide, agent avatar, title, elapsed/port meta, and live status.
#[allow(clippy::too_many_arguments)]
fn paint_session_row(
    ui: &Ui,
    rect: Rect,
    s: &SessionRow,
    hovered: bool,
    is_last: bool,
    in_worktree: bool,
    outer_continues: bool,
    dark: bool,
    sc: SidebarScale,
    p: &style::Palette,
) {
    let painter = ui.painter();
    let rail_x = rect.left() + 8.0 + sc.icon / 2.0;
    let (body, av, pad_x, gap, title_sz, glyph_sz) = if in_worktree {
        if outer_continues {
            painter.vline(rail_x, (rect.top() - 1.0)..=(rect.bottom() + 1.0), Stroke::new(1.0, p.border_strong));
        }
        let wt_ic = (sc.icon - 2.0).max(12.0);
        let rail2_x = rail_x + 9.0 + 6.5 + wt_ic / 2.0;
        let body = Rect::from_min_max(pos2(rail2_x + 8.5, rect.top()), rect.max);
        paint_tree_elbow(painter, rect, rail2_x, body.left() - 1.5, is_last, p.border_strong);
        (body, (sc.icon - 2.0).max(13.0), 6.0, 6.0, (sc.small - 0.5).max(11.0), 11.5)
    } else {
        let body = Rect::from_min_max(pos2(rail_x + 9.0, rect.top()), rect.max);
        paint_tree_elbow(painter, rect, rail_x, body.left() - 1.5, is_last, p.border_strong);
        (body, sc.icon, 8.0, 7.0, sc.small, 12.0)
    };
    if s.on {
        painter.rect_filled(body, CornerRadius::same(6), p.accent_soft);
    } else if hovered {
        painter.rect_filled(body, CornerRadius::same(6), p.hover);
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let cy = rect.center().y;
    let mut x = body.left() + pad_x;
    kit::paint_agent_avatar(painter, Rect::from_center_size(pos2(x + av / 2.0, cy), vec2(av, av)), s.agent, p);
    x += av + gap;
    let mut right = rect.right() - 7.0;
    if let Some(st) = s.status {
        status::paint_glyph(painter, pos2(right - glyph_sz / 2.0, cy), st, p, glyph_sz);
        right -= glyph_sz + 6.0;
    }
    if !s.meta.is_empty() {
        if s.meta.starts_with(':') {
            let g = painter.layout_no_wrap(s.meta.clone(), style::mono((sc.data - 0.5).max(9.5)), p.ok);
            let pw = g.size().x + 6.0;
            if right - pw - 5.0 - x >= 36.0 {
                let pill = Rect::from_min_size(pos2(right - pw, cy - 7.5), vec2(pw, 15.0));
                painter.rect_filled(pill, CornerRadius::same(4), style::tint(p.ok, dark));
                painter.galley(pos2(pill.left() + 3.0, cy - g.size().y / 2.0), g, p.ok);
                right -= pw + 5.0;
            }
        } else {
            let g = painter.layout_no_wrap(s.meta.clone(), style::mono(sc.data), p.text_faint);
            if right - g.size().x - 6.0 - x >= 36.0 {
                right -= g.size().x;
                painter.galley(pos2(right, cy - g.size().y / 2.0), g, p.text_faint);
                right -= 6.0;
            }
        }
    }
    let ink = if s.on || hovered { p.text } else { p.text_muted };
    let font = if s.on { style::medium(title_sz) } else { egui::FontId::proportional(title_sz) };
    let mut job = egui::text::LayoutJob::simple_singleline(s.title.clone(), font, ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(16.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, ink);
}

/// A name with the letters the search found set in the accent (medium).
fn name_job(name: &str, marks: &[bool], font: egui::FontId, color: egui::Color32, p: &style::Palette) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    if marks.is_empty() {
        job.append(name, 0.0, egui::TextFormat::simple(font, color));
        return job;
    }
    let found = style::medium(font.size);
    for (k, ch) in name.chars().enumerate() {
        let on = marks.get(k).copied().unwrap_or(false);
        let f = if on { egui::TextFormat::simple(found.clone(), p.accent_text) } else { egui::TextFormat::simple(font.clone(), color) };
        job.append(&ch.to_string(), 0.0, f);
    }
    job
}

/// Where a project lives, short: its parent folder (`~/Developments`), or its last two names
/// (`…/RUST/tools`).
fn folder_line(path: &std::path::Path) -> String {
    let parent = path.parent().unwrap_or(path);
    let short = crate::paths::contract_tilde(parent);
    let parts: Vec<&str> = short.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 2 {
        short
    } else {
        format!("…/{}", parts[parts.len() - 2..].join("/"))
    }
}

impl App {
    pub fn project_row(&self, pr: &crate::project::Project, p: &style::Palette) -> ProjectRow {
        let panes = pr.panes();
        let waiting = panes
            .iter()
            .filter_map(|id| self.panes.get(id))
            .filter(|pane| pane.tracker.wants_attention())
            .count();
        let asks = panes.iter().filter_map(|id| self.panes.get(id)).filter(|pane| pane.tracker.status == Status::NeedsInput).count();
        let asleep = panes.iter().all(|id| self.panes.get(id).is_none_or(crate::pane::Pane::dormant));
        let status = self.project_status(pr).map(|(s, _)| s);
        let dots = if panes.len() > 1 {
            panes.iter().take(4).filter_map(|id| self.panes.get(id)).map(|x| status::dot_color(x.tracker.status, p)).collect()
        } else {
            Vec::new()
        };
        // The status: what waits; else the agent or program at work (with its progress); else
        // an agent that can be resumed.
        let pane = self.project_pane(pr.id).and_then(|id| self.panes.get(&id));
        let busy = pane.filter(|x| x.tracker.agent.is_some() || matches!(x.tracker.fg, tc_core::status::Foreground::Program { .. }));
        let chip = match (pane, status) {
            // Something waits (seen or not) or failed: the status says it.
            (Some(x), Some(s)) if x.tracker.wants_attention() || matches!(x.tracker.status, Status::NeedsInput | Status::Failed(_)) => {
                let s = if x.tracker.wants_attention() { s } else { x.tracker.status };
                let text = match s {
                    Status::Failed(_) => t("Failed").to_string(),
                    Status::CommandDone => t("Finished").to_string(),
                    _ => status::text(s),
                };
                Some((RowMark::Status(s), text, status::color(s, p)))
            }
            _ => busy.map(|x| {
                let s = x.tracker.status;
                let c = match s {
                    Status::Unknown | Status::Idle | Status::Ended => p.idle,
                    _ => status::color(s, p),
                };
                let text = match x.progress {
                    Some((_, Some(v))) => format!("{} · {v}%", x.label()),
                    _ => x.label(),
                };
                let mark = match x.tracker.agent {
                    Some(a) if super::agent_logo::has(a) => RowMark::Agent(a),
                    Some(_) if s == Status::Unknown => RowMark::Status(Status::Unknown),
                    _ if s == Status::Running => RowMark::Icon(icons::TERMINAL_WINDOW),
                    _ => RowMark::Status(s),
                };
                (mark, text, c)
            }),
        };
        let chip = chip.or_else(|| {
            panes
                .iter()
                .filter_map(|id| self.panes.get(id))
                .any(|x| x.resume.is_some())
                .then(|| (RowMark::Icon(icons::CLOCK_COUNTER_CLOCKWISE), t("Can resume").to_string(), p.idle))
        });
        // A worktree: gone, merged (when nothing else is to say), commits ahead.
        let chip = match (pr.is_worktree(), chip) {
            (true, _) if pr.missing => Some((RowMark::Status(Status::Failed(1)), t("Folder not found").to_string(), p.err)),
            (true, None) if pr.git.as_ref().is_some_and(|g| g.merged) => Some((RowMark::Status(Status::Done), t("Merged").to_string(), p.ok)),
            (_, chip) => chip,
        };
        let worktrees: Vec<&crate::project::Project> = self.projects.iter().filter(|x| x.parent == Some(pr.id)).collect();
        let mut row = ProjectRow {
            id: pr.id,
            name: pr.name.clone(),
            color: pr.color,
            status,
            waiting,
            asks,
            asleep,
            dots,
            branch: pr.git.as_ref().map(|g| (g.branch.clone(), g.changed)),
            place: if asleep { t("Not started").to_string() } else { folder_line(&pr.path) },
            chip,
            worktrees: worktrees.len(),
            folded: pr.folded,
            ahead: pr.is_worktree().then(|| pr.git.as_ref().map_or(0, |g| g.ahead)),
            web: self.browsers.values().filter(|b| b.project == pr.id && b.live()).map(|b| b.driven()).max(),
        };
        // When a project's worktrees are not currently expanded under it (folded or unselected Idle),
        // roll up their waiting count and most urgent status onto the parent project row.
        let active_root = self.active.and_then(|id| self.root_project(id)).map(|x| x.id);
        let expanded = active_root == Some(pr.id) || self.is_sidebar_active(pr.id);
        if !worktrees.is_empty() && !expanded {
            for w in worktrees {
                let kid = self.project_row(w, p);
                row.waiting += kid.waiting;
                row.asks += kid.asks;
                if !kid.asleep {
                    row.asleep = false;
                }
                let rank = |s: Option<Status>| s.and_then(Status::priority).unwrap_or(u8::MAX);
                if rank(kid.status) < rank(row.status) {
                    row.status = kid.status;
                    row.chip = kid.chip.map(|(mark, text, c)| (mark, format!("{} · {text}", kid.name), c));
                }
            }
        }
        row
    }
}

/// "main · 3 changed".
pub fn branch_text(g: &crate::project::GitInfo) -> String {
    if g.changed > 0 {
        crate::i18n::tf("{} · {} changed", &[&g.branch, &g.changed.to_string()])
    } else {
        g.branch.clone()
    }
}

/// A small upper-case section title at the left of `rect`.
pub fn section_title(ui: &Ui, rect: Rect, text: &str) {
    let p = style::pal(ui);
    let g = kit::section_galley(ui.painter(), text, p.text_faint);
    ui.painter().galley(pos2(rect.left() + 14.0, rect.center().y - g.size().y / 2.0), g, p.text_faint);
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_project_line_is_short() {
        let home = crate::paths::home_dir();
        assert_eq!(super::folder_line(&home.join("Developments/webapp")), "~/Developments");
        assert_eq!(super::folder_line(&home.join("Developments/RUST/tools/x")), "…/RUST/tools");
    }

    #[test]
    fn worktree_branch_deduplication_matches_prefixes_and_tails() {
        assert!(super::branch_is_redundant("", "update-readme-2", "tc/"));
        assert!(super::branch_is_redundant("update-readme-2", "update-readme-2", "tc/"));
        assert!(super::branch_is_redundant("tc/update-readme-2", "update-readme-2", "tc/"));
        assert!(super::branch_is_redundant("feature/update-readme-2", "update-readme-2", "tc/"));
        assert!(!super::branch_is_redundant("rel/v2", "hotfix-v2", "tc/"));
        assert!(!super::branch_is_redundant("main", "feat", "tc/"));
    }
}
