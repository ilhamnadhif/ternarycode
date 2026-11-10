//! Tests against real repositories created in temporary directories.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tc_git::{
    ChangeKind, DiffLine, EMPTY_TREE, FileChange, LineKind, changes, empty_tree, file_diff,
    head_tree, repo_root, snapshot, worktree_vs_head,
};

struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Repo {
    fn new() -> Repo {
        Repo::init(&[])
    }

    fn init(extra_args: &[&str]) -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let root = canonical(dir.path()).join("repo");
        fs::create_dir(&root).unwrap();
        let mut args = vec!["init", "-q", "-b", "main"];
        args.extend(extra_args);
        git(&root, &args);
        // Local config only, so the user's global config (signing, hooks, autocrlf) does not
        // leak into the tests.
        for (key, value) in [
            ("user.name", "Test"),
            ("user.email", "test@example.com"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", "/dev/null"),
            ("core.autocrlf", "false"),
        ] {
            git(&root, &["config", key, value]);
        }
        Repo { _dir: dir, root }
    }

    fn write(&self, path: &str, content: impl AsRef<[u8]>) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn remove(&self, path: &str) {
        fs::remove_file(self.root.join(path)).unwrap();
    }

    fn git(&self, args: &[&str]) -> String {
        git(&self.root, args)
    }

    fn commit_all(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--no-verify", "-m", message]);
    }
}

/// `canonicalize` without the `\\?\` prefix it adds on Windows: git, and paths joined to it
/// with `/`, need the plain form.
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
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn numbered(lines: std::ops::RangeInclusive<u32>) -> String {
    lines.map(|n| format!("line{n}\n")).collect()
}

fn find<'a>(changes: &'a [FileChange], path: &str) -> &'a FileChange {
    changes
        .iter()
        .find(|change| change.path == path)
        .unwrap_or_else(|| panic!("{path} not in {changes:#?}"))
}

fn paths(changes: &[FileChange]) -> Vec<&str> {
    let mut paths: Vec<_> = changes.iter().map(|change| change.path.as_str()).collect();
    paths.sort();
    paths
}

fn lines(lines: &[DiffLine]) -> Vec<(LineKind, Option<u32>, Option<u32>, &str)> {
    lines
        .iter()
        .map(|line| (line.kind, line.old_no, line.new_no, line.text.as_str()))
        .collect()
}

/// Every file under `dir` with its content, skipping `skip` (relative to `dir`).
fn files(dir: &Path, skip: &[&str]) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(dir).unwrap().to_path_buf();
            if skip.iter().any(|skip| relative == Path::new(skip)) {
                continue;
            }
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push((relative, fs::read(&path).unwrap()));
            }
        }
    }
    found.sort();
    found
}

/// Everything about a repository that a snapshot must leave alone.
#[derive(Debug, PartialEq)]
struct RepoState {
    index_modified: std::time::SystemTime,
    stash: String,
    refs: String,
    status: String,
    /// Working tree files, and every file in `.git` except objects (index, config, refs, logs).
    files: Vec<(PathBuf, Vec<u8>)>,
}

fn state(root: &Path) -> RepoState {
    RepoState {
        index_modified: fs::metadata(root.join(".git/index"))
            .unwrap()
            .modified()
            .unwrap(),
        stash: git(root, &["stash", "list"]),
        refs: git(root, &["for-each-ref"]),
        status: git(
            root,
            &[
                "--no-optional-locks",
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignored",
            ],
        ),
        files: files(root, &[".git/objects"]),
    }
}

