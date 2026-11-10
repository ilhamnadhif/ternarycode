//! Reviewing what changed (FR-E4, E5): every file changed since the agent run started (a
//! snapshot taken when the agent appeared, outside the user's index) or since the last commit,
//! with the diff of the selected file.

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui::{self, Align2, RichText, Sense, vec2};
use tc_git::{ChangeKind, FileChange, FileDiff, LineKind};

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::{App, AppMsg};
use crate::i18n::{t, tf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    /// Since the agent run in this pane started.
    Run,
    /// Since the last commit.
    Head,
}

pub struct Review {
    pub root: PathBuf,
    pub base: Base,
    /// Agent label and when its run started (for `Base::Run`).
    pub run: Option<(String, Instant, String)>,
    pub loading: bool,
    pub from: Option<String>,
    pub to: Option<String>,
    pub files: Vec<FileChange>,
    pub selected: usize,
    pub diff: Option<FileDiff>,
    /// Why the changes could not be listed.
    pub error: Option<String>,
    /// Why the selected file's diff could not be made (the list stays).
    pub diff_error: Option<String>,
    /// Which load and which diff the answers must belong to (an older, slower one is dropped).
    load: u64,
    diff_load: u64,
}

/// Numbers review work, so late answers of earlier work are told apart.
static WORK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_work() -> u64 {
    WORK.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Results of review work done off the UI thread.
pub enum ReviewMsg {
    Loaded {
        load: u64,
        root: PathBuf,
        from: String,
        to: String,
        files: Result<Vec<FileChange>, String>,
    },
    Diff(u64, usize, Result<FileDiff, String>),
}

impl App {
    /// Opens the review for the current pane: its agent run when there is a snapshot, else
    /// changes since the last commit.
    pub fn open_review(&mut self, base: Option<Base>) {
        let Some(project) = self.active_project() else { return };
        let (folder, name) = (project.path.clone(), project.name.clone());
        let pane = self.current_pane();
        let run = pane.and_then(|id| self.panes.get(&id)).and_then(|p| {
            p.run_snapshot
                .as_ref()
                .map(|(tree, at, agent)| (agent.label().to_string(), *at, tree.clone()))
        });
        let base = base.unwrap_or(if run.is_some() { Base::Run } else { Base::Head });
        let load = next_work();
        self.review = Some(Review {
            load,
            diff_load: 0,
            root: folder.clone(),
            base,
            run: run.clone(),
            loading: true,
            from: None,
            to: None,
            files: Vec::new(),
            selected: 0,
            diff: None,
            error: None,
            diff_error: None,
        });
        let tx = self.app_tx.clone();
        let run_tree = run.map(|(_, _, tree)| tree);
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            // Even finding the repository runs git: not on the UI thread.
            let root = tc_git::repo_root(&folder);
            let result = (|| {
                let root = root.as_ref().ok_or_else(|| tf("{} is not a git repository.", &[&name]))?;
                let (head, now) = tc_git::worktree_vs_head(root)?;
                let from = match base {
                    Base::Run => run_tree.ok_or_else(|| crate::i18n::t("No snapshot of this run.").to_string())?,
                    Base::Head => match head {
                        Some(h) => h,
                        None => tc_git::empty_tree(root)?,
                    },
                };
                let files = tc_git::changes(root, &from, &now);
                Ok::<_, String>((from, now, files))
            })();
            let root = root.unwrap_or(folder);
            let msg = match result {
                Ok((from, to, files)) => ReviewMsg::Loaded { load, root, from, to, files },
                Err(e) => ReviewMsg::Loaded {
                    load,
                    root,
                    from: String::new(),
                    to: String::new(),
                    files: Err(e),
                },
            };
            let _ = tx.send(AppMsg::Review(msg));
            ctx.request_repaint();
        });
    }

    pub fn on_review_msg(&mut self, msg: ReviewMsg) {
        let Some(r) = self.review.as_mut() else { return };
        match msg {
            ReviewMsg::Loaded { load, root, from, to, files } => {
                if load != r.load {
                    return;
                }
                r.loading = false;
                r.root = root;
                match files {
                    Ok(f) => {
                        r.files = f;
                        r.from = Some(from);
                        r.to = Some(to);
                        r.selected = 0;
                        if !r.files.is_empty() {
                            self.load_review_diff(0);
                        }
                    }
                    Err(e) => r.error = Some(e),
                }
            }
            ReviewMsg::Diff(load, i, d) => {
                if r.selected == i && r.diff_load == load {
                    match d {
                        Ok(d) => r.diff = Some(d),
                        Err(e) => r.diff_error = Some(e),
                    }
                }
            }
        }
    }

    fn load_review_diff(&mut self, i: usize) {
        let Some(r) = self.review.as_mut() else { return };
        let (Some(from), Some(to), Some(file)) = (r.from.clone(), r.to.clone(), r.files.get(i).cloned()) else {
            return;
        };
        r.selected = i;
        r.diff = None;
        r.diff_error = None;
        let load = next_work();
        r.diff_load = load;
        let root = r.root.clone();
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let d = tc_git::file_diff(&root, &from, &to, &file.path, file.old_path.as_deref());
            let _ = tx.send(AppMsg::Review(ReviewMsg::Diff(load, i, d)));
            ctx.request_repaint();
        });
    }

    pub fn review_ui(&mut self, ctx: &egui::Context) {
        if self.review.is_none() {
            return;
        }
        let screen = ctx.content_rect();
        let size = vec2((screen.width() - 80.0).min(1400.0), screen.height() - 90.0);
        let mut close = false;
        let mut select = None;
        let mut switch = None;
        let mut open_file = None;
        let modal = egui::Modal::new(egui::Id::new("review")).show(ctx, |ui| {
            ui.set_width(size.x);
            ui.set_height(size.y);
            let p = style::pal(ui);
            let Some(r) = self.review.as_ref() else { return };
            ui.horizontal(|ui| {
                let title = match (&r.base, &r.run) {
                    (Base::Run, Some((agent, at, _))) => tf("Changes since {} started ({} ago)", &[agent, &super::status::age(*at)]),
                    _ => t("Changes since the last commit").to_string(),
                };
                ui.label(RichText::new(title).font(style::semibold(16.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if kit::icon_button(ui, icons::X, t("Close")).clicked() {
                        close = true;
                    }
                    let mut base = r.base;
                    let mut items: Vec<(Base, &str)> = vec![(Base::Head, t("Since last commit"))];
                    if r.run.is_some() {
                        items.insert(0, (Base::Run, t("Since the agent started")));
                    }
                    if items.len() > 1 {
                        kit::tab_strip(ui, &mut base, &items);
                        if base != r.base {
                            switch = Some(base);
                        }
                    }
                });
            });
            ui.add_space(6.0);
            if r.loading {
                ui.label(RichText::new(t("Looking at the working tree…")).color(p.text_muted));
                return;
            }
            if let Some(e) = &r.error {
                ui.label(RichText::new(e).color(p.err));
                return;
            }
            if r.files.is_empty() {
                ui.label(RichText::new(t("Nothing changed.")).color(p.text_muted));
                return;
            }
            let (adds, dels): (u32, u32) = r.files.iter().fold((0, 0), |(a, d), f| (a + f.additions.unwrap_or(0), d + f.deletions.unwrap_or(0)));
            ui.label(
                RichText::new(tf("{} files · +{} −{}", &[&r.files.len().to_string(), &adds.to_string(), &dels.to_string()]))
                    .size(style::SMALL)
                    .color(p.text_muted),
            );
            ui.add_space(4.0);
            let body_h = ui.available_height();
            ui.horizontal_top(|ui| {
                // File list.
                ui.allocate_ui_with_layout(vec2(300.0, body_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                    egui::ScrollArea::vertical().id_salt("review-files").auto_shrink([false, false]).show(ui, |ui| {
                        for (i, f) in r.files.iter().enumerate() {
                            let (badge, color) = match f.kind {
                                ChangeKind::Added => ("A", p.ok),
                                ChangeKind::Modified => ("M", p.warn),
                                ChangeKind::Deleted => ("D", p.err),
                                ChangeKind::Renamed => ("R", p.info),
                            };
                            let (rect, resp) = ui.allocate_exact_size(vec2(292.0, 26.0), Sense::click());
                            if i == r.selected || resp.hovered() {
                                ui.painter().rect_filled(rect, egui::CornerRadius::same(5), if i == r.selected { p.accent_soft } else { p.faint_row() });
                            }
                            ui.painter().text(rect.left_center() + vec2(10.0, 0.0), Align2::CENTER_CENTER, badge, style::semibold(11.0), color);
                            let name = match &f.old_path {
                                Some(old) => format!("{old} → {}", f.path),
                                None => f.path.clone(),
                            };
                            let mut job = egui::text::LayoutJob::simple_singleline(name, egui::FontId::proportional(12.5), p.text);
                            job.wrap = egui::text::TextWrapping::truncate_at_width(200.0);
                            let g = ui.painter().layout_job(job);
                            ui.painter().galley(rect.left_center() + vec2(22.0, -g.size().y / 2.0), g, p.text);
                            let counts = if f.binary { t("binary").to_string() } else { format!("+{} −{}", f.additions.unwrap_or(0), f.deletions.unwrap_or(0)) };
                            ui.painter().text(rect.right_center() - vec2(6.0, 0.0), Align2::RIGHT_CENTER, counts, egui::FontId::proportional(style::SMALL), p.text_faint);
                            if resp.clicked() {
                                select = Some(i);
                            }
                        }
                    });
                });
                ui.separator();
                // Diff.
                ui.vertical(|ui| {
                    let Some(f) = r.files.get(r.selected) else { return };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&f.path).font(style::medium(13.0)));
                        if f.kind != ChangeKind::Deleted && kit::ghost(ui, Some(icons::ARROW_SQUARE_OUT), t("Open in Editor")).clicked() {
                            open_file = Some(r.root.join(&f.path));
                        }
                    });
                    match (&r.diff, &r.diff_error) {
                        (_, Some(e)) => {
                            ui.label(RichText::new(e).color(p.err));
                        }
                        (None, None) => {
                            ui.label(RichText::new(t("Loading the diff…")).color(p.text_muted));
                        }
                        (Some(d), None) if d.binary => {
                            ui.label(RichText::new(t("Binary file: no text diff.")).color(p.text_muted));
                        }
                        (Some(d), None) => {
                            egui::ScrollArea::both().id_salt(("review-diff", r.selected)).auto_shrink([false, false]).show(ui, |ui| {
                                let mono = egui::FontId::monospace(12.0);
                                for h in &d.hunks {
                                    ui.label(RichText::new(&h.header).font(mono.clone()).color(p.info));
                                    for l in &h.lines {
                                        let (sign, fg, bg) = match l.kind {
                                            LineKind::Added => ("+", p.ok, style::soft(p.ok, 0.10)),
                                            LineKind::Removed => ("-", p.err, style::soft(p.err, 0.10)),
                                            LineKind::Context => (" ", p.text_muted, egui::Color32::TRANSPARENT),
                                        };
                                        let nums = format!(
                                            "{:>5} {:>5} {sign} ",
                                            l.old_no.map(|n| n.to_string()).unwrap_or_default(),
                                            l.new_no.map(|n| n.to_string()).unwrap_or_default()
                                        );
                                        // The tint goes under the text: its place is taken before the text.
                                        let tint = (bg != egui::Color32::TRANSPARENT).then(|| ui.painter().add(egui::Shape::Noop));
                                        let resp = ui.add(egui::Label::new(RichText::new(format!("{nums}{}", l.text)).font(mono.clone()).color(fg)).extend());
                                        if let Some(tint) = tint {
                                            ui.painter().set(tint, egui::Shape::rect_filled(resp.rect.expand2(vec2(4.0, 0.0)), egui::CornerRadius::ZERO, bg));
                                        }
                                    }
                                    ui.add_space(6.0);
                                }
                            });
                        }
                    }
                });
            });
        });
        if modal.should_close() {
            close = true;
        }
        if let Some(i) = select {
            self.load_review_diff(i);
        }
        if let Some(b) = switch {
            self.open_review(Some(b));
        }
        if let Some(path) = open_file {
            self.review = None;
            self.actions.push(Action::OpenFile(path, None));
            return;
        }
        if close {
            self.review = None;
            self.focus_request = self.current_pane();
        }
    }
}
