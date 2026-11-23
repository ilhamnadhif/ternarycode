//! The first run's help: git repositories found in the usual folders (`~/code`,
//! `~/Developments`, …) to add as projects in one click, and which agents are installed (with
//! their versions). Looked for only while the welcome screen shows, once, on a thread of its own
//! (and again on "Look again"); nothing is read outside those folders, two levels deep.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use tc_core::agent::AgentKind;

use crate::project::Kind;

/// A git repository found.
#[derive(Clone, Debug)]
pub struct FoundRepo {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub branch: String,
    /// When something last happened in it (its HEAD's log, else its index).
    pub touched: Option<SystemTime>,
}

/// An agent, and whether its program is on the PATH (with its version).
#[derive(Clone, Debug)]
pub struct FoundAgent {
    pub agent: AgentKind,
    pub program: String,
    pub found: bool,
    pub version: Option<String>,
}

/// What the scan found.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub repos: Vec<FoundRepo>,
    pub agents: Vec<FoundAgent>,
    /// The folders looked in (shown as "~/code and ~/Developments").
    pub roots: Vec<PathBuf>,
}

/// The welcome screen's state: the scan running or done, and which repositories are ticked.
#[derive(Default)]
pub struct Onboarding {
    pub found: Option<Found>,
    pub picked: std::collections::HashSet<PathBuf>,
    rx: Option<mpsc::Receiver<Found>>,
    /// When the welcome screen first asked for a scan (it waits a moment for the shell's PATH).
    pub asked: Option<std::time::Instant>,
}

/// Folders under the home folder where projects usually live.
const ROOTS: &[&str] = &["code", "Code", "Developments", "Development", "Projects", "projects", "src", "dev", "work", "repos", "git", "GitHub", "Documents/GitHub"];
/// At most this many repositories are listed.
const MAX_REPOS: usize = 40;
/// Folders not looked into.
const SKIP: &[&str] = &["node_modules", "target", "vendor", "dist", "build", "Library", "Applications"];

impl Onboarding {
    /// Starts looking (again), off the UI thread. `agents`: each agent and its program;
    /// `path`: the PATH the panes get.
    pub fn scan(&mut self, ctx: &eframe::egui::Context, agents: Vec<(AgentKind, String)>, path: Option<std::ffi::OsString>) {
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("onboarding-scan".into())
            .spawn(move || {
                let home = crate::paths::home_dir();
                let found = Found {
                    repos: find_repos(&home),
                    agents: find_agents(agents, path.as_ref()),
                    roots: roots(&home),
                };
                let _ = tx.send(found);
                ctx.request_repaint();
            })
            .ok();
    }

    /// Whether a scan is under way.
    pub fn scanning(&self) -> bool {
        self.rx.is_some()
    }

    /// Takes what a scan found, when it is done. The repositories touched in the last two
    /// weeks are ticked (at most six).
    pub fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        if let Ok(found) = rx.try_recv() {
            let recent = SystemTime::now() - Duration::from_secs(14 * 86_400);
            self.picked = found.repos.iter().filter(|r| r.touched.is_some_and(|t| t >= recent)).take(6).map(|r| r.path.clone()).collect();
            self.found = Some(found);
            self.rx = None;
        }
    }
}

/// The usual folders there are, each once: on a file system that ignores case (macOS, Windows)
/// `~/code` and `~/Code` are one folder.
fn roots(home: &Path) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    ROOTS
        .iter()
        .map(|r| home.join(r))
        .filter(|p| p.is_dir())
        .filter(|p| seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())))
        .collect()
}

/// The repositories in the usual folders, one or two levels down, the most recently touched
/// first.
pub fn find_repos(home: &Path) -> Vec<FoundRepo> {
    let mut out: Vec<FoundRepo> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for root in roots(home) {
        for dir in children(&root) {
            if is_repo(&dir) {
                if seen.insert(dir.clone()) {
                    out.push(repo(&dir));
                }
                continue;
            }
            for sub in children(&dir) {
                if is_repo(&sub) && seen.insert(sub.clone()) {
                    out.push(repo(&sub));
                }
            }
        }
    }
    out.sort_by(|a, b| b.touched.cmp(&a.touched).then(a.name.cmp(&b.name)));
    out.truncate(MAX_REPOS);
    out
}

/// The folders in `dir` worth looking into (no hidden ones, no build output), at most 200.
fn children(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            !name.starts_with('.') && !SKIP.contains(&name.as_str())
        })
        .map(|e| e.path())
        .take(200)
        .collect()
}

/// A repository's own folder (not a worktree's, whose `.git` is a file).
fn is_repo(dir: &Path) -> bool {
    dir.join(".git").is_dir()
}

fn repo(dir: &Path) -> FoundRepo {
    let git = dir.join(".git");
    let head = std::fs::read_to_string(git.join("HEAD")).unwrap_or_default();
    let branch = match head.trim().strip_prefix("ref: refs/heads/") {
        Some(b) => b.to_string(),
        None => head.trim().chars().take(7).collect(),
    };
    let touched = [git.join("logs").join("HEAD"), git.join("index"), git.join("HEAD")]
        .iter()
        .find_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    FoundRepo {
        name: dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        path: dir.to_path_buf(),
        kind: Kind::detect(dir),
        branch,
        touched,
    }
}

/// Each agent's program on `path`, with what `--version` says (two seconds at most each).
fn find_agents(agents: Vec<(AgentKind, String)>, path: Option<&std::ffi::OsString>) -> Vec<FoundAgent> {
    agents
        .into_iter()
        .map(|(agent, program)| {
            let exe = path.and_then(|p| tc_lsp::servers::resolve(&program, p));
            let version = exe.as_ref().and_then(|e| version_of(e));
            FoundAgent { agent, program, found: exe.is_some(), version }
        })
        .collect()
}

/// The first version-looking word `exe --version` prints ("2.1.319").
fn version_of(exe: &Path) -> Option<String> {
    let mut child = std::process::Command::new(exe)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(30)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    parse_version(&out)
}

/// The first word made of digits and dots with at least one dot ("claude 2.1.319 (Claude
/// Code)" → "2.1.319").
pub fn parse_version(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
        .map(|w| w.trim_start_matches('v'))
        .find(|w| w.contains('.') && w.chars().all(|c| c.is_ascii_digit() || c == '.') && w.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_from_what_programs_print() {
        assert_eq!(parse_version("2.1.319 (Claude Code)\n").as_deref(), Some("2.1.319"));
        assert_eq!(parse_version("codex-cli 0.98.0").as_deref(), Some("0.98.0"));
        assert_eq!(parse_version("v1.4.2").as_deref(), Some("1.4.2"));
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn repositories_are_found_two_levels_down() {
        let home = tempfile::tempdir().unwrap();
        let code = home.path().join("code");
        for repo in ["code/shop", "code/clients/acme"] {
            let git = home.path().join(repo).join(".git");
            std::fs::create_dir_all(&git).unwrap();
            std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        }
        // Too deep, hidden, and build output: not found.
        std::fs::create_dir_all(code.join("a/b/c/.git")).unwrap();
        std::fs::create_dir_all(code.join(".hidden/.git")).unwrap();
        std::fs::create_dir_all(code.join("node_modules/x/.git")).unwrap();
        let mut names: Vec<String> = find_repos(home.path()).into_iter().map(|r| r.name).collect();
        names.sort();
        assert_eq!(names, ["acme", "shop"]);
        assert_eq!(find_repos(home.path())[0].branch, "main");
    }
}
