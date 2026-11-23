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
const SESSION_H: f32 = 28.0;
const FOOT_H: f32 = 30.0;
const USAGE_H: f32 = 68.0;

/// A line of the project list.
enum Line {
    /// A project (index), the letters the search found, dimmed (shown for a worktree found
    /// under it).
    Project(usize, Vec<bool>, bool),
    /// A tab of the selected project (its index in the project's tabs).
    Session(tc_core::ProjectId, usize),
    /// A worktree under its project, the letters found.
    Worktree(usize, Vec<bool>),
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
        let search = Rect::from_min_size(pos2(inner_l, y), vec2(inner_r - inner_l, SEARCH_H));
        let typing = self.sidebar_search(ui, search);
        y = search.bottom() + GAP;
        let searching = typing || !self.project_filter.trim().is_empty();

        // ----- Inbox, Overview ---------------------------------------------------------------
        if !searching {
            let waiting = self.attention_queue().len();
            let inbox = Rect::from_min_size(pos2(inner_l, y), vec2(inner_r - inner_l, NAV_H));
            let r = nav_item(ui, inbox, icons::TRAY, t("Inbox"), (waiting > 0).then_some(waiting), None, self.view == View::Inbox, &p);
            #[cfg(debug_assertions)]
            crate::devtest::record("nav:inbox", inbox);
            if r.on_hover_text(kit::tip(t("Everything that waits for you"), Action::ToggleAttention)).clicked() {
                self.actions.push(Action::ToggleAttention);
            }
            let overview = inbox.translate(vec2(0.0, NAV_H + 2.0));
            let keys = kit::keys(&Action::ToggleGrid);
            let r = nav_item(ui, overview, icons::SQUARES_FOUR, t("Overview"), None, keys, self.view == View::Grid && !self.attention_open, &p);
            #[cfg(debug_assertions)]
            crate::devtest::record("nav:overview", overview);
            if r.on_hover_text(t("Every project at once")).clicked() {
                self.actions.push(Action::SetView(if self.view == View::Grid { View::Focus } else { View::Grid }));
            }
            y = overview.bottom() + GAP;
        }

        // ----- foot: the account in use, Settings (while searching: the keys) -------------------
        let bottom = full.bottom() - 10.0;
        let list_bottom = if searching {
            let hint = Rect::from_min_max(pos2(inner_l, bottom - 18.0), pos2(inner_r, bottom));
            ui.painter().text(pos2(hint.left() + 2.0, hint.center().y), Align2::LEFT_CENTER, t("↑↓ choose · ↵ open · esc clear"), egui::FontId::proportional(style::SMALL), p.text_faint);
            hint.top() - GAP
        } else {
            let foot = Rect::from_min_max(pos2(inner_l, bottom - FOOT_H), pos2(inner_r, bottom));
            self.sidebar_footer(ui, foot);
            let mut top = foot.top() - GAP;
            if let Some(meter) = self.usage_meter_data() {
                let rect = Rect::from_min_max(pos2(inner_l, top - USAGE_H), pos2(inner_r, top));
                if rect.top() > y + SECTION_H + 3.0 * PROJECT_H {
                    self.usage_meter(ui, rect, &meter);
                    top = rect.top() - GAP;
                }
            }
            top
        };

