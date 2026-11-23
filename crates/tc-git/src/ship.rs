//! Committing changes, pushing their branch and opening a pull request: what the commit box needs
//! to know about a branch, and the steps it runs.
//!
//! Push and `gh` run without a terminal. git is told never to prompt (`GIT_TERMINAL_PROMPT=0`,
//! ssh in batch mode), so a step that needs a password or a passphrase stops at once with an
//! error instead of waiting for an answer nobody sees; the app then offers a terminal. Every
//! function blocks until its program exits: call them off the UI thread.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{git, git_dir_path, literal_pathspec, rev_parse, run, trim_line};

/// How long a push, fetch, `gh`, or `glab` call may take before it is given up.
pub(crate) const NETWORK_TIMEOUT: Duration = Duration::from_secs(180);

/// The site a remote lives on, for its pull/merge request form, `gh`, and `glab`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forge {
    GitHub,
    GitLab,
}

impl Forge {
    pub fn label(self) -> &'static str {
        match self {
            Forge::GitHub => "GitHub",
            Forge::GitLab => "GitLab",
        }
    }

    pub fn cli_name(self) -> &'static str {
        match self {
            Forge::GitHub => "gh",
            Forge::GitLab => "glab",
        }
    }

    pub fn request_short(self) -> &'static str {
        match self {
            Forge::GitHub => "PR",
            Forge::GitLab => "MR",
        }
    }

    pub fn request_prefix(self) -> &'static str {
        match self {
            Forge::GitHub => "#",
            Forge::GitLab => "!",
        }
    }
}

/// Where a repository's pages are: `github.com` / `gitlab.com` and `owner/repo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Web {
    pub forge: Forge,
    pub host: String,
    /// `owner/repo` (on GitLab, groups may nest: `group/sub/repo`).
    pub repo: String,
}

impl Web {
    pub fn url(&self) -> String {
        format!("https://{}/{}", self.host, self.repo)
    }

    /// The repository as `gh -R` takes it: `owner/repo`, or `host/owner/repo` off github.com.
    pub fn gh_repo(&self) -> String {
        if self.host == "github.com" { self.repo.clone() } else { format!("{}/{}", self.host, self.repo) }
    }

    /// The repository as `glab -R` takes it: `group/repo`, or full URL off gitlab.com so
    /// self-hosted instances and dotted group names are never misparsed by `glrepo.FromFullName`.
    pub fn glab_repo(&self) -> String {
        if self.host == "gitlab.com" { self.repo.clone() } else { self.url() }
    }

    /// The repository identifier formatted for this forge's CLI (`gh` or `glab`).
    pub fn cli_repo(&self) -> String {
        match self.forge {
            Forge::GitHub => self.gh_repo(),
            Forge::GitLab => self.glab_repo(),
        }
    }

    /// Web page for viewing one commit on GitHub or GitLab.
    pub fn commit_url(&self, hash: &str) -> String {
        match self.forge {
            Forge::GitHub => format!("{}/commit/{}", self.url(), encode(hash, false)),
            Forge::GitLab => format!("{}/-/commit/{}", self.url(), encode(hash, false)),
        }
    }

    /// Web page for browsing a branch on GitHub or GitLab.
    pub fn branch_url(&self, branch: &str) -> String {
        match self.forge {
            Forge::GitHub => format!("{}/tree/{}", self.url(), encode(branch, true)),
            Forge::GitLab => format!("{}/-/tree/{}", self.url(), encode(branch, true)),
        }
    }

    /// Web page listing Pull Requests (GitHub) or Merge Requests (GitLab).
    pub fn prs_url(&self) -> String {
        match self.forge {
            Forge::GitHub => format!("{}/pulls", self.url()),
            Forge::GitLab => format!("{}/-/merge_requests", self.url()),
        }
    }

    /// Web page for GitHub Actions or GitLab CI/CD Pipelines.
    pub fn actions_url(&self) -> String {
        match self.forge {
            Forge::GitHub => format!("{}/actions", self.url()),
            Forge::GitLab => format!("{}/-/pipelines", self.url()),
        }
    }

    /// The site's form for a new pull (merge) request from `branch` into `base`, filled in.
    pub fn new_pr_url(&self, base: &str, branch: &str, title: &str, body: &str) -> String {
        // A browser takes a few thousand characters of address; the description is cut first.
        let body = cut_chars(body, 1500);
        match self.forge {
            Forge::GitHub => format!(
                "{}/compare/{}...{}?expand=1&title={}&body={}",
                self.url(),
                encode(base, true),
                encode(branch, true),
                encode(title, false),
                encode(&body, false)
            ),
            Forge::GitLab => format!(
                "{}/-/merge_requests/new?merge_request%5Bsource_branch%5D={}&merge_request%5Btarget_branch%5D={}&merge_request%5Btitle%5D={}&merge_request%5Bdescription%5D={}",
                self.url(),
                encode(branch, false),
                encode(base, false),
                encode(title, false),
                encode(&body, false)
            ),
        }
    }
}

/// The remote a branch is pushed to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub url: String,
    pub web: Option<Web>,
}

/// An operation git is in the middle of: nothing can be committed until it is finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

/// What the commit box needs to know about the branch a folder is on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// `None`: HEAD is detached.
    pub branch: Option<String>,
    /// The repository has a commit (a new one has none yet).
    pub has_commits: bool,
    /// Where the branch goes: its own remote, else `origin`, else the first one.
    pub remote: Option<Remote>,
    /// The branch it tracks (`origin/feature/x`), once it was pushed with `-u`.
    pub upstream: Option<String>,
    /// Commits on HEAD that the remote has on none of its branches: what a push would send.
    pub unpushed: usize,
    /// Commits on the upstream that HEAD does not have.
    pub behind: usize,
    /// The remote's default branch (`main`), the base of a pull request.
    pub default_branch: Option<String>,
    pub busy: Option<Busy>,
    /// Files with merge conflicts.
    pub conflicts: Vec<String>,
}

