//! The composer at the foot of Changes (`crate::ship`): the message, where it came from, one
//! button that says the whole of what it will do and where the branch goes; while it runs, its
//! steps; then the pull request, or what stopped it and the way on.

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Modifiers, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::ProjectId;
use tc_core::model::ShipAction;

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};
use crate::ship::{self, Failure, Plan, RunReq, StepKind, StepState};

/// Room around the composer's parts, and between them.
const PAD_X: f32 = 14.0;
const PAD_TOP: f32 = 12.0;
const PAD_BOTTOM: f32 = 14.0;
const GAP: f32 = 10.0;
const GO_H: f32 = 32.0;
const LINE_H: f32 = 16.0;
const STEP_H: f32 = 34.0;
const ACTS_H: f32 = 28.0;
const BRANCH_H: f32 = 28.0;
/// The message box: two lines at least, at most this.
const MSG_MIN_H: f32 = 60.0;
const MSG_MAX_H: f32 = 140.0;
/// The message box while the agent writes it (three bars).
const MSG_WRITING_H: f32 = 74.0;

/// What a click in the composer asks for (done after drawing, when nothing is borrowed).
#[derive(Clone)]
enum Ask {
    Go,
    Pick(ShipAction),
    Draft,
    NewBranch,
    CancelBranch,
    Terminal(Vec<String>),
    TerminalFrom { from: usize, prefix: Option<String> },
    ToggleCommands,
    Write,
    Undo,
    Done,
    OpenPr(String),
    Copy(String),
    Fix(String),
}

/// A notice of the composer: blocked, nothing to commit, no remote, a failure and its way on.
struct Note {
    tone: kit::Tone,
    glyph: Option<&'static str>,
    title: String,
    body: Option<String>,
    /// Its buttons, left to right (the last is the primary), with what each asks.
    buttons: Vec<(String, kit::Kind, Option<&'static str>, Ask)>,
}

/// The parts of the composer, top to bottom.
enum Part {
    /// While it runs: the commit's subject.
    Title,
    Steps,
    Message(f32),
    Source,
    Branch,
    Notice(Note),
    /// "Reading the branch…", a spinner before it.
    Loading,
    /// A quiet line (keep working, the pull request form is open), maybe an icon before it.
    Text(std::sync::Arc<egui::Galley>, Option<&'static str>),
    Output(std::sync::Arc<egui::Galley>),
    PrCard,
    Acts,
    /// Where the branch goes, and the ship button.
    Go,
    Commands(Vec<String>),
}

impl App {
    /// The id of a project's message field (Review's keys stay out of it).
    pub fn ship_message_id(project: ProjectId) -> egui::Id {
        egui::Id::new(("ship-message", project))
    }

