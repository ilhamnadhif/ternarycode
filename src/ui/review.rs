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
    /// The commit it branched at (short id) and when that was made (Unix seconds).
    pub fork: String,
    pub fork_time: Option<i64>,
}

/// One part of a worktree's work picked in full-screen Review: what is not committed, or one
/// of its commits (by short id). None: all of it since it branched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Uncommitted,
    Commit(String),
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
    /// A worktree's commit (or what is not committed) shown instead of all of its work.
    pub pick: Option<Pick>,
    /// Files that differ from the last commit: the ones the commit box can take.
    pub uncommitted: HashSet<String>,
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
    },
    Diff(u64, usize, Result<FileDiff, String>),
    /// The files of a picked commit (or of what is not committed), and the trees compared.
    Picked { load: u64, from: String, to: String, files: Result<Vec<FileChange>, String> },
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
            self.history = None;
            self.drawer_maximized = false;
            self.editors.entry(project_id).or_insert_with(|| crate::editor::Editor { width: 0.54, ..Default::default() }).visible = true;
        }
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
            expanded,
            from_agent,
            counts: Vec::new(),
            fell_back: false,
            new_dirs: Vec::new(),
            dir_of: Vec::new(),
            folded: HashSet::new(),
            pick: None,
            uncommitted: HashSet::new(),
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
                    let (fork_short, fork_time) = tc_git::worktree::commit_line(root, &fork_commit).map_or((String::new(), None), |(s, t)| (s, Some(t)));
                    branch = Some(BranchView {
                        fork: fork_short,
                        fork_time,
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
                    ReviewMsg::Loaded { load, root, from, to, files: Ok(files), branch: branch.map(Box::new), base, counts, fell_back, uncommitted }
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
                },
            };
            let _ = tx.send(AppMsg::Review(msg));
            ctx.request_repaint();
        });
    }

    /// Shows one part of a worktree's work (full screen): what is not committed, or one of its
    /// commits; `None`: all of it again.
    pub fn review_pick(&mut self, pick: Option<Pick>) {
        let Some(r) = self.review.as_mut() else { return };
        let Some(pick) = pick else {
            self.review_hunk = 0;
            self.open_review(Some(Base::Branch));
            return;
        };
        let load = next_work();
        r.load = load;
        r.loading = true;
        r.pick = Some(pick.clone());
        r.files.clear();
        r.diff = None;
        r.selected = 0;
        self.review_hunk = 0;
        let root = r.root.clone();
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let result = (|| {
                let (from, to) = match &pick {
                    Pick::Uncommitted => {
                        let (head, now) = tc_git::worktree_vs_head(&root)?;
                        (match head {
                            Some(h) => h,
                            None => tc_git::empty_tree(&root)?,
                        }, now)
                    }
                    Pick::Commit(c) => {
                        // Its parent's tree (none for a first commit: the empty tree), its own.
                        let from = tc_git::worktree::tree_of(&root, &format!("{c}^")).or_else(|_| tc_git::empty_tree(&root))?;
                        (from, tc_git::worktree::tree_of(&root, c)?)
                    }
                };
                let files = tc_git::changes(&root, &from, &to)?;
                Ok::<_, String>((from, to, files))
            })();
            let msg = match result {
                Ok((from, to, files)) => ReviewMsg::Picked { load, from, to, files: Ok(files) },
                Err(e) => ReviewMsg::Picked { load, from: String::new(), to: String::new(), files: Err(e) },
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
            ReviewMsg::Loaded { load, root, from, to, files, branch, base, counts, fell_back, uncommitted } => {
                if load != r.load {
                    return;
                }
                r.loading = false;
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
            ReviewMsg::Picked { load, from, to, files } => {
                if load != r.load {
                    return;
                }
                r.loading = false;
                r.error = None;
                r.diff = None;
                r.diff_error = None;
                match files {
                    Ok(f) => {
                        r.files = f;
                        r.from = Some(from);
                        r.to = Some(to);
                        r.selected = 0;
                        group_new_folders(r, &new_folders);
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
        let dark = ui.visuals().dark_mode;
        let full = ui.max_rect();
        // Where: the project (a worktree: its project, then the worktree).
        let (name, letters, color, parent) = match self.project(r.project) {
            Some(x) => (x.name.clone(), x.letters(), x.color32(), x.parent.and_then(|id| self.project(id)).map(|pp| (pp.name.clone(), pp.letters(), pp.color32()))),
            None => (String::new(), String::new(), p.text_faint, None),
        };
        let base = r.base;
        let bases = self.review_bases();
        let (scope_icon, scope_label) = match bases.iter().find(|(b, ..)| *b == base) {
            Some((Base::Branch, long, ..)) => (icons::GIT_BRANCH, long.clone()),
            Some((Base::Run, long, ..)) => (icons::SPARKLE, long.clone()),
            Some((_, long, ..)) => (icons::GIT_COMMIT, long.clone()),
            None => (icons::GIT_COMMIT, t("Since last commit").to_string()),
        };
        let back = if self.view == tc_core::model::View::Focus { t("Back to the Code drawer") } else { t("Back") };
        let body = self.top_bar(
            ui,
            full,
            |app, ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if kit::icon_button_ex(ui, icons::ARROW_LEFT, &format!("{back}   Esc"), false, 24.0).clicked() {
                    app.review_close = true;
                }
                ui.label(RichText::new(t("Review")).font(style::semibold(14.0)).color(p.text));
                ui.add_space(6.0);
                // The project's tile and name; a worktree's project › the worktree.
                let (tile_letters, tile_color, first) = match &parent {
                    Some((pn, pl, pc)) => (pl.clone(), *pc, pn.clone()),
                    None => (letters.clone(), color, name.clone()),
                };
                let (tr, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                kit::paint_project_tile(ui.painter(), tr, &tile_letters, tile_color, dark);
                ui.label(RichText::new(&first).size(12.5).color(p.text_muted));
                if parent.is_some() {
                    ui.label(RichText::new(icons::CARET_RIGHT).font(style::icon_font(10.0)).color(p.text_ghost));
                    ui.label(RichText::new(icons::GIT_FORK).font(style::icon_font(12.0)).color(p.text_faint));
                    ui.label(RichText::new(&name).font(style::medium(12.5)).color(p.text));
                }
                kit::vsep(ui);
                // What it is compared with, as a chip: a click lists the others with their counts.
                let chip = kit::chip(ui, Some(scope_icon), &scope_label, false, Some(icons::CARET_DOWN));
                #[cfg(debug_assertions)]
                crate::devtest::record("review-scope-chip", chip.rect);
                egui::Popup::menu(&chip).align(egui::RectAlign::BOTTOM_START).width(280.0).show(|ui| {
                    kit::menu_body(ui);
                    for (b, long, _, n) in &bases {
                        let icon = match b {
                            Base::Branch => icons::GIT_BRANCH,
                            Base::Run => icons::SPARKLE,
                            Base::Head => icons::GIT_COMMIT,
                        };
                        let count = n.map(|n| n.to_string());
                        let icon = if *b == base { icons::CHECK } else { icon };
                        let item = kit::menu_item(ui, Some(icon), long, count.as_deref(), false);
                        #[cfg(debug_assertions)]
                        crate::devtest::record(format!("review-base:{b:?}"), item.rect);
                        if item.clicked() {
                            if *b != base {
                                app.review_switch = Some(*b);
                            }
                            ui.close();
                        }
                    }
                });
            },
            None,
        );
        let area = super::focus::sheets_area(body, self.settings.sidebar_hidden);
        super::focus::paint_sheet(ui.painter(), area, &p, super::focus::SheetLook::Plain);
        self.review_body(ui, area.shrink(1.0), false);
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
        let has_branch = r.worktree.is_some();
        let branch = r.branch.clone();
        let bases = self.review_bases();
        let nfiles = r.files.len();
        let head_count = bases.iter().find(|(b, ..)| *b == Base::Head).and_then(|(.., n)| *n).unwrap_or(0);
        let mut switch = None;
        let files_w = if drawer { (body.width() * 0.42).clamp(240.0, 320.0) } else { FILES_W };

        // ----- keys: full screen always; in the drawer once it was clicked ----------------------
        let keys_id = egui::Id::new("review-keys");
        if drawer {
            let resp = ui.interact(body, keys_id, Sense::click());
            let filter_area = ui.data(|d| d.get_temp::<Rect>(egui::Id::new("review-filter-rect"))).unwrap_or(Rect::NOTHING);
            let ship_area = ui.data(|d| d.get_temp::<Rect>(egui::Id::new("ship-box-rect"))).unwrap_or(Rect::NOTHING);
            let pressed_here = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pos| body.contains(pos) && !filter_area.contains(pos) && !ship_area.contains(pos)));
            if resp.clicked() || pressed_here {
                ui.memory_mut(|m| m.request_focus(keys_id));
            }
        }
        let has_keys = !drawer || ui.memory(|m| m.has_focus(keys_id));
        let filter_focused = ui.ctx().memory(|m| m.focused().is_some_and(|f| f == egui::Id::new("review-filter")));
        // Typing in the commit box: Review's keys leave it alone (Esc too).
        let ship_typing = ui.ctx().memory(|m| m.focused().is_some_and(|f| f == App::ship_message_id(r.project) || f == egui::Id::new(("ship-branch", r.project))));
        let mut toggle_in: Option<usize> = None;
        let mut close = false;
        let mut to_terminal = false;
        let mut select: Option<usize> = None;
        let mut hunk_step: i32 = 0;
        let mut open = false;
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
                if i.consume_key(egui::Modifiers::NONE, Key::J)
                    && let Some(&next) = visible.iter().find(|&&idx| idx > sel)
                {
                    select = Some(next);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::K)
                    && let Some(&prev) = visible.iter().rev().find(|&&idx| idx < sel)
                {
                    select = Some(prev);
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
                // Space: the file in or out of the commit.
                if i.consume_key(egui::Modifiers::NONE, Key::Space) && sel < nfiles {
                    toggle_in = Some(sel);
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
        let dark = ui.visuals().dark_mode;
        let pick = r.pick.clone();
        let run_kind = who.as_deref().and_then(|w| tc_core::agent::AgentKind::ALL.iter().copied().find(|k| k.label() == w));
        // Full screen, a worktree: its foot (the branch, Merge, Remove) under both columns.
        let wt_foot = (!drawer && has_branch && branch.is_some() && base == Base::Branch).then(|| Rect::from_min_max(pos2(body.left(), body.bottom() - WT_FOOT_H), body.max));
        let main = Rect::from_min_max(body.min, pos2(body.right(), wt_foot.map_or(body.bottom(), |f| f.top())));
        let files_rect = Rect::from_min_max(main.min, pos2(main.left() + files_w, main.bottom()));
        let diff_rect = Rect::from_min_max(pos2(files_rect.right(), main.top()), main.max);
        ui.painter().vline(files_rect.right() - 0.5, files_rect.y_range(), Stroke::new(1.0, p.border));
        let mut list_top = files_rect.top() + 8.0;

        // In the drawer: what the changes are counted since (the agent's start, the branch, the
        // last commit), each with how many files; a click shows it.
        if drawer && bases.len() > 1 {
            let opts: Vec<&(Base, String, String, Option<usize>)> = [Base::Run, Base::Branch, Base::Head].iter().filter_map(|b| bases.iter().find(|x| x.0 == *b)).collect();
            let n = opts.len() as f32;
            let scope = Rect::from_min_size(pos2(files_rect.left() + 8.0, list_top), vec2(files_w - 16.0, 6.0 + n * SCOPE_ROW_H + (n - 1.0) * 2.0));
            let well = if dark { p.inset } else { Color32::from_rgb(0xf0, 0xf0, 0xef) };
            ui.painter().rect(scope, CornerRadius::same(9), well, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
            let mut y = scope.top() + 3.0;
            for (b, _, short, count) in opts {
                let row = Rect::from_min_size(pos2(scope.left() + 3.0, y), vec2(scope.width() - 6.0, SCOPE_ROW_H));
                y += SCOPE_ROW_H + 2.0;
                let on = *b == base;
                let resp = ui.interact(row, egui::Id::new(("review-scope", *b as u8)), Sense::click());
                #[cfg(debug_assertions)]
                crate::devtest::record(format!("review-base:{b:?}"), row);
                paint_scope_row(ui, row, *b, short, *count, on, resp.hovered(), run_age.as_deref(), run_kind, &p);
                if resp.clicked() && !on {
                    switch = Some(*b);
                }
            }
            list_top = scope.bottom() + 8.0;
        }

        // The diff side when there is nothing to show: why.
        let mut show_head = false;
        if r.loading {
            centered(ui, diff_rect, t("Looking at the working tree…"), p.text_muted);
        } else if let Some(e) = &r.error {
            centered(ui, diff_rect, e, p.err);
        } else if r.files.is_empty() {
            match base {
                _ if pick.is_some() => centered(ui, diff_rect, t("Nothing changed."), p.text_muted),
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

        // A worktree's branch. In the drawer: where it goes, ahead / behind, its last commits.
        // Full screen: its commits as a line (what is not committed on top, then each commit,
        // then where it branched); a click shows that one alone, All all of them.
        let mut pick_click: Option<Option<Pick>> = None;
        if base == Base::Branch
            && let Some(b) = &branch
        {
            if drawer {
                list_top = branch_summary(ui, files_rect, list_top, b, &p);
            } else {
                let loose = b.uncommitted.len();
                let head = Rect::from_min_size(pos2(files_rect.left(), list_top - 8.0), vec2(files_w, SECTION_H));
                let n = if loose > 0 { tf("{} + not committed", &[&b.commits.len().to_string()]) } else { b.commits.len().to_string() };
                let all = paint_section_head(ui, head, t("Commits"), &n, pick.is_some().then_some(t("All")), &p);
                #[cfg(debug_assertions)]
                if let Some(a) = &all {
                    crate::devtest::record("review-commits-all", a.rect);
                }
                if all.is_some_and(|a| a.clicked()) {
                    pick_click = Some(None);
                }
                // Rows: (pick, title, sub, kind) — kind 0: not committed, 1: a commit, 2: the base.
                let mut rows: Vec<(Option<Pick>, String, String, u8)> = Vec::new();
                if loose > 0 {
                    let mut first: Vec<&String> = b.uncommitted.iter().collect();
                    first.sort();
                    let name = first.first().map(|f| f.rsplit('/').next().unwrap_or(f).to_string()).unwrap_or_default();
                    let files = crate::i18n::count(loose, t("1 file"), t("{} files"));
                    rows.push((Some(Pick::Uncommitted), t("Not committed").to_string(), if loose == 1 { format!("{files} · {name}") } else { files }, 0));
                }
                for c in &b.commits {
                    let mut sub = c.short.clone();
                    if !c.author.is_empty() {
                        sub.push_str(&format!(" · {}", c.author));
                    }
                    if c.time > 0 {
                        sub.push_str(&format!(" · {}", unix_age(c.time)));
                    }
                    rows.push((Some(Pick::Commit(c.short.clone())), c.subject.clone(), sub, 1));
                }
                let mut fork = tf("branched from {} at {}", &[&b.base, &b.fork]);
                if let Some(t0) = b.fork_time {
                    fork.push_str(&format!(" · {}", unix_age(t0)));
                }
                rows.push((None, fork, String::new(), 2));
                let rows_h: f32 = rows.iter().map(|r| if r.3 == 2 { 32.0 } else { 44.0 }).sum::<f32>() + 6.0;
                let max_h = (files_rect.height() * 0.42).max(120.0);
                let area = Rect::from_min_size(pos2(files_rect.left() + 6.0, head.bottom() + 2.0), vec2(files_w - 12.0, rows_h.min(max_h)));
                ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                    ui.set_clip_rect(area);
                    egui::ScrollArea::vertical().id_salt("review-commits").auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let last = rows.len().saturating_sub(1);
                        for (i, (rp, title, sub, kind)) in rows.iter().enumerate() {
                            let h = if *kind == 2 { 32.0 } else { 44.0 };
                            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), if rp.is_some() { Sense::click() } else { Sense::hover() });
                            let painter = ui.painter();
                            let on = rp.is_some() && *rp == pick;
                            if on {
                                painter.rect(row, CornerRadius::same(7), p.accent_tint, Stroke::new(1.0, style::soft(p.accent, 0.28)), egui::StrokeKind::Inside);
                            } else if resp.hovered() && rp.is_some() {
                                painter.rect_filled(row, CornerRadius::same(7), p.hover);
                            }
                            // The line through the dots, then the dot.
                            let dx = row.left() + 10.0 + 4.5;
                            let top = if i == 0 { row.center().y } else { row.top() };
                            let bottom = if i == last { row.center().y } else { row.bottom() };
                            painter.vline(dx, top..=bottom, Stroke::new(1.0, p.border_heavy));
                            if *kind == 2 {
                                painter.circle_filled(pos2(dx, row.center().y), 3.5, p.text_ghost);
                            } else {
                                let ring = if *kind == 0 { p.accent_text } else { p.text_faint };
                                painter.circle(pos2(dx, row.center().y), 4.5, p.canvas, Stroke::new(2.0, ring));
                            }
                            let x = dx + 4.5 + 12.0;
                            let w = (row.right() - 8.0 - x).max(20.0);
                            if *kind == 2 {
                                let mut job = egui::text::LayoutJob::simple_singleline(title.clone(), egui::FontId::proportional(11.5), p.text_faint);
                                job.wrap = egui::text::TextWrapping::truncate_at_width(w);
                                let g = painter.layout_job(job);
                                painter.galley(pos2(x, row.center().y - g.size().y / 2.0), g, p.text_faint);
                            } else {
                                let (font, color) = if *kind == 0 { (style::medium(12.5), p.accent_text) } else { (egui::FontId::proportional(12.5), p.text) };
                                let mut job = egui::text::LayoutJob::simple_singleline(title.clone(), font, color);
                                job.wrap = egui::text::TextWrapping::truncate_at_width(w);
                                let g = painter.layout_job(job);
                                let gh = g.size().y;
                                painter.galley(pos2(x, row.center().y - gh - 0.5), g, color);
                                let mut job = egui::text::LayoutJob::simple_singleline(sub.clone(), egui::FontId::proportional(11.0), p.text_faint);
                                job.wrap = egui::text::TextWrapping::truncate_at_width(w);
                                let g = painter.layout_job(job);
                                painter.galley(pos2(x, row.center().y + 1.5), g, p.text_faint);
                            }
                            #[cfg(debug_assertions)]
                            if let Some(rp) = rp {
                                crate::devtest::record(format!("review-commit:{}", match rp { Pick::Uncommitted => "uncommitted", Pick::Commit(c) => c.as_str() }), row);
                            }
                            if resp.clicked()
                                && let Some(rp) = rp
                            {
                                pick_click = Some(if on { None } else { Some(rp.clone()) });
                            }
                        }
                    });
                });
                let fh = Rect::from_min_size(pos2(files_rect.left(), area.bottom() + 6.0), vec2(files_w, SECTION_H));
                paint_section_head(ui, fh, t("Files"), &nfiles.to_string(), None, &p);
                list_top = fh.bottom() + 2.0;
            }
        }

        // Many files: a filter over them.
        let Some(r) = self.review.as_ref() else { return };
        let mut filter = self.review_filter.clone();
        if nfiles > FILTER_FROM || !filter.is_empty() || filter_focused {
            let filter_rect = Rect::from_min_size(pos2(files_rect.left() + 8.0, list_top), vec2(files_w - 16.0, 26.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(filter_rect), |ui| {
                kit::field(ui, egui::Id::new("review-filter"), &mut filter, Some(icons::FUNNEL), t("Filter changed files"), filter_rect.width(), true)
            });
            self.review_filter = filter.clone();
            list_top = filter_rect.bottom() + 8.0;
            ui.data_mut(|d| d.insert_temp(egui::Id::new("review-filter-rect"), filter_rect));
        } else {
            ui.data_mut(|d| d.insert_temp(egui::Id::new("review-filter-rect"), Rect::NOTHING));
        }

        // The column's foot: the commit box (in the drawer, a worktree's Merge and Remove under
        // it); outside a repository, how much changed and against what.
        let (adds, dels): (u32, u32) = r.files.iter().fold((0, 0), |(a, d), f| (a + f.additions.unwrap_or(0), d + f.deletions.unwrap_or(0)));
        let has_box = r.error.is_none();
        let fell_back = r.fell_back;
        let foot_top = if has_box {
            let col_bottom = if base == Base::Branch && branch.is_some() && drawer { self.review_branch_actions(ui, files_rect, branch.as_ref()).top() } else { files_rect.bottom() };
            let top = self.ship_box(ui, Rect::from_min_max(files_rect.min, pos2(files_rect.right(), col_bottom)));
            // The agent's run changed nothing: why these are the changes since the last commit.
            if fell_back {
                let text = tf("{} changed nothing since it started: these are the changes since the last commit.", &[who.as_deref().unwrap_or("")]);
                let g = ui.painter().layout(text, egui::FontId::proportional(style::SMALL), p.text_faint, files_w - 28.0);
                let note_top = top - g.size().y - 20.0;
                ui.painter().hline(files_rect.x_range(), note_top + 0.5, Stroke::new(1.0, p.border));
                ui.painter().galley(pos2(files_rect.left() + 14.0, note_top + 10.0), g, p.text_faint);
                note_top
            } else {
                top
            }
        } else if base == Base::Branch && branch.is_some() {
            if drawer { self.review_branch_actions(ui, files_rect, branch.as_ref()).top() } else { files_rect.bottom() }
        } else {
            let note = match base {
                _ if self.review.as_ref().is_some_and(|r| r.fell_back) => String::new(),
                Base::Run => tf("since {} started", &[who.as_deref().unwrap_or("")]),
                _ => t("not committed").to_string(),
            };
            let foot = Rect::from_min_max(pos2(files_rect.left(), files_rect.bottom() - COL_FOOT_H), files_rect.max);
            let painter = ui.painter();
            painter.hline(foot.x_range(), foot.top() + 0.5, Stroke::new(1.0, p.border));
            let mut job = egui::text::LayoutJob::default();
            job.append(&format!("+{adds}"), 0.0, egui::TextFormat::simple(style::mono(10.5), p.ok));
            job.append(&format!("−{dels}"), 6.0, egui::TextFormat::simple(style::mono(10.5), p.err));
            if !note.is_empty() {
                job.append("·", 6.0, egui::TextFormat::simple(egui::FontId::proportional(11.0), p.text_ghost));
                job.append(&note, 6.0, egui::TextFormat::simple(egui::FontId::proportional(11.0), p.text_faint));
            }
            job.wrap = egui::text::TextWrapping::truncate_at_width(foot.width() - 24.0);
            let g = painter.layout_job(job);
            painter.galley(pos2(foot.left() + 12.0, foot.center().y - g.size().y / 2.0), g, p.text_faint);
            // The agent's run changed nothing: why these are the changes since the last commit.
            if self.review.as_ref().is_some_and(|r| r.fell_back) {
                let text = tf("{} changed nothing since it started: these are the changes since the last commit.", &[who.as_deref().unwrap_or("")]);
                let g = ui.painter().layout(text, egui::FontId::proportional(style::SMALL), p.text_faint, files_w - 28.0);
                let top = foot.top() - g.size().y - 20.0;
                ui.painter().hline(foot.x_range(), top + 0.5, Stroke::new(1.0, p.border));
                ui.painter().galley(pos2(foot.left() + 14.0, top + 10.0), g, p.text_faint);
                top
            } else {
                foot.top()
            }
        };

        // The files: their change, name and folder, who changed them, + and −.
        let Some(r) = self.review.as_ref() else { return };
        let list_rect = Rect::from_min_max(pos2(files_rect.left() + 6.0, list_top), pos2(files_rect.right() - 6.0, foot_top));
        let filtering = !filter.trim().is_empty();
        let shown: Vec<usize> = r
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| !filtering || f.path.to_lowercase().contains(&filter.trim().to_lowercase()))
            .map(|(i, _)| i)
            .collect();
        let agent_logo = (base == Base::Run).then_some(run_kind).flatten().filter(|k| super::agent_logo::has(*k));
        // The commit box takes the files not committed yet: a box before each says in or out.
        let ship_out = self.ship.get(&r.project).map(|s| s.out.clone()).unwrap_or_default();
        let cb_col = has_box && r.files.iter().any(|f| r.uncommitted.contains(&f.path));
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
                    let indent = if dir.is_some() { 14.0 } else { 0.0 };
                    if let Some(d) = dir
                        && last_dir != Some(d)
                    {
                        let (name, count) = &r.new_dirs[d];
                        let folded = r.folded.contains(&d);
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(7), p.hover);
                        }
                        let painter = ui.painter();
                        painter.text(pos2(rect.left() + 6.0 + 7.0, rect.center().y), Align2::CENTER_CENTER, if folded { icons::CARET_RIGHT } else { icons::CARET_DOWN }, style::icon_font(10.0), p.text_faint);
                        painter.text(pos2(rect.left() + 28.0 + 7.0, rect.center().y), Align2::CENTER_CENTER, if folded { icons::FOLDER } else { icons::FOLDER_OPEN }, style::icon_font(14.0), p.folder);
                        let g = painter.layout_no_wrap(format!("{name}/"), egui::FontId::proportional(12.5), p.text);
                        let nw = g.size().x.min(rect.width() - 90.0);
                        painter.galley(pos2(rect.left() + 48.0, rect.center().y - g.size().y / 2.0), g, p.text);
                        painter.text(pos2(rect.left() + 48.0 + nw + 6.0, rect.center().y), Align2::LEFT_CENTER, count.to_string(), style::mono(10.5), p.text_faint);
                        painter.text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, "A", style::mono_semibold(10.5), p.ok);
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
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), FILE_ROW_H), Sense::click());
                    #[cfg(debug_assertions)]
                    crate::devtest::record(format!("review-file:{}", f.path), rect);
                    let painter = ui.painter();
                    if *i == r.selected {
                        painter.rect(rect, CornerRadius::same(7), p.accent_tint, Stroke::new(1.0, style::soft(p.accent, 0.28)), egui::StrokeKind::Inside);
                    } else if resp.hovered() {
                        painter.rect_filled(rect, CornerRadius::same(7), p.hover);
                    }
                    let mut left = rect.left() + 6.0 + indent;
                    if cb_col {
                        if r.uncommitted.contains(&f.path) {
                            let cb = Rect::from_center_size(pos2(left + 7.5, rect.center().y), vec2(15.0, 15.0));
                            let cb_resp = ui.interact(cb.expand(3.0), egui::Id::new(("review-in", &f.path)), Sense::click());
                            let painter = ui.painter();
                            kit::paint_checkbox(painter, cb, !ship_out.contains(&f.path), cb_resp.hovered(), &p);
                            #[cfg(debug_assertions)]
                            crate::devtest::record(format!("review-in:{}", f.path), cb);
                            let in_commit = !ship_out.contains(&f.path);
                            if cb_resp.on_hover_text(if in_commit { t("In the commit: click to leave it out") } else { t("Left out of the commit: click to put it in") }).clicked() {
                                toggle_in = Some(*i);
                            }
                        }
                        left += 20.0;
                    }
                    let painter = ui.painter();
                    painter.text(pos2(left + 7.0, rect.center().y), Align2::CENTER_CENTER, letter, style::mono_semibold(10.5), color);
                    // The counts at the right, the agent's mark before them.
                    let mut job = egui::text::LayoutJob::default();
                    if f.binary {
                        job.append(t("binary"), 0.0, egui::TextFormat::simple(egui::FontId::proportional(11.0), p.text_faint));
                    } else {
                        if let Some(a) = f.additions.filter(|a| *a > 0) {
                            job.append(&format!("+{a}"), 0.0, egui::TextFormat::simple(style::mono(10.5), p.ok));
                        }
                        if let Some(d) = f.deletions.filter(|d| *d > 0) {
                            job.append(&format!("−{d}"), 5.0, egui::TextFormat::simple(style::mono(10.5), p.err));
                        }
                    }
                    let counts = painter.layout_job(job);
                    let cw = counts.size().x;
                    painter.galley(pos2(rect.right() - 8.0 - cw, rect.center().y - counts.size().y / 2.0), counts, p.text_faint);
                    let mut right = rect.right() - 8.0 - cw - 8.0;
                    if let Some(k) = agent_logo {
                        super::agent_logo::paint(painter, Rect::from_min_size(pos2(right - 11.0, rect.center().y - 5.5), vec2(11.0, 11.0)), k, p.text, false);
                        right -= 11.0 + 8.0;
                    }
                    // The name, and its folder under it (under a new folder: within it).
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
                    // A worktree's file not committed yet: a dot after its name.
                    let loose = base == Base::Branch && branch.as_ref().is_some_and(|b| b.uncommitted.contains(&f.path));
                    let dot_w = if loose { 12.0 } else { 0.0 };
                    let x = left + 14.0 + 8.0;
                    let w = (right - x - dot_w).max(20.0);
                    let mut job = egui::text::LayoutJob::simple_singleline(name, egui::FontId::proportional(12.5), p.text);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(w);
                    let g = painter.layout_job(job);
                    let (nw, nh) = (g.size().x, g.size().y);
                    let name_y = if sub.is_empty() { rect.center().y - nh / 2.0 } else { rect.center().y - nh + 0.5 };
                    painter.galley(pos2(x, name_y), g, p.text);
                    if loose {
                        painter.circle_filled(pos2(x + nw + 8.0, name_y + nh / 2.0), 3.0, p.warn);
                    }
                    if !sub.is_empty() {
                        let mut job = egui::text::LayoutJob::simple_singleline(sub, style::mono(10.5), p.text_faint);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(w + dot_w);
                        let g = painter.layout_job(job);
                        painter.galley(pos2(x, rect.center().y + 2.0), g, p.text_faint);
                    }
                    if resp.clicked() {
                        select = Some(*i);
                    }
                }
            });
        });
        let next_file = shown.iter().position(|i| *i == r.selected).and_then(|at| shown.get(at + 1)).copied();

        // The diff: the file's header (change, path, + −, where it is from, Unified | Split,
        // Open in Editor), the hunks, then the keys (drawer) or the next file (full screen).
        if let Some(f) = r.files.get(r.selected).filter(|_| !r.loading && r.error.is_none()) {
            let deleted = f.kind == ChangeKind::Deleted;
            let hunks = r.diff.as_ref().map_or(0, |d| d.hunks.len());
            let cur_hunk = self.review_hunk.min(hunks.saturating_sub(1));
            // Two columns need room: in a narrow drawer only one.
            let split_ok = !drawer || diff_rect.width() >= SPLIT_MIN_W;
            let split = self.review_split && split_ok;
            let head = Rect::from_min_size(diff_rect.min, vec2(diff_rect.width(), DIFF_HEAD_H));
            let foot_h = if drawer { STATUS_H } else if next_file.is_some() { NEXT_H } else { 0.0 };
            let foot = Rect::from_min_max(pos2(diff_rect.left(), diff_rect.bottom() - foot_h), diff_rect.max);
            let sides = Rect::from_min_size(pos2(diff_rect.left(), head.bottom()), vec2(diff_rect.width(), if split { SIDES_H } else { 0.0 }));
            let lines_rect = Rect::from_min_max(pos2(diff_rect.left(), sides.bottom()), pos2(diff_rect.right(), foot.top()));
            ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, p.border));
            let cy = head.center().y;
            // Right, from the right: Open in Editor; in the drawer Unified | Split and the hunk;
            // a worktree's file: the commit it is from.
            let inner = Rect::from_min_max(pos2(head.left() + 14.0, head.top()), pos2(head.right() - 8.0, head.bottom()));
            let mut split_pick = None;
            let left_end = ui
                .scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    // A narrow drawer: the shorter word, and no commit name (the path needs the room).
                    let narrow = diff_rect.width() < 480.0;
                    if !deleted && kit::button_keys(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), if narrow { t("Open") } else { t("Open in Editor") }, Some("↵"), true).clicked() {
                        open = true;
                    }
                    if drawer && split_ok {
                        let options = [kit::Seg::new(t("Unified"), ""), kit::Seg::new(t("Split"), "")];
                        let w = kit::segmented_width(ui, &options);
                        let (sr, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
                        #[cfg(debug_assertions)]
                        for (r, name) in kit::segment_rects(sr, 2).into_iter().zip(["review-unified", "review-split"]) {
                            crate::devtest::record(name, r);
                        }
                        split_pick = kit::segmented_in(ui, sr, egui::Id::new("review-split-drawer"), &options, self.review_split as usize);
                    }
                    if drawer && hunks > 1 {
                        ui.label(RichText::new(tf("hunk {} of {}", &[&(cur_hunk + 1).to_string(), &hunks.to_string()])).font(style::mono(10.5)).color(p.text_faint));
                    }
                    if base == Base::Branch
                        && !narrow
                        && let Some(b) = &branch
                    {
                        if b.uncommitted.contains(&f.path) && !matches!(pick, Some(Pick::Commit(_))) {
                            ui.label(RichText::new(t("not committed")).size(11.5).color(p.warn));
                        } else if let Some(c) = match &pick {
                            Some(Pick::Commit(c)) => Some(c),
                            _ => b.by_file.get(&f.path),
                        } {
                            ui.label(RichText::new(tf("in {}", &[c])).font(style::mono(10.5)).color(p.text_faint));
                        }
                    }
                    ui.min_rect().left()
                })
                .inner;
            if let Some(i) = split_pick {
                self.review_split = i == 1;
            }
            // Left: the change, the path, + − and their share.
            let (letter, lcolor) = change_letter(f.kind, &p);
            let painter = ui.painter();
            painter.text(pos2(head.left() + 14.0 + 7.0, cy), Align2::CENTER_CENTER, letter, style::mono_semibold(10.5), lcolor);
            let a = f.additions.unwrap_or(0);
            let d = f.deletions.unwrap_or(0);
            let mut cjob = egui::text::LayoutJob::default();
            if a > 0 {
                cjob.append(&format!("+{a}"), 0.0, egui::TextFormat::simple(style::mono(11.0), p.ok));
            }
            if d > 0 {
                cjob.append(&format!("−{d}"), if a > 0 { 6.0 } else { 0.0 }, egui::TextFormat::simple(style::mono(11.0), p.err));
            }
            let counts = painter.layout_job(cjob);
            let cw = counts.size().x;
            let bars_w = if f.binary { 0.0 } else { BARS_W + 8.0 };
            let x = head.left() + 14.0 + 14.0 + 8.0;
            let path = match &f.old_path {
                Some(old) => format!("{old} → {}", f.path),
                None => f.path.clone(),
            };
            let mut job = egui::text::LayoutJob::simple_singleline(path, style::medium(12.5), p.text);
            job.wrap = egui::text::TextWrapping::truncate_at_width((left_end - 12.0 - x - cw - 8.0 - bars_w).max(20.0));
            let g = painter.layout_job(job);
            let gw = g.size().x;
            painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
            painter.galley(pos2(x + gw + 8.0, cy - counts.size().y / 2.0), counts, p.text_faint);
            if !f.binary {
                paint_bars(painter, pos2(x + gw + 8.0 + cw + 8.0, cy), a, d, &p);
            }

            // Split: what each side is.
            if split {
                let half = (sides.width() / 2.0).floor();
                let (l, rt) = side_names(base, &pick, branch.as_ref(), who.as_deref());
                painter.hline(sides.x_range(), sides.bottom() - 0.5, Stroke::new(1.0, p.border));
                painter.vline(sides.left() + half, sides.y_range(), Stroke::new(1.0, p.border));
                painter.text(pos2(sides.left() + 14.0, sides.center().y), Align2::LEFT_CENTER, l, style::mono(10.5), p.text_faint);
                painter.text(pos2(sides.left() + half + 14.0, sides.center().y), Align2::LEFT_CENTER, rt, style::mono(10.5), p.text_faint);
            }

            let syntax = crate::editor::syntax_for(std::path::Path::new(&f.path));
            let (line_h, font) = if drawer && !split { (20.0, 11.0) } else { (21.0, 11.5) };
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
                    let look = LineLook { h: line_h, font, num_w: if drawer { 36.0 } else { 40.0 } };
                    ui.scope_builder(egui::UiBuilder::new().max_rect(lines_rect), |ui| {
                        ui.set_clip_rect(lines_rect);
                        egui::ScrollArea::both().id_salt(("review-diff", r.selected)).auto_shrink([false, false]).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let w = ui.available_width().max(lines_rect.width());
                            for (hi, h) in d.hunks.iter().enumerate() {
                                let (hr, _) = ui.allocate_exact_size(vec2(w, HUNK_H), Sense::hover());
                                if scroll_to == Some(hi) {
                                    ui.scroll_to_rect(hr, Some(egui::Align::Min));
                                }
                                // The hunk in view: ↑ ↓ to the one before and after.
                                let nav = (hi == cur_hunk && hunks > 1).then(|| (hr.right().min(lines_rect.right()) - 10.0, hi));
                                if let Some(step) = paint_hunk_header(ui, hr, &h.header, &p, hi == 0, nav) {
                                    hunk_step = step;
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
                }
            }
            if drawer {
                // The keys, as many as the drawer has room for.
                ui.painter().hline(foot.x_range(), foot.top() + 0.5, Stroke::new(1.0, p.border));
                let w = diff_rect.width();
                let full_key = kit::keys(&Action::Review).unwrap_or_default();
                let mut hints: Vec<(&[&str], &str)> = vec![(&["j", "k"], t("file"))];
                if w >= 420.0 {
                    hints.push((&["n", "p"], t("hunk")));
                }
                if w >= 560.0 {
                    hints.push((&["↵"], t("open in editor")));
                }
                let full_keys = [full_key];
                if w >= 700.0 {
                    hints.push((&full_keys, t("full window")));
                }
                if w >= 820.0 && bases.len() > 1 {
                    hints.push((&["⇥"], t("since")));
                }
                super::statusbar::paint_key_hints(ui, Rect::from_min_max(pos2(foot.left() + 14.0, foot.top()), pos2(foot.right() - 14.0, foot.bottom())), &hints, None);
            } else if let Some(n) = next_file.and_then(|i| r.files.get(i).map(|f| (i, f))) {
                // The next file, a key away.
                let (ni, nf) = n;
                let resp = ui.interact(foot, egui::Id::new("review-next"), Sense::click());
                #[cfg(debug_assertions)]
                crate::devtest::record("review-next", foot);
                let painter = ui.painter();
                if resp.hovered() {
                    painter.rect_filled(foot.shrink2(vec2(0.0, 1.0)), CornerRadius::ZERO, p.hover);
                }
                painter.hline(foot.x_range(), foot.top() + 0.5, Stroke::new(1.0, p.border));
                let cy = foot.center().y;
                let (letter, lcolor) = change_letter(nf.kind, &p);
                painter.text(pos2(foot.left() + 14.0 + 7.0, cy), Align2::CENTER_CENTER, letter, style::mono_semibold(10.5), lcolor);
                let hint_g = painter.layout_no_wrap(t("next file").to_string(), egui::FontId::proportional(11.5), p.text_faint);
                let hw = hint_g.size().x;
                painter.galley(pos2(foot.right() - 14.0 - hw, cy - hint_g.size().y / 2.0), hint_g, p.text_faint);
                let kw = kit::kbd_width(painter, "j");
                kit::paint_kbd_text(painter, pos2(foot.right() - 14.0 - hw - 5.0 - kw, cy), "j", &p);
                let x = foot.left() + 14.0 + 14.0 + 8.0;
                let a = nf.additions.unwrap_or(0);
                let d = nf.deletions.unwrap_or(0);
                let mut cjob = egui::text::LayoutJob::default();
                if a > 0 {
                    cjob.append(&format!("+{a}"), 0.0, egui::TextFormat::simple(style::mono(11.0), p.ok));
                }
                if d > 0 {
                    cjob.append(&format!("−{d}"), if a > 0 { 6.0 } else { 0.0 }, egui::TextFormat::simple(style::mono(11.0), p.err));
                }
                let counts = painter.layout_job(cjob);
                let cw = counts.size().x;
                let mut job = egui::text::LayoutJob::simple_singleline(nf.path.clone(), style::medium(12.5), p.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width((foot.right() - 14.0 - hw - kw - 24.0 - x - cw - BARS_W - 16.0).max(20.0));
                let g = painter.layout_job(job);
                let gw = g.size().x;
                painter.galley(pos2(x, cy - g.size().y / 2.0), g, p.text);
                painter.galley(pos2(x + gw + 8.0, cy - counts.size().y / 2.0), counts, p.text_faint);
                if !nf.binary {
                    paint_bars(painter, pos2(x + gw + 8.0 + cw + 8.0, cy), a, d, &p);
                }
                if resp.clicked() {
                    select = Some(ni);
                }
            }
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

        // Full screen, a worktree: the branch, how it stands beside its base, Merge and Remove.
        if let (Some(fr), Some(b)) = (wt_foot, branch.as_ref()) {
            self.review_worktree_foot(ui, fr, b);
        }
        if let Some(pc) = pick_click {
            self.review_pick(pc);
        }
        if let Some(d) = fold
            && let Some(r) = self.review.as_mut()
            && !r.folded.remove(&d)
        {
            r.folded.insert(d);
        }
        if let Some(i) = toggle_in
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
        // What is not committed is the commit box's (above): here Merge and Remove only.
        let p = style::pal(ui);
        let base = branch.map(|b| b.base.clone()).unwrap_or_default();
        let rect = Rect::from_min_max(pos2(files_rect.left(), files_rect.bottom() - 10.0 - 24.0 - 12.0), files_rect.max);
        ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, p.border));
        let y = rect.top() + 10.0;
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

    /// Full-screen Review of a worktree, at the foot: its branch, how it stands beside its base
    /// and what is not committed, then Remove and Merge (each opens its dialog).
    fn review_worktree_foot(&mut self, ui: &mut Ui, fr: Rect, b: &BranchView) {
        let p = style::pal(ui);
        let corner = style::RADIUS_LG.saturating_sub(1);
        let painter = ui.painter();
        painter.rect_filled(fr, CornerRadius { nw: 0, ne: 0, sw: corner, se: corner }, style::over(p.canvas, p.chrome, 0.25));
        painter.hline(fr.x_range(), fr.top() + 0.5, Stroke::new(1.0, p.border));
        let cy = fr.center().y;
        let x = fr.left() + 16.0;
        painter.text(pos2(x + 7.0, cy), Align2::CENTER_CENTER, icons::GIT_FORK, style::icon_font(14.0), p.text_faint);
        let x = x + 14.0 + 9.0;
        let id = self.active;
        let buttons = Rect::from_min_max(pos2(x, fr.top()), pos2(fr.right() - 10.0, fr.bottom()));
        let right_end = ui
            .scope_builder(egui::UiBuilder::new().max_rect(buttons).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let merge = kit::button_ex(ui, kit::Kind::Primary, Some(icons::GIT_MERGE), &tf("Merge into {}…", &[&b.base]), false);
                #[cfg(debug_assertions)]
                crate::devtest::record("review-merge", merge.rect);
                if merge.clicked()
                    && let Some(id) = id
                {
                    self.actions.push(Action::MergeWorktree(id));
                }
                let remove = kit::button_ex(ui, kit::Kind::Danger, Some(icons::TRASH), t("Remove Worktree…"), false);
                #[cfg(debug_assertions)]
                crate::devtest::record("review-remove", remove.rect);
                if remove.clicked()
                    && let Some(id) = id
                {
                    self.actions.push(Action::RemoveWorktree(id));
                }
                ui.min_rect().left()
            })
            .inner;
        let small = egui::FontId::proportional(12.0);
        let mut job = egui::text::LayoutJob::default();
        job.append(&b.branch, 0.0, egui::TextFormat::simple(style::mono(11.0), p.text));
        job.append(&crate::i18n::count(b.ahead, t("1 commit ahead of"), t("{} commits ahead of")), 9.0, egui::TextFormat::simple(small.clone(), p.text_faint));
        job.append(&b.base, 4.0, egui::TextFormat::simple(style::mono(11.0), p.text_faint));
        if b.behind == 0 {
            job.append(&format!(", {}", t("nothing new on")), 0.0, egui::TextFormat::simple(small.clone(), p.text_faint));
        } else {
            job.append(&format!(", {}", crate::i18n::count(b.behind, t("1 new commit on"), t("{} new commits on"))), 0.0, egui::TextFormat::simple(small.clone(), p.text_faint));
        }
        job.append(&b.base, 4.0, egui::TextFormat::simple(style::mono(11.0), p.text_faint));
        let loose = b.uncommitted.len();
        if loose > 0 {
            job.append(&format!(", {}", crate::i18n::count(loose, t("1 file not committed"), t("{} files not committed"))), 0.0, egui::TextFormat::simple(small, p.warn));
        }
        job.wrap = egui::text::TextWrapping::truncate_at_width((right_end - 12.0 - x).max(20.0));
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, p.text_faint);
    }

    /// The Review's toolbar: how much changed, Unified | Split, Close.
    pub fn review_toolbar(&mut self, ui: &mut Ui) {
        let Some(r) = self.review.as_ref() else { return };
        let p = style::pal(ui);
        ui.spacing_mut().item_spacing.x = 6.0;
        if kit::button_keys(ui, kit::Kind::Ghost, None, t("Close"), Some("esc"), true).clicked() {
            self.review_close = true;
        }
        kit::vsep(ui);
        let options = [kit::Seg::new(t("Unified"), ""), kit::Seg::new(t("Split"), "")];
        let w = kit::segmented_width(ui, &options);
        let (sr, _) = ui.allocate_exact_size(vec2(w, kit::SEGMENTED_H), Sense::hover());
        #[cfg(debug_assertions)]
        for (r, name) in kit::segment_rects(sr, 2).into_iter().zip(["review-unified", "review-split"]) {
            crate::devtest::record(name, r);
        }
        if let Some(i) = kit::segmented_in(ui, sr, egui::Id::new("review-split-full"), &options, self.review_split as usize) {
            self.review_split = i == 1;
        }
        ui.add_space(6.0);
        if !r.files.is_empty() {
            let (adds, dels): (u32, u32) = r.files.iter().fold((0, 0), |(a, d), f| (a + f.additions.unwrap_or(0), d + f.deletions.unwrap_or(0)));
            let mut job = egui::text::LayoutJob::default();
            let mono = style::mono(11.0);
            job.append(&crate::i18n::count(r.files.len(), t("1 file"), t("{} files")), 0.0, egui::TextFormat::simple(mono.clone(), p.text_faint));
            job.append(" · ", 0.0, egui::TextFormat::simple(mono.clone(), p.text_faint));
            job.append(&format!("+{adds}"), 0.0, egui::TextFormat::simple(mono.clone(), p.ok));
            job.append(&format!("−{dels}"), 6.0, egui::TextFormat::simple(mono, p.err));
            ui.label(job);
        }
    }
}

