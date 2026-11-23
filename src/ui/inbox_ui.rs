//! The Inbox (Slate): everything that waits for the user, from every project, in one place — an
//! agent's question, a command that failed, an account at its usage limit, a turn that finished
//! and was not looked at yet. A list in two groups (what needs the user, what finished), and
//! beside it the row chosen: what it asks or what happened, its terminal's last lines, and what
//! can be done about it. The keyboard: ↑ ↓ choose, ↵ approve (an ask) or open, O open the pane,
//! R review a finished turn, ⌫ mark seen, Esc reject (an ask) or go back.

use std::time::Instant;

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use tc_core::PaneId;
use tc_core::agent::AgentKind;
use tc_core::model::View;
use tc_core::status::{Source, Status};
use tc_term::widget;

use super::focus::{Ask, ask_of};
use super::{icons, kit, status, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};

/// Which rows the Inbox lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InboxFilter {
    #[default]
    All,
    NeedsYou,
    Finished,
}

impl InboxFilter {
    fn admits(self, kind: InboxKind) -> bool {
        match self {
            Self::All => true,
            Self::NeedsYou => kind.needs_you(),
            Self::Finished => !kind.needs_you(),
        }
    }
}

/// What a row of the Inbox is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InboxKind {
    /// An agent asks: a permission, a plan, a question.
    Ask,
    /// A command or a program failed (its exit code).
    Failed(i32),
    /// An agent's account hit its usage limit.
    Limited,
    /// An agent finished its turn.
    Done,
    /// A long command finished.
    CommandDone,
}

impl InboxKind {
    /// In the "Needs you" group (else "Finished").
    pub fn needs_you(self) -> bool {
        matches!(self, Self::Ask | Self::Failed(_) | Self::Limited)
    }
}

/// One thing that waits: its pane, what about it, since when.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InboxItem {
    pub pane: PaneId,
    pub kind: InboxKind,
    pub since: Instant,
}

/// What is done with a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    Approve,
    Reject,
    Open,
    Review,
    Seen,
}

/// A row, a group's title, the list's width, the key hints at the bottom.
const ROW_H: f32 = 58.0;
const ROW_GAP: f32 = 2.0;
const GROUP_H: f32 = 30.0;
const LIST_W: f32 = 460.0;
const FOOT_H: f32 = 40.0;

/// What a row shows (read before drawing, nothing borrowed while it draws).
struct Row {
    item: InboxItem,
    project: String,
    letters: String,
    color: Color32,
    agent: Option<AgentKind>,
    what: String,
    age: String,
}

impl App {
    /// Everything that waits, in the Inbox's order: what needs the user (questions, failures,
    /// usage limits), then what finished; the most urgent first, then the oldest. A pane at its
    /// usage limit is listed as that, whatever its status says.
    pub fn inbox_items(&self) -> Vec<InboxItem> {
        let limited = |id: &PaneId| self.panes.get(id).and_then(|p| p.rate_limit.as_ref()).is_some_and(|b| b.auto_switched_to.is_none());
        let mut out: Vec<InboxItem> = self
            .attention_queue()
            .into_iter()
            .filter(|e| !limited(&e.pane))
            .filter_map(|e| {
                let kind = match e.status {
                    Status::NeedsInput => InboxKind::Ask,
                    Status::Failed(c) => InboxKind::Failed(c),
                    Status::Done => InboxKind::Done,
                    Status::CommandDone => InboxKind::CommandDone,
                    _ => return None,
                };
                Some(InboxItem { pane: e.pane, kind, since: e.since })
            })
            .collect();
        let mut limits: Vec<InboxItem> = self
            .panes
            .values()
            .filter_map(|p| {
                let b = p.rate_limit.as_ref().filter(|b| b.auto_switched_to.is_none())?;
                Some(InboxItem { pane: p.id, kind: InboxKind::Limited, since: b.created_at })
            })
            .collect();
        limits.sort_by_key(|i| i.since);
        // After the failures: the last of what needs the user.
        let at = out.iter().position(|i| !i.kind.needs_you()).unwrap_or(out.len());
        out.splice(at..at, limits);
        out
    }

    /// Where the Inbox goes back to (Esc, ⌥⌘J again): the view it was opened from.
    pub fn inbox_back(&self) -> View {
        if crate::ade::is_management_view(self.prev_view) || self.prev_view == View::Inbox { View::Focus } else { self.prev_view }
    }

    /// The Inbox's keys are free: nothing floats over it and no text field has the keyboard.
    fn inbox_keys_free(&self, ui: &Ui) -> bool {
        self.dialog.is_none()
            && self.palette.is_none()
            && !self.updater.dialog_open
            && !self.monitor_popover
            && !egui::Popup::is_any_open(ui.ctx())
            && !ui.ctx().egui_wants_keyboard_input()
    }

