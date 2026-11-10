//! The language servers TernaryCode knows: which files they serve, how they start, where a
//! project's root is for them, and how to install them (the user does; the app shows how).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::time::Duration;

#[derive(Debug)]
pub struct Spec {
    /// Shown in the editor's status line.
    pub name: &'static str,
    pub program: &'static str,
    pub args: &'static [&'static str],
    /// File extensions and the LSP language id of each.
    pub languages: &'static [(&'static str, &'static str)],
    /// Files that mark a root (a crate, a Go module, a package).
    pub root_markers: &'static [&'static str],
    /// Take the outermost root under the project (a Cargo workspace), not the nearest.
    pub outermost: bool,
    /// The command that installs it.
    pub install: &'static str,
}

pub const SERVERS: &[Spec] = &[
    Spec {
        name: "rust-analyzer",
        program: "rust-analyzer",
        args: &[],
        languages: &[("rs", "rust")],
        root_markers: &["Cargo.toml"],
        outermost: true,
        install: "rustup component add rust-analyzer",
    },
    Spec {
        name: "gopls",
        program: "gopls",
        args: &[],
        languages: &[("go", "go")],
        root_markers: &["go.work", "go.mod"],
        outermost: false,
        install: "go install golang.org/x/tools/gopls@latest",
    },
    Spec {
        name: "typescript-language-server",
        program: "typescript-language-server",
        args: &["--stdio"],
        languages: &[
            ("ts", "typescript"),
            ("mts", "typescript"),
            ("cts", "typescript"),
            ("tsx", "typescriptreact"),
            ("js", "javascript"),
            ("mjs", "javascript"),
            ("cjs", "javascript"),
            ("jsx", "javascriptreact"),
        ],
        root_markers: &["tsconfig.json", "jsconfig.json", "package.json"],
        outermost: false,
        install: "npm install -g typescript-language-server typescript",
    },
    Spec {
        name: "dart",
        program: "dart",
        args: &["language-server", "--protocol=lsp"],
        languages: &[("dart", "dart")],
        root_markers: &["pubspec.yaml"],
        outermost: false,
        install: "https://dart.dev/get-dart (or the Flutter SDK)",
    },
    Spec {
        name: "pyright",
        program: "pyright-langserver",
        args: &["--stdio"],
        languages: &[("py", "python"), ("pyi", "python")],
        root_markers: &["pyproject.toml", "setup.py", "requirements.txt"],
        outermost: false,
        install: "npm install -g pyright",
    },
    Spec {
        name: "clangd",
        program: "clangd",
        args: &[],
        languages: &[
            ("c", "c"),
            ("h", "c"),
            ("cc", "cpp"),
            ("cpp", "cpp"),
            ("cxx", "cpp"),
            ("hpp", "cpp"),
            ("hh", "cpp"),
            ("m", "objective-c"),
            ("mm", "objective-cpp"),
        ],
        root_markers: &["compile_commands.json", "CMakeLists.txt", ".clangd"],
        outermost: false,
        install: "xcode-select --install (macOS) or your package manager's clangd",
    },
    Spec {
        name: "sourcekit-lsp",
        program: "sourcekit-lsp",
        args: &[],
        languages: &[("swift", "swift")],
        root_markers: &["Package.swift"],
        outermost: false,
        install: "xcode-select --install",
    },
];

/// The server for a file and the file's language id.
pub fn for_path(path: &Path) -> Option<(&'static Spec, &'static str)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    SERVERS.iter().find_map(|s| {
        s.languages
            .iter()
            .find(|(e, _)| *e == ext)
            .map(|(_, lang)| (s, *lang))
    })
}

/// The folder a server for `file` runs in: the nearest folder up from it (the outermost, for
/// a workspace-wide server) holding one of the spec's markers, not above `project`; else
/// `project`.
pub fn find_root(spec: &Spec, file: &Path, project: &Path) -> PathBuf {
    let mut found: Option<&Path> = None;
    let mut dir = file.parent();
    while let Some(d) = dir {
        if !d.starts_with(project) {
            break;
        }
        if spec.root_markers.iter().any(|m| d.join(m).exists()) {
            found = Some(d);
            if !spec.outermost {
                break;
            }
        }
        if d == project {
            break;
        }
        dir = d.parent();
    }
    found.unwrap_or(project).to_path_buf()
}