#[test]
fn snapshot_leaves_index_refs_stash_and_worktree_alone() {
    let repo = Repo::new();
    repo.write(".gitignore", "*.log\n");
    repo.write("tracked.txt", "one\n");
    repo.write("staged.txt", "staged\n");
    repo.write("gone.txt", "gone\n");
    repo.commit_all("first");
    repo.write("tracked.txt", "one\nstashed\n");
    repo.git(&["stash", "-q"]);
    repo.write("staged.txt", "staged\nmore\n");
    repo.git(&["add", "staged.txt"]);
    repo.write("tracked.txt", "one\nunstaged\n");
    repo.write("new dir/untracked.txt", "untracked\n");
    repo.write("debug.log", "ignored\n");
    repo.remove("gone.txt");

    let before = state(&repo.root);
    assert!(before.stash.contains("stash@{0}"));
    let index_bytes = fs::read(repo.root.join(".git/index")).unwrap();

    let first = snapshot(&repo.root).unwrap();
    let second = snapshot(&repo.root).unwrap();
    assert_eq!(first, second);
    assert_eq!(state(&repo.root), before);
    assert_eq!(fs::read(repo.root.join(".git/index")).unwrap(), index_bytes);

    let head = repo.git(&["rev-parse", "HEAD"]);
    assert_eq!(first.head.as_deref(), Some(head.trim()));
    let found = changes(&repo.root, &head_tree(&repo.root).unwrap(), &first.tree).unwrap();
    assert_eq!(
        paths(&found),
        [
            "gone.txt",
            "new dir/untracked.txt",
            "staged.txt",
            "tracked.txt"
        ]
    );
    assert_eq!(find(&found, "gone.txt").kind, ChangeKind::Deleted);
    assert_eq!(
        find(&found, "new dir/untracked.txt").kind,
        ChangeKind::Added
    );
    assert_eq!(find(&found, "tracked.txt").kind, ChangeKind::Modified);
}

