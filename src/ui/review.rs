//! Reviewing what changed (FR-E4, E5): every file changed since the agent run started (a
//! snapshot taken when the agent appeared, outside the user's index) or since the last commit,
//! with the diff of the selected file.

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
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
    /// A file to select once the changes are listed (the Files column's Changes).
    pub want: Option<PathBuf>,
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
            want: None,
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
                        // The file asked for (from the Files column), else the first.
                        let want = r.want.take();
                        let root = r.root.clone();
                        r.selected = want
                            .and_then(|w| r.files.iter().position(|f| root.join(&f.path) == w || std::path::Path::new(&f.path) == w))
                            .unwrap_or(0);
                        let first = r.selected;
                        if !r.files.is_empty() {
                            self.load_review_diff(first);
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

    /// Review, in place of the panes (the sidebar stays in view): what changed since the agent
    /// started or since the last commit, a file list at the left and the selected file's diff.
    /// j / k change file, n / p hunk, ↵ opens it in the editor, ⇥ switches the base, Esc goes
    /// back to the terminals.
    pub fn review_view(&mut self, ui: &mut Ui) {
        let Some(r) = self.review.as_ref() else { return };
        let p = style::pal(ui);
        let full = ui.max_rect();
        let project = self.active_project().map(|x| x.name.clone()).unwrap_or_default();
        let who = r.run.as_ref().map(|(agent, _, _)| agent.clone());
        let pane_label = self.current_pane().and_then(|id| self.panes.get(&id)).map(|x| x.label()).unwrap_or_default();
        let base = r.base;
        let has_run = r.run.is_some();
        let run_age = r.run.as_ref().map(|(_, at, _)| super::status::age(*at));
        let (adds, dels): (u32, u32) = r.files.iter().fold((0, 0), |(a, d), f| (a + f.additions.unwrap_or(0), d + f.deletions.unwrap_or(0)));
        let nfiles = r.files.len();
        let mut switch = None;

        // ----- keys ---------------------------------------------------------------------------
        let free = self.dialog.is_none() && self.palette.is_none() && !ui.ctx().memory(|m| m.focused().is_some_and(|f| f == egui::Id::new("review-filter")));
        let mut close = false;
        let mut select: Option<usize> = None;
        let mut hunk_step: i32 = 0;
        let mut open = false;
        if self.dialog.is_none() && self.palette.is_none() {
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                    close = true;
                }
                if !free {
                    return;
                }
                let sel = r.selected;
                if i.consume_key(egui::Modifiers::NONE, Key::J) && sel + 1 < nfiles {
                    select = Some(sel + 1);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::K) && sel > 0 {
                    select = Some(sel - 1);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::N) {
                    hunk_step = 1;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::P) {
                    hunk_step = -1;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Enter) {
                    open = true;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Tab) && has_run {
                    switch = Some(if base == Base::Run { Base::Head } else { Base::Run });
                }
            });
        }

        // ----- top bar ------------------------------------------------------------------------
        let since = match (&who, &run_age) {
            (Some(agent), Some(age)) => tf("Since {} started · {}", &[agent, age]),
            _ => t("Since the agent started").to_string(),
        };
        let mut options = vec![kit::Seg::new(t("Since last commit"), "")];
        if has_run {
            options.insert(0, kit::Seg::new(&since, ""));
        }
        let on = if has_run && base == Base::Head { 1 } else { 0 };
        let width = kit::segmented_width(ui, &options);
        let mut centre = |app: &mut Self, ui: &mut Ui, rect: Rect| {
            if let Some(i) = kit::segmented_in(ui, rect, egui::Id::new("review-base"), &options, on) {
                app.review_switch = Some(if has_run && i == 0 { Base::Run } else { Base::Head });
            }
        };
        let body = self.top_bar(
            ui,
            full,
            |app, ui| {
                if kit::icon_button_ex(ui, icons::ARROW_LEFT, &tf("Back   {}", &["Esc"]), false, 26.0).clicked() {
                    app.review_close = true;
                }
                ui.label(RichText::new(t("Review")).font(style::semibold(14.0)).color(p.text));
                ui.label(RichText::new(format!("{project} · {pane_label}")).size(12.5).color(p.text_faint));
            },
            Some((width, &mut centre)),
        );
        if std::mem::take(&mut self.review_close) {
            close = true;
        }
        if let Some(b) = self.review_switch.take() {
            switch = Some(b);
        }

        // ----- body ---------------------------------------------------------------------------
        let Some(r) = self.review.as_ref() else { return };
        let files_rect = Rect::from_min_max(body.min, pos2(body.left() + FILES_W, body.bottom()));
        let diff_rect = Rect::from_min_max(pos2(files_rect.right(), body.top()), body.max);
        ui.painter().rect_filled(files_rect, CornerRadius::ZERO, p.chrome);
        ui.painter().vline(files_rect.right() - 0.5, files_rect.y_range(), Stroke::new(1.0, p.border));
        ui.painter().rect_filled(diff_rect, CornerRadius::ZERO, p.canvas);
        let empty = |ui: &mut Ui, text: &str, color: Color32| {
            ui.painter().text(diff_rect.center(), Align2::CENTER_CENTER, text, egui::FontId::proportional(13.0), color);
        };
        if r.loading {
            empty(ui, t("Looking at the working tree…"), p.text_muted);
        } else if let Some(e) = &r.error {
            empty(ui, e, p.err);
        } else if r.files.is_empty() {
            empty(ui, t("Nothing changed."), p.text_muted);
        }

        // Files: a filter, the list, a note on what it is compared with.
        let filter_rect = Rect::from_min_size(files_rect.min + vec2(8.0, 8.0), vec2(FILES_W - 16.0, 26.0));
        let mut filter = self.review_filter.clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(filter_rect), |ui| {
            kit::field(ui, egui::Id::new("review-filter"), &mut filter, Some(icons::FUNNEL), t("Filter changed files"), filter_rect.width(), true)
        });
        self.review_filter = filter.clone();
        let note = match base {
            Base::Run => tf("Snapshot taken when {} started, outside your git index.", &[who.as_deref().unwrap_or("")]),
            Base::Head => t("Compared with the last commit.").to_string(),
        };
        let note_g = ui.painter().layout(note, egui::FontId::proportional(style::SMALL), p.text_faint, FILES_W - 28.0);
        let note_rect = Rect::from_min_max(pos2(files_rect.left(), files_rect.bottom() - note_g.size().y - 20.0), files_rect.max);
        ui.painter().hline(note_rect.x_range(), note_rect.top() + 0.5, Stroke::new(1.0, p.border));
        ui.painter().galley(pos2(note_rect.left() + 14.0, note_rect.top() + 10.0), note_g, p.text_faint);
        let list_rect = Rect::from_min_max(pos2(files_rect.left() + 6.0, filter_rect.bottom() + 8.0), pos2(files_rect.right() - 6.0, note_rect.top()));
        let shown: Vec<usize> = r
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| filter.trim().is_empty() || f.path.to_lowercase().contains(&filter.trim().to_lowercase()))
            .map(|(i, _)| i)
            .collect();
        ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
            ui.set_clip_rect(list_rect);
            egui::ScrollArea::vertical().id_salt("review-files").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for i in &shown {
                    let f = &r.files[*i];
                    let (letter, color) = change_letter(f.kind, &p);
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("review-file:{}", f.path), rect);
                    if *i == r.selected || resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(5), if *i == r.selected { p.accent_soft } else { p.faint_row() });
                    }
                    ui.painter().text(pos2(rect.left() + 10.0 + 5.0, rect.center().y), Align2::CENTER_CENTER, letter, style::semibold(11.0), color);
                    // The counts at the right, the path cut before them.
                    let mut job = egui::text::LayoutJob::default();
                    if f.binary {
                        job.append(t("binary"), 0.0, egui::TextFormat::simple(egui::FontId::proportional(11.0), p.text_faint));
                    } else {
                        if let Some(a) = f.additions.filter(|a| *a > 0) {
                            job.append(&format!("+{a}"), 0.0, egui::TextFormat::simple(egui::FontId::proportional(11.0), p.ok));
                        }
                        if let Some(d) = f.deletions.filter(|d| *d > 0) {
                            job.append(&format!("−{d}"), 4.0, egui::TextFormat::simple(egui::FontId::proportional(11.0), p.err));
                        }
                    }
                    let counts = ui.painter().layout_job(job);
                    let cw = counts.size().x;
                    ui.painter().galley(pos2(rect.right() - 10.0 - cw, rect.center().y - counts.size().y / 2.0), counts, p.text_faint);
                    let name = match &f.old_path {
                        Some(old) => format!("{old} → {}", f.path),
                        None => f.path.clone(),
                    };
                    let mut job = egui::text::LayoutJob::simple_singleline(name, egui::FontId::proportional(12.5), p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 28.0 - cw - 18.0).max(20.0));
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(rect.left() + 28.0, rect.center().y - g.size().y / 2.0), g, p.text);
                    if resp.clicked() {
                        select = Some(*i);
                    }
                }
            });
        });

        // The diff: the file's header (hunks, Open in Editor), the lines, the keys.
        if let Some(f) = r.files.get(r.selected).filter(|_| !r.loading && r.error.is_none()) {
            let head = Rect::from_min_size(diff_rect.min, vec2(diff_rect.width(), 40.0));
            let foot = Rect::from_min_max(pos2(diff_rect.left(), diff_rect.bottom() - 32.0), diff_rect.max);
            let lines_rect = Rect::from_min_max(pos2(diff_rect.left(), head.bottom()), pos2(diff_rect.right(), foot.top()));
            ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.border));
            let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
            let (icon, icolor) = icons::file_icon(&name, false, false, &p);
            let cy = head.center().y;
            ui.painter().text(pos2(head.left() + 14.0 + 7.5, cy), Align2::CENTER_CENTER, icon, style::icon_font(15.0), icolor);
            let mut job = egui::text::LayoutJob::default();
            job.append(&f.path, 0.0, egui::TextFormat::simple(style::medium(13.5), p.text));
            if let Some(a) = f.additions.filter(|a| *a > 0) {
                job.append(&format!("+{a}"), 10.0, egui::TextFormat::simple(egui::FontId::proportional(style::SMALL), p.ok));
            }
            if let Some(d) = f.deletions.filter(|d| *d > 0) {
                job.append(&format!("−{d}"), 4.0, egui::TextFormat::simple(egui::FontId::proportional(style::SMALL), p.err));
            }
            let hunks = r.diff.as_ref().map_or(0, |d| d.hunks.len());
            let cur_hunk = self.review_hunk.min(hunks.saturating_sub(1));
            // Right: hunk navigation and Open in Editor.
            let inner = Rect::from_min_max(pos2(head.left() + 14.0, head.top()), pos2(head.right() - 10.0, head.bottom()));
            let deleted = f.kind == ChangeKind::Deleted;
            let left_end = ui
                .scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if !deleted {
                        let o = kit::button_keys(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), t("Open in Editor"), Some("↵"), true);
                        if o.clicked() {
                            open = true;
                        }
                        kit::vdivider(ui, 18.0);
                    }
                    if hunks > 0 {
                        ui.label(RichText::new(tf("hunk {} of {}", &[&(cur_hunk + 1).to_string(), &hunks.to_string()])).size(style::SMALL).color(p.text_faint));
                        if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::CARET_DOWN), "", true).on_hover_text(t("Next hunk   n")).clicked() {
                            hunk_step = 1;
                        }
                        if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::CARET_UP), "", true).on_hover_text(t("Previous hunk   p")).clicked() {
                            hunk_step = -1;
                        }
                    }
                    ui.min_rect().left()
                })
                .inner;
            job.wrap = egui::text::TextWrapping::truncate_at_width((left_end - 12.0 - (head.left() + 14.0 + 15.0 + 10.0)).max(20.0));
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(head.left() + 14.0 + 15.0 + 10.0, cy - g.size().y / 2.0), g, p.text);

            let syntax = crate::editor::syntax_for(std::path::Path::new(&f.path));
            let dark = ui.visuals().dark_mode;
            let split = self.review_split;
            match (&r.diff, &r.diff_error) {
                (_, Some(e)) => {
                    ui.painter().text(lines_rect.center(), Align2::CENTER_CENTER, e, egui::FontId::proportional(13.0), p.err);
                }
                (None, None) => {
                    ui.painter().text(lines_rect.center(), Align2::CENTER_CENTER, t("Loading the diff…"), egui::FontId::proportional(13.0), p.text_muted);
                }
                (Some(d), None) if d.binary => {
                    ui.painter().text(lines_rect.center(), Align2::CENTER_CENTER, t("Binary file: no text diff."), egui::FontId::proportional(13.0), p.text_muted);
                }
                (Some(d), None) => {
                    let scroll_to = std::mem::take(&mut self.review_scroll).then_some(cur_hunk);
                    ui.scope_builder(egui::UiBuilder::new().max_rect(lines_rect), |ui| {
                        ui.set_clip_rect(lines_rect);
                        egui::ScrollArea::both().id_salt(("review-diff", r.selected)).auto_shrink([false, false]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let w = ui.available_width().max(lines_rect.width());
                            for (hi, h) in d.hunks.iter().enumerate() {
                                let (hr, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
                                if scroll_to == Some(hi) {
                                    ui.scroll_to_rect(hr, Some(egui::Align::Min));
                                }
                                paint_hunk_header(ui, hr, &h.header, &p);
                                if split {
                                    paint_split(ui, w, &h.lines, &syntax, dark, &p);
                                } else {
                                    for l in &h.lines {
                                        let (lr, _) = ui.allocate_exact_size(vec2(w, 19.0), Sense::hover());
                                        if ui.is_rect_visible(lr) {
                                            paint_line(ui, lr, l.kind, l.old_no, l.new_no, &l.text, &syntax, dark, &p, true);
                                        }
                                    }
                                }
                            }
                            ui.add_space(12.0);
                        });
                    });
                }
            }
            // The keys.
            ui.painter().rect_filled(foot, CornerRadius::ZERO, style::over(p.surface, p.chrome, 0.5));
            ui.painter().hline(foot.x_range(), foot.top() + 0.5, Stroke::new(1.0, p.border));
            let mut hints: Vec<(&[&str], &str)> = vec![(&["j", "k"], t("file")), (&["n", "p"], t("hunk")), (&["↵"], t("open in editor"))];
            if has_run {
                hints.push((&["⇥"], t("since start / last commit")));
            }
            let esc = t("back to terminals");
            super::statusbar::paint_key_hints(ui, Rect::from_min_max(pos2(foot.left() + 14.0, foot.top()), pos2(foot.right() - 14.0 - style::text_width(ui, esc) - 6.0, foot.bottom())), &hints, Some("esc"));
            ui.painter().text(pos2(foot.right() - 14.0, foot.center().y), Align2::RIGHT_CENTER, esc, egui::FontId::proportional(style::SMALL), p.text_faint);
            let _ = hunks;
            if hunk_step != 0 && hunks > 0 {
                self.review_hunk = (cur_hunk as i32 + hunk_step).clamp(0, hunks as i32 - 1) as usize;
                self.review_scroll = true;
            }
            if open && !deleted {
                let line = r.diff.as_ref().and_then(|d| d.hunks.get(cur_hunk)).map(|h| h.new_start as usize);
                let path = r.root.join(&f.path);
                self.review = None;
                self.actions.push(Action::OpenFile(path, line));
                return;
            }
        }
        let _ = (adds, dels);
        if let Some(i) = select {
            self.review_hunk = 0;
            self.load_review_diff(i);
        }
        if let Some(b) = switch {
            self.open_review(Some(b));
        }
        if close {
            self.review = None;
            self.focus_request = self.current_pane();
        }
    }

    /// The Review's toolbar: how much changed, Unified | Split, Close.
    pub fn review_toolbar(&mut self, ui: &mut Ui) {
        let Some(r) = self.review.as_ref() else { return };
        let p = style::pal(ui);
        if kit::button_keys(ui, kit::Kind::Ghost, None, t("Close"), Some("esc"), true).clicked() {
            self.review_close = true;
        }
        ui.add_space(4.0);
        let options = [kit::Seg::new(t("Unified"), ""), kit::Seg::new(t("Split"), "")];
        if let Some(i) = kit::segmented(ui, &options, self.review_split as usize) {
            self.review_split = i == 1;
        }
        ui.add_space(8.0);
        if !r.files.is_empty() {
            let (adds, dels): (u32, u32) = r.files.iter().fold((0, 0), |(a, d), f| (a + f.additions.unwrap_or(0), d + f.deletions.unwrap_or(0)));
            let mut job = egui::text::LayoutJob::default();
            let small = egui::FontId::proportional(style::SMALL);
            job.append(&crate::i18n::count(r.files.len(), t("1 file"), t("{} files")), 0.0, egui::TextFormat::simple(small.clone(), p.text_faint));
            job.append(" · ", 0.0, egui::TextFormat::simple(small.clone(), p.text_faint));
            job.append(&format!("+{adds}"), 0.0, egui::TextFormat::simple(small.clone(), p.ok));
            job.append(&format!("−{dels}"), 4.0, egui::TextFormat::simple(small, p.err));
            ui.label(job);
        }
    }
}