/// Reads the facts of the branch checked out at `root`.
pub fn facts(root: &Path) -> Facts {
    let branch = super::worktree::current_branch(root);
    let has_commits = rev_parse(root, "HEAD").is_some();
    let names: Vec<String> = text(run(git(root).arg("remote")).ok())
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let own = branch.as_ref().and_then(|b| config(root, &format!("branch.{b}.remote")));
    let name = own
        .filter(|n| names.contains(n))
        .or_else(|| names.iter().find(|n| *n == "origin").cloned())
        .or_else(|| names.first().cloned());
    let remote = name.map(|name| {
        // Its pages from where it is fetched (a push address may be a mirror's).
        let url = text(run(git(root).args(["remote", "get-url", &name])).ok()).trim().to_string();
        let web = parse_remote_url(&url);
        Remote { name, url, web }
    });
    let upstream = run(git(root).args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]))
        .ok()
        .map(|out| String::from_utf8_lossy(trim_line(&out)).into_owned())
        .filter(|u| !u.is_empty());
    let count = |args: &[&str]| -> usize { text(run(git(root).args(args)).ok()).trim().parse().unwrap_or(0) };
    let unpushed = match (&remote, has_commits) {
        (Some(r), true) => count(&["rev-list", "--count", "HEAD", "--not", &format!("--remotes={}", r.name)]),
        _ => 0,
    };
    let behind = if upstream.is_some() { count(&["rev-list", "--count", "HEAD..@{upstream}"]) } else { 0 };
    let default_branch = remote.as_ref().and_then(|r| default_branch(root, &r.name));
    let busy = busy(root);
    let conflicts = text(run(git(root).args(["diff", "--name-only", "--diff-filter=U"])).ok())
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    Facts { branch, has_commits, remote, upstream, unpushed, behind, default_branch, busy, conflicts }
}

/// The branch `refs/remotes/<remote>/HEAD` points to, else `main` or `master` when the remote
/// has one.
fn default_branch(root: &Path, remote: &str) -> Option<String> {
    let head = run(git(root).args(["symbolic-ref", "--short", "-q", &format!("refs/remotes/{remote}/HEAD")])).ok();
    if let Some(head) = head {
        let head = String::from_utf8_lossy(trim_line(&head)).into_owned();
        if let Some(name) = head.strip_prefix(&format!("{remote}/")).filter(|n| !n.is_empty()) {
            return Some(name.to_string());
        }
    }
    ["main", "master"]
        .into_iter()
        .find(|b| rev_parse(root, &format!("refs/remotes/{remote}/{b}")).is_some())
        .map(String::from)
}

fn busy(root: &Path) -> Option<Busy> {
    let exists = |name: &str| git_dir_path(root, name).is_ok_and(|p| p.exists());
    if exists("rebase-merge") || exists("rebase-apply") {
        Some(Busy::Rebase)
    } else if exists("MERGE_HEAD") {
        Some(Busy::Merge)
    } else if exists("CHERRY_PICK_HEAD") {
        Some(Busy::CherryPick)
    } else if exists("REVERT_HEAD") {
        Some(Busy::Revert)
    } else {
        None
    }
}

fn config(root: &Path, key: &str) -> Option<String> {
    let out = run(git(root).args(["config", "--get", key])).ok()?;
    let v = String::from_utf8_lossy(trim_line(&out)).into_owned();
    (!v.is_empty()).then_some(v)
}

fn text(out: Option<Vec<u8>>) -> String {
    out.map(|o| String::from_utf8_lossy(&o).into_owned()).unwrap_or_default()
}

/// Commits the working tree's content of `paths` (relative to the root; a renamed file's old
/// path too) with `message`. Only those paths: what else is staged stays staged, uncommitted.
/// Hooks run as usual. Returns the new commit's short id; on failure, what git and the hooks
/// printed.
pub fn commit(root: &Path, paths: &[String], message: &str) -> Result<String, String> {
    if paths.is_empty() {
        return Err("Nothing to commit.".to_string());
    }
    let specs: Vec<String> = paths.iter().map(|p| literal_pathspec(p)).collect();
    let mut add = git(root);
    add.args(["add", "-A", "--"]).args(&specs);
    output_of(&mut add, None, None)?;
    let mut commit = git(root);
    // `--cleanup=whitespace` preserves lines starting with `#` (e.g. `#123 Fix auth`), since GUI
    // messages never include git's commented editor template.
    commit.args(["commit", "--cleanup=whitespace", "-F", "-", "--"]).args(&specs);
    output_of(&mut commit, Some(message.as_bytes()), None)?;
    let out = run(git(root).args(["rev-parse", "--short", "HEAD"]))?;
    Ok(String::from_utf8_lossy(trim_line(&out)).into_owned())
}

/// Makes a branch at HEAD and switches to it; the working tree's changes come along.
pub fn create_branch(root: &Path, name: &str) -> Result<(), String> {
    if !super::worktree::valid_branch(name) {
        return Err(format!("“{name}” can’t be a branch name."));
    }
    output_of(git(root).args(["switch", "-c", name]), None, None).map(|_| ())
}

/// Why a push stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushError {
    /// The remote has commits this branch does not: it must be pulled first.
    Rejected(String),
    /// A password, a token or a key's passphrase is needed (it can only be typed in a terminal).
    Credentials(String),
    Failed(String),
}

impl PushError {
    pub fn output(&self) -> &str {
        match self {
            PushError::Rejected(s) | PushError::Credentials(s) | PushError::Failed(s) => s,
        }
    }
}

/// Pushes `branch` to `remote` (with `-u` when it tracks nothing yet). `path` is the PATH to run
/// with (credential helpers such as `gh` are found on it).
pub fn push(root: &Path, remote: &str, branch: &str, set_upstream: bool, path: Option<&OsStr>) -> Result<(), PushError> {
    if remote.is_empty() || remote.starts_with('-') || branch.is_empty() || branch.starts_with('-') {
        return Err(PushError::Failed("Invalid remote or branch name.".into()));
    }
    let mut cmd = git(root);
    no_prompts(&mut cmd, root);
    cmd.arg("push");
    if set_upstream {
        cmd.arg("-u");
    }
    cmd.arg("--").args([remote, branch]);
    if let Some(path) = path {
        cmd.env("PATH", path);
    }
    match output_of(&mut cmd, None, Some(NETWORK_TIMEOUT)) {
        Ok(_) => Ok(()),
        Err(out) => Err(classify_push(out)),
    }
}

