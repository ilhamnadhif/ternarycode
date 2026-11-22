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

/// How long a push or a `gh` call may take before it is given up (a server that never answers).
const NETWORK_TIMEOUT: Duration = Duration::from_secs(180);

/// The site a remote lives on, for its pull request form and `gh`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forge {
    GitHub,
    GitLab,
}

/// Where a repository's pages are: `github.com` and `owner/repo`.
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
    commit.args(["commit", "--cleanup=strip", "-F", "-", "--"]).args(&specs);
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
fn no_prompts(cmd: &mut Command, root: &Path) {
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

/// The pull request named by the last `…/pull/N` address in `text`.
fn pr_from_url(text: &str, state: PrState) -> Option<Pr> {
    let url = text.split_whitespace().rev().find(|w| w.starts_with("https://") && (w.contains("/pull/") || w.contains("/merge_requests/")))?;
    let url = url.trim_end_matches(['.', ',', ')']).to_string();
    let number = url.rsplit('/').next()?.parse().ok()?;
    Some(Pr { number, url, state, checks: Checks::None })
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
    let rest = if let Some((scheme, rest)) = url.split_once("://") {
        if !matches!(scheme, "https" | "http" | "ssh" | "git" | "git+ssh" | "ssh+git") {
            return None;
        }
        rest.to_string()
    } else if let Some((host, path)) = url.split_once(':') {
        // scp-like: [user@]host:path (not a Windows drive letter).
        if host.len() <= 1 || host.contains('/') {
            return None;
        }
        format!("{host}/{path}")
    } else {
        return None;
    };
    let (authority, path) = rest.split_once('/')?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split(':').next()?.to_lowercase();
    let repo = path.trim_matches('/');
    let repo = repo.strip_suffix(".git").unwrap_or(repo).trim_matches('/').to_string();
    if repo.split('/').filter(|p| !p.is_empty()).count() < 2 {
        return None;
    }
    let forge = if host.contains("github") {
        Forge::GitHub
    } else if host.contains("gitlab") {
        Forge::GitLab
    } else {
        return None;
    };
    // An ssh host alias (`github-work`) is not where the pages are.
    let host = if host.contains('.') {
        host
    } else {
        match forge {
            Forge::GitHub => "github.com".to_string(),
            Forge::GitLab => "gitlab.com".to_string(),
        }
    };
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
fn output_of(cmd: &mut Command, input: Option<&[u8]>, timeout: Option<Duration>) -> Result<String, String> {
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