    /// An ask that Approve (Enter in its terminal) and Reject (Esc) answer: not a question.
    fn inbox_approvable(&self, item: InboxItem) -> bool {
        item.kind == InboxKind::Ask && self.panes.get(&item.pane).is_some_and(|p| !matches!(ask_of(p.tracker.message.as_deref()), Ask::Question(_)))
    }

    /// The Inbox view: its bar (title, count; the filter and Mark all seen in `inbox_toolbar`),
    /// then one sheet with the list, the row chosen and the key hints.
    pub fn inbox_view(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let full = ui.max_rect();
        let items = self.inbox_items();
        let total = items.len();
        let body = self.top_bar(
            ui,
            full,
            |_, ui| {
                ui.add(egui::Label::new(RichText::new(t("Inbox")).font(style::semibold(style::TITLE)).color(p.text)).selectable(false));
                if total > 0 {
                    ui.add_space(8.0);
                    let sub = crate::i18n::count(total, t("1 thing, oldest first"), t("{} things, oldest first"));
                    ui.add(egui::Label::new(RichText::new(sub).font(egui::FontId::proportional(style::SMALL)).color(p.text_faint)).selectable(false));
                }
            },
            None,
        );
        let sheet = super::focus::sheets_area(body, self.settings.sidebar_hidden);
        #[cfg(debug_assertions)]
        crate::devtest::record("inbox", sheet);
        super::focus::paint_sheet(ui.painter(), sheet, &p, super::focus::SheetLook::Plain);
        let keys_free = self.inbox_keys_free(ui);
        if total == 0 {
            let body = tf("Questions, failures and finished turns show up here. {} jumps to the next one from anywhere.", &[kit::keys(&Action::NextAttention).unwrap_or("⌘J")]);
            kit::empty_state_at(ui, sheet, icons::TRAY, t("Nothing waits for you"), &body, None);
            if keys_free && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.actions.push(Action::SetView(self.inbox_back()));
            }
            return;
        }
        let shown: Vec<InboxItem> = items
            .iter()
            .copied()
            .filter(|i| self.inbox_filter.admits(i.kind) && self.panes.get(&i.pane).is_some_and(|x| self.project(x.project).is_some()))
            .collect();
        // The row chosen: the same pane as before, else the one now in its place.
        let mut sel = self
            .inbox_sel
            .and_then(|id| shown.iter().position(|i| i.pane == id))
            .unwrap_or_else(|| self.inbox_ix.min(shown.len().saturating_sub(1)));

        // The keyboard, before drawing (the row it chose shows this frame).
        let mut act: Option<(InboxItem, Do)> = None;
        let mut moved = false;
        let mut back = false;
        if keys_free {
            let cur = shown.get(sel).copied();
            let approvable = cur.is_some_and(|i| self.inbox_approvable(i));
            let n = shown.len();
            ui.input_mut(|inp| {
                use egui::{Key, Modifiers as M};
                if n > 0 && inp.consume_key(M::NONE, Key::ArrowDown) {
                    sel = (sel + 1).min(n - 1);
                    moved = true;
                }
                if n > 0 && inp.consume_key(M::NONE, Key::ArrowUp) {
                    sel = sel.saturating_sub(1);
                    moved = true;
                }
                if let Some(i) = cur.filter(|_| !moved) {
                    if inp.consume_key(M::NONE, Key::Enter) {
                        act = Some((i, if approvable { Do::Approve } else { Do::Open }));
                    } else if inp.consume_key(M::NONE, Key::O) {
                        act = Some((i, Do::Open));
                    } else if i.kind == InboxKind::Done && inp.consume_key(M::NONE, Key::R) {
                        act = Some((i, Do::Review));
                    } else if inp.consume_key(M::NONE, Key::Backspace) || inp.consume_key(M::NONE, Key::Delete) {
                        act = Some((i, Do::Seen));
                    }
                }
                if act.is_none() && inp.consume_key(M::NONE, Key::Escape) {
                    match cur {
                        Some(i) if approvable => act = Some((i, Do::Reject)),
                        _ => back = true,
                    }
                }
            });
        }

        let foot = Rect::from_min_max(pos2(sheet.left(), sheet.bottom() - FOOT_H), sheet.max);
        let main = Rect::from_min_max(sheet.min, pos2(sheet.right(), foot.top()));
        if shown.is_empty() {
            self.inbox_filter_empty(ui, main);
        } else {
            let with_detail = main.width() >= 640.0;
            let list_w = if with_detail { LIST_W.min((main.width() * 0.42).max(300.0)) } else { main.width() };
            let list = Rect::from_min_max(main.min, pos2(main.left() + list_w, main.bottom()));
            if with_detail {
                ui.painter().vline(list.right() + 0.5, main.y_range(), Stroke::new(1.0, p.border));
            }
            let rows = self.inbox_rows(&shown);
            if let Some((i, double)) = self.inbox_list(ui, list, &rows, sel, moved) {
                sel = i;
                if double {
                    act = Some((shown[i], Do::Open));
                }
            }
            if with_detail {
                let detail = Rect::from_min_max(pos2(list.right() + 1.0, main.top()), main.max);
                if let Some(d) = self.inbox_detail(ui, detail, shown[sel]) {
                    act = Some((shown[sel], d));
                }
            }
        }
        ui.painter().hline(sheet.x_range(), foot.top() + 0.5, Stroke::new(1.0, p.border));
        self.inbox_hints(ui, foot);