/// Width of the Review's file list.
const FILES_W: f32 = 300.0;

fn change_letter(kind: ChangeKind, p: &style::Palette) -> (&'static str, Color32) {
    match kind {
        ChangeKind::Added => ("A", p.ok),
        ChangeKind::Modified => ("M", p.warn),
        ChangeKind::Deleted => ("D", p.err),
        ChangeKind::Renamed => ("R", p.info),
    }
}

/// A hunk's header: its line ranges in the info colour, then the function it is in.
fn paint_hunk_header(ui: &Ui, r: Rect, header: &str, p: &style::Palette) {
    if !ui.is_rect_visible(r) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(r, CornerRadius::ZERO, style::over(p.canvas, p.info, 0.08));
    painter.hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border));
    painter.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border));
    let (ranges, context) = match header.match_indices("@@").nth(1) {
        Some((i, _)) => (&header[..i + 2], header[i + 2..].trim()),
        None => (header, ""),
    };
    let g = painter.layout_no_wrap(ranges.to_string(), egui::FontId::monospace(12.0), p.info);
    let gw = g.size().x;
    painter.galley(pos2(r.left() + 12.0, r.center().y - g.size().y / 2.0), g, p.info);
    if !context.is_empty() {
        painter.text(pos2(r.left() + 12.0 + gw + 10.0, r.center().y), Align2::LEFT_CENTER, context, egui::FontId::proportional(11.5), p.text_muted);
    }
}

