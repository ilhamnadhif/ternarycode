//! The earlier sessions of an agent in a project, read from the agent's own files, so that Run
//! Agent can continue any of them (not only the last). Nothing is written. Formats checked on
//! 13 Nov 2026 against Claude Code 2.1.323, Codex 0.166 and Gemini CLI 0.43.0:
//!
//! - Claude Code: `<config>/projects/<project path, every other character than a letter or a
//!   digit made '-'>/<session id>.jsonl`. Its title is a `custom-title` line (`/rename`), else
//!   an `ai-title` line, else the first prompt; `gitBranch` is on every message. The files grow
//!   to tens of MB: only their start and end are read.
//! - Codex: `<home>/sessions/YYYY/MM/DD/rollout-….jsonl`, whose first line (`session_meta`)
//!   has the id and the folder it ran in; titles in `<home>/session_index.jsonl`.
//! - Gemini CLI: `~/.gemini/tmp/<name>/chats/session-….jsonl`, `<name>` the project's entry in
//!   `~/.gemini/projects.json` (or the folder whose `.project_root` names it); the first line
//!   has the session id.
//!
//! OpenCode keeps its sessions in a database: it is not listed.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;
use tc_core::agent::AgentKind;

/// One earlier session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// What the agent's resume option takes.
    pub id: String,
    /// Its title, or its first prompt (one line).
    pub title: String,
    /// When it last changed.
    pub updated: SystemTime,
    pub branch: Option<String>,
}

/// Where each agent keeps its files.
#[derive(Clone, Debug)]
pub struct Roots {
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub gemini: PathBuf,
}

impl Roots {
    /// The agents' defaults under `home`, or where their own variables move them.
    pub fn from_env(home: &Path) -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        Self {
            claude: var("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
            codex: var("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
            gemini: home.join(".gemini"),
        }
    }

    /// The defaults under `home` only (tests).
    pub fn under(home: &Path) -> Self {
        Self {
            claude: home.join(".claude"),
            codex: home.join(".codex"),
            gemini: home.join(".gemini"),
        }
    }
}

/// Whether the agent's sessions can be listed (and resumed by id).
pub fn listable(agent: AgentKind) -> bool {
    matches!(agent, AgentKind::Claude | AgentKind::Codex | AgentKind::Gemini)
}

/// The command that continues session `id`: the preset's own command (its flags kept) with the
/// agent's option for it. `None` for other agents, or an id that is not plainly one.
pub fn resume_command(agent: AgentKind, command: &str, id: &str) -> Option<String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let command = command.trim();
    match agent {
        AgentKind::Claude | AgentKind::Gemini => Some(format!("{command} --resume {id}")),
        AgentKind::Codex => Some(format!("{command} resume {id}")),
        _ => None,
    }
}

/// The option that starts the agent without asking before each action (`None` when it has
/// none). Checked: Claude Code 2.1.323, Codex 0.166, Gemini CLI 0.43.0.
pub fn skip_permissions_flag(agent: AgentKind) -> Option<&'static str> {
    match agent {
        AgentKind::Claude => Some("--dangerously-skip-permissions"),
        AgentKind::Codex => Some("--dangerously-bypass-approvals-and-sandbox"),
        AgentKind::Gemini => Some("--yolo"),
        _ => None,
    }
}

/// The agent's sessions in `project`, newest first, at most `limit`.
pub fn list(agent: AgentKind, project: &Path, roots: &Roots, limit: usize) -> Vec<Session> {
    let mut out = match agent {
        AgentKind::Claude => claude(project, &roots.claude, limit),
        AgentKind::Codex => codex(project, &roots.codex, limit),
        AgentKind::Gemini => gemini(project, &roots.gemini, limit),
        _ => Vec::new(),
    };
    out.sort_by_key(|s| std::cmp::Reverse(s.updated));
    // One row per session (Codex can keep a session in more than one file): its newest.
    let mut seen = std::collections::HashSet::new();
    out.retain(|s| seen.insert(s.id.clone()));
    out.truncate(limit);
    out
}

// ----- reading --------------------------------------------------------------------------------