/// Width of the Review's file list (full screen).
const FILES_W: f32 = 300.0;
/// A choice of what the changes are counted since (two lines), in the drawer.
const SCOPE_ROW_H: f32 = 44.0;
/// A section's head in the full-screen column (Commits, Files).
const SECTION_H: f32 = 30.0;
/// From this many changed files on, a filter over them.
const FILTER_FROM: usize = 12;
/// The file list's foot (how much changed), a file's row, the diff's head.
const COL_FOOT_H: f32 = 30.0;
const FILE_ROW_H: f32 = 40.0;
const DIFF_HEAD_H: f32 = 42.0;
/// The keys under the diff (drawer), the next file (full screen), the names of Split's sides.
const STATUS_H: f32 = 28.0;
const NEXT_H: f32 = 40.0;
const SIDES_H: f32 = 28.0;
/// The narrowest diff (in the drawer) shown side by side.
const SPLIT_MIN_W: f32 = 560.0;
pub(super) const HUNK_H: f32 = 30.0;
/// A worktree's foot in full screen (its branch, Merge, Remove).
const WT_FOOT_H: f32 = 52.0;
/// The five boxes that show the share of added and removed lines.
pub(super) const BARS_W: f32 = 5.0 * 6.0 + 4.0 * 2.0;

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