/// One line of a diff: its numbers (old, new, or one of them in a split half), its sign, its
/// code in syntax colours, on green or red when added or removed.
#[allow(clippy::too_many_arguments)]
fn paint_line(ui: &Ui, r: Rect, kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str, syntax: &str, dark: bool, p: &style::Palette, both: bool) {
    let painter = ui.painter();
    let (bg, num_bg, sign, sc) = match kind {
        LineKind::Added => (style::soft(p.ok, 0.10), style::soft(p.ok, 0.08), "+", p.ok),
        LineKind::Removed => (style::soft(p.err, 0.10), style::soft(p.err, 0.08), "−", p.err),
        LineKind::Context => (Color32::TRANSPARENT, Color32::TRANSPARENT, "", p.text_faint),
    };
    painter.rect_filled(r, CornerRadius::ZERO, bg);
    let mono = egui::FontId::monospace(12.0);
    let num_w = 44.0;
    let cols = if both { 2.0 } else { 1.0 };
    painter.rect_filled(Rect::from_min_size(r.min, vec2(num_w * cols, r.height())), CornerRadius::ZERO, num_bg);
    let faint = p.text_faint.gamma_multiply(0.8);
    let mut x = r.left();
    let nums: Vec<Option<u32>> = if both { vec![old, new] } else { vec![new.or(old)] };
    for n in nums {
        if let Some(n) = n {
            painter.text(pos2(x + num_w - 8.0, r.center().y), Align2::RIGHT_CENTER, n.to_string(), mono.clone(), faint);
        }
        x += num_w;
    }
    painter.text(pos2(x + 9.0, r.center().y), Align2::CENTER_CENTER, sign, mono.clone(), sc);
    x += 18.0;
    let plain = if kind == LineKind::Context { p.text_muted } else { p.text };
    let mut job = egui::text::LayoutJob::default();
    let text = text.trim_end_matches(['\n', '\r']);
    if kind == LineKind::Context {
        job.append(text, 0.0, egui::TextFormat::simple(mono.clone(), plain));
    } else {
        for (c, piece) in crate::editor::line_colors(text, syntax, dark, plain) {
            job.append(&piece, 0.0, egui::TextFormat::simple(mono.clone(), c));
        }
    }
    let g = painter.layout_job(job);
    painter.galley(pos2(x, r.center().y - g.size().y / 2.0), g, plain);
}

