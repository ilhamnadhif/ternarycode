//! Git operations for reviewing what an AI agent changed since its run started (PRD FR-E4) and
//! what changed since the last commit (FR-E5). Everything goes through the `git` command line.
//!
//! The central piece is [`snapshot`]: it records the whole working tree (tracked and untracked
//! files, minus ignored ones) as a git tree object without touching the user's repository state.
//! It stages into a temporary index file (`GIT_INDEX_FILE`) that starts as a copy of the real
//! index, so git only re-hashes files whose stat data changed, and then runs `git write-tree`. The
//! real index, refs, stash, config and working tree are never written. The only trace is the new
//! objects, which no ref points to: `git gc` prunes them once they are older than
//! `gc.pruneExpire` (two weeks by default).
//!
//! Two trees (two snapshots, or [`head_tree`] and a snapshot) are compared with [`changes`] for
//! the list of files and with [`file_diff`] for the hunks of one file. In a repository without
//! commits, [`empty_tree`] is the starting point.
//!
//! Every function blocks until git exits, so call them off the UI thread. Errors are
//! human-readable strings that include git's stderr.

mod parse;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The id of the empty tree in a SHA-1 repository. [`empty_tree`] also covers SHA-256
/// repositories.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// The state of a working tree at one moment, as returned by [`snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Tree id of the working tree: tracked and untracked files, without ignored files.
    pub tree: String,
    /// Commit id of `HEAD` when the snapshot was taken; `None` in a repository without commits.
    pub head: Option<String>,
}

/// How a file changed between two trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Added,
    /// Content, mode or type changed.
    Modified,
    Deleted,
    /// Moved, possibly with edits; [`FileChange::old_path`] holds the previous path.
    Renamed,
}

/// One changed file, as returned by [`changes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Path relative to the repository root, with `/` separators. For a deleted file, the path
    /// it had.
    pub path: String,
    /// The previous path; only set for [`ChangeKind::Renamed`].
    pub old_path: Option<String>,
    pub kind: ChangeKind,
    /// Lines added; `None` for binary files.
    pub additions: Option<u32>,
    /// Lines removed; `None` for binary files.
    pub deletions: Option<u32>,
    pub binary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// One line of a hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// 1-based line number in the old file; `None` for added lines.
    pub old_no: Option<u32>,
    /// 1-based line number in the new file; `None` for removed lines.
    pub new_no: Option<u32>,
    /// The line without its `+`/`-`/space prefix and without the line ending (`\n` or `\r\n`).
    /// Bytes that are not UTF-8 are replaced with U+FFFD.
    pub text: String,
}

/// One hunk of a unified diff with three lines of context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// The whole hunk header line, such as `@@ -10,7 +10,8 @@ fn main() {`.
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// The diff of one file, as returned by [`file_diff`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// Git considers the file binary; `hunks` is then empty.
    pub binary: bool,
    /// Empty when the file did not change, only its mode changed, or it is binary.
    pub hunks: Vec<Hunk>,
}

/// Returns the top-level directory of the working tree that contains `path` (a directory or a
/// file), or `None` when `path` is not inside a git working tree or git cannot run.
pub fn repo_root(path: &Path) -> Option<PathBuf> {
    let dir = if path.is_dir() {
        path
    } else {
        match path.parent()? {
            parent if parent.as_os_str().is_empty() => Path::new("."),
            parent => parent,
        }
    };
    let out = run(git(dir).args(["rev-parse", "--show-toplevel"])).ok()?;
    let line = trim_line(&out);
    (!line.is_empty()).then(|| path_from_bytes(line))
}