#[test]
fn detects_changes_between_two_snapshots() {
    let repo = Repo::new();
    repo.write("a.txt", "alpha\n");
    repo.write("b.txt", "bravo\nbravo\n");
    repo.write("c.txt", numbered(1..=20));
    repo.commit_all("first");
    let start = snapshot(&repo.root).unwrap();

    repo.write("a.txt", "alpha\nalpha2\n");
    repo.remove("b.txt");
    fs::create_dir(repo.root.join("sub")).unwrap();
    fs::rename(repo.root.join("c.txt"), repo.root.join("sub/d.txt")).unwrap();
    repo.write("e.txt", "echo\n");
    let end = snapshot(&repo.root).unwrap();

    let found = changes(&repo.root, &start.tree, &end.tree).unwrap();
    assert_eq!(paths(&found), ["a.txt", "b.txt", "e.txt", "sub/d.txt"]);
    let expect = |path: &str, kind, old_path: Option<&str>, additions, deletions| {
        let change = find(&found, path);
        assert_eq!(change.kind, kind, "{path}");
        assert_eq!(change.old_path.as_deref(), old_path, "{path}");
        assert_eq!(change.additions, Some(additions), "{path}");
        assert_eq!(change.deletions, Some(deletions), "{path}");
        assert!(!change.binary, "{path}");
    };
    expect("a.txt", ChangeKind::Modified, None, 1, 0);
    expect("b.txt", ChangeKind::Deleted, None, 0, 2);
    expect("e.txt", ChangeKind::Added, None, 1, 0);
    expect("sub/d.txt", ChangeKind::Renamed, Some("c.txt"), 0, 0);

    // Nothing changed since the end snapshot.
    let again = snapshot(&repo.root).unwrap();
    assert_eq!(again.tree, end.tree);
    assert!(
        changes(&repo.root, &end.tree, &again.tree)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn ignored_files_are_left_out_but_tracked_ones_stay() {
    let repo = Repo::new();
    repo.write(".gitignore", "*.ignored\nbuild/\n");
    repo.write("forced.ignored", "tracked anyway\n");
    repo.git(&["add", "-f", "forced.ignored"]);
    repo.commit_all("first");

    repo.write("forced.ignored", "tracked anyway\nchanged\n");
    repo.write("scratch.ignored", "x\n");
    repo.write("build/out.bin", "x\n");
    repo.write("normal.txt", "x\n");
    let (head, now) = worktree_vs_head(&repo.root).unwrap();
    let found = changes(&repo.root, &head.unwrap(), &now).unwrap();
    assert_eq!(paths(&found), ["forced.ignored", "normal.txt"]);
}

#[test]
fn binary_files_are_flagged() {
    let repo = Repo::new();
    repo.write("image.bin", b"\x89PNG\x00\x01\x02\x03");
    repo.commit_all("first");
    let start = snapshot(&repo.root).unwrap();
    repo.write("image.bin", b"\x89PNG\x00\x01\x02\x04\x05");
    repo.write("new.bin", b"\x00\x00\x00");
    let end = snapshot(&repo.root).unwrap();

    let found = changes(&repo.root, &start.tree, &end.tree).unwrap();
    for (path, kind) in [
        ("image.bin", ChangeKind::Modified),
        ("new.bin", ChangeKind::Added),
    ] {
        let change = find(&found, path);
        assert_eq!(change.kind, kind);
        assert!(change.binary);
        assert_eq!((change.additions, change.deletions), (None, None));
        let diff = file_diff(&repo.root, &start.tree, &end.tree, path, None).unwrap();
        assert!(diff.binary);
        assert!(diff.hunks.is_empty());
    }
}

#[test]
fn works_without_commits() {
    let repo = Repo::new();
    assert_eq!(empty_tree(&repo.root).unwrap(), EMPTY_TREE);
    assert_eq!(head_tree(&repo.root), None);
    // Nothing on disk, nothing staged, no index file yet.
    assert!(!repo.root.join(".git/index").exists());
    let empty = snapshot(&repo.root).unwrap();
    assert_eq!(empty.tree, EMPTY_TREE);
    assert_eq!(empty.head, None);
    assert!(!repo.root.join(".git/index").exists());

    repo.write("staged.txt", "first\nsecond\n");
    repo.git(&["add", "staged.txt"]);
    repo.write("src/main.rs", "fn main() {}\n");
    let index_bytes = fs::read(repo.root.join(".git/index")).unwrap();

    let (head, now) = worktree_vs_head(&repo.root).unwrap();
    assert_eq!(head, None);
    assert_eq!(fs::read(repo.root.join(".git/index")).unwrap(), index_bytes);
    let found = changes(&repo.root, EMPTY_TREE, &now).unwrap();
    assert_eq!(paths(&found), ["src/main.rs", "staged.txt"]);
    assert!(found.iter().all(|change| change.kind == ChangeKind::Added));

    let diff = file_diff(&repo.root, EMPTY_TREE, &now, "staged.txt", None).unwrap();
    assert_eq!(diff.hunks.len(), 1);
    assert_eq!(diff.hunks[0].header, "@@ -0,0 +1,2 @@");
    assert_eq!(
        lines(&diff.hunks[0].lines),
        [
            (LineKind::Added, None, Some(1), "first"),
            (LineKind::Added, None, Some(2), "second"),
        ]
    );
    // Only `git status` itself knows about the staged file; HEAD is still unborn.
    assert!(
        repo.git(&["status", "--porcelain"])
            .contains("A  staged.txt")
    );
}

#[test]
fn paths_with_spaces_unicode_quotes_and_glob_characters() {
    let repo = Repo::new();
    let names = [
        "dir with space/file name.txt",
        "café/naïve résumé.md",
        "日本語.txt",
        "weird \"quote\" and\ttab.txt",
        "back\\slash.txt",
        "star*.txt",
        "starX.txt",
    ];
    for name in names {
        repo.write(name, format!("{name}\n"));
    }
    repo.write("old name.txt", numbered(1..=10));
    repo.commit_all("first");
    let start = snapshot(&repo.root).unwrap();

    for name in names {
        repo.write(name, format!("{name}\nedited {name}\n"));
    }
    repo.remove("old name.txt");
    repo.write("new näme.txt", numbered(1..=9) + "line10 edited\n");
    repo.write("ünïcode new.txt", "brand new\n");
    let end = snapshot(&repo.root).unwrap();

    let found = changes(&repo.root, &start.tree, &end.tree).unwrap();
    let mut expected: Vec<&str> = names.to_vec();
    expected.extend(["new näme.txt", "ünïcode new.txt"]);
    expected.sort();
    assert_eq!(paths(&found), expected);

    for name in names {
        let change = find(&found, name);
        assert_eq!(change.kind, ChangeKind::Modified, "{name}");
        assert_eq!((change.additions, change.deletions), (Some(1), Some(0)));
        let diff = file_diff(&repo.root, &start.tree, &end.tree, name, None).unwrap();
        assert_eq!(diff.hunks.len(), 1, "{name}: {diff:#?}");
        assert_eq!(
            lines(&diff.hunks[0].lines),
            [
                (LineKind::Context, Some(1), Some(1), name),
                (
                    LineKind::Added,
                    None,
                    Some(2),
                    format!("edited {name}").as_str()
                ),
            ],
        );
    }

    let renamed = find(&found, "new näme.txt");
    assert_eq!(renamed.kind, ChangeKind::Renamed);
    assert_eq!(renamed.old_path.as_deref(), Some("old name.txt"));
    let diff = file_diff(
        &repo.root,
        &start.tree,
        &end.tree,
        "new näme.txt",
        Some("old name.txt"),
    )
    .unwrap();
    assert_eq!(diff.hunks.len(), 1);
    assert_eq!(diff.hunks[0].header, "@@ -7,4 +7,4 @@ line6");
    // Without the old path git cannot pair the rename: the whole file shows as added.
    let alone = file_diff(&repo.root, &start.tree, &end.tree, "new näme.txt", None).unwrap();
    assert_eq!(alone.hunks[0].header, "@@ -0,0 +1,10 @@");
}

#[test]
fn file_diff_line_numbers() {
    let repo = Repo::new();
    repo.write("f.txt", numbered(1..=20));
    repo.commit_all("first");
    let start = snapshot(&repo.root).unwrap();

    let mut new = numbered(1..=2) + "ins-a\nins-b\n";
    new += &numbered(3..=9);
    new += "CHANGED10\n";
    new += &numbered(11..=17);
    new += &numbered(19..=20);
    repo.write("f.txt", new);
    let end = snapshot(&repo.root).unwrap();

    let diff = file_diff(&repo.root, &start.tree, &end.tree, "f.txt", None).unwrap();
    assert!(!diff.binary);
    let ranges: Vec<_> = diff
        .hunks
        .iter()
        .map(|h| (h.old_start, h.old_lines, h.new_start, h.new_lines))
        .collect();
    assert_eq!(ranges, [(1, 5, 1, 7), (7, 7, 9, 7), (15, 6, 17, 5)]);
    assert_eq!(diff.hunks[1].header, "@@ -7,7 +9,7 @@ line6");
    assert_eq!(
        lines(&diff.hunks[1].lines),
        [
            (LineKind::Context, Some(7), Some(9), "line7"),
            (LineKind::Context, Some(8), Some(10), "line8"),
            (LineKind::Context, Some(9), Some(11), "line9"),
            (LineKind::Removed, Some(10), None, "line10"),
            (LineKind::Added, None, Some(12), "CHANGED10"),
            (LineKind::Context, Some(11), Some(13), "line11"),
            (LineKind::Context, Some(12), Some(14), "line12"),
            (LineKind::Context, Some(13), Some(15), "line13"),
        ]
    );
    assert_eq!(
        lines(&diff.hunks[2].lines)[3],
        (LineKind::Removed, Some(18), None, "line18")
    );
    let insert = lines(&diff.hunks[0].lines);
    assert_eq!(insert[2], (LineKind::Added, None, Some(3), "ins-a"));
    assert_eq!(insert[4], (LineKind::Context, Some(3), Some(5), "line3"));

    let change = &changes(&repo.root, &start.tree, &end.tree).unwrap()[0];
    assert_eq!((change.additions, change.deletions), (Some(3), Some(2)));
    // An unchanged file has no hunks.
    repo.write("same.txt", "same\n");
    let diff = file_diff(&repo.root, &start.tree, &end.tree, "same.txt", None).unwrap();
    assert_eq!(diff, tc_git::FileDiff::default());
}

#[test]
fn crlf_and_missing_final_newline() {
    let repo = Repo::new();
    repo.write("win.txt", "one\r\ntwo\r\nthree");
    repo.commit_all("first");
    repo.write("win.txt", "one\r\nTWO\r\nthree\r\nfour\r\n");
    let (head, now) = worktree_vs_head(&repo.root).unwrap();
    let head = head.unwrap();

    let diff = file_diff(&repo.root, &head, &now, "win.txt", None).unwrap();
    assert_eq!(diff.hunks.len(), 1);
    assert_eq!(diff.hunks[0].header, "@@ -1,3 +1,4 @@");
    assert_eq!(
        lines(&diff.hunks[0].lines),
        [
            (LineKind::Context, Some(1), Some(1), "one"),
            (LineKind::Removed, Some(2), None, "two"),
            (LineKind::Removed, Some(3), None, "three"),
            (LineKind::Added, None, Some(2), "TWO"),
            (LineKind::Added, None, Some(3), "three"),
            (LineKind::Added, None, Some(4), "four"),
        ]
    );
}

// Symbolic links need extra rights on Windows.
#[cfg(unix)]
#[test]
fn file_replaced_by_directory_and_symlink_type_change() {
    let repo = Repo::new();
    repo.write("thing", "a file\n");
    repo.write("target.txt", "target\n");
    std::os::unix::fs::symlink("target.txt", repo.root.join("link")).unwrap();
    repo.commit_all("first");
    let start = snapshot(&repo.root).unwrap();

    repo.remove("thing");
    repo.write("thing/inside.txt", "inside\n");
    repo.remove("link");
    repo.write("link", "now a file\n");
    let end = snapshot(&repo.root).unwrap();

    let found = changes(&repo.root, &start.tree, &end.tree).unwrap();
    assert_eq!(find(&found, "thing").kind, ChangeKind::Deleted);
    assert_eq!(find(&found, "thing/inside.txt").kind, ChangeKind::Added);
    assert_eq!(find(&found, "link").kind, ChangeKind::Modified);

    // The pathspec `thing` also matches `thing/inside.txt`; only the file `thing` is returned.
    let diff = file_diff(&repo.root, &start.tree, &end.tree, "thing", None).unwrap();
    assert_eq!(diff.hunks.len(), 1);
    assert_eq!(
        lines(&diff.hunks[0].lines),
        [(LineKind::Removed, Some(1), None, "a file")]
    );
    // A type change is printed as a deletion and an addition of the same path.
    let diff = file_diff(&repo.root, &start.tree, &end.tree, "link", None).unwrap();
    let all: Vec<_> = diff.hunks.iter().flat_map(|h| lines(&h.lines)).collect();
    assert_eq!(
        all,
        [
            (LineKind::Removed, Some(1), None, "target.txt"),
            (LineKind::Added, None, Some(1), "now a file"),
        ]
    );
}

#[test]
fn user_diff_config_does_not_change_the_output() {
    let repo = Repo::new();
    for (key, value) in [
        ("diff.noprefix", "true"),
        ("diff.mnemonicPrefix", "true"),
        ("diff.suppressBlankEmpty", "true"),
        ("diff.relative", "true"),
        ("diff.renames", "false"),
        ("diff.external", "/usr/bin/false"),
        ("diff.context", "10"),
        ("diff.upper.textconv", "tr a-z A-Z"),
        ("color.ui", "always"),
        ("color.diff", "always"),
        ("core.quotepath", "true"),
    ] {
        repo.git(&["config", key, value]);
    }
    repo.write(".gitattributes", "*.txt diff=upper\n");
    repo.write("sub/blank.txt", "a\n\nb\nc\n");
    repo.write("sub/moved.txt", numbered(1..=10));
    repo.write("é.txt", "x\n");
    repo.commit_all("first");
    let start = snapshot(&repo.root).unwrap();
    repo.write("sub/blank.txt", "a\n\nB\nc\n");
    fs::rename(repo.root.join("sub/moved.txt"), repo.root.join("moved.txt")).unwrap();
    repo.write("é.txt", "y\n");
    // Run from a subdirectory, where `diff.relative` would hide and shorten paths.
    let sub = repo.root.join("sub");
    let end = snapshot(&sub).unwrap();

    let found = changes(&sub, &start.tree, &end.tree).unwrap();
    assert_eq!(paths(&found), ["moved.txt", "sub/blank.txt", "é.txt"]);
    assert_eq!(find(&found, "moved.txt").kind, ChangeKind::Renamed);

    let diff = file_diff(&sub, &start.tree, &end.tree, "sub/blank.txt", None).unwrap();
    assert_eq!(diff.hunks[0].header, "@@ -1,4 +1,4 @@");
    assert_eq!(
        lines(&diff.hunks[0].lines),
        [
            (LineKind::Context, Some(1), Some(1), "a"),
            (LineKind::Context, Some(2), Some(2), ""),
            (LineKind::Removed, Some(3), None, "b"),
            (LineKind::Added, None, Some(3), "B"),
            (LineKind::Context, Some(4), Some(4), "c"),
        ]
    );
    let diff = file_diff(&sub, &start.tree, &end.tree, "é.txt", None).unwrap();
    assert_eq!(diff.hunks.len(), 1);
    let diff = file_diff(
        &sub,
        &start.tree,
        &end.tree,
        "moved.txt",
        Some("sub/moved.txt"),
    )
    .unwrap();
    assert!(diff.hunks.is_empty() && !diff.binary);
}

#[test]
fn sha256_repository() {
    let repo = Repo::init(&["--object-format=sha256"]);
    let empty = empty_tree(&repo.root).unwrap();
    assert_eq!(empty.len(), 64);
    repo.write("a.txt", "a\n");
    let snap = snapshot(&repo.root).unwrap();
    assert_eq!(snap.tree.len(), 64);
    let found = changes(&repo.root, &empty, &snap.tree).unwrap();
    assert_eq!(paths(&found), ["a.txt"]);
}

#[test]
fn linked_worktree_uses_its_own_index() {
    let repo = Repo::new();
    repo.write("a.txt", "a\n");
    repo.commit_all("first");
    let worktree = repo.root.parent().unwrap().join("wt");
    repo.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "side",
        worktree.to_str().unwrap(),
    ]);
    fs::write(worktree.join("a.txt"), "a\nside\n").unwrap();
    fs::write(worktree.join("b.txt"), "b\n").unwrap();
    let main_index = fs::read(repo.root.join(".git/index")).unwrap();
    let side_index = fs::read(repo.root.join(".git/worktrees/wt/index")).unwrap();

    assert_eq!(repo_root(&worktree.join("a.txt")), Some(worktree.clone()));
    let (head, now) = worktree_vs_head(&worktree).unwrap();
    let found = changes(&worktree, &head.unwrap(), &now).unwrap();
    assert_eq!(paths(&found), ["a.txt", "b.txt"]);
    assert_eq!(fs::read(repo.root.join(".git/index")).unwrap(), main_index);
    assert_eq!(
        fs::read(repo.root.join(".git/worktrees/wt/index")).unwrap(),
        side_index
    );
    // The main working tree is unaffected.
    let (head, now) = worktree_vs_head(&repo.root).unwrap();
    assert_eq!(head.as_deref(), Some(now.as_str()));
}