/// git without prompts: none of its own, none of ssh's (unless the user set their own ssh
/// command, which is left alone), none of Git Credential Manager's.
pub(crate) fn no_prompts(cmd: &mut Command, root: &Path) {
    cmd.env("GIT_TERMINAL_PROMPT", "0").env("GCM_INTERACTIVE", "never");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() && config(root, "core.sshCommand").is_none() {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
}

pub fn classify_push(out: String) -> PushError {
    let l = out.to_lowercase();
    if l.contains("[rejected]") || l.contains("fetch first") || l.contains("non-fast-forward") || l.contains("updates were rejected") {
        PushError::Rejected(out)
    } else if [
        "terminal prompts disabled",
        "could not read username",
        "could not read password",
        "authentication failed",
        "permission denied (publickey",
        "host key verification failed",
        "batchmode",
        "passphrase",
        "invalid username or password",
        "403",
    ]
    .iter()
    .any(|w| l.contains(w))
    {
        PushError::Credentials(out)
    } else {
        PushError::Failed(out)
    }
}

/// The state of a pull request's checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checks {
    None,
    Pending,
    Passed,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pr {
    pub number: u64,
    pub url: String,
    pub state: PrState,
    pub checks: Checks,
}

/// Why `gh` did not answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhError {
    /// Not signed in to the host (`gh auth login`).
    SignedOut(String),
    Failed(String),
}

impl GhError {
    pub fn output(&self) -> &str {
        match self {
            GhError::SignedOut(s) | GhError::Failed(s) => s,
        }
    }
}

/// What `gh pr view --json` is asked for, as one tab-separated line: number, address, state,
/// draft or not, and each check's outcome.
const PR_JQ: &str = r#"[.number, .url, .state, .isDraft, ([.statusCheckRollup[]? | (if (.conclusion // "") != "" then .conclusion elif (.state // "") != "" then .state else (.status // "") end)] | join(","))] | @tsv"#;

fn gh(gh: &Path, root: &Path, path: Option<&OsStr>) -> Command {
    let mut cmd = Command::new(gh);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
    cmd.current_dir(root)
        .stdin(Stdio::null())
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_SPINNER_DISABLED", "1")
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env("GH_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(path) = path {
        cmd.env("PATH", path);
    }
    cmd
}

fn gh_error(out: String) -> GhError {
    let l = out.to_lowercase();
    if l.contains("gh auth login") || l.contains("not logged in") || l.contains("authentication required") || l.contains("bad credentials") {
        GhError::SignedOut(out)
    } else {
        GhError::Failed(out)
    }
}

/// The pull request whose head is `branch`, if there is one (`gh` at the path given).
pub fn find_pr(gh_bin: &Path, root: &Path, web: &Web, branch: &str, path: Option<&OsStr>) -> Result<Option<Pr>, GhError> {
    let mut cmd = gh(gh_bin, root, path);
    cmd.args(["pr", "view", branch, "-R", &web.gh_repo(), "--json", "number,url,state,isDraft,statusCheckRollup", "--jq", PR_JQ]);
    match output_of(&mut cmd, None, Some(NETWORK_TIMEOUT)) {
        Ok(out) => Ok(parse_pr_line(&out)),
        Err(out) if out.to_lowercase().contains("no pull requests found") => Ok(None),
        Err(out) => Err(gh_error(out)),
    }
}

/// Opens a pull request from `branch` into `base` with `gh pr create`. One that already exists
/// for the branch is returned instead.
#[allow(clippy::too_many_arguments)]
pub fn create_pr(gh_bin: &Path, root: &Path, web: &Web, base: &str, branch: &str, title: &str, body: &str, draft: bool, path: Option<&OsStr>) -> Result<Pr, GhError> {
    let mut cmd = gh(gh_bin, root, path);
    cmd.args(["pr", "create", "-R", &web.gh_repo(), "--base", base, "--head", branch, "--title", title, "--body-file", "-"]);
    if draft {
        cmd.arg("--draft");
    }
    let state = if draft { PrState::Draft } else { PrState::Open };
    match output_of(&mut cmd, Some(body.as_bytes()), Some(NETWORK_TIMEOUT)) {
        Ok(out) => pr_from_url(&out, state).ok_or(GhError::Failed(out)),
        Err(out) if out.contains("already exists") => pr_from_url(&out, PrState::Open).ok_or(GhError::Failed(out)),
        Err(out) => Err(gh_error(out)),
    }
}

/// An open pull request (GitHub) or merge request (GitLab) listed from the remote forge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemotePr {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub branch: String,
    pub author: String,
    pub state: PrState,
    pub checks: Checks,
}

/// The pull/merge request named by the last `…/pull/N` or `…/merge_requests/N` address in `text`.
fn pr_from_url(text: &str, state: PrState) -> Option<Pr> {
    text.split_whitespace().rev().find_map(|raw| {
        let trimmed = raw
            .trim_start_matches(['(', '[', '<', '"', '\''])
            .trim_end_matches(['.', ',', ')', ']', '>', '"', '\'']);
        if !(trimmed.starts_with("https://") || trimmed.starts_with("http://"))
            || !(trimmed.contains("/pull/") || trimmed.contains("/merge_requests/"))
        {
            return None;
        }
        let after = trimmed
            .rsplit_once("/pull/")
            .or_else(|| trimmed.rsplit_once("/merge_requests/"))?
            .1;
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        let number = digits.parse().ok()?;
        let clean_url = trimmed.split(['?', '#']).next().unwrap_or(trimmed).to_string();
        Some(Pr { number, url: clean_url, state, checks: Checks::None })
    })
}

fn glab(glab_bin: &Path, root: &Path, web: &Web, path: Option<&OsStr>) -> Command {
    let mut cmd = Command::new(glab_bin);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
    cmd.current_dir(root)
        .stdin(Stdio::null())
        .env("GLAB_NO_PROMPT", "1")
        .env("NO_PROMPT", "1")
        .env("PROMPT_DISABLED", "1")
        .env("GLAB_CHECK_UPDATE", "false")
        .env("GLAB_PAGER", "cat")
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env("PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0");
    if web.host != "gitlab.com" {
        cmd.env("GITLAB_HOST", &web.host);
    }
    if let Some(path) = path {
        cmd.env("PATH", path);
    }
    cmd
}

