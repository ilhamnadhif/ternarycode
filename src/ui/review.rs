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
    /// The commit it branched at (short id).
    pub fork: String,
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
    /// Kept false (Changes always lives inside the docked Code stage; never duplicates as a modal overlay).
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
    /// Files that differ from the last commit: the ones the commit box can take.
    pub uncommitted: HashSet<String>,
    /// The last commit's short id ("Since last commit · HEAD 1a2b3c4").
    pub head_short: Option<String>,
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
        uncommitted: Vec<String>,
        head_short: Option<String>,
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
        if self.view != tc_core::model::View::Focus {
            self.show_focus();
        }
        // Always open docked inside the Code stage (`Files | Changes | History`).
        self.history = None;
        self.drawer_maximized = false;
        self.editors.entry(project_id).or_insert_with(|| crate::editor::Editor { width: 0.54, ..Default::default() }).visible = true;
        let load = next_work();
        self.review_close = false;
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
            expanded: false,
            from_agent,
            counts: Vec::new(),
            fell_back: false,
            new_dirs: Vec::new(),
            dir_of: Vec::new(),
            folded: HashSet::new(),
            uncommitted: HashSet::new(),
            head_short: None,
        });
        let tx = self.app_tx.clone();
        let run_tree = run.map(|(_, _, tree)| tree);
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            // Even finding the repository runs git: not on the UI thread.
            let root = tc_git::repo_root(&folder);
            let head_short = root.as_ref().and_then(|r| tc_git::worktree::commit_line(r, "HEAD")).map(|(short, _)| short);
            let result = (|| {
                let root = root.as_ref().ok_or_else(|| tf("{} is not a git repository.", &[&name]))?;
                let (head, now) = tc_git::worktree_vs_head(root)?;
                let head_tree = match &head {
                    Some(h) => h.clone(),
                    None => tc_git::empty_tree(root)?,
                };
                // Every base it can be compared with, counted: each is shown with its number.
                let head_files = tc_git::changes(root, &head_tree, &now)?;
                let uncommitted: Vec<String> = head_files.iter().map(|c| c.path.clone()).collect();
                let run_files = match &run_tree {
                    Some(tree) => Some(tc_git::changes(root, tree, &now)?),
                    None => None,
                };
                let mut branch = None;
                let mut fork = None;
                if let Some((wt_branch, wt_base)) = worktree.clone() {
                    let fork_commit = tc_git::worktree::merge_base(root, &wt_base, "HEAD")?;
                    let (ahead, behind) = tc_git::worktree::ahead_behind(root, &wt_base, "HEAD").unwrap_or((0, 0));
                    let (fork_short, _) = tc_git::worktree::commit_line(root, &fork_commit).unwrap_or_default();
                    branch = Some(BranchView {
                        fork: fork_short,
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
                Ok::<_, String>((from, now, files, branch, chosen, counts, fell_back, uncommitted))
            })();
            let root = root.unwrap_or(folder);
            let msg = match result {
                Ok((from, to, files, branch, base, counts, fell_back, uncommitted)) => {
                    ReviewMsg::Loaded { load, root, from, to, files: Ok(files), branch: branch.map(Box::new), base, counts, fell_back, uncommitted, head_short }
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
                    uncommitted: Vec::new(),
                    head_short: None,
                },
            };
            let _ = tx.send(AppMsg::Review(msg));
            ctx.request_repaint();
        });
    }

    /// Review always runs docked inside the Code stage (no duplicate full-screen overlay).
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
            ReviewMsg::Loaded { load, root, from, to, files, branch, base, counts, fell_back, uncommitted, head_short } => {
                if load != r.load {
                    return;
                }
                r.loading = false;
                r.head_short = head_short;
                r.uncommitted = uncommitted.into_iter().collect();
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

    /// The bases as Changes shows them, in its order (this run, the branch, the last commit):
    /// what each is, a line under it, and how many files.
    fn review_since(&self) -> Vec<Since> {
        let Some(r) = self.review.as_ref() else { return Vec::new() };
        let bases = self.review_bases();
        [Base::Run, Base::Branch, Base::Head]
            .iter()
            .filter_map(|b| bases.iter().find(|x| x.0 == *b))
            .map(|(b, long, short, count)| {
                let (title, sub, label) = match b {
                    Base::Run => (
                        tf("Since {} started", &[short]),
                        r.run.as_ref().map(|(_, at, _)| tf("{} ago", &[&super::status::age(*at)])).unwrap_or_default(),
                        t("This run").to_string(),
                    ),
                    Base::Branch => (
                        tf("Since branched from {}", &[short]),
                        r.branch.as_ref().map(|v| crate::i18n::count(v.commits.len(), t("1 commit"), t("{} commits"))).unwrap_or_default(),
                        t("Branch").to_string(),
                    ),
                    Base::Head => (
                        t("Since last commit").to_string(),
                        r.head_short.as_ref().map(|s| format!("HEAD {s}")).unwrap_or_default(),
                        t("Last commit").to_string(),
                    ),
                };
                Since { base: *b, title, sub, label, tip: long.clone(), count: *count }
            })
            .collect()
    }

    /// Changes, in the code panel: what the changes are counted since, the files (each in or out
    /// of the commit), the selected file's diff, and the composer that ships them. It lays itself
    /// out for the panel's width: stacked in the panel beside the terminals, and once the panel
    /// is expanded ([`WIDE_W`] and wider) the files in a column at the left, the diff beside them.
    /// j / k change file, n / p hunk, ↵ opens it in the editor, space puts it in or out of the
    /// commit, ⇥ switches the base, Esc returns to the terminal.
    pub fn review_drawer(&mut self, ui: &mut Ui) {
        let body = ui.max_rect();
        let Some(r) = self.review.as_ref() else { return };
        let p = style::pal(ui);
        let wide = body.width() >= WIDE_W;
        let who = r.run.as_ref().map(|(agent, _, _)| agent.clone());
        let run_age = r.run.as_ref().map(|(_, at, _)| super::status::age(*at));
        let base = r.base;
        let branch = r.branch.clone();
        let bases = self.review_bases();
        let since = self.review_since();
        let nfiles = r.files.len();
        let head_count = bases.iter().find(|(b, ..)| *b == Base::Head).and_then(|(.., n)| *n).unwrap_or(0);
        let mut asks = Asks::default();

        // ----- keys: active once the Changes panel is clicked ----------------------------------
        let keys_id = egui::Id::new("review-keys");
        let resp = ui.interact(body, keys_id, Sense::click());
        let filter_area = ui.data(|d| d.get_temp::<Rect>(egui::Id::new("review-filter-rect"))).unwrap_or(Rect::NOTHING);
        let ship_area = ui.data(|d| d.get_temp::<Rect>(egui::Id::new("ship-box-rect"))).unwrap_or(Rect::NOTHING);
        let pressed_here = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pos| body.contains(pos) && !filter_area.contains(pos) && !ship_area.contains(pos)));
        if resp.clicked() || pressed_here {
            ui.memory_mut(|m| m.request_focus(keys_id));
        }
        let has_keys = ui.memory(|m| m.has_focus(keys_id));
        let filter_focused = ui.ctx().memory(|m| m.focused().is_some_and(|f| f == egui::Id::new("review-filter")));
        // Typing in the commit box: Review's keys leave it alone (Esc too).
        let ship_typing = ui.ctx().memory(|m| m.focused().is_some_and(|f| f == App::ship_message_id(r.project) || f == egui::Id::new(("ship-branch", r.project))));
        let mut close = false;
        let mut to_terminal = false;
        if !self.overlay_open() && !self.updater.dialog_open && has_keys && !ship_typing {
            let filter_q = self.review_filter.trim().to_lowercase();
            let visible: Vec<usize> = r
                .files
                .iter()
                .enumerate()
                .filter(|(_, f)| filter_q.is_empty() || f.path.to_lowercase().contains(&filter_q))
                .map(|(i, _)| i)
                .collect();
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                    to_terminal = true;
                }
                if filter_focused {
                    return;
                }
                let sel = r.selected;
                if i.consume_key(egui::Modifiers::NONE, Key::J)
                    && let Some(&next) = visible.iter().find(|&&idx| idx > sel)
                {
                    asks.select = Some(next);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::K)
                    && let Some(&prev) = visible.iter().rev().find(|&&idx| idx < sel)
                {
                    asks.select = Some(prev);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::N) {
                    asks.hunk_step = 1;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::P) {
                    asks.hunk_step = -1;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Enter) {
                    asks.open = true;
                }
                // Space: the file in or out of the commit.
                if i.consume_key(egui::Modifiers::NONE, Key::Space) && sel < nfiles {
                    asks.toggle_in = Some(sel);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Tab) && bases.len() > 1 {
                    let at = bases.iter().position(|(b, ..)| *b == base).unwrap_or(0);
                    asks.switch = Some(bases[(at + 1) % bases.len()].0);
                }
            });
        }
        if std::mem::take(&mut self.review_close) {
            close = true;
        }
        if let Some(b) = self.review_switch.take() {
            asks.switch = Some(b);
        }

        // ----- the files' side: since when, the files, the composer --------------------------
        let Some(r) = self.review.as_ref() else { return };
        let has_box = r.error.is_none();
        let fell_back = r.fell_back;
        let loading = r.loading;
        let (adds, dels): (u32, u32) = r.files.iter().fold((0, 0), |(a, d), f| (a + f.additions.unwrap_or(0), d + f.deletions.unwrap_or(0)));
        let col = if wide { Rect::from_min_max(body.min, pos2(body.left() + FILES_COL_W, body.bottom())) } else { body };
        if wide {
            ui.painter().vline(col.right() - 0.5, col.y_range(), Stroke::new(1.0, p.border));
        }
        let margin = if wide { 4.0 } else { 12.0 };
        let mut top;
        if wide {
            // A well of the bases at the top of the column.
            let (picked, bottom) = since_well(ui, pos2(col.left() + margin, col.top() + 12.0), col.width() - margin * 2.0, &since, base, &p);
            if picked.is_some() {
                asks.switch = picked;
            }
            top = bottom + 10.0;
        } else {
            // A row: the bases as a segmented control, how much changed at the right.
            let row = Rect::from_min_size(pos2(body.left() + 12.0, body.top() + 10.0), vec2(body.width() - 24.0, kit::SEGMENTED_H));
            let labels: Vec<String> = since.iter().map(|s| match s.count {
                Some(n) => format!("{}  {n}", s.label),
                None => s.label.clone(),
            }).collect();
            let options: Vec<kit::Seg> = since
                .iter()
                .zip(&labels)
                .map(|(s, l)| {
                    let seg = kit::Seg::new(l, &s.tip);
                    if s.base == Base::Run { seg.icon(icons::SPARKLE) } else { seg }
                })
                .collect();
            let on = since.iter().position(|s| s.base == base).unwrap_or(0);
            let seg_right = ui
                .scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                    let at = ui.cursor().min.x;
                    if let Some(i) = kit::segmented(ui, &options, on) {
                        asks.switch = since.get(i).map(|s| s.base);
                    }
                    #[cfg(debug_assertions)]
                    {
                        let mut x = at + 2.0;
                        for (s, l) in since.iter().zip(&labels) {
                            let w = ui.painter().layout_no_wrap(l.clone(), style::medium(12.0), Color32::PLACEHOLDER).size().x
                                + if s.base == Base::Run { 13.0 + 5.0 } else { 0.0 }
                                + 20.0;
                            crate::devtest::record(format!("review-base:{:?}", s.base), Rect::from_min_size(pos2(x, row.top() + 2.0), vec2(w, row.height() - 4.0)));
                            x += w + 2.0;
                        }
                    }
                    let _ = at;
                    ui.min_rect().right()
                })
                .inner;
            if !loading && has_box {
                let text = format!("{} · +{adds} −{dels}", crate::i18n::count(nfiles, t("1 file"), t("{} files")));
                let g = ui.painter().layout_no_wrap(text, style::mono(style::DATA), p.text_faint);
                if seg_right + 12.0 + g.size().x <= row.right() {
                    ui.painter().galley(pos2(row.right() - g.size().x, row.center().y - g.size().y / 2.0), g, p.text_faint);
                }
            }
            top = row.bottom() + 4.0;
        }

        // A worktree's branch summary above its files: where it goes, ahead / behind, its last commits.
        if base == Base::Branch
            && let Some(b) = &branch
        {
            top = branch_summary(ui, col, top + 4.0, b, &p);
        }

        // A filter over the files: always when expanded, else once there are many.
        let mut filter = self.review_filter.clone();
        if wide || nfiles > FILTER_FROM || !filter.is_empty() || filter_focused {
            let filter_rect = Rect::from_min_size(pos2(col.left() + margin, top), vec2(col.width() - margin * 2.0, 28.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(filter_rect), |ui| {
                kit::filter_field(ui, egui::Id::new("review-filter"), &mut filter, Some(icons::FUNNEL), t("Filter changed files"), filter_rect.width())
            });
            self.review_filter = filter.clone();
            top = filter_rect.bottom() + if wide { 10.0 } else { 4.0 };
            ui.data_mut(|d| d.insert_temp(egui::Id::new("review-filter-rect"), filter_rect));
        } else {
            ui.data_mut(|d| d.insert_temp(egui::Id::new("review-filter-rect"), Rect::NOTHING));
        }

        // The foot: a worktree's Merge and Remove at the bottom, the composer above them; with an
        // error and no composer, how much changed and against what.
        let mut foot_top = col.bottom();
        if base == Base::Branch && branch.is_some() {
            foot_top = self.review_branch_actions(ui, col, branch.as_ref()).top();
        }
        if has_box {
            // The composer may take all but two file rows (and, stacked, a little of the diff).
            let limit = (top + 2.0 * (FILE_ROW_H + 1.0) + if wide { 0.0 } else { 120.0 }).min(foot_top);
            foot_top = self.ship_box(ui, Rect::from_min_max(pos2(col.left(), limit), pos2(col.right(), foot_top)));
        } else if !(base == Base::Branch && branch.is_some()) {
            let note = match base {
                _ if fell_back => String::new(),
                Base::Run => tf("since {} started", &[who.as_deref().unwrap_or("")]),
                _ => t("not committed").to_string(),
            };
            let foot = Rect::from_min_max(pos2(col.left(), col.bottom() - COL_FOOT_H), pos2(col.right(), foot_top));
            let painter = ui.painter();
            painter.hline(foot.x_range(), foot.top() + 0.5, Stroke::new(1.0, p.border));
            let mut job = egui::text::LayoutJob::default();
            job.append(&format!("+{adds}"), 0.0, egui::TextFormat::simple(style::mono(style::DATA), p.ok));
            job.append(&format!("−{dels}"), 6.0, egui::TextFormat::simple(style::mono(style::DATA), p.err));
            if !note.is_empty() {
                job.append("·", 6.0, egui::TextFormat::simple(egui::FontId::proportional(style::SMALL), p.text_ghost));
                job.append(&note, 6.0, egui::TextFormat::simple(egui::FontId::proportional(style::SMALL), p.text_faint));
            }
            job.wrap = egui::text::TextWrapping::truncate_at_width(foot.width() - 24.0);
            let g = painter.layout_job(job);
            painter.galley(pos2(foot.left() + 12.0, foot.center().y - g.size().y / 2.0), g, p.text_faint);
            foot_top = foot.top();
        }
        // The agent's run changed nothing: why these are the changes since the last commit.
        if fell_back {
            let text = tf("{} changed nothing since it started: these are the changes since the last commit.", &[who.as_deref().unwrap_or("")]);
            let g = ui.painter().layout(text, egui::FontId::proportional(style::SMALL), p.text_faint, col.width() - 28.0);
            let note_top = foot_top - g.size().y - 20.0;
            if note_top > top + 40.0 {
                ui.painter().hline(col.x_range(), note_top + 0.5, Stroke::new(1.0, p.border));
                ui.painter().galley(pos2(col.left() + 14.0, note_top + 10.0), g, p.text_faint);
                foot_top = note_top;
            }
        }

        // The files, then (stacked) the diff under them.
        let Some(r) = self.review.as_ref() else { return };
        let rows = file_row_count(r, &filter);
        let list = if wide {
            Rect::from_min_max(pos2(col.left() + margin, top), pos2(col.right() - margin, foot_top.max(top)))
        } else if rows == 0 {
            Rect::from_min_size(pos2(col.left() + 6.0, top), vec2(col.width() - 12.0, 0.0))
        } else {
            let natural = rows as f32 * (FILE_ROW_H + 1.0) - 1.0;
            let room = (foot_top - top - 12.0).max(0.0);
            let h = natural.min((room * 0.42).max(room.min(2.0 * (FILE_ROW_H + 1.0)))).max(0.0);
            Rect::from_min_size(pos2(col.left() + 6.0, top + 6.0), vec2(col.width() - 12.0, h))
        };
        if rows > 0 {
            self.review_file_rows(ui, list, &filter, has_box, &mut asks);
        }
        let diff_rect = if wide {
            Rect::from_min_max(pos2(col.right(), body.top()), body.max)
        } else {
            let y = if rows > 0 { list.bottom() + 6.0 } else { top };
            if rows > 0 {
                ui.painter().hline(body.x_range(), y + 0.5, Stroke::new(1.0, p.border));
            }
            Rect::from_min_max(pos2(body.left(), y + if rows > 0 { 1.0 } else { 0.0 }), pos2(body.right(), foot_top.max(y)))
        };

        // The diff side when there is nothing to show: why.
        let Some(r) = self.review.as_ref() else { return };
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
                    show_head = empty_state(ui, diff_rect, &title, &sub, button.as_deref());
                }
                Base::Head => {
                    empty_state(ui, diff_rect, t("No changes since the last commit."), "", None);
                }
                Base::Branch => {
                    empty_state(ui, diff_rect, t("Nothing changed."), "", None);
                }
            }
        } else if r.files.get(r.selected).is_some() {
            self.review_diff(ui, diff_rect, wide, has_box, bases.len(), &mut asks);
        }
        if show_head {
            asks.switch = Some(Base::Head);
        }

        // ----- what was asked -----------------------------------------------------------------
        if let Some(i) = asks.split {
            self.review_split = i;
        }
        let hunks = self.review.as_ref().and_then(|r| r.diff.as_ref()).map_or(0, |d| d.hunks.len());
        let cur_hunk = self.review_hunk.min(hunks.saturating_sub(1));
        if asks.hunk_step != 0 && hunks > 0 {
            self.review_hunk = (cur_hunk as i32 + asks.hunk_step).clamp(0, hunks as i32 - 1) as usize;
            self.review_scroll = true;
        }
        if asks.open
            && let Some(r) = self.review.as_ref()
            && let Some(f) = r.files.get(r.selected).filter(|f| f.kind != ChangeKind::Deleted && !r.loading && r.error.is_none())
        {
            let line = r.diff.as_ref().and_then(|d| d.hunks.get(cur_hunk)).map(|h| h.new_start as usize);
            let path = r.root.join(&f.path);
            self.review = None;
            self.actions.push(Action::OpenFile(path, line));
            return;
        }
        if let Some(d) = asks.fold
            && let Some(r) = self.review.as_mut()
            && !r.folded.remove(&d)
        {
            r.folded.insert(d);
        }
        if let Some(i) = asks.toggle_in
            && let Some(r) = self.review.as_ref()
            && let Some(f) = r.files.get(i)
            && r.uncommitted.contains(&f.path)
        {
            let (project, path) = (r.project, f.path.clone());
            let s = self.ship_mut(project);
            if !s.out.remove(&path) {
                s.out.insert(path);
            }
        }
        if let Some(i) = asks.select {
            // A file chosen with j / k inside a folded folder: the folder opens.
            if let Some(r) = self.review.as_mut()
                && let Some(d) = r.dir_of.get(i).copied().flatten()
            {
                r.folded.remove(&d);
            }
            self.review_hunk = 0;
            self.load_review_diff(i);
        }
        if let Some(b) = asks.switch {
            self.open_review(Some(b));
        }
        if to_terminal {
            ui.memory_mut(|m| m.surrender_focus(keys_id));
            self.focus_request = self.current_pane();
        }
        if close {
            self.review = None;
            self.focus_request = self.current_pane();
        }
    }

    /// The changed files as rows in `rect` (scrolling), each with its box (in or out of the
    /// commit), change, name and folder, the agent that changed it, + and −. A folder new as a
    /// whole is one row its files fold under.
    fn review_file_rows(&self, ui: &mut Ui, rect: Rect, filter: &str, has_box: bool, asks: &mut Asks) {
        let Some(r) = self.review.as_ref() else { return };
        let p = style::pal(ui);
        let base = r.base;
        let q = filter.trim().to_lowercase();
        let filtering = !q.is_empty();
        let shown: Vec<usize> = r.files.iter().enumerate().filter(|(_, f)| !filtering || f.path.to_lowercase().contains(&q)).map(|(i, _)| i).collect();
        let run_kind = r.run.as_ref().and_then(|(a, _, _)| tc_core::agent::AgentKind::ALL.iter().copied().find(|k| k.label() == a));
        let touched = self.agent_touched.get(&r.project);
        // The commit box takes the files not committed yet: a box before each says in or out.
        let ship_out = self.ship.get(&r.project).map(|s| &s.out);
        let cb_col = has_box && r.files.iter().any(|f| r.uncommitted.contains(&f.path));
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical().id_salt("review-files").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let mut last_dir: Option<usize> = None;
                for i in &shown {
                    let f = &r.files[*i];
                    // A folder new as a whole: one row for it (folded or not), its files under.
                    let dir = if filtering { None } else { r.dir_of.get(*i).copied().flatten() };
                    let indent = if dir.is_some() { 14.0 } else { 0.0 };
                    if let Some(d) = dir
                        && last_dir != Some(d)
                    {
                        let (name, count) = &r.new_dirs[d];
                        let folded = r.folded.contains(&d);
                        let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), FILE_ROW_H), Sense::click());
                        paint_folder_row(ui.painter(), row, name, *count, folded, resp.hovered(), cb_col, &p);
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("review-dir:{name}"), row);
                        if resp.clicked() {
                            asks.fold = Some(d);
                        }
                    }
                    last_dir = dir;
                    if dir.is_some_and(|d| r.folded.contains(&d)) {
                        continue;
                    }
                    let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), FILE_ROW_H), Sense::click());
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("review-file:{}", f.path), row);
                    let mut check = None;
                    if cb_col && r.uncommitted.contains(&f.path) {
                        let cb = checkbox_rect(row, indent);
                        let cb_resp = ui.interact(cb.expand(4.0), egui::Id::new(("review-in", &f.path)), Sense::click());
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("review-in:{}", f.path), cb);
                        let in_commit = ship_out.is_none_or(|o| !o.contains(&f.path));
                        let hovered = cb_resp.hovered();
                        if cb_resp.on_hover_text(if in_commit { t("In the commit: click to leave it out") } else { t("Left out of the commit: click to put it in") }).clicked() {
                            asks.toggle_in = Some(*i);
                        }
                        check = Some((in_commit, hovered));
                    }
                    // The name, and its folder after it (under a new folder: within it).
                    let rel = match dir {
                        Some(d) => f.path.strip_prefix(&format!("{}/", r.new_dirs[d].0)).unwrap_or(&f.path).to_string(),
                        None => f.path.clone(),
                    };
                    let (folder, name) = match rel.rsplit_once('/') {
                        Some((d, n)) => (d.to_string(), n.to_string()),
                        None => (String::new(), rel.clone()),
                    };
                    let sub = match &f.old_path {
                        Some(old) => format!("← {old}"),
                        None => folder,
                    };
                    let agent = touched
                        .and_then(|t| t.get(&r.root.join(&f.path)).copied())
                        .or((base == Base::Run).then_some(run_kind).flatten())
                        .filter(|k| super::agent_logo::has(*k));
                    // A worktree's file not committed yet: a dot after its name.
                    let loose = base == Base::Branch && r.branch.as_ref().is_some_and(|b| b.uncommitted.contains(&f.path));
                    paint_file_row(
                        ui.painter(),
                        row,
                        &FileRow {
                            kind: f.kind,
                            name: &name,
                            sub: &sub,
                            adds: f.additions,
                            dels: f.deletions,
                            binary: f.binary,
                            check,
                            box_slot: cb_col,
                            agent,
                            selected: *i == r.selected,
                            hovered: resp.hovered(),
                            loose,
                            indent,
                        },
                        &p,
                    );
                    if resp.clicked() {
                        asks.select = Some(*i);
                    }
                }
            });
        });
    }

    /// The selected file's diff in `rect`: its head (change, path, + −, the hunk shown, Open),
    /// the names of Split's sides, the hunks; expanded, Unified | Split and the keys at the foot.
    fn review_diff(&mut self, ui: &mut Ui, rect: Rect, wide: bool, has_box: bool, nbases: usize, asks: &mut Asks) {
        let p = style::pal(ui);
        let Some(r) = self.review.as_ref() else { return };
        let Some(f) = r.files.get(r.selected) else { return };
        let base = r.base;
        let deleted = f.kind == ChangeKind::Deleted;
        let hunks = r.diff.as_ref().map_or(0, |d| d.hunks.len());
        let cur_hunk = self.review_hunk.min(hunks.saturating_sub(1));
        let split = wide && self.review_split;
        let head = Rect::from_min_size(rect.min, vec2(rect.width(), if wide { 44.0 } else { 36.0 }));
        let hints_h = if wide { HINTS_H } else { 0.0 };
        let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - hints_h), rect.max);
        let sides = Rect::from_min_size(pos2(rect.left(), head.bottom()), vec2(rect.width(), if split { SIDES_H } else { 0.0 }));
        let lines_rect = Rect::from_min_max(pos2(rect.left(), sides.bottom()), pos2(rect.right(), foot.top()));
        ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.border));
        let cy = head.center().y;

        // Right: Open, the hunk shown, Unified | Split (expanded), where the file is from (a worktree).
        let inner = Rect::from_min_max(pos2(head.left() + 12.0, head.top()), pos2(head.right() - if wide { 12.0 } else { 8.0 }, head.bottom()));
        let review_split = self.review_split;
        let left_end = ui
            .scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = if wide { 10.0 } else { 8.0 };
                if !deleted {
                    let open = if wide {
                        kit::button_ex(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), t("Open in editor"), true)
                    } else {
                        kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROW_SQUARE_OUT), t("Open"), true)
                    };
                    #[cfg(debug_assertions)]
                    crate::devtest::record("review-open", open.rect);
                    if open.on_hover_text(format!("{}   ↵", t("Open in editor"))).clicked() {
                        asks.open = true;
                    }
                }
                if hunks > 1 {
                    ui.label(RichText::new(tf("hunk {} of {}", &[&(cur_hunk + 1).to_string(), &hunks.to_string()])).font(style::mono(style::DATA)).color(p.text_faint));
                }
                if wide {
                    let options = [kit::Seg::new(t("Unified"), ""), kit::Seg::new(t("Split"), "")];
                    let w = kit::segmented_width(ui, &options);
                    let (sr, _) = ui.allocate_exact_size(vec2(w, kit::SEGMENTED_H), Sense::hover());
                    #[cfg(debug_assertions)]
                    for (r, name) in kit::segment_rects(sr, 2).into_iter().zip(["review-unified", "review-split"]) {
                        crate::devtest::record(name, r);
                    }
                    if let Some(i) = kit::segmented_in(ui, sr, egui::Id::new("review-split-drawer"), &options, review_split as usize) {
                        asks.split = Some(i == 1);
                    }
                    if base == Base::Branch
                        && let Some(b) = &r.branch
                    {
                        if b.uncommitted.contains(&f.path) {
                            ui.label(RichText::new(t("not committed")).size(style::SMALL).color(p.warn));
                        } else if let Some(c) = b.by_file.get(&f.path) {
                            ui.label(RichText::new(tf("in {}", &[c])).font(style::mono(style::DATA)).color(p.text_faint));
                        }
                    }
                }
                ui.min_rect().left()
            })
            .inner;

        // Left: the change, the path, + − and (expanded) their share.
        let (letter, lcolor) = change_letter(f.kind, &p);
        let painter = ui.painter();
        let gap = if wide { 10.0 } else { 8.0 };
        let lg = painter.layout_no_wrap(letter.to_string(), style::mono_semibold(style::DATA), lcolor);
        let mut x = head.left() + 12.0;
        let lw = lg.size().x;
        painter.galley(pos2(x, cy - lg.size().y / 2.0), lg, lcolor);
        x += lw + gap;
        let a = f.additions.unwrap_or(0);
        let d = f.deletions.unwrap_or(0);
        let counts = (!f.binary).then(|| painter.layout_no_wrap(format!("+{a} −{d}"), style::mono(style::DATA), p.text_faint));
        let cw = counts.as_ref().map_or(0.0, |g| g.size().x + gap);
        let bars_w = if wide && !f.binary { BARS_W + gap } else { 0.0 };
        let path = match &f.old_path {
            Some(old) => format!("{old} → {}", f.path),
            None => f.path.clone(),
        };
        let avail = (left_end - 12.0 - x).max(0.0);
        let show_bars = bars_w > 0.0 && avail >= cw + bars_w + 80.0;
        let show_counts = avail >= cw + 60.0;
        let reserved = if show_bars { cw + bars_w } else if show_counts { cw } else { 0.0 };
        let mut job = egui::text::LayoutJob::simple_singleline(path, style::mono(style::MONO), p.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((avail - reserved).max(16.0));
        let g = painter.layout_job(job);
        let gw = g.size().x;
        let head_clip = painter.with_clip_rect(Rect::from_min_max(pos2(x, head.top()), pos2(left_end.max(x), head.bottom())));
        head_clip.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
        x += gw + gap;
        if show_counts && let Some(c) = counts {
            let w = c.size().x;
            head_clip.galley(pos2(x, cy - c.size().y / 2.0), c, p.text_faint);
            x += w + gap;
        }
        if show_bars {
            paint_bars(&head_clip, pos2(x, cy), a, d, &p);
        }

        // Split: what each side is.
        if split {
            let half = (sides.width() / 2.0).floor();
            let (l, rt) = side_names(base, r.branch.as_ref(), r.run.as_ref().map(|(a, ..)| a.as_str()));
            painter.hline(sides.x_range(), sides.bottom() - 0.5, Stroke::new(1.0, p.border));
            let font = style::medium(style::SMALL);
            for (text, x0, w) in [(l, sides.left(), half), (rt, sides.left() + half, sides.width() - half)] {
                let mut job = egui::text::LayoutJob::simple_singleline(text, font.clone(), p.text_muted);
                job.wrap = egui::text::TextWrapping::truncate_at_width((w - 24.0).max(10.0));
                let g = painter.layout_job(job);
                painter.galley(pos2(x0 + 12.0, sides.center().y - g.size().y / 2.0), g, p.text_muted);
            }
        }

        // The hunks.
        match (&r.diff, &r.diff_error) {
            (_, Some(e)) => centered(ui, lines_rect, e, p.err),
            (None, None) => centered(ui, lines_rect, t("Loading the diff…"), p.text_muted),
            (Some(d), None) if d.binary => centered(ui, lines_rect, t("Binary file: no text diff."), p.text_muted),
            (Some(d), None) => {
                let scroll_to = std::mem::take(&mut self.review_scroll).then_some(cur_hunk);
                let step = diff_view(ui, lines_rect, ("review-diff", r.selected), d, &f.path, split, Some(cur_hunk), scroll_to);
                if step != 0 {
                    asks.hunk_step = step;
                }
            }
        }

        // Expanded: the keys at the foot.
        if wide {
            ui.painter().hline(foot.x_range(), foot.top() + 0.5, Stroke::new(1.0, p.border));
            let mut hints: Vec<(&str, &str)> = vec![("j k", t("file")), ("n p", t("hunk"))];
            if has_box && !r.uncommitted.is_empty() {
                hints.push(("space", t("in commit")));
            }
            if !deleted {
                hints.push(("↵", t("open")));
            }
            if nbases > 1 {
                hints.push(("⇥", t("since")));
            }
            let clip = ui.painter().with_clip_rect(foot.intersect(ui.clip_rect()));
            paint_hints(&clip, pos2(foot.left() + 12.0, foot.center().y), &hints, &p);
        }
    }

    /// The foot of a worktree's file list: Merge into its base, Remove. Returns where it is (the
    /// list ends above it).
    fn review_branch_actions(&mut self, ui: &mut Ui, col: Rect, branch: Option<&BranchView>) -> Rect {
        // What is not committed is the composer's (above): here Merge and Remove only.
        let p = style::pal(ui);
        let base = branch.map(|b| b.base.clone()).unwrap_or_default();
        let rect = Rect::from_min_max(pos2(col.left(), col.bottom() - 10.0 - 24.0 - 10.0), col.max);
        ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, p.border));
        let y = rect.top() + 10.0;
        let row = Rect::from_min_max(pos2(rect.left() + 14.0, y), pos2(rect.right() - 14.0, y + 24.0));
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
}

