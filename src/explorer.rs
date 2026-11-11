//! The file tree of a project (the sidebar's Files section). A folder is read once, when it is
//! first shown, and kept until the file-system watcher reports a change in it, so drawing the
//! tree never touches the disk. The visible rows are flattened once per change for the same
//! reason, and drawn virtualised.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// Entries shown per folder; the rest is summarised as "N more".
pub const MAX_ENTRIES: usize = 2000;

/// Never shown.
const HIDDEN: &[&str] = &[".git", ".DS_Store"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_link: bool,
    /// Build output and dependencies (`target`, `node_modules`, ...) and what is under them:
    /// shown dimmed, and the watcher does not report changes in them.
    pub dimmed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// Entries left out above [`MAX_ENTRIES`].
    pub more: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Item { depth: usize, entry: Entry, open: bool },
    More { depth: usize, count: usize },
    /// The name field of a file or folder being created.
    New { depth: usize, folder: bool },
}

/// A name typed in the tree: a new file or folder in `dir`, or a new name for `target`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub dir: PathBuf,
    pub kind: EditKind,
    pub text: String,
    /// Put the keyboard in the field on the next frame.
    pub focus: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditKind {
    NewFile,
    NewFolder,
    Rename(PathBuf),
}

pub struct Explorer {
    pub root: PathBuf,
    dirs: HashMap<PathBuf, Listing>,
    expanded: BTreeSet<PathBuf>,
    /// The file or folder last clicked.
    pub selected: Option<PathBuf>,
    rows: Option<Vec<Row>>,
    /// A name being typed in the tree.
    pub edit: Option<Edit>,
    /// Folders read from disk so far (lets tests check that drawing does not read).
    pub reads: u64,
    /// What the Files column's filter holds (files whose path matches, instead of the tree).
    pub filter: String,
    /// The column lists what git says changed instead of the tree.
    pub changes_mode: bool,
}

impl Explorer {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            dirs: HashMap::new(),
            expanded: BTreeSet::new(),
            selected: None,
            rows: None,
            edit: None,
            reads: 0,
            filter: String::new(),
            changes_mode: false,
        }
    }

    /// Opens a closed folder, closes an open one.
    pub fn toggle(&mut self, dir: &Path) {
        if !self.expanded.remove(dir) {
            self.expanded.insert(dir.to_path_buf());
            // The watcher reports nothing under a build or dependency folder: what is kept of
            // it is read again each time it is opened.
            if crate::watch::ignored(&self.root, dir) {
                self.dirs.retain(|p, _| !p.starts_with(dir));
            }
        }
        self.rows = None;
    }

    pub fn collapse_all(&mut self) {
        self.expanded.clear();
        self.rows = None;
    }

    /// Forgets every folder read (they are read again when shown).
    pub fn refresh(&mut self) {
        self.dirs.clear();
        self.rows = None;
    }

    /// Paths changed on disk: the folders holding them (and the paths themselves, when they
    /// are folders that were read) are read again when shown next.
    pub fn invalidate(&mut self, changed: &[PathBuf]) {
        for path in changed {
            let mut hit = self.dirs.remove(path.as_path()).is_some();
            if let Some(parent) = path.parent() {
                hit |= self.dirs.remove(parent).is_some();
            }
            if hit {
                self.rows = None;
            }
        }
    }

    /// Opens the folders down to `file` and selects it.
    pub fn reveal(&mut self, file: &Path) {
        let Ok(rel) = file.strip_prefix(&self.root) else { return };
        let mut dir = self.root.clone();
        let mut parts = rel.components().peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                break;
            }
            dir.push(part);
            self.expanded.insert(dir.clone());
        }
        self.selected = Some(file.to_path_buf());
        self.rows = None;
    }

    /// Starts typing the name of a new file or folder in `dir` (opened if it is closed).
    pub fn start_new(&mut self, dir: &Path, folder: bool) {
        if dir != self.root {
            self.expanded.insert(dir.to_path_buf());
        }
        self.edit = Some(Edit {
            dir: dir.to_path_buf(),
            kind: if folder { EditKind::NewFolder } else { EditKind::NewFile },
            text: String::new(),
            focus: true,
        });
        self.rows = None;
    }

    /// Starts renaming `path` in place.
    pub fn start_rename(&mut self, path: &Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.edit = Some(Edit {
            dir: path.parent().unwrap_or(&self.root).to_path_buf(),
            kind: EditKind::Rename(path.to_path_buf()),
            text: name,
            focus: true,
        });
        self.rows = None;
    }

    pub fn cancel_edit(&mut self) {
        if self.edit.take().is_some() {
            self.rows = None;
        }
    }

    /// Creates or renames what the edit names. Returns the new path; on an error the edit stays
    /// open so the name can be fixed.
    pub fn commit_edit(&mut self) -> Result<Option<(PathBuf, Option<PathBuf>)>, String> {
        let Some(edit) = self.edit.clone() else { return Ok(None) };
        let name = edit.text.trim();
        if name.is_empty() {
            self.cancel_edit();
            return Ok(None);
        }
        let result = apply_edit(&edit.dir, &edit.kind, name)?;
        self.edit = None;
        // The folders that changed are read again; the new item is shown and selected.
        let mut changed = vec![result.clone()];
        if let EditKind::Rename(old) = &edit.kind {
            changed.push(old.clone());
        }
        self.invalidate(&changed);
        self.reveal(&result);
        self.rows = None;
        let old = match edit.kind {
            EditKind::Rename(old) => Some(old),
            _ => None,
        };
        Ok(Some((result, old)))
    }

    /// Whether `path` is among the rows drawn last (false until they are drawn again after a
    /// change).
    pub fn shows(&self, path: &Path) -> bool {
        self.rows
            .as_deref()
            .is_some_and(|rows| rows.iter().any(|r| matches!(r, Row::Item { entry, .. } if entry.path == path)))
    }

    /// The rows to draw: the root's entries, and those of every open folder under them.
    pub fn rows(&mut self) -> &[Row] {
        if self.rows.is_none() {
            let mut out = Vec::new();
            let root = self.root.clone();
            self.collect(&root, 0, &mut out);
            self.rows = Some(out);
        }
        self.rows.as_deref().unwrap_or_default()
    }

    fn collect(&mut self, dir: &Path, depth: usize, out: &mut Vec<Row>) {
        let listing = self.listing(dir).clone();
        if let Some(edit) = &self.edit
            && edit.dir == dir
            && !matches!(edit.kind, EditKind::Rename(_))
        {
            out.push(Row::New { depth, folder: edit.kind == EditKind::NewFolder });
        }
        for entry in listing.entries {
            let open = entry.is_dir && self.expanded.contains(&entry.path);
            let path = entry.path.clone();
            out.push(Row::Item { depth, entry, open });
            if open {
                self.collect(&path, depth + 1, out);
            }
        }
        if listing.more > 0 {
            out.push(Row::More { depth, count: listing.more });
        }
    }

    fn listing(&mut self, dir: &Path) -> &Listing {
        if !self.dirs.contains_key(dir) {
            self.reads += 1;
            let listing = read(&self.root, dir);
            self.dirs.insert(dir.to_path_buf(), listing);
        }
        &self.dirs[dir]
    }
}