        // ----- projects ---------------------------------------------------------------------
        let list = Rect::from_min_max(pos2(full.left(), y.min(list_bottom)), pos2(full.right(), list_bottom.max(y)));
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            ui.set_clip_rect(list.intersect(ui.clip_rect()));
            self.project_list(ui, searching)
        });
    }

    /// The search field at the top: a magnifier, what is typed (or "Search…" and ⌘K), × to
    /// empty it. While it has the keyboard ↑ ↓ choose a project, ↵ opens it, Esc empties it and
    /// lets go. Returns whether it has the keyboard.
    fn sidebar_search(&mut self, ui: &mut Ui, rect: Rect) -> bool {
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
        painter.text(pos2(rect.left() + 10.0 + 7.0, rect.center().y), Align2::CENTER_CENTER, icons::MAGNIFYING_GLASS, style::icon_font(14.0), if typing { p.text_muted } else { p.text_faint });
        let text_rect = Rect::from_min_max(pos2(rect.left() + 10.0 + 14.0 + 8.0, rect.top()), pos2(rect.right() - 30.0, rect.bottom()));
        let mut text = self.project_filter.clone();
        let edit = ui
            .scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .id(fid)
                        .frame(egui::Frame::NONE)
                        .font(egui::FontId::proportional(style::BODY))
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
            ui.painter().text(pos2(text_rect.left(), rect.center().y), Align2::LEFT_CENTER, t("Search…"), egui::FontId::proportional(style::BODY), p.text_faint);
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
    fn sidebar_footer(&mut self, ui: &mut Ui, rect: Rect) {
        let p = style::pal(ui);
        let label = t("Settings");
        let g = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(style::BODY), p.text_muted);
        let b = Rect::from_min_size(rect.min, vec2(8.0 + 16.0 + 10.0 + g.size().x + 8.0, FOOT_H));
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
        painter.text(pos2(b.left() + 8.0 + 8.0, b.center().y), Align2::CENTER_CENTER, icons::GEAR_SIX, style::icon_font(16.0), ink);
        painter.galley(pos2(b.left() + 8.0 + 16.0 + 10.0, b.center().y - g.size().y / 2.0), g, ink);
        if let Some(k) = kit::keys(&Action::Settings) {
            painter.text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, k, style::mono(style::DATA), p.text_ghost);
        }
        if r.clicked() {
            self.actions.push(Action::Settings);
        }
    }

    /// The account the meter at the foot shows: the one the agent in view uses, else the one
    /// most used in its short window (only accounts whose usage is known).
    fn usage_meter_data(&self) -> Option<UsageMeter> {
        let profiles = self.effective_profiles();
        let pick_id = self.current_pane().and_then(|id| self.panes.get(&id)).and_then(|x| x.active_profile.clone());
        let candidates = profiles.iter().filter(|pr| pr.enabled).filter_map(|pr| {
            let rt = self.profiles_rt.get(&pr.id)?;
            let pct = rt.quota_5h_used_pct(pr).or_else(|| rt.quota_weekly_used_pct(pr))?;
            Some((pr, rt, pct))
        });
        let mut best: Option<(&tc_core::model::AgentProfile, &crate::profiles::ProfileRuntime, f32)> = None;
        for c in candidates {
            if pick_id.as_deref() == Some(c.0.id.as_str()) {
                best = Some(c);
                break;
            }
            if best.as_ref().is_none_or(|b| c.2 > b.2) {
                best = Some(c);
            }
        }
        let (pr, rt, pct) = best?;
        let now = std::time::Instant::now();
        let now_unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let short = rt.quota_5h_used_pct(pr).is_some();
        let (window, reset) = if short {
            (rt.window_5h_label(), rt.window_5h_reset_secs(now_unix, now))
        } else {
            (rt.window_weekly_label(), rt.window_weekly_reset_secs(now_unix))
        };
        Some(UsageMeter {
            agent: pr.agent,
            label: format!("{} · {}", short_agent(pr.agent), pr.id),
            pct,
            window,
            reset: reset.map(|s| tf("resets {}", &[&crate::profiles::format_countdown(s)])),
            limited: matches!(rt.state, crate::profiles::ProfileState::RateLimited | crate::profiles::ProfileState::Cooldown),
        })
    }

    /// The meter: the agent's mark, "Claude · claude1", how much is used, a bar, the window and
    /// when it resets. A click opens Accounts & usage.
    fn usage_meter(&mut self, ui: &mut Ui, rect: Rect, m: &UsageMeter) {
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
        let row1 = inner.top() + 8.0;
        kit::paint_agent_avatar(painter, Rect::from_center_size(pos2(inner.left() + 8.0, row1), vec2(16.0, 16.0)), Some(m.agent), &p);
        let pct = format!("{}%", m.pct.round() as u32);
        let pg = painter.layout_no_wrap(pct, style::mono(style::DATA), p.text_muted);
        let pw = pg.size().x;
        painter.galley(pos2(inner.right() - pw, row1 - pg.size().y / 2.0), pg, p.text_muted);
        let mut job = egui::text::LayoutJob::simple_singleline(m.label.clone(), style::medium(style::SMALL), p.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((inner.width() - 24.0 - pw - 8.0).max(20.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(inner.left() + 24.0, row1 - g.size().y / 2.0), g, p.text);
        let track = Rect::from_min_size(pos2(inner.left(), row1 + 8.0 + 7.0), vec2(inner.width(), 4.0));
        painter.rect_filled(track, CornerRadius::same(2), p.subtle);
        let fill_c = if m.limited || m.pct >= 90.0 { p.rate_limited } else { p.accent };
        let fw = (track.width() * (m.pct / 100.0).clamp(0.0, 1.0)).max(if m.pct > 0.0 { 4.0 } else { 0.0 });
        painter.rect_filled(Rect::from_min_size(track.min, vec2(fw, 4.0)), CornerRadius::same(2), fill_c);
        let row3 = track.bottom() + 7.0 + 7.0;
        painter.text(pos2(inner.left(), row3), Align2::LEFT_CENTER, tf("{} window", &[&m.window]), egui::FontId::proportional(style::LABEL), p.text_faint);
        if let Some(reset) = &m.reset {
            painter.text(pos2(inner.right(), row3), Align2::RIGHT_CENTER, reset, egui::FontId::proportional(style::LABEL), p.text_faint);
        }
        if r.on_hover_text(t("Accounts & usage")).clicked() {
            self.monitor_popover = !self.monitor_popover;
        }
        self.monitor_pill_rect = Some(rect);
    }

    /// The projects: "Projects", how many, Search and Add; then each project in its place, the
    /// selected one with its sessions and worktrees under it. While searching: what was found,
    /// the keyboard's row with ↵.
    fn project_list(&mut self, ui: &mut Ui, searching: bool) {
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

        // The lines.
        let query = self.project_filter.trim().to_string();
        let filtering = !query.is_empty();
        let mut lines: Vec<Line> = Vec::new();
        if filtering {
            for (i, dim) in self.project_matches() {
                let pr = &self.projects[i];
                let marks = if dim { Vec::new() } else { super::palette::matched(&query, &pr.name) };
                if pr.is_worktree() {
                    lines.push(Line::Worktree(i, marks));
                } else {
                    lines.push(Line::Project(i, marks, dim));
                }
            }
        } else {
            let active_root = self.active.and_then(|id| self.root_project(id)).map(|x| x.id);
            for (i, pr) in self.projects.iter().enumerate().filter(|(_, x)| !x.is_worktree() || x.parent.and_then(|id| self.project(id)).is_none()) {
                lines.push(Line::Project(i, Vec::new(), false));
                if active_root != Some(pr.id) {
                    continue;
                }
                // The selected project: its tabs, then its worktrees.
                for k in 0..pr.tabs.len() {
                    lines.push(Line::Session(pr.id, k));
                }
                for (j, _) in self.projects.iter().enumerate().filter(|(_, x)| x.parent == Some(pr.id)) {
                    lines.push(Line::Worktree(j, Vec::new()));
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
        ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink2(vec2(PAD, 0.0))), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(if filtering { "project-list-found" } else { "project-list" })
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let mut found = 0usize;
                    for line in &lines {
                        match line {
                            Line::Project(i, marks, dim) => {
                                let pr = &self.projects[*i];
                                let id = pr.id;
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), PROJECT_H), Sense::click());
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
                                let selected = !filtering && self.active == Some(id) && self.view == View::Focus && !self.attention_open;
                                let row = self.project_row(pr, &p);
                                paint_project_row(ui, rect, &row, &pr.letters(), pr.color32(), marks, selected, resp.hovered() || chosen, *dim, chosen, dark, &p);
                                if resp.clicked() {
                                    pick = Some(Action::SelectProject(id));
                                }
                                let name = row.name.clone();
                                let index = top_indices.get(&id).copied().unwrap_or(usize::MAX);
                                let resp = resp.on_hover_ui(|ui| self.project_tooltip(ui, id, index));
                                resp.context_menu(|ui| self.project_menu(ui, id, &name));
                            }
                            Line::Worktree(j, marks) => {
                                let pr = &self.projects[*j];
                                let id = pr.id;
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), SESSION_H), Sense::click());
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
                                let selected = !filtering && self.active == Some(id) && self.view == View::Focus && !self.attention_open;
                                let st = self.project_status(pr).map(|(s, _)| s).filter(|s| shows_status(*s));
                                let branch = pr.git.as_ref().map(|g| g.branch.clone()).unwrap_or_default();
                                paint_worktree_row(ui, rect, &pr.name, &branch, marks, st, selected, resp.hovered() || chosen, &p);
                                if resp.clicked() {
                                    pick = Some(Action::SelectProject(id));
                                }
                                resp.context_menu(|ui| self.worktree_menu(ui, id));
                            }
                            Line::Session(pid, k) => {
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), SESSION_H), Sense::click());
                                if !ui.is_rect_visible(rect) {
                                    continue;
                                }
                                #[cfg(debug_assertions)]
                                crate::devtest::record(format!("session:{pid}:{k}"), rect.intersect(ui.clip_rect()));
                                let Some(s) = self.session_row(*pid, *k) else { continue };
                                paint_session_row(ui, rect, &s, resp.hovered(), &p);
                                if resp.on_hover_text(&s.title).clicked() {
                                    pick = Some(Action::SelectTab(*k));
                                    if self.active != Some(*pid) {
                                        self.actions.push(Action::SelectProject(*pid));
                                    }
                                    if self.view != View::Focus {
                                        self.actions.push(Action::SetView(View::Focus));
                                    }
                                }
                            }
                        }
                    }
                });
        });
        self.project_filter_rows = order;
        if let Some(a) = pick {
            if self.attention_open {
                self.attention_open = false;
            }
            self.actions.push(a);
        }
    }

    /// A tab of a project as a session row: who works there, what it is called, how long it
    /// has been at it (or the port its server listens on), its status.
    fn session_row(&self, project: tc_core::ProjectId, k: usize) -> Option<SessionRow> {
        let pr = self.project(project)?;
        let tab = pr.tabs.get(k)?;
        let pane = self.panes.get(&tab.focused);
        let title = tab.title.clone().unwrap_or_else(|| self.leaf_label(tab.focused));
        let urgent = tab
            .layout
            .panes()
            .iter()
            .filter_map(|id| self.panes.get(id))
            .filter_map(|x| Some(x.tracker.status).filter(|s| shows_status(*s)))
            .min_by_key(|s| s.priority());
        let server = self.project_servers(project).into_iter().find(|(id, _)| tab.layout.panes().contains(id)).map(|(_, s)| s.url);
        let meta = match (&server, pane) {
            (Some(url), _) => super::focus::server_label(url).rsplit_once(':').map_or_else(|| super::focus::server_label(url).to_string(), |(_, port)| format!(":{port}")),
            (None, Some(x)) if x.tracker.agent.is_some() && !matches!(x.tracker.status, Status::Idle | Status::Unknown | Status::Ended) => short_age(x.tracker.since.elapsed().as_secs()),
            _ => String::new(),
        };
        Some(SessionRow {
            title,
            agent: pane.and_then(|x| x.tracker.agent.or(x.last_agent)),
            meta,
            status: urgent,
            on: self.view == View::Focus && pr.active_tab == k && self.active == Some(project) && !self.attention_open,
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
        let waiting = self.attention_queue().len();
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
        ui.painter().hline((cx - 14.0)..=(cx + 14.0), y + 0.5, Stroke::new(1.0, p.border));
        y += 1.0 + 8.0;
        let gear = Rect::from_min_size(pos2(cx - 18.0, full.bottom() - 10.0 - 32.0), vec2(36.0, 32.0));
        let r = rail_button(ui, gear, icons::GEAR_SIX, &kit::tip(t("Settings"), Action::Settings), false, false, &p);
        #[cfg(debug_assertions)]
        crate::devtest::record("sidebar-settings", gear);
        if r.clicked() {
            self.actions.push(Action::Settings);
        }
        // The projects (a worktree shows as its project).
        let list = Rect::from_min_max(pos2(full.left(), y), pos2(full.right(), gear.top() - 8.0));
        let active_root = self.active.and_then(|id| self.root_project(id)).map(|x| x.id);
        let rows: Vec<(tc_core::ProjectId, String, String, egui::Color32, Option<egui::Color32>)> = self
            .projects
            .iter()
            .filter(|x| x.parent.is_none())
            .map(|x| {
                // Its dot: the most urgent of it and its worktrees, when it asks for the user.
                let mut dot: Option<(u8, egui::Color32)> = None;
                for pr in self.projects.iter().filter(|c| c.id == x.id || c.parent == Some(x.id)) {
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
                (x.id, x.name.clone(), x.letters(), x.color32(), dot.map(|(_, c)| c))
            })
            .collect();
        let mut pick = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            ui.set_clip_rect(list.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt("sidebar-compact")
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    for (id, name, letters, color, dot) in &rows {
                        let (cell, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                        let on = active_root == Some(*id) && self.view == View::Focus && !self.attention_open;
                        let back = Rect::from_center_size(cell.center(), vec2(36.0, 36.0));
                        if on {
                            raised(ui.painter(), back, 8, &p);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(back, CornerRadius::same(8), p.hover);
                        }
                        let tile = Rect::from_center_size(cell.center(), vec2(20.0, 20.0));
                        kit::paint_project_tile(ui.painter(), tile, letters, *color, dark);
                        if let Some(c) = dot {
                            let at = pos2(back.left() + 27.0 + 4.0, back.top() + 1.0 + 4.0);
                            ui.painter().circle_filled(at, 5.0, p.chrome);
                            ui.painter().circle_filled(at, 3.0, *c);
                        }
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("project:{name}"), cell);
                        if resp.on_hover_text(name).clicked() {
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
        let current = self.project(id).map(|x| x.color).unwrap_or_default() as usize;
        let letters = self.project(id).map(|x| x.letters()).unwrap_or_default();
        let color = kit::submenu_item(ui, Some(icons::PALETTE), t("Colour"));
        egui::containers::menu::SubMenu::new().show(ui, &color, |ui| {
            let p = style::pal(ui);
            let dark = ui.visuals().dark_mode;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (ci, (r, g, b)) in crate::project::COLORS.iter().enumerate().take(6) {
                    let c = egui::Color32::from_rgb(*r, *g, *b);
                    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                    if ci == current {
                        ui.painter().rect_stroke(rect, CornerRadius::same(7), Stroke::new(2.0, p.accent), StrokeKind::Inside);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(7), p.hover);
                    }
                    kit::paint_project_tile(ui.painter(), Rect::from_center_size(rect.center(), vec2(20.0, 20.0)), &letters, c, dark);
                    if resp.clicked() {
                        self.actions.push(Action::SetProjectColor(id, ci as u8));
                        ui.close();
                    }
                }
            });
        });
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
        let tops: Vec<tc_core::ProjectId> = self.top_order();
        let at = tops.iter().position(|x| *x == id);
        let first = at == Some(0);
        let last = at.is_some_and(|i| i + 1 == tops.len());
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
    pct: f32,
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
fn nav_item(ui: &mut Ui, rect: Rect, icon: &str, label: &str, count: Option<usize>, keys: Option<&str>, on: bool, p: &style::Palette) -> egui::Response {
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
    painter.text(pos2(rect.left() + 8.0 + 8.0, cy), Align2::CENTER_CENTER, icon, style::icon_font(16.0), ink);
    let font = if on { style::medium(style::BODY) } else { egui::FontId::proportional(style::BODY) };
    painter.text(pos2(rect.left() + 8.0 + 16.0 + 10.0, cy), Align2::LEFT_CENTER, label, font, ink);
    let mut right = rect.right() - 8.0;
    if let Some(n) = count {
        right -= count_pill(painter, pos2(right, cy), n, p, ui.visuals().dark_mode);
    }
    if let Some(k) = keys
        && count.is_none()
    {
        painter.text(pos2(right, cy), Align2::RIGHT_CENTER, k, style::mono(style::DATA), p.text_faint);
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

/// A project's row: its tile, its name (the letters found in the accent), how many wait,
/// its status.
#[allow(clippy::too_many_arguments)]
fn paint_project_row(
    ui: &Ui,
    rect: Rect,
    row: &ProjectRow,
    letters: &str,
    color: egui::Color32,
    marks: &[bool],
    selected: bool,
    hovered: bool,
    dim: bool,
    chosen: bool,
    dark: bool,
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
    let tile = Rect::from_min_size(pos2(rect.left() + 6.0, cy - 10.0), vec2(20.0, 20.0));
    kit::paint_project_tile(painter, tile, letters, color, dark);
    let mut right = rect.right() - 8.0;
    if chosen {
        let kw = kit::kbd_width(painter, "↵");
        kit::paint_kbd_text(painter, pos2(right - kw, cy), "↵", p);
        right -= kw + 8.0;
    } else if !dim {
        if let Some(s) = row.status.filter(|s| shows_status(*s)) {
            status::paint_glyph(painter, pos2(right - 7.0, cy), s, p, 14.0);
            right -= 14.0 + 9.0;
        } else if row.asleep {
            painter.circle_filled(pos2(right - 3.0, cy), 2.5, p.text_ghost);
            right -= 6.0 + 9.0;
        }
        if row.waiting > 0 {
            right -= count_pill(painter, pos2(right, cy), row.waiting, p, dark) + 9.0;
        }
    }
    let x = tile.right() + 9.0;
    let ink = if dim { p.text_faint } else if selected || hovered { p.text } else { p.text_muted };
    let font = if selected { style::medium(style::BODY) } else { egui::FontId::proportional(style::BODY) };
    let mut job = name_job(&row.name, marks, font, ink, p);
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(16.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, ink);
}

/// A worktree under its project: the rail, ⑂, its name, its branch, its status.
#[allow(clippy::too_many_arguments)]
fn paint_worktree_row(ui: &Ui, rect: Rect, name: &str, branch: &str, marks: &[bool], st: Option<Status>, selected: bool, hovered: bool, p: &style::Palette) {
    let painter = ui.painter();
    let body = Rect::from_min_max(pos2(rect.left() + 6.0 + 18.0 - 4.0, rect.top()), rect.max);
    if selected {
        painter.rect_filled(body, CornerRadius::same(6), p.accent_soft);
    } else if hovered {
        painter.rect_filled(body, CornerRadius::same(6), p.hover);
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let cy = rect.center().y;
    painter.vline(rect.left() + 6.0 + 9.0, (rect.top() - 1.0)..=(rect.bottom() + 1.0), Stroke::new(1.0, p.border_strong));
    let mut x = rect.left() + 6.0 + 18.0 + 4.0;
    painter.text(pos2(x + 7.0, cy), Align2::CENTER_CENTER, icons::GIT_FORK, style::icon_font(14.0), p.text_faint);
    x += 14.0 + 7.0;
    let mut right = rect.right() - 8.0;
    if let Some(s) = st {
        status::paint_glyph(painter, pos2(right - 6.0, cy), s, p, 12.0);
        right -= 12.0 + 7.0;
    }
    if !branch.is_empty() && branch != name {
        let room = ((right - x) * 0.45).max(0.0);
        let mut job = egui::text::LayoutJob::simple_singleline(branch.to_string(), style::mono(style::DATA), p.text_faint);
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let g = painter.layout_job(job);
        if g.size().x > 12.0 {
            right -= g.size().x;
            painter.galley(pos2(right, cy - g.size().y / 2.0), g, p.text_faint);
            right -= 7.0;
        }
    }
    let ink = if selected || hovered { p.text } else { p.text_muted };
    let mut job = name_job(name, marks, egui::FontId::proportional(style::SMALL), ink, p);
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(16.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, ink);
}

/// A session (a tab) under the selected project: the rail, who works there, its title, how
/// long or where, its status.
fn paint_session_row(ui: &Ui, rect: Rect, s: &SessionRow, hovered: bool, p: &style::Palette) {
    let painter = ui.painter();
    let body = Rect::from_min_max(pos2(rect.left() + 6.0 + 18.0 - 4.0, rect.top()), rect.max);
    if s.on {
        painter.rect_filled(body, CornerRadius::same(6), p.accent_soft);
    } else if hovered {
        painter.rect_filled(body, CornerRadius::same(6), p.hover);
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let cy = rect.center().y;
    painter.vline(rect.left() + 6.0 + 9.0, (rect.top() - 1.0)..=(rect.bottom() + 1.0), Stroke::new(1.0, p.border_strong));
    let mut x = rect.left() + 6.0 + 18.0 + 4.0;
    kit::paint_agent_avatar(painter, Rect::from_center_size(pos2(x + 8.0, cy), vec2(16.0, 16.0)), s.agent, p);
    x += 16.0 + 7.0;
    let mut right = rect.right() - 8.0;
    if let Some(st) = s.status {
        status::paint_glyph(painter, pos2(right - 6.0, cy), st, p, 12.0);
        right -= 12.0 + 7.0;
    }
    if !s.meta.is_empty() {
        let g = painter.layout_no_wrap(s.meta.clone(), style::mono(style::DATA), p.text_faint);
        right -= g.size().x;
        painter.galley(pos2(right, cy - g.size().y / 2.0), g, p.text_faint);
        right -= 7.0;
    }
    let ink = if s.on || hovered { p.text } else { p.text_muted };
    let mut job = egui::text::LayoutJob::simple_singleline(s.title.clone(), egui::FontId::proportional(style::SMALL), ink);
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
        // Folded away, its worktrees' most urgent status shows on the project.
        if pr.folded {
            for w in worktrees {
                let kid = self.project_row(w, p);
                row.waiting += kid.waiting;
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
}