/// From this width on, Changes (and History) lay themselves out expanded: a column at the left,
/// the diff beside it; narrower, they stack.
pub const WIDE_W: f32 = 900.0;
/// The expanded panel's column of files.
const FILES_COL_W: f32 = 380.0;
/// From this many changed files on, a filter over them (always, expanded).
const FILTER_FROM: usize = 12;
/// The column's foot without a composer (how much changed), a file's row, the expanded diff's
/// key hints and the names of Split's sides.
const COL_FOOT_H: f32 = 30.0;
pub(super) const FILE_ROW_H: f32 = 30.0;
const HINTS_H: f32 = 36.0;
const SIDES_H: f32 = 30.0;
/// A hunk's header.
pub(super) const HUNK_H: f32 = 28.0;
/// The five boxes that show the share of added and removed lines.
pub(super) const BARS_W: f32 = 5.0 * 6.0 + 4.0 * 2.0;

/// What a frame of Changes asked for (done once drawing is over).
#[derive(Default)]
struct Asks {
    select: Option<usize>,
    toggle_in: Option<usize>,
    fold: Option<usize>,
    switch: Option<Base>,
    hunk_step: i32,
    open: bool,
    split: Option<bool>,
}

/// A base as Changes shows it: the well's title and line, the segment's word, a tooltip, files.
struct Since {
    base: Base,
    title: String,
    sub: String,
    label: String,
    tip: String,
    count: Option<usize>,
}