    /// The composer at the bottom of `area` (the foot of Changes, its full width), growing up to
    /// `area`'s top at most (it scrolls inside beyond). Returns its top.
    pub(super) fn ship_box(&mut self, ui: &mut Ui, area: Rect) -> f32 {
        let Some(r) = self.review.as_ref() else { return area.bottom() };
        let project = r.project;
        let p = style::pal(ui);
        let out = self.ship.get(&project).map(|s| s.out.clone()).unwrap_or_default();
        let paths = ship::checked(&r.files, &r.uncommitted, &out);
        let n_files = r.files.iter().filter(|f| r.uncommitted.contains(&f.path) && !out.contains(&f.path)).count();
        let review_loading = r.loading;
        let root = r.root.clone();
        let to_tree = r.to.clone();
        let base_hint = r.worktree.as_ref().map(|(_, b)| b.clone());
        self.ship_want_facts(project);
        self.ship_prefill(project);
        let agent = self.ship_agent_pane(project).map(|(_, a)| a);
        let prefs = self.project(project).map(|x| x.ship).unwrap_or_default();
        let Some(s) = self.ship.get(&project) else { return area.bottom() };
        let web_info = s.facts.as_ref().and_then(|f| f.remote.as_ref()).and_then(|r| r.web.as_ref());
        let is_gitlab = web_info.is_some_and(|w| w.forge == tc_git::ship::Forge::GitLab);
        let has_forge = web_info.is_some();
        let has_cli = ship::cli_for(web_info, &s.gh, &s.glab).is_some();
        let plan: Option<Plan> = s.facts.as_ref().map(|f| {
            ship::plan(&ship::Inputs {
                facts: f,
                files: n_files,
                prefs,
                new_branch: s.new_branch.as_deref(),
                base: base_hint.as_deref(),
                pr: s.pr.as_ref(),
                gh: has_cli,
            })
        });
        let run = s.run.clone();
        let message = s.message.clone();
        let auto = s.auto;
        let written_by = s.written_by;
        let writing = s.writing;
        let write_error = s.write_error.clone();
        let has_undo = s.undo.is_some();
        let new_branch = s.new_branch.clone();
        let show_commands = s.show_commands;
        let pr = s.pr.clone();
        let facts = s.facts.clone();
        let loading = s.facts.is_none() || review_loading;

        let w = area.width() - PAD_X * 2.0;
        let mut asks: Vec<Ask> = Vec::new();
        let quiet = |ui: &Ui, text: &str, color: Color32, indent: f32| ui.painter().layout(text.to_string(), egui::FontId::proportional(style::SMALL), color, w - indent);

        // ----- what the composer holds, measured first (it grows up from the bottom) ----------
        let mut parts: Vec<Part> = Vec::new();
        let mut req_for_go: Option<RunReq> = None;
        if let Some(run) = &run {
            let run_gitlab = run.req.web.as_ref().is_some_and(|w| w.forge == tc_git::ship::Forge::GitLab);
            let running = run.failed().is_none() && !run.finished;
            if running && run.steps.iter().any(|s| s.kind == StepKind::Commit) && !run.title.is_empty() {
                parts.push(Part::Title);
            }
            parts.push(Part::Steps);
            match (run.failed(), run.finished) {
                (Some((i, f)), _) => {
                    let close = (t("Close").to_string(), kit::Kind::Secondary, None, Ask::Done);
                    match f {
                        Failure::Rejected(_) => {
                            let remote = run.req.remote.clone().unwrap_or_else(|| "origin".into());
                            let pull = format!("git pull --rebase {} {}", ship::quote(&remote), ship::quote(&run.req.branch));
                            parts.push(Part::Notice(Note {
                                tone: kit::Tone::Failed,
                                glyph: None,
                                title: tf("{} has commits you don’t have", &[&format!("{remote}/{}", run.req.branch)]),
                                body: Some(t("Your commit is safe here. Pull them in, then push.").to_string()),
                                buttons: vec![close, (t("Pull & push in terminal").to_string(), kit::Kind::Primary, Some("↵"), Ask::TerminalFrom { from: i, prefix: Some(pull) })],
                            }));
                        }
                        Failure::Credentials(_) => {
                            parts.push(Part::Notice(Note {
                                tone: kit::Tone::Failed,
                                glyph: Some(icons::KEY),
                                title: t("git wants a password or an SSH passphrase").to_string(),
                                body: Some(t("It can only ask in a terminal.").to_string()),
                                buttons: vec![close, (t("Push in terminal").to_string(), kit::Kind::Primary, Some("↵"), Ask::TerminalFrom { from: i, prefix: None })],
                            }));
                        }
                        Failure::SignedOut(_) => {
                            let (title, body) = if run_gitlab {
                                (t("GitLab CLI isn’t signed in"), t("The branch is pushed: finish the merge request on GitLab."))
                            } else {
                                (t("GitHub CLI isn’t signed in"), t("The branch is pushed: finish the pull request on GitHub."))
                            };
                            parts.push(Part::Notice(Note { tone: kit::Tone::Neutral, glyph: Some(icons::INFO), title: title.to_string(), body: Some(body.to_string()), buttons: Vec::new() }));
                            parts.push(Part::Acts);
                        }
                        Failure::Output(out) => {
                            let out = out.trim();
                            if !out.is_empty() {
                                let g = ui.painter().layout(out.to_string(), style::mono(style::DATA), p.err, w - 24.0);
                                parts.push(Part::Output(g));
                            }
                            parts.push(Part::Acts);
                        }
                    }
                }
                (None, true) => {
                    if run.pr.is_some() {
                        parts.push(Part::PrCard);
                    } else if run.form.is_some() {
                        let msg = if run_gitlab {
                            t("The merge request form is open in the Browser panel, filled in.")
                        } else {
                            t("The pull request form is open in the Browser panel, filled in.")
                        };
                        parts.push(Part::Text(quiet(ui, msg, p.text_muted, 20.0), Some(icons::GLOBE_SIMPLE)));
                    }
                    parts.push(Part::Acts);
                }
                (None, false) => {
                    parts.push(Part::Text(quiet(ui, t("Keep working: it runs in the background and says when it’s done."), p.text_faint, 0.0), None));
                }
            }
        } else if loading {
            parts.push(Part::Loading);
        } else if let Some(plan) = &plan {
            let commits = plan.has(&StepKind::Commit) || plan.pr_step().is_some() || plan.blocked.is_some() || n_files > 0;
            if commits {
                let can_write = agent.is_some_and(ship::can_write);
                let h = if writing.is_some() { MSG_WRITING_H } else { message_height(ui, &message, w - 22.0 - if can_write { 34.0 } else { 0.0 }) };
                parts.push(Part::Message(h));
                if writing.is_some() || written_by.is_some() || write_error.is_some() || (auto && agent.is_some()) {
                    parts.push(Part::Source);
                }
            }
            if new_branch.is_some() {
                parts.push(Part::Branch);
            }
            let note = |tone, glyph, title: String, body: Option<String>| Part::Notice(Note { tone, glyph, title, body, buttons: Vec::new() });
            if let Some(b) = &plan.blocked {
                let (title, body) = split_note(b);
                parts.push(note(kit::Tone::Input, Some(icons::WARNING), title, body));
            } else if plan.idle() {
                let branch = facts.as_ref().and_then(|f| f.branch.clone()).unwrap_or_default();
                let body = match (&pr, facts.as_ref().and_then(|f| f.remote.as_ref())) {
                    (Some(pr), _) if matches!(pr.state, tc_git::ship::PrState::Open | tc_git::ship::PrState::Draft) => Some(if is_gitlab {
                        tf("{} is pushed; MR !{} is up to date.", &[&branch, &pr.number.to_string()])
                    } else {
                        tf("{} is pushed; PR #{} is up to date.", &[&branch, &pr.number.to_string()])
                    }),
                    (_, Some(remote)) if facts.as_ref().is_some_and(|f| f.upstream.is_some()) => Some(tf("{} is the same as {}.", &[&branch, &remote.name])),
                    _ => None,
                };
                parts.push(note(kit::Tone::Done, None, t("Nothing to commit").to_string(), body));
            } else if facts.as_ref().is_some_and(|f| f.remote.is_none()) && plan.has(&StepKind::Commit) && prefs.action != ShipAction::Commit {
                parts.push(note(kit::Tone::Neutral, Some(icons::INFO), t("No remote yet").to_string(), Some(t("Add one (git remote add origin …) and the button pushes too.").to_string())));
            } else if plan.pr_step() == Some(false) && has_forge {
                let (title, body) = if is_gitlab {
                    (t("GitLab CLI isn’t installed"), t("After the push, the merge request form opens in the Browser panel, filled in."))
                } else {
                    (t("GitHub CLI isn’t installed"), t("After the push, the pull request form opens in the Browser panel, filled in."))
                };
                parts.push(note(kit::Tone::Neutral, Some(icons::INFO), title.to_string(), Some(body.to_string())));
            }
            if !plan.idle() || plan.blocked.is_some() {
                parts.push(Part::Go);
                if show_commands && plan.blocked.is_none() {
                    let req = self.ship_request(project, plan, &root, &paths, &message);
                    let cmds = req.preview_commands(0);
                    if !cmds.is_empty() {
                        parts.push(Part::Commands(cmds));
                    }
                    req_for_go = Some(req);
                }
            }
        }
        let height = |part: &Part| -> f32 {
            match part {
                Part::Title | Part::Source | Part::Loading => LINE_H,
                Part::Steps => run.as_ref().map_or(0.0, |r| r.steps.len() as f32 * STEP_H + 8.0),
                Part::Message(h) => *h,
                Part::Branch => BRANCH_H,
                Part::Notice(n) => if n.body.is_some() { kit::NOTICE_H } else { kit::NOTICE_H_SHORT },
                Part::Text(g, _) => g.size().y.max(LINE_H),
                Part::Output(g) => g.size().y.min(8.0 * 15.0) + 24.0,
                Part::PrCard => 56.0,
                Part::Acts => ACTS_H,
                Part::Go => GO_H,
                Part::Commands(c) => c.len() as f32 * 15.0 + 14.0,
            }
        };
        let total: f32 = parts.iter().map(height).sum::<f32>() + GAP * (parts.len().saturating_sub(1)) as f32;
        let top = (area.bottom() - total - PAD_TOP - PAD_BOTTOM).max(area.top());
        let rect = Rect::from_min_max(pos2(area.left(), top), area.max);
        ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, p.border));
        ui.data_mut(|d| d.insert_temp(egui::Id::new("ship-box-rect"), rect));
        #[cfg(debug_assertions)]
        crate::devtest::record("ship-box", rect);

