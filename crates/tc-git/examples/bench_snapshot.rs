//! Times `snapshot` on a real repository and checks that it left the repository alone.
//!
//! ```text
//! cargo run --release -p tc-git --example bench_snapshot -- [path to a repository]
//! ```
//!
//! Without an argument it uses the TernaryShell checkout next to this workspace.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../ternaryshell"));
    let root = tc_git::repo_root(&path)
        .unwrap_or_else(|| panic!("{} is not inside a git working tree", path.display()));
    let index = PathBuf::from(
        git(
            &root,
            &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        )
        .trim(),
    );
    let read_state = || {
        let bytes = std::fs::read(&index).ok();
        let modified = std::fs::metadata(&index).and_then(|m| m.modified()).ok();
        let status = git(&root, &["--no-optional-locks", "status", "--porcelain"]);
        let refs = git(&root, &["for-each-ref"]);
        let stash = git(&root, &["stash", "list"]);
        (bytes, modified, status, refs, stash)
    };
    let before = read_state();
    let files = git(&root, &["ls-files", "-z"]).matches('\0').count();
    println!("{} ({files} tracked files)", root.display());

    let mut times = Vec::new();
    let mut tree = String::new();
    for run in 1..=6 {
        let start = Instant::now();
        let snapshot = tc_git::snapshot(&root).expect("snapshot failed");
        let elapsed = start.elapsed();
        println!(
            "snapshot {run}: {:>7.1} ms  tree {}",
            ms(elapsed),
            snapshot.tree
        );
        if run > 1 {
            assert_eq!(snapshot.tree, tree, "the tree changed between runs");
            times.push(elapsed);
        }
        tree = snapshot.tree;
    }
    times.sort();
    println!("median of runs 2-6: {:.1} ms", ms(times[times.len() / 2]));

    let start = Instant::now();
    let (head, now) = tc_git::worktree_vs_head(&root).expect("worktree_vs_head failed");
    let from = head.unwrap_or_else(|| tc_git::empty_tree(&root).unwrap());
    let changes = tc_git::changes(&root, &from, &now).expect("changes failed");
    println!(
        "worktree_vs_head + changes: {:.1} ms, {} changed files",
        ms(start.elapsed()),
        changes.len()
    );

    assert!(before == read_state(), "the repository changed");
    println!("index bytes and mtime, status, refs and stash unchanged");
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("could not run git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}
