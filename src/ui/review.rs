//! Reviewing what changed (FR-E4, E5): every file changed since the agent run started (a
//! snapshot taken when the agent appeared, outside the user's index) or since the last commit,
//! with the diff of the selected file.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use tc_git::{ChangeKind, FileChange, FileDiff, LineKind};

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::{App, AppMsg};
use crate::i18n::{t, tf};
use tc_core::ProjectId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    /// Since the agent run in this pane started.
    Run,
    /// Since the last commit.
    Head,
    /// A worktree: since its branch left its base (its commits and what is not committed).
    Branch,
}

/// A worktree's branch beside its base, for Review: its commits, which of them last changed each
/// file, and the files not committed yet.
#[derive(Clone, Debug, Default)]
pub struct BranchView {
    pub branch: String,
    pub base: String,
    pub ahead: usize,
    pub behind: usize,
    pub commits: Vec<tc_git::worktree::Commit>,
    pub by_file: HashMap<String, String>,
    pub uncommitted: HashSet<String>,
}

pub struct Review {
    /// The project reviewed (Review follows the project in view).
    pub project: ProjectId,
    pub root: PathBuf,
    pub base: Base,
    /// Agent label and when its run started (for `Base::Run`).
    pub run: Option<(String, Instant, String)>,
    /// A worktree's branch and base (for `Base::Branch`), and what Review found about them.
    pub worktree: Option<(String, String)>,
    pub branch: Option<BranchView>,
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
    /// Drawn over the panes (full screen) instead of in the Code drawer.
    pub expanded: bool,
    /// Opened from an agent's finished run (its Review button, the waiting list): shows what
    /// that run changed, when it changed something.
    pub from_agent: bool,
    /// How many files each base has, shown beside its name.
    pub counts: Vec<(Base, usize)>,
    /// Opened for the agent's run, which changed nothing: the changes since the last commit are
    /// shown instead.
    pub fell_back: bool,
    /// Folders new as a whole (git status lists them as one line), with how many files each
    /// has; each file's folder among them; the folders folded away.
    pub new_dirs: Vec<(String, usize)>,
    pub dir_of: Vec<Option<usize>>,
    pub folded: HashSet<usize>,
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
        branch: Option<Box<BranchView>>,
        /// The base shown (the agent's run may have fallen back to the last commit).
        base: Base,
        counts: Vec<(Base, usize)>,
        fell_back: bool,
    },
    Diff(u64, usize, Result<FileDiff, String>),
}

impl App {
    /// Review follows the project in view: another project chosen while it is open (the
    /// sidebar, ⌘1–9, the palette, a jump to a pane) shows its own changes, as ⇧⌘R would there.
    pub fn review_follow(&mut self) {
        let Some(r) = self.review.as_ref() else { return };
        if Some(r.project) == self.active {
            return;
        }
        self.review_filter.clear();
        self.review_hunk = 0;
        if self.active.is_some() {
            self.open_review(None);
        } else {
            self.review = None;
        }
    }

    /// Opens the review for the current pane: its agent run when there is a snapshot, else
    /// changes since the last commit.
    /// Review of the active project, in the Code drawer (over the panes from the Grid).
    /// `base`: what to compare with; `None` chooses: a worktree since it branched, else since
    /// the last commit (the count "N changed" shows).
    pub fn open_review(&mut self, base: Option<Base>) {
        self.open_review_with(base, false);
    }