        self.inbox_sel = shown.get(sel).map(|i| i.pane);
        self.inbox_ix = sel;
        if let Some((item, what)) = act {
            self.inbox_do(item, what);
        }
        if back {
            self.actions.push(Action::SetView(self.inbox_back()));
        }
    }

    /// The right side of the Inbox's bar (laid out right to left): Mark all seen, then the
    /// filter (All, Needs you, Finished, each with its count).
    pub(super) fn inbox_toolbar(&mut self, ui: &mut Ui) {
        let items = self.inbox_items();
        // Nothing waits: nothing to filter or mark.
        if items.is_empty() {
            return;
        }
        let needs = items.iter().filter(|i| i.kind.needs_you()).count();
        let finished = items.len() - needs;
        let all = kit::button_ex(ui, kit::Kind::Ghost, Some(icons::CHECKS), t("Mark all seen"), false);
        #[cfg(debug_assertions)]
        crate::devtest::record("inbox-all-seen", all.rect);
        if all.clicked() {
            self.actions.push(Action::MarkAllSeen);
            for i in items.iter().filter(|i| i.kind == InboxKind::Limited) {
                self.actions.push(Action::DismissRateLimit(i.pane));
            }
        }
        ui.add_space(10.0);
        let labels = [
            format!("{} {}", t("All"), items.len()),
            format!("{} {needs}", t("Needs you")),
            format!("{} {finished}", t("Finished")),
        ];
        let options = [
            kit::Seg::new(&labels[0], t("Everything that waits for you")),
            kit::Seg::new(&labels[1], t("Questions, failures and usage limits")),
            kit::Seg::new(&labels[2], t("Finished turns and commands")),
        ];
        let filters = [InboxFilter::All, InboxFilter::NeedsYou, InboxFilter::Finished];
        let on = filters.iter().position(|f| *f == self.inbox_filter).unwrap_or(0);
        let r = ui.push_id("inbox-filter", |ui| kit::segmented(ui, &options, on));
        #[cfg(debug_assertions)]
        for (rect, name) in super::grid::word_segments(ui, r.response.rect, &labels).into_iter().zip(["inbox:all", "inbox:needs", "inbox:finished"]) {
            crate::devtest::record(name, rect);
        }
        if let Some(i) = r.inner {
            self.inbox_filter = filters[i];
            self.inbox_ix = 0;
            self.inbox_sel = None;
        }
    }

    /// What each row shows.
    fn inbox_rows(&mut self, shown: &[InboxItem]) -> Vec<Row> {
        let mut rows = Vec::with_capacity(shown.len());
        for item in shown {
            // A finished turn: what it changed, worked out once (in the background).
            let stats = if item.kind == InboxKind::Done { self.done_stats(item.pane).filter(|s| s.0 > 0) } else { None };
            let Some(pane) = self.panes.get(&item.pane) else { continue };
            let Some(project) = self.project(pane.project) else { continue };
            let what = match item.kind {
                InboxKind::Ask => match ask_of(pane.tracker.message.as_deref()) {
                    Ask::Permission { tool, target } if tool == "Bash" => tf("Wants to run {}", &[&target]),
                    Ask::Permission { tool, target } => tf("Wants to use {} · {}", &[&tool, &target]),
                    Ask::Plan(plan) => tf("Wants to apply its plan · {}", &[&plan]),
                    Ask::Question(q) => tf("Asks: {}", &[&q]),
                    Ask::Confirm(Some(m)) => m,
                    Ask::Confirm(None) => t("Needs input").to_string(),
                },
                InboxKind::Failed(code) => tf("{} exited with code {}", &[&failed_what(pane), &code.to_string()]),
                InboxKind::Limited => match &pane.rate_limit {
                    Some(b) => tf("{} hit its usage limit", &[&b.from_profile]),
                    None => t("Usage limit").to_string(),
                },
                InboxKind::Done => match (stats, pane.tracker.message.as_deref()) {
                    (Some(s), _) => format!("{} · {}", t("Finished"), stats_text(s)),
                    (None, Some(m)) => format!("{} · “{m}”", t("Finished")),
                    (None, None) => t("Finished").to_string(),
                },
                InboxKind::CommandDone => tf("{} finished", &[&pane.label()]),
            };
            rows.push(Row {
                item: *item,
                project: self.project_label(pane.project),
                letters: project.letters(),
                color: project.color32(),
                agent: pane.tracker.agent.or(pane.last_agent),
                what,
                age: status::short_age(item.since),
            });
        }
        rows
    }

    /// The list: a title over each group, then its rows. Returns the row clicked (and whether
    /// twice).
    fn inbox_list(&mut self, ui: &mut Ui, list: Rect, rows: &[Row], sel: usize, scroll_to: bool) -> Option<(usize, bool)> {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let needs_n = rows.iter().filter(|r| r.item.kind.needs_you()).count();
        let finished_n = rows.len() - needs_n;
        let mut clicked = None;
        let inner = list.shrink(8.0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            egui::ScrollArea::vertical().id_salt("inbox-list").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                let w = ui.available_width();
                let mut group: Option<bool> = None;
                for (i, row) in rows.iter().enumerate() {
                    let needs = row.item.kind.needs_you();
                    if group != Some(needs) {
                        if group.is_some() {
                            ui.add_space(ROW_GAP);
                        }
                        group = Some(needs);
                        let (r, _) = ui.allocate_exact_size(vec2(w, GROUP_H), Sense::hover());
                        let painter = ui.painter();
                        let (label, n) = if needs { (t("Needs you"), needs_n) } else { (t("Finished"), finished_n) };
                        let g = painter.layout_no_wrap(label.to_string(), style::medium(style::LABEL), p.text_faint);
                        let gw = g.size().x;
                        let cy = r.top() + 12.0 + 7.0;
                        painter.galley(pos2(r.left() + 12.0, cy - g.size().y / 2.0), g, p.text_faint);
                        let c = painter.layout_no_wrap(n.to_string(), style::mono(style::DATA), p.text_ghost);
                        painter.galley(pos2(r.left() + 12.0 + gw + 6.0, cy - c.size().y / 2.0), c, p.text_ghost);
                        ui.add_space(ROW_GAP);
                    } else {
                        ui.add_space(ROW_GAP);
                    }
                    let (r, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("inbox-row:{}", row.item.pane), r);
                    if scroll_to && i == sel {
                        ui.scroll_to_rect(r, None);
                    }
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.double_clicked() {
                        clicked = Some((i, true));
                    } else if resp.clicked() {
                        clicked = Some((i, false));
                    }
                    paint_row(ui.painter(), r, row, i == sel, resp.hovered(), &p, dark);
                }
            });
        });
        clicked
    }

    /// The row chosen, at the right: its project, pane and status; what it asks or what
    /// happened; what can be done; its terminal's last lines. Returns what was clicked.
    fn inbox_detail(&mut self, ui: &mut Ui, area: Rect, item: InboxItem) -> Option<Do> {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let stats = if item.kind == InboxKind::Done { self.done_stats(item.pane).filter(|s| s.0 > 0) } else { None };
        let approvable = self.inbox_approvable(item);
        let pane = self.panes.get(&item.pane)?;
        let project = self.project(pane.project)?;
        let name = self.project_label(pane.project);
        let (letters, color) = (project.letters(), project.color32());
        let tab_title = project.tab_of(item.pane).and_then(|i| project.tabs.get(i)).and_then(|tab| tab.title.clone());
        let agent = pane.tracker.agent.or(pane.last_agent);
        let who = agent.map_or_else(|| pane.label(), |a| a.label().to_string());
        let task = pane.task.clone().or(tab_title).unwrap_or_else(|| pane.label());
        let alive = pane.alive();
        let says = says(pane, item, &who, stats, &task);

        let inner = Rect::from_min_max(area.min + vec2(24.0, 20.0), area.max - vec2(24.0, 20.0));
        if inner.width() < 120.0 || inner.height() < 120.0 {
            return None;
        }
        let painter = ui.painter_at(area);
        let mut out = None;

        // Head (20 high): the project, ›, the pane's agent and task; its status at the right.
        let cy = inner.top() + 10.0;
        let pill = match item.kind {
            InboxKind::Limited => {
                let text = t("Rate limited");
                let w = kit::pill_width(&painter, Some(icons::HOURGLASS_MEDIUM), text);
                kit::paint_pill(&painter, pos2(inner.right() - w, cy), Some(icons::HOURGLASS_MEDIUM), text, p.rate_limited, style::tint(p.rate_limited, dark))
            }
            kind => {
                let s = kind_status(kind);
                let text = match s {
                    Status::Failed(_) => t("Failed").to_string(),
                    Status::CommandDone => t("Finished").to_string(),
                    s => status::text(s),
                };
                let w = kit::state_pill_width(&painter, s, &text);
                kit::paint_state_pill(&painter, pos2(inner.right() - w, cy), s, &text, &p, dark)
            }
        };
        let mut x = inner.left();
        kit::paint_project_tile(&painter, Rect::from_min_size(pos2(x, cy - 10.0), vec2(20.0, 20.0)), &letters, color, dark);
        x += 20.0 + 8.0;
        let room = |x: f32| (pill.left() - 12.0 - x).max(0.0);
        let g = one_line(&painter, &name, style::semibold(style::HEADING), p.text, room(x));
        let gw = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
        x += gw + 8.0;
        if room(x) > 60.0 {
            painter.text(pos2(x + 5.0, cy), Align2::CENTER_CENTER, icons::CARET_RIGHT, style::icon_font(10.0), p.text_faint);
            x += 10.0 + 8.0;
            paint_avatar(&painter, Rect::from_min_size(pos2(x, cy - 8.0), vec2(16.0, 16.0)), agent, &p);
            x += 16.0 + 8.0;
            let g = one_line(&painter, &task, style::semibold(style::HEADING), p.text_muted, room(x));
            painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text_muted);
        }
        let mut y = inner.top() + 20.0 + 20.0;

        // What it asks, or what happened.
        let g = wrapped(&painter, &says.title, style::semibold(style::TITLE), p.text, inner.width(), 2);
        let th = g.size().y;
        painter.galley(pos2(inner.left(), y), g, p.text);
        y += th.max(24.0) + 10.0;
        if let Some((dollar, text, right)) = &says.command {
            let b = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 40.0));
            painter.rect(b, CornerRadius::same(8), p.inset, Stroke::new(1.0, p.border), StrokeKind::Inside);
            let bcy = b.center().y;
            let mut bx = b.left() + 14.0;
            let mut end = b.right() - 14.0;
            if let Some(r) = right {
                let g = painter.layout_no_wrap(r.clone(), style::mono(style::DATA), p.text_faint);
                end -= g.size().x;
                painter.galley(pos2(end, bcy - g.size().y / 2.0), g, p.text_faint);
                end -= 10.0;
            }
            if *dollar {
                let g = painter.layout_no_wrap("$".into(), style::mono(style::MONO), p.text_faint);
                let w = g.size().x;
                painter.galley(pos2(bx, bcy - g.size().y / 2.0), g, p.text_faint);
                bx += w + 10.0;
            }
            let g = one_line(&painter, text, style::mono(style::MONO), p.text, (end - bx).max(10.0));
            painter.galley(pos2(bx, bcy - g.size().y / 2.0), g, p.text);
            y += 40.0 + 10.0;
        }
        if let Some(words) = &says.words {
            let g = wrapped(&painter, words, egui::FontId::proportional(style::BODY), p.text_muted, inner.width(), 3);
            let h = g.size().y;
            painter.galley(pos2(inner.left(), y), g, p.text_muted);
            y += h + 10.0;
        }
        let g = wrapped(&painter, &says.note, egui::FontId::proportional(style::SMALL), p.text_faint, inner.width(), 2);
        let h = g.size().y;
        painter.galley(pos2(inner.left(), y), g, p.text_faint);
        y += h + 20.0;

        // What can be done: answer it (or open it), then Mark seen at the right.
        let actions = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 28.0));
        let left_end = ui
            .scope_builder(egui::UiBuilder::new().max_rect(actions).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let mut button = |ui: &mut Ui, kind: kit::Kind, icon: Option<&str>, text: &str, keys: &str, what: Do, name: &str| {
                    let r = kit::button_keys(ui, kind, icon, text, Some(keys), false);
                    #[cfg(debug_assertions)]
                    crate::devtest::record(name, r.rect);
                    let _ = name;
                    if r.clicked() {
                        out = Some(what);
                    }
                };
                if approvable {
                    button(ui, kit::Kind::Primary, None, t("Approve"), "↵", Do::Approve, "inbox-approve");
                    button(ui, kit::Kind::Secondary, None, t("Reject"), "esc", Do::Reject, "inbox-reject");
                    button(ui, kit::Kind::Ghost, Some(icons::ARROW_SQUARE_OUT), t("Open pane"), "O", Do::Open, "inbox-open");
                } else {
                    let label = if item.kind == InboxKind::Ask { t("Answer in terminal") } else { t("Open pane") };
                    button(ui, kit::Kind::Primary, Some(icons::ARROW_SQUARE_OUT), label, "↵", Do::Open, "inbox-open");
                    if item.kind == InboxKind::Done {
                        button(ui, kit::Kind::Secondary, Some(icons::GIT_DIFF), t("Review"), "R", Do::Review, &format!("att-review:{}", item.pane));
                    }
                }
                ui.min_rect().right()
            })
            .inner;
        let seen_rect = Rect::from_min_max(pos2((left_end + 8.0).min(actions.right()), actions.top()), actions.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(seen_rect).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
            ui.set_clip_rect(seen_rect.intersect(ui.clip_rect()));
            let r = kit::button_keys(ui, kit::Kind::Ghost, None, t("Mark seen"), Some("⌫"), false);
            #[cfg(debug_assertions)]
            crate::devtest::record("inbox-seen", r.rect);
            if r.clicked() {
                out = Some(Do::Seen);
            }
        });
        y += 28.0 + 20.0;

        // Its terminal's last lines, live.
        let label = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 14.0));
        let painter = ui.painter_at(area);
        painter.text(label.left_center(), Align2::LEFT_CENTER, t("Last lines"), style::medium(style::LABEL), p.text_faint);
        if alive {
            painter.text(label.right_center(), Align2::RIGHT_CENTER, t("live"), style::mono(style::DATA), p.info);
        }
        y += 14.0 + 8.0;
        let tail = Rect::from_min_max(pos2(inner.left(), y), inner.max);
        if tail.height() < 40.0 {
            return out;
        }
        painter.rect(tail, CornerRadius::same(style::RADIUS_CARD), p.inset, Stroke::new(1.0, p.border), StrokeKind::Inside);
        #[cfg(debug_assertions)]
        crate::devtest::record("inbox-tail", tail);
        // The terminal itself, in the data font (12 at the default 13), read only: its newest
        // rows, cut at the right when wider than the sheet.
        let base = self.term_style(ui);
        let size = base.font_size * style::MONO / 13.0;
        // On the inset sheet's colour, not the terminal's own background.
        let scheme = tc_term::scheme::Scheme { background: p.inset, ..base.scheme.clone() };
        let term_style = widget::TermStyle { font_size: size, cell: widget::cell_size(ui.ctx(), size), scheme: &scheme, ..base };
        if let Some(pane) = self.panes.get_mut(&item.pane) {
            let rect = tail.shrink2(vec2(4.0, 1.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                let alive = pane.alive();
                let _ = widget::show_readable(ui, rect, &mut pane.view, &pane.shared, &term_style, false, alive, 1.0, false);
            });
            self.shown_now.insert(item.pane, crate::pane::Shown::Preview);
        }
        // A click on it opens the pane.
        let r = ui.interact(tail, egui::Id::new("inbox-tail"), Sense::click());
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t("Open pane")).clicked() {
            out = Some(Do::Open);
        }
        out
    }

    /// The filter keeps nothing (though something waits): say so, and offer them all.
    fn inbox_filter_empty(&mut self, ui: &mut Ui, area: Rect) {
        let (title, body) = match self.inbox_filter {
            InboxFilter::Finished => (t("Nothing has finished"), t("Finished turns and long commands show up here.")),
            _ => (t("Nothing needs you"), t("Questions, failures and usage limits show up here.")),
        };
        if kit::empty_state_at(ui, area, icons::CHECK_CIRCLE, title, body, Some((None, t("Show all"), None))).is_some_and(|r| r.clicked()) {
            self.inbox_filter = InboxFilter::All;
        }
    }

    /// The keys at the bottom, and where ⌘J goes at the right.
    fn inbox_hints(&self, ui: &Ui, foot: Rect) {
        let p = style::pal(ui);
        let painter = ui.painter_at(foot);
        let cy = foot.center().y;
        let jump = tf("{} jumps to the next one from anywhere", &[kit::keys(&Action::NextAttention).unwrap_or("⌘J")]);
        let g = painter.layout_no_wrap(jump, egui::FontId::proportional(style::SMALL), p.text_faint);
        let right_x = foot.right() - 16.0 - g.size().x;
        let mut limit = foot.right() - 16.0;
        if right_x > foot.left() + 16.0 + 200.0 {
            painter.galley(pos2(right_x, cy - g.size().y / 2.0), g, p.text_faint);
            limit = right_x - 24.0;
        }
        let mut x = foot.left() + 16.0;
        for (keys, word) in [("↑↓", t("choose")), ("↵", t("approve or open")), ("O", t("open pane")), ("R", t("review")), ("⌫", t("mark seen"))] {
            let g = painter.layout_no_wrap(word.to_string(), egui::FontId::proportional(style::SMALL), p.text_faint);
            let w = kit::kbd_width(&painter, keys) + 6.0 + g.size().x;
            if x + w > limit {
                break;
            }
            let kw = kit::paint_kbd_text(&painter, pos2(x, cy), keys, &p);
            painter.galley(pos2(x + kw + 6.0, cy - g.size().y / 2.0), g, p.text_faint);
            x += w + 14.0;
        }
    }

    /// Does what was chosen for a row.
    fn inbox_do(&mut self, item: InboxItem, what: Do) {
        match what {
            // As the pane's own notice does: Enter, or Esc, in its terminal.
            Do::Approve => {
                if let Some(pane) = self.panes.get_mut(&item.pane) {
                    pane.entered(Instant::now());
                    pane.input(b"\r".to_vec());
                }
            }
            Do::Reject => {
                if let Some(pane) = self.panes.get_mut(&item.pane) {
                    pane.input(vec![0x1b]);
                }
                self.actions.push(Action::MarkSeen(item.pane));
            }
            Do::Open => self.actions.push(Action::FocusPane(item.pane)),
            Do::Review => {
                self.actions.push(Action::FocusPane(item.pane));
                self.actions.push(Action::ReviewAgent);
            }
            Do::Seen => {
                if matches!(item.kind, InboxKind::Limited | InboxKind::Ask | InboxKind::Done) {
                    self.actions.push(Action::DismissNotice(item.pane));
                } else {
                    self.actions.push(Action::MarkSeen(item.pane));
                }
            }
        }
    }
}