/// `program` found on `path` (or as given, when it is a path). On Windows also with the
/// extensions programs have there (`gopls.exe`, and `.cmd` for the launchers npm installs).
pub fn resolve(program: &str, path: &OsString) -> Option<PathBuf> {
    let suffixes: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    let with = |base: PathBuf| {
        suffixes.iter().find_map(|s| {
            let mut p = base.clone().into_os_string();
            p.push(s);
            let p = PathBuf::from(p);
            is_executable(&p).then_some(p)
        })
    };
    if program.contains('/') || (cfg!(windows) && program.contains('\\')) {
        return with(PathBuf::from(program));
    }
    std::env::split_paths(path).find_map(|d| with(d.join(program)))
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// The user's `PATH` as their login shell sets it, plus the folders tools usually install to.
/// An app opened from Finder or the Dock inherits only the system's. Blocking (it runs the
/// shell once, at most a few seconds): call it off the UI thread.
pub fn user_path() -> OsString {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(p) = login_shell_path() {
        dirs.extend(std::env::split_paths(&p));
    }
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    for extra in [
        home.join(".cargo/bin"),
        home.join("go/bin"),
        home.join(".local/bin"),
        home.join(".bun/bin"),
        home.join(".deno/bin"),
    ] {
        dirs.push(extra);
    }
    if cfg!(windows) {
        // npm's global programs.
        dirs.extend(std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("npm")));
    } else {
        for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/local/go/bin"] {
            dirs.push(PathBuf::from(extra));
        }
        dirs.push(home.join(".npm-global/bin"));
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
    std::env::join_paths(dirs).unwrap_or_default()
}

/// Windows: programs see the user's PATH already (it is in the registry, not a shell's files).
#[cfg(windows)]
fn login_shell_path() -> Option<OsString> {
    None
}

/// `PATH` printed by the user's interactive login shell (it reads their startup files), between
/// markers so whatever those print around it is ignored.
#[cfg(unix)]
fn login_shell_path() -> Option<OsString> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut child = Command::new(shell)
        .args(["-ilc", "printf '\\n__TC_PATH__%s__TC_END__\\n' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Read on a thread: a background job started by a startup file can keep the output open
    // after the shell is gone, and reading would then never end.
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut out = String::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.push_str(&String::from_utf8_lossy(&buf[..n]));
            if out.contains("__TC_END__") {
                break;
            }
        }
        let _ = tx.send(out);
    });
    let out = rx.recv_timeout(Duration::from_secs(4));
    if !matches!(child.try_wait(), Ok(Some(_))) {
        let _ = child.kill();
    }
    let _ = child.wait();
    let out = out.ok()?;
    let start = out.find("__TC_PATH__")? + "__TC_PATH__".len();
    let end = out[start..].find("__TC_END__")? + start;
    Some(OsString::from(&out[start..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_find_their_server_and_root() {
        let (s, lang) = for_path(Path::new("/x/src/main.rs")).unwrap();
        assert_eq!((s.name, lang), ("rust-analyzer", "rust"));
        assert_eq!(for_path(Path::new("a/App.TSX")).unwrap().1, "typescriptreact");
        assert_eq!(for_path(Path::new("main.go")).unwrap().0.name, "gopls");
        assert!(for_path(Path::new("notes.txt")).is_none());

        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().canonicalize().unwrap();
        // Windows: without the `\\?\` prefix, under which `/` in joined paths is no separator.
        #[cfg(windows)]
        let project = PathBuf::from(project.to_string_lossy().trim_start_matches(r"\\?\"));
        std::fs::create_dir_all(project.join("crates/a/src")).unwrap();
        std::fs::write(project.join("Cargo.toml"), "").unwrap();
        std::fs::write(project.join("crates/a/Cargo.toml"), "").unwrap();
        std::fs::create_dir_all(project.join("web/src")).unwrap();
        std::fs::write(project.join("web/package.json"), "{}").unwrap();
        // A Cargo workspace: its top, not the member crate.
        let rust = for_path(Path::new("x.rs")).unwrap().0;
        assert_eq!(find_root(rust, &project.join("crates/a/src/lib.rs"), &project), project);
        // A package in a sub-folder: that folder.
        let ts = for_path(Path::new("x.ts")).unwrap().0;
        assert_eq!(find_root(ts, &project.join("web/src/app.ts"), &project), project.join("web"));
        // No marker: the project.
        let go = for_path(Path::new("x.go")).unwrap().0;
        assert_eq!(find_root(go, &project.join("web/src/x.go"), &project), project);
    }

    #[cfg(windows)]
    #[test]
    fn programs_are_found_on_a_path() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let path = OsString::from(format!(r"C:\nowhere;{root}\System32"));
        let cmd = resolve("cmd", &path).expect("cmd.exe");
        assert!(cmd.to_string_lossy().to_ascii_lowercase().ends_with("cmd.exe"));
        assert_eq!(resolve("no-such-program-tc", &path), None);
    }

    #[cfg(unix)]
    #[test]
    fn programs_are_found_on_a_path() {
        let path = OsString::from("/nowhere:/bin:/usr/bin");
        assert_eq!(resolve("sh", &path), Some(PathBuf::from("/bin/sh")));
        assert_eq!(resolve("no-such-program-tc", &path), None);
    }
}