/// How long ago a Unix time was ("6 min", "2 h", "3 d").
pub(super) fn unix_age(t: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let s = (now - t).max(0);
    if s < 60 {
        tf("{} s", &[&s.to_string()])
    } else if s < 3600 {
        tf("{} min", &[&(s / 60).to_string()])
    } else if s < 86_400 {
        tf("{} h", &[&(s / 3600).to_string()])
    } else {
        tf("{} d", &[&(s / 86_400).to_string()])
    }
}

/// The bases as a well (the expanded panel's column): an icon, what it is with a mono line
/// under it, how many files; the one shown raised. Returns the base clicked and the well's
/// bottom.
fn since_well(ui: &mut Ui, at: egui::Pos2, width: f32, opts: &[Since], on: Base, p: &style::Palette) -> (Option<Base>, f32) {
    const ROW: f32 = 48.0;
    let n = opts.len() as f32;
    let well = Rect::from_min_size(at, vec2(width, 8.0 + n * ROW + (n - 1.0).max(0.0) * 2.0));
    ui.painter().rect_filled(well, CornerRadius::same(style::RADIUS_CARD), p.inset);
    let mut picked = None;
    let mut y = well.top() + 4.0;
    for s in opts {
        let row = Rect::from_min_size(pos2(well.left() + 4.0, y), vec2(well.width() - 8.0, ROW));
        y += ROW + 2.0;
        let sel = s.base == on;
        let resp = ui.interact(row, egui::Id::new(("review-scope", s.base as u8)), Sense::click());
        #[cfg(debug_assertions)]
        crate::devtest::record(format!("review-base:{:?}", s.base), row);
        let painter = ui.painter();
        if sel {
            painter.rect(row, CornerRadius::same(8), p.surface, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        } else if resp.hovered() {
            painter.rect_filled(row, CornerRadius::same(8), p.hover);
        }
        let icon = match s.base {
            Base::Run => icons::SPARKLE,
            Base::Branch => icons::GIT_BRANCH,
            Base::Head => icons::GIT_COMMIT,
        };
        painter.text(pos2(row.left() + 10.0 + 7.0, row.center().y), Align2::CENTER_CENTER, icon, style::icon_font(14.0), if sel { p.accent_text } else { p.text_muted });
        let count = s.count.map(|c| painter.layout_no_wrap(c.to_string(), style::mono(style::DATA), p.text_faint));
        let cw = count.as_ref().map_or(0.0, |g| g.size().x + 10.0);
        let x = row.left() + 10.0 + 14.0 + 10.0;
        let w = (row.right() - 10.0 - cw - x).max(20.0);
        let (font, color) = if sel { (style::medium(style::SMALL), p.text) } else { (egui::FontId::proportional(style::SMALL), p.text_muted) };
        let mut job = egui::text::LayoutJob::simple_singleline(s.title.clone(), font, color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(w);
        let g = painter.layout_job(job);
        let has_sub = !s.sub.is_empty();
        let ty = if has_sub { row.top() + 8.0 } else { row.center().y - g.size().y / 2.0 };
        painter.galley(pos2(x, ty), g, color);
        if has_sub {
            let mut job = egui::text::LayoutJob::simple_singleline(s.sub.clone(), style::mono(style::DATA), p.text_faint);
            job.wrap = egui::text::TextWrapping::truncate_at_width(w);
            let g = painter.layout_job(job);
            painter.galley(pos2(x, row.top() + 8.0 + 16.0 + 2.0), g, p.text_faint);
        }
        if let Some(g) = count {
            painter.galley(pos2(row.right() - 10.0 - g.size().x, row.center().y - g.size().y / 2.0), g, p.text_faint);
        }
        let resp = resp.on_hover_text(&s.tip);
        if resp.hovered() && !sel {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() && !sel {
            picked = Some(s.base);
        }
    }
    (picked, well.bottom())
}

/// The keys of the expanded diff's foot: each key cap, then what it does; returns where they end.
fn paint_hints(painter: &egui::Painter, left_center: egui::Pos2, hints: &[(&str, &str)], p: &style::Palette) -> f32 {
    let mut x = left_center.x;
    for (keys, what) in hints {
        x += kit::paint_kbd_text(painter, pos2(x, left_center.y), keys, p) + 6.0;
        let g = painter.layout_no_wrap(what.to_string(), egui::FontId::proportional(style::SMALL), p.text_faint);
        let w = g.size().x;
        painter.galley(pos2(x, left_center.y - g.size().y / 2.0), g, p.text_faint);
        x += w + 14.0;
    }
    x
}

/// How many rows the file list has (a folder new as a whole is one, its files when open).
fn file_row_count(r: &Review, filter: &str) -> usize {
    let q = filter.trim().to_lowercase();
    if !q.is_empty() {
        return r.files.iter().filter(|f| f.path.to_lowercase().contains(&q)).count();
    }
    let dirs = r.new_dirs.len();
    let files = r.files.iter().enumerate().filter(|(i, _)| r.dir_of.get(*i).copied().flatten().is_none_or(|d| !r.folded.contains(&d))).count();
    dirs + files
}

/// The box of a file row (in or out of the commit).
fn checkbox_rect(row: Rect, indent: f32) -> Rect {
    Rect::from_center_size(pos2(row.left() + 10.0 + indent + 7.0, row.center().y), vec2(14.0, 14.0))
}

/// A changed file's row, as [`paint_file_row`] draws it.
pub(super) struct FileRow<'a> {
    pub kind: ChangeKind,
    pub name: &'a str,
    /// Its folder, or where it was renamed from.
    pub sub: &'a str,
    pub adds: Option<u32>,
    pub dels: Option<u32>,
    pub binary: bool,
    /// Its box: in the commit, hovered (`None`: no box).
    pub check: Option<(bool, bool)>,
    /// Room for a box (rows line up when some have one).
    pub box_slot: bool,
    /// The agent that changed it.
    pub agent: Option<tc_core::agent::AgentKind>,
    pub selected: bool,
    pub hovered: bool,
    /// A worktree's file not committed yet: a dot after its name.
    pub loose: bool,
    pub indent: f32,
}

/// One changed file (30 high, radius 7): its box, the change's letter, the name, its folder in
/// mono, the agent's tile, + and −; selected on the accent's tint.
pub(super) fn paint_file_row(painter: &egui::Painter, rect: Rect, row: &FileRow, p: &style::Palette) {
    if row.selected {
        painter.rect_filled(rect, CornerRadius::same(style::RADIUS_ROW), p.accent_tint);
    } else if row.hovered {
        painter.rect_filled(rect, CornerRadius::same(style::RADIUS_ROW), p.hover);
    }
    let cy = rect.center().y;
    let mut x = rect.left() + 10.0 + row.indent;
    if row.box_slot {
        if let Some((on, hovered)) = row.check {
            kit::paint_checkbox(painter, checkbox_rect(rect, row.indent), on, hovered, p);
        }
        x += 14.0 + 9.0;
    }
    let (letter, color) = change_letter(row.kind, p);
    painter.text(pos2(x, cy), Align2::LEFT_CENTER, letter, style::mono_semibold(style::DATA), color);
    x += 10.0 + 9.0;
    // From the right: − and +, then the agent.
    let mut right = rect.right() - 10.0;
    let small = style::mono(style::DATA);
    if row.binary {
        let g = painter.layout_no_wrap(t("binary").to_string(), small.clone(), p.text_faint);
        right -= g.size().x;
        painter.galley(pos2(right, cy - g.size().y / 2.0), g, p.text_faint);
        right -= 9.0;
    } else if let (Some(a), Some(d)) = (row.adds, row.dels) {
        for (text, c) in [(format!("−{d}"), p.err), (format!("+{a}"), p.ok)] {
            let g = painter.layout_no_wrap(text, small.clone(), c);
            right -= g.size().x;
            painter.galley(pos2(right, cy - g.size().y / 2.0), g, c);
            right -= 9.0;
        }
    }
    if let Some(k) = row.agent {
        right -= 16.0;
        kit::paint_agent_badge(painter, Rect::from_min_size(pos2(right, cy - 8.0), vec2(16.0, 16.0)), k, p);
        right -= 9.0;
    }
    let room = (right - x).max(10.0);
    let font = if row.selected { style::medium(style::BODY) } else { egui::FontId::proportional(style::BODY) };
    let dot = if row.loose { 10.0 } else { 0.0 };
    let mut job = egui::text::LayoutJob::simple_singleline(row.name.to_string(), font, p.text);
    job.wrap = egui::text::TextWrapping::truncate_at_width((room - dot).max(10.0));
    let g = painter.layout_job(job);
    let nw = g.size().x;
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
    x += nw;
    if row.loose {
        painter.circle_filled(pos2(x + 6.0, cy), 3.0, p.warn);
        x += dot;
    }
    if !row.sub.is_empty() && right - x - 9.0 > 24.0 {
        let mut job = egui::text::LayoutJob::simple_singleline(row.sub.to_string(), small, p.text_faint);
        job.wrap = egui::text::TextWrapping::truncate_at_width(right - x - 9.0);
        let g = painter.layout_job(job);
        painter.galley(pos2(x + 9.0, cy - g.size().y / 2.0 + 0.5), g, p.text_faint);
    }
}

/// A folder new as a whole, in the file list: its caret, the folder, how many files, A.
#[allow(clippy::too_many_arguments)]
fn paint_folder_row(painter: &egui::Painter, rect: Rect, name: &str, count: usize, folded: bool, hovered: bool, box_slot: bool, p: &style::Palette) {
    if hovered {
        painter.rect_filled(rect, CornerRadius::same(style::RADIUS_ROW), p.hover);
    }
    let cy = rect.center().y;
    let mut x = rect.left() + 10.0;
    if box_slot {
        x += 14.0 + 9.0;
    }
    painter.text(pos2(x + 5.0, cy), Align2::CENTER_CENTER, if folded { icons::CARET_RIGHT } else { icons::CARET_DOWN }, style::icon_font(10.0), p.text_faint);
    x += 10.0 + 6.0;
    painter.text(pos2(x + 7.0, cy), Align2::CENTER_CENTER, if folded { icons::FOLDER_SIMPLE } else { icons::FOLDER_OPEN }, style::icon_font(14.0), p.folder);
    x += 14.0 + 8.0;
    let a = painter.layout_no_wrap("A".to_string(), style::mono_semibold(style::DATA), p.ok);
    let right = rect.right() - 10.0 - a.size().x;
    painter.galley(pos2(right, cy - a.size().y / 2.0), a, p.ok);
    let mut job = egui::text::LayoutJob::simple_singleline(format!("{name}/"), egui::FontId::proportional(style::BODY), p.text);
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - 40.0 - x).max(20.0));
    let g = painter.layout_job(job);
    let nw = g.size().x;
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
    painter.text(pos2(x + nw + 8.0, cy), Align2::LEFT_CENTER, count.to_string(), style::mono(style::DATA), p.text_faint);
}