    /// As [`App::open_review`]; `from_agent`: opened from an agent's finished run, whose changes
    /// are shown when it made some (else the changes since the last commit, with a note).
    pub fn open_review_with(&mut self, base: Option<Base>, from_agent: bool) {
        let Some(project) = self.active_project() else { return };
        let (folder, name, project_id) = (project.path.clone(), project.name.clone(), project.id);
        let pane = self.current_pane();
        let run = pane.and_then(|id| self.panes.get(&id)).and_then(|p| {
            p.run_snapshot
                .as_ref()
                .map(|(tree, at, agent)| (agent.label().to_string(), *at, tree.clone()))
        });
        let worktree = project.worktree.as_ref().map(|w| (w.branch.clone(), w.base.clone()));
        // Full screen stays full screen; the Grid has no drawer to show it in.
        let expanded = self.view == tc_core::model::View::Grid
            || self.review.as_ref().is_some_and(|r| r.project == project_id && r.expanded);
        if !expanded {
            // The drawer opens as ⌘E opens it (a first time at its usual width).
            self.editors.entry(project_id).or_insert_with(|| crate::editor::Editor { width: 0.54, ..Default::default() }).visible = true;
        }
        let load = next_work();
        self.review = Some(Review {
            project: project_id,
            load,
            diff_load: 0,
            root: folder.clone(),
            base: base.unwrap_or(Base::Head),
            run: run.clone(),
            worktree: worktree.clone(),
            branch: None,
            loading: true,
            from: None,
            to: None,
            files: Vec::new(),
            selected: 0,
            diff: None,
            error: None,
            diff_error: None,
            want: None,
            expanded,
            from_agent,
            counts: Vec::new(),
            fell_back: false,
            new_dirs: Vec::new(),
            dir_of: Vec::new(),
            folded: HashSet::new(),
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
                let head_tree = match &head {
                    Some(h) => h.clone(),
                    None => tc_git::empty_tree(root)?,
                };
                // Every base it can be compared with, counted: each is shown with its number.
                let head_files = tc_git::changes(root, &head_tree, &now)?;
                let run_files = match &run_tree {
                    Some(tree) => Some(tc_git::changes(root, tree, &now)?),
                    None => None,
                };
                let mut branch = None;
                let mut fork = None;
                if let Some((wt_branch, wt_base)) = worktree.clone() {
                    let fork_commit = tc_git::worktree::merge_base(root, &wt_base, "HEAD")?;
                    let (ahead, behind) = tc_git::worktree::ahead_behind(root, &wt_base, "HEAD").unwrap_or((0, 0));
                    branch = Some(BranchView {
                        branch: wt_branch,
                        base: wt_base.clone(),
                        ahead,
                        behind,
                        commits: tc_git::worktree::commits(root, &fork_commit, "HEAD").unwrap_or_default(),
                        by_file: tc_git::worktree::last_commit_by_file(root, &fork_commit, "HEAD").unwrap_or_default(),
                        uncommitted: head_files.iter().map(|c| c.path.clone()).collect(),
                    });
                    let tree = tc_git::worktree::tree_of(root, &fork_commit)?;
                    let files = tc_git::changes(root, &tree, &now)?;
                    fork = Some((tree, files));
                }
                let mut counts = Vec::new();
                if let Some(f) = &run_files {
                    counts.push((Base::Run, f.len()));
                }
                if let Some((_, f)) = &fork {
                    counts.push((Base::Branch, f.len()));
                }
                counts.push((Base::Head, head_files.len()));
                // What is shown: what was asked; else a worktree since it branched; an agent's
                // run when it changed something; else since the last commit.
                let mut fell_back = false;
                let chosen = match base {
                    Some(b) => b,
                    None if fork.is_some() => Base::Branch,
                    None if from_agent && run_files.is_some() => {
                        if run_files.as_ref().is_some_and(|f| f.is_empty()) && !head_files.is_empty() {
                            fell_back = true;
                            Base::Head
                        } else {
                            Base::Run
                        }
                    }
                    None => Base::Head,
                };
                let (from, files) = match chosen {
                    Base::Run => match (run_tree, run_files) {
                        (Some(tree), Some(files)) => (tree, files),
                        _ => return Err(crate::i18n::t("No snapshot of this run.").to_string()),
                    },
                    Base::Branch => match fork {
                        Some(f) => f,
                        None => (head_tree.clone(), head_files),
                    },
                    Base::Head => (head_tree.clone(), head_files),
                };
                let chosen = if chosen == Base::Branch && branch.is_none() { Base::Head } else { chosen };
                Ok::<_, String>((from, now, files, branch, chosen, counts, fell_back))
            })();
            let root = root.unwrap_or(folder);
            let msg = match result {
                Ok((from, to, files, branch, base, counts, fell_back)) => {
                    ReviewMsg::Loaded { load, root, from, to, files: Ok(files), branch: branch.map(Box::new), base, counts, fell_back }
                }
                Err(e) => ReviewMsg::Loaded {
                    load,
                    root,
                    from: String::new(),
                    to: String::new(),
                    files: Err(e),
                    branch: None,
                    base: base.unwrap_or(Base::Head),
                    counts: Vec::new(),
                    fell_back: false,
                },
            };
            let _ = tx.send(AppMsg::Review(msg));
            ctx.request_repaint();
        });
    }

    /// Review covers the panes (full screen): the panes are not what the user looks at.
    pub fn review_full(&self) -> bool {
        self.review.as_ref().is_some_and(|r| r.expanded)
    }

    pub fn on_review_msg(&mut self, msg: ReviewMsg) {
        // Folders git status lists as new as a whole: their files are folded under them.
        let new_folders: Vec<PathBuf> = self
            .review
            .as_ref()
            .and_then(|r| self.project(r.project))
            .and_then(|p| p.git.clone())
            .map(|g| g.changes.iter().filter(|(path, c)| **c == crate::project::Change::Untracked && path.is_dir()).map(|(path, _)| path.clone()).collect())
            .unwrap_or_default();
        let Some(r) = self.review.as_mut() else { return };
        match msg {
            ReviewMsg::Loaded { load, root, from, to, files, branch, base, counts, fell_back } => {
                if load != r.load {
                    return;
                }
                r.loading = false;
                r.root = root;
                r.branch = branch.map(|b| *b);
                r.base = base;
                r.counts = counts;
                r.fell_back = fell_back;
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
                        group_new_folders(r, &new_folders);
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

    /// The bases Review can show, in order (⇥ goes round them): long and short names, count.
    fn review_bases(&self) -> Vec<(Base, String, String, Option<usize>)> {
        let Some(r) = self.review.as_ref() else { return Vec::new() };
        let count = |b: Base| r.counts.iter().find(|(x, _)| *x == b).map(|(_, n)| *n);
        // Since the last commit counts as "N changed" does (git status: a new folder is one).
        let head = self.project(r.project).and_then(|p| p.git.as_ref()).map(|g| g.changes.len()).or(count(Base::Head));
        let mut out = vec![(Base::Head, t("Since last commit").to_string(), t("Last commit").to_string(), head)];
        if let Some((_, base)) = &r.worktree {
            out.push((Base::Branch, tf("Since branched from {}", &[base]), base.clone(), count(Base::Branch)));
        }
        if let Some((agent, at, _)) = &r.run {
            let long = if r.worktree.is_some() {
                tf("Since {} started", &[agent])
            } else {
                tf("Since {} started · {}", &[agent, &super::status::age(*at)])
            };
            out.push((Base::Run, long, agent.clone(), count(Base::Run)));
        }
        out
    }

    /// Review over the panes (full screen; the sidebar stays): the bases in the top bar, the
    /// file list and the selected file's diff. Esc goes back to the Code drawer.
    pub fn review_view(&mut self, ui: &mut Ui) {
        let Some(r) = self.review.as_ref() else { return };
        let p = style::pal(ui);
        let full = ui.max_rect();
        let project = self.active_project().map(|x| x.name.clone()).unwrap_or_default();
        let pane_label = self.current_pane().and_then(|id| self.panes.get(&id)).map(|x| x.label()).unwrap_or_default();
        let base = r.base;
        let bases = self.review_bases();
        let options: Vec<kit::Seg> = bases
            .iter()
            .map(|(b, long, _, n)| {
                let s = kit::Seg::new(long, "");
                let s = if *b == Base::Branch { s.icon(icons::GIT_FORK) } else { s };
                match n {
                    Some(n) => s.count(n, if *n > 0 { p.warn } else { p.text_faint }),
                    None => s,
                }
            })
            .collect();
        let on = bases.iter().position(|(b, ..)| *b == base).unwrap_or(0);
        let width = kit::segmented_width(ui, &options);
        let choices: Vec<Base> = bases.iter().map(|(b, ..)| *b).collect();
        let mut centre = |app: &mut Self, ui: &mut Ui, rect: Rect| {
            #[cfg(debug_assertions)]
            for (r, b) in kit::segment_rects(rect, choices.len()).into_iter().zip(&choices) {
                crate::devtest::record(format!("review-base:{b:?}"), r);
            }
            if let Some(i) = kit::segmented_in(ui, rect, egui::Id::new("review-base"), &options, on) {
                app.review_switch = Some(choices[i]);
            }
        };
        let back = if self.view == tc_core::model::View::Focus { t("Back to the Code drawer") } else { t("Back") };
        let body = self.top_bar(
            ui,
            full,
            |app, ui| {
                if kit::icon_button_ex(ui, icons::ARROW_LEFT, &format!("{back}   Esc"), false, 26.0).clicked() {
                    app.review_close = true;
                }
                ui.label(RichText::new(t("Review")).font(style::semibold(14.0)).color(p.text));
                ui.label(RichText::new(format!("{project} · {pane_label}")).size(12.5).color(p.text_faint));
            },
            Some((width, &mut centre)),
        );
        self.review_body(ui, body, false);
    }

    /// Review in the Code drawer, beside the terminals: Files | Changes and the bases above the
    /// file list, the selected file's diff at its right. Its keys work once it is clicked.
    pub fn review_drawer(&mut self, ui: &mut Ui) {
        let area = ui.max_rect();
        self.review_body(ui, area, true);
    }

    /// The file list and the diff, in `body`: full screen (`drawer` false) or in the drawer.
    /// j / k change file, n / p hunk, ↵ opens it in the editor, ⇥ switches the base, Esc goes
    /// back (to the drawer from full screen, to the terminal from the drawer).
    fn review_body(&mut self, ui: &mut Ui, body: Rect, drawer: bool) {
        let Some(r) = self.review.as_ref() else { return };
        let p = style::pal(ui);
        let who = r.run.as_ref().map(|(agent, _, _)| agent.clone());
        let run_age = r.run.as_ref().map(|(_, at, _)| super::status::age(*at));
        let base = r.base;
        let has_run = r.run.is_some();
        let has_branch = r.worktree.is_some();
        let branch = r.branch.clone();
        let bases = self.review_bases();
        let nfiles = r.files.len();
        let head_count = bases.iter().find(|(b, ..)| *b == Base::Head).and_then(|(.., n)| *n).unwrap_or(0);
        let mut switch = None;
        let files_w = if drawer { (body.width() * 0.42).clamp(200.0, 290.0) } else { FILES_W };

        // ----- keys: full screen always; in the drawer once it was clicked ----------------------
        let keys_id = egui::Id::new("review-keys");
        if drawer {
            let resp = ui.interact(body, keys_id, Sense::click());
            let filter_area = Rect::from_min_max(body.min, pos2(body.left() + files_w, body.top() + DRAWER_HEAD_H + DRAWER_SCOPE_H + 44.0));
            let pressed_here = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pos| body.contains(pos) && !filter_area.contains(pos)));
            if resp.clicked() || pressed_here {
                ui.memory_mut(|m| m.request_focus(keys_id));
            }
        }
        let has_keys = !drawer || ui.memory(|m| m.has_focus(keys_id));
        let filter_focused = ui.ctx().memory(|m| m.focused().is_some_and(|f| f == egui::Id::new("review-filter")));
        let mut close = false;
        let mut expand = false;
        let mut to_terminal = false;
        let mut select: Option<usize> = None;
        let mut hunk_step: i32 = 0;
        let mut open = false;
        if self.dialog.is_none() && self.palette.is_none() && has_keys {
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                    if drawer {
                        to_terminal = true;
                    } else {
                        close = true;
                    }
                }
                if filter_focused {
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
                if i.consume_key(egui::Modifiers::NONE, Key::Tab) && bases.len() > 1 {
                    let at = bases.iter().position(|(b, ..)| *b == base).unwrap_or(0);
                    switch = Some(bases[(at + 1) % bases.len()].0);
                }
            });
        }
        if std::mem::take(&mut self.review_close) {
            close = true;
        }
        if let Some(b) = self.review_switch.take() {
            switch = Some(b);
        }

        // ----- body ---------------------------------------------------------------------------
        let Some(r) = self.review.as_ref() else { return };
        let files_rect = Rect::from_min_max(body.min, pos2(body.left() + files_w, body.bottom()));
        let diff_rect = Rect::from_min_max(pos2(files_rect.right(), body.top()), body.max);
        ui.painter().rect_filled(files_rect, CornerRadius::ZERO, p.chrome);
        ui.painter().vline(files_rect.right() - 0.5, files_rect.y_range(), Stroke::new(1.0, p.border));
        ui.painter().rect_filled(diff_rect, CornerRadius::ZERO, p.canvas);
        let mut list_top = files_rect.top();

        // In the drawer: Files | Changes (Files leaves Review), then the bases with their counts.
        if drawer {
            let head = Rect::from_min_size(files_rect.min, vec2(files_w, DRAWER_HEAD_H));
            ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.border));
            let seg = Rect::from_min_max(pos2(head.left() + 8.0, head.center().y - kit::SEGMENTED_H / 2.0), pos2(head.right() - 7.0, head.center().y + kit::SEGMENTED_H / 2.0));
            let mut options = vec![kit::Seg::new(t("Files"), ""), kit::Seg::new(t("Changes"), "")];
            if head_count > 0 {
                options[1] = options[1].clone().count(head_count, p.warn);
            }
            #[cfg(debug_assertions)]
            for (r, name) in kit::segment_rects(seg, 2).into_iter().zip(["files:files", "files:changes"]) {
                crate::devtest::record(name, r);
            }
            if kit::segmented_in(ui, seg, egui::Id::new("review-files-mode"), &options, 1) == Some(0) {
                close = true;
            }
            let scope = Rect::from_min_size(pos2(files_rect.left(), head.bottom()), vec2(files_w, DRAWER_SCOPE_H));
            if bases.len() > 1 {
                let label = t("Since");
                let g = ui.painter().layout_no_wrap(label.to_uppercase(), style::semibold(10.5), p.text_faint);
                let lw = g.size().x;
                ui.painter().galley(pos2(scope.left() + 10.0, scope.center().y - g.size().y / 2.0), g, p.text_faint);
                let seg = Rect::from_min_max(pos2(scope.left() + 10.0 + lw + 8.0, scope.center().y - kit::SEGMENTED_H / 2.0), pos2(scope.right() - 7.0, scope.center().y + kit::SEGMENTED_H / 2.0));
                let tips: Vec<String> = bases.iter().map(|(_, long, ..)| long.clone()).collect();
                let options: Vec<kit::Seg> = bases
                    .iter()
                    .zip(&tips)
                    .map(|((_, _, short, n), tip)| {
                        let s = kit::Seg::new(short, tip);
                        match n {
                            Some(n) => s.count(n, if *n > 0 { p.warn } else { p.text_faint }),
                            None => s,
                        }
                    })
                    .collect();
                let on = bases.iter().position(|(b, ..)| *b == base).unwrap_or(0);
                #[cfg(debug_assertions)]
                for (r, (b, ..)) in kit::segment_rects(seg, bases.len()).into_iter().zip(&bases) {
                    crate::devtest::record(format!("review-base:{b:?}"), r);
                }
                if let Some(i) = kit::segmented_in(ui, seg, egui::Id::new("review-base-drawer"), &options, on)
                    && bases[i].0 != base
                {
                    switch = Some(bases[i].0);
                }
                list_top = scope.bottom();
            } else {
                list_top = head.bottom();
            }
        }

        // The diff side when there is nothing to show: why.
        let mut show_head = false;
        if r.loading {
            centered(ui, diff_rect, t("Looking at the working tree…"), p.text_muted);
        } else if let Some(e) = &r.error {
            centered(ui, diff_rect, e, p.err);
        } else if r.files.is_empty() {
            match base {
                Base::Run => {
                    let title = tf("{} hasn’t changed any files since it started, {} ago.", &[who.as_deref().unwrap_or(""), run_age.as_deref().unwrap_or("")]);
                    let sub = if head_count > 0 {
                        tf("Its work so far is in the terminal. {} changed since the last commit.", &[&crate::i18n::count(head_count, t("1 file"), t("{} files"))])
                    } else {
                        t("Its work so far is in the terminal.").to_string()
                    };
                    let button = (head_count > 0).then(|| crate::i18n::count(head_count, t("Show the Change"), t("Show the {} Changes")));
                    show_head = empty_state(ui, diff_rect, icons::CHECK_CIRCLE, &title, &sub, button.as_deref(), &p);
                }
                Base::Head => {
                    empty_state(ui, diff_rect, icons::CHECK_CIRCLE, t("No changes since the last commit."), "", None, &p);
                }
                Base::Branch => centered(ui, diff_rect, t("Nothing changed."), p.text_muted),
            }
        }
        if show_head {
            switch = Some(Base::Head);
        }

        // A worktree's branch: where it goes, ahead / behind, its commits.
        if base == Base::Branch
            && let Some(b) = &branch
        {
            let x = files_rect.left() + 14.0;
            let mut y = list_top + 10.0;
            let small = egui::FontId::proportional(style::SMALL);
            let painter = ui.painter();
            painter.text(pos2(x + 6.0, y + 9.0), Align2::CENTER_CENTER, icons::GIT_FORK, style::icon_font(13.0), p.text_muted);
            let mut job = egui::text::LayoutJob::default();
            job.append(&b.branch, 0.0, egui::TextFormat::simple(egui::FontId::monospace(12.5), p.text));
            job.append("→", 6.0, egui::TextFormat::simple(egui::FontId::proportional(12.5), p.text_faint));
            job.append(&b.base, 6.0, egui::TextFormat::simple(egui::FontId::monospace(12.5), p.text));
            let counts = painter.layout_no_wrap(format!("↑{} ↓{}", b.ahead, b.behind), small.clone(), p.text_faint);
            let cw = counts.size().x;
            job.wrap = egui::text::TextWrapping::truncate_at_width((files_rect.right() - 14.0 - cw - 8.0 - (x + 18.0)).max(20.0));
            let g = painter.layout_job(job);
            painter.galley(pos2(x + 18.0, y + 9.0 - g.size().y / 2.0), g, p.text);
            painter.galley(pos2(files_rect.right() - 14.0 - cw, y + 9.0 - counts.size().y / 2.0), counts, p.text_faint);
            y += 18.0 + 6.0;
            const SHOWN: usize = 3;
            for c in b.commits.iter().take(SHOWN) {
                let hash = painter.layout_no_wrap(c.short.clone(), egui::FontId::monospace(11.5), p.text_faint);
                let hw = hash.size().x;
                painter.galley(pos2(x, y + 8.5 - hash.size().y / 2.0), hash, p.text_faint);
                let mut job = egui::text::LayoutJob::simple_singleline(c.subject.clone(), small.clone(), p.text_muted);
                job.wrap = egui::text::TextWrapping::truncate_at_width((files_rect.right() - 14.0 - (x + hw + 7.0)).max(20.0));
                let g = painter.layout_job(job);
                painter.galley(pos2(x + hw + 7.0, y + 8.5 - g.size().y / 2.0), g, p.text_muted);
                y += 17.0;
            }
            if b.commits.len() > SHOWN {
                painter.text(pos2(x, y + 8.5), Align2::LEFT_CENTER, tf("{} more", &[&(b.commits.len() - SHOWN).to_string()]), small.clone(), p.text_faint);
                y += 17.0;
            }
            if b.commits.is_empty() {
                painter.text(pos2(x, y + 8.5), Align2::LEFT_CENTER, t("No commits yet."), small, p.text_faint);
                y += 17.0;
            }
            y += 6.0;
            painter.hline(files_rect.x_range(), y + 0.5, Stroke::new(1.0, p.border));
            list_top = y;
        }

        // Files: a filter, the list, a note on what it is compared with.
        let filter_rect = Rect::from_min_size(pos2(files_rect.left() + 8.0, list_top + 8.0), vec2(files_w - 16.0, 26.0));
        let mut filter = self.review_filter.clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(filter_rect), |ui| {
            kit::field(ui, egui::Id::new("review-filter"), &mut filter, Some(icons::FUNNEL), t("Filter changed files"), filter_rect.width(), true)
        });
        self.review_filter = filter.clone();
        let note_rect = if base == Base::Branch {
            self.review_branch_actions(ui, files_rect, branch.as_ref())
        } else {
            let note = match base {
                Base::Run => tf("Snapshot taken when {} started, {} ago.", &[who.as_deref().unwrap_or(""), run_age.as_deref().unwrap_or("")]),
                _ if self.review.as_ref().is_some_and(|r| r.fell_back) => {
                    tf("{} changed nothing since it started: these are the changes since the last commit.", &[who.as_deref().unwrap_or("")])
                }
                _ => t("Compared with the last commit.").to_string(),
            };
            let note_g = ui.painter().layout(note, egui::FontId::proportional(style::SMALL), p.text_faint, files_w - 28.0);
            let note_rect = Rect::from_min_max(pos2(files_rect.left(), files_rect.bottom() - note_g.size().y - 20.0), files_rect.max);
            ui.painter().hline(note_rect.x_range(), note_rect.top() + 0.5, Stroke::new(1.0, p.border));
            ui.painter().galley(pos2(note_rect.left() + 14.0, note_rect.top() + 10.0), note_g, p.text_faint);
            note_rect
        };
        let Some(r) = self.review.as_ref() else { return };
        let list_rect = Rect::from_min_max(pos2(files_rect.left() + 6.0, filter_rect.bottom() + 8.0), pos2(files_rect.right() - 6.0, note_rect.top()));
        let filtering = !filter.trim().is_empty();
        let shown: Vec<usize> = r
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| !filtering || f.path.to_lowercase().contains(&filter.trim().to_lowercase()))
            .map(|(i, _)| i)
            .collect();
        let mut fold: Option<usize> = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
            ui.set_clip_rect(list_rect);
            egui::ScrollArea::vertical().id_salt("review-files").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut last_dir: Option<usize> = None;
                for i in &shown {
                    let f = &r.files[*i];
                    // A folder new as a whole: one line for it (folded or not), its files under.
                    let dir = if filtering { None } else { r.dir_of.get(*i).copied().flatten() };
                    let indent = if dir.is_some() { 16.0 } else { 0.0 };
                    if let Some(d) = dir
                        && last_dir != Some(d)
                    {
                        let (name, count) = &r.new_dirs[d];
                        let folded = r.folded.contains(&d);
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(5), p.faint_row());
                        }
                        ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), Align2::CENTER_CENTER, if folded { icons::CARET_RIGHT } else { icons::CARET_DOWN }, style::icon_font(10.0), p.text_faint);
                        ui.painter().text(pos2(rect.left() + 26.0, rect.center().y), Align2::CENTER_CENTER, if folded { icons::FOLDER } else { icons::FOLDER_OPEN }, style::icon_font(14.0), p.folder);
                        let g = ui.painter().layout_no_wrap(format!("{name}/"), egui::FontId::proportional(12.5), p.text);
                        let nw = g.size().x.min(rect.width() - 80.0);
                        ui.painter().galley(pos2(rect.left() + 38.0, rect.center().y - g.size().y / 2.0), g, p.text);
                        ui.painter().text(pos2(rect.left() + 38.0 + nw + 6.0, rect.center().y), Align2::LEFT_CENTER, count.to_string(), egui::FontId::proportional(11.0), p.text_faint);
                        ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, "U", style::semibold(11.0), p.ok);
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("review-dir:{name}"), rect);
                        if resp.clicked() {
                            fold = Some(d);
                        }
                    }
                    last_dir = dir;
                    if dir.is_some_and(|d| r.folded.contains(&d)) {
                        continue;
                    }
                    let (letter, color) = change_letter(f.kind, &p);
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("review-file:{}", f.path), rect);
                    if *i == r.selected || resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(5), if *i == r.selected { p.accent_soft } else { p.faint_row() });
                    }
                    ui.painter().text(pos2(rect.left() + indent + 10.0 + 5.0, rect.center().y), Align2::CENTER_CENTER, letter, style::semibold(11.0), color);
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
                    // Under a new folder the name alone (the folder is the line above).
                    let name = match (&f.old_path, dir) {
                        (Some(old), _) => format!("{old} → {}", f.path),
                        (None, Some(d)) => f.path.strip_prefix(&format!("{}/", r.new_dirs[d].0)).unwrap_or(&f.path).to_string(),
                        (None, None) => f.path.clone(),
                    };
                    // A worktree's file not committed yet: a dot after its name.
                    let loose = base == Base::Branch && branch.as_ref().is_some_and(|b| b.uncommitted.contains(&f.path));
                    let dot_w = if loose { 12.0 } else { 0.0 };
                    let mut job = egui::text::LayoutJob::simple_singleline(name, egui::FontId::proportional(12.5), p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - indent - 28.0 - cw - 18.0 - dot_w).max(20.0));
                    let g = ui.painter().layout_job(job);
                    let nw = g.size().x;
                    ui.painter().galley(pos2(rect.left() + indent + 28.0, rect.center().y - g.size().y / 2.0), g, p.text);
                    if loose {
                        ui.painter().circle_filled(pos2(rect.left() + indent + 28.0 + nw + 9.0, rect.center().y), 3.0, p.warn);
                    }
                    if resp.clicked() {
                        select = Some(*i);
                    }
                }
            });
        });

        // The diff: the file's header (hunks, Open in Editor), the lines, the keys.
        if let Some(f) = r.files.get(r.selected).filter(|_| !r.loading && r.error.is_none()) {
            let head = Rect::from_min_size(diff_rect.min, vec2(diff_rect.width(), if drawer { DRAWER_HEAD_H } else { 40.0 }));
            let foot = Rect::from_min_max(pos2(diff_rect.left(), diff_rect.bottom() - 32.0), diff_rect.max);
            let lines_rect = Rect::from_min_max(pos2(diff_rect.left(), head.bottom()), pos2(diff_rect.right(), foot.top()));
            if drawer {
                ui.painter().rect_filled(head, CornerRadius::ZERO, p.chrome);
            }
            ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.border));
            let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
            let (icon, icolor) = icons::file_icon(&name, false, false, &p);
            let cy = head.center().y;
            ui.painter().text(pos2(head.left() + 14.0 + 7.5, cy), Align2::CENTER_CENTER, icon, style::icon_font(15.0), icolor);
            let mut job = egui::text::LayoutJob::default();
            job.append(if drawer { &name } else { &f.path }, 0.0, egui::TextFormat::simple(style::medium(if drawer { 12.5 } else { 13.5 }), p.text));
            if let Some(a) = f.additions.filter(|a| *a > 0) {
                job.append(&format!("+{a}"), 10.0, egui::TextFormat::simple(egui::FontId::proportional(style::SMALL), p.ok));
            }
            if let Some(d) = f.deletions.filter(|d| *d > 0) {
                job.append(&format!("−{d}"), 4.0, egui::TextFormat::simple(egui::FontId::proportional(style::SMALL), p.err));
            }
            // A worktree's file: the commit it was last changed in, else that it is not committed.
            if base == Base::Branch
                && let Some(b) = &branch
            {
                let small = egui::FontId::proportional(style::SMALL);
                if b.uncommitted.contains(&f.path) {
                    job.append(&format!("· {}", t("not committed")), 10.0, egui::TextFormat::simple(small, p.warn));
                } else if let Some(c) = b.by_file.get(&f.path) {
                    job.append(&format!("· {}", tf("committed in {}", &[c])), 10.0, egui::TextFormat::simple(small, p.text_faint));
                }
            }
            let hunks = r.diff.as_ref().map_or(0, |d| d.hunks.len());
            let cur_hunk = self.review_hunk.min(hunks.saturating_sub(1));
            // Right: hunk navigation and Open in Editor (in the drawer also full screen, close).
            let inner = Rect::from_min_max(pos2(head.left() + 14.0, head.top()), pos2(head.right() - if drawer { 6.0 } else { 10.0 }, head.bottom()));
            let deleted = f.kind == ChangeKind::Deleted;
            let left_end = ui
                .scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if drawer {
                        if kit::icon_button_ex(ui, icons::X, &kit::tip(t("Close Review"), Action::ToggleEditor), false, 22.0).clicked() {
                            close = true;
                        }
                        let e = kit::icon_button_ex(ui, icons::ARROWS_OUT_SIMPLE, &kit::tip(t("Full-Screen Review"), Action::Review), false, 22.0);
                        #[cfg(debug_assertions)]
                        crate::devtest::record("review-expand", e.rect);
                        if e.clicked() {
                            expand = true;
                        }
                    }
                    if !deleted {
                        let o = if drawer {
                            kit::button_keys(ui, kit::Kind::Ghost, Some(icons::PENCIL_SIMPLE), t("Open"), Some("↵"), true)
                        } else {
                            kit::button_keys(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), t("Open in Editor"), Some("↵"), true)
                        };
                        if o.clicked() {
                            open = true;
                        }
                        kit::vdivider(ui, 18.0);
                    }
                    if hunks > 0 {
                        if !drawer {
                            ui.label(RichText::new(tf("hunk {} of {}", &[&(cur_hunk + 1).to_string(), &hunks.to_string()])).size(style::SMALL).color(p.text_faint));
                        }
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
            // Two columns need room: the drawer shows them only when it is wide.
            let split = self.review_split && (!drawer || diff_rect.width() >= 900.0);
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
            if drawer {
                // As many as the drawer has room for.
                let w = diff_rect.width();
                hints = vec![(&["j", "k"], t("file"))];
                if w >= 480.0 {
                    hints.push((&["n", "p"], t("hunk")));
                }
                if w >= 600.0 {
                    hints.push((&["↵"], t("open")));
                }
                if w >= 700.0 && bases.len() > 1 {
                    hints.push((&["⇥"], t("since")));
                }
            } else if has_branch && has_run {
                hints.push((&["⇥"], t("started / branched / last commit")));
            } else if has_branch {
                hints.push((&["⇥"], t("branched / last commit")));
            } else if has_run {
                hints.push((&["⇥"], t("since start / last commit")));
            }
            let (right_key, right_text) = if drawer {
                (kit::keys(&Action::Review).unwrap_or_default(), t("full screen"))
            } else if self.view == tc_core::model::View::Focus {
                ("esc", t("back to the drawer"))
            } else {
                ("esc", t("back to terminals"))
            };
            let rw = style::text_width(ui, right_text) + style::text_width(ui, right_key) + 12.0;
            super::statusbar::paint_key_hints(ui, Rect::from_min_max(pos2(foot.left() + 14.0, foot.top()), pos2(foot.right() - 14.0 - rw, foot.bottom())), &hints, None);
            ui.painter().text(pos2(foot.right() - 14.0, foot.center().y), Align2::RIGHT_CENTER, right_text, egui::FontId::proportional(style::SMALL), p.text_faint);
            ui.painter().text(pos2(foot.right() - 14.0 - style::text_width(ui, right_text) - 6.0, foot.center().y), Align2::RIGHT_CENTER, right_key, style::medium(style::SMALL), p.text_faint);
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
        if expand && let Some(r) = self.review.as_mut() {
            r.expanded = true;
        }
        if let Some(d) = fold
            && let Some(r) = self.review.as_mut()
            && !r.folded.remove(&d)
        {
            r.folded.insert(d);
        }
        if let Some(i) = select {
            // A file chosen with j / k inside a folded folder: the folder opens.
            if let Some(r) = self.review.as_mut()
                && let Some(d) = r.dir_of.get(i).copied().flatten()
            {
                r.folded.remove(&d);
            }
            self.review_hunk = 0;
            self.load_review_diff(i);
        }
        if let Some(b) = switch {
            self.open_review(Some(b));
        }
        if to_terminal {
            ui.memory_mut(|m| m.surrender_focus(keys_id));
            self.focus_request = self.current_pane();
        }
        if close {
            // Full screen opened from the Focus view goes back to the drawer; else Review ends.
            let back_to_drawer = !drawer && self.view == tc_core::model::View::Focus;
            match self.review.as_mut() {
                Some(r) if back_to_drawer => r.expanded = false,
                _ => {
                    self.review = None;
                    self.focus_request = self.current_pane();
                }
            }
        }
    }

    /// The foot of a worktree's file list: what Merge leaves out (files not committed), Merge
    /// into its base, Remove. Returns where it is (the list ends above it).
    fn review_branch_actions(&mut self, ui: &mut Ui, files_rect: Rect, branch: Option<&BranchView>) -> Rect {
        let p = style::pal(ui);
        let loose = branch.map_or(0, |b| b.uncommitted.len());
        let base = branch.map(|b| b.base.clone()).unwrap_or_default();
        let width = FILES_W - 24.0;
        let warning = (loose > 0).then(|| {
            let files = crate::i18n::count(loose, t("1 file is not committed"), t("{} files are not committed"));
            let text = tf("{} (●). Merge takes commits only: ask the agent to commit, or commit in the terminal.", &[&files]);
            ui.painter().layout(text, egui::FontId::proportional(style::SMALL), p.warn, width - 20.0)
        });
        let warn_h = warning.as_ref().map_or(0.0, |g| g.size().y + 8.0);
        let rect = Rect::from_min_max(pos2(files_rect.left(), files_rect.bottom() - 10.0 - warn_h - 24.0 - 12.0), files_rect.max);
        ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, p.border));
        let mut y = rect.top() + 10.0;
        if let Some(g) = warning {
            ui.painter().text(pos2(rect.left() + 12.0 + 6.5, y + 8.0), Align2::CENTER_CENTER, icons::WARNING, style::icon_font(12.5), p.warn);
            let h = g.size().y;
            ui.painter().galley(pos2(rect.left() + 12.0 + 20.0, y), g, p.warn);
            y += h + 8.0;
        }
        let row = Rect::from_min_max(pos2(rect.left() + 12.0, y), pos2(rect.right() - 12.0, y + 24.0));
        let id = self.active;
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let merge = kit::button_ex(ui, kit::Kind::Secondary, Some(icons::GIT_MERGE), &tf("Merge into {}…", &[&base]), true);
            #[cfg(debug_assertions)]
            crate::devtest::record("review-merge", merge.rect);
            if merge.clicked()
                && let Some(id) = id
            {
                self.actions.push(Action::MergeWorktree(id));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let remove = kit::button_ex(ui, kit::Kind::Ghost, Some(icons::TRASH), t("Remove…"), true);
                #[cfg(debug_assertions)]
                crate::devtest::record("review-remove", remove.rect);
                if remove.on_hover_text(t("Remove Worktree…")).clicked()
                    && let Some(id) = id
                {
                    self.actions.push(Action::RemoveWorktree(id));
                }
            });
        });
        rect
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
/// In the drawer: the Files | Changes header (as the Files column's) and the bases under it.
const DRAWER_HEAD_H: f32 = 36.0;
const DRAWER_SCOPE_H: f32 = 34.0;

