//! The sidebar: where and who. The Focus | Grid switch, the inbox of what waits for the user
//! (only while something does), the project list (FR-P3) by name with a search over it, each
//! project with its most urgent status, a dot per pane and a count of what it has that was not
//! seen yet (right-click for the rest), and Settings at the bottom. Files live in the Code drawer
//! at the right.

use eframe::egui::{self, Align2, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::{App, Dialog};
use crate::i18n::t;
use crate::project::COLORS;
use tc_core::model::View;
use tc_core::status::Status;

/// A project: its name over a line about it.
const PROJECT_H: f32 = 46.0;
/// A worktree under its project: one line.
const WORKTREE_H: f32 = 30.0;
/// Height of the row at the top of the sidebar and of the project header beside it.
pub const TOP_H: f32 = 40.0;
/// Height of a section title ("PROJECTS").
pub const SECTION_H: f32 = 28.0;
/// The row at the bottom (Settings).
const FOOT_H: f32 = 40.0;
const PAD: f32 = 8.0;
/// A group's line in the project list (Active, Idle).
const GROUP_H: f32 = 26.0;

/// How long a project stays under Active after it went quiet (it does not leave just as you
/// reach for it). The end-to-end checks make it shorter.
fn active_linger() -> std::time::Duration {
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var("TERNARYCODE_ACTIVE_LINGER_MS").ok().and_then(|v| v.parse().ok()) {
        return std::time::Duration::from_millis(ms);
    }
    std::time::Duration::from_secs(30)
}

/// A line of the project list: a group's header (Active or not), or a project's row (its index,
/// the letters found, dimmed, another worktree after it).
enum Line {
    Group(bool),
    Row((usize, Vec<bool>, bool, bool)),
}
/// At most this many waiting panes are listed in the inbox (the rest: "N more…").
const INBOX_MAX: usize = 3;

impl App {
    pub fn sidebar(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();

        // ----- top row: room for the window buttons, the window drag area, the toggle ----------
        let top = Rect::from_min_size(full.min, vec2(full.width(), TOP_H));
        super::view::window_drag_area(ui, top, "sidebar-top");
        let toggle = Rect::from_center_size(pos2(top.right() - PAD - 13.0, top.center().y), vec2(26.0, 26.0));
        let r = ui
            .scope_builder(egui::UiBuilder::new().max_rect(toggle), |ui| {
                kit::icon_button_ex(ui, icons::SIDEBAR_SIMPLE, &kit::tip(t("Hide Sidebar"), Action::ToggleSidebar), false, 26.0)
            })
            .inner;
        if r.clicked() {
            self.actions.push(Action::ToggleSidebar);
        }

        // ----- the switch, the inbox ------------------------------------------------------------
        let mut y = top.bottom();
        if !self.projects.is_empty() {
            let row = Rect::from_min_size(pos2(full.left() + PAD, y + 2.0), vec2(full.width() - 2.0 * PAD, kit::SEGMENTED_H));
            self.view_switch(ui, row);
            y = row.bottom() + 8.0;
        }
        if let Some(b) = self.inbox(ui, Rect::from_min_max(pos2(full.left() + PAD, y), pos2(full.right() - PAD, full.bottom()))) {
            y = b + 8.0;
        }

        // ----- the footer: Settings ---------------------------------------------------------------
        let foot = Rect::from_min_max(pos2(full.left(), full.bottom() - FOOT_H), full.max);
        self.sidebar_footer(ui, foot);

        // ----- projects -------------------------------------------------------------------------
        let list = Rect::from_min_max(pos2(full.left(), y), pos2(full.right(), foot.top()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
            ui.set_clip_rect(list);
            self.project_list(ui)
        });
    }

    /// What waits for the user, most urgent first (the attention queue), at most three, in a
    /// warm box under the switch. Nothing at all when nothing waits. Returns its bottom.
    ///
    /// ⌥⌘J opens it in full (instead of the attention list over the window, which is for a
    /// hidden sidebar): every row, the keyboard's row with what can be done with it, and the keys
    /// (`App::attention_panel` takes them).
    fn inbox(&mut self, ui: &mut Ui, area: Rect) -> Option<f32> {
        let queue = self.attention_queue();
        if queue.is_empty() {
            return None;
        }
        let p = style::pal(ui);
        let items: Vec<(tc_core::PaneId, Status, String, String, String)> = queue
            .iter()
            .filter_map(|e| {
                let pane = self.panes.get(&e.pane)?;
                let project = self.project_label(pane.project);
                Some((e.pane, e.status, project, self.attention_detail(pane), status::short_age(e.since)))
            })
            .collect();
        let n = items.len();
        if n == 0 {
            return None;
        }
        const HEAD_H: f32 = 30.0;
        const ROW_H: f32 = 45.0;
        const MORE_H: f32 = 27.0;
        // The keyboard's row, open: its buttons under it; the keys at the bottom.
        const ACT_H: f32 = 32.0;
        const KEYS_H: f32 = 28.0;
        let expanded = self.attention_open;
        if self.attention_sel >= n {
            self.attention_sel = n - 1;
        }
        let sel = self.attention_sel;
        let (first, shown, more) = if expanded {
            let room = ((area.height() * 0.6 - HEAD_H - ACT_H - KEYS_H) / ROW_H).floor().max(1.0) as usize;
            let shown = n.min(room);
            if sel < self.inbox_scroll {
                self.inbox_scroll = sel;
            } else if sel >= self.inbox_scroll + shown {
                self.inbox_scroll = sel + 1 - shown;
            }
            self.inbox_scroll = self.inbox_scroll.min(n - shown);
            (self.inbox_scroll, shown, 0)
        } else {
            let shown = n.min(INBOX_MAX);
            (0, shown, n - shown)
        };
        let h = HEAD_H
            + shown as f32 * ROW_H
            + if expanded { ACT_H + KEYS_H } else { 0.0 }
            + if more > 0 { MORE_H } else { 0.0 };
        let rect = Rect::from_min_size(area.min, vec2(area.width(), h));
        #[cfg(debug_assertions)]
        crate::devtest::record("inbox", rect);
        if expanded {
            self.inbox_rect = Some(rect);
        }
        ui.painter().rect(
            rect,
            CornerRadius::same(9),
            style::over(p.chrome, p.warn, if expanded { 0.10 } else { 0.07 }),
            Stroke::new(1.0, style::soft(p.warn, if expanded { 0.5 } else { 0.26 })),
            StrokeKind::Inside,
        );
        let line = Stroke::new(1.0, style::soft(p.warn, 0.14));

        // Header: it opens and closes in full (⌥⌘J).
        let head = Rect::from_min_size(rect.min, vec2(rect.width(), HEAD_H));
        let hr = ui.interact(head, egui::Id::new("inbox-head"), Sense::click());
        if hr.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let tip = if expanded { t("Close") } else { t("Show All") };
        if hr.on_hover_text(kit::tip(tip, Action::ToggleAttention)).clicked() {
            self.actions.push(Action::ToggleAttention);
        }
        let painter = ui.painter();
        let cy = head.center().y;
        let mut x = head.left() + 10.0;
        let g = painter.layout_no_wrap(icons::TRAY.to_string(), style::icon_font(13.0), p.warn);
        let iw = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.warn);
        x += iw + 7.0;
        let g = painter.layout_no_wrap(t("Waiting for you").to_string(), style::semibold(11.5), p.warn);
        let w = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.warn);
        x += w + 7.0;
        let count = n.to_string();
        let bw = kit::paint_badge(painter, pos2(10_000.0, cy), &count, egui::Color32::TRANSPARENT, egui::Color32::TRANSPARENT, 15.0).width();
        kit::paint_badge(painter, pos2(x + bw, cy), &count, p.warn, p.on_accent, 15.0);
        if expanded {
            let k = kit::kbd_width(painter, "esc");
            kit::paint_kbd_text(painter, pos2(head.right() - 8.0 - k, cy), "esc", &p);
            let all = Rect::from_center_size(pos2(head.right() - 8.0 - k - 6.0 - 11.0, cy), vec2(22.0, 22.0));
            let r = ui
                .scope_builder(egui::UiBuilder::new().max_rect(all), |ui| kit::icon_button_ex(ui, icons::CHECK, t("Mark All Seen"), false, 22.0))
                .inner;
            if r.clicked() {
                self.actions.push(Action::MarkAllSeen);
            }
        } else if let Some(k) = kit::keys(&Action::ToggleAttention) {
            let g = kit::keys_galley(painter, k, p.text_faint);
            painter.galley(pos2(head.right() - 8.0 - g.size().x, cy - g.size().y / 2.0), g, p.text_faint);
        }

        // Rows.
        let mut top = head.bottom();
        let mut chosen: Option<(usize, super::statusbar::Choice)> = None;
        for (i, (pane, s, project, detail, age)) in items.iter().enumerate().skip(first).take(shown) {
            let open = expanded && i == sel;
            let row = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), ROW_H + if open { ACT_H } else { 0.0 }));
            let r = ui.interact(row, egui::Id::new(("inbox-row", *pane)), Sense::click());
            #[cfg(debug_assertions)]
            crate::devtest::record(format!("inbox:{pane}"), row);
            #[cfg(debug_assertions)]
            if expanded {
                crate::devtest::record(format!("att:{pane}"), row);
            }
            let painter = ui.painter();
            let last = i + 1 == first + shown && more == 0 && !expanded;
            let radius = if last { CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 } } else { CornerRadius::ZERO };
            let fill_rect = row.shrink2(vec2(1.0, 0.0)).with_max_y(row.bottom() - if last { 1.0 } else { 0.0 });
            if open {
                painter.rect_filled(fill_rect, radius, p.accent_soft);
            } else if r.hovered() {
                painter.rect_filled(fill_rect, radius, style::soft(p.warn, 0.08));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            painter.hline(row.x_range().shrink(1.0), row.top() + 0.5, line);
            let c = status::color(*s, &p);
            let l1y = row.top() + 6.0 + 8.0;
            painter.text(pos2(row.left() + 10.0 + 7.0, l1y + 1.0), Align2::CENTER_CENTER, status::icon(*s), style::icon_font(14.0), c);
            let tx = row.left() + 33.0;
            let right = row.right() - 10.0;
            // Age at the right of the first line; ⌘J before it on the most urgent.
            let age_g = painter.layout_no_wrap(age.clone(), egui::FontId::proportional(11.0), p.text_faint);
            let mut rx = right - age_g.size().x;
            painter.galley(pos2(rx, l1y - age_g.size().y / 2.0), age_g, p.text_faint);
            if i == 0
                && !expanded
                && let Some(k) = kit::keys(&Action::NextAttention)
            {
                let kg = painter.layout_no_wrap(k.to_string(), style::medium(10.5), p.text_muted);
                rx -= kg.size().x + 12.0 + 6.0;
                kit::paint_kbd_text(painter, pos2(rx, l1y), k, &p);
            }
            let mut job = egui::text::LayoutJob::simple_singleline(project.clone(), style::medium(12.5), p.text);
            job.wrap = egui::text::TextWrapping::truncate_at_width((rx - 6.0 - tx).max(10.0));
            let g = painter.layout_job(job);
            painter.galley(pos2(tx, l1y - g.size().y / 2.0), g, p.text);
            let mut job = egui::text::LayoutJob::simple_singleline(detail.clone(), egui::FontId::proportional(style::SMALL), p.text_muted);
            job.wrap = egui::text::TextWrapping::truncate_at_width((right - tx).max(10.0));
            let g = painter.layout_job(job);
            painter.galley(pos2(tx, row.top() + 6.0 + 16.0 + 2.0), g, p.text_muted);
            if open {
                // What can be done with it: jump there, review it (when done), mark it as seen.
                use super::statusbar::Choice;
                let cy = row.top() + ROW_H + ACT_H / 2.0 - 4.0;
                let mut bx = tx;
                let mut button = |ui: &mut Ui, kind: kit::Kind, icon: Option<&str>, text: &str, keys: Option<&str>| {
                    let w = kit::button_width(ui, icon, text, keys, true);
                    let r = Rect::from_min_size(pos2(bx, cy - 12.0), vec2(w, 24.0));
                    bx += w + 6.0;
                    ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| kit::button_keys(ui, kind, icon, text, keys, true)).inner
                };
                if button(ui, kit::Kind::Primary, None, t("Jump"), Some("↵")).clicked() {
                    chosen = Some((i, Choice::Jump));
                }
                if *s == Status::Done {
                    let b = button(ui, kit::Kind::Secondary, Some(icons::GIT_DIFF), t("Review"), None);
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("att-review:{pane}"), b.rect);
                    if b.clicked() {
                        chosen = Some((i, Choice::Review));
                    }
                }
                let seen = Rect::from_min_size(pos2(bx, cy - 12.0), vec2(24.0, 24.0));
                let b = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(seen), |ui| kit::icon_button_ex(ui, icons::EYE, &format!("{}   ⌫", t("Mark as Seen")), false, 24.0))
                    .inner;
                if b.clicked() {
                    chosen = Some((i, Choice::Seen));
                }
            } else if r.clicked() {
                if expanded {
                    chosen = Some((i, super::statusbar::Choice::Jump));
                } else {
                    self.actions.push(Action::FocusPane(*pane));
                }
            } else if expanded && r.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                self.attention_sel = i;
            }
            top = row.bottom();
        }
        if expanded {
            // The keys at the bottom.
            let keys = Rect::from_min_max(pos2(rect.left(), top), rect.max);
            ui.painter().hline(keys.x_range().shrink(1.0), keys.top() + 0.5, line);
            let inner = Rect::from_min_max(pos2(keys.left() + 10.0, keys.top()), pos2(keys.right() - 10.0, keys.bottom()));
            // R only for a finished agent; what does not fit is left out (the row's buttons say it).
            let done = items.get(sel).is_some_and(|x| x.1 == Status::Done);
            let mut hints: Vec<(&[&str], &str)> = vec![(&["↑", "↓"], t("choose"))];
            if done {
                hints.push((&["R"], t("review")));
            }
            hints.push((&["⌫"], t("seen")));
            while hints.len() > 1 && super::statusbar::key_hints_width(ui, &hints) > inner.width() {
                hints.remove(if done && hints.len() == 3 { 1 } else { hints.len() - 1 });
            }
            super::statusbar::paint_key_hints(ui, inner, &hints, None);
            if let Some(c) = chosen {
                let mut close = false;
                self.attention_choose(&queue, c, &mut close);
                if close {
                    self.attention_open = false;
                }
            }
        } else if more > 0 {
            let row = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), MORE_H));
            let r = ui.interact(row, egui::Id::new("inbox-more"), Sense::click());
            let painter = ui.painter();
            if r.hovered() {
                painter.rect_filled(row.shrink(1.0), CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 }, style::soft(p.warn, 0.08));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            painter.hline(row.x_range().shrink(1.0), row.top() + 0.5, line);
            painter.text(
                pos2(row.left() + 33.0, row.center().y),
                Align2::LEFT_CENTER,
                crate::i18n::tf("{} more…", &[&more.to_string()]),
                egui::FontId::proportional(style::SMALL),
                p.text_muted,
            );
            if r.clicked() {
                self.actions.push(Action::ToggleAttention);
            }
        }
        Some(rect.bottom())
    }

    /// The second line of something waiting: who, and what it says or what happened
    /// ("Claude Code · Bash: flutter test", "zsh · Failed (code 1)").
    pub fn attention_detail(&self, pane: &crate::pane::Pane) -> String {
        format!("{} · {}", pane.label(), attention_what(pane))
    }

    fn sidebar_footer(&mut self, ui: &mut Ui, rect: Rect) {
        let p = style::pal(ui);
        ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, p.border));
        let label = t("Settings");
        let g = ui.painter().layout_no_wrap(label.to_string(), style::medium(12.5), p.text_muted);
        let w = 6.0 + 15.0 + 7.0 + g.size().x + 6.0;
        let b = Rect::from_min_size(pos2(rect.left() + PAD, rect.center().y - 13.0), vec2(w, 26.0));
        let r = ui.interact(b, egui::Id::new("sidebar-settings"), Sense::click());
        #[cfg(debug_assertions)]
        crate::devtest::record("sidebar-settings", b);
        let painter = ui.painter();
        let color = if r.hovered() { p.text } else { p.text_muted };
        if r.hovered() {
            painter.rect_filled(b, CornerRadius::same(style::RADIUS), p.hover);
        }
        painter.text(pos2(b.left() + 6.0 + 7.5, b.center().y), Align2::CENTER_CENTER, icons::GEAR_SIX, style::icon_font(15.0), color);
        let gw = g.size();
        painter.galley(pos2(b.left() + 28.0, b.center().y - gw.y / 2.0), g, color);
        if let Some(k) = kit::keys(&Action::Settings) {
            let kg = kit::keys_galley(painter, k, p.text_faint);
            painter.galley(pos2(rect.right() - 14.0 - kg.size().x, rect.center().y - kg.size().y / 2.0), kg, p.text_faint);
        }
        if r.clicked() {
            self.actions.push(Action::Settings);
        }
    }

    fn project_list(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let area = ui.max_rect();
        let total = self.projects.len();

        // Title: PROJECTS, how many (how many found, while searching: below), and + at the right.
        let title = Rect::from_min_size(area.min, vec2(area.width(), SECTION_H));
        let g = kit::section_galley(ui.painter(), t("Projects"), p.text_faint);
        let count_at = pos2(title.left() + 14.0 + g.size().x + 6.0, title.center().y);
        ui.painter().galley(pos2(title.left() + 14.0, title.center().y - g.size().y / 2.0), g, p.text_faint);
        let paint_count = |ui: &Ui, text: String| ui.painter().text(count_at, Align2::LEFT_CENTER, text, style::medium(10.5), p.text_faint);
        let add = Rect::from_center_size(pos2(title.right() - PAD - 11.0, title.center().y), vec2(22.0, 22.0));
        #[cfg(debug_assertions)]
        crate::devtest::record("add-project", add);
        if ui
            .scope_builder(egui::UiBuilder::new().max_rect(add), |ui| {
                kit::icon_button_ex(ui, icons::PLUS, &kit::tip(t("Add Project…"), Action::AddProject), false, 22.0)
            })
            .inner
            .clicked()
        {
            self.actions.push(Action::AddProject);
        }
        if total == 0 {
            paint_count(ui, "0".into());
            ui.painter().text(
                pos2(title.left() + 14.0, title.bottom() + 10.0),
                Align2::LEFT_CENTER,
                t("No projects yet."),
                egui::FontId::proportional(style::SMALL),
                p.text_faint,
            );
            return;
        }

        // The search field. While it has the keyboard: ↑ ↓ choose a project, Enter opens it (and
        // empties the search), Esc empties the search and gives the keyboard back.
        let fid = egui::Id::new("project-filter");
        let field = Rect::from_min_size(pos2(area.left() + PAD, title.bottom()), vec2(area.width() - 2.0 * PAD, 26.0));
        #[cfg(debug_assertions)]
        crate::devtest::record("project-filter", field);
        let typing = ui.memory(|m| m.has_focus(fid));
        let found: Vec<usize> = self.project_matches().into_iter().filter(|(_, dim)| !dim).map(|(i, _)| i).collect();
        let n = found.len();
        let mut sel = self.project_filter_sel.min(n.saturating_sub(1));
        let mut reveal = false;
        let mut open = None;
        if typing {
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
                reveal = true;
            }
            if up && n > 0 {
                sel = (sel + n - 1) % n;
                reveal = true;
            }
            if enter && let Some(&i) = found.get(sel) {
                open = Some(self.projects[i].id);
            }
            if esc || open.is_some() {
                self.project_filter.clear();
                sel = 0;
                ui.memory_mut(|m| m.surrender_focus(fid));
            }
        }
        let mut text = self.project_filter.clone();
        let fr = ui
            .scope_builder(egui::UiBuilder::new().max_rect(field), |ui| {
                kit::field(ui, fid, &mut text, Some(icons::MAGNIFYING_GLASS), t("Search projects"), field.width(), true)
            })
            .inner;
        if fr.changed() {
            self.project_filter = text;
            sel = 0;
            reveal = true;
        }
        // Esc and ↑ ↓ are the field's (egui would otherwise take the keyboard away on them).
        if fr.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(fid, egui::EventFilter { vertical_arrows: true, escape: true, ..Default::default() })
            });
        }
        // An × empties it (and keeps the keyboard there).
        if !self.project_filter.is_empty() {
            let x = Rect::from_center_size(pos2(field.right() - 13.0, field.center().y), vec2(18.0, 18.0));
            let r = ui.interact(x, fid.with("clear"), Sense::click()).on_hover_text(t("Clear"));
            let c = if r.hovered() { p.text } else { p.text_faint };
            ui.painter().text(x.center(), Align2::CENTER_CENTER, icons::X_CIRCLE, style::icon_font(14.0), c);
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
        // The list follows what is in the field now (typed, or emptied by Enter, Esc or ×).
        let shown = self.project_matches();
        let query = self.project_filter.trim().to_string();
        let filtering = !query.is_empty();
        let typing = ui.memory(|m| m.has_focus(fid));
        let tops = self.projects.iter().filter(|p| !p.is_worktree()).count();
        let found_tops = shown.iter().filter(|(i, dim)| !dim && !self.projects[*i].is_worktree()).count();
        paint_count(ui, if filtering { crate::i18n::tf("{} of {}", &[&found_tops.to_string(), &tops.to_string()]) } else { tops.to_string() });

        let list = Rect::from_min_max(pos2(area.left(), field.bottom() + 6.0), area.max);
        if shown.is_empty() {
            ui.painter().text(
                pos2(list.left() + 14.0, list.top() + 12.0),
                Align2::LEFT_CENTER,
                t("No matching projects."),
                egui::FontId::proportional(style::SMALL),
                p.text_faint,
            );
            return;
        }
        // Each row: its project, the letters found, dimmed (a project shown for a worktree found
        // under it), and for a worktree whether another one follows it. What a row shows is
        // worked out only when it is in view.
        let rows: Vec<(usize, Vec<bool>, bool, bool)> = shown
            .iter()
            .enumerate()
            .map(|(k, &(i, dim))| {
                let pr = &self.projects[i];
                let marks = if filtering && !dim { super::palette::matched(&query, &pr.name) } else { Vec::new() };
                let more = pr.is_worktree() && shown.get(k + 1).is_some_and(|&(j, _)| self.projects[j].parent == pr.parent);
                (i, marks, dim, more)
            })
            .collect();
        // Projects where something runs or waits first, under Active (the Grid's rule), each
        // with its worktrees. While the pointer is over the list nothing moves.
        let hold = ui.rect_contains_pointer(list) || typing;
        let active = if self.settings.group_active { self.sidebar_active(hold) } else { std::collections::HashSet::new() };
        let top_of = |i: usize| {
            let pr = &self.projects[i];
            pr.parent.filter(|x| self.project(*x).is_some()).unwrap_or(pr.id)
        };
        let grouped = !active.is_empty();
        let (first, second): (Vec<_>, Vec<_>) = rows.into_iter().partition(|(i, ..)| grouped && active.contains(&top_of(*i)));
        if !filtering {
            self.sidebar_order = first.iter().chain(second.iter()).filter(|(i, ..)| !self.projects[*i].is_worktree()).map(|(i, ..)| self.projects[*i].id).collect();
        }
        let tops = |v: &[(usize, Vec<bool>, bool, bool)]| v.iter().filter(|(i, _, dim, _)| !dim && !self.projects[*i].is_worktree()).count();
        let counts = (tops(&first), tops(&second));
        let summary = if grouped { self.group_summary(&active) } else { [0; 4] };
        let mut lines: Vec<Line> = Vec::new();
        if grouped && !first.is_empty() {
            lines.push(Line::Group(true));
            if !self.sidebar_folded.contains("active") {
                lines.extend(first.into_iter().map(Line::Row));
            }
        }
        if grouped && !second.is_empty() {
            lines.push(Line::Group(false));
            if !self.sidebar_folded.contains("idle") {
                lines.extend(second.into_iter().map(Line::Row));
            }
        } else if !grouped {
            lines.extend(second.into_iter().map(Line::Row));
        }
        let mut toggle_group: Option<&str> = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink2(vec2(6.0, 0.0))), |ui| {
            ui.set_clip_rect(list);
            // The search's results scroll on their own: the whole list keeps its place.
            egui::ScrollArea::vertical()
                .id_salt(if filtering { "project-list-found" } else { "project-list" })
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let mut result = 0;
                    let mut groups_seen = 0;
                    for line in lines.iter() {
                        let (i, marks, dim, more) = match line {
                            Line::Group(is_active) => {
                                // Space before the second group's line.
                                if groups_seen > 0 {
                                    ui.add_space(8.0);
                                }
                                groups_seen += 1;
                                let key = if *is_active { "active" } else { "idle" };
                                let folded = self.sidebar_folded.contains(key);
                                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), GROUP_H), Sense::click());
                                #[cfg(debug_assertions)]
                                crate::devtest::record(format!("sidebar-group:{key}"), rect);
                                let (label, count) = if *is_active { (t("Active"), counts.0) } else { (t("Idle"), counts.1) };
                                let summary: &[(&str, egui::Color32, usize)] = if *is_active {
                                    &[(icons::CIRCLE_NOTCH, p.info, summary[0]), (icons::BELL, p.warn, summary[3]), (icons::X_CIRCLE, p.err, summary[2]), (icons::CHECK_CIRCLE, p.ok, summary[1])]
                                } else {
                                    &[]
                                };
                                paint_group_head(ui, rect, label, count, folded, resp.hovered(), summary, &p);
                                let resp = if *is_active { resp.on_hover_text(t("Only projects where something runs or waits, or an agent can be resumed")) } else { resp };
                                if resp.clicked() {
                                    toggle_group = Some(key);
                                }
                                continue;
                            }
                            Line::Row(row) => (&row.0, &row.1, &row.2, &row.3),
                        };
                        let worktree = self.projects[*i].is_worktree();
                        let h = if worktree { WORKTREE_H } else { PROJECT_H };
                        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("project:{}", self.projects[*i].name), rect);
                        let id = self.projects[*i].id;
                        let active = self.view == View::Focus && self.active == Some(id);
                        // The keyboard's row (while the search has it) looks hovered, with ↵.
                        let chosen = typing && !dim && result == sel;
                        if !dim {
                            result += 1;
                        }
                        if chosen && reveal {
                            ui.scroll_to_rect(rect, None);
                        }
                        // Scrolled out of view: only its place (it is drawn when scrolled to).
                        if !ui.is_rect_visible(rect) {
                            continue;
                        }
                        let row = &self.project_row(&self.projects[*i], &p);
                        let hovered = response.hovered() || chosen;
                        if worktree {
                            let keys = chosen.then_some("↵");
                            paint_worktree_row(ui, rect, row, marks, active, hovered, *more, keys, &p);
                        } else {
                            let keys = if chosen { Some("↵") } else { self.top_index(id).and_then(|t| kit::keys(&Action::SelectProjectIndex(t))) };
                            let toggle = paint_project_row(ui, rect, row, marks, active, hovered, keys, *dim, &p);
                            // "⑂ 2 ˅": folds and unfolds its worktrees.
                            if let Some(fold) = toggle {
                                let tr = ui.interact(fold, egui::Id::new(("fold-worktrees", id)), Sense::click());
                                #[cfg(debug_assertions)]
                                crate::devtest::record(format!("fold:{}", row.name), fold);
                                let tip = if row.folded { t("Show Worktrees") } else { t("Hide Worktrees") };
                                if tr.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
                                    self.actions.push(Action::ToggleWorktrees(id));
                                    continue;
                                }
                            }
                        }
                        if response.clicked() {
                            self.actions.push(Action::SelectProject(id));
                        }
                        let name = row.name.clone();
                        let index = self.top_index(id).unwrap_or(usize::MAX);
                        let response = response.on_hover_ui(|ui| self.project_tooltip(ui, id, index));
                        if worktree {
                            response.context_menu(|ui| self.worktree_menu(ui, id));
                        } else {
                            response.context_menu(|ui| self.project_menu(ui, id, &name));
                        }
                    }
                });
        });
        if let Some(key) = toggle_group {
            if !self.sidebar_folded.remove(key) {
                self.sidebar_folded.insert(key.to_string());
            }
            self.mark_dirty();
        }
    }

    /// The projects (worktrees aside) under the sidebar's Active: something runs or waits in
    /// them or their worktrees now (the Grid's rule), or did less than `active_linger` ago.
    /// While `hold` (the pointer over the list, the search typed into) it stays as it was.
    fn sidebar_active(&mut self, hold: bool) -> std::collections::HashSet<tc_core::ProjectId> {
        use super::grid::Activity;
        let now = std::time::Instant::now();
        let linger = active_linger();
        let mut wake: Option<std::time::Duration> = None;
        // One pass: a worktree's activity counts for the project it belongs to.
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
        self.sidebar_active_until.retain(|id, until| *until > now && projects.iter().any(|p| p.id == *id && !p.is_worktree()));
        // Drawn again when a program has run long enough, or a quiet project leaves.
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

    /// How many of these projects are, at their most pressing: running, done, failed, waiting
    /// for the user (in that order).
    fn group_summary(&self, ids: &std::collections::HashSet<tc_core::ProjectId>) -> [usize; 4] {
        let mut out = [0; 4];
        for &id in ids {
            let mut rank = 0;
            for pid in std::iter::once(id).chain(self.worktrees_of(id)) {
                let Some(pr) = self.project(pid) else { continue };
                for pane in pr.panes().iter().filter_map(|x| self.panes.get(x)).filter(|x| x.tracker.wants_attention()) {
                    rank = rank.max(match pane.tracker.status {
                        Status::NeedsInput => 3,
                        Status::Failed(_) => 2,
                        Status::Done | Status::CommandDone => 1,
                        _ => 0,
                    });
                }
            }
            out[rank] += 1;
        }
        out
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

    /// The switch between the two views, at the top of the list: Focus (the active project on its
    /// own) and Grid (every project). A project in the list opens in Focus.
    fn view_switch(&mut self, ui: &mut Ui, rect: Rect) {
        let focus_tip = kit::tip(t("Focus View"), Action::ToggleGrid);
        let grid_tip = kit::tip(t("Grid of All Projects"), Action::ToggleGrid);
        let options = [
            kit::Seg::new(t("Focus"), &focus_tip).icon(icons::TERMINAL_WINDOW),
            kit::Seg::new(t("Grid"), &grid_tip).icon(icons::SQUARES_FOUR),
        ];
        #[cfg(debug_assertions)]
        for (r, name) in kit::segment_rects(rect, 2).into_iter().zip(["view:focus", "view:grid"]) {
            crate::devtest::record(name, r);
        }
        let on = (self.view == View::Grid) as usize;
        if let Some(i) = kit::segmented_in(ui, rect, egui::Id::new("sidebar-views"), &options, on) {
            self.actions.push(Action::SetView(if i == 1 { View::Grid } else { View::Focus }));
        }
    }

    fn project_menu(&mut self, ui: &mut Ui, id: u64, name: &str) {
        kit::menu_body(ui);
        if kit::menu_item(ui, Some(icons::PLAY), t("Run Agent…"), kit::keys(&Action::PickPreset), false).clicked() {
            self.actions.push(Action::SelectProject(id));
            self.actions.push(Action::PickPreset);
            ui.close();
        }
        if kit::menu_item(ui, Some(icons::GIT_DIFF), t("Review Changes…"), None, false).clicked() {
            self.actions.push(Action::SelectProject(id));
            self.actions.push(Action::Review);
            ui.close();
        }
        kit::menu_sep(ui);
        if kit::menu_item(ui, Some(icons::PENCIL_SIMPLE), t("Rename…"), None, false).clicked() {
            self.dialog = Some(Dialog::Rename(id, name.to_string()));
            ui.close();
        }
        let current = self.project(id).map(|x| x.color).unwrap_or_default() as usize;
        let color = kit::submenu_item(ui, Some(icons::PALETTE), t("Color"));
        egui::containers::menu::SubMenu::new().show(ui, &color, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                ui.add_space(3.0);
                for (ci, (r, g, b)) in COLORS.iter().enumerate() {
                    let c = egui::Color32::from_rgb(*r, *g, *b);
                    let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 22.0), Sense::click());
                    let center = rect.center();
                    if ci == current {
                        ui.painter().circle_stroke(center, 10.0, Stroke::new(1.5, style::pal(ui).accent));
                    }
                    ui.painter().circle_filled(center, if resp.hovered() { 8.5 } else { 8.0 }, c);
                    if resp.clicked() {
                        self.actions.push(Action::SetProjectColor(id, ci as u8));
                        ui.close();
                    }
                }
                ui.add_space(3.0);
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
        if kit::menu_item(ui, Some(icons::TRASH), t("Remove from Workspace"), None, true).clicked() {
            self.actions.push(Action::RemoveProject(id));
            ui.close();
        }
    }
}