/// The complete lines in the first `head` and the last `tail` bytes of a file (a line cut at
/// either edge is left out). The two overlap when the file is small.
fn edges(path: &Path, head: u64, tail: u64) -> (Vec<String>, Vec<String>) {
    let Ok(mut f) = File::open(path) else {
        return (Vec::new(), Vec::new());
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut first = Vec::new();
    let _ = (&mut f).take(head).read_to_end(&mut first);
    let whole_head = len <= head;
    let mut last = Vec::new();
    if !whole_head {
        let from = len.saturating_sub(tail);
        if f.seek(SeekFrom::Start(from)).is_ok() {
            let _ = f.read_to_end(&mut last);
        }
        // The first line of the tail may be cut.
        if from > 0 {
            match last.iter().position(|b| *b == b'\n') {
                Some(i) => last.drain(..=i),
                None => last.drain(..),
            };
        }
    }
    let lines = |bytes: &[u8], keep_last: bool| -> Vec<String> {
        let text = String::from_utf8_lossy(bytes);
        let mut v: Vec<String> = text.split('\n').map(str::to_string).collect();
        // The last piece is cut unless the read reached the end.
        if !keep_last {
            v.pop();
        }
        v.retain(|l| !l.trim().is_empty());
        v
    };
    (lines(&first, whole_head), lines(&last, true))
}

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

fn text(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string).filter(|s| !s.trim().is_empty())
}

/// One line, short.
fn one_line(s: &str) -> String {
    let line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= 120 {
        return line;
    }
    let mut out: String = line.chars().take(119).collect();
    out.push('…');
    out
}

/// The text of a message's content: a string, or blocks with `text`.
fn content_text(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(blocks) => blocks.iter().find_map(|b| text(&b["text"])),
        _ => None,
    }
}

/// A prompt the user typed (not a command, a hook's or the tool's own text).
fn typed_prompt(s: &str) -> bool {
    let s = s.trim_start();
    !s.is_empty() && !s.starts_with('<') && !s.starts_with('/') && !s.starts_with("Caveat:") && !s.starts_with('[')
}

// ----- Claude Code ----------------------------------------------------------------------------

/// The folder name Claude Code keeps a project's sessions under.
pub fn claude_dir_name(project: &Path) -> String {
    project.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn claude(project: &Path, root: &Path, limit: usize) -> Vec<Session> {
    // The folder as given and as it really is (Claude Code names it after the real path; a
    // project added through a link has both).
    let mut dirs = vec![root.join("projects").join(claude_dir_name(project))];
    if let Ok(real) = fs::canonicalize(project) {
        let dir = root.join("projects").join(claude_dir_name(&real));
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    // Newest first: only those are read.
    let mut files: Vec<(SystemTime, PathBuf)> = dirs
        .iter()
        .filter_map(|d| fs::read_dir(d).ok())
        .flat_map(|r| r.flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .filter(|p| p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.len() == 36 && s.matches('-').count() == 4))
        .map(|p| (modified(&p), p))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.into_iter().filter_map(|(updated, path)| claude_session(&path, updated)).take(limit).collect()
}

fn claude_session(path: &Path, updated: SystemTime) -> Option<Session> {
    let id = path.file_stem()?.to_str()?.to_string();
    let (head, tail) = edges(path, 64 * 1024, 512 * 1024);
    let (mut custom, mut ai, mut first, mut last_prompt, mut branch) = (None, None, None, None, None);
    for (from_tail, line) in head.iter().map(|l| (false, l)).chain(tail.iter().map(|l| (true, l))) {
        // Only the kinds of lines that matter are parsed (others can be very long).
        let kind = ["\"type\":\"custom-title\"", "\"type\":\"ai-title\"", "\"type\":\"last-prompt\"", "\"type\":\"user\""]
            .iter()
            .position(|k| line.contains(k));
        let Some(kind) = kind else { continue };
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match kind {
            0 => custom = text(&v["customTitle"]).or(custom),
            1 => ai = text(&v["aiTitle"]).or(ai),
            2 => last_prompt = text(&v["lastPrompt"]).or(last_prompt),
            _ => {
                if v["isSidechain"].as_bool() == Some(true) {
                    continue;
                }
                if let Some(b) = text(&v["gitBranch"]) {
                    branch = Some(b);
                }
                if !from_tail && first.is_none() && v["isMeta"].as_bool() != Some(true) {
                    first = content_text(&v["message"]["content"]).filter(|s| typed_prompt(s));
                }
            }
        }
    }
    let title = custom.or(ai).or(first).or(last_prompt)?;
    Some(Session { id, title: one_line(&title), updated, branch })
}

// ----- Codex ----------------------------------------------------------------------------------

/// The folders under `dir` (or its files), newest name first.
fn names_desc(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    v.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    v
}

fn same_folder(a: &Path, b: &Path) -> bool {
    a == b || (fs::canonicalize(a).ok().is_some_and(|x| fs::canonicalize(b).ok().is_some_and(|y| x == y)))
}

fn codex(project: &Path, root: &Path, limit: usize) -> Vec<Session> {
    // Titles: the index, the last entry for an id winning.
    let mut titles: HashMap<String, String> = HashMap::new();
    if let Ok(f) = File::open(root.join("session_index.jsonl")) {
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&line)
                && let (Some(id), Some(name)) = (text(&v["id"]), text(&v["thread_name"]))
            {
                titles.insert(id, name);
            }
        }
    }
    let mut out = Vec::new();
    let mut looked = 0;
    'days: for year in names_desc(&root.join("sessions")) {
        for month in names_desc(&year) {
            for day in names_desc(&month) {
                for file in names_desc(&day) {
                    if file.extension().is_none_or(|x| x != "jsonl") {
                        continue;
                    }
                    looked += 1;
                    if looked > 2000 || out.len() >= limit {
                        break 'days;
                    }
                    if let Some(s) = codex_session(&file, project, &titles) {
                        out.push(s);
                    }
                }
            }
        }
    }
    out
}