/// Creates the file or folder `name` in `dir`, or renames `kind`'s target to `name`. A new
/// file or folder may be a path (`src/new.rs`): the folders on the way are made.
pub fn apply_edit(dir: &Path, kind: &EditKind, name: &str) -> Result<PathBuf, String> {
    use crate::i18n::{t, tf};
    use std::path::Component;
    // Plain names that stay in `dir`: no `.` or `..`, no root (`/x`) and, on Windows, no drive
    // or root (`C:\x`, `\x`) and no `:` (that names a stream of another file).
    let parts: Vec<Component> = Path::new(name).components().collect();
    let bad = parts.is_empty()
        || parts.iter().any(|c| !matches!(c, Component::Normal(_)))
        || name.contains('\0')
        || (cfg!(windows) && name.contains(':'));
    let nested = parts.len() > 1;
    if bad || (nested && matches!(kind, EditKind::Rename(_))) {
        return Err(tf("\"{}\" is not a valid name.", &[name]));
    }
    let target = dir.join(name);
    // `a.rs` → `A.rs` on a file system that ignores case finds the file itself: a rename.
    let same_file = match kind {
        EditKind::Rename(old) => {
            old.parent() == Some(dir)
                && old.file_name().is_some_and(|o| o != name && o.to_string_lossy().to_lowercase() == name.to_lowercase())
                && std::fs::read_dir(dir).is_ok_and(|mut d| !d.any(|e| e.is_ok_and(|e| e.file_name() == name)))
        }
        _ => false,
    };
    if target.symlink_metadata().is_ok() && !same_file {
        return Err(tf("{} already exists here.", &[name]));
    }
    let io = |e: std::io::Error| e.to_string();
    match kind {
        EditKind::NewFile => {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(io)?;
            }
            std::fs::OpenOptions::new().write(true).create_new(true).open(&target).map_err(io)?;
        }
        EditKind::NewFolder => std::fs::create_dir_all(&target).map_err(io)?,
        EditKind::Rename(old) => {
            if !old.exists() {
                return Err(t("It no longer exists.").to_string());
            }
            std::fs::rename(old, &target).map_err(io)?;
        }
    }
    Ok(target)
}

