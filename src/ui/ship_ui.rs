//! The commit box at the foot of Review's file column (`crate::ship`): the message, one button
//! that says the whole of what it will do, where the branch goes; while it runs, its steps;
//! then the pull request, or what stopped it and the way on.

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Modifiers, Rect, Sense, Stroke, Ui, pos2, vec2};
use tc_core::ProjectId;
use tc_core::model::ShipAction;
use tc_core::status::Status;

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};
use crate::ship::{self, Failure, Plan, RunReq, StepKind, StepState};

/// Space inside the box, and between its parts.
const PAD: f32 = 10.0;
const GAP: f32 = 8.0;
const GO_H: f32 = 32.0;
const LINE_H: f32 = 16.0;
const STEP_H: f32 = 30.0;
const ACTS_H: f32 = 26.0;
const BRANCH_H: f32 = 26.0;
/// The message field: at least two lines high, at most this.
const MSG_MAX_H: f32 = 132.0;

/// What a click in the box asks for (done after drawing, when nothing is borrowed).
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

impl App {
    /// The id of a project's message field (Review's keys stay out of it).
    pub fn ship_message_id(project: ProjectId) -> egui::Id {
        egui::Id::new(("ship-message", project))
    }

    /// The commit box at the bottom of `col` (Review's file column). Returns its top.
    pub(super) fn ship_box(&mut self, ui: &mut Ui, col: Rect) -> f32 {
        let Some(r) = self.review.as_ref() else { return col.bottom() };
        let project = r.project;
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
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
        let Some(s) = self.ship.get(&project) else { return col.bottom() };
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

        let x = col.left() + PAD;
        let w = col.width() - PAD * 2.0;
        let font_note = egui::FontId::proportional(11.5);
        let note_w = w - 20.0;
        let mut asks: Vec<Ask> = Vec::new();

        // ----- what the box holds, measured first (it grows up from the bottom) -------------
        enum Part {
            Steps,
            Title,
            Message(f32),
            Source,
            Branch,
            Note(std::sync::Arc<egui::Galley>, &'static str, Color32),
            Output(std::sync::Arc<egui::Galley>),
            PrCard,
            Acts,
            Go,
            To,
            Commands(Vec<String>),
        }
        let mut parts: Vec<Part> = Vec::new();
        let note = |ui: &Ui, text: &str, icon: &'static str, color: Color32| {
            Part::Note(ui.painter().layout(text.to_string(), font_note.clone(), p.text_muted, note_w), icon, color)
        };
        let mut req_for_go: Option<RunReq> = None;
        if let Some(run) = &run {
            if run.steps.iter().any(|s| s.kind == StepKind::Commit) && !run.title.is_empty() {
                parts.push(Part::Title);
            }
            parts.push(Part::Steps);
            match (run.failed(), run.finished) {
                (Some((_, f)), _) => {
                    let run_gitlab = run.req.web.as_ref().is_some_and(|w| w.forge == tc_git::ship::Forge::GitLab);
                    let (text, icon, color) = match f {
                        Failure::Rejected(_) => (tf("{} has commits you don’t have. Your commit is safe here.", &[&format!("{}/{}", run.req.remote.as_deref().unwrap_or("origin"), run.req.branch)]), icons::WARNING, p.warn),
                        Failure::Credentials(_) => (t("git wants a password or an SSH passphrase. It can only ask in a terminal.").to_string(), icons::KEY, p.warn),
                        Failure::SignedOut(_) => (
                            if run_gitlab {
                                t("GitLab CLI isn’t signed in. The branch is pushed: finish the merge request on GitLab.").to_string()
                            } else {
                                t("GitHub CLI isn’t signed in. The branch is pushed: finish the pull request on GitHub.").to_string()
                            },
                            icons::INFO,
                            p.text_faint,
                        ),
                        Failure::Output(_) => (String::new(), icons::INFO, p.text_faint),
                    };
                    if !text.is_empty() {
                        parts.push(note(ui, &text, icon, color));
                    }
                    let out = f.output().trim();
                    if matches!(f, Failure::Output(_)) && !out.is_empty() {
                        let g = ui.painter().layout(out.to_string(), style::mono(10.5), p.text_muted, w - 20.0);
                        parts.push(Part::Output(g));
                    }
                    parts.push(Part::Acts);
                }
                (None, true) => {
                    let run_gitlab = run.req.web.as_ref().is_some_and(|w| w.forge == tc_git::ship::Forge::GitLab);
                    if run.pr.is_some() {
                        parts.push(Part::PrCard);
                    } else if run.form.is_some() {
                        let msg = if run_gitlab {
                            t("The merge request form is open in the Browser panel, filled in.")
                        } else {
                            t("The pull request form is open in the Browser panel, filled in.")
                        };
                        parts.push(note(ui, msg, icons::GLOBE, p.text_faint));
                    }
                    parts.push(Part::Acts);
                }
                (None, false) => {
                    parts.push(note(ui, t("Keep working: it runs in the background and says when it’s done."), icons::INFO, p.text_faint));
                }
            }
        } else if loading {
            parts.push(note(ui, t("Reading the branch…"), "", p.text_faint));
        } else if let Some(plan) = &plan {
            let commits = plan.has(&StepKind::Commit) || plan.blocked.is_some() || n_files > 0;
            if commits {
                let h = message_height(ui, &message, w - 42.0, &p);
                parts.push(Part::Message(h));
                if writing.is_some() || written_by.is_some() || write_error.is_some() || (auto && agent.is_some()) {
                    parts.push(Part::Source);
                }
            }
            if new_branch.is_some() {
                parts.push(Part::Branch);
            }
            if let Some(b) = &plan.blocked {
                parts.push(note(ui, b, icons::WARNING, p.warn));
            } else if plan.idle() {
                let branch = facts.as_ref().and_then(|f| f.branch.clone()).unwrap_or_default();
                let text = match (&pr, facts.as_ref().and_then(|f| f.remote.as_ref())) {
                    (Some(pr), _) if matches!(pr.state, tc_git::ship::PrState::Open | tc_git::ship::PrState::Draft) => {
                        if is_gitlab {
                            tf("Nothing to commit. {} is pushed; MR !{} is up to date.", &[&branch, &pr.number.to_string()])
                        } else {
                            tf("Nothing to commit. {} is pushed; PR #{} is up to date.", &[&branch, &pr.number.to_string()])
                        }
                    }
                    (_, Some(remote)) if facts.as_ref().is_some_and(|f| f.upstream.is_some()) => tf("Nothing to commit. {} is the same as {}.", &[&branch, &remote.name]),
                    _ => t("Nothing to commit.").to_string(),
                };
                parts.push(note(ui, &text, icons::CHECK_CIRCLE, p.ok));
            } else if facts.as_ref().is_some_and(|f| f.remote.is_none()) && plan.has(&StepKind::Commit) && prefs.action != ShipAction::Commit {
                parts.push(note(ui, t("No remote yet, so nothing to push to. Add one (git remote add origin …) and the button pushes too."), icons::INFO, p.text_faint));
            } else if plan.pr_step() == Some(false) && has_forge {
                let msg = if is_gitlab {
                    t("GitLab CLI isn’t installed: after the push, the merge request form opens in the Browser panel, filled in.")
                } else {
                    t("GitHub CLI isn’t installed: after the push, the pull request form opens in the Browser panel, filled in.")
                };
                parts.push(note(ui, msg, icons::INFO, p.text_faint));
            }
            if !plan.idle() || plan.blocked.is_some() {
                parts.push(Part::Go);
                parts.push(Part::To);
                if show_commands && plan.blocked.is_none() {
                    let req = self.ship_request(project, plan, &root, &paths, &message);
                    parts.push(Part::Commands(req.preview_commands(0)));
                    req_for_go = Some(req);
                }
            }
        }
        let height = |part: &Part| -> f32 {
            match part {
                Part::Steps => run.as_ref().map_or(0.0, |r| r.steps.len() as f32 * STEP_H + 8.0),
                Part::Title => LINE_H,
                Part::Message(h) => *h,
                Part::Source => LINE_H,
                Part::Branch => BRANCH_H,
                Part::Note(g, ..) => g.size().y.max(LINE_H),
                Part::Output(g) => g.size().y.min(8.0 * 15.0) + 16.0,
                Part::PrCard => 50.0,
                Part::Acts => ACTS_H,
                Part::Go => GO_H,
                Part::To => LINE_H,
                Part::Commands(c) => c.len() as f32 * 15.0 + 14.0,
            }
        };
        let total: f32 = parts.iter().map(height).sum::<f32>() + GAP * (parts.len().saturating_sub(1)) as f32;
        let top = (col.bottom() - total - PAD * 2.0).max(col.top() + 60.0);
        let area = Rect::from_min_max(pos2(col.left(), top), col.max);
        let painter = ui.painter().clone();
        painter.rect_filled(area.shrink2(vec2(0.5, 0.0)), CornerRadius::ZERO, style::over(p.canvas, p.chrome, 0.22));
        painter.hline(area.x_range(), area.top() + 0.5, Stroke::new(1.0, p.border));
        ui.data_mut(|d| d.insert_temp(egui::Id::new("ship-box-rect"), area));
        #[cfg(debug_assertions)]
        crate::devtest::record("ship-box", area);

        // ----- drawn top-down ---------------------------------------------------------------
        let mut y = area.top() + PAD;
        let plan_label = plan.as_ref().map(ship::label).unwrap_or_default();
        for part in &parts {
            let h = height(part);
            let row = Rect::from_min_size(pos2(x, y), vec2(w, h));
            match part {
                Part::Title => {
                    let title = run.as_ref().map(|r| r.title.clone()).unwrap_or_default();
                    painter.text(pos2(row.left() + 6.0, row.center().y), Align2::CENTER_CENTER, icons::GIT_COMMIT, style::icon_font(12.0), p.text_faint);
                    let mut job = egui::text::LayoutJob::simple_singleline(title, egui::FontId::proportional(12.0), p.text_muted);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(w - 20.0);
                    let g = painter.layout_job(job);
                    painter.galley(pos2(row.left() + 18.0, row.center().y - g.size().y / 2.0), g, p.text_muted);
                }
                Part::Steps => {
                    let Some(run) = &run else { continue };
                    let run_gitlab = run.req.web.as_ref().is_some_and(|w| w.forge == tc_git::ship::Forge::GitLab);
                    let fill = if dark { p.inset } else { Color32::WHITE };
                    painter.rect(row, CornerRadius::same(9), fill, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
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
                    if focused {
                        painter.rect_stroke(row.expand(3.0), CornerRadius::same(11), Stroke::new(3.0, p.accent_tint), egui::StrokeKind::Inside);
                    }
                    let fill = if dark { p.inset } else { Color32::WHITE };
                    painter.rect(row, CornerRadius::same(8), fill, Stroke::new(1.0, if focused { p.accent } else { p.border_strong }), egui::StrokeKind::Inside);
                    let inner = Rect::from_min_max(pos2(row.left() + 10.0, row.top() + 7.0), pos2(row.right() - 32.0, row.bottom() - 6.0));
                    let mut text = message.clone();
                    let (title_c, body_c) = (p.text, p.text_muted);
                    let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                        let s = buf.as_str();
                        let mut job = egui::text::LayoutJob::default();
                        let (first, rest) = match s.find('\n') {
                            Some(i) => (&s[..i], Some(&s[i..])),
                            None => (s, None),
                        };
                        job.append(first, 0.0, egui::TextFormat::simple(style::medium(12.5), title_c));
                        if let Some(rest) = rest {
                            job.append(rest, 0.0, egui::TextFormat::simple(egui::FontId::proportional(12.0), body_c));
                        }
                        job.wrap.max_width = wrap;
                        ui.fonts_mut(|f| f.layout_job(job))
                    };
                    let resp = ui
                        .scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                            egui::ScrollArea::vertical().id_salt(("ship-message-scroll", project)).max_height(inner.height()).auto_shrink([false, true]).show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut text)
                                        .id(id)
                                        .frame(egui::Frame::NONE)
                                        .margin(egui::Margin::ZERO)
                                        .desired_width(inner.width())
                                        .desired_rows(2)
                                        .hint_text(egui::RichText::new(t("Message (first line: what it does)")).color(p.text_faint))
                                        .layouter(&mut layouter),
                                )
                            })
                            .inner
                        })
                        .inner;
                    #[cfg(debug_assertions)]
                    crate::devtest::record("ship-message", row);
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
                    // ✦: the agent that did the work writes it from the diff.
                    if let Some(a) = agent.filter(|a| ship::can_write(*a)) {
                        let br = Rect::from_center_size(pos2(row.right() - 15.0, row.top() + 15.0), vec2(22.0, 22.0));
                        let resp = ui.interact(br, egui::Id::new(("ship-write", project)), Sense::click());
                        if resp.hovered() {
                            painter.rect_filled(br, CornerRadius::same(6), p.hover);
                        }
                        if writing.is_some() {
                            kit::paint_lattice(&painter, br.center(), 10.0, p.accent_text, true);
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                        } else {
                            let c = if written_by.is_some() { p.accent_text } else if resp.hovered() { p.text } else { p.text_faint };
                            painter.text(br.center(), Align2::CENTER_CENTER, icons::SPARKLE, style::icon_font(13.0), c);
                        }
                        #[cfg(debug_assertions)]
                        crate::devtest::record("ship-write", br);
                        if resp.on_hover_text(tf("Write it with {} (from the diff, with your plan)", &[a.label()])).clicked() && writing.is_none() {
                            asks.push(Ask::Write);
                        }
                    }
                }
                Part::Source => {
                    let (text, link) = if let Some(a) = writing {
                        (tf("{} is writing it from the diff…", &[a.label()]), None)
                    } else if let Some(e) = &write_error {
                        (e.clone(), None)
                    } else if let Some(a) = written_by {
                        (tf("Written by {} from the diff", &[a.label()]), has_undo.then_some(t("Undo")))
                    } else {
                        (tf("From {}’s task and its last words", &[agent.map(|a| a.label()).unwrap_or("")]), None)
                    };
                    let color = if write_error.is_some() && writing.is_none() { p.err } else { p.text_faint };
                    let mut lx = row.left();
                    if let Some(a) = agent.filter(|_| write_error.is_none() || writing.is_some()) {
                        super::agent_logo::paint(&painter, Rect::from_min_size(pos2(lx, row.center().y - 5.0), vec2(10.0, 10.0)), a, p.text, false);
                        lx += 15.0;
                    }
                    let link_w = link.map_or(0.0, |l| painter.layout_no_wrap(l.to_string(), egui::FontId::proportional(11.0), p.accent_text).size().x + 8.0);
                    let mut job = egui::text::LayoutJob::simple_singleline(text, egui::FontId::proportional(11.0), color);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(row.right() - lx - link_w);
                    let g = painter.layout_job(job);
                    painter.galley(pos2(lx, row.center().y - g.size().y / 2.0), g, color);
                    if let Some(l) = link {
                        let g = painter.layout_no_wrap(l.to_string(), egui::FontId::proportional(11.0), p.accent_text);
                        let lr = Rect::from_min_size(pos2(row.right() - g.size().x, row.center().y - g.size().y / 2.0), g.size());
                        painter.galley(lr.min, g, p.accent_text);
                        if ui.interact(lr.expand(3.0), egui::Id::new(("ship-undo", project)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            asks.push(Ask::Undo);
                        }
                    }
                }
                Part::Branch => {
                    painter.text(pos2(row.left() + 6.0, row.center().y), Align2::CENTER_CENTER, icons::GIT_BRANCH, style::icon_font(12.0), p.text_faint);
                    let fr = Rect::from_min_max(pos2(row.left() + 18.0, row.top()), pos2(row.right() - 26.0, row.bottom()));
                    let mut name = new_branch.clone().unwrap_or_default();
                    let resp = ui.scope_builder(egui::UiBuilder::new().max_rect(fr), |ui| kit::field(ui, egui::Id::new(("ship-branch", project)), &mut name, None, t("New branch"), fr.width(), true)).inner;
                    if resp.changed() {
                        self.ship_mut(project).new_branch = Some(name);
                    }
                    let xr = Rect::from_center_size(pos2(row.right() - 11.0, row.center().y), vec2(22.0, 22.0));
                    let x_resp = ui.interact(xr, egui::Id::new(("ship-branch-x", project)), Sense::click());
                    painter.text(xr.center(), Align2::CENTER_CENTER, icons::X, style::icon_font(11.0), if x_resp.hovered() { p.text } else { p.text_faint });
                    if x_resp.on_hover_text(t("Commit on this branch")).clicked() {
                        asks.push(Ask::CancelBranch);
                    }
                }
                Part::Note(g, icon, color) => {
                    if !icon.is_empty() {
                        painter.text(pos2(row.left() + 6.5, row.top() + 8.0), Align2::CENTER_CENTER, *icon, style::icon_font(13.0), *color);
                    } else {
                        kit::paint_lattice(&painter, pos2(row.left() + 6.5, row.top() + 8.0), 10.0, p.info, true);
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                    }
                    painter.galley(pos2(row.left() + 20.0, row.top()), g.clone(), p.text_muted);
                }
                Part::Output(g) => {
                    let fill = if dark { p.inset } else { Color32::WHITE };
                    painter.rect(row, CornerRadius::same(8), fill, Stroke::new(1.0, style::soft(p.err, 0.3)), egui::StrokeKind::Inside);
                    let clip = row.shrink2(vec2(10.0, 8.0));
                    painter.with_clip_rect(clip).galley(clip.min, g.clone(), p.text_muted);
                }
                Part::PrCard => {
                    let Some(pr) = run.as_ref().and_then(|r| r.pr.clone()) else { continue };
                    let run_gitlab = run.as_ref().and_then(|r| r.req.web.as_ref()).is_some_and(|w| w.forge == tc_git::ship::Forge::GitLab);
                    let prefix = if run_gitlab { "!" } else { "#" };
                    painter.rect(row, CornerRadius::same(9), style::over(p.canvas, p.ok, 0.09), Stroke::new(1.0, style::soft(p.ok, 0.26)), egui::StrokeKind::Inside);
                    let ic = Rect::from_min_size(pos2(row.left() + 10.0, row.center().y - 14.0), vec2(28.0, 28.0));
                    painter.rect_filled(ic, CornerRadius::same(8), style::soft(p.ok, 0.14));
                    painter.text(ic.center(), Align2::CENTER_CENTER, icons::GIT_PULL_REQUEST, style::icon_font(15.0), p.ok);
                    let tx = ic.right() + 10.0;
                    let title = format!("{prefix}{} {}", pr.number, run.as_ref().map(|r| r.title.as_str()).unwrap_or(""));
                    let mut job = egui::text::LayoutJob::simple_singleline(title, style::semibold(12.5), p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(row.right() - 10.0 - tx);
                    let g = painter.layout_job(job);
                    painter.galley(pos2(tx, row.center().y - g.size().y - 0.5), g, p.text);
                    let short = pr.url.trim_start_matches("https://").to_string();
                    let mut job = egui::text::LayoutJob::simple_singleline(short, style::mono(10.5), p.text_faint);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(row.right() - 10.0 - tx);
                    let g = painter.layout_job(job);
                    painter.galley(pos2(tx, row.center().y + 2.0), g, p.text_faint);
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
                                Failure::Rejected(_) => {
                                    let remote = run.req.remote.clone().unwrap_or_else(|| "origin".into());
                                    let pull = format!("git pull --rebase {} {}", ship::quote(&remote), ship::quote(&run.req.branch));
                                    btns.push((Act::new(kit::Kind::Primary, Some(icons::TERMINAL_WINDOW), t("Pull & Push in Terminal"), "ship-act-pull"), Ask::TerminalFrom { from: i, prefix: Some(pull) }));
                                }
                                Failure::Credentials(_) => {
                                    btns.push((Act::new(kit::Kind::Primary, Some(icons::TERMINAL_WINDOW), t("Push in Terminal"), "ship-act-push"), Ask::TerminalFrom { from: i, prefix: None }));
                                }
                                Failure::SignedOut(_) => {
                                    let run_gitlab = run.req.web.as_ref().is_some_and(|w| w.forge == tc_git::ship::Forge::GitLab);
                                    if let Some(web) = &run.req.web {
                                        let (title, body) = tc_git::ship::split_message(&run.req.message);
                                        let url = web.new_pr_url(run.req.base.as_deref().unwrap_or("main"), &run.req.branch, &title, &body);
                                        let finish_label = if run_gitlab { t("Finish on GitLab") } else { t("Finish on GitHub") };
                                        btns.push((Act::new(kit::Kind::Primary, Some(icons::GLOBE), finish_label, "ship-act-web"), Ask::OpenPr(url)));
                                    }
                                    let login_cmd = if run_gitlab { "glab auth login" } else { "gh auth login" };
                                    btns.push((Act::new(kit::Kind::Ghost, Some(icons::TERMINAL_WINDOW), login_cmd, "ship-act-login"), Ask::Terminal(vec![login_cmd.to_string()])));
                                }
                                Failure::Output(out) => {
                                    let commit = run.steps.get(i).is_some_and(|s| s.kind == StepKind::Commit);
                                    if commit && let Some(a) = agent {
                                        btns.push((Act::new(kit::Kind::Primary, None, &tf("Ask {} to Fix", &[a.label()]), "ship-act-fix"), Ask::Fix(out.clone())));
                                    }
                                    btns.push((Act::new(kit::Kind::Secondary, Some(icons::TERMINAL_WINDOW), t("Run in Terminal"), "ship-act-terminal"), Ask::TerminalFrom { from: i, prefix: None }));
                                }
                            }
                        }
                        None => {
                            close = (t("Done"), "ship-act-done");
                            if let Some(pr) = &run.pr {
                                let mut open = Act::new(kit::Kind::Primary, Some(icons::GLOBE), t("Open"), "ship-act-open");
                                open.keys = Some("↵");
                                btns.push((open, Ask::OpenPr(pr.url.clone())));
                                btns.push((Act::new(kit::Kind::Secondary, Some(icons::COPY), t("Copy Link"), "ship-act-copy"), Ask::Copy(pr.url.clone())));
                            } else if let Some(url) = &run.form {
                                btns.push((Act::new(kit::Kind::Secondary, Some(icons::GLOBE), t("Open Again"), "ship-act-form"), Ask::OpenPr(url.clone())));
                            }
                        }
                    }
                    let (acts, mut tasks): (Vec<Act>, Vec<Ask>) = btns.into_iter().unzip();
                    match act_row(ui, row, &acts, close) {
                        Some(n) if n < tasks.len() => asks.push(tasks.swap_remove(n)),
                        Some(_) => asks.push(Ask::Done),
                        None => {}
                    }
                    // ↵ opens the pull request once it is made (when the box has the keyboard).
                    if run.failed().is_none()
                        && let Some(pr) = &run.pr
                        && ui.memory(|m| m.focused().is_none())
                        && self.dialog.is_none()
                        && self.palette.is_none()
                        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
                    {
                        asks.push(Ask::OpenPr(pr.url.clone()));
                    }
                }
                Part::Go => {
                    let Some(plan) = &plan else { continue };
                    let enabled = plan.blocked.is_none() && !plan.idle() && !(plan.has(&StepKind::Commit) && message.trim().is_empty()) && writing.is_none() && new_branch.as_deref().is_none_or(|b| tc_git::worktree::valid_branch(b.trim()));
                    let icon = if plan.pr_step() == Some(false) {
                        icons::GLOBE
                    } else if plan.pr_step().is_some() {
                        icons::GIT_PULL_REQUEST
                    } else if plan.has(&StepKind::Push) {
                        icons::ARROW_UP
                    } else {
                        icons::GIT_COMMIT
                    };
                    let label = if plan.blocked.is_some() { t("Commit").to_string() } else { plan_label.clone() };
                    let (main, caret) = split_button(ui, row, project, icon, &label, enabled, &p);
                    #[cfg(debug_assertions)]
                    {
                        crate::devtest::record("ship-go", main.rect);
                        crate::devtest::record("ship-more", caret.rect);
                    }
                    let tip = if !enabled && plan.has(&StepKind::Commit) && message.trim().is_empty() { t("Write a message first") } else { "" };
                    let main = if tip.is_empty() { main } else { main.on_disabled_hover_text(tip) };
                    if main.clicked() && enabled {
                        asks.push(Ask::Go);
                    }
                    egui::Popup::menu(&caret).align(egui::RectAlign::TOP_END).width(260.0).show(|ui| {
                        kit::menu_body(ui);
                        kit::menu_header(ui, t("The button does"));
                        for (a, icon, label) in [
                            (ShipAction::Commit, icons::GIT_COMMIT, t("Commit")),
                            (ShipAction::CommitPush, icons::ARROW_UP, t("Commit & Push")),
                            (
                                ShipAction::PullRequest,
                                icons::GIT_PULL_REQUEST,
                                if is_gitlab { t("Commit, Push & Create MR") } else { t("Commit, Push & Create PR") },
                            ),
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
                        let draft_label = if is_gitlab { t("Create MR as Draft") } else { t("Create PR as Draft") };
                        let item = kit::menu_item(ui, Some(if prefs.draft { icons::CHECK } else { icons::FILE_DASHED }), draft_label, None, false);
                        if item.clicked() {
                            asks.push(Ask::Draft);
                            ui.close();
                        }
                        let web = facts.as_ref().and_then(|f| f.remote.as_ref()).is_some_and(|r| r.web.is_some());
                        if web && new_branch.is_none() {
                            let item = kit::menu_item(ui, Some(icons::GIT_BRANCH), t("Commit to a New Branch…"), None, false);
                            #[cfg(debug_assertions)]
                            crate::devtest::record("ship-new-branch", item.rect);
                            if item.clicked() {
                                asks.push(Ask::NewBranch);
                                ui.close();
                            }
                        }
                        kit::menu_sep(ui);
                        ui.add_enabled_ui(plan.blocked.is_none() && !plan.idle(), |ui| {
                            let item = kit::menu_item(ui, Some(icons::TERMINAL_WINDOW), t("Run in Terminal Instead"), None, false);
                            if item.clicked() {
                                let req = self.ship_request(project, plan, &root, &paths, &message);
                                asks.push(Ask::Terminal(req.commands(0)));
                                ui.close();
                            }
                        });
                        let item = kit::menu_item(ui, Some(icons::CODE), if show_commands { t("Hide the Commands") } else { t("Show the Commands") }, None, false);
                        if item.clicked() {
                            asks.push(Ask::ToggleCommands);
                            ui.close();
                        }
                    });
                }
                Part::To => {
                    let Some(plan) = &plan else { continue };
                    let mono = style::mono(10.5);
                    let mut job = egui::text::LayoutJob::default();
                    let fmt = |c: Color32| egui::TextFormat::simple(mono.clone(), c);
                    job.append(&plan.branch, 0.0, fmt(p.text_faint));
                    match (&plan.remote, prefs.action) {
                        (Some(remote), a) if a != ShipAction::Commit => {
                            job.append("→", 5.0, fmt(p.text_ghost));
                            if let Some(n) = plan.updates {
                                let upd = if is_gitlab {
                                    tf("updates MR !{}", &[&n.to_string()])
                                } else {
                                    tf("updates PR #{}", &[&n.to_string()])
                                };
                                job.append(&upd, 5.0, fmt(p.text_faint));
                            } else if plan.pr_step().is_some() {
                                let into = if is_gitlab {
                                    tf("MR into {}", &[plan.base.as_deref().unwrap_or("main")])
                                } else {
                                    tf("PR into {}", &[plan.base.as_deref().unwrap_or("main")])
                                };
                                job.append(&format!("{remote} · {into}"), 5.0, fmt(p.text_faint));
                            } else {
                                job.append(&format!("{remote}/{}", plan.branch), 5.0, fmt(p.text_faint));
                            }
                        }
                        _ => job.append(&format!("· {}", t("commit only")), 5.0, fmt(p.text_faint)),
                    }
                    let link = plan.can_branch.then_some(if is_gitlab { t("Branch & MR…") } else { t("Branch & PR…") });
                    let link_g = link.map(|l| painter.layout_no_wrap(l.to_string(), egui::FontId::proportional(11.0), p.text_faint));
                    let link_w = link_g.as_ref().map_or(0.0, |g| g.size().x + 10.0);
                    painter.text(pos2(row.left() + 6.0, row.center().y), Align2::CENTER_CENTER, icons::GIT_BRANCH, style::icon_font(11.0), p.text_faint);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(w - 16.0 - link_w);
                    let g = painter.layout_job(job);
                    painter.galley(pos2(row.left() + 16.0, row.center().y - g.size().y / 2.0), g, p.text_faint);
                    if let Some(g) = link_g {
                        let lr = Rect::from_min_size(pos2(row.right() - g.size().x, row.center().y - g.size().y / 2.0), g.size());
                        let resp = ui.interact(lr.expand(3.0), egui::Id::new(("ship-branch-link", project)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                        painter.galley(lr.min, g, if resp.hovered() { p.text } else { p.text_faint });
                        #[cfg(debug_assertions)]
                        crate::devtest::record("ship-branch-link", lr);
                        if resp.on_hover_text(t("Make a branch for a pull request, then commit there")).clicked() {
                            asks.push(Ask::NewBranch);
                        }
                    }
                }
                Part::Commands(cmds) => {
                    let fill = if dark { p.inset } else { Color32::from_rgb(0xf3, 0xf3, 0xf2) };
                    painter.rect(row, CornerRadius::same(8), fill, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                    let clip = row.shrink2(vec2(10.0, 7.0));
                    let pc = painter.with_clip_rect(clip);
                    for (i, c) in cmds.iter().enumerate() {
                        let cy = clip.top() + i as f32 * 15.0 + 7.5;
                        pc.text(pos2(clip.left(), cy), Align2::LEFT_CENTER, "$", style::mono(10.5), p.text_ghost);
                        pc.text(pos2(clip.left() + 12.0, cy), Align2::LEFT_CENTER, c, style::mono(10.5), p.text_muted);
                    }
                    let copy = Rect::from_center_size(pos2(row.right() - 14.0, row.top() + 13.0), vec2(20.0, 20.0));
                    let resp = ui.interact(copy, egui::Id::new(("ship-commands-copy", project)), Sense::click());
                    painter.rect_filled(copy, CornerRadius::same(5), if resp.hovered() { p.hover } else { fill });
                    painter.text(copy.center(), Align2::CENTER_CENTER, icons::COPY, style::icon_font(11.5), p.text_faint);
                    if resp.on_hover_text(t("Copy")).clicked() {
                        let materialized = req_for_go.as_ref().map(|r| r.commands(0));
                        asks.push(Ask::Copy(ship::chain(materialized.as_deref().unwrap_or(cmds))));
                    }
                }
            }
            y += h + GAP;
        }

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

/// The message field's height for `text` at `width`: two lines at least.
fn message_height(ui: &Ui, text: &str, width: f32, p: &style::Palette) -> f32 {
    let mut job = egui::text::LayoutJob::default();
    let (first, rest) = match text.find('\n') {
        Some(i) => (&text[..i], Some(&text[i..])),
        None => (text, None),
    };
    job.append(if first.is_empty() { " " } else { first }, 0.0, egui::TextFormat::simple(style::medium(12.5), p.text));
    if let Some(rest) = rest {
        job.append(rest, 0.0, egui::TextFormat::simple(egui::FontId::proportional(12.0), p.text));
    }
    job.wrap.max_width = width;
    let g = ui.painter().layout_job(job);
    (g.size().y + 14.0).clamp(52.0, MSG_MAX_H)
}

fn step_name(k: &StepKind) -> &'static str {
    match k {
        StepKind::Branch(_) => "branch",
        StepKind::Commit => "commit",
        StepKind::Push => "push",
        StepKind::PullRequest { .. } => "pr",
    }
}

/// One step's row: its state, its name, what it made, how long it took.
fn paint_step(painter: &egui::Painter, r: Rect, step: &ship::Step, gitlab: bool, p: &style::Palette) {
    let cy = r.center().y;
    let gx = r.left() + 12.0;
    let (name_c, detail_c) = match &step.state {
        StepState::Failed(_) => {
            painter.rect_filled(r, CornerRadius::same(6), style::soft(p.err, 0.10));
            (p.text, p.text_faint)
        }
        StepState::Todo => (p.text_faint, p.text_ghost),
        _ => (p.text, p.text_faint),
    };
    match &step.state {
        StepState::Done => super::status::paint_glyph(painter, pos2(gx, cy), Status::Done, p, 14.0),
        StepState::Running => kit::paint_lattice(painter, pos2(gx, cy), 10.0, p.info, true),
        StepState::Failed(_) => super::status::paint_glyph(painter, pos2(gx, cy), Status::Failed(1), p, 14.0),
        StepState::Todo => {
            painter.circle_stroke(pos2(gx, cy), 5.0, Stroke::new(1.2, p.text_ghost));
        }
    }
    let name = match &step.kind {
        StepKind::Branch(_) => t("Branch"),
        StepKind::Commit => t("Commit"),
        StepKind::Push => t("Push"),
        StepKind::PullRequest { .. } => if gitlab { t("Merge request") } else { t("Pull request") },
    };
    let font = if step.state == StepState::Todo { egui::FontId::proportional(12.5) } else { style::medium(12.5) };
    let g = painter.layout_no_wrap(name.to_string(), font, name_c);
    let nx = gx + 13.0;
    let nw = g.size().x;
    painter.galley(pos2(nx, cy - g.size().y / 2.0), g, name_c);
    let took = step.took.filter(|_| matches!(step.state, StepState::Done)).map(|d| format!("{:.1} s", d.as_secs_f32()));
    let took_w = took.as_ref().map_or(0.0, |s| painter.layout_no_wrap(s.clone(), style::mono(10.0), p.text_ghost).size().x + 8.0);
    if let Some(s) = &took {
        painter.text(pos2(r.right() - 8.0, cy), Align2::RIGHT_CENTER, s, style::mono(10.0), p.text_ghost);
    }
    let detail = match (&step.state, &step.kind) {
        (StepState::Failed(f), _) => match f {
            Failure::Rejected(_) => t("rejected").to_string(),
            Failure::Credentials(_) => t("needs your key").to_string(),
            Failure::SignedOut(_) => if gitlab { t("glab isn’t signed in").to_string() } else { t("gh isn’t signed in").to_string() },
            Failure::Output(_) => t("stopped").to_string(),
        },
        (StepState::Todo, StepKind::Branch(name)) => name.clone(),
        _ => step.detail.clone(),
    };
    if !detail.is_empty() {
        let mut job = egui::text::LayoutJob::simple_singleline(detail, style::mono(10.5), detail_c);
        job.wrap = egui::text::TextWrapping::truncate_at_width((r.right() - 8.0 - took_w - (nx + nw + 8.0)).max(10.0));
        let g = painter.layout_job(job);
        painter.galley(pos2(nx + nw + 8.0, cy - g.size().y / 2.0), g, detail_c);
    }
}

/// The button and its ▾, painted as one: the main part and a narrow part for the menu.
fn split_button(ui: &mut Ui, row: Rect, project: ProjectId, icon: &str, label: &str, enabled: bool, p: &style::Palette) -> (egui::Response, egui::Response) {
    let caret_w = 30.0;
    let main_r = Rect::from_min_max(row.min, pos2(row.right() - caret_w - 1.0, row.bottom()));
    let caret_r = Rect::from_min_max(pos2(row.right() - caret_w, row.top()), row.max);
    let main = ui.interact(main_r, egui::Id::new(("ship-go", project)), if enabled { Sense::click() } else { Sense::hover() });
    let caret = ui.interact(caret_r, egui::Id::new(("ship-more", project)), Sense::click());
    let painter = ui.painter();
    let fill = |hover: bool, on: bool| {
        let c = if hover && on { p.accent_hover } else { p.accent };
        if on { c } else { c.gamma_multiply(0.45) }
    };
    let r = 7;
    painter.rect_filled(main_r, CornerRadius { nw: r, sw: r, ne: 0, se: 0 }, fill(main.hovered(), enabled));
    painter.rect_filled(caret_r, CornerRadius { nw: 0, sw: 0, ne: r, se: r }, fill(caret.hovered(), true));
    let on_c = if enabled { p.on_accent } else { p.on_accent.gamma_multiply(0.7) };
    // Icon, words, keys, centered in the main part.
    let icon_g = painter.layout_no_wrap(icon.to_string(), style::icon_font(14.0), on_c);
    let keys = if cfg!(target_os = "macos") { "⌘↵" } else { "Ctrl+↵" };
    let keys_g = painter.layout_no_wrap(keys.to_string(), style::mono(10.5), on_c.gamma_multiply(0.7));
    let room = main_r.width() - 20.0 - icon_g.size().x - 7.0 - keys_g.size().x - 7.0;
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), style::medium(12.5), on_c);
    job.wrap = egui::text::TextWrapping::truncate_at_width(room.max(20.0));
    let text_g = painter.layout_job(job);
    let total = icon_g.size().x + 7.0 + text_g.size().x + 7.0 + keys_g.size().x;
    let mut cx = main_r.center().x - total / 2.0;
    let cy = main_r.center().y;
    painter.galley(pos2(cx, cy - icon_g.size().y / 2.0), icon_g.clone(), on_c);
    cx += icon_g.size().x + 7.0;
    painter.galley(pos2(cx, cy - text_g.size().y / 2.0), text_g.clone(), on_c);
    cx += text_g.size().x + 7.0;
    painter.galley(pos2(cx, cy - keys_g.size().y / 2.0), keys_g, on_c);
    painter.text(caret_r.center(), Align2::CENTER_CENTER, icons::CARET_DOWN, style::icon_font(12.0), p.on_accent);
    let main = main.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default });
    (main, caret.on_hover_text(t("What the button does")))
}