fn glab_error(out: String) -> GhError {
    let l = out.to_lowercase();
    if l.contains("glab auth login")
        || l.contains("not logged in")
        || l.contains("no token found")
        || l.contains("401 unauthorized")
        || l.contains("status code 401")
        || l.contains("token is expired")
        || l.contains("token has expired")
        || l.contains("bad credentials")
        || l.contains("invalid_token")
        || l.contains("authentication required")
    {
        GhError::SignedOut(out)
    } else {
        GhError::Failed(out)
    }
}

/// Maps GitLab pipeline status strings (`success`, `failed`, `running`, `pending`, …) to [`Checks`].
pub fn classify_gitlab_pipeline(status: &str) -> Checks {
    match status.trim().to_ascii_lowercase().as_str() {
        "" => Checks::None,
        "success" => Checks::Passed,
        "failed" | "canceled" | "cancelled" => Checks::Failed,
        "running" | "pending" | "created" | "waiting_for_resource" | "preparing" | "scheduled" => Checks::Pending,
        _ => Checks::None,
    }
}

/// The merge request whose source branch is `branch`, if there is one (`glab` at the path given).
pub fn find_mr(glab_bin: &Path, root: &Path, web: &Web, branch: &str, path: Option<&OsStr>) -> Result<Option<Pr>, GhError> {
    let mut cmd = glab(glab_bin, root, web, path);
    cmd.args(["mr", "view", branch, "-R", &web.glab_repo(), "-F", "json"]);
    match output_of(&mut cmd, None, Some(NETWORK_TIMEOUT)) {
        Ok(out) => Ok(parse_glab_mr_json(&out)),
        Err(out) => {
            if let GhError::SignedOut(e) = glab_error(out.clone()) {
                return Err(GhError::SignedOut(e));
            }
            let l = out.to_lowercase();
            if l.contains("no open merge request")
                || l.contains("no merge request")
                || l.contains("merge request not found")
            {
                Ok(None)
            } else {
                Err(glab_error(out))
            }
        }
    }
}

/// Opens a GitLab merge request from `branch` into `base` with `glab mr create`.
#[allow(clippy::too_many_arguments)]
pub fn create_mr(
    glab_bin: &Path,
    root: &Path,
    web: &Web,
    base: &str,
    branch: &str,
    title: &str,
    body: &str,
    draft: bool,
    path: Option<&OsStr>,
) -> Result<Pr, GhError> {
    let mut cmd = glab(glab_bin, root, web, path);
    // Avoid `glab` treating `--description "-"` as an interactive `$EDITOR` sentinel.
    let safe_body = if body.trim() == "-" { "- " } else { body };
    cmd.args([
        "mr",
        "create",
        "-R",
        &web.glab_repo(),
        "--source-branch",
        branch,
        "--target-branch",
        base,
        "--title",
        title,
        "--description",
        safe_body,
        "--no-editor",
        "--yes",
    ]);
    if draft {
        cmd.arg("--draft");
    }
    let state = if draft { PrState::Draft } else { PrState::Open };
    match output_of(&mut cmd, None, Some(NETWORK_TIMEOUT)) {
        Ok(out) => pr_from_url(&out, state).ok_or(GhError::Failed(out)),
        Err(out) if out.to_lowercase().contains("already exists") => {
            pr_from_url(&out, PrState::Open)
                .or_else(|| find_mr(glab_bin, root, web, branch, path).ok().flatten())
                .ok_or(GhError::Failed(out))
        }
        Err(out) => Err(glab_error(out)),
    }
}

/// Finds the open/closed PR (GitHub) or MR (GitLab) for `branch` using the forge's CLI.
pub fn find_request(cli_bin: &Path, root: &Path, web: &Web, branch: &str, path: Option<&OsStr>) -> Result<Option<Pr>, GhError> {
    match web.forge {
        Forge::GitHub => find_pr(cli_bin, root, web, branch, path),
        Forge::GitLab => find_mr(cli_bin, root, web, branch, path),
    }
}

/// Creates a PR (GitHub) or MR (GitLab) using the forge's CLI.
#[allow(clippy::too_many_arguments)]
pub fn create_request(
    cli_bin: &Path,
    root: &Path,
    web: &Web,
    base: &str,
    branch: &str,
    title: &str,
    body: &str,
    draft: bool,
    path: Option<&OsStr>,
) -> Result<Pr, GhError> {
    match web.forge {
        Forge::GitHub => create_pr(cli_bin, root, web, base, branch, title, body, draft, path),
        Forge::GitLab => create_mr(cli_bin, root, web, base, branch, title, body, draft, path),
    }
}

/// `gh pr list --jq` output: tab-separated open PR summaries.
const PR_LIST_JQ: &str = r#".[] | [.number, .url, .isDraft, .headRefName, (.author.login // ""), ([.statusCheckRollup[]? | (if (.conclusion // "") != "" then .conclusion elif (.state // "") != "" then .state else (.status // "") end)] | join(",")), (.title | gsub("[\t\r\n]+"; " "))] | @tsv"#;

/// Lists open Pull Requests (GitHub via `gh`) or Merge Requests (GitLab via `glab`).
pub fn list_requests(
    cli_bin: &Path,
    root: &Path,
    web: &Web,
    limit: usize,
    path: Option<&OsStr>,
) -> Result<Vec<RemotePr>, GhError> {
    let lim = limit.clamp(1, 30).to_string();
    match web.forge {
        Forge::GitHub => {
            let mut cmd = gh(cli_bin, root, path);
            cmd.args([
                "pr",
                "list",
                "-R",
                &web.gh_repo(),
                "--state",
                "open",
                "--limit",
                &lim,
                "--json",
                "number,title,url,headRefName,author,isDraft,statusCheckRollup",
                "--jq",
                PR_LIST_JQ,
            ]);
            match output_of(&mut cmd, None, Some(NETWORK_TIMEOUT)) {
                Ok(out) => Ok(parse_gh_pr_list(&out)),
                Err(out) => Err(gh_error(out)),
            }
        }
        Forge::GitLab => {
            let mut cmd = glab(cli_bin, root, web, path);
            cmd.args(["mr", "list", "-R", &web.glab_repo(), "--per-page", &lim, "-F", "json"]);
            match output_of(&mut cmd, None, Some(NETWORK_TIMEOUT)) {
                Ok(out) => Ok(parse_glab_mr_list_json(&out)),
                Err(out) => Err(glab_error(out)),
            }
        }
    }
}