/// Records the current working tree of the repository at `root` as a tree object, without
/// changing the repository: not the index, refs, stash, config or any file in the working tree.
///
/// The tree holds every file `git add --all` would stage: tracked files as they are on disk,
/// untracked files, and deletions. Ignored files are left out unless they are already tracked.
/// Fails if git cannot index a file, for example a nested repository without commits.
pub fn snapshot(root: &Path) -> Result<Snapshot, String> {
    let index = git_dir_path(root, "index")?;
    let head = rev_parse(root, "HEAD^{commit}");
    let temp = TempIndex::copy_of(&index)?;
    // A split index would make git write a new shared index file into the git directory.
    run(git(root).env("GIT_INDEX_FILE", &temp.file).args([
        "-c",
        "core.splitIndex=false",
        "-c",
        "core.safecrlf=false",
        "-c",
        "advice.addEmbeddedRepo=false",
        "add",
        "--all",
    ]))?;
    let out = run(git(root).env("GIT_INDEX_FILE", &temp.file).args([
        "-c",
        "core.splitIndex=false",
        "write-tree",
    ]))?;
    Ok(Snapshot {
        tree: parse_oid(&out)?,
        head,
    })
}

/// Returns the tree id of `HEAD`, or `None` in a repository without commits (or outside one).
pub fn head_tree(root: &Path) -> Option<String> {
    rev_parse(root, "HEAD^{tree}")
}

/// Returns the id of the empty tree for this repository's hash algorithm. Use it as `from_tree`
/// in a repository without commits; git knows this tree even though it is never stored.
pub fn empty_tree(root: &Path) -> Result<String, String> {
    // stdin is null, so this hashes zero bytes as a tree object without writing it.
    let out = run(git(root).args(["hash-object", "-t", "tree", "--stdin"]))?;
    parse_oid(&out)
}

/// Returns the files that differ between two trees, in git's path order, with renames detected
/// (`-M`). `from_tree` and `to_tree` are tree ids (or anything git resolves to a tree, such as a
/// commit id).
pub fn changes(root: &Path, from_tree: &str, to_tree: &str) -> Result<Vec<FileChange>, String> {
    check_rev(from_tree)?;
    check_rev(to_tree)?;
    // `--raw` carries the same status letters as `--name-status`; unlike `--name-status` it can
    // be combined with `--numstat`, so one git run (and one rename detection) gives both.
    let out = run(git(root).args(DIFF_ARGS).args([
        "--raw",
        "--numstat",
        "-z",
        from_tree,
        to_tree,
        "--",
    ]))?;
    parse::changes(&out)
}

/// Returns the unified diff (three lines of context) of one file between two trees. For a
/// renamed file, pass the previous path as `old_path` so that the diff shows the edits instead of
/// a deletion and an addition. A file that did not change gives an empty [`FileDiff`].
pub fn file_diff(
    root: &Path,
    from_tree: &str,
    to_tree: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<FileDiff, String> {
    check_rev(from_tree)?;
    check_rev(to_tree)?;
    let mut cmd = git(root);
    // Explicit prefixes override `diff.noprefix` and `diff.mnemonicPrefix`, so headers parse the
    // same everywhere.
    cmd.args(DIFF_ARGS).args([
        "--unified=3",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        from_tree,
        to_tree,
        "--",
    ]);
    let old_path = old_path.filter(|old| *old != path);
    if let Some(old) = old_path {
        cmd.arg(literal_pathspec(old));
    }
    cmd.arg(literal_pathspec(path));
    let out = run(&mut cmd)?;
    parse::file_diff(&out, path, old_path)
}

/// Returns the tree of `HEAD` (`None` without commits) and a [`snapshot`] tree of the working
/// tree now: compare them for "changes since the last commit", staged or not.
pub fn worktree_vs_head(root: &Path) -> Result<(Option<String>, String), String> {
    let snapshot = snapshot(root)?;
    let head_tree = match &snapshot.head {
        Some(commit) => Some(
            rev_parse(root, &format!("{commit}^{{tree}}"))
                .ok_or_else(|| format!("could not read the tree of HEAD ({commit})"))?,
        ),
        None => None,
    };
    Ok((head_tree, snapshot.tree))
}

/// Options shared by every diff, chosen so that user config cannot change the output format:
/// no colors, no external diff or textconv drivers, paths relative to the repository root, and
/// renames detected even with `diff.renames=false`.
const DIFF_ARGS: [&str; 6] = [
    "diff",
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--no-relative",
    "-M",
];

/// Variables that would point git at another repository, index, object store or config, or
/// change how pathspecs are read. They may be inherited when the app is started from a git hook
/// or a shell that sets them; git clears the same list (`git rev-parse --local-env-vars`) when it
/// runs commands in submodules.
const INHERITED_GIT_VARS: [&str; 22] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_PREFIX",
    "GIT_NAMESPACE",
    "GIT_GRAFT_FILE",
    "GIT_SHALLOW_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_LITERAL_PATHSPECS",
    "GIT_GLOB_PATHSPECS",
    "GIT_NOGLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
    "GIT_EXTERNAL_DIFF",
    "GIT_DIFF_OPTS",
];

