//! Worktrees for agent runs: one repository, a folder and a branch per agent, so agents working
//! in the same repository never write over each other's files. Everything goes through the git
//! command line, like the rest of this crate; nothing here touches the user's index or the
//! main folder's checkout.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::{check_rev, git, parse_oid, rev_parse, run, trim_line};

/// A commit on a branch: its short id and its subject line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub short: String,
    pub subject: String,
    /// Who made it (its author's name) and when (Unix seconds).
    pub author: String,
    pub time: i64,
}

/// Creates `dir` as a new worktree of the repository at `repo`, on a new branch `branch` that
/// starts at `base`. Returns the commit it starts from.
pub fn add(repo: &Path, dir: &Path, branch: &str, base: &str) -> Result<String, String> {
    check_rev(base)?;
    check_branch(branch)?;
    run(git(repo).args(["worktree", "add", "-b", branch]).arg(dir).arg(base)).map_err(reason)?;
    let out = run(git(dir).args(["rev-parse", "HEAD"])).map_err(reason)?;
    parse_oid(&out)
}

/// Removes the worktree at `dir` (its folder too). `force` removes it with changes not
/// committed in it (they are lost). A folder already gone is only forgotten by git.
pub fn remove(repo: &Path, dir: &Path, force: bool) -> Result<(), String> {
    if !dir.exists() {
        return run(git(repo).args(["worktree", "prune"])).map(|_| ()).map_err(reason);
    }
    let mut cmd = git(repo);
    cmd.args(["worktree", "remove"]);
    if force {
        cmd.arg("--force");
    }
    run(cmd.arg(dir)).map(|_| ()).map_err(reason)
}

/// Deletes a branch: only when it is merged into its upstream or HEAD, unless `force`.
pub fn delete_branch(repo: &Path, branch: &str, force: bool) -> Result<(), String> {
    check_branch(branch)?;
    run(git(repo).args(["branch", if force { "-D" } else { "-d" }, branch])).map(|_| ()).map_err(reason)
}

/// The local branches, by name.
pub fn branches(repo: &Path) -> Vec<String> {
    let Ok(out) = run(git(repo).args(["for-each-ref", "--format=%(refname:short)", "refs/heads/"])) else {
        return Vec::new();
    };
    let mut names: Vec<String> = String::from_utf8_lossy(&out).lines().map(str::to_string).filter(|l| !l.is_empty()).collect();
    names.sort();
    names
}

/// The commit checked out in `dir` (`None` without commits).
pub fn head(dir: &Path) -> Option<String> {
    rev_parse(dir, "HEAD^{commit}")
}

/// The branch checked out in `dir` (`None` when detached or without commits).
pub fn current_branch(dir: &Path) -> Option<String> {
    let out = run(git(dir).args(["symbolic-ref", "--short", "-q", "HEAD"])).ok()?;
    let name = String::from_utf8_lossy(trim_line(&out)).into_owned();
    (!name.is_empty()).then_some(name)
}

/// The short id and the commit time (Unix seconds) of `rev`.
pub fn commit_line(repo: &Path, rev: &str) -> Option<(String, i64)> {
    check_rev(rev).ok()?;
    let out = run(git(repo).args(["log", "-1", "--format=%h%x00%ct", rev, "--"])).ok()?;
    let line = String::from_utf8_lossy(trim_line(&out)).into_owned();
    let (short, time) = line.split_once('\0')?;
    Some((short.to_string(), time.trim().parse().ok()?))
}

/// How many commits `head` has that `base` has not (ahead), and the other way (behind).
pub fn ahead_behind(repo: &Path, base: &str, head: &str) -> Result<(usize, usize), String> {
    check_rev(base)?;
    check_rev(head)?;
    let out = run(git(repo).args(["rev-list", "--left-right", "--count", &format!("{base}...{head}"), "--"]))?;
    let text = String::from_utf8_lossy(trim_line(&out)).into_owned();
    let mut it = text.split_whitespace().map(|n| n.parse::<usize>().unwrap_or(0));
    let behind = it.next().unwrap_or(0);
    let ahead = it.next().unwrap_or(0);
    Ok((ahead, behind))
}

/// How many files the commits of `head` changed since it left `base`.
pub fn files_changed(repo: &Path, base: &str, head: &str) -> Result<usize, String> {
    check_rev(base)?;
    check_rev(head)?;
    let out = run(git(repo).args(["diff", "--no-ext-diff", "--name-only", "-z", &format!("{base}...{head}"), "--"]))?;
    Ok(out.split(|b| *b == 0).filter(|r| !r.is_empty()).count())
}

/// What the commits of `head` changed since it left `base`: files, lines added, lines removed.
pub fn diff_stat(repo: &Path, base: &str, head: &str) -> Result<(usize, u32, u32), String> {
    check_rev(base)?;
    check_rev(head)?;
    let out = run(git(repo).args(["diff", "--no-ext-diff", "--numstat", "-z", &format!("{base}...{head}"), "--"]))?;
    let text = String::from_utf8_lossy(&out);
    let (mut files, mut adds, mut dels) = (0, 0, 0);
    // "<added>\t<removed>\t<path>\0" (a rename: "<added>\t<removed>\t\0<old>\0<new>\0").
    let mut fields = text.split('\0');
    while let Some(rec) = fields.next() {
        let mut it = rec.splitn(3, '\t');
        let (Some(a), Some(d), Some(path)) = (it.next(), it.next(), it.next()) else { continue };
        if path.is_empty() {
            fields.next();
            fields.next();
        }
        files += 1;
        adds += a.parse::<u32>().unwrap_or(0);
        dels += d.parse::<u32>().unwrap_or(0);
    }
    Ok((files, adds, dels))
}