/// Unescapes `jq @tsv` escape sequences (`\\`, `\t`, `\n`, `\r`).
fn unescape_tsv(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('t') => out.push('\t'),
                Some('n') => out.push(' '),
                Some('r') => {}
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parses TSV lines emitted by `PR_LIST_JQ`.
pub fn parse_gh_pr_list(out: &str) -> Vec<RemotePr> {
    out.lines()
        .filter_map(|line| {
            let line = line.trim_end_matches(['\r', '\n']);
            if line.trim().is_empty() {
                return None;
            }
            let mut parts = line.splitn(7, '\t');
            let number = parts.next()?.trim().parse().ok()?;
            let url = parts.next()?.trim().to_string();
            let draft = parts.next().unwrap_or("").trim() == "true";
            let branch = unescape_tsv(parts.next().unwrap_or("").trim());
            let author = unescape_tsv(parts.next().unwrap_or("").trim());
            let checks = classify_checks(parts.next().unwrap_or("").trim());
            let title = unescape_tsv(parts.next().unwrap_or("").trim());
            Some(RemotePr {
                number,
                title,
                url,
                branch,
                author,
                state: if draft { PrState::Draft } else { PrState::Open },
                checks,
            })
        })
        .collect()
}

/// Parses one GitLab Merge Request JSON object from `glab mr view -F json`.
pub fn parse_glab_mr_json(json: &str) -> Option<Pr> {
    let val = parse_json(json)?;
    glab_obj_to_pr(&val)
}

/// Parses a JSON array of GitLab Merge Requests from `glab mr list -F json`.
pub fn parse_glab_mr_list_json(json: &str) -> Vec<RemotePr> {
    let Some(JsonVal::Arr(items)) = parse_json(json) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|obj| {
            let pr = glab_obj_to_pr(obj)?;
            let title = obj.str_field("title").unwrap_or("").to_string();
            let branch = obj.str_field("source_branch").unwrap_or("").to_string();
            let author = obj
                .field("author")
                .and_then(|a| a.str_field("username").or_else(|| a.str_field("name")))
                .unwrap_or("")
                .to_string();
            Some(RemotePr {
                number: pr.number,
                title,
                url: pr.url,
                branch,
                author,
                state: pr.state,
                checks: pr.checks,
            })
        })
        .collect()
}

fn glab_obj_to_pr(obj: &JsonVal) -> Option<Pr> {
    let number = obj.u64_field("iid")?;
    let url = obj.str_field("web_url")?.to_string();
    let draft = obj.bool_field("draft").unwrap_or(false) || obj.bool_field("work_in_progress").unwrap_or(false);
    let state = match obj.str_field("state").unwrap_or("").to_ascii_lowercase().as_str() {
        "merged" => PrState::Merged,
        "closed" => PrState::Closed,
        _ if draft => PrState::Draft,
        _ => PrState::Open,
    };
    let pipe = obj
        .field("head_pipeline")
        .and_then(|p| p.str_field("status"))
        .or_else(|| obj.field("pipeline").and_then(|p| p.str_field("status")))
        .unwrap_or("");
    Some(Pr { number, url, state, checks: classify_gitlab_pipeline(pipe) })
}

// ----- minimal dependency-free JSON reader (for `glab -F json`) ----------------------------

#[derive(Debug)]
enum JsonVal {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<JsonVal>),
    Obj(Vec<(String, JsonVal)>),
}

