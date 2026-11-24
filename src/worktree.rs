//! Worktrees for agent runs: Run Agent → New worktree makes a git worktree of the project on a
//! branch of its own (`tc/‹name›`), beside the repository by default, and starts the agent in
//! it. The worktree is a project with a parent: it is listed under its project and has tabs,
//! panes, a Grid tile, Review and the editor like any other. Its work is reviewed since it
//! branched, merged by a command typed in its project's terminal (so git is seen as it is), and
//! removed with its folder, and with its branch once that is merged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;
use tc_core::ProjectId;
use tc_core::model::{ProjectSave, WorktreePlace, WorktreeSave, WorktreeSetup};

use crate::actions::Action;
use crate::app::{App, AppMsg, ToastKind};
use crate::i18n::{t, tf};
use crate::project::{GitInfo, Project, Tab};

/// What Run Agent needs to know about a repository before making a worktree of it.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub branches: Vec<String>,
    /// The branch checked out in the project's folder.
    pub current: Option<String>,
    /// Each branch's last commit: its short id and time (Unix seconds).
    pub last: HashMap<String, (String, i64)>,
    /// What git ignores at the top of the repository (folders end in `/`).
    pub ignored: Vec<String>,
}

/// A worktree made off the UI thread, or why it was not.
pub struct Made {
    pub root: ProjectId,
    pub name: String,
    pub path: PathBuf,
    pub branch: String,
    pub base: String,
    pub preset: String,
    /// The commit it started at and the files copied into it.
    pub result: Result<(String, Vec<String>), String>,
}

/// Run Agent's choices for a new worktree.
#[derive(Clone, Debug, Default)]
pub struct Form {
    /// Work in a new worktree (else in the project's folder).
    pub on: bool,
    pub name: String,
    /// The name was typed (else it follows the agent chosen).
    pub typed: bool,
    /// The branch it starts from (`None`: the one checked out).
    pub base: Option<String>,
}

/// A worktree's branch beside its base: commits ahead and behind, and whether it is merged
/// (it has commits of its own, all in the base now). Runs git: off the UI thread.
pub fn branch_state(path: &Path, wt: &WorktreeSave, info: &mut GitInfo) {
    if wt.base.is_empty() {
        return;
    }
    if let Ok((ahead, behind)) = tc_git::worktree::ahead_behind(path, &wt.base, "HEAD") {
        info.ahead = ahead;
        info.behind = behind;
        if ahead > 0 {
            info.branch_files = tc_git::worktree::files_changed(path, &wt.base, "HEAD").unwrap_or(0);
        }
    }
    if let Some(head) = tc_git::worktree::head(path) {
        info.merged = head != wt.start && tc_git::worktree::is_ancestor(path, &head, &wt.base);
    }
}

/// The repository's branches, its current one, their last commits and what it ignores.
fn read_facts(root: &Path) -> Facts {
    let branches = tc_git::worktree::branches(root);
    let last = branches
        .iter()
        .take(60)
        .filter_map(|b| tc_git::worktree::commit_line(root, b).map(|c| (b.clone(), c)))
        .collect();
    Facts { current: tc_git::worktree::current_branch(root), ignored: tc_git::worktree::ignored_top(root), branches, last }
}