/// What the detail says: its title, the command (a `$`, the text, what runs it), the agent's
/// own words, and a line on when and where.
struct Says {
    title: String,
    command: Option<(bool, String, Option<String>)>,
    words: Option<String>,
    note: String,
}

fn says(pane: &crate::pane::Pane, item: InboxItem, who: &str, stats: Option<(usize, u32, u32)>, task: &str) -> Says {
    let mut command = None;
    let mut words = None;
    let title = match item.kind {
        InboxKind::Ask => match ask_of(pane.tracker.message.as_deref()) {
            Ask::Permission { tool, target } => {
                let bash = tool == "Bash";
                let title = if bash { tf("{} wants to run a command", &[who]) } else { tf("{} wants to use {}", &[who, &tool]) };
                command = Some((bash, target, Some(tool)));
                title
            }
            Ask::Plan(plan) => {
                command = Some((false, plan, Some(t("Plan").to_string())));
                tf("{} wants to apply its plan", &[who])
            }
            Ask::Question(q) => {
                words = Some(q);
                tf("{} has a question", &[who])
            }
            Ask::Confirm(said) => {
                words = said;
                tf("{} needs your confirmation", &[who])
            }
        },
        InboxKind::Failed(code) => tf("{} exited with code {}", &[&failed_what(pane), &code.to_string()]),
        InboxKind::Limited => {
            let b = pane.rate_limit.as_ref();
            if let Some(reason) = b.map(|b| b.reason.trim()).filter(|r| !r.is_empty()) {
                command = Some((false, reason.to_string(), None));
            }
            tf("{} hit its usage limit", &[b.map_or(who, |b| b.from_profile.as_str())])
        }
        InboxKind::Done => {
            words = pane.tracker.message.clone().or_else(|| pane.last_words.clone()).map(|m| format!("“{m}”"));
            tf("{} finished its turn", &[who])
        }
        InboxKind::CommandDone => tf("{} finished", &[&pane.label()]),
    };
    // When, through what, which pane, which account.
    let ago = status::age(item.since);
    let mut first = match item.kind {
        InboxKind::Ask => tf("Asked {} ago", &[&ago]),
        InboxKind::Failed(_) => tf("Failed {} ago", &[&ago]),
        InboxKind::Limited => tf("Stopped {} ago", &[&ago]),
        InboxKind::Done | InboxKind::CommandDone => tf("Finished {} ago", &[&ago]),
    };
    let via = match pane.tracker.source {
        _ if item.kind == InboxKind::Limited => None,
        Source::Hook(a) => Some(tf("through {}'s hooks", &[a.label()])),
        Source::Osc => Some(t("through a terminal notification").to_string()),
        Source::Bell => Some(t("through the terminal bell").to_string()),
        Source::Shell => Some(t("through shell integration").to_string()),
        _ => None,
    };
    if let Some(via) = via {
        first = format!("{first} {via}");
    }
    let mut parts = vec![first];
    if let Some(s) = stats {
        parts.push(stats_text(s));
    }
    if let Some(b) = pane.rate_limit.as_ref().filter(|_| item.kind == InboxKind::Limited) {
        let left = b.reset_at.saturating_duration_since(Instant::now());
        let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
        if secs > 0 {
            parts.push(tf("Resets in {}", &[&crate::profiles::format_countdown(secs)]));
        }
    }
    parts.push(tf("pane “{}”", &[task]));
    if let Some(profile) = &pane.active_profile {
        parts.push(tf("account {}", &[profile]));
    }
    Says { title, command, words, note: parts.join(" · ") }
}