impl JsonVal {
    fn field(&self, key: &str) -> Option<&JsonVal> {
        let JsonVal::Obj(pairs) = self else { return None };
        pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    fn str_field(&self, key: &str) -> Option<&str> {
        match self.field(key)? {
            JsonVal::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }
    fn bool_field(&self, key: &str) -> Option<bool> {
        match self.field(key)? {
            JsonVal::Bool(b) => Some(*b),
            _ => None,
        }
    }
    fn u64_field(&self, key: &str) -> Option<u64> {
        match self.field(key)? {
            JsonVal::Num(n) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n <= (u64::MAX as f64) => Some(*n as u64),
            _ => None,
        }
    }
}

const JSON_MAX_DEPTH: usize = 128;

fn parse_json(src: &str) -> Option<JsonVal> {
    let mut p = JsonParser { bytes: src.as_bytes(), pos: 0, depth: 0 };
    if let Some(v) = p.value() {
        return Some(v);
    }
    // Some CLI builds print warning lines (like `[WARNING] ...`) before JSON: scan line starts
    // that begin with `{` or `[` and return the first valid JSON value.
    let mut offset = 0usize;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            let lead = line.len() - trimmed.len();
            let mut p2 = JsonParser { bytes: src.as_bytes(), pos: offset + lead, depth: 0 };
            if let Some(v) = p2.value() {
                return Some(v);
            }
        }
        offset += line.len();
    }
    None
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> JsonParser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }
    fn value(&mut self) -> Option<JsonVal> {
        if self.depth >= JSON_MAX_DEPTH {
            return None;
        }
        self.skip_ws();
        match self.peek()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(JsonVal::Str),
            b't' => self.literal(b"true", JsonVal::Bool(true)),
            b'f' => self.literal(b"false", JsonVal::Bool(false)),
            b'n' => self.literal(b"null", JsonVal::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }
    fn literal(&mut self, lit: &[u8], val: JsonVal) -> Option<JsonVal> {
        if self.bytes.get(self.pos..self.pos + lit.len()) == Some(lit) {
            self.pos += lit.len();
            Some(val)
        } else {
            None
        }
    }
    fn number(&mut self) -> Option<JsonVal> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos]).ok()?;
        let n = s.parse::<f64>().ok()?;
        n.is_finite().then_some(JsonVal::Num(n))
    }
    fn hex4(&mut self) -> Option<u16> {
        let hex = std::str::from_utf8(self.bytes.get(self.pos..self.pos + 4)?).ok()?;
        self.pos += 4;
        u16::from_str_radix(hex, 16).ok()
    }
    fn string(&mut self) -> Option<String> {
        if self.peek() != Some(b'"') {
            return None;
        }
        self.pos += 1;
        let mut out = String::new();
        while let Some(b) = self.peek() {
            self.pos += 1;
            match b {
                b'"' => return Some(out),
                b'\\' => {
                    let esc = self.peek()?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.hex4()?;
                            if (0xD800..=0xDBFF).contains(&code) {
                                // UTF-16 high surrogate: consume matching `\uXXXX` low surrogate if present.
                                if self.bytes.get(self.pos..self.pos + 2) == Some(b"\\u") {
                                    let saved = self.pos;
                                    self.pos += 2;
                                    if let Some(low) = self.hex4()
                                        && (0xDC00..=0xDFFF).contains(&low)
                                    {
                                        let combined = 0x10000 + ((u32::from(code) - 0xD800) << 10) + (u32::from(low) - 0xDC00);
                                        out.push(char::from_u32(combined).unwrap_or('\u{FFFD}'));
                                    } else {
                                        self.pos = saved;
                                        out.push('\u{FFFD}');
                                    }
                                } else {
                                    out.push('\u{FFFD}');
                                }
                            } else if (0xDC00..=0xDFFF).contains(&code) {
                                out.push('\u{FFFD}');
                            } else if let Some(ch) = char::from_u32(u32::from(code)) {
                                out.push(ch);
                            }
                        }
                        _ => return None,
                    }
                }
                _ => {
                    // Preserve UTF-8 sequences directly without skipping past invalid continuation bytes.
                    let start = self.pos - 1;
                    let width = if b < 0x80 {
                        1
                    } else if b & 0xE0 == 0xC0 {
                        2
                    } else if b & 0xF0 == 0xE0 {
                        3
                    } else if b & 0xF8 == 0xF0 {
                        4
                    } else {
                        1
                    };
                    let end = (start + width).min(self.bytes.len());
                    if let Ok(s) = std::str::from_utf8(&self.bytes[start..end]) {
                        out.push_str(s);
                    }
                    self.pos = end;
                }
            }
        }
        None
    }
    fn array(&mut self) -> Option<JsonVal> {
        self.depth += 1;
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            self.depth -= 1;
            return Some(JsonVal::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.peek()? {
                b',' => self.pos += 1,
                b']' => {
                    self.pos += 1;
                    self.depth -= 1;
                    return Some(JsonVal::Arr(items));
                }
                _ => return None,
            }
        }
    }
    fn object(&mut self) -> Option<JsonVal> {
        self.depth += 1;
        self.pos += 1; // '{'
        let mut pairs = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            self.depth -= 1;
            return Some(JsonVal::Obj(pairs));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return None;
            }
            self.pos += 1;
            let val = self.value()?;
            pairs.push((key, val));
            self.skip_ws();
            match self.peek()? {
                b',' => self.pos += 1,
                b'}' => {
                    self.pos += 1;
                    self.depth -= 1;
                    return Some(JsonVal::Obj(pairs));
                }
                _ => return None,
            }
        }
    }
}

/// Reads the line `PR_JQ` makes.
pub fn parse_pr_line(line: &str) -> Option<Pr> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let mut parts = line.split('\t');
    let number = parts.next()?.trim().parse().ok()?;
    let url = parts.next()?.trim().to_string();
    let state = parts.next().unwrap_or("").trim();
    let draft = parts.next().unwrap_or("").trim() == "true";
    let checks = parts.next().unwrap_or("").trim();
    let state = match state {
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ if draft => PrState::Draft,
        _ => PrState::Open,
    };
    Some(Pr { number, url, state, checks: classify_checks(checks) })
}

/// The checks as one state: any failure fails them, else any still running keeps them pending.
pub fn classify_checks(list: &str) -> Checks {
    let outcomes: Vec<String> = list.split(',').map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty()).collect();
    if outcomes.is_empty() {
        return Checks::None;
    }
    let failed = ["FAILURE", "ERROR", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE"];
    let pending = ["PENDING", "IN_PROGRESS", "QUEUED", "WAITING", "REQUESTED", "EXPECTED"];
    if outcomes.iter().any(|o| failed.contains(&o.as_str())) {
        Checks::Failed
    } else if outcomes.iter().any(|o| pending.contains(&o.as_str())) {
        Checks::Pending
    } else {
        Checks::Passed
    }
}

/// Where a remote's pages are, read from its address (`git@github.com:o/r.git`,
/// `https://github.com/o/r`, `ssh://git@host:22/o/r.git`). `None` off GitHub and GitLab.
pub fn parse_remote_url(url: &str) -> Option<Web> {
    let url = url.trim();
    let (scheme_low, rest) = if let Some((scheme, rest)) = url.split_once("://") {
        let low = scheme.to_ascii_lowercase();
        if !matches!(low.as_str(), "https" | "http" | "ssh" | "git" | "git+ssh" | "ssh+git") {
            return None;
        }
        (Some(low), rest.to_string())
    } else if let Some((host, path)) = url.split_once(':') {
        // scp-like: [user@]host:path (not a Windows drive letter).
        if host.len() <= 1 || host.contains('/') {
            return None;
        }
        (None, format!("{host}/{path}"))
    } else {
        return None;
    };
    let (authority, raw_path) = rest.split_once('/')?;
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host_only, port) = match host_port.split_once(':') {
        Some((h, p)) => (h.to_ascii_lowercase(), Some(p)),
        None => (host_port.to_ascii_lowercase(), None),
    };
    let forge = if host_only.contains("github") {
        Forge::GitHub
    } else if host_only.contains("gitlab") {
        Forge::GitLab
    } else {
        return None;
    };
    // Strip query/fragment and GitLab `/-/...` web suffixes before removing `.git`.
    let path_clean = raw_path.split(['?', '#']).next().unwrap_or(raw_path);
    let path_clean = if forge == Forge::GitLab {
        path_clean.split_once("/-/").map_or(path_clean, |(p, _)| p)
    } else {
        path_clean
    };
    let repo = path_clean.trim_matches('/');
    let repo = repo.strip_suffix(".git").unwrap_or(repo).trim_matches('/').to_string();
    if repo.split('/').filter(|p| !p.is_empty()).count() < 2 {
        return None;
    }
    // Normalize SSH config aliases (`github-work`, `github.com-work`), `www.*`, and official alt SSH hosts.
    let mut host = match forge {
        Forge::GitHub
            if !host_only.contains('.')
                || host_only == "ssh.github.com"
                || host_only == "www.github.com"
                || host_only.starts_with("github.com-")
                || host_only.starts_with("github.com_") =>
        {
            "github.com".to_string()
        }
        Forge::GitLab
            if !host_only.contains('.')
                || host_only == "altssh.gitlab.com"
                || host_only == "www.gitlab.com"
                || host_only.starts_with("gitlab.com-")
                || host_only.starts_with("gitlab.com_") =>
        {
            "gitlab.com".to_string()
        }
        _ => host_only,
    };
    // Preserve explicit non-default HTTP/HTTPS ports on self-hosted instances.
    if let (Some(sch), Some(p)) = (scheme_low.as_deref(), port)
        && matches!(sch, "https" | "http")
        && !p.is_empty()
        && p.bytes().all(|b| b.is_ascii_digit())
        && !(sch == "https" && p == "443")
        && !(sch == "http" && p == "80")
    {
        host.push(':');
        host.push_str(p);
    }
    Some(Web { forge, host, repo })
}