#[test]
fn repo_root_finds_the_top_level() {
    let repo = Repo::new();
    repo.write("sub/deeper/file.txt", "x\n");
    assert_eq!(repo_root(&repo.root), Some(repo.root.clone()));
    assert_eq!(
        repo_root(&repo.root.join("sub/deeper")),
        Some(repo.root.clone())
    );
    assert_eq!(
        repo_root(&repo.root.join("sub/deeper/file.txt")),
        Some(repo.root.clone())
    );
    let outside = tempfile::tempdir().unwrap();
    assert_eq!(repo_root(outside.path()), None);
    assert_eq!(repo_root(&repo.root.join("does/not/exist")), None);
}

#[test]
fn errors_carry_git_stderr() {
    let repo = Repo::new();
    repo.write("a.txt", "a\n");
    repo.commit_all("first");
    let err = changes(
        &repo.root,
        EMPTY_TREE,
        "0123456789012345678901234567890123456789",
    )
    .unwrap_err();
    assert!(err.contains("git diff"), "{err}");
    assert!(
        err.contains("bad object") || err.contains("0123456789"),
        "{err}"
    );
    assert!(changes(&repo.root, "--output=x", EMPTY_TREE).is_err());
    assert!(!repo.root.join("x").exists());

    // A nested repository without commits cannot be indexed: the snapshot fails with git's
    // message instead of silently leaving the directory out, and the repository is untouched.
    fs::create_dir(repo.root.join("nested")).unwrap();
    git(&repo.root.join("nested"), &["init", "-q"]);
    let before = state(&repo.root);
    let err = snapshot(&repo.root).unwrap_err();
    assert!(err.contains("does not have a commit checked out"), "{err}");
    assert_eq!(state(&repo.root), before);

    let outside = tempfile::tempdir().unwrap();
    assert!(
        snapshot(outside.path())
            .unwrap_err()
            .contains("not a git repository")
    );
}