/// What failed: what the program said, or its title, or the pane's label.
fn failed_what(pane: &crate::pane::Pane) -> String {
    pane.tracker.message.clone().or_else(|| pane.program_title.clone()).unwrap_or_else(|| pane.label())
}

/// "2 files · +41 −2" (no "−0").
fn stats_text((files, added, removed): (usize, u32, u32)) -> String {
    let mut s = format!("{} · +{added}", crate::i18n::count(files, t("1 file"), t("{} files")));
    if removed > 0 {
        s.push_str(&format!(" −{removed}"));
    }
    s
}

/// The status a row's glyph and pill show.
fn kind_status(kind: InboxKind) -> Status {
    match kind {
        InboxKind::Ask => Status::NeedsInput,
        InboxKind::Failed(c) => Status::Failed(c),
        InboxKind::Done | InboxKind::Limited => Status::Done,
        InboxKind::CommandDone => Status::CommandDone,
    }
}

/// One line of text cut with "…" at `max_w`.
fn one_line(painter: &egui::Painter, text: &str, font: egui::FontId, color: Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping { max_width: max_w.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    painter.layout_job(job)
}

/// Text wrapped at `max_w`, at most `rows` lines (the last cut with "…").
fn wrapped(painter: &egui::Painter, text: &str, font: egui::FontId, color: Color32, max_w: f32, rows: usize) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping { max_width: max_w.max(1.0), max_rows: rows, break_anywhere: false, overflow_character: Some('…') };
    painter.layout_job(job)
}