/// `name copy.ext`, `name copy 2.ext`, ...: a free name for a duplicate of `path`.
pub fn duplicate_name(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..)
        .map(|n| {
            let suffix = if n == 1 { " copy".to_string() } else { format!(" copy {n}") };
            dir.join(format!("{stem}{suffix}{ext}"))
        })
        .find(|p| p.symlink_metadata().is_err())
        .unwrap_or_else(|| dir.join(format!("{stem} copy{ext}")))
}

/// One folder, sorted: folders first, then names regardless of case.
fn read(root: &Path, dir: &Path) -> Listing {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Listing::default();
    };
    let mut entries: Vec<Entry> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if HIDDEN.contains(&name.as_str()) {
                return None;
            }
            let kind = e.file_type().ok()?;
            let path = e.path();
            let is_link = kind.is_symlink();
            let is_dir = if is_link { path.is_dir() } else { kind.is_dir() };
            let dimmed = crate::watch::ignored(root, &path);
            Some(Entry {
                name,
                path,
                is_dir,
                is_link,
                dimmed,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let more = entries.len().saturating_sub(MAX_ENTRIES);
    entries.truncate(MAX_ENTRIES);
    Listing { entries, more }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                Row::Item { depth, entry, .. } => format!("{}{}", "  ".repeat(*depth), entry.name),
                Row::More { depth, count } => format!("{}… {count}", "  ".repeat(*depth)),
                Row::New { depth, folder } => format!("{}<new {}>", "  ".repeat(*depth), if *folder { "folder" } else { "file" }),
            })
            .collect()
    }

    #[test]
    fn create_rename_and_duplicate_in_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "x").unwrap();
        let mut ex = Explorer::new(root.to_path_buf());
        ex.rows();
        // A new file in src: the field shows first in the opened folder.
        ex.start_new(&root.join("src"), false);
        assert_eq!(names(ex.rows()), ["src", "  <new file>", "  a.rs"]);
        ex.edit.as_mut().unwrap().text = "b.rs".into();
        let (made, old) = ex.commit_edit().unwrap().unwrap();
        assert_eq!((made.as_path(), old), (root.join("src/b.rs").as_path(), None));
        assert!(made.is_file());
        assert_eq!(names(ex.rows()), ["src", "  a.rs", "  b.rs"]);
        assert_eq!(ex.selected.as_deref(), Some(made.as_path()));
        // An existing name is refused and the field stays.
        ex.start_new(&root.join("src"), false);
        ex.edit.as_mut().unwrap().text = "a.rs".into();
        assert!(ex.commit_edit().is_err());
        assert!(ex.edit.is_some());
        ex.cancel_edit();
        // Nested folders in one go, at the root.
        ex.start_new(root, true);
        assert_eq!(names(ex.rows())[0], "<new folder>");
        ex.edit.as_mut().unwrap().text = "docs/api".into();
        ex.commit_edit().unwrap();
        assert!(root.join("docs/api").is_dir());
        // Rename in place.
        ex.start_rename(&root.join("src/a.rs"));
        assert_eq!(ex.edit.as_ref().unwrap().text, "a.rs");
        ex.edit.as_mut().unwrap().text = "main.rs".into();
        let (renamed, old) = ex.commit_edit().unwrap().unwrap();
        assert_eq!(renamed, root.join("src/main.rs"));
        assert_eq!(old, Some(root.join("src/a.rs")));
        assert!(names(ex.rows()).contains(&"  main.rs".to_string()));
        // Bad names, and a rename may not move the file elsewhere.
        for bad in ["..", ".", "a/../../x", "../x", "/abs", "./x", "a\0b"] {
            assert!(apply_edit(root, &EditKind::NewFile, bad).is_err(), "{bad}");
        }
        #[cfg(windows)]
        for bad in [r"a\..\..\x", r"C:\x", r"\x", "C:x", "a.rs:s"] {
            assert!(apply_edit(root, &EditKind::NewFile, bad).is_err(), "{bad}");
            assert!(apply_edit(root, &EditKind::Rename(renamed.clone()), bad).is_err(), "{bad}");
        }
        assert!(!root.parent().unwrap().join("x").exists());
        assert!(apply_edit(root, &EditKind::Rename(renamed.clone()), "x/y.rs").is_err());
        // Only the case changes: the same file, renamed (also where case is ignored).
        let src = root.join("src");
        let upper = apply_edit(&src, &EditKind::Rename(renamed.clone()), "Main.rs").unwrap();
        let listed: Vec<String> = std::fs::read_dir(&src).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(listed.contains(&"Main.rs".to_string()) && !listed.contains(&"main.rs".to_string()), "{listed:?}");
        apply_edit(&src, &EditKind::Rename(upper), "main.rs").unwrap();
        // Another file whose name differs only in case is still a clash where case is ignored.
        if root.join("SRC").exists() {
            assert!(apply_edit(&src, &EditKind::Rename(src.join("b.rs")), "MAIN.rs").is_err());
        }
        // Duplicates get free names.
        assert_eq!(duplicate_name(&root.join("src/main.rs")), root.join("src/main copy.rs"));
        std::fs::write(root.join("src/main copy.rs"), "").unwrap();
        assert_eq!(duplicate_name(&root.join("src/main.rs")), root.join("src/main copy 2.rs"));
    }

    #[test]
    fn folders_first_then_names_and_hidden_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for d in ["src", "Docs", ".git", "target"] {
            std::fs::create_dir(root.join(d)).unwrap();
        }
        for f in ["b.txt", "A.md", ".DS_Store", ".env", "src/main.rs", "target/x"] {
            std::fs::write(root.join(f), "x").unwrap();
        }
        let mut ex = Explorer::new(root.to_path_buf());
        assert_eq!(names(ex.rows()), ["Docs", "src", "target", ".env", "A.md", "b.txt"]);
        // Build folders are listed, dimmed; ordinary ones are not.
        let dimmed: Vec<_> = ex
            .rows()
            .iter()
            .filter_map(|r| match r {
                Row::Item { entry, .. } if entry.dimmed => Some(entry.name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(dimmed, ["target"]);
        ex.toggle(&root.join("src"));
        assert_eq!(names(ex.rows()), ["Docs", "src", "  main.rs", "target", ".env", "A.md", "b.txt"]);
        ex.collapse_all();
        assert_eq!(ex.rows().len(), 6);
        // The watcher says nothing about build folders: opening one again reads it again.
        ex.toggle(&root.join("target"));
        assert!(names(ex.rows()).contains(&"  x".to_string()));
        std::fs::write(root.join("target/y"), "y").unwrap();
        ex.toggle(&root.join("target"));
        ex.toggle(&root.join("target"));
        assert!(names(ex.rows()).contains(&"  y".to_string()));
    }

    #[test]
    fn drawing_reads_nothing_until_the_watcher_reports_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "x").unwrap();
        let mut ex = Explorer::new(root.to_path_buf());
        ex.toggle(&root.join("src"));
        ex.rows();
        let reads = ex.reads;
        assert_eq!(reads, 2);
        for _ in 0..100 {
            ex.rows();
        }
        assert_eq!(ex.reads, reads, "drawing again must not read");
        // An agent adds a file: only its folder is read again.
        std::fs::write(root.join("src/b.rs"), "x").unwrap();
        ex.invalidate(&[root.join("src/b.rs")]);
        assert_eq!(names(ex.rows()), ["src", "  a.rs", "  b.rs"]);
        assert_eq!(ex.reads, reads + 1);
        // A change somewhere never read changes nothing.
        ex.invalidate(&[root.join("elsewhere/c.rs")]);
        ex.rows();
        assert_eq!(ex.reads, reads + 1);
        // A deleted folder disappears.
        std::fs::remove_dir_all(root.join("src")).unwrap();
        ex.invalidate(&[root.join("src")]);
        assert!(ex.rows().is_empty());
    }

    #[test]
    fn long_folders_are_cut_and_reveal_opens_the_way() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let many = root.join("many");
        std::fs::create_dir(&many).unwrap();
        for i in 0..MAX_ENTRIES + 5 {
            std::fs::write(many.join(format!("f{i:05}")), "").unwrap();
        }
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/deep.rs"), "").unwrap();
        let mut ex = Explorer::new(root.to_path_buf());
        ex.toggle(&many);
        let rows = ex.rows();
        assert_eq!(rows.len(), 2 + MAX_ENTRIES + 1);
        assert_eq!(rows.last(), Some(&Row::More { depth: 1, count: 5 }));
        ex.toggle(&many);
        ex.reveal(&root.join("a/b/deep.rs"));
        assert_eq!(names(ex.rows()), ["a", "  b", "    deep.rs", "many"]);
        assert_eq!(ex.selected.as_deref(), Some(root.join("a/b/deep.rs").as_path()));
        assert!(ex.expanded.contains(&root.join("a")) && ex.expanded.contains(&root.join("a/b")));
    }
}