/// A button of the box's last row.
struct Act {
    kind: kit::Kind,
    icon: Option<&'static str>,
    label: String,
    keys: Option<&'static str>,
    name: &'static str,
}

impl Act {
    fn new(kind: kit::Kind, icon: Option<&'static str>, label: &str, name: &'static str) -> Act {
        Act { kind, icon, label: label.to_string(), keys: None, name }
    }
}

/// The box's last row: its buttons from the left and the closing word at the right, made to fit
/// a narrow column (the buttons after the first keep only their icon, then the closing word
/// becomes ×). Returns the index of the button clicked, `acts.len()` for the closing word. Each
/// is recorded for the end-to-end tests.
fn act_row(ui: &mut Ui, row: Rect, acts: &[Act], close: (&str, &'static str)) -> Option<usize> {
    const GAP: f32 = 6.0;
    let full = |a: &Act| kit::button_width(ui, a.icon, &a.label, a.keys, true);
    let short = |i: usize, a: &Act| if i > 0 && a.icon.is_some() { kit::button_width(ui, a.icon, "", None, true) } else { full(a) };
    let close_w = kit::button_width(ui, None, close.0, None, true);
    let sum = |w: &dyn Fn(usize, &Act) -> f32| acts.iter().enumerate().map(|(i, a)| w(i, a) + GAP).sum::<f32>();
    let wide = sum(&|_, a| full(a)) + close_w <= row.width();
    let icons_only = !wide;
    let close_x = icons_only && sum(&short) + close_w > row.width();
    let mut clicked = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        for (i, a) in acts.iter().enumerate() {
            let bare = icons_only && i > 0 && a.icon.is_some();
            let r = if bare {
                kit::button_ex(ui, a.kind, a.icon, "", true).on_hover_text(&a.label)
            } else {
                kit::button_keys(ui, a.kind, a.icon, &a.label, a.keys, true)
            };
            #[cfg(debug_assertions)]
            crate::devtest::record(a.name, r.rect);
            if r.clicked() {
                clicked = Some(i);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let r = if close_x { kit::icon_button_ex(ui, icons::X, close.0, false, 24.0) } else { kit::button_ex(ui, kit::Kind::Ghost, None, close.0, true) };
            #[cfg(debug_assertions)]
            crate::devtest::record(close.1, r.rect);
            if r.clicked() {
                clicked = Some(acts.len());
            }
        });
    });
    clicked
}
