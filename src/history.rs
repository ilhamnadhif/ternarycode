//! Commit history browsing and GitHub/GitLab forge integration for the active project:
//! paginated `git log`, per-commit changed files and diffs, non-interactive `git fetch`,
//! and live open Pull Request / Merge Request listing via `gh` and `glab`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tc_core::ProjectId;
use tc_git::history::{CommitEntry, LogPage};
use tc_git::ship::{GhError, PushError, RemotePr};
use tc_git::{FileChange, FileDiff};

use crate::actions::Action;
use crate::app::{App, AppMsg};
use crate::i18n::{t, tf};

/// How many commits each page of `git log` loads.
pub const PAGE_SIZE: usize = 50;

/// Numbers background history jobs so stale responses from earlier projects/commits are dropped.
static WORK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_work() -> u64 {
    WORK.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Active Commit History state for one project (shown inside the Code Editor stage).
pub struct History {
    pub project: ProjectId,
    pub root: PathBuf,
    pub commits: Vec<CommitEntry>,
    pub has_more: bool,
    pub branch: Option<String>,
    pub loading: bool,
    pub loading_more: bool,
    pub error: Option<String>,
    /// Text currently in the search/filter input box.
    pub query: String,
    /// Query that was sent to `git log` (empty = full branch history).
    pub active_query: String,
    pub selected: usize,
    pub files_loading: bool,
    pub files_error: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub files: Vec<FileChange>,
    pub selected_file: usize,
    pub diff: Option<FileDiff>,
    pub diff_error: Option<String>,
    pub split: bool,
    pub fetching: bool,
    pub open_prs: Vec<RemotePr>,
    pub prs_loading: bool,
    pub prs_expanded: bool,
    pub prs_signed_out: bool,
    pub loaded_at: Option<Instant>,
    load: u64,
    files_load: u64,
    diff_load: u64,
    prs_load: u64,
}

impl History {
    fn new(project: ProjectId, root: PathBuf, split: bool) -> Self {
        Self {
            project,
            root,
            commits: Vec::new(),
            has_more: false,
            branch: None,
            loading: true,
            loading_more: false,
            error: None,
            query: String::new(),
            active_query: String::new(),
            selected: 0,
            files_loading: false,
            files_error: None,
            from: None,
            to: None,
            files: Vec::new(),
            selected_file: 0,
            diff: None,
            diff_error: None,
            split,
            fetching: false,
            open_prs: Vec::new(),
            prs_loading: false,
            prs_expanded: false,
            prs_signed_out: false,
            loaded_at: None,
            load: 0,
            files_load: 0,
            diff_load: 0,
            prs_load: 0,
        }
    }
}

/// Background messages delivered back to the UI thread for Commit History & Forge PRs/MRs.
pub enum HistoryMsg {
    Log {
        load: u64,
        root: PathBuf,
        append: bool,
        result: Result<LogPage, String>,
    },
    Files {
        load: u64,
        result: Result<(String, String, Vec<FileChange>), String>,
    },
    Diff {
        load: u64,
        file_idx: usize,
        result: Result<FileDiff, String>,
    },
    Fetched {
        project: ProjectId,
        remote: String,
        result: Result<(), PushError>,
    },
    OpenPrs {
        load: u64,
        result: Result<Vec<RemotePr>, GhError>,
    },
}

impl App {
    /// Opens Commit History in the Code Editor stage for the active project.
    pub fn open_history(&mut self) {
        let Some(pid) = self.active else { return };
        let Some(folder) = self.project(pid).map(|p| p.path.clone()) else { return };
        // Close any active Review (inline or full-screen) and Zen Mode so History is always visible.
        self.exit_zen_mode();
        self.review = None;
        self.review_close = false;
        self.drawer_maximized = false;
        self.ensure_editor_for_project(pid, 0.0);
        if let Some(e) = self.editors.get_mut(&pid) {
            e.visible = true;
        }
        self.show_focus();
        self.ship_want_facts(pid);

        let same = self.history.as_ref().is_some_and(|h| h.project == pid);
        if !same {
            self.history = Some(History::new(pid, folder, self.review_split));
        }
        self.history_reload(false);
        self.history_load_open_prs();
    }

    /// Refreshes open Commit History when terminal git operations change the repository (throttled).
    pub fn history_git_changed(&mut self, project: ProjectId) {
        let should_reload = self.history.as_ref().is_some_and(|h| {
            h.project == project
                && !h.loading
                && !h.loading_more
                && h.loaded_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(2))
        });
        if should_reload {
            self.history_reload(false);
        }
    }

    /// Keeps Commit History synced with whichever project is active in Focus view.
    pub fn history_follow(&mut self) {
        let Some(h) = self.history.as_ref() else { return };
        if Some(h.project) == self.active {
            return;
        }
        if self.active.is_some() {
            self.open_history();
        } else {
            self.history = None;
        }
    }

    /// Loads (or appends the next page of) git commits off the UI thread.
    pub fn history_reload(&mut self, append: bool) {
        let Some((pid, fallback_root)) = self.history.as_ref().map(|h| (h.project, h.root.clone())) else {
            return;
        };
        let folder = self.project(pid).map(|p| p.path.clone()).unwrap_or(fallback_root);
        let remote = self
            .ship
            .get(&pid)
            .and_then(|s| s.facts.as_ref())
            .and_then(|f| f.remote.as_ref())
            .map(|r| r.name.clone());
        let Some(h) = self.history.as_mut() else { return };
        if append && (h.loading || h.loading_more) {
            return;
        }
        let skip = if append { h.commits.len() } else { 0 };
        let query = if append { h.active_query.clone() } else { h.query.trim().to_string() };
        let load = next_work();
        h.load = load;
        if !append {
            h.active_query = query.clone();
        }
        if append {
            h.loading_more = true;
        } else {
            h.loading = true;
            h.error = None;
        }
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let result = match tc_git::repo_root(&folder) {
                Some(root) => {
                    let q = (!query.is_empty()).then_some(query.as_str());
                    let r = tc_git::history::log(&root, PAGE_SIZE, skip, q, remote.as_deref());
                    (root, r)
                }
                None => (folder, Err(t("Not a git repository.").to_string())),
            };
            let _ = tx.send(AppMsg::History(HistoryMsg::Log {
                load,
                root: result.0,
                append,
                result: result.1,
            }));
            ctx.request_repaint();
        });
    }

    /// Selects commit `idx` and loads its changed files off the UI thread.
    pub fn history_select_commit(&mut self, idx: usize) {
        let Some(h) = self.history.as_mut() else { return };
        let Some(commit) = h.commits.get(idx) else { return };
        let hash = commit.hash.clone();
        let root = h.root.clone();
        let load = next_work();
        h.selected = idx;
        h.files_load = load;
        h.diff_load = 0;
        h.files_loading = true;
        h.files_error = None;
        h.from = None;
        h.to = None;
        h.files.clear();
        h.selected_file = 0;
        h.diff = None;
        h.diff_error = None;
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let result = tc_git::history::commit_files(&root, &hash);
            let _ = tx.send(AppMsg::History(HistoryMsg::Files { load, result }));
            ctx.request_repaint();
        });
    }

    /// Selects file `idx` inside the currently inspected commit and loads its diff off-thread.
    pub fn history_select_file(&mut self, idx: usize) {
        let Some(h) = self.history.as_mut() else { return };
        let (Some(from), Some(to), Some(file)) = (h.from.clone(), h.to.clone(), h.files.get(idx).cloned()) else {
            return;
        };
        let root = h.root.clone();
        let load = next_work();
        h.selected_file = idx;
        h.diff_load = load;
        h.diff = None;
        h.diff_error = None;
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let result = tc_git::history::commit_file_diff(&root, &from, &to, &file.path, file.old_path.as_deref());
            let _ = tx.send(AppMsg::History(HistoryMsg::Diff {
                load,
                file_idx: idx,
                result,
            }));
            ctx.request_repaint();
        });
    }

    /// Runs `git fetch --prune` for the project's configured remote off the UI thread.
    pub fn history_fetch(&mut self) {
        let Some(h) = self.history.as_mut() else { return };
        if h.fetching {
            return;
        }
        let project = h.project;
        let root = h.root.clone();
        let remote = self
            .ship
            .get(&project)
            .and_then(|s| s.facts.as_ref())
            .and_then(|f| f.remote.as_ref())
            .map(|r| r.name.clone())
            .unwrap_or_else(|| "origin".into());
        let path = self.lsp.path_env.clone();
        h.fetching = true;
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let path = path.or_else(|| Some(tc_lsp::servers::user_path()));
            let result = tc_git::history::fetch(&root, &remote, path.as_deref());
            let _ = tx.send(AppMsg::History(HistoryMsg::Fetched { project, remote, result }));
            ctx.request_repaint();
        });
    }

    /// Loads open Pull Requests (GitHub via `gh`) or Merge Requests (GitLab via `glab`).
    pub fn history_load_open_prs(&mut self) {
        let Some(h) = self.history.as_mut() else { return };
        let pid = h.project;
        let Some(s) = self.ship.get(&pid) else { return };
        let Some(web) = s.facts.as_ref().and_then(|f| f.remote.as_ref()).and_then(|r| r.web.clone()) else {
            return;
        };
        let Some(cli) = crate::ship::cli_for(Some(&web), &s.gh, &s.glab).cloned() else {
            return;
        };
        let root = s.root.clone().unwrap_or_else(|| h.root.clone());
        let path = self.lsp.path_env.clone();
        let load = next_work();
        h.prs_load = load;
        h.prs_loading = true;
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let path = path.or_else(|| Some(tc_lsp::servers::user_path()));
            let result = tc_git::ship::list_requests(&cli, &root, &web, 15, path.as_deref());
            let _ = tx.send(AppMsg::History(HistoryMsg::OpenPrs { load, result }));
            ctx.request_repaint();
        });
    }

    /// Handles background messages from [`HistoryMsg`].
    pub fn on_history_msg(&mut self, msg: HistoryMsg) {
        match msg {
            HistoryMsg::Log { load, root, append, result } => {
                let Some(h) = self.history.as_mut() else { return };
                if h.load != load {
                    return;
                }
                h.root = root;
                h.loading = false;
                h.loading_more = false;
                h.loaded_at = Some(Instant::now());
                match result {
                    Ok(page) => {
                        h.error = None;
                        h.branch = page.branch;
                        h.has_more = page.has_more;
                        if append {
                            h.commits.extend(page.commits);
                        } else {
                            let prev_hash = h.commits.get(h.selected).map(|c| c.hash.clone());
                            let had_files = !h.files.is_empty() && h.files_error.is_none();
                            h.commits = page.commits;
                            let next_sel = prev_hash
                                .as_deref()
                                .and_then(|hash| h.commits.iter().position(|c| c.hash == hash))
                                .unwrap_or(0);
                            if !h.commits.is_empty() {
                                let same_commit = prev_hash.as_deref() == Some(h.commits[next_sel].hash.as_str());
                                if same_commit && had_files {
                                    h.selected = next_sel;
                                } else {
                                    self.history_select_commit(next_sel);
                                }
                            } else {
                                h.selected = 0;
                                h.files_load = 0;
                                h.diff_load = 0;
                                h.from = None;
                                h.to = None;
                                h.files.clear();
                                h.selected_file = 0;
                                h.files_error = None;
                                h.diff = None;
                                h.diff_error = None;
                                h.files_loading = false;
                            }
                        }
                    }
                    Err(e) => {
                        if !append {
                            h.commits.clear();
                            h.selected = 0;
                            h.files_load = 0;
                            h.diff_load = 0;
                            h.from = None;
                            h.to = None;
                            h.files.clear();
                            h.selected_file = 0;
                            h.files_error = None;
                            h.diff = None;
                            h.diff_error = None;
                            h.files_loading = false;
                        }
                        h.error = Some(e);
                    }
                }
            }
            HistoryMsg::Files { load, result } => {
                let Some(h) = self.history.as_mut() else { return };
                if h.files_load != load {
                    return;
                }
                h.files_loading = false;
                match result {
                    Ok((from, to, files)) => {
                        h.from = Some(from);
                        h.to = Some(to);
                        h.files = files;
                        h.files_error = None;
                        if !h.files.is_empty() {
                            self.history_select_file(0);
                        }
                    }
                    Err(e) => {
                        h.files_error = Some(e);
                    }
                }
            }
            HistoryMsg::Diff { load, file_idx, result } => {
                let Some(h) = self.history.as_mut() else { return };
                if h.diff_load != load || h.selected_file != file_idx {
                    return;
                }
                match result {
                    Ok(d) => {
                        h.diff = Some(d);
                        h.diff_error = None;
                    }
                    Err(e) => {
                        h.diff = None;
                        h.diff_error = Some(e);
                    }
                }
            }
            HistoryMsg::Fetched { project, remote, result } => {
                if let Some(h) = self.history.as_mut().filter(|h| h.project == project) {
                    h.fetching = false;
                }
                match result {
                    Ok(()) => {
                        self.toast_ok(t("Fetched latest changes from remote.").to_string());
                        self.ship_load(project);
                        self.refresh_git(project);
                        if self.history.as_ref().is_some_and(|h| h.project == project) {
                            self.history_reload(false);
                            self.history_load_open_prs();
                        }
                    }
                    Err(PushError::Credentials(_)) => {
                        let cmd = format!("git fetch --prune {}", crate::ship::quote(&remote));
                        self.toast_with(
                            crate::app::ToastKind::Warn,
                            t("Fetch needs credentials — run git fetch in a terminal.").to_string(),
                            Some((t("Run in Terminal").to_string(), Action::TypeInTerminal(cmd))),
                        );
                    }
                    Err(e) => {
                        self.toast_err(tf("Fetch failed: {}", &[&crate::ship::last_lines(e.output(), 2)]));
                    }
                }
            }
            HistoryMsg::OpenPrs { load, result } => {
                let Some(h) = self.history.as_mut() else { return };
                if h.prs_load != load {
                    return;
                }
                h.prs_loading = false;
                match result {
                    Ok(prs) => {
                        h.open_prs = prs;
                        h.prs_signed_out = false;
                    }
                    Err(GhError::SignedOut(_)) => {
                        h.open_prs.clear();
                        h.prs_signed_out = true;
                    }
                    Err(GhError::Failed(_)) => {
                        h.prs_signed_out = false;
                    }
                }
            }
        }
    }
}