fn codex_session(path: &Path, project: &Path, titles: &HashMap<String, String>) -> Option<Session> {
    let f = File::open(path).ok()?;
    let mut lines = BufReader::new(f).lines().map_while(Result::ok);
    let meta: Value = serde_json::from_str(&lines.next()?).ok()?;
    let payload = &meta["payload"];
    let cwd = text(&payload["cwd"])?;
    if !same_folder(Path::new(&cwd), project) {
        return None;
    }
    let id = text(&payload["id"]).or_else(|| text(&payload["session_id"]))?;
    let mut branch = text(&payload["git"]["branch"]);
    let title = match titles.get(&id) {
        Some(t) => Some(t.clone()),
        // Else its first prompt, near the start.
        None => lines.take(400).find_map(|l| {
            if !l.contains("\"user_message\"") {
                return None;
            }
            let v: Value = serde_json::from_str(&l).ok()?;
            if branch.is_none() {
                branch = text(&v["payload"]["git"]["branch"]);
            }
            text(&v["payload"]["message"]).filter(|s| typed_prompt(s))
        }),
    }?;
    Some(Session { id, title: one_line(&title), updated: modified(path), branch })
}

// ----- Gemini CLI -----------------------------------------------------------------------------

/// The folder under `<root>/tmp` that holds the project's chats.
fn gemini_dir(project: &Path, root: &Path) -> Option<PathBuf> {
    let tmp = root.join("tmp");
    if let Ok(text) = fs::read_to_string(root.join("projects.json"))
        && let Ok(v) = serde_json::from_str::<Value>(&text)
        && let Some(map) = v["projects"].as_object()
    {
        for (path, name) in map {
            if let Some(name) = name.as_str()
                && same_folder(Path::new(path), project)
            {
                return Some(tmp.join(name));
            }
        }
    }
    names_desc(&tmp).into_iter().find(|d| {
        fs::read_to_string(d.join(".project_root")).is_ok_and(|p| same_folder(Path::new(p.trim()), project))
    })
}