/// A `git` command running in `dir`, isolated from inherited repository variables, that never
/// takes optional locks (so it never refreshes the user's index) and prints paths unquoted.
fn git(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    // No console window for each git run (the app has none of its own on Windows).
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
    cmd.current_dir(dir)
        .stdin(Stdio::null())
        .env("GIT_OPTIONAL_LOCKS", "0");
    for var in INHERITED_GIT_VARS {
        cmd.env_remove(var);
    }
    cmd.args(["-c", "core.quotepath=off"]);
    cmd
}

/// Runs `cmd` and returns its stdout, or an error with git's stderr when it fails.
fn run(cmd: &mut Command) -> Result<Vec<u8>, String> {
    let output = cmd
        .output()
        .map_err(|err| format!("could not run git: {err}"))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let dir = cmd
        .get_current_dir()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    let detail = if stderr.is_empty() {
        match output.status.code() {
            Some(code) => format!("exit code {code}"),
            None => "killed by a signal".to_string(),
        }
    } else {
        stderr.to_string()
    };
    Err(format!("`{}` failed in {dir}: {detail}", describe(cmd)))
}

/// The command line of `cmd` for error messages, without the `-c` config overrides.
fn describe(cmd: &Command) -> String {
    let mut words = vec!["git".to_string()];
    let mut args = cmd.get_args();
    while let Some(arg) = args.next() {
        if arg == "-c" {
            args.next();
        } else {
            words.push(arg.to_string_lossy().into_owned());
        }
    }
    words.join(" ")
}

/// Resolves `rev` to an object id, or `None` if it does not exist.
fn rev_parse(root: &Path, rev: &str) -> Option<String> {
    let out = run(git(root).args(["rev-parse", "--verify", "--quiet", rev])).ok()?;
    parse_oid(&out).ok()
}

/// The path of `name` inside the git directory, which for a linked worktree is the worktree's
/// own directory under `.git/worktrees`.
fn git_dir_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    let out = run(git(root).args(["rev-parse", "--git-path", name]))?;
    // Relative to the working directory of the command, which `join` keeps; absolute otherwise.
    Ok(root.join(path_from_bytes(trim_line(&out))))
}

/// Rejects revisions git would read as options.
fn check_rev(rev: &str) -> Result<(), String> {
    if rev.is_empty() || rev.starts_with('-') {
        return Err(format!("invalid tree id {rev:?}"));
    }
    Ok(())
}

/// A pathspec that matches `path` relative to the repository root, with `*`, `?` and `[` taken
/// literally. Like any pathspec it also matches files below `path` if it is a directory.
fn literal_pathspec(path: &str) -> String {
    format!(":(top,literal){path}")
}

/// The single object id printed by git.
fn parse_oid(out: &[u8]) -> Result<String, String> {
    let id = String::from_utf8_lossy(trim_line(out)).into_owned();
    let valid = matches!(id.len(), 40 | 64) && id.bytes().all(|b| b.is_ascii_hexdigit());
    if valid {
        Ok(id)
    } else {
        Err(format!(
            "git printed {id:?} where an object id was expected"
        ))
    }
}