/// A line of text in the middle of `rect`.
fn centered(ui: &Ui, rect: Rect, text: &str, color: Color32) {
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, egui::FontId::proportional(13.0), color);
}

/// Why there is nothing to show: an icon, a sentence, a line under it and maybe a button.
/// Returns whether the button was clicked.
fn empty_state(ui: &mut Ui, rect: Rect, icon: &str, title: &str, sub: &str, button: Option<&str>, p: &style::Palette) -> bool {
    let w = (rect.width() - 48.0).clamp(160.0, 420.0);
    let col = Rect::from_center_size(rect.center(), vec2(w, rect.height()));
    let mut clicked = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(col).layout(egui::Layout::top_down(egui::Align::Center)), |ui| {
        let title_g = ui.painter().layout(title.to_string(), style::semibold(14.0), p.text, w);
        let sub_g = (!sub.is_empty()).then(|| ui.painter().layout(sub.to_string(), egui::FontId::proportional(12.5), p.text_muted, w));
        let h = 28.0 + 10.0 + title_g.size().y + sub_g.as_ref().map_or(0.0, |g| g.size().y + 8.0) + if button.is_some() { 40.0 } else { 0.0 };
        ui.add_space(((rect.height() - h) / 2.0).max(12.0));
        ui.label(RichText::new(icon).font(style::icon_font(28.0)).color(p.text_faint));
        ui.add_space(10.0);
        ui.label(RichText::new(title).font(style::semibold(14.0)).color(p.text));
        if !sub.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(sub).size(12.5).color(p.text_muted));
        }
        if let Some(b) = button {
            ui.add_space(14.0);
            let r = kit::button_keys(ui, kit::Kind::Primary, None, b, Some("⇥"), true);
            #[cfg(debug_assertions)]
            crate::devtest::record("review-show-head", r.rect);
            clicked = r.clicked();
        }
    });
    clicked
}

/// Files under folders new as a whole are listed under one line per folder, folded away except
/// the one holding the selected file.
fn group_new_folders(r: &mut Review, folders: &[PathBuf]) {
    r.new_dirs.clear();
    r.dir_of = vec![None; r.files.len()];
    r.folded.clear();
    for (i, f) in r.files.iter().enumerate() {
        if f.kind != ChangeKind::Added {
            continue;
        }
        let full = r.root.join(&f.path);
        let Some(dir) = folders.iter().filter(|d| full.starts_with(d)).max_by_key(|d| d.as_os_str().len()) else { continue };
        let Ok(rel) = dir.strip_prefix(&r.root) else { continue };
        let name = rel.to_string_lossy().replace('\\', "/");
        let at = match r.new_dirs.iter().position(|(n, _)| *n == name) {
            Some(at) => at,
            None => {
                r.new_dirs.push((name, 0));
                r.new_dirs.len() - 1
            }
        };
        r.new_dirs[at].1 += 1;
        r.dir_of[i] = Some(at);
    }
    let open = r.dir_of.get(r.selected).copied().flatten();
    r.folded = (0..r.new_dirs.len()).filter(|d| Some(*d) != open).collect();
}

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