/// The five boxes' width, and the boxes, for others (the Merge dialog).
pub const SHARE_W: f32 = BARS_W;
pub fn paint_share(painter: &egui::Painter, left_center: egui::Pos2, a: u32, d: u32, p: &style::Palette) {
    paint_bars(painter, left_center, a, d, p);
}

/// Five boxes: the share of added (green) and removed (red) lines, starting at `left_center`.
pub(super) fn paint_bars(painter: &egui::Painter, left_center: egui::Pos2, a: u32, d: u32, p: &style::Palette) {
    let total = (a + d).max(1) as f32;
    let na = ((a as f32 / total) * 5.0).round() as usize;
    let nd = (((d as f32 / total) * 5.0).round() as usize).min(5 - na.min(5));
    for i in 0..5 {
        let c = if i < na {
            p.ok
        } else if i < na + nd {
            p.err
        } else {
            p.subtle
        };
        let r = Rect::from_min_size(pos2(left_center.x + i as f32 * 8.0, left_center.y - 3.0), vec2(6.0, 6.0));
        painter.rect_filled(r, CornerRadius::same(2), c);
    }
}

/// What the two sides of Split are.
fn side_names(base: Base, branch: Option<&BranchView>, who: Option<&str>) -> (String, String) {
    let (before, now) = match base {
        Base::Branch => match branch {
            Some(b) if !b.fork.is_empty() => (format!("{} · {}", b.base, b.fork), Some(b.branch.clone())),
            Some(b) => (b.base.clone(), Some(b.branch.clone())),
            None => ("HEAD".to_string(), Some(t("not committed").to_string())),
        },
        Base::Run => (tf("when {} started", &[who.unwrap_or("")]), None),
        Base::Head => ("HEAD".to_string(), Some(t("not committed").to_string())),
    };
    let now = match now {
        Some(n) => tf("Now · {}", &[&n]),
        None => t("Now").to_string(),
    };
    (tf("Before · {}", &[&before]), now)
}