/// Whether commit `a` is in the history of `b` (merged into it).
pub fn is_ancestor(repo: &Path, a: &str, b: &str) -> bool {
    check_rev(a).is_ok() && check_rev(b).is_ok() && run(git(repo).args(["merge-base", "--is-ancestor", a, b])).is_ok()
}

/// The commit `a` and `b` both come from.
pub fn merge_base(repo: &Path, a: &str, b: &str) -> Result<String, String> {
    check_rev(a)?;
    check_rev(b)?;
    let out = run(git(repo).args(["merge-base", a, b]))?;
    parse_oid(&out)
}

/// The tree of a commit.
pub fn tree_of(repo: &Path, commit: &str) -> Result<String, String> {
    check_rev(commit)?;
    rev_parse(repo, &format!("{commit}^{{tree}}")).ok_or_else(|| format!("could not read the tree of {commit}"))
}

/// The commits in `to` that are not in `from`, newest first (at most 100).
pub fn commits(repo: &Path, from: &str, to: &str) -> Result<Vec<Commit>, String> {
    check_rev(from)?;
    check_rev(to)?;
    let out = run(git(repo).args(["log", "--max-count=100", "--format=%h%x00%an%x00%ct%x00%s", &format!("{from}..{to}"), "--"]))?;
    Ok(String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(4, '\0');
            let short = it.next()?.to_string();
            let author = it.next()?.to_string();
            let time = it.next()?.trim().parse().unwrap_or(0);
            let subject = it.next()?.to_string();
            Some(Commit { short, subject, author, time })
        })
        .collect())
}

/// For each file changed by the commits in `to` that are not in `from`: the newest of them
/// that changed it (its short id), by path from the repository root.
pub fn last_commit_by_file(repo: &Path, from: &str, to: &str) -> Result<HashMap<String, String>, String> {
    check_rev(from)?;
    check_rev(to)?;
    let out = run(git(repo).args(["log", "--max-count=500", "--format=%x01%h", "--name-only", "--no-renames", &format!("{from}..{to}"), "--"]))?;
    let mut by_file = HashMap::new();
    let mut current = String::new();
    for line in String::from_utf8_lossy(&out).lines() {
        if let Some(short) = line.strip_prefix('\u{1}') {
            current = short.to_string();
        } else if !line.is_empty() && !current.is_empty() {
            by_file.entry(line.to_string()).or_insert_with(|| current.clone());
        }
    }
    Ok(by_file)
}

/// The files not committed in `dir` (changed, added, deleted, new; each file of a new folder),
/// by path from its top, sorted.
pub fn dirty_files(dir: &Path) -> Vec<String> {
    let Ok(out) = run(git(dir).args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    let mut records = out.split(|b| *b == 0);
    while let Some(rec) = records.next() {
        if rec.len() < 4 {
            continue;
        }
        if matches!(rec[0], b'R' | b'C') || matches!(rec[1], b'R' | b'C') {
            // The old name of a rename follows as a record of its own (also for one in the
            // working tree, ` R new\0old`).
            records.next();
        }
        files.push(String::from_utf8_lossy(&rec[3..]).into_owned());
    }
    files.sort();
    files
}

/// What git ignores at the top of the repository (files and folders, by name; folders end in
/// `/`), at most 40: what a new worktree does not get unless it is copied.
pub fn ignored_top(repo: &Path) -> Vec<String> {
    let Ok(out) = run(git(repo).args(["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z"])) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for rec in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let path = String::from_utf8_lossy(rec);
        let top = match path.split_once('/') {
            Some((first, rest)) if !rest.is_empty() => format!("{first}/"),
            Some((first, _)) => format!("{first}/"),
            None => path.into_owned(),
        };
        if seen.insert(top.clone()) {
            names.push(top);
        }
        if names.len() >= 40 {
            break;
        }
    }
    names.sort();
    names
}

/// Rejects branch names git would read as an option or would not take.
fn check_branch(name: &str) -> Result<(), String> {
    if valid_branch(name) { Ok(()) } else { Err(format!("invalid branch name {name:?}")) }
}

/// A branch name git accepts (`git check-ref-format --branch`), kept simple: letters, digits,
/// `-`, `_`, `.` and `/` between parts, no `..`, no part starting with `.` or `-`, no `.lock`.
pub fn valid_branch(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.contains("..")
        && !name.ends_with('/')
        && !name.ends_with(".lock")
        && name.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && !part.starts_with('-')
                && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
}

/// Git's own words for why a command failed (without the command line and `fatal:`), for the
/// message the user reads.
fn reason(error: String) -> String {
    // `run` says "`git …` failed in <dir>: <stderr>".
    let detail = match error.find(" failed in ") {
        Some(i) => error[i..].split_once(": ").map_or(error.as_str(), |(_, d)| d),
        None => error.as_str(),
    };
    let first = detail.lines().find(|l| l.starts_with("fatal:") || l.starts_with("error:")).unwrap_or(detail);
    first.trim_start_matches("fatal:").trim_start_matches("error:").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names() {
        for ok in ["tc/generics", "claude-2", "fix_lexer", "a/b/c", "v1.2"] {
            assert!(valid_branch(ok), "{ok}");
        }
        for bad in ["", "-x", "tc/", "a..b", ".hidden", "a/.b", "x.lock", "has space", "tc//x", "ünï"] {
            assert!(!valid_branch(bad), "{bad}");
        }
    }

    #[test]
    fn reasons_are_gits_own_words() {
        let e = "`git worktree add -b tc/x /tmp/x main` failed in /r: fatal: a branch named 'tc/x' already exists".to_string();
        assert_eq!(reason(e), "a branch named 'tc/x' already exists");
        assert_eq!(reason("could not run git: not found".into()), "could not run git: not found");
    }
}