/// A branch name made from a commit title: `Add a cart badge` → `add-a-cart-badge`.
pub fn branch_from_title(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let mut out = out.trim_matches('-').to_string();
    if out.len() > 40 {
        let cut = out[..40].rfind('-').filter(|&i| i >= 16).unwrap_or(40);
        out.truncate(cut);
        out = out.trim_matches('-').to_string();
    }
    if out.is_empty() { "changes".to_string() } else { out }
}

/// A message's subject (its first line) and body (the rest, trimmed).
pub fn split_message(message: &str) -> (String, String) {
    let message = message.trim();
    match message.split_once('\n') {
        Some((title, body)) => (title.trim().to_string(), body.trim().to_string()),
        None => (message.to_string(), String::new()),
    }
}

/// The diff of `paths` between two trees (or a tree and a commit), with its stat, as text for a
/// message to be written from: at most `max` bytes, cut at a line.
pub fn diff_text(root: &Path, from: &str, to: &str, paths: &[String], max: usize) -> Result<String, String> {
    super::check_rev(from)?;
    super::check_rev(to)?;
    let specs: Vec<String> = paths.iter().map(|p| literal_pathspec(p)).collect();
    let out = run(git(root).args(["diff", "--no-color", "--no-ext-diff", "--no-textconv", "--no-relative", "--stat", "--patch", from, to, "--"]).args(&specs))?;
    let text = String::from_utf8_lossy(&out).into_owned();
    if text.len() <= max {
        return Ok(text);
    }
    let mut cut = max;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let end = text[..cut].rfind('\n').unwrap_or(cut);
    Ok(format!("{}\n[… the rest of the diff is left out]\n", &text[..end]))
}