/// The agent avatar (S, 16): its mark on a quiet tile; a shell's terminal glyph.
fn paint_avatar(painter: &egui::Painter, rect: Rect, agent: Option<AgentKind>, p: &style::Palette) {
    painter.rect_filled(rect, CornerRadius::same(4), p.subtle);
    let mark = Rect::from_center_size(rect.center(), egui::Vec2::splat((rect.width() * 0.68).round()));
    if let Some(a) = agent
        && super::agent_logo::paint(painter, mark, a, p.text, false)
    {
        return;
    }
    painter.text(rect.center(), Align2::CENTER_CENTER, icons::TERMINAL, style::icon_font((rect.width() * 0.72).round()), p.text_muted);
}

/// One row: its status glyph, the project's tile, the project and how long ago; the agent and
/// what it asks or did. The one chosen is raised (a surface, a hairline, a soft shadow).
fn paint_row(painter: &egui::Painter, r: Rect, row: &Row, sel: bool, hovered: bool, p: &style::Palette, dark: bool) {
    let radius = CornerRadius::same(9);
    if sel {
        let shadow = egui::epaint::Shadow { offset: [0, 1], blur: 2, spread: 0, color: Color32::from_black_alpha(if dark { 64 } else { 20 }) };
        painter.add(shadow.as_shape(r, radius));
        painter.rect(r, radius, p.surface, Stroke::new(1.0, p.border), StrokeKind::Inside);
    } else if hovered {
        painter.rect_filled(r, radius, p.hover);
    }
    let x0 = r.left() + 12.0;
    let top = r.top() + 10.0;
    let glyph = pos2(x0 + 7.0, top + 7.0);
    match row.item.kind {
        InboxKind::Limited => {
            painter.text(glyph, Align2::CENTER_CENTER, icons::HOURGLASS_MEDIUM, style::icon_fill(14.0), p.rate_limited);
        }
        kind => status::paint_glyph(painter, glyph, kind_status(kind), p, 14.0),
    }
    kit::paint_project_tile(painter, Rect::from_min_size(pos2(x0 + 14.0 + 12.0, top), vec2(20.0, 20.0)), &row.letters, row.color, dark);
    let tx = x0 + 14.0 + 12.0 + 20.0 + 12.0;
    let right = r.right() - 12.0;
    // First line: the project, how long ago at the right.
    let l1 = top + 9.0;
    let age = painter.layout_no_wrap(row.age.clone(), style::mono(style::DATA), p.text_faint);
    let age_x = right - age.size().x;
    painter.galley(pos2(age_x, l1 - age.size().y / 2.0), age, p.text_faint);
    let font = if sel { style::medium(style::BODY) } else { egui::FontId::proportional(style::BODY) };
    let g = one_line(painter, &row.project, font, p.text, (age_x - 8.0 - tx).max(10.0));
    painter.galley(pos2(tx, l1 - g.size().y / 2.0), g, p.text);
    // Second line: the agent, what it asks or did.
    let l2 = top + 18.0 + 4.0 + 8.0;
    paint_avatar(painter, Rect::from_min_size(pos2(tx, l2 - 8.0), vec2(16.0, 16.0)), row.agent, p);
    let wx = tx + 16.0 + 6.0;
    let g = one_line(painter, &row.what, egui::FontId::proportional(style::SMALL), p.text_muted, (right - wx).max(10.0));
    painter.galley(pos2(wx, l2 - g.size().y / 2.0), g, p.text_muted);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_splits_what_needs_you_from_what_finished() {
        let all = [InboxKind::Ask, InboxKind::Failed(1), InboxKind::Limited, InboxKind::Done, InboxKind::CommandDone];
        let needs: Vec<_> = all.iter().filter(|k| InboxFilter::NeedsYou.admits(**k)).collect();
        let finished: Vec<_> = all.iter().filter(|k| InboxFilter::Finished.admits(**k)).collect();
        assert_eq!(needs, [&InboxKind::Ask, &InboxKind::Failed(1), &InboxKind::Limited]);
        assert_eq!(finished, [&InboxKind::Done, &InboxKind::CommandDone]);
        assert!(all.iter().all(|k| InboxFilter::All.admits(*k)));
    }

    #[test]
    fn stats_leave_out_nothing_removed() {
        assert_eq!(stats_text((2, 41, 0)), "2 files · +41");
        assert_eq!(stats_text((3, 102, 2)), "3 files · +102 −2");
    }
}
