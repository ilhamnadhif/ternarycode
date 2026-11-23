//! Commit history browsing and remote sync (`git log`, per-commit file lists & diffs, and
//! non-interactive `git fetch`). Every function blocks until `git` exits: call off the UI thread.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;

use super::{FileChange, FileDiff, check_rev, empty_tree, git, rev_parse, run};
use crate::ship::{NETWORK_TIMEOUT, PushError, classify_push, no_prompts, output_of};

/// One commit in a repository's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitEntry {
    pub hash: String,
    pub short: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    /// Author timestamp (Unix seconds).
    pub time: i64,
    pub subject: String,
    pub body: String,
    /// Ref labels pointing at this commit (`HEAD -> main`, `origin/main`, `tag: v0.2.3`).
    pub refs: Vec<String>,
    /// True when this commit is ahead of the configured remote (not yet pushed).
    pub unpushed: bool,
}

/// One page of commits from [`log`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogPage {
    pub commits: Vec<CommitEntry>,
    pub has_more: bool,
    pub branch: Option<String>,
}

/// Reads up to `limit` commits starting at `skip` (newest first).
/// When `query` is non-empty, resolves an exact commit hash prefix if it matches one, otherwise
/// searches commit messages case-insensitively (`--fixed-strings --regexp-ignore-case --grep`).
pub fn log(
    root: &Path,
    limit: usize,
    skip: usize,
    query: Option<&str>,
    remote: Option<&str>,
) -> Result<LogPage, String> {
    let branch = super::worktree::current_branch(root);
    if rev_parse(root, "HEAD").is_none() {
        return Ok(LogPage { commits: Vec::new(), has_more: false, branch });
    }
    let unpushed = unpushed_hashes(root, remote);
    let q = query.map(str::trim).filter(|s| !s.is_empty());
    // Exact commit hash lookup when query looks like a hex SHA prefix (4..=40 hex chars).
    if skip == 0
        && let Some(hex) = q.filter(|s| (4..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()))
        && let Some(full) = rev_parse(root, &format!("{hex}^{{commit}}"))
    {
        let mut cmd = git(root);
        cmd.args([
            "log",
            "-n",
            "1",
            "--format=%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1f%b%x1e",
            &full,
            "--",
        ]);
        let out = run(&mut cmd)?;
        let commits = parse_log(&String::from_utf8_lossy(&out), &unpushed);
        return Ok(LogPage { commits, has_more: false, branch });
    }
    let fetch_n = limit.saturating_add(1).max(2);
    let mut cmd = git(root);
    cmd.args([
        "log",
        &format!("--max-count={fetch_n}"),
        &format!("--skip={skip}"),
        "--format=%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1f%b%x1e",
    ]);
    if let Some(q) = q {
        cmd.args(["--fixed-strings", "--regexp-ignore-case", &format!("--grep={q}")]);
    }
    cmd.arg("--");
    let out = run(&mut cmd)?;
    let mut commits = parse_log(&String::from_utf8_lossy(&out), &unpushed);
    let has_more = commits.len() > limit;
    commits.truncate(limit);
    Ok(LogPage { commits, has_more, branch })
}