/// Copies a file or a folder (and what is in it) to `to`, making the folders above it.
fn copy_path(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_path(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else if meta.file_type().is_symlink() {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(std::fs::read_link(from)?, to)
        }
        #[cfg(not(unix))]
        {
            std::fs::copy(from, to).map(|_| ())
        }
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// What a project never set up gives a new worktree: the `.env` files git ignores at its top
/// (a project rarely runs without them).
pub fn default_setup(ignored: Vec<String>) -> WorktreeSetup {
    WorktreeSetup { copy: ignored.into_iter().filter(|n| n.starts_with(".env") && !n.ends_with('/')).collect(), run: String::new() }
}

/// "Copies .env · runs nothing" (Run Agent's line about a project's setup).
pub fn setup_summary(setup: &WorktreeSetup) -> String {
    let copies = if setup.copy.is_empty() { t("Copies nothing").to_string() } else { tf("Copies {}", &[&setup.copy.join(", ")]) };
    let runs = if setup.run.trim().is_empty() { t("runs nothing").to_string() } else { tf("runs {}", &[setup.run.trim()]) };
    format!("{copies} · {runs}")
}

impl App {
    /// The project a worktree belongs to, else the project itself.
    pub fn root_project(&self, id: ProjectId) -> Option<&Project> {
        let p = self.project(id)?;
        match p.parent {
            Some(parent) => self.project(parent).or(Some(p)),
            None => Some(p),
        }
    }

    /// How a project is named where it is not under its parent: "ternary-engine › generics".
    pub fn project_label(&self, id: ProjectId) -> String {
        let Some(p) = self.project(id) else { return String::new() };
        match p.parent.and_then(|parent| self.project(parent)) {
            Some(parent) => format!("{} › {}", parent.name, p.name),
            None => p.name.clone(),
        }
    }

    /// The worktrees listed under a project.
    pub fn worktrees_of(&self, id: ProjectId) -> Vec<ProjectId> {
        self.projects.iter().filter(|p| p.parent == Some(id)).map(|p| p.id).collect()
    }

    /// What Run Agent needs to know about the repository at `root` (`None` while it is read,
    /// off the UI thread).
    pub fn worktree_facts(&mut self, root: &Path) -> Option<&Facts> {
        if !self.worktree_facts.contains_key(root) {
            self.worktree_facts.insert(root.to_path_buf(), None);
            let (tx, ctx, dir) = (self.app_tx.clone(), self.ctx.clone(), root.to_path_buf());
            self.rt.spawn_blocking(move || {
                let facts = read_facts(&dir);
                let _ = tx.send(AppMsg::WorktreeFacts(dir, facts));
                ctx.request_repaint();
            });
        }
        self.worktree_facts.get(root).and_then(Option::as_ref)
    }

    /// What a new worktree of a project gets: as set for it, else the `.env` files git ignores
    /// at its top (a project rarely runs without them).
    pub fn worktree_setup_of(&self, root: ProjectId) -> WorktreeSetup {
        let Some(p) = self.project(root) else { return WorktreeSetup::default() };
        if let Some(s) = &p.worktree_setup {
            return s.clone();
        }
        let ignored = self.worktree_facts.get(&p.path).and_then(Option::as_ref).map(|f| f.ignored.clone()).unwrap_or_default();
        default_setup(ignored)
    }

    /// Where a new worktree called `name` of `root` goes.
    pub fn worktree_dir(&self, root: &Project, name: &str) -> PathBuf {
        let folder = root.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "project".into());
        match self.settings.worktrees.place {
            WorktreePlace::Beside => root.path.parent().unwrap_or(&root.path).join(format!("{folder}.worktrees")).join(name),
            WorktreePlace::DataFolder => crate::paths::data_dir().join("worktrees").join(folder).join(name),
        }
    }

    /// The branch a worktree called `name` gets.
    pub fn worktree_branch(&self, name: &str) -> String {
        format!("{}{}", self.settings.worktrees.prefix.trim(), name.trim())
    }

    /// Why a new worktree of `root` cannot be called `name`, if it cannot.
    pub fn worktree_name_problem(&self, root: ProjectId, name: &str) -> Option<String> {
        let p = self.project(root)?;
        let name = name.trim();
        if name.is_empty() {
            return Some(t("Type a name.").into());
        }
        let branch = self.worktree_branch(name);
        if name.contains('/') || !tc_git::worktree::valid_branch(name) || !tc_git::worktree::valid_branch(&branch) {
            return Some(t("Use letters, digits, - _ and . only.").into());
        }
        let facts = self.worktree_facts.get(&p.path).and_then(Option::as_ref);
        if facts.is_some_and(|f| f.branches.contains(&branch)) {
            return Some(tf("The branch {} exists already.", &[&branch]));
        }
        let dir = self.worktree_dir(p, name);
        if dir.exists() || self.projects.iter().any(|x| x.path == dir) {
            return Some(tf("{} exists already.", &[&crate::paths::contract_tilde(&dir)]));
        }
        None
    }

    /// A free name for a new worktree of `root` made for an agent: `claude-1`, `claude-2`…
    pub fn suggest_worktree_name(&self, root: ProjectId, agent: &str) -> String {
        let stem: String = agent.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        let stem = stem.trim_matches('-');
        let stem = if stem.is_empty() { "agent" } else { stem };
        (1..1000).map(|n| format!("{stem}-{n}")).find(|n| self.worktree_name_problem(root, n).is_none()).unwrap_or_else(|| format!("{stem}-1"))
    }

    /// Makes the worktree off the UI thread (`git worktree add`, then the setup's files);
    /// [`App::on_worktree_made`] lists it and starts the agent in it.
    pub fn create_worktree(&mut self, root: ProjectId, preset: String, name: String, base: String) {
        let Some(p) = self.project(root) else { return };
        let name = name.trim().to_string();
        if let Some(problem) = self.worktree_name_problem(root, &name) {
            self.toast_err(tf("The worktree was not created: {}", &[&problem]));
            return;
        }
        let dir = self.worktree_dir(p, &name);
        let branch = self.worktree_branch(&name);
        let repo = p.path.clone();
        let saved = p.worktree_setup.clone();
        let (tx, ctx) = (self.app_tx.clone(), self.ctx.clone());
        self.rt.spawn_blocking(move || {
            // A project may be a folder inside its repository: the worktree holds the whole
            // repository, and the project is the same folder inside it.
            let inside = tc_git::repo_root(&repo)
                .and_then(|top| repo.strip_prefix(&top).ok().map(Path::to_path_buf))
                .unwrap_or_default();
            let path = dir.join(&inside);
            // Never set up: the defaults, from what git ignores there now.
            let setup = saved.unwrap_or_else(|| default_setup(tc_git::worktree::ignored_top(&repo)));
            let result = (|| {
                if let Some(parent) = dir.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let start = tc_git::worktree::add(&repo, &dir, &branch, &base)?;
                let mut copied = Vec::new();
                for item in &setup.copy {
                    let rel = item.trim().trim_end_matches('/');
                    if rel.is_empty() || Path::new(rel).is_absolute() || rel.split(['/', '\\']).any(|part| part == "..") {
                        continue;
                    }
                    if copy_path(&repo.join(rel), &path.join(rel)).is_ok() {
                        copied.push(rel.to_string());
                    }
                }
                Ok::<_, String>((start, copied))
            })();
            let _ = tx.send(AppMsg::WorktreeMade(Box::new(Made { root, name, path, branch, base, preset, result })));
            ctx.request_repaint();
        });
    }

    /// A worktree was made: it is listed under its project, opened, and its first tab runs the
    /// setup command and then the agent.
    pub fn on_worktree_made(&mut self, ctx: &egui::Context, made: Made) {
        let (start, copied) = match made.result {
            Ok(r) => r,
            Err(e) => {
                self.toast_err(tf("The worktree was not created: {}", &[&e]));
                return;
            }
        };
        let Some(color) = self.project(made.root).map(|p| p.color) else { return };
        let setup = self.worktree_setup_of(made.root);
        let path = crate::paths::canonical(&made.path);
        let id = self.new_id();
        let size = self.grid_in(&tc_term::layout::Layout::Pane(0), 0);
        let pane = self.new_pane(ctx, id, path.clone(), size);
        let mut project = Project::from_save(&ProjectSave {
            id,
            name: made.name.clone(),
            path,
            color,
            parent: Some(made.root),
            worktree: Some(WorktreeSave { branch: made.branch, base: made.base.clone(), start }),
            preset: Some(made.preset.clone()),
            ..Default::default()
        });
        project.tabs = vec![Tab { layout: tc_term::layout::Layout::Pane(pane), focused: pane, title: None }];
        self.watch_project(ctx, &project);
        self.projects.push(project);
        if let Some(parent) = self.project_mut(made.root) {
            parent.folded = false;
        }
        self.sort_projects();
        self.refresh_git(id);
        self.dispatch(ctx, Action::SelectProject(id));
        // The setup's command, then the agent (only when the setup worked).
        if let Some(preset) = self.presets().into_iter().find(|p| p.id == made.preset) {
            let profile = preset.agent.and_then(|a| self.preferred_profile_for(a, Some(id)));
            let agent = self.launch_command_for_profile(&preset, &preset.command, profile.as_ref());
            let run = setup.run.trim();
            let line = if run.is_empty() { agent } else { crate::ship::chain(&[run.to_string(), agent]) };
            if let Some(p) = self.panes.get_mut(&pane) {
                p.preset = Some(preset.id.clone());
                p.active_profile = profile.map(|pr| pr.id);
                p.type_command(line);
            }
        }
        let text = if copied.is_empty() {
            tf("Worktree {} created from {}", &[&made.name, &made.base])
        } else {
            tf("Worktree {} created from {} · copied {}", &[&made.name, &made.base, &copied.join(", ")])
        };
        self.toast_with(ToastKind::Worktree, text, None);
        self.mark_dirty();
    }

    /// A worktree's branch is merged now: once, offer to remove it (Settings → General →
    /// Worktrees can turn this off).
    pub fn offer_merged_removal(&mut self, id: ProjectId) {
        let offer = self.settings.worktrees.offer_remove;
        let Some(p) = self.project_mut(id) else { return };
        let Some(wt) = p.worktree.clone() else { return };
        if !offer || p.merge_offered || !p.git.as_ref().is_some_and(|g| g.merged) {
            return;
        }
        p.merge_offered = true;
        self.toast_with(ToastKind::Ok, tf("Merged {} into {}.", &[&wt.branch, &wt.base]), Some((t("Remove Worktree").into(), Action::RemoveWorktree(id))));
    }

    /// Removes a worktree: its panes are stopped and it leaves the list, then (off the UI
    /// thread) git removes its folder, and its branch when asked.
    pub fn remove_worktree_now(&mut self, id: ProjectId, delete_branch: bool) {
        let Some(p) = self.project(id) else { return };
        let (dir, name, wt, merged, missing) = (p.path.clone(), p.name.clone(), p.worktree.clone(), p.git.as_ref().is_some_and(|g| g.merged), p.missing);
        let repo = self.root_project(id).filter(|r| r.id != id).map(|r| r.path.clone());
        self.remove_project(id);
        let Some(repo) = repo else { return };
        let (tx, ctx) = (self.app_tx.clone(), self.ctx.clone());
        self.rt.spawn_blocking(move || {
            // The worktree's top (the project may be a folder inside it).
            let top = if missing { dir.clone() } else { tc_git::repo_root(&dir).unwrap_or_else(|| dir.clone()) };
            let mut result = tc_git::worktree::remove(&repo, &top, true);
            if result.is_ok()
                && delete_branch
                && let Some(wt) = &wt
            {
                result = tc_git::worktree::delete_branch(&repo, &wt.branch, !merged);
            }
            let _ = tx.send(AppMsg::WorktreeRemoved(name, result));
            ctx.request_repaint();
        });
    }

    /// Lists the files not committed in a worktree, off the UI thread (git's own summary counts
    /// a new folder once; Merge and Remove name each file).
    pub fn list_worktree_dirty(&mut self, id: ProjectId) {
        let Some(dir) = self.project(id).map(|p| p.path.clone()) else { return };
        let (tx, ctx) = (self.app_tx.clone(), self.ctx.clone());
        self.rt.spawn_blocking(move || {
            let _ = tx.send(AppMsg::WorktreeDirty(id, tc_git::worktree::dirty_files(&dir)));
            ctx.request_repaint();
        });
    }

    /// The files not committed in a worktree: as listed for it, else from its git summary.
    pub fn worktree_dirty_files(&self, id: ProjectId) -> Vec<String> {
        if let Some(files) = self.worktree_dirty.get(&id) {
            return files.clone();
        }
        let Some(p) = self.project(id) else { return Vec::new() };
        let mut files: Vec<String> = p
            .git
            .as_ref()
            .map(|g| g.changes.keys().filter_map(|f| f.strip_prefix(&p.path).ok()).map(|f| f.to_string_lossy().replace('\\', "/")).collect())
            .unwrap_or_default();
        files.sort();
        files
    }

    /// Suggests a clean default commit message when committing worktree changes before a merge:
    /// prefers any drafted Ship message, then the worktree agent's task/summary, else the
    /// worktree's name humanized (e.g. `update-readme-2` → `Update readme 2`).
    pub fn default_merge_commit_message(&self, id: ProjectId) -> String {
        if let Some(draft) = self.ship.get(&id).map(|s| s.message.trim()).filter(|m| !m.is_empty())
            && let Some(first) = draft.lines().next().map(str::trim).filter(|l| !l.is_empty())
        {
            return first.to_string();
        }
        if let Some((pane_id, _)) = self.ship_agent_pane(id)
            && let Some(pane) = self.panes.get(&pane_id)
        {
            let drafted = crate::ship::message_from(pane.task.as_deref(), pane.last_words.as_deref());
            if let Some(first) = drafted.lines().next().map(str::trim).filter(|l| !l.is_empty()) {
                return first.to_string();
            }
        }
        let name = self.project(id).map(|p| p.name.as_str()).unwrap_or("worktree");
        humanize_worktree_name(name)
    }

    /// Commits all uncommitted files in the worktree off the UI thread, then notifies
    /// [`App::on_worktree_committed_for_merge`] to proceed into `git merge`.
    pub fn commit_and_merge_worktree(&mut self, id: ProjectId, message: String) {
        let Some(dir) = self.project(id).filter(|p| p.worktree.is_some()).map(|p| p.path.clone()) else {
            return;
        };
        let (tx, ctx) = (self.app_tx.clone(), self.ctx.clone());
        self.rt.spawn_blocking(move || {
            let result = tc_git::worktree::commit_all(&dir, &message);
            let _ = tx.send(AppMsg::WorktreeCommittedForMerge(id, result));
            ctx.request_repaint();
        });
    }

    /// Handles completion of [`App::commit_and_merge_worktree`]: on success closes the dialog and
    /// runs `git merge` in the parent project; on failure shows the git/hook error inside the dialog.
    pub fn on_worktree_committed_for_merge(&mut self, ctx: &egui::Context, id: ProjectId, result: Result<String, String>) {
        let name = self.project(id).map(|p| p.name.clone()).unwrap_or_default();
        let waiting_in_dialog = matches!(
            &self.dialog,
            Some(crate::app::Dialog::MergeWorktree { id: did, committing: true, .. }) if *did == id
        );
        match result {
            Ok(short) => {
                self.worktree_dirty.remove(&id);
                self.refresh_git(id);
                self.toast_with(ToastKind::Ok, tf("Committed {} in {}.", &[&short, &name]), None);
                if waiting_in_dialog {
                    ctx.data_mut(|d| d.remove::<(usize, u32, u32)>(egui::Id::new(("merge-stat", id))));
                    self.dialog = None;
                    self.merge_worktree_now(ctx, id);
                }
            }
            Err(err) => {
                if let Some(crate::app::Dialog::MergeWorktree { id: did, committing, error, .. }) = &mut self.dialog
                    && *did == id
                {
                    *committing = false;
                    *error = Some(err);
                } else {
                    self.toast_err(tf("Commit failed: {}", &[&err]));
                }
            }
        }
    }

    /// Types `git merge --no-ff --no-edit ‹branch›` in a new tab of the worktree's project, so the
    /// merge and any conflict are seen and handled in a terminal (git's own message, no editor
    /// opening in the pane).
    pub fn merge_worktree_now(&mut self, ctx: &egui::Context, id: ProjectId) {
        let Some((parent, branch)) = self.project(id).and_then(|p| Some((p.parent?, p.worktree.as_ref()?.branch.clone()))) else {
            return;
        };
        self.dispatch(ctx, Action::SelectProject(parent));
        self.dispatch(ctx, Action::NewTab);
        if let Some(pane) = self.current_pane()
            && let Some(p) = self.panes.get_mut(&pane)
        {
            p.type_command(format!("git merge --no-ff --no-edit {branch}"));
        }
    }
}

/// Turns a slug like `update-readme-2` or `fix_auth` into a clean commit subject (`Update readme 2`).
fn humanize_worktree_name(name: &str) -> String {
    let words: Vec<&str> = name.split(['-', '_', '/']).filter(|s| !s.is_empty()).collect();
    if words.is_empty() {
        return "Worktree changes".to_string();
    }
    let joined = words.join(" ");
    let mut chars = joined.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => "Worktree changes".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::humanize_worktree_name;

    #[test]
    fn humanizes_worktree_slugs_into_commit_subjects() {
        assert_eq!(humanize_worktree_name("update-readme-2"), "Update readme 2");
        assert_eq!(humanize_worktree_name("fix_auth_flow"), "Fix auth flow");
        assert_eq!(humanize_worktree_name("feat"), "Feat");
        assert_eq!(humanize_worktree_name("---"), "Worktree changes");
    }
}