/// Runs `cmd` (stdin: `input`, or nothing), at most `timeout` long. Ok: its stdout. Err: what
/// it printed (stderr after stdout: hooks write to either), or why it could not run.
pub(crate) fn output_of(cmd: &mut Command, input: Option<&[u8]>, timeout: Option<Duration>) -> Result<String, String> {
    cmd.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd.spawn().map_err(|e| format!("could not run {program}: {e}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        let input = input.to_vec();
        // Written on its own thread: a program that prints much before it reads cannot block us.
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let (mut out, mut err) = (child.stdout.take(), child.stderr.take());
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out_t = reader(out.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err_t = reader(err.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        if timeout.is_some_and(|t| started.elapsed() > t) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{program} did not answer in {} s and was stopped.", timeout.map_or(0, |t| t.as_secs())));
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let stdout = String::from_utf8_lossy(&out_t.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_t.join().unwrap_or_default()).into_owned();
    match status {
        Some(s) if s.success() => Ok(stdout),
        _ => {
            let both = format!("{}\n{}", stdout.trim(), stderr.trim());
            let both = both.trim();
            Err(if both.is_empty() { format!("{program} failed") } else { both.to_string() })
        }
    }
}

/// `text` cut to at most `max` characters, with an ellipsis.
fn cut_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Percent-encodes `s` for an address; `path`: `/` stays (a branch name in a path).
fn encode(s: &str, path: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') || (path && b == b'/');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The PATH to run programs with: `path`, else this process's.
pub fn path_or_env(path: Option<&OsString>) -> Option<OsString> {
    path.cloned().or_else(|| std::env::var_os("PATH"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_addresses() {
        let gh = |repo: &str| Some(Web { forge: Forge::GitHub, host: "github.com".into(), repo: repo.into() });
        assert_eq!(parse_remote_url("git@github.com:acme/storefront.git"), gh("acme/storefront"));
        assert_eq!(parse_remote_url("https://github.com/acme/storefront"), gh("acme/storefront"));
        assert_eq!(parse_remote_url("https://github.com/acme/storefront.git/"), gh("acme/storefront"));
        assert_eq!(parse_remote_url("https://user:token@github.com/acme/storefront.git"), gh("acme/storefront"));
        assert_eq!(parse_remote_url("ssh://git@github.com:22/acme/storefront.git"), gh("acme/storefront"));
        assert_eq!(parse_remote_url("git@github-work:acme/storefront.git"), gh("acme/storefront"));
        assert_eq!(parse_remote_url("https://github.com/acme/repo@v2.git"), gh("acme/repo@v2"));
        assert_eq!(parse_remote_url("https://github.com/acme/my.git.git"), gh("acme/my.git"));
        assert!(push(Path::new("."), "--delete", "main", false, None).is_err());
        assert!(push(Path::new("."), "origin", "--all", false, None).is_err());
        assert_eq!(
            parse_remote_url("git@gitlab.com:group/sub/app.git"),
            Some(Web { forge: Forge::GitLab, host: "gitlab.com".into(), repo: "group/sub/app".into() })
        );
        assert_eq!(
            parse_remote_url("https://github.acme.io/team/app"),
            Some(Web { forge: Forge::GitHub, host: "github.acme.io".into(), repo: "team/app".into() })
        );
        for none in ["/srv/git/app.git", "C:\\repos\\app", "https://example.com/a/b", "git@github.com:onlyone", "file:///tmp/x.git", ""] {
            assert_eq!(parse_remote_url(none), None, "{none}");
        }
    }

    #[test]
    fn pull_request_forms() {
        let web = parse_remote_url("git@github.com:acme/storefront.git").unwrap();
        assert_eq!(web.gh_repo(), "acme/storefront");
        assert_eq!(
            web.new_pr_url("main", "feature/cart badge", "Add a badge & count", "Line one\nTwo"),
            "https://github.com/acme/storefront/compare/main...feature/cart%20badge?expand=1&title=Add%20a%20badge%20%26%20count&body=Line%20one%0ATwo"
        );
        let lab = parse_remote_url("https://gitlab.com/g/app.git").unwrap();
        assert!(lab.new_pr_url("main", "fix", "T", "").starts_with("https://gitlab.com/g/app/-/merge_requests/new?merge_request%5Bsource_branch%5D=fix&merge_request%5Btarget_branch%5D=main&"));
        let ghe = parse_remote_url("https://github.acme.io/team/app").unwrap();
        assert_eq!(ghe.gh_repo(), "github.acme.io/team/app");
        assert_eq!(web.commit_url("abc1234"), "https://github.com/acme/storefront/commit/abc1234");
        assert_eq!(web.prs_url(), "https://github.com/acme/storefront/pulls");
        assert_eq!(lab.commit_url("abc1234"), "https://gitlab.com/g/app/-/commit/abc1234");
        assert_eq!(lab.prs_url(), "https://gitlab.com/g/app/-/merge_requests");
        assert_eq!(lab.glab_repo(), "g/app");
    }

    #[test]
    fn gitlab_mr_json_and_list_parsing() {
        let single = r#"{"iid": 17, "title": "Add GitLab support", "state": "opened", "draft": false, "web_url": "https://gitlab.com/acme/app/-/merge_requests/17", "source_branch": "feat/gitlab", "author": {"username": "ilham"}, "head_pipeline": {"status": "running"}}"#;
        let mr = parse_glab_mr_json(single).unwrap();
        assert_eq!(
            mr,
            Pr {
                number: 17,
                url: "https://gitlab.com/acme/app/-/merge_requests/17".into(),
                state: PrState::Open,
                checks: Checks::Pending,
            }
        );
        let list = format!("[{single}]");
        let parsed = parse_glab_mr_list_json(&list);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].number, 17);
        assert_eq!(parsed[0].branch, "feat/gitlab");
        assert_eq!(parsed[0].author, "ilham");
        assert_eq!(parsed[0].title, "Add GitLab support");

        let from_glab = pr_from_url("Creating merge request...\nhttps://gitlab.com/acme/app/-/merge_requests/42\n", PrState::Open).unwrap();
        assert_eq!(from_glab.number, 42);
    }

    #[test]
    fn push_errors() {
        assert!(matches!(classify_push(" ! [rejected]        main -> main (fetch first)".into()), PushError::Rejected(_)));
        assert!(matches!(classify_push("hint: Updates were rejected because the tip of your current branch is behind".into()), PushError::Rejected(_)));
        assert!(matches!(classify_push("fatal: could not read Username for 'https://github.com': terminal prompts disabled".into()), PushError::Credentials(_)));
        assert!(matches!(classify_push("git@github.com: Permission denied (publickey).".into()), PushError::Credentials(_)));
        assert!(matches!(classify_push("remote: error: GH006: Protected branch update failed".into()), PushError::Failed(_)));
    }

    #[test]
    fn pull_request_lines() {
        let pr = parse_pr_line("42\thttps://github.com/acme/app/pull/42\tOPEN\tfalse\tSUCCESS,SKIPPED\n").unwrap();
        assert_eq!(pr, Pr { number: 42, url: "https://github.com/acme/app/pull/42".into(), state: PrState::Open, checks: Checks::Passed });
        assert_eq!(parse_pr_line("7\tu\tOPEN\ttrue\t").unwrap().state, PrState::Draft);
        assert_eq!(parse_pr_line("7\tu\tMERGED\tfalse\tSUCCESS").unwrap().state, PrState::Merged);
        assert_eq!(parse_pr_line(""), None);
        assert_eq!(classify_checks(""), Checks::None);
        assert_eq!(classify_checks("SUCCESS,IN_PROGRESS"), Checks::Pending);
        assert_eq!(classify_checks("SUCCESS,PENDING,FAILURE"), Checks::Failed);
        assert_eq!(classify_checks("NEUTRAL,SUCCESS"), Checks::Passed);
        let made = pr_from_url("Creating pull request for x into main\n\nhttps://github.com/acme/app/pull/128\n", PrState::Draft).unwrap();
        assert_eq!((made.number, made.state), (128, PrState::Draft));
        let exists = pr_from_url("a pull request for branch \"x\" into branch \"main\" already exists:\nhttps://github.com/acme/app/pull/9", PrState::Open).unwrap();
        assert_eq!(exists.number, 9);
    }

    #[test]
    fn branch_names_from_titles() {
        assert_eq!(branch_from_title("Add a cart badge to the header"), "add-a-cart-badge-to-the-header");
        assert_eq!(branch_from_title("Fix: rounding (checkout) — once!"), "fix-rounding-checkout-once");
        assert_eq!(branch_from_title("Ünïcode only ✓"), "n-code-only");
        assert_eq!(branch_from_title("…"), "changes");
        let long = branch_from_title("Make the parser handle nested generics and closing angle brackets everywhere");
        assert!(long.len() <= 40 && !long.ends_with('-'), "{long}");
        assert!(crate::worktree::valid_branch(&long));
    }

    #[test]
    fn messages() {
        assert_eq!(split_message("Title\n\nBody one.\nBody two.\n"), ("Title".into(), "Body one.\nBody two.".into()));
        assert_eq!(split_message("  Only a title  "), ("Only a title".into(), String::new()));
    }
}