/// What something waiting says or what happened: the agent's question (or its last words when
/// it finished), else its status.
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
    /// Under the name, at the right: what is at work or waits (icon, text, colour).
    pub chip: Option<(&'static str, String, egui::Color32)>,
    /// How many worktrees are listed under it, and whether they are folded away.
    pub worktrees: usize,
    pub folded: bool,
    /// A worktree: the commits its branch has that its base has not.
    pub ahead: Option<usize>,
    /// It has a browser with its page open: whether an agent uses it (a globe after the dots,
    /// in the info colour then).
    pub web: Option<bool>,
}

/// A group's line: ˅ or ›, its name and how many projects, and (Active) how many run, wait,
/// failed or are done.
#[allow(clippy::too_many_arguments)]
fn paint_group_head(ui: &Ui, rect: Rect, label: &str, count: usize, folded: bool, hovered: bool, summary: &[(&str, egui::Color32, usize)], p: &style::Palette) {
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, CornerRadius::same(6), p.faint_row());
    }
    let cy = rect.center().y;
    painter.text(pos2(rect.left() + 13.0, cy), Align2::CENTER_CENTER, if folded { icons::CARET_RIGHT } else { icons::CARET_DOWN }, style::icon_font(10.0), p.text_faint);
    let g = kit::section_galley(painter, label, p.text_muted);
    let gw = g.size().x;
    painter.galley(pos2(rect.left() + 24.0, cy - g.size().y / 2.0), g, p.text_muted);
    painter.text(pos2(rect.left() + 24.0 + gw + 6.0, cy), Align2::LEFT_CENTER, count.to_string(), style::medium(10.5), p.text_faint);
    let mut x = rect.right() - 8.0;
    for (icon, color, n) in summary.iter().rev().filter(|s| s.2 > 0) {
        let num = painter.layout_no_wrap(n.to_string(), style::medium(11.0), *color);
        x -= num.size().x;
        painter.galley(pos2(x, cy - num.size().y / 2.0), num, *color);
        x -= 3.0 + 6.0;
        painter.text(pos2(x, cy), Align2::CENTER_CENTER, *icon, style::icon_font(11.5), *color);
        x -= 6.0 + 8.0;
    }
}