/// One of the drawer's choices of what the changes are counted since: its mark (the agent's
/// logo, a commit, a branch), what it is, and how many files (with how long ago the agent
/// started); the one shown raised.
#[allow(clippy::too_many_arguments)]
fn paint_scope_row(ui: &Ui, row: Rect, b: Base, short: &str, count: Option<usize>, on: bool, hovered: bool, age: Option<&str>, kind: Option<tc_core::agent::AgentKind>, p: &style::Palette) {
    let painter = ui.painter();
    let dark = ui.visuals().dark_mode;
    if on {
        if dark {
            painter.rect_filled(row.translate(vec2(0.0, 1.0)), CornerRadius::same(7), Color32::from_black_alpha(64));
        }
        painter.rect(row, CornerRadius::same(7), p.surface, Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
        kit::paint_highlight(painter, row, 7, p);
    } else if hovered {
        painter.rect_filled(row, CornerRadius::same(7), p.hover);
    }
    let ic = pos2(row.left() + 8.0 + 7.0, row.top() + 7.0 + 8.0);
    match (b, kind.filter(|k| super::agent_logo::has(*k))) {
        (Base::Run, Some(k)) => {
            super::agent_logo::paint(painter, Rect::from_center_size(ic, vec2(12.0, 12.0)), k, p.text, false);
        }
        _ => {
            let icon = match b {
                Base::Run => icons::SPARKLE,
                Base::Head => icons::GIT_COMMIT,
                Base::Branch => icons::GIT_BRANCH,
            };
            painter.text(ic, Align2::CENTER_CENTER, icon, style::icon_font(13.0), p.text_faint);
        }
    }
    let x = row.left() + 8.0 + 14.0 + 8.0;
    let w = (row.right() - 8.0 - x).max(20.0);
    let title = match b {
        Base::Run => tf("Since {} started", &[short]),
        Base::Head => t("Since last commit").to_string(),
        Base::Branch => tf("Since branched from {}", &[short]),
    };
    let files = count.map(|n| crate::i18n::count(n, t("1 file"), t("{} files")));
    let sub = match (b, age, files) {
        (Base::Run, Some(a), Some(f)) => format!("{} · {f}", tf("{} ago", &[a])),
        (Base::Run, Some(a), None) => tf("{} ago", &[a]),
        (_, _, Some(f)) => f,
        _ => String::new(),
    };
    let (font, color) = if on { (style::medium(12.0), p.text) } else { (egui::FontId::proportional(12.0), p.text_muted) };
    let mut job = egui::text::LayoutJob::simple_singleline(title, font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(w);
    let g = painter.layout_job(job);
    let gh = g.size().y;
    painter.galley(pos2(x, row.top() + 7.0), g, color);
    let mut job = egui::text::LayoutJob::simple_singleline(sub, style::mono_medium(10.5), if on { p.text_muted } else { p.text_faint });
    job.wrap = egui::text::TextWrapping::truncate_at_width(w);
    let g = painter.layout_job(job);
    painter.galley(pos2(x, row.top() + 7.0 + gh + 2.0), g, p.text_faint);
}

/// A section's head in Review's full-screen column ("Commits 2 + not committed", "Files 5"),
/// with a link at its right; returns the link's response.
fn paint_section_head(ui: &mut Ui, rect: Rect, title: &str, n: &str, link: Option<&str>, p: &style::Palette) -> Option<egui::Response> {
    let painter = ui.painter();
    let g = painter.layout_no_wrap(title.to_string(), style::medium(11.5), p.text_faint);
    let gw = g.size().x;
    painter.galley(pos2(rect.left() + 14.0, rect.center().y - g.size().y / 2.0), g, p.text_faint);
    painter.text(pos2(rect.left() + 14.0 + gw + 6.0, rect.center().y), Align2::LEFT_CENTER, n, style::mono_medium(10.5), p.text_ghost);
    let link = link?;
    let lg = painter.layout_no_wrap(link.to_string(), egui::FontId::proportional(11.5), p.text_muted);
    let lr = Rect::from_min_max(pos2(rect.right() - 14.0 - lg.size().x - 6.0, rect.center().y - 10.0), pos2(rect.right() - 8.0, rect.center().y + 10.0));
    let resp = ui.interact(lr, egui::Id::new(("review-section-link", title)), Sense::click());
    let c = if resp.hovered() { p.text } else { p.text_muted };
    if resp.hovered() {
        ui.painter().rect_filled(lr, CornerRadius::same(5), p.hover);
    }
    ui.painter().galley(pos2(lr.left() + 3.0, lr.center().y - lg.size().y / 2.0), lg, c);
    Some(resp.on_hover_cursor(egui::CursorIcon::PointingHand))
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
fn side_names(base: Base, pick: &Option<Pick>, branch: Option<&BranchView>, who: Option<&str>) -> (String, String) {
    match (pick, base) {
        (Some(Pick::Commit(c)), _) => (format!("{c}^"), c.clone()),
        (Some(Pick::Uncommitted), _) => ("HEAD".to_string(), t("not committed").to_string()),
        (None, Base::Branch) => match branch {
            Some(b) if !b.fork.is_empty() => (format!("{} · {}", b.base, b.fork), b.branch.clone()),
            Some(b) => (b.base.clone(), b.branch.clone()),
            None => ("HEAD".to_string(), t("not committed").to_string()),
        },
        (None, Base::Run) => (tf("when {} started", &[who.unwrap_or("")]), t("now").to_string()),
        (None, Base::Head) => ("HEAD".to_string(), t("not committed").to_string()),
    }
}

/// A worktree's branch in the drawer, above its files: where it goes, ahead / behind, its
/// last commits. Returns where the files start.
fn branch_summary(ui: &Ui, files_rect: Rect, top: f32, b: &BranchView, p: &style::Palette) -> f32 {
    let x = files_rect.left() + 14.0;
    let mut y = top + 2.0;
    let small = egui::FontId::proportional(style::SMALL);
    let painter = ui.painter();
    painter.text(pos2(x + 6.0, y + 9.0), Align2::CENTER_CENTER, icons::GIT_FORK, style::icon_font(13.0), p.text_faint);
    let mut job = egui::text::LayoutJob::default();
    job.append(&b.branch, 0.0, egui::TextFormat::simple(style::mono(11.5), p.text));
    job.append("→", 6.0, egui::TextFormat::simple(egui::FontId::proportional(12.0), p.text_faint));
    job.append(&b.base, 6.0, egui::TextFormat::simple(style::mono(11.5), p.text));
    let counts = painter.layout_no_wrap(format!("↑{} ↓{}", b.ahead, b.behind), style::mono(10.5), p.text_faint);
    let cw = counts.size().x;
    job.wrap = egui::text::TextWrapping::truncate_at_width((files_rect.right() - 14.0 - cw - 8.0 - (x + 18.0)).max(20.0));
    let g = painter.layout_job(job);
    painter.galley(pos2(x + 18.0, y + 9.0 - g.size().y / 2.0), g, p.text);
    painter.galley(pos2(files_rect.right() - 14.0 - cw, y + 9.0 - counts.size().y / 2.0), counts, p.text_faint);
    y += 18.0 + 6.0;
    const SHOWN: usize = 3;
    for c in b.commits.iter().take(SHOWN) {
        let hash = painter.layout_no_wrap(c.short.clone(), style::mono(10.5), p.text_faint);
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
    y + 8.0
}

/// How a diff's lines are set: their height, the size of their text, the width of a number.
pub(super) struct LineLook {
    pub(super) h: f32,
    pub(super) font: f32,
    pub(super) num_w: f32,
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

pub(super) fn change_letter(kind: ChangeKind, p: &style::Palette) -> (&'static str, Color32) {
    match kind {
        ChangeKind::Added => ("A", p.ok),
        ChangeKind::Modified => ("M", p.accent_text),
        ChangeKind::Deleted => ("D", p.err),
        ChangeKind::Renamed => ("R", p.info),
    }
}

/// A hunk's header: its line ranges, then the function it is in; the hunk in view (`nav`: the
/// right edge in view) has ↑ ↓ at its right. Returns the step asked for.
pub(super) fn paint_hunk_header(ui: &Ui, r: Rect, header: &str, p: &style::Palette, first: bool, nav: Option<(f32, usize)>) -> Option<i32> {
    if !ui.is_rect_visible(r) {
        return None;
    }
    let painter = ui.painter();
    painter.rect_filled(r, CornerRadius::ZERO, style::over(p.canvas, p.accent, 0.06));
    if !first {
        painter.hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border));
    }
    painter.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border));
    let (ranges, context) = match header.match_indices("@@").nth(1) {
        Some((i, _)) => (&header[..i + 2], header[i + 2..].trim()),
        None => (header, ""),
    };
    let g = painter.layout_no_wrap(ranges.replace('-', "−"), style::mono(11.0), p.text_faint);
    let gw = g.size().x;
    painter.galley(pos2(r.left() + 14.0, r.center().y - g.size().y / 2.0), g, p.text_faint);
    if !context.is_empty() {
        painter.text(pos2(r.left() + 14.0 + gw + 8.0, r.center().y), Align2::LEFT_CENTER, context, style::mono(11.0), p.text_muted);
    }
    let (right, hi) = nav?;
    let mut step = None;
    for (k, (icon, s, tip)) in [(icons::CARET_DOWN, 1, t("Next hunk   n")), (icons::CARET_UP, -1, t("Previous hunk   p"))].into_iter().enumerate() {
        let b = Rect::from_center_size(pos2(right - 9.0 - k as f32 * 20.0, r.center().y), vec2(18.0, 18.0));
        let resp = ui.interact(b, egui::Id::new(("review-hunk-nav", hi, k)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(b, CornerRadius::same(4), p.hover);
        }
        ui.painter().text(b.center(), Align2::CENTER_CENTER, icon, style::icon_font(11.0), if resp.hovered() { p.text } else { p.text_faint });
        if resp.on_hover_text(tip).clicked() {
            step = Some(s);
        }
    }
    step
}

/// One line of a diff: its numbers (old, new, or one of them in a split half), its sign, its
/// code in syntax colours, on green or red when added or removed; `hl`: the words that changed,
/// on a stronger tint.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_line(ui: &Ui, r: Rect, kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str, syntax: &str, dark: bool, p: &style::Palette, both: bool, hl: Option<std::ops::Range<usize>>, look: &LineLook) {
    let painter = ui.painter();
    let (bg, hl_bg, sign, sc, num_c) = match kind {
        LineKind::Added => (style::soft(p.ok, if dark { 0.10 } else { 0.08 }), style::soft(p.ok, if dark { 0.28 } else { 0.20 }), "+", p.ok, style::soft(p.ok, 0.6)),
        LineKind::Removed => (style::soft(p.err, if dark { 0.10 } else { 0.07 }), style::soft(p.err, if dark { 0.28 } else { 0.20 }), "−", p.err, style::soft(p.err, 0.6)),
        LineKind::Context => (Color32::TRANSPARENT, Color32::TRANSPARENT, "", p.text_ghost, p.text_ghost),
    };
    painter.rect_filled(r, CornerRadius::ZERO, bg);
    let mono = style::mono(look.font);
    let mut x = r.left();
    let nums: Vec<Option<u32>> = if both { vec![old, new] } else { vec![new.or(old)] };
    for n in nums {
        if let Some(n) = n {
            painter.text(pos2(x + look.num_w - 8.0, r.center().y), Align2::RIGHT_CENTER, n.to_string(), style::mono(10.5), num_c);
        }
        x += look.num_w;
    }
    painter.text(pos2(x + 9.0, r.center().y), Align2::CENTER_CENTER, sign, mono.clone(), sc);
    x += 18.0;
    let plain = if kind == LineKind::Context { p.text_muted } else { p.text };
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

/// The side of a split row with no line: faint diagonal lines, continuous from row to row.
fn paint_hatch(painter: &egui::Painter, r: Rect, p: &style::Palette) {
    let painter = painter.with_clip_rect(r.intersect(painter.clip_rect()));
    let h = r.height();
    let k = ((r.left() - h + r.bottom()) / 6.0).floor();
    let mut x = k * 6.0 - r.bottom();
    while x < r.right() {
        painter.line_segment([pos2(x, r.bottom()), pos2(x + h, r.top())], Stroke::new(1.0, p.hover));
        x += 6.0;
    }
}

/// A hunk side by side: removed lines at the left beside what replaced them at the right (the
/// words that changed marked), a hatched side where one has no line.
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
            None => paint_hatch(ui.painter(), left, p),
        }
        match rgt {
            Some(b) => {
                let kind = if lines[b].kind == LineKind::Context { LineKind::Context } else { LineKind::Added };
                paint_line(ui, right, kind, None, lines[b].new_no, &lines[b].text, syntax, dark, p, false, changed(b, l), look);
            }
            None => paint_hatch(ui.painter(), right, p),
        }
        ui.painter().vline(right.left(), row.y_range(), Stroke::new(1.0, p.border));
    }
}