/// A worktree's branch above its files: where it goes, ahead / behind, its last commits.
/// Returns where the files start.
fn branch_summary(ui: &Ui, col: Rect, top: f32, b: &BranchView, p: &style::Palette) -> f32 {
    let x = col.left() + 14.0;
    let mut y = top + 2.0;
    let small = egui::FontId::proportional(style::SMALL);
    let painter = ui.painter();
    painter.text(pos2(x + 6.0, y + 9.0), Align2::CENTER_CENTER, icons::GIT_FORK, style::icon_font(13.0), p.text_faint);
    let mut job = egui::text::LayoutJob::default();
    job.append(&b.branch, 0.0, egui::TextFormat::simple(style::mono(style::MONO), p.text));
    job.append("→", 6.0, egui::TextFormat::simple(egui::FontId::proportional(style::SMALL), p.text_faint));
    job.append(&b.base, 6.0, egui::TextFormat::simple(style::mono(style::MONO), p.text));
    let counts = painter.layout_no_wrap(format!("↑{} ↓{}", b.ahead, b.behind), style::mono(style::DATA), if b.ahead > 0 { p.warn } else { p.text_faint });
    let cw = counts.size().x;
    job.wrap = egui::text::TextWrapping::truncate_at_width((col.right() - 14.0 - cw - 8.0 - (x + 18.0)).max(20.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(x + 18.0, y + 9.0 - g.size().y / 2.0), g, p.text);
    let cc = if b.ahead > 0 { p.warn } else { p.text_faint };
    painter.galley(pos2(col.right() - 14.0 - cw, y + 9.0 - counts.size().y / 2.0), counts, cc);
    y += 18.0 + 6.0;
    const SHOWN: usize = 3;
    for c in b.commits.iter().take(SHOWN) {
        let hash = painter.layout_no_wrap(c.short.clone(), style::mono(style::DATA), p.accent_text);
        let hw = hash.size().x;
        painter.galley(pos2(x, y + 8.5 - hash.size().y / 2.0), hash, p.accent_text);
        let mut job = egui::text::LayoutJob::simple_singleline(c.subject.clone(), small.clone(), p.text_muted);
        job.wrap = egui::text::TextWrapping::truncate_at_width((col.right() - 14.0 - (x + hw + 7.0)).max(20.0));
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
    painter.hline(col.x_range(), y + 0.5, Stroke::new(1.0, p.border));
    y + 8.0
}

/// How a diff's lines are set: their height, the size of their code, a line number's width.
pub(super) struct LineLook {
    pub(super) h: f32,
    pub(super) font: f32,
    pub(super) num_w: f32,
}

impl LineLook {
    /// Lines 20 high, Mono 12, numbers 30 wide.
    pub(super) const DIFF: LineLook = LineLook { h: 20.0, font: style::MONO, num_w: 30.0 };
}

/// For each line of a hunk, the line it replaced or that replaced it (a removed line and the
/// added line in the same place of the run that follows), for marking the words that changed.
pub(super) fn word_pairs(lines: &[tc_git::DiffLine]) -> Vec<Option<usize>> {
    let mut out = vec![None; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Removed {
            i += 1;
            continue;
        }
        let dels = lines[i..].iter().take_while(|l| l.kind == LineKind::Removed).count();
        let adds = lines[i + dels..].iter().take_while(|l| l.kind == LineKind::Added).count();
        for k in 0..dels.min(adds) {
            out[i + k] = Some(i + dels + k);
            out[i + dels + k] = Some(i + k);
        }
        i += dels + adds;
    }
    out
}

/// The part of `text` that differs from `other` (from the first difference to the last, out to
/// whole words), as a byte range; none when the lines have too little in common for it to help.
pub(super) fn changed_part(text: &str, other: &str) -> Option<std::ops::Range<usize>> {
    let text = text.trim_end_matches(['\n', '\r']);
    let other = other.trim_end_matches(['\n', '\r']);
    let a: Vec<(usize, char)> = text.char_indices().collect();
    let b: Vec<char> = other.chars().collect();
    let prefix = a.iter().zip(&b).take_while(|((_, x), y)| x == *y).count();
    let max_suffix = a.len().min(b.len()) - prefix;
    let suffix = a.iter().rev().zip(b.iter().rev()).take(max_suffix).take_while(|((_, x), y)| x == *y).count();
    let same = prefix + suffix;
    if a.is_empty() || same * 4 < a.len().max(b.len()) {
        return None;
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut start = prefix;
    while start > 0 && word(a[start - 1].1) && start < a.len() && word(a[start].1) {
        start -= 1;
    }
    let mut end = a.len() - suffix;
    while end < a.len() && end > 0 && word(a[end].1) && word(a[end - 1].1) {
        end += 1;
    }
    if end <= start {
        return None;
    }
    let byte = |i: usize| a.get(i).map_or(text.len(), |(b, _)| *b);
    Some(byte(start)..byte(end))
}

/// A line of text in the middle of `rect`.
fn centered(ui: &Ui, rect: Rect, text: &str, color: Color32) {
    let g = ui.painter().layout(text.to_string(), egui::FontId::proportional(style::BODY), color, (rect.width() - 32.0).max(40.0));
    let sz = g.size();
    ui.painter().with_clip_rect(rect).galley(pos2(rect.center().x - sz.x / 2.0, rect.center().y - sz.y / 2.0), g, color);
}

/// Why there is nothing to show: the empty state (an icon tile, a title, a line under it) and
/// maybe a button. Returns whether the button was clicked.
fn empty_state(ui: &mut Ui, rect: Rect, title: &str, sub: &str, button: Option<&str>) -> bool {
    let mut clicked = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(16.0, 0.0))), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        kit::empty_state(ui, icons::CHECK_CIRCLE, title, sub, |ui| {
            if let Some(b) = button {
                let r = kit::button_keys(ui, kit::Kind::Primary, None, b, Some("⇥"), true);
                #[cfg(debug_assertions)]
                crate::devtest::record("review-show-head", r.rect);
                clicked = r.clicked();
            }
        });
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

pub(super) fn change_letter(kind: ChangeKind, p: &style::Palette) -> (&'static str, Color32) {
    match kind {
        ChangeKind::Added => ("A", p.ok),
        ChangeKind::Modified => ("M", p.accent_text),
        ChangeKind::Deleted => ("D", p.err),
        ChangeKind::Renamed => ("R", p.info),
    }
}

/// The tints of a diff: added and removed rows, and the words that changed in them.
fn diff_tints(p: &style::Palette, dark: bool) -> [Color32; 4] {
    [
        style::soft(p.ok, if dark { 0.09 } else { 0.07 }),
        style::soft(p.err, if dark { 0.09 } else { 0.06 }),
        style::soft(p.ok, if dark { 0.20 } else { 0.15 }),
        style::soft(p.err, if dark { 0.20 } else { 0.14 }),
    ]
}

/// A hunk's header (28 high, on the accent's tint): its line ranges, then the function it is in;
/// the hunk in view (`nav`: the right edge in view) has ↑ ↓ at its right. Returns the step asked
/// for.
pub(super) fn paint_hunk_header(ui: &Ui, r: Rect, header: &str, p: &style::Palette, nav: Option<(f32, usize)>) -> Option<i32> {
    if !ui.is_rect_visible(r) {
        return None;
    }
    let painter = ui.painter();
    painter.rect_filled(r, CornerRadius::ZERO, p.accent_tint);
    let (ranges, context) = match header.match_indices("@@").nth(1) {
        Some((i, _)) => (&header[..i + 2], header[i + 2..].trim()),
        None => (header, ""),
    };
    let g = painter.layout_no_wrap(ranges.to_string(), style::mono(style::DATA), p.text_faint);
    let gw = g.size().x;
    painter.galley(pos2(r.left() + 12.0, r.center().y - g.size().y / 2.0), g, p.text_faint);
    if !context.is_empty() {
        painter.text(pos2(r.left() + 12.0 + gw + 10.0, r.center().y), Align2::LEFT_CENTER, context, style::mono(style::DATA), p.text_muted);
    }
    let (right, hi) = nav?;
    let mut step = None;
    for (k, (icon, s, tip)) in [(icons::CARET_DOWN, 1, t("Next hunk   n")), (icons::CARET_UP, -1, t("Previous hunk   p"))].into_iter().enumerate() {
        let b = Rect::from_center_size(pos2(right - 8.0 - k as f32 * 18.0, r.center().y), vec2(16.0, 18.0));
        let resp = ui.interact(b, egui::Id::new(("review-hunk-nav", hi, k)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(b, CornerRadius::same(4), p.hover);
        }
        ui.painter().text(b.center(), Align2::CENTER_CENTER, icon, style::icon_font(12.0), if resp.hovered() { p.text } else { p.text_faint });
        if resp.on_hover_text(tip).clicked() {
            step = Some(s);
        }
    }
    step
}

/// A diff's hunks in `rect`, scrolling (both ways when unified): each hunk's header (↑ ↓ on
/// hunk `nav` when there are several), then its lines, unified or side by side; `scroll_to`
/// brings a hunk into view. Returns the hunk step asked for with ↑ ↓ (0 for none).
#[allow(clippy::too_many_arguments)]
pub(super) fn diff_view(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, d: &FileDiff, path: &str, split: bool, nav: Option<usize>, scroll_to: Option<usize>) -> i32 {
    let p = style::pal(ui);
    let dark = ui.visuals().dark_mode;
    let syntax = crate::editor::syntax_for(std::path::Path::new(path));
    let look = LineLook::DIFF;
    let char_w = ui.fonts_mut(|f| f.glyph_width(&style::mono(look.font), '0'));
    let max_cols = if split { 0 } else { d.hunks.iter().flat_map(|h| h.lines.iter()).map(|l| l.text.chars().count()).max().unwrap_or(0) };
    let content_w = if split { rect.width() } else { rect.width().max(UNIFIED_CODE_X + max_cols as f32 * char_w + 16.0) };
    let hunks = d.hunks.len();
    let mut step = 0;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let w = ui.available_width().max(content_w);
            for (hi, h) in d.hunks.iter().enumerate() {
                let (hr, _) = ui.allocate_exact_size(vec2(w, HUNK_H), Sense::hover());
                if scroll_to == Some(hi) {
                    ui.scroll_to_rect(hr, Some(egui::Align::Min));
                }
                // The hunk in view: ↑ ↓ to the one before and after.
                let here = nav.filter(|c| *c == hi && hunks > 1).map(|_| (hr.right().min(rect.right()) - 12.0, hi));
                if let Some(s) = paint_hunk_header(ui, hr, &h.header, &p, here) {
                    step = s;
                }
                if split {
                    paint_split(ui, w, &h.lines, &syntax, dark, &p, &look);
                } else {
                    let pairs = word_pairs(&h.lines);
                    for (li, l) in h.lines.iter().enumerate() {
                        let (lr, _) = ui.allocate_exact_size(vec2(w, look.h), Sense::hover());
                        if ui.is_rect_visible(lr) {
                            let hl = pairs[li].and_then(|o| changed_part(&l.text, &h.lines[o].text));
                            paint_line(ui, lr, l.kind, l.old_no, l.new_no, &l.text, &syntax, dark, &p, true, hl, &look);
                        }
                    }
                }
            }
            ui.add_space(12.0);
        });
    });
    step
}