/// A hunk side by side: removed lines at the left beside what replaced them at the right.
fn paint_split(ui: &mut Ui, w: f32, lines: &[tc_git::DiffLine], syntax: &str, dark: bool, p: &style::Palette) {
    let mut rows: Vec<(Option<&tc_git::DiffLine>, Option<&tc_git::DiffLine>)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        match lines[i].kind {
            LineKind::Context => {
                rows.push((Some(&lines[i]), Some(&lines[i])));
                i += 1;
            }
            _ => {
                let dels: Vec<&tc_git::DiffLine> = lines[i..].iter().take_while(|l| l.kind == LineKind::Removed).collect();
                let adds: Vec<&tc_git::DiffLine> = lines[i + dels.len()..].iter().take_while(|l| l.kind == LineKind::Added).collect();
                let n = dels.len().max(adds.len()).max(1);
                for k in 0..n {
                    rows.push((dels.get(k).copied(), adds.get(k).copied()));
                }
                i += (dels.len() + adds.len()).max(1);
            }
        }
    }
    let half = (w / 2.0).floor();
    for (l, rgt) in rows {
        let (row, _) = ui.allocate_exact_size(vec2(w, 19.0), Sense::hover());
        if !ui.is_rect_visible(row) {
            continue;
        }
        let left = Rect::from_min_size(row.min, vec2(half, row.height()));
        let right = Rect::from_min_max(pos2(row.left() + half, row.top()), row.max);
        if let Some(l) = l {
            let kind = if l.kind == LineKind::Context { LineKind::Context } else { LineKind::Removed };
            paint_line(ui, left, kind, l.old_no, None, &l.text, syntax, dark, p, false);
        }
        if let Some(r) = rgt {
            let kind = if r.kind == LineKind::Context { LineKind::Context } else { LineKind::Added };
            paint_line(ui, right, kind, None, r.new_no, &r.text, syntax, dark, p, false);
        }
        ui.painter().vline(right.left(), row.y_range(), Stroke::new(1.0, p.border));
    }
}