        // ----- drawn top-down (scrolling when the panel is short) ----------------------------
        let plan_label = plan.as_ref().map(ship::label).unwrap_or_default();
        let inner = Rect::from_min_max(pos2(rect.left() + PAD_X - 4.0, rect.top() + 1.0), pos2(rect.right() - PAD_X + 4.0, rect.bottom()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt(("ship-box-scroll", project))
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .show(ui, |ui| {
                    egui::Frame::NONE.inner_margin(egui::Margin { left: 4, right: 4, top: (PAD_TOP - 1.0) as i8, bottom: PAD_BOTTOM as i8 }).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = GAP;
                        let painter = ui.painter().clone();
                        for part in &parts {
                            let h = height(part);
                            if let Part::Notice(n) = part {
                                let buttons: Vec<kit::NoticeButton> = n.buttons.iter().map(|(label, kind, keys, _)| kit::NoticeButton { label, kind: *kind, keys: *keys }).collect();
                                let (clicked, nr) = kit::notice_ex(ui, w, n.tone, n.glyph, &n.title, n.body.as_deref(), &buttons);
                                #[cfg(debug_assertions)]
                                crate::devtest::record("ship-notice", nr);
                                let _ = nr;
                                if let Some(i) = clicked {
                                    asks.push(n.buttons[i].3.clone());
                                }
                                // ↵ takes the way on (the primary) when nothing has the keyboard.
                                if let Some((_, kit::Kind::Primary, Some("↵"), ask)) = n.buttons.last()
                                    && ui.memory(|m| m.focused().is_none())
                                    && !self.overlay_open()
                                    && !self.updater.dialog_open
                                    && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
                                {
                                    asks.push(ask.clone());
                                }
                                continue;
                            }
                            let (row, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                            match part {
                                Part::Notice(_) => {}
                                Part::Title => {
                                    let title = run.as_ref().map(|r| r.title.clone()).unwrap_or_default();
                                    painter.text(pos2(row.left() + 7.0, row.center().y), Align2::CENTER_CENTER, icons::GIT_COMMIT, style::icon_font(14.0), p.text_faint);
                                    let mut job = egui::text::LayoutJob::simple_singleline(title, egui::FontId::proportional(style::SMALL), p.text_muted);
                                    job.wrap = egui::text::TextWrapping::truncate_at_width(w - 22.0);
                                    let g = painter.layout_job(job);
                                    painter.galley(pos2(row.left() + 22.0, row.center().y - g.size().y / 2.0), g, p.text_muted);
                                }
                                Part::Steps => {
                                    let Some(run) = &run else { continue };
                                    let run_gitlab = run.req.web.as_ref().is_some_and(|w| w.forge == tc_git::ship::Forge::GitLab);
                                    painter.rect(row, CornerRadius::same(style::RADIUS_CARD), p.inset, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                                    let mut sy = row.top() + 4.0;
                                    for step in &run.steps {
                                        let sr = Rect::from_min_size(pos2(row.left() + 4.0, sy), vec2(row.width() - 8.0, STEP_H));
                                        sy += STEP_H;
                                        paint_step(&painter, sr, step, run_gitlab, &p);
                                        #[cfg(debug_assertions)]
                                        crate::devtest::record(format!("ship-step:{}", step_name(&step.kind)), sr);
                                    }
                                }
                                Part::Message(_) => {
                                    let id = App::ship_message_id(project);
                                    let focused = ui.memory(|m| m.has_focus(id));
                                    let can_write = agent.filter(|a| ship::can_write(*a));
                                    if focused && writing.is_none() {
                                        painter.rect_stroke(row.expand(3.0), CornerRadius::same(12), Stroke::new(3.0, style::soft(p.accent, 0.25)), egui::StrokeKind::Inside);
                                    }
                                    let stroke = if focused || writing.is_some() { p.accent } else { p.border_strong };
                                    painter.rect(row, CornerRadius::same(9), p.inset, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
                                    let btn_w = if can_write.is_some() { 26.0 + 8.0 } else { 0.0 };
                                    if writing.is_some() {
                                        // The agent writes it: three bars that breathe.
                                        let t_now = ui.input(|i| i.time) as f32;
                                        let a = 0.75 + 0.25 * (t_now * 3.0).sin();
                                        let x0 = row.left() + 12.0;
                                        let tw = row.width() - 12.0 - 10.0 - btn_w;
                                        let bars = [(row.top() + 12.0, 12.0, 0.55, p.subtle), (row.top() + 32.0, 10.0, 0.8, p.hover), (row.top() + 50.0, 10.0, 0.45, p.hover)];
                                        for (y, h, f, c) in bars {
                                            let r = Rect::from_min_size(pos2(x0, y), vec2(tw * f, h));
                                            painter.rect_filled(r, CornerRadius::same((h / 2.0) as u8), c.gamma_multiply(a));
                                        }
                                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
                                    } else {
                                        let text_rect = Rect::from_min_max(pos2(row.left() + 12.0, row.top() + 10.0), pos2(row.right() - 10.0 - btn_w, row.bottom() - 10.0));
                                        let mut text = message.clone();
                                        let (title_c, body_c) = (p.text, p.text_muted);
                                        let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                                            let job = message_job(buf.as_str(), wrap, title_c, body_c);
                                            ui.fonts_mut(|f| f.layout_job(job))
                                        };
                                        let resp = place(ui, text_rect, |ui| {
                                                egui::ScrollArea::vertical().id_salt(("ship-message-scroll", project)).max_height(text_rect.height()).auto_shrink([false, true]).show(ui, |ui| {
                                                    ui.add(
                                                        egui::TextEdit::multiline(&mut text)
                                                            .id(id)
                                                            .frame(egui::Frame::NONE)
                                                            .margin(egui::Margin::ZERO)
                                                            .desired_width(text_rect.width())
                                                            .desired_rows(2)
                                                            .hint_text(egui::RichText::new(t("Message (first line: what it does)")).color(p.text_faint))
                                                            .layouter(&mut layouter),
                                                    )
                                                })
                                                .inner
                                            });
                                        if std::mem::take(&mut self.ship_focus) {
                                            resp.request_focus();
                                        }
                                        if resp.changed() {
                                            let s = self.ship_mut(project);
                                            s.message = text;
                                            s.auto = false;
                                            s.written_by = None;
                                            s.undo = None;
                                        }
                                        if resp.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
                                            asks.push(Ask::Go);
                                        }
                                    }
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record("ship-message", row);
                                    // ✦: the agent that did the work writes it from the diff.
                                    if let Some(a) = can_write {
                                        let br = Rect::from_min_size(pos2(row.right() - 10.0 - 26.0, row.top() + 10.0), vec2(26.0, 26.0));
                                        let resp = ui.interact(br, egui::Id::new(("ship-write", project)), Sense::click());
                                        let fill = if resp.hovered() && writing.is_none() { style::soft(p.accent, 0.26) } else { p.accent_tint };
                                        painter.rect_filled(br, CornerRadius::same(style::RADIUS), fill);
                                        if writing.is_some() {
                                            super::status::paint_spinner(&painter, br.center(), 12.0, p.info);
                                        } else {
                                            painter.text(br.center(), Align2::CENTER_CENTER, icons::SPARKLE, style::icon_font(14.0), p.accent_text);
                                        }
                                        #[cfg(debug_assertions)]
                                        crate::devtest::record("ship-write", br);
                                        if writing.is_none() && resp.hovered() {
                                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                        if resp.on_hover_text(tf("Write it with {} (from the diff, with your plan)", &[a.label()])).clicked() && writing.is_none() {
                                            asks.push(Ask::Write);
                                        }
                                    }
                                }
                                Part::Source => {
                                    let (text, link, color) = if let Some(a) = writing {
                                        (tf("{} is writing it from the diff and your plan…", &[a.label()]), None, p.text_muted)
                                    } else if let Some(e) = &write_error {
                                        (e.clone(), None, p.err)
                                    } else if let Some(a) = written_by {
                                        (tf("Written by {} from the diff", &[a.label()]), has_undo.then_some(t("Undo")), p.text_faint)
                                    } else {
                                        (tf("From {}’s task and its last words", &[agent.map(|a| a.label()).unwrap_or("")]), None, p.text_faint)
                                    };
                                    let mut lx = row.left();
                                    if let Some(a) = agent.filter(|_| write_error.is_none() || writing.is_some()).filter(|a| super::agent_logo::has(*a)) {
                                        kit::paint_agent_badge(&painter, Rect::from_min_size(pos2(lx, row.center().y - 8.0), vec2(16.0, 16.0)), a, &p);
                                        lx += 16.0 + 6.0;
                                    }
                                    let font = egui::FontId::proportional(style::SMALL);
                                    let link_w = link.map_or(0.0, |l| painter.layout_no_wrap(l.to_string(), font.clone(), p.accent_text).size().x + 8.0);
                                    let mut job = egui::text::LayoutJob::simple_singleline(text, font.clone(), color);
                                    job.wrap = egui::text::TextWrapping::truncate_at_width(row.right() - lx - link_w);
                                    let g = painter.layout_job(job);
                                    painter.galley(pos2(lx, row.center().y - g.size().y / 2.0), g, color);
                                    if let Some(l) = link {
                                        let g = painter.layout_no_wrap(l.to_string(), font, p.accent_text);
                                        let lr = Rect::from_min_size(pos2(row.right() - g.size().x, row.center().y - g.size().y / 2.0), g.size());
                                        painter.galley(lr.min, g, p.accent_text);
                                        if ui.interact(lr.expand(3.0), egui::Id::new(("ship-undo", project)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                            asks.push(Ask::Undo);
                                        }
                                    }
                                }
                                Part::Branch => {
                                    painter.text(pos2(row.left() + 6.0, row.center().y), Align2::CENTER_CENTER, icons::GIT_BRANCH, style::icon_font(12.0), p.text_faint);
                                    let fr = Rect::from_min_max(pos2(row.left() + 20.0, row.top() + 1.0), pos2(row.right() - 30.0, row.bottom() - 1.0));
                                    let mut name = new_branch.clone().unwrap_or_default();
                                    let resp = place(ui, fr, |ui| kit::field(ui, egui::Id::new(("ship-branch", project)), &mut name, None, t("New branch"), fr.width(), true));
                                    if resp.changed() {
                                        self.ship_mut(project).new_branch = Some(name);
                                    }
                                    let xr = Rect::from_center_size(pos2(row.right() - 12.0, row.center().y), vec2(24.0, 24.0));
                                    let x = place(ui, xr, |ui| kit::icon_button_ex(ui, icons::X, t("Commit on this branch"), false, 24.0));
                                    if x.clicked() {
                                        asks.push(Ask::CancelBranch);
                                    }
                                }
                                Part::Loading => {
                                    super::status::paint_spinner(&painter, pos2(row.left() + 7.0, row.center().y), 12.0, p.info);
                                    painter.text(pos2(row.left() + 22.0, row.center().y), Align2::LEFT_CENTER, t("Reading the branch…"), egui::FontId::proportional(style::SMALL), p.text_faint);
                                }
                                Part::Text(g, icon) => {
                                    let mut x = row.left();
                                    if let Some(i) = icon {
                                        painter.text(pos2(x + 7.0, row.top() + 8.0), Align2::CENTER_CENTER, *i, style::icon_font(13.0), p.text_faint);
                                        x += 20.0;
                                    }
                                    painter.galley(pos2(x, row.top()), g.clone(), p.text_faint);
                                }
                                Part::Output(g) => {
                                    painter.rect(row, CornerRadius::same(8), p.inset, Stroke::new(1.0, style::tint(p.err, ui.visuals().dark_mode)), egui::StrokeKind::Inside);
                                    let clip = row.shrink(12.0);
                                    painter.with_clip_rect(clip).galley(clip.min, g.clone(), p.err);
                                }
                                Part::PrCard => {
                                    let Some(pr) = run.as_ref().and_then(|r| r.pr.clone()) else { continue };
                                    let run_gitlab = run.as_ref().and_then(|r| r.req.web.as_ref()).is_some_and(|w| w.forge == tc_git::ship::Forge::GitLab);
                                    let prefix = if run_gitlab { "!" } else { "#" };
                                    let dark = ui.visuals().dark_mode;
                                    painter.rect_filled(row, CornerRadius::same(style::RADIUS_CARD), style::tint(p.ok, dark));
                                    let ic = Rect::from_min_size(pos2(row.left() + 12.0, row.center().y - 16.0), vec2(32.0, 32.0));
                                    painter.rect_filled(ic, CornerRadius::same(8), style::tint(p.ok, dark));
                                    painter.text(ic.center(), Align2::CENTER_CENTER, icons::GIT_PULL_REQUEST, style::icon_font(16.0), p.ok);
                                    let tx = ic.right() + 12.0;
                                    let title = format!("{prefix}{} {}", pr.number, run.as_ref().map(|r| r.title.as_str()).unwrap_or(""));
                                    let mut job = egui::text::LayoutJob::simple_singleline(title, style::medium(style::BODY), p.text);
                                    job.wrap = egui::text::TextWrapping::truncate_at_width(row.right() - 12.0 - tx);
                                    let g = painter.layout_job(job);
                                    painter.galley(pos2(tx, row.center().y - g.size().y - 1.0), g, p.text);
                                    let short = pr.url.trim_start_matches("https://").to_string();
                                    let mut job = egui::text::LayoutJob::simple_singleline(short, style::mono(style::DATA), p.text_faint);
                                    job.wrap = egui::text::TextWrapping::truncate_at_width(row.right() - 12.0 - tx);
                                    let g = painter.layout_job(job);
                                    painter.galley(pos2(tx, row.center().y + 1.0), g, p.text_faint);
                                    #[cfg(debug_assertions)]
                                    crate::devtest::record("ship-pr", row);
                                }
                                Part::Acts => {
                                    let Some(run) = &run else { continue };
                                    // The buttons first, then drawn to fit the row.
                                    let mut btns: Vec<(Act, Ask)> = Vec::new();
                                    let close;
                                    match run.failed() {
                                        Some((i, f)) => {
                                            close = (t("Close"), "ship-act-close");
                                            match f {
                                                Failure::SignedOut(_) => {
                                                    let run_gitlab = run.req.web.as_ref().is_some_and(|w| w.forge == tc_git::ship::Forge::GitLab);
                                                    if let Some(web) = &run.req.web {
                                                        let (title, body) = tc_git::ship::split_message(&run.req.message);
                                                        let url = web.new_pr_url(run.req.base.as_deref().unwrap_or("main"), &run.req.branch, &title, &body);
                                                        let finish_label = if run_gitlab { t("Finish on GitLab") } else { t("Finish on GitHub") };
                                                        btns.push((Act::new(kit::Kind::Primary, Some(icons::GLOBE_SIMPLE), finish_label, "ship-act-web"), Ask::OpenPr(url)));
                                                    }
                                                    let login_cmd = match &run.req.web {
                                                        Some(w) if run_gitlab && w.host != "gitlab.com" => format!("glab auth login --hostname {}", ship::quote(&w.host)),
                                                        Some(w) if !run_gitlab && w.host != "github.com" => format!("gh auth login --hostname {}", ship::quote(&w.host)),
                                                        _ if run_gitlab => "glab auth login".to_string(),
                                                        _ => "gh auth login".to_string(),
                                                    };
                                                    let login_lbl = if run_gitlab { "glab auth login" } else { "gh auth login" };
                                                    btns.push((Act::new(kit::Kind::Ghost, Some(icons::TERMINAL_WINDOW), login_lbl, "ship-act-login"), Ask::Terminal(vec![login_cmd])));
                                                }
                                                Failure::Output(out) => {
                                                    let commit = run.steps.get(i).is_some_and(|s| s.kind == StepKind::Commit);
                                                    if commit && let Some(a) = agent {
                                                        btns.push((Act::new(kit::Kind::Primary, Some(icons::SPARKLE), &tf("Ask {} to fix", &[a.label()]), "ship-act-fix"), Ask::Fix(out.clone())));
                                                    }
                                                    btns.push((Act::new(kit::Kind::Secondary, Some(icons::TERMINAL_WINDOW), t("Run in terminal"), "ship-act-terminal"), Ask::TerminalFrom { from: i, prefix: None }));
                                                }
                                                // Told by their notice, with its buttons.
                                                Failure::Rejected(_) | Failure::Credentials(_) => {}
                                            }
                                        }
                                        None => {
                                            close = (t("Done"), "ship-act-done");
                                            if let Some(pr) = &run.pr {
                                                let mut open = Act::new(kit::Kind::Primary, Some(icons::GLOBE_SIMPLE), t("Open"), "ship-act-open");
                                                open.keys = Some("↵");
                                                btns.push((open, Ask::OpenPr(pr.url.clone())));
                                                btns.push((Act::new(kit::Kind::Secondary, Some(icons::COPY), t("Copy link"), "ship-act-copy"), Ask::Copy(pr.url.clone())));
                                            } else if let Some(url) = &run.form {
                                                btns.push((Act::new(kit::Kind::Secondary, Some(icons::GLOBE_SIMPLE), t("Open again"), "ship-act-form"), Ask::OpenPr(url.clone())));
                                            }
                                        }
                                    }
                                    let (acts, mut tasks): (Vec<Act>, Vec<Ask>) = btns.into_iter().unzip();
                                    match act_row(ui, row, &acts, close) {
                                        Some(n) if n < tasks.len() => asks.push(tasks.swap_remove(n)),
                                        Some(_) => asks.push(Ask::Done),
                                        None => {}
                                    }
                                    // ↵ opens the pull request once it is made (when nothing has the keyboard).
                                    if run.failed().is_none()
                                        && let Some(pr) = &run.pr
                                        && ui.memory(|m| m.focused().is_none())
                                        && !self.overlay_open()
                                        && !self.updater.dialog_open
                                        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
                                    {
                                        asks.push(Ask::OpenPr(pr.url.clone()));
                                    }
                                }
                                Part::Go => {
                                    let Some(plan) = &plan else { continue };
                                    let enabled = plan.blocked.is_none() && !plan.idle() && !(plan.has(&StepKind::Commit) && message.trim().is_empty()) && writing.is_none() && new_branch.as_deref().is_none_or(|b| tc_git::worktree::valid_branch(b.trim()));
                                    let icon = if plan.pr_step() == Some(false) {
                                        icons::GLOBE_SIMPLE
                                    } else if plan.pr_step().is_some() {
                                        icons::GIT_PULL_REQUEST
                                    } else if plan.has(&StepKind::Push) {
                                        icons::ARROW_UP
                                    } else {
                                        icons::GIT_COMMIT
                                    };
                                    let label = if plan.blocked.is_some() { t("Commit").to_string() } else { plan_label.clone() };
                                    let (main, caret, button_left) = ship_button(ui, row, project, icon, &label, enabled, writing.is_none(), &p);
                                    #[cfg(debug_assertions)]
                                    {
                                        crate::devtest::record("ship-go", main.rect);
                                        if let Some(c) = &caret {
                                            crate::devtest::record("ship-more", c.rect);
                                        }
                                    }
                                    let tip = if !enabled {
                                        if writing.is_some() {
                                            t("Writing the commit message…")
                                        } else if new_branch.as_deref().is_some_and(|b| !tc_git::worktree::valid_branch(b.trim())) {
                                            t("Enter a valid branch name")
                                        } else if plan.has(&StepKind::Commit) && message.trim().is_empty() {
                                            t("Write a message first")
                                        } else {
                                            ""
                                        }
                                    } else {
                                        ""
                                    };
                                    let main = if tip.is_empty() { main } else { main.on_hover_text(tip) };
                                    if main.clicked() && enabled {
                                        asks.push(Ask::Go);
                                    }
                                    // Where the branch goes, at the left of the button.
                                    let to = to_line(plan, prefs.action, is_gitlab);
                                    let font = style::mono(style::DATA);
                                    let link = plan.can_branch.then_some(if is_gitlab { t("Branch & MR…") } else { t("Branch & PR…") });
                                    let link_g = link.map(|l| painter.layout_no_wrap(l.to_string(), egui::FontId::proportional(style::SMALL), p.text_faint));
                                    let room = button_left - 12.0 - (row.left() + 18.0);
                                    let link_w = link_g.as_ref().map_or(0.0, |g| g.size().x + 10.0);
                                    let show_link = link_w > 0.0 && room - link_w > 90.0;
                                    painter.text(pos2(row.left() + 6.0, row.center().y), Align2::CENTER_CENTER, icons::GIT_BRANCH, style::icon_font(12.0), p.text_faint);
                                    let mut job = egui::text::LayoutJob::simple_singleline(to, font, p.text_faint);
                                    job.wrap = egui::text::TextWrapping::truncate_at_width((room - if show_link { link_w } else { 0.0 }).max(10.0));
                                    let g = painter.layout_job(job);
                                    let gw = g.size().x;
                                    painter.galley(pos2(row.left() + 18.0, row.center().y - g.size().y / 2.0), g, p.text_faint);
                                    if show_link && let Some(g) = link_g {
                                        let lr = Rect::from_min_size(pos2(row.left() + 18.0 + gw + 10.0, row.center().y - g.size().y / 2.0), g.size());
                                        let resp = ui.interact(lr.expand(3.0), egui::Id::new(("ship-branch-link", project)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                                        painter.galley(lr.min, g, if resp.hovered() { p.text } else { p.accent_text });
                                        #[cfg(debug_assertions)]
                                        crate::devtest::record("ship-branch-link", lr);
                                        let branch_tip = if is_gitlab {
                                            t("Make a branch for a merge request, then commit there")
                                        } else {
                                            t("Make a branch for a pull request, then commit there")
                                        };
                                        if resp.on_hover_text(branch_tip).clicked() {
                                            asks.push(Ask::NewBranch);
                                        }
                                    }
                                    let Some(caret) = caret else { continue };
                                    egui::Popup::menu(&caret).align(egui::RectAlign::TOP_END).width(280.0).show(|ui| {
                                        kit::menu_body(ui);
                                        ui.set_min_width(268.0);
                                        kit::menu_header(ui, t("The button does"));
                                        for (a, icon, label) in [
                                            (ShipAction::Commit, icons::GIT_COMMIT, t("Commit")),
                                            (ShipAction::CommitPush, icons::ARROW_UP, t("Commit & Push")),
                                            (ShipAction::PullRequest, icons::GIT_PULL_REQUEST, if is_gitlab { t("Commit, Push & Create MR") } else { t("Commit, Push & Create PR") }),
                                        ] {
                                            let on = prefs.action == a;
                                            let item = kit::menu_item(ui, Some(if on { icons::CHECK } else { icon }), label, None, false);
                                            #[cfg(debug_assertions)]
                                            crate::devtest::record(format!("ship-pick:{a:?}"), item.rect);
                                            if item.clicked() {
                                                asks.push(Ask::Pick(a));
                                                ui.close();
                                            }
                                        }
                                        kit::menu_sep(ui);
                                        let draft_label = if is_gitlab { t("Create MR as draft") } else { t("Create PR as draft") };
                                        let item = kit::menu_item(ui, Some(if prefs.draft { icons::CHECK } else { icons::FILE_DASHED }), draft_label, None, false);
                                        if item.clicked() {
                                            asks.push(Ask::Draft);
                                            ui.close();
                                        }
                                        let web = facts.as_ref().and_then(|f| f.remote.as_ref()).is_some_and(|r| r.web.is_some());
                                        if web && new_branch.is_none() {
                                            let item = kit::menu_item(ui, Some(icons::GIT_BRANCH), t("Commit to a new branch…"), None, false);
                                            #[cfg(debug_assertions)]
                                            crate::devtest::record("ship-new-branch", item.rect);
                                            if item.clicked() {
                                                asks.push(Ask::NewBranch);
                                                ui.close();
                                            }
                                        }
                                        kit::menu_sep(ui);
                                        let has_cmds = !self.ship_request(project, plan, &root, &paths, &message).preview_commands(0).is_empty();
                                        ui.add_enabled_ui(plan.blocked.is_none() && !plan.idle() && has_cmds, |ui| {
                                            let item = kit::menu_item(ui, Some(icons::TERMINAL_WINDOW), t("Run in terminal instead"), None, false);
                                            if item.clicked() {
                                                let req = self.ship_request(project, plan, &root, &paths, &message);
                                                asks.push(Ask::Terminal(req.commands(0)));
                                                ui.close();
                                            }
                                        });
                                        let item = kit::menu_item(ui, Some(icons::CODE), if show_commands { t("Hide the commands") } else { t("Show the commands") }, None, false);
                                        if item.clicked() {
                                            asks.push(Ask::ToggleCommands);
                                            ui.close();
                                        }
                                    });
                                }
                                Part::Commands(cmds) => {
                                    painter.rect(row, CornerRadius::same(8), p.inset, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                                    let clip = row.shrink2(vec2(10.0, 7.0));
                                    let pc = painter.with_clip_rect(clip);
                                    for (i, c) in cmds.iter().enumerate() {
                                        let cy = clip.top() + i as f32 * 15.0 + 7.5;
                                        pc.text(pos2(clip.left(), cy), Align2::LEFT_CENTER, "$", style::mono(style::DATA), p.text_ghost);
                                        pc.text(pos2(clip.left() + 12.0, cy), Align2::LEFT_CENTER, c, style::mono(style::DATA), p.text_muted);
                                    }
                                    let copy = Rect::from_center_size(pos2(row.right() - 15.0, row.top() + 14.0), vec2(22.0, 22.0));
                                    let resp = place(ui, copy, |ui| kit::icon_button_ex(ui, icons::COPY, t("Copy"), false, 22.0));
                                    if resp.clicked() {
                                        let materialized = req_for_go.as_ref().map(|r| r.commands(0));
                                        asks.push(Ask::Copy(ship::chain(materialized.as_deref().unwrap_or(cmds))));
                                    }
                                }
                            }
                        }
                    });
                });
        });

        // ----- what was asked -------------------------------------------------------------
        for ask in asks {
            match ask {
                Ask::Go => {
                    let Some(plan) = &plan else { continue };
                    if plan.blocked.is_some()
                        || plan.idle()
                        || (plan.has(&StepKind::Commit) && message.trim().is_empty())
                        || writing.is_some()
                        || new_branch.as_deref().is_some_and(|b| !tc_git::worktree::valid_branch(b.trim()))
                    {
                        continue;
                    }
                    let req = req_for_go.take().unwrap_or_else(|| self.ship_request(project, plan, &root, &paths, &message));
                    self.ship_run(project, req);
                }
                Ask::Pick(a) => {
                    if let Some(x) = self.project_mut(project) {
                        x.ship.action = a;
                    }
                    self.mark_dirty();
                }
                Ask::Draft => {
                    if let Some(x) = self.project_mut(project) {
                        x.ship.draft = !x.ship.draft;
                    }
                    self.mark_dirty();
                }
                Ask::NewBranch => {
                    let (title, _) = tc_git::ship::split_message(&message);
                    let name = tc_git::ship::branch_from_title(&title);
                    if let Some(x) = self.project_mut(project) {
                        x.ship.action = ShipAction::PullRequest;
                    }
                    self.ship_mut(project).new_branch = Some(name);
                }
                Ask::CancelBranch => self.ship_mut(project).new_branch = None,
                Ask::Terminal(cmds) => {
                    let ctx = ui.ctx().clone();
                    self.ship_mut(project).run = None;
                    self.ship_in_terminal(&ctx, project, cmds);
                }
                Ask::TerminalFrom { from, prefix } => {
                    let ctx = ui.ctx().clone();
                    let mut cmds = Vec::new();
                    if let Some(first) = prefix {
                        cmds.push(first);
                    }
                    if let Some(r) = self.ship_mut(project).run.take() {
                        cmds.extend(r.req.commands(from));
                    }
                    self.ship_in_terminal(&ctx, project, cmds);
                }
                Ask::ToggleCommands => {
                    let s = self.ship_mut(project);
                    s.show_commands = !s.show_commands;
                }
                Ask::Write => {
                    if let (Some(a), Some(to)) = (agent, to_tree.clone()) {
                        self.ship_write(project, a, root.clone(), to, paths.clone());
                    }
                }
                Ask::Undo => {
                    let s = self.ship_mut(project);
                    if let Some(before) = s.undo.take() {
                        s.message = before;
                        s.written_by = None;
                    }
                }
                Ask::Done => {
                    self.ship_mut(project).run = None;
                }
                Ask::OpenPr(url) => self.actions.push(Action::OpenWeb(url)),
                Ask::Copy(text) => {
                    ui.ctx().copy_text(text);
                    self.toast(t("Copied.").to_string());
                }
                Ask::Fix(out) => self.ship_ask_fix(project, &out),
            }
        }
        top
    }

    /// What a press of the button asks for, as it stands now.
    pub fn ship_request(&self, project: ProjectId, plan: &Plan, root: &std::path::Path, paths: &[String], message: &str) -> RunReq {
        let s = self.ship.get(&project);
        let facts = s.and_then(|s| s.facts.clone()).unwrap_or_default();
        let prefs = self.project(project).map(|x| x.ship).unwrap_or_default();
        let new = plan.steps.iter().any(|k| matches!(k, StepKind::Branch(_)));
        RunReq {
            project,
            root: s.and_then(|s| s.root.clone()).unwrap_or_else(|| root.to_path_buf()),
            steps: plan.steps.clone(),
            paths: paths.to_vec(),
            message: message.trim().to_string(),
            remote: plan.remote.clone(),
            branch: plan.branch.clone(),
            base: plan.base.clone(),
            set_upstream: new || facts.upstream.is_none(),
            draft: prefs.draft,
            web: facts.remote.and_then(|r| r.web),
            gh: s.and_then(|s| s.gh.clone()),
            glab: s.and_then(|s| s.glab.clone()),
            path: self.lsp.path_env.clone(),
        }
    }
}

/// A note "What it is: what to do." as a notice's title and body (the body starting with a
/// capital); one without a colon is all title.
fn split_note(text: &str) -> (String, Option<String>) {
    match text.split_once(": ") {
        Some((title, body)) if !body.trim().is_empty() => {
            let mut chars = body.trim().chars();
            let body = chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default();
            (title.trim().to_string(), Some(body))
        }
        _ => (text.trim().trim_end_matches('.').to_string(), None),
    }
}

/// Where the branch goes: `main → origin/main · updates PR #12`, `x → origin/x · PR into main`,
/// `main · commit only`.
fn to_line(plan: &Plan, action: ShipAction, gitlab: bool) -> String {
    match (&plan.remote, action) {
        (Some(remote), a) if a != ShipAction::Commit => {
            let to = format!("{} → {remote}/{}", plan.branch, plan.branch);
            if let Some(n) = plan.updates {
                let upd = if gitlab { tf("updates MR !{}", &[&n.to_string()]) } else { tf("updates PR #{}", &[&n.to_string()]) };
                format!("{to} · {upd}")
            } else if plan.pr_step().is_some() {
                let base = plan.base.as_deref().unwrap_or("main");
                let into = if gitlab { tf("MR into {}", &[base]) } else { tf("PR into {}", &[base]) };
                format!("{to} · {into}")
            } else {
                to
            }
        }
        _ => format!("{} · {}", plan.branch, t("commit only")),
    }
}

/// The message as the box shows it: its first line in Body Strong, the rest in Small, quieter;
/// the blank line under the first one only a little gap.
fn message_job(s: &str, wrap: f32, title: Color32, body: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for (i, line) in s.split_inclusive('\n').enumerate() {
        let font = match i {
            0 => style::medium(style::BODY),
            1 if line == "\n" => egui::FontId::proportional(5.0),
            _ => egui::FontId::proportional(style::SMALL),
        };
        job.append(line, 0.0, egui::TextFormat::simple(font, if i == 0 { title } else { body }));
    }
    if s.is_empty() {
        job.append("", 0.0, egui::TextFormat::simple(style::medium(style::BODY), title));
    }
    job.wrap.max_width = wrap;
    job
}

/// The message box's height for `text` at `width`: two lines at least.
fn message_height(ui: &Ui, text: &str, width: f32) -> f32 {
    let job = message_job(if text.is_empty() { " " } else { text }, width, Color32::PLACEHOLDER, Color32::PLACEHOLDER);
    let g = ui.painter().layout_job(job);
    (g.size().y + 22.0).clamp(MSG_MIN_H, MSG_MAX_H)
}

#[cfg(debug_assertions)]
fn step_name(k: &StepKind) -> &'static str {
    match k {
        StepKind::Branch(_) => "branch",
        StepKind::Commit => "commit",
        StepKind::Push => "push",
        StepKind::PullRequest { .. } => "pr",
    }
}

/// One step's row (34 high): its state, its icon, its name, what it made, how long it took.
fn paint_step(painter: &egui::Painter, r: Rect, step: &ship::Step, gitlab: bool, p: &style::Palette) {
    let cy = r.center().y;
    let gx = r.left() + 12.0 + 9.0;
    let failed = matches!(step.state, StepState::Failed(_));
    if failed {
        painter.rect_filled(r, CornerRadius::same(style::RADIUS_ROW), style::tint(p.err, painter.ctx().global_style().visuals.dark_mode));
    }
    match &step.state {
        StepState::Done => {
            painter.text(pos2(gx, cy), Align2::CENTER_CENTER, icons::CHECK_CIRCLE, style::icon_fill(16.0), p.ok);
        }
        StepState::Running => super::status::paint_spinner(painter, pos2(gx, cy), 14.0, p.info),
        StepState::Failed(_) => {
            painter.text(pos2(gx, cy), Align2::CENTER_CENTER, icons::X_CIRCLE, style::icon_fill(16.0), p.err);
        }
        StepState::Todo => {
            painter.circle_stroke(pos2(gx, cy), 5.25, Stroke::new(1.5, p.border_heavy));
        }
    }
    let icon = match &step.kind {
        StepKind::Branch(_) => icons::GIT_BRANCH,
        StepKind::Commit => icons::GIT_COMMIT,
        StepKind::Push => icons::ARROW_UP,
        StepKind::PullRequest { .. } => icons::GIT_PULL_REQUEST,
    };
    let ix = r.left() + 12.0 + 18.0 + 10.0;
    painter.text(pos2(ix + 7.0, cy), Align2::CENTER_CENTER, icon, style::icon_font(14.0), p.text_faint);
    let name = match &step.kind {
        StepKind::Branch(_) => t("Branch"),
        StepKind::Commit => t("Commit"),
        StepKind::Push => t("Push"),
        StepKind::PullRequest { .. } => if gitlab { t("Merge request") } else { t("Pull request") },
    };
    let strong = matches!(step.state, StepState::Running | StepState::Failed(_));
    let font = if strong { style::medium(style::BODY) } else { egui::FontId::proportional(style::BODY) };
    let name_c = if step.state == StepState::Todo { p.text_faint } else { p.text };
    let g = painter.layout_no_wrap(name.to_string(), font, name_c);
    let nx = ix + 14.0 + 10.0;
    let nw = g.size().x;
    painter.galley(pos2(nx, cy - g.size().y / 2.0), g, name_c);
    let took = step.took.filter(|_| matches!(step.state, StepState::Done)).map(|d| format!("{:.1} s", d.as_secs_f32()));
    let mono = style::mono(style::DATA);
    let took_w = took.as_ref().map_or(0.0, |s| painter.layout_no_wrap(s.clone(), mono.clone(), p.text_faint).size().x + 10.0);
    if let Some(s) = &took {
        painter.text(pos2(r.right() - 12.0, cy), Align2::RIGHT_CENTER, s, mono.clone(), p.text_faint);
    }
    let detail = match (&step.state, &step.kind) {
        (StepState::Failed(f), _) => match f {
            Failure::Rejected(_) => t("rejected").to_string(),
            Failure::Credentials(_) => t("needs your key").to_string(),
            Failure::SignedOut(_) => if gitlab { t("glab isn’t signed in").to_string() } else { t("gh isn’t signed in").to_string() },
            Failure::Output(out) if out.to_lowercase().contains("hook") => t("stopped by a hook").to_string(),
            Failure::Output(_) => t("stopped").to_string(),
        },
        (StepState::Todo, StepKind::Branch(name)) => name.clone(),
        _ => step.detail.clone(),
    };
    if !detail.is_empty() {
        let mut job = egui::text::LayoutJob::simple_singleline(detail, mono, p.text_faint);
        job.wrap = egui::text::TextWrapping::truncate_at_width((r.right() - 12.0 - took_w - (nx + nw + 10.0)).max(10.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(nx + nw + 10.0, cy - g.size().y / 2.0), g, p.text_faint);
    }
}

/// The ship button (32 high, radius 8, the accent) at the right of `row`: its icon, the words of
/// what it does and ⌘↵, then past a divider ▾ for its menu. While the agent writes the message:
/// the icon and words only, faded, and no menu. Returns the main part, the menu part and where
/// the button starts.
#[allow(clippy::too_many_arguments)]
fn ship_button(ui: &mut Ui, row: Rect, project: ProjectId, icon: &str, label: &str, enabled: bool, with_menu: bool, p: &style::Palette) -> (egui::Response, Option<egui::Response>, f32) {
    let painter = ui.painter().clone();
    let on = p.on_accent;
    let icon_g = painter.layout_no_wrap(icon.to_string(), style::icon_font(14.0), on);
    let keys = if cfg!(target_os = "macos") { "⌘↵" } else { "Ctrl+↵" };
    let keys_g = with_menu.then(|| painter.layout_no_wrap(keys.to_string(), style::mono(style::DATA), on));
    let pad = if with_menu { 12.0 } else { 14.0 };
    let caret_w = if with_menu { 30.0 } else { 0.0 };
    let fixed = pad * 2.0 + icon_g.size().x + 7.0 + keys_g.as_ref().map_or(0.0, |g| g.size().x + 7.0) + caret_w;
    let max_label = (row.width() * 0.72 - fixed).max(40.0);
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), style::medium(style::BODY), on);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_label);
    let text_g = painter.layout_job(job);
    let width = fixed + text_g.size().x;
    let rect = Rect::from_min_max(pos2(row.right() - width, row.top()), row.max);
    let main_r = Rect::from_min_max(rect.min, pos2(rect.right() - caret_w, rect.bottom()));
    let main = ui.interact(main_r, egui::Id::new(("ship-go", project)), if enabled { Sense::click() } else { Sense::hover() });
    let caret = with_menu.then(|| ui.interact(Rect::from_min_max(pos2(main_r.right(), rect.top()), rect.max), egui::Id::new(("ship-more", project)), Sense::click()));
    let fill = |hover: bool, live: bool| {
        let c = if hover && live { p.accent_hover } else { p.accent };
        if live { c } else { c.gamma_multiply(0.45) }
    };
    let r = 8;
    if let Some(c) = &caret {
        painter.rect_filled(main_r, CornerRadius { nw: r, sw: r, ne: 0, se: 0 }, fill(main.hovered(), enabled));
        painter.rect_filled(c.rect, CornerRadius { nw: 0, sw: 0, ne: r, se: r }, fill(c.hovered(), true));
        painter.vline(main_r.right(), (rect.center().y - 9.0)..=(rect.center().y + 9.0), Stroke::new(1.0, style::soft(Color32::WHITE, 0.25)));
        painter.text(c.rect.center(), Align2::CENTER_CENTER, icons::CARET_DOWN, style::icon_font(12.0), on);
    } else {
        painter.rect_filled(main_r, CornerRadius::same(r), fill(false, enabled));
    }
    let ink = if enabled { on } else { on.gamma_multiply(0.75) };
    let mut x = main_r.left() + pad;
    let cy = main_r.center().y;
    let iw = icon_g.size().x;
    painter.galley(pos2(x, cy - icon_g.size().y / 2.0), icon_g, ink);
    x += iw + 7.0;
    let tw = text_g.size().x;
    painter.galley(pos2(x, cy - text_g.size().y / 2.0), text_g, ink);
    x += tw + 7.0;
    if let Some(g) = keys_g {
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, style::soft(Color32::WHITE, if enabled { 0.65 } else { 0.5 }));
    }
    let main = main.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default });
    (main, caret.map(|c| c.on_hover_text(t("What the button does"))), rect.left())
}