fn trim_line(out: &[u8]) -> &[u8] {
    let out = out.strip_suffix(b"\n").unwrap_or(out);
    out.strip_suffix(b"\r").unwrap_or(out)
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

/// Windows: git prints `C:/dir`; the path gets the system's separators.
#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// A private temporary directory, outside the repository, holding the index file a snapshot
/// stages into. Dropping it removes the directory, the index and any `index.lock` left behind.
struct TempIndex {
    dir: PathBuf,
    file: PathBuf,
}

impl TempIndex {
    /// Creates the directory and copies the real index at `source` into it, if there is one.
    fn copy_of(source: &Path) -> Result<Self, String> {
        let dir = create_private_dir()?;
        let temp = TempIndex {
            file: dir.join("index"),
            dir,
        };
        let metadata = match fs::metadata(source) {
            Ok(metadata) => metadata,
            // No index yet (nothing was ever staged): git starts from an empty one.
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(temp),
            Err(err) => return Err(format!("could not read {}: {err}", source.display())),
        };
        fs::copy(source, &temp.file)
            .map_err(|err| format!("could not copy {}: {err}", source.display()))?;
        // Git trusts the stat data of an entry only if the file is older than the index file
        // itself (the "racy git" check). Giving the copy the original's modification time keeps
        // that check as strict as it is for the real index. The time is read before copying, so
        // if git replaced the index meanwhile, the copy only looks older, which is safe.
        let modified = metadata
            .modified()
            .map_err(|err| format!("could not read the time of {}: {err}", source.display()))?;
        fs::File::options()
            .write(true)
            .open(&temp.file)
            .and_then(|file| file.set_modified(modified))
            .map_err(|err| format!("could not set the time of the index copy: {err}"))?;
        Ok(temp)
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Creates a new directory only this user can read, under the system temporary directory.
fn create_private_dir() -> Result<PathBuf, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    let mut attempts = 0;
    loop {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!("tc-git-{}-{nanos}-{n}", std::process::id()));
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists && attempts < 100 => {
                attempts += 1;
            }
            Err(err) => {
                return Err(format!(
                    "could not create a temporary directory in {}: {err}",
                    base.display()
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_index_copies_with_mtime_and_cleans_up() {
        let scratch = tempfile::tempdir().unwrap();
        let source = scratch.path().join("index");
        fs::write(&source, b"DIRC fake index").unwrap();
        let old = UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(old)
            .unwrap();

        let temp = TempIndex::copy_of(&source).unwrap();
        assert!(!temp.dir.starts_with(scratch.path()));
        assert_eq!(fs::read(&temp.file).unwrap(), b"DIRC fake index");
        assert_eq!(fs::metadata(&temp.file).unwrap().modified().unwrap(), old);
        fs::write(temp.dir.join("index.lock"), b"left by a crash").unwrap();
        let dir = temp.dir.clone();
        drop(temp);
        assert!(!dir.exists());

        let missing = TempIndex::copy_of(&scratch.path().join("nope")).unwrap();
        assert!(missing.dir.is_dir());
        assert!(!missing.file.exists());
        let dir = missing.dir.clone();
        drop(missing);
        assert!(!dir.exists());
    }

    #[test]
    fn rejects_revisions_that_look_like_options() {
        assert!(check_rev("--output=/tmp/x").is_err());
        assert!(check_rev("").is_err());
        assert!(check_rev(EMPTY_TREE).is_ok());
    }

    #[test]
    fn describes_commands_without_config_overrides() {
        let mut cmd = git(Path::new("."));
        cmd.args(["-c", "core.splitIndex=false", "add", "--all"]);
        assert_eq!(describe(&cmd), "git add --all");
    }
}