/// Paints a project's row: name, pane dots and badge (or the shortcut, hovered) on the first
/// line; branch and changes, and what is at work or waits, on the second.
///
/// A project with worktrees has "⑂ 2 ˅" after its dots, which folds them: returns where.
#[allow(clippy::too_many_arguments)]
fn paint_project_row(ui: &Ui, rect: Rect, row: &ProjectRow, marks: &[bool], active: bool, hovered: bool, keys: Option<&str>, dim: bool, p: &style::Palette) -> Option<Rect> {
    let painter = ui.painter_at(rect);
    // Shown only for a worktree found under it: quieter.
    let p = &if dim { p.dimmed() } else { *p };
    let fill = if active {
        p.accent_soft
    } else if hovered {
        p.faint_row()
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(rect, CornerRadius::same(8), fill);
    let (r, g, b) = COLORS[row.color as usize % COLORS.len()];
    if active {
        let bar = Rect::from_min_max(pos2(rect.left() + 1.0, rect.top() + 11.0), pos2(rect.left() + 4.0, rect.bottom() - 11.0));
        painter.rect_filled(bar, CornerRadius::same(2), egui::Color32::from_rgb(r, g, b));
    }
    let x = rect.left() + 12.0;
    let right_edge = rect.right() - 10.0;
    let l1 = rect.top() + 15.0;
    let l2 = rect.top() + 33.5;

    // Right of the first line: what was not seen (a count in the status colour), else the
    // shortcut while hovered.
    let attention = row.waiting > 0;
    let mut right = right_edge;
    if attention {
        let c = row.status.map_or(p.warn, |s| status::color(s, p));
        let b = kit::paint_badge(&painter, pos2(right, l1), &row.waiting.to_string(), c, p.on_accent, 16.0);
        right = b.left() - 6.0;
    } else if hovered && let Some(k) = keys {
        let g = kit::keys_galley(&painter, k, p.text_faint);
        right -= g.size().x;
        painter.galley(pos2(right, l1 - g.size().y / 2.0), g, p.text_faint);
        right -= 6.0;
    }
    // The pane dots after the name.
    let dots_w = if row.dots.is_empty() { 0.0 } else { row.dots.len() as f32 * 8.0 - 3.0 + 6.0 } + if row.web.is_some() { 16.0 } else { 0.0 };
    let name_color = if row.asleep {
        p.text_muted
    } else if active || attention {
        p.text
    } else {
        p.text.gamma_multiply(0.9)
    };
    let font = if active { style::semibold(style::BODY) } else { style::medium(style::BODY) };
    // The letters a search matched, in the accent.
    let mut job = if marks.iter().any(|m| *m) {
        let mut job = egui::text::LayoutJob::default();
        for (ci, ch) in row.name.chars().enumerate() {
            let hit = marks.get(ci).copied().unwrap_or(false);
            let (f, c) = if hit { (style::semibold(style::BODY), p.accent_text) } else { (font.clone(), name_color) };
            job.append(&ch.to_string(), 0.0, egui::TextFormat::simple(f, c));
        }
        job
    } else {
        egui::text::LayoutJob::simple_singleline(row.name.clone(), font, name_color)
    };
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x - dots_w).max(10.0));
    let g = painter.layout_job(job);
    let name_w = g.size().x;
    painter.galley(pos2(x, l1 - g.size().y / 2.0), g, name_color);
    let mut dx = x + name_w + 6.0 + 2.5;
    for c in &row.dots {
        painter.circle_filled(pos2(dx, l1 + 0.5), 2.5, *c);
        dx += 8.0;
    }
    // Its browser: a globe, in the info colour while an agent uses it.
    if let Some(driven) = row.web {
        let gx = if row.dots.is_empty() { x + name_w + 6.0 } else { dx - 2.5 + 2.0 };
        painter.text(pos2(gx + 6.0, l1 + 0.5), Align2::CENTER_CENTER, icons::GLOBE, style::icon_font(11.5), if driven { p.info } else { p.text_faint });
        dx = gx + 14.0 + 2.5;
    }
    // Its worktrees: how many, and the caret that folds them.
    let mut toggle = None;
    if row.worktrees > 0 {
        let start = if row.dots.is_empty() && row.web.is_none() { x + name_w + 6.0 } else { dx - 2.5 + 3.0 };
        let fork = painter.layout_no_wrap(icons::GIT_FORK.to_string(), style::icon_font(11.5), p.text_faint);
        let count = painter.layout_no_wrap(row.worktrees.to_string(), style::medium(11.0), p.text_faint);
        let caret = painter.layout_no_wrap(if row.folded { icons::CARET_RIGHT } else { icons::CARET_DOWN }.to_string(), style::icon_font(10.0), p.text_faint);
        let w = fork.size().x + 2.0 + count.size().x + 2.0 + caret.size().x;
        if start + w <= right {
            let mut cx = start;
            painter.galley(pos2(cx, l1 - fork.size().y / 2.0), fork.clone(), p.text_faint);
            cx += fork.size().x + 2.0;
            painter.galley(pos2(cx, l1 - count.size().y / 2.0), count.clone(), p.text_faint);
            cx += count.size().x + 2.0;
            painter.galley(pos2(cx, l1 - caret.size().y / 2.0 + 0.5), caret, p.text_faint);
            toggle = Some(Rect::from_min_max(pos2(start - 3.0, l1 - 9.0), pos2(start + w + 3.0, l1 + 9.0)));
        }
    }

    // Second line: the chip at the right, the branch (or where it lives) in what is left.
    let small = egui::FontId::proportional(style::SMALL);
    let mut line_right = right_edge;
    if let Some((icon, text, c)) = &row.chip {
        let max = (right_edge - x) * 0.62;
        let mut job = egui::text::LayoutJob::simple_singleline(text.clone(), small.clone(), *c);
        job.wrap = egui::text::TextWrapping::truncate_at_width((max - 15.0).max(20.0));
        let g = painter.layout_job(job);
        line_right -= g.size().x;
        painter.galley(pos2(line_right, l2 - g.size().y / 2.0), g, *c);
        line_right -= 4.0 + 11.0;
        painter.text(pos2(line_right + 5.5, l2), Align2::CENTER_CENTER, icon, style::icon_font(11.0), *c);
        line_right -= 6.0;
    }
    let room = (line_right - x).max(0.0);
    if let Some((name, changed)) = &row.branch {
        painter.text(pos2(x + 5.5, l2), Align2::CENTER_CENTER, icons::GIT_BRANCH, style::icon_font(11.0), p.text_faint);
        let mut job = egui::text::LayoutJob::default();
        job.append(name, 0.0, egui::TextFormat::simple(small.clone(), p.text_faint));
        if *changed > 0 {
            job.append(&format!("+{changed}"), 5.0, egui::TextFormat::simple(small.clone(), p.warn.gamma_multiply(0.85)));
        }
        job.wrap = egui::text::TextWrapping::truncate_at_width((room - 16.0).max(0.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(x + 16.0, l2 - g.size().y / 2.0), g, p.text_faint);
    } else {
        let mut job = egui::text::LayoutJob::simple_singleline(row.place.clone(), small, p.text_faint);
        job.wrap = egui::text::TextWrapping::truncate_at_width(room.max(10.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(x, l2 - g.size().y / 2.0), g, p.text_faint);
    }
    toggle
}

/// Paints a worktree's row under its project: the tree line at the left, its name and the
/// commits it is ahead of its base, then what is at work or waits and the count not seen yet
/// (or ↵ while the search's keyboard is on it). `more`: another worktree follows (the line goes
/// on down).
#[allow(clippy::too_many_arguments)]
fn paint_worktree_row(ui: &Ui, rect: Rect, row: &ProjectRow, marks: &[bool], active: bool, hovered: bool, more: bool, keys: Option<&str>, p: &style::Palette) {
    let painter = ui.painter_at(rect);
    let fill = if active {
        p.accent_soft
    } else if hovered {
        p.faint_row()
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(rect, CornerRadius::same(7), fill);
    let (r, g, b) = COLORS[row.color as usize % COLORS.len()];
    if active {
        let bar = Rect::from_min_max(pos2(rect.left() + 1.0, rect.top() + 8.0), pos2(rect.left() + 4.0, rect.bottom() - 8.0));
        painter.rect_filled(bar, CornerRadius::same(2), egui::Color32::from_rgb(r, g, b));
    }
    // The tree line: down from the project (on to the next worktree), then to the name.
    let line = Stroke::new(1.0, p.border_strong);
    let lx = rect.left() + 17.5;
    let cy = rect.center().y;
    painter.line_segment([pos2(lx, rect.top() - 3.0), pos2(lx, cy - 4.0)], line);
    painter.add(egui::Shape::QuadraticBezier(egui::epaint::QuadraticBezierShape::from_points_stroke(
        [pos2(lx, cy - 4.0), pos2(lx, cy), pos2(lx + 4.0, cy)],
        false,
        egui::Color32::TRANSPARENT,
        line,
    )));
    painter.line_segment([pos2(lx + 4.0, cy), pos2(lx + 8.0, cy)], line);
    if more {
        painter.line_segment([pos2(lx, cy - 4.0), pos2(lx, rect.bottom() + 3.0)], line);
    }
    let x = rect.left() + 30.0;
    let mut right = rect.right() - 10.0;
    // At the right: what was not seen (a count), else ↵ for the search's keyboard.
    if row.waiting > 0 {
        let c = row.status.map_or(p.warn, |s| status::color(s, p));
        let b = kit::paint_badge(&painter, pos2(right, cy), &row.waiting.to_string(), c, p.on_accent, 16.0);
        right = b.left() - 6.0;
    } else if let Some(k) = keys {
        let g = kit::keys_galley(&painter, k, p.text_faint);
        right -= g.size().x;
        painter.galley(pos2(right, cy - g.size().y / 2.0), g, p.text_faint);
        right -= 6.0;
    }
    // Then what is at work or waits (at most a bit more than half the row).
    let small = egui::FontId::proportional(style::SMALL);
    if let Some((icon, text, c)) = &row.chip {
        let max = (right - x) * 0.58;
        let mut job = egui::text::LayoutJob::simple_singleline(text.clone(), small.clone(), *c);
        job.wrap = egui::text::TextWrapping::truncate_at_width((max - 15.0).max(20.0));
        let g = painter.layout_job(job);
        right -= g.size().x;
        painter.galley(pos2(right, cy - g.size().y / 2.0), g, *c);
        right -= 4.0 + 11.0;
        painter.text(pos2(right + 5.5, cy), Align2::CENTER_CENTER, icon, style::icon_font(11.0), *c);
        right -= 8.0;
    }
    // The name (letters found in the accent) and "↑2".
    let name_color = if active { p.text } else { p.text.gamma_multiply(0.88) };
    let font = if active { style::semibold(12.5) } else { style::medium(12.5) };
    let mut job = egui::text::LayoutJob::default();
    for (ci, ch) in row.name.chars().enumerate() {
        let hit = marks.get(ci).copied().unwrap_or(false);
        let (f, c) = if hit { (style::semibold(12.5), p.accent_text) } else { (font.clone(), name_color) };
        job.append(&ch.to_string(), 0.0, egui::TextFormat::simple(f, c));
    }
    if let Some(ahead) = row.ahead {
        job.append(&format!("↑{ahead}"), 6.0, egui::TextFormat::simple(egui::FontId::proportional(11.0), p.text_faint));
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - x).max(10.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, name_color);
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
        // The chip: what waits; else the agent or program at work (with its progress).
        let pane = self.project_pane(pr.id).and_then(|id| self.panes.get(&id));
        let busy = pane.filter(|x| x.tracker.agent.is_some() || matches!(x.tracker.fg, tc_core::status::Foreground::Program { .. }));
        let chip = match (pane, status) {
            // Something waits (seen or not) or failed: the status says it.
            (Some(x), Some(s)) if x.tracker.wants_attention() || matches!(x.tracker.status, Status::NeedsInput | Status::Failed(_)) => {
                let s = if x.tracker.wants_attention() { s } else { x.tracker.status };
                let text = match s {
                    Status::Failed(_) => crate::i18n::tf("{} · Failed", &[&x.label()]),
                    _ => status::text(s),
                };
                Some((status::icon(s), text, status::color(s, p)))
            }
            _ => busy.map(|x| {
                let s = x.tracker.status;
                let c = match s {
                    Status::Unknown => p.text_muted,
                    Status::Idle | Status::Ended => p.text_faint,
                    _ => status::color(s, p),
                };
                let icon = if s == Status::Unknown { icons::CIRCLE_HALF } else { status::icon(s) };
                let text = match x.progress {
                    Some((_, Some(v))) => format!("{} · {v}%", x.label()),
                    _ => x.label(),
                };
                (icon, text, c)
            }),
        };
        // A worktree: gone, merged (when nothing else is to say), commits ahead.
        let chip = match (pr.is_worktree(), chip) {
            (true, _) if pr.missing => Some((icons::X_CIRCLE, t("Folder not found").to_string(), p.err)),
            (true, None) if pr.git.as_ref().is_some_and(|g| g.merged) => Some((icons::CHECK_CIRCLE, t("Merged").to_string(), p.ok)),
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
            web: self.browsers.values().filter(|b| b.project == pr.id && b.live()).map(|b| b.driven).max(),
        };
        // Folded away, its worktrees' most urgent status shows on the project.
        if pr.folded {
            for w in worktrees {
                let kid = self.project_row(w, p);
                row.waiting += kid.waiting;
                let rank = |s: Option<Status>| s.and_then(Status::priority).unwrap_or(u8::MAX);
                if rank(kid.status) < rank(row.status) {
                    row.status = kid.status;
                    row.chip = kid.chip.map(|(icon, text, c)| (icon, format!("{} · {text}", kid.name), c));
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