/// Where a unified line's code starts: 4, the old and new numbers (30 each, 8 apart), the sign.
const UNIFIED_CODE_X: f32 = 4.0 + 30.0 + 8.0 + 30.0 + 8.0 + 12.0 + 8.0;

/// One line of a diff (20 high): its numbers (old and new, or one in a split half), its sign
/// (unified), its code in syntax colours, on green or red when added or removed; `hl`: the words
/// that changed, on a stronger tint.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_line(ui: &Ui, r: Rect, kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str, syntax: &str, dark: bool, p: &style::Palette, both: bool, hl: Option<std::ops::Range<usize>>, look: &LineLook) {
    let painter = ui.painter_at(r.intersect(ui.clip_rect()));
    let [add_bg, del_bg, add_hl, del_hl] = diff_tints(p, dark);
    let (bg, hl_bg, sign, sc) = match kind {
        LineKind::Added => (add_bg, add_hl, "+", p.ok),
        LineKind::Removed => (del_bg, del_hl, "-", p.err),
        LineKind::Context => (Color32::TRANSPARENT, Color32::TRANSPARENT, "", p.text_faint),
    };
    painter.rect_filled(r, CornerRadius::ZERO, bg);
    let mono = style::mono(look.font);
    let num = style::mono(style::DATA);
    let mut x = r.left() + 4.0;
    if both {
        for n in [old, new] {
            if let Some(n) = n {
                painter.text(pos2(x + look.num_w, r.center().y), Align2::RIGHT_CENTER, n.to_string(), num.clone(), p.text_faint);
            }
            x += look.num_w + 8.0;
        }
        painter.text(pos2(x, r.center().y), Align2::LEFT_CENTER, sign, mono.clone(), sc);
        x += 12.0 + 8.0;
    } else {
        if let Some(n) = new.or(old) {
            painter.text(pos2(x + look.num_w, r.center().y), Align2::RIGHT_CENTER, n.to_string(), num, p.text_faint);
        }
        x += look.num_w + 10.0;
    }
    let plain = p.text;
    let mut job = egui::text::LayoutJob::default();
    let text = text.trim_end_matches(['\n', '\r']);
    let mut at = 0;
    for (c, piece) in crate::editor::line_colors(text, syntax, dark, plain) {
        // A piece split where the changed words begin and end.
        let end = at + piece.len();
        let mut cuts = vec![at, end];
        if let Some(h) = &hl {
            for b in [h.start, h.end] {
                if b > at && b < end && piece.is_char_boundary(b - at) {
                    cuts.push(b);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for w in cuts.windows(2) {
            let (s, e) = (w[0], w[1]);
            let on = hl.as_ref().is_some_and(|h| s >= h.start && e <= h.end);
            let format = egui::TextFormat { font_id: mono.clone(), color: c, background: if on { hl_bg } else { Color32::TRANSPARENT }, ..Default::default() };
            job.append(&piece[s - at..e - at], 0.0, format);
        }
        at = end;
    }
    let g = painter.layout_job(job);
    painter.galley(pos2(x, r.center().y - g.size().y / 2.0), g, plain);
}

/// A hunk side by side: removed lines at the left beside what replaced them at the right (the
/// words that changed marked), the side with no line on the hover fill.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_split(ui: &mut Ui, w: f32, lines: &[tc_git::DiffLine], syntax: &str, dark: bool, p: &style::Palette, look: &LineLook) {
    let mut rows: Vec<(Option<usize>, Option<usize>)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        match lines[i].kind {
            LineKind::Context => {
                rows.push((Some(i), Some(i)));
                i += 1;
            }
            _ => {
                let dels = lines[i..].iter().take_while(|l| l.kind == LineKind::Removed).count();
                let adds = lines[i + dels..].iter().take_while(|l| l.kind == LineKind::Added).count();
                let n = dels.max(adds).max(1);
                for k in 0..n {
                    rows.push(((k < dels).then_some(i + k), (k < adds).then_some(i + dels + k)));
                }
                i += (dels + adds).max(1);
            }
        }
    }
    let half = (w / 2.0).floor();
    for (l, rgt) in rows {
        let (row, _) = ui.allocate_exact_size(vec2(w, look.h), Sense::hover());
        if !ui.is_rect_visible(row) {
            continue;
        }
        let left = Rect::from_min_size(row.min, vec2(half, row.height()));
        let right = Rect::from_min_max(pos2(row.left() + half, row.top()), row.max);
        let changed = |a: usize, b: Option<usize>| match b {
            Some(b) if lines[a].kind != LineKind::Context => changed_part(&lines[a].text, &lines[b].text),
            _ => None,
        };
        match l {
            Some(a) => {
                let kind = if lines[a].kind == LineKind::Context { LineKind::Context } else { LineKind::Removed };
                paint_line(ui, left, kind, lines[a].old_no, None, &lines[a].text, syntax, dark, p, false, changed(a, rgt), look);
            }
            None => {
                ui.painter().rect_filled(left, CornerRadius::ZERO, p.hover);
            }
        }
        match rgt {
            Some(b) => {
                let kind = if lines[b].kind == LineKind::Context { LineKind::Context } else { LineKind::Added };
                paint_line(ui, right, kind, None, lines[b].new_no, &lines[b].text, syntax, dark, p, false, changed(b, l), look);
            }
            None => {
                ui.painter().rect_filled(right, CornerRadius::ZERO, p.hover);
            }
        }
        ui.painter().vline(right.left(), row.y_range(), Stroke::new(1.0, p.border));
    }
}