/// Full hashes of commits reachable from `HEAD` that `remote` does not have on any branch.
fn unpushed_hashes(root: &Path, remote: Option<&str>) -> HashSet<String> {
    let Some(remote) = remote.filter(|r| !r.is_empty() && !r.starts_with('-')) else {
        return HashSet::new();
    };
    let Ok(out) = run(git(root).args([
        "rev-list",
        "--max-count=500",
        "HEAD",
        "--not",
        &format!("--remotes={remote}"),
    ])) else {
        return HashSet::new();
    };
    String::from_utf8_lossy(&out)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// Parses records formatted with `%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s%x1f%b%x1e`.
pub fn parse_log(raw: &str, unpushed: &HashSet<String>) -> Vec<CommitEntry> {
    let mut out = Vec::new();
    for rec in raw.split('\x1e') {
        let rec = rec.trim_start_matches(['\r', '\n']);
        if rec.is_empty() {
            continue;
        }
        let mut fields = rec.splitn(9, '\x1f');
        let Some(hash) = fields.next().map(str::trim).filter(|s| !s.is_empty()) else { continue };
        let short = fields.next().unwrap_or("").trim().to_string();
        let parents: Vec<String> = fields
            .next()
            .unwrap_or("")
            .split_whitespace()
            .map(String::from)
            .collect();
        let author = fields.next().unwrap_or("").trim().to_string();
        let email = fields.next().unwrap_or("").trim().to_string();
        let time = fields.next().unwrap_or("0").trim().parse::<i64>().unwrap_or(0);
        let refs: Vec<String> = fields
            .next()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(String::from)
            .collect();
        let subject = fields.next().unwrap_or("").trim().to_string();
        let body = fields.next().unwrap_or("").trim().to_string();
        let is_unpushed = unpushed.contains(hash);
        out.push(CommitEntry {
            hash: hash.to_string(),
            short,
            parents,
            author,
            email,
            time,
            subject,
            body,
            refs,
            unpushed: is_unpushed,
        });
    }
    out
}

/// Resolves `(from_tree, to_tree)` for inspecting a single commit `hash`.
/// Uses the first parent's tree (`hash^1^{tree}`), falling back to the empty tree for root commits.
pub fn commit_trees(root: &Path, hash: &str) -> Result<(String, String), String> {
    check_rev(hash)?;
    let to = rev_parse(root, &format!("{hash}^{{tree}}"))
        .ok_or_else(|| format!("could not read tree for commit {hash}"))?;
    let from = match rev_parse(root, &format!("{hash}^1^{{tree}}")) {
        Some(t) => t,
        None => empty_tree(root)?,
    };
    Ok((from, to))
}

/// Lists all files changed by commit `hash`, returning `(from_tree, to_tree, files)`.
pub fn commit_files(root: &Path, hash: &str) -> Result<(String, String, Vec<FileChange>), String> {
    let (from, to) = commit_trees(root, hash)?;
    let files = super::changes(root, &from, &to)?;
    Ok((from, to, files))
}

/// Unified diff of one file inside a commit (using the `(from, to)` trees from [`commit_files`]).
pub fn commit_file_diff(
    root: &Path,
    from: &str,
    to: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<FileDiff, String> {
    super::file_diff(root, from, to, path, old_path)
}

/// Fetches updates from `remote` (with `--prune`) without prompting for passwords or SSH passphrases.
pub fn fetch(root: &Path, remote: &str, path: Option<&OsStr>) -> Result<(), PushError> {
    if remote.is_empty() || remote.starts_with('-') {
        return Err(PushError::Failed("Invalid remote name.".into()));
    }
    let mut cmd = git(root);
    no_prompts(&mut cmd, root);
    cmd.args(["fetch", "--prune", "--", remote]);
    if let Some(path) = path {
        cmd.env("PATH", path);
    }
    match output_of(&mut cmd, None, Some(NETWORK_TIMEOUT)) {
        Ok(_) => Ok(()),
        Err(out) => Err(classify_push(out)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_formatted_log_records_and_refs() {
        let raw = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111\x1faaaa111\x1fbbbb222\x1fJane Dev\x1fjane@example.com\x1f1700000100\x1fHEAD -> main, origin/main, tag: v1.0.0\x1fAdd history view\x1fLine one\nLine two\x1e\
                   bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222\x1fbbbb222\x1f\x1fJane Dev\x1fjane@example.com\x1f1700000000\x1f\x1fInitial commit\x1f\x1e";
        let mut unpushed = HashSet::new();
        unpushed.insert("aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111".to_string());
        let rows = parse_log(raw, &unpushed);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].short, "aaaa111");
        assert_eq!(rows[0].parents, vec!["bbbb222"]);
        assert_eq!(rows[0].refs, vec!["HEAD -> main", "origin/main", "tag: v1.0.0"]);
        assert_eq!(rows[0].subject, "Add history view");
        assert_eq!(rows[0].body, "Line one\nLine two");
        assert!(rows[0].unpushed);
        assert!(rows[1].parents.is_empty());
        assert!(!rows[1].unpushed);
    }

    #[test]
    fn reads_log_and_root_commit_files_from_real_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run(git(root).args(["init", "-b", "main"])).unwrap();
        run(git(root).args(["config", "user.name", "Test User"])).unwrap();
        run(git(root).args(["config", "user.email", "test@example.com"])).unwrap();
        // Empty repo returns empty page cleanly.
        let empty = log(root, 20, 0, None, None).unwrap();
        assert!(empty.commits.is_empty());
        assert!(!empty.has_more);

        std::fs::write(root.join("hello.txt"), "hello\n").unwrap();
        run(git(root).args(["add", "hello.txt"])).unwrap();
        run(git(root).args(["commit", "-m", "Initial commit"])).unwrap();

        std::fs::write(root.join("hello.txt"), "hello\nworld\n").unwrap();
        run(git(root).args(["add", "hello.txt"])).unwrap();
        run(git(root).args(["commit", "-m", "Greet world\n\nSecond paragraph."])).unwrap();

        let page = log(root, 10, 0, None, None).unwrap();
        assert_eq!(page.commits.len(), 2);
        assert!(!page.has_more);
        assert_eq!(page.commits[0].subject, "Greet world");
        assert_eq!(page.commits[0].body, "Second paragraph.");
        assert_eq!(page.commits[1].subject, "Initial commit");

        // Root commit files & diff work cleanly against empty tree.
        let (from, to, files) = commit_files(root, &page.commits[1].hash).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "hello.txt");
        let diff = commit_file_diff(root, &from, &to, "hello.txt", None).unwrap();
        assert_eq!(diff.hunks.len(), 1);

        // Search by message and by short hash prefix.
        let filtered = log(root, 10, 0, Some("greet"), None).unwrap();
        assert_eq!(filtered.commits.len(), 1);
        assert_eq!(filtered.commits[0].subject, "Greet world");
        let by_sha = log(root, 10, 0, Some(&page.commits[1].short), None).unwrap();
        assert_eq!(by_sha.commits.len(), 1);
        assert_eq!(by_sha.commits[0].subject, "Initial commit");
    }
}
