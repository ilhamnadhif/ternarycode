//! Commit, push and branch facts against real repositories and a bare remote in a temporary
//! directory (no network).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tc_git::ship::{self, Busy, PushError};

/// `canonicalize` without the `\\?\` prefix it adds on Windows (git needs the plain form).
fn canonical(p: &Path) -> PathBuf {
    let c = p.canonicalize().unwrap();
    #[cfg(windows)]
    if let Some(rest) = c.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        return PathBuf::from(rest);
    }
    c
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn configure(root: &Path) {
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
        ("core.autocrlf", "false"),
    ] {
        git(root, &["config", key, value]);
    }
}

/// A bare remote and a clone of it with one commit on `main`, pushed.
struct Setup {
    _dir: tempfile::TempDir,
    base: PathBuf,
    remote: PathBuf,
    root: PathBuf,
}

impl Setup {
    fn new() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let base = canonical(dir.path());
        let remote = base.join("remote.git");
        git(&base, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        let root = base.join("work");
        git(&base, &["clone", "-q", remote.to_str().unwrap(), root.to_str().unwrap()]);
        configure(&root);
        git(&root, &["switch", "-q", "-c", "main"]);
        fs::write(root.join("README.md"), "hello\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "First"]);
        git(&root, &["push", "-q", "-u", "origin", "main"]);
        git(&root, &["remote", "set-head", "origin", "main"]);
        Setup { _dir: dir, base, remote, root }
    }

    fn write(&self, path: &str, text: &str) {
        let p = self.root.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
}

#[test]
fn facts_of_a_pushed_branch() {
    let s = Setup::new();
    let f = ship::facts(&s.root);
    assert_eq!(f.branch.as_deref(), Some("main"));
    assert!(f.has_commits);
    let remote = f.remote.unwrap();
    assert_eq!(remote.name, "origin");
    assert_eq!(remote.web, None, "a folder remote has no pages");
    assert_eq!(f.upstream.as_deref(), Some("origin/main"));
    assert_eq!((f.unpushed, f.behind), (0, 0));
    assert_eq!(f.default_branch.as_deref(), Some("main"));
    assert_eq!(f.busy, None);
    assert!(f.conflicts.is_empty());
}

#[test]
fn commits_only_the_paths_given() {
    let s = Setup::new();
    s.write("a.txt", "a\n");
    s.write("dir/b.txt", "b\n");
    s.write("README.md", "hello again\n");
    // Something staged that is not part of the commit stays staged.
    s.write("staged.txt", "s\n");
    git(&s.root, &["add", "staged.txt"]);
    fs::remove_file(s.root.join("README.md")).unwrap();
    let short = ship::commit(&s.root, &["a.txt".into(), "README.md".into()], "Add a\n\nAnd remove the readme.").unwrap();
    assert!(!short.is_empty());
    assert_eq!(git(&s.root, &["log", "-1", "--format=%s%n%b"]), "Add a\nAnd remove the readme.");
    let files = git(&s.root, &["show", "--name-status", "--format=", "HEAD"]);
    assert_eq!(files, "D\tREADME.md\nA\ta.txt");
    let status = git(&s.root, &["status", "--porcelain"]);
    assert!(status.contains("A  staged.txt"), "{status}");
    assert!(status.contains("?? dir/"), "{status}");
    // One commit not on the remote yet.
    assert_eq!(ship::facts(&s.root).unpushed, 1);
    assert!(ship::commit(&s.root, &[], "x").is_err());
}

#[test]
fn a_new_branch_is_pushed_with_its_upstream() {
    let s = Setup::new();
    ship::create_branch(&s.root, "add-a-badge").unwrap();
    s.write("badge.txt", "1\n");
    ship::commit(&s.root, &["badge.txt".into()], "Add a badge").unwrap();
    let f = ship::facts(&s.root);
    assert_eq!(f.branch.as_deref(), Some("add-a-badge"));
    assert_eq!(f.upstream, None);
    assert_eq!(f.unpushed, 1);
    ship::push(&s.root, "origin", "add-a-badge", true, None).unwrap();
    let f = ship::facts(&s.root);
    assert_eq!(f.upstream.as_deref(), Some("origin/add-a-badge"));
    assert_eq!(f.unpushed, 0);
    assert_eq!(git(&s.remote, &["log", "-1", "--format=%s", "add-a-badge"]), "Add a badge");
    assert!(ship::create_branch(&s.root, "bad name").is_err());
}

#[test]
fn a_push_behind_the_remote_is_rejected() {
    let s = Setup::new();
    // Someone else pushes first.
    let other = s.base.join("other");
    git(&s.base, &["clone", "-q", s.remote.to_str().unwrap(), other.to_str().unwrap()]);
    configure(&other);
    fs::write(other.join("theirs.txt"), "t\n").unwrap();
    git(&other, &["add", "-A"]);
    git(&other, &["commit", "-q", "-m", "Theirs"]);
    git(&other, &["push", "-q", "origin", "main"]);
    s.write("mine.txt", "m\n");
    ship::commit(&s.root, &["mine.txt".into()], "Mine").unwrap();
    match ship::push(&s.root, "origin", "main", false, None) {
        Err(PushError::Rejected(out)) => assert!(out.contains("rejected"), "{out}"),
        other => panic!("{other:?}"),
    }
    // The commit stays.
    assert_eq!(git(&s.root, &["log", "-1", "--format=%s"]), "Mine");
}

#[test]
fn a_merge_in_progress_is_seen_with_its_conflicts() {
    let s = Setup::new();
    git(&s.root, &["switch", "-q", "-c", "side"]);
    s.write("README.md", "side\n");
    git(&s.root, &["commit", "-q", "-am", "Side"]);
    git(&s.root, &["switch", "-q", "main"]);
    s.write("README.md", "main\n");
    git(&s.root, &["commit", "-q", "-am", "Main"]);
    let out = Command::new("git").current_dir(&s.root).args(["merge", "side"]).output().unwrap();
    assert!(!out.status.success());
    let f = ship::facts(&s.root);
    assert_eq!(f.busy, Some(Busy::Merge));
    assert_eq!(f.conflicts, vec!["README.md".to_string()]);
}

#[test]
fn a_diff_to_write_a_message_from() {
    let s = Setup::new();
    s.write("a.txt", &"line\n".repeat(50));
    s.write("b.txt", "b\n");
    let snap = tc_git::snapshot(&s.root).unwrap();
    let head = tc_git::head_tree(&s.root).unwrap();
    let text = ship::diff_text(&s.root, &head, &snap.tree, &["a.txt".into()], 100_000).unwrap();
    assert!(text.contains("a.txt") && !text.contains("b.txt"), "{text}");
    let cut = ship::diff_text(&s.root, &head, &snap.tree, &["a.txt".into()], 120).unwrap();
    assert!(cut.len() < 200 && cut.contains("left out"), "{cut}");
}

#[test]
fn a_repository_without_a_remote() {
    let dir = tempfile::tempdir().unwrap();
    let root = canonical(dir.path());
    git(&root, &["init", "-q", "-b", "main"]);
    configure(&root);
    let f = ship::facts(&root);
    assert_eq!(f.branch.as_deref(), Some("main"));
    assert!(!f.has_commits);
    assert_eq!(f.remote, None);
    fs::write(root.join("x.txt"), "x\n").unwrap();
    // The very first commit.
    ship::commit(&root, &["x.txt".into()], "Start").unwrap();
    assert!(ship::facts(&root).has_commits);
}