/// A button of the composer's last row.
struct Act {
    kind: kit::Kind,
    icon: Option<&'static str>,
    label: String,
    keys: Option<&'static str>,
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    name: &'static str,
}

impl Act {
    fn new(kind: kit::Kind, icon: Option<&'static str>, label: &str, name: &'static str) -> Act {
        Act { kind, icon, label: label.to_string(), keys: None, name }
    }
}

/// The composer's last row: its buttons from the left and the closing word at the right, made
/// to fit a narrow column (the buttons after the first keep only their icon, then the closing
/// word becomes ×). Returns the index of the button clicked, `acts.len()` for the closing word.
/// Each is recorded for the end-to-end tests.
fn act_row(ui: &mut Ui, row: Rect, acts: &[Act], close: (&str, &'static str)) -> Option<usize> {
    const GAP: f32 = 8.0;
    let full = |a: &Act| kit::button_width(ui, a.icon, &a.label, a.keys, false);
    let short = |i: usize, a: &Act| if i > 0 && a.icon.is_some() { kit::button_width(ui, a.icon, "", None, false) } else { full(a) };
    let close_w = kit::button_width(ui, None, close.0, None, false);
    let sum = |w: &dyn Fn(usize, &Act) -> f32| acts.iter().enumerate().map(|(i, a)| w(i, a) + GAP).sum::<f32>();
    let wide = sum(&|_, a| full(a)) + close_w <= row.width();
    let icons_only = !wide;
    let close_x = icons_only && sum(&short) + close_w > row.width();
    let mut clicked = None;
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let ui = &mut ui;
    {
        ui.spacing_mut().item_spacing.x = GAP;
        for (i, a) in acts.iter().enumerate() {
            let bare = icons_only && i > 0 && a.icon.is_some();
            let r = if bare {
                kit::button_ex(ui, a.kind, a.icon, "", false).on_hover_text(&a.label)
            } else {
                kit::button_keys(ui, a.kind, a.icon, &a.label, a.keys, false)
            };
            #[cfg(debug_assertions)]
            crate::devtest::record(a.name, r.rect);
            if r.clicked() {
                clicked = Some(i);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let r = if close_x { kit::icon_button_ex(ui, icons::X, close.0, false, 28.0) } else { kit::button_ex(ui, kit::Kind::Ghost, None, close.0, false) };
            #[cfg(debug_assertions)]
            crate::devtest::record(close.1, r.rect);
            if r.clicked() {
                clicked = Some(acts.len());
            }
        });
    }
    clicked
}

/// `add` in a child of `ui` over `rect`, a place already taken (the parent's layout is left
/// alone).
fn place<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
    add(&mut child)
}