fn gemini(project: &Path, root: &Path, limit: usize) -> Vec<Session> {
    let Some(dir) = gemini_dir(project, root) else {
        return Vec::new();
    };
    let mut files: Vec<(SystemTime, PathBuf)> = names_desc(&dir.join("chats"))
        .into_iter()
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .map(|p| (modified(&p), p))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files
        .into_iter()
        .filter_map(|(updated, path)| {
            let (head, _) = edges(&path, 256 * 1024, 0);
            let meta: Value = serde_json::from_str(head.first()?).ok()?;
            let id = text(&meta["sessionId"])?;
            let title = head.iter().skip(1).find_map(|l| {
                if !l.contains("\"type\":\"user\"") {
                    return None;
                }
                let v: Value = serde_json::from_str(l).ok()?;
                content_text(&v["content"]).filter(|s| typed_prompt(s) && !s.trim_start().starts_with('?'))
            })?;
            Some(Session { id, title: one_line(&title), updated, branch: None })
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn age(path: &Path, secs: u64) {
        let t = SystemTime::now() - Duration::from_secs(secs);
        File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn resume_commands_keep_the_preset_flags() {
        let id = "d30064f0-e749-483a-b48c-6f7ecaa61ffc";
        assert_eq!(resume_command(AgentKind::Claude, "claude --model opus ", id).unwrap(), format!("claude --model opus --resume {id}"));
        assert_eq!(resume_command(AgentKind::Codex, "codex", "01a1"), Some("codex resume 01a1".into()));
        assert_eq!(resume_command(AgentKind::Gemini, "gemini", "f23b"), Some("gemini --resume f23b".into()));
        assert_eq!(resume_command(AgentKind::OpenCode, "opencode", "x"), None);
        assert_eq!(resume_command(AgentKind::Claude, "claude", "x; rm -rf ~"), None);
    }

    #[test]
    fn claude_folder_names() {
        assert_eq!(claude_dir_name(Path::new("/Users/me/Developments/RUST/ternarycode")), "-Users-me-Developments-RUST-ternarycode");
        assert_eq!(claude_dir_name(Path::new("/a/my_app.v2 x")), "-a-my-app-v2-x");
    }

    #[test]
    fn claude_sessions_newest_first_with_their_titles() {
        let home = tempfile::tempdir().unwrap();
        let project = Path::new("/work/shop");
        let dir = home.path().join(".claude/projects/-work-shop");
        let a = dir.join("11111111-1111-1111-1111-111111111111.jsonl");
        write(
            &a,
            concat!(
                "{\"type\":\"user\",\"isMeta\":true,\"message\":{\"content\":\"<command-name>/init</command-name>\"},\"gitBranch\":\"main\"}\n",
                "{\"type\":\"user\",\"message\":{\"content\":\"fix the   checkout\\nbug\"},\"gitBranch\":\"main\"}\n",
                "{\"type\":\"ai-title\",\"aiTitle\":\"Checkout bug\"}\n",
                "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"more\"}]},\"gitBranch\":\"fix/checkout\"}\n",
            ),
        );
        let b = dir.join("22222222-2222-2222-2222-222222222222.jsonl");
        write(
            &b,
            concat!(
                "{\"type\":\"user\",\"message\":{\"content\":\"add search\"},\"gitBranch\":\"main\"}\n",
                "{\"type\":\"ai-title\",\"aiTitle\":\"Search\"}\n",
                "{\"type\":\"custom-title\",\"customTitle\":\"My search work\"}\n",
            ),
        );
        // No conversation, and not a session file.
        write(&dir.join("33333333-3333-3333-3333-333333333333.jsonl"), "{\"type\":\"mode\",\"mode\":\"normal\"}\n");
        write(&dir.join("notes.jsonl"), "{\"type\":\"user\",\"message\":{\"content\":\"x\"}}\n");
        age(&a, 10);
        age(&b, 100);
        let got = list(AgentKind::Claude, project, &Roots::under(home.path()), 20);
        let short: Vec<(&str, &str, Option<&str>)> = got.iter().map(|s| (&s.id[..8], s.title.as_str(), s.branch.as_deref())).collect();
        assert_eq!(short, [("11111111", "Checkout bug", Some("fix/checkout")), ("22222222", "My search work", Some("main"))]);
        // A session without titles is named by its first prompt.
        let c = dir.join("44444444-4444-4444-4444-444444444444.jsonl");
        write(&c, "{\"type\":\"user\",\"message\":{\"content\":\"rename the   API\"}}\n");
        let got = list(AgentKind::Claude, project, &Roots::under(home.path()), 1);
        assert_eq!((got.len(), got[0].title.as_str()), (1, "rename the API"));
    }

    #[test]
    fn claude_reads_the_end_of_long_sessions() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".claude/projects/-p/55555555-5555-5555-5555-555555555555.jsonl");
        let mut text = String::from("{\"type\":\"user\",\"message\":{\"content\":\"start\"},\"gitBranch\":\"main\"}\n");
        let filler = format!("{{\"type\":\"assistant\",\"x\":\"{}\"}}\n", "y".repeat(1000));
        for _ in 0..2000 {
            text.push_str(&filler);
        }
        text.push_str("{\"type\":\"ai-title\",\"aiTitle\":\"Late title\"}\n");
        write(&path, &text);
        let got = list(AgentKind::Claude, Path::new("/p"), &Roots::under(home.path()), 5);
        assert_eq!(got[0].title, "Late title");
    }

    #[test]
    fn codex_sessions_of_the_project_only() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".codex");
        let meta = |id: &str, cwd: &str| {
            format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\",\"git\":{{\"branch\":\"dev\"}}}}}}\n")
        };
        write(&root.join("sessions/2026/11/02/rollout-a.jsonl"), &(meta("aaa", "/work/shop") + "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"add tests\"}}\n"));
        write(&root.join("sessions/2026/11/05/rollout-b.jsonl"), &meta("bbb", "/work/shop"));
        write(&root.join("sessions/2026/11/06/rollout-c.jsonl"), &meta("ccc", "/elsewhere"));
        // The same session in a second file.
        write(&root.join("sessions/2026/11/01/rollout-b2.jsonl"), &meta("bbb", "/work/shop"));
        age(&root.join("sessions/2026/11/01/rollout-b2.jsonl"), 900);
        write(&root.join("session_index.jsonl"), "{\"id\":\"bbb\",\"thread_name\":\"Old name\"}\n{\"id\":\"bbb\",\"thread_name\":\"Deploy\"}\n");
        age(&root.join("sessions/2026/11/02/rollout-a.jsonl"), 500);
        let got = list(AgentKind::Codex, Path::new("/work/shop"), &Roots::under(home.path()), 20);
        let short: Vec<(&str, &str, Option<&str>)> = got.iter().map(|s| (s.id.as_str(), s.title.as_str(), s.branch.as_deref())).collect();
        assert_eq!(short, [("bbb", "Deploy", Some("dev")), ("aaa", "add tests", Some("dev"))]);
    }

    #[test]
    fn gemini_sessions_through_its_project_map() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".gemini");
        write(&root.join("projects.json"), "{\"projects\":{\"/work/shop\":\"shop\"}}");
        write(
            &root.join("tmp/shop/chats/session-2026-05-12T10-55-f23b3f01.jsonl"),
            concat!(
                "{\"sessionId\":\"f23b3f01-abed\",\"kind\":\"main\"}\n",
                "{\"type\":\"user\",\"content\":[{\"text\":\"/init\"}]}\n",
                "{\"type\":\"user\",\"content\":[{\"text\":\"explain the cart\"}]}\n",
            ),
        );
        let got = list(AgentKind::Gemini, Path::new("/work/shop"), &Roots::under(home.path()), 20);
        assert_eq!(got.iter().map(|s| (s.id.as_str(), s.title.as_str())).collect::<Vec<_>>(), [("f23b3f01-abed", "explain the cart")]);
        // Without the map, the folder that names the project.
        fs::remove_file(root.join("projects.json")).unwrap();
        write(&root.join("tmp/shop/.project_root"), "/work/shop");
        assert_eq!(list(AgentKind::Gemini, Path::new("/work/shop"), &Roots::under(home.path()), 20).len(), 1);
        assert!(list(AgentKind::OpenCode, Path::new("/work/shop"), &Roots::under(home.path()), 20).is_empty());
    }
}
