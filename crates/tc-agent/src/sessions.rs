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
    pub antigravity: PathBuf,
}

impl Roots {
    /// The agents' defaults under `home`, or where their own variables move them.
    pub fn from_env(home: &Path) -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let gemini = var("GEMINI_CONFIG_DIR")
            .or_else(|| var("GEMINI_CLI_HOME").map(|h| h.join(".gemini")))
            .unwrap_or_else(|| home.join(".gemini"));
        let antigravity = var("AGY_DATA_DIR")
            .unwrap_or_else(|| home.join(".gemini").join("antigravity-cli"));
        Self {
            claude: var("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
            codex: var("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
            gemini,
            antigravity,
        }
    }

    /// The defaults under `home` only (tests).
    pub fn under(home: &Path) -> Self {
        Self {
            claude: home.join(".claude"),
            codex: home.join(".codex"),
            gemini: home.join(".gemini"),
            antigravity: home.join(".gemini").join("antigravity-cli"),
        }
    }
}

/// Whether the agent's sessions can be listed (and resumed by id).
pub fn listable(agent: AgentKind) -> bool {
    matches!(agent, AgentKind::Claude | AgentKind::Codex | AgentKind::Gemini | AgentKind::Antigravity)
}

pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id.len() <= 128
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The command that continues session `id`: the preset's own command (its flags kept) with the
/// agent's option for it. `None` for other agents, or an id that is not plainly one.
pub fn resume_command(agent: AgentKind, command: &str, id: &str) -> Option<String> {
    if !valid_session_id(id) {
        return None;
    }
    let command = command.trim();
    match agent {
        AgentKind::Claude | AgentKind::Gemini => Some(format!("{command} --resume {id}")),
        AgentKind::Codex => Some(format!("{command} resume {id}")),
        AgentKind::Antigravity => Some(format!("{command} --conversation {id}")),
        _ => None,
    }
}

/// The option that starts the agent without asking before each action (`None` when it has
/// none). Checked: Claude Code 2.1.323, Codex 0.166, Gemini CLI 0.43.0; Antigravity CLI from its
/// documentation (17 Nov 2026).
pub fn skip_permissions_flag(agent: AgentKind) -> Option<&'static str> {
    match agent {
        AgentKind::Claude => Some("--dangerously-skip-permissions"),
        AgentKind::Codex => Some("--dangerously-bypass-approvals-and-sandbox"),
        AgentKind::Gemini => Some("--yolo"),
        AgentKind::Antigravity => Some("--dangerously-skip-permissions"),
        _ => None,
    }
}

/// The agent's sessions in `project`, newest first, at most `limit`.
pub fn list(agent: AgentKind, project: &Path, roots: &Roots, limit: usize) -> Vec<Session> {
    let mut out = match agent {
        AgentKind::Claude => claude(project, &roots.claude, limit),
        AgentKind::Codex => codex(project, &roots.codex, limit),
        AgentKind::Gemini => gemini(project, &roots.gemini, limit),
        AgentKind::Antigravity => antigravity(project, &roots.antigravity, limit),
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
        // Never overlap the already-read `head` slice (prevents double-counting in usage scans).
        let from = len.saturating_sub(tail).max(head);
        if from < len && f.seek(SeekFrom::Start(from)).is_ok() {
            let _ = f.read_to_end(&mut last);
        }
        // The first line of the tail may be cut.
        if from > 0 && !last.is_empty() {
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
    same_folder_canon(a, b, fs::canonicalize(b).ok().as_deref())
}

fn same_folder_canon(a: &Path, b: &Path, b_canon: Option<&Path>) -> bool {
    a == b || b_canon.is_some_and(|y| fs::canonicalize(a).ok().as_deref() == Some(y))
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
    let project_canon = fs::canonicalize(project).ok();
    let mut out = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    let mut looked = 0;
    'days: for year in names_desc(&root.join("sessions")) {
        for month in names_desc(&year) {
            for day in names_desc(&month) {
                for file in names_desc(&day) {
                    if file.extension().is_none_or(|x| x != "jsonl") {
                        continue;
                    }
                    looked += 1;
                    if looked > 2000 || seen_ids.len() >= limit {
                        break 'days;
                    }
                    if let Some(s) = codex_session(&file, project, project_canon.as_deref(), &titles) {
                        seen_ids.insert(s.id.clone());
                        out.push(s);
                    }
                }
            }
        }
    }
    out
}

fn codex_session(path: &Path, project: &Path, project_canon: Option<&Path>, titles: &HashMap<String, String>) -> Option<Session> {
    let f = File::open(path).ok()?;
    let mut lines = BufReader::new(f).lines().map_while(Result::ok);
    let meta: Value = serde_json::from_str(&lines.next()?).ok()?;
    let payload = &meta["payload"];
    let cwd = text(&payload["cwd"])?;
    if !same_folder_canon(Path::new(&cwd), project, project_canon) {
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

/// Resolves the actual Gemini state directory (Gemini CLI writes under `$GEMINI_CLI_HOME/.gemini`,
/// whereas direct config roots may already point at `.gemini`).
fn gemini_state_root(root: &Path) -> PathBuf {
    let nested = root.join(".gemini");
    if nested.join("tmp").is_dir() || nested.join("projects.json").is_file() {
        nested
    } else {
        root.to_path_buf()
    }
}

/// The folder under `<root>/tmp` that holds the project's chats.
fn gemini_dir(project: &Path, root: &Path) -> Option<PathBuf> {
    let root = gemini_state_root(root);
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

// ----- Antigravity CLI ------------------------------------------------------------------------

/// Resolves the Antigravity CLI history file (`~/.gemini/antigravity-cli/history.jsonl`).
fn antigravity_history(root: &Path) -> Option<PathBuf> {
    let direct = root.join("history.jsonl");
    if direct.is_file() {
        return Some(direct);
    }
    let nested = root.join("antigravity-cli").join("history.jsonl");
    if nested.is_file() {
        return Some(nested);
    }
    let dot = root.join(".gemini").join("antigravity-cli").join("history.jsonl");
    dot.is_file().then_some(dot)
}

fn antigravity(project: &Path, root: &Path, limit: usize) -> Vec<Session> {
    let Some(hist) = antigravity_history(root) else {
        return Vec::new();
    };
    let (head, tail) = edges(&hist, 256 * 1024, 2 * 1024 * 1024);
    let mut by_id: HashMap<String, (SystemTime, String, bool)> = HashMap::new();
    for line in head.iter().chain(tail.iter()) {
        if !line.contains("conversationId") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let Some(ws_raw) = v["workspace"].as_str() else { continue };
        let ws = ws_raw.strip_prefix("file://").unwrap_or(ws_raw);
        if !same_folder(Path::new(ws), project) {
            continue;
        }
        let Some(id) = text(&v["conversationId"]).filter(|s| valid_session_id(s)) else { continue };
        let ts_ms = v["timestamp"].as_u64().unwrap_or(0);
        let updated = if ts_ms > 10_000_000_000 {
            SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(ts_ms)
        } else if ts_ms > 0 {
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(ts_ms)
        } else {
            modified(&hist)
        };
        let display = v["display"].as_str().map(one_line).unwrap_or_default();
        let good_title = typed_prompt(&display) && display.chars().count() >= 3;
        let entry = by_id.entry(id).or_insert_with(|| (updated, String::new(), false));
        if updated > entry.0 {
            entry.0 = updated;
        }
        if (!entry.2 && good_title) || (entry.1.is_empty() && !display.is_empty()) {
            entry.1 = display;
            entry.2 = good_title;
        }
    }
    let mut out: Vec<Session> = by_id
        .into_iter()
        .filter(|(_, (_, title, _))| !title.is_empty())
        .map(|(id, (updated, title, _))| Session { id, title, updated, branch: None })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.updated));
    out.truncate(limit);
    out
}

/// Token consumption summed from real agent session logs under a profile's config folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
}

impl TokenUsage {
    pub fn total(self) -> u64 {
        self.input.saturating_add(self.output)
    }

    pub fn add_assign(&mut self, other: Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
    }
}

/// Real usage & provider quota telemetry for one rolling window (e.g. 5-hour or 7-day weekly).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowQuota {
    /// Percentage of the window quota already USED (`0.0..=100.0`) when reported directly by the
    /// provider CLI (e.g. Codex `rate_limits.primary` / `secondary`).
    pub used_percent: Option<f32>,
    /// Window length in minutes (`300` = 5 hours, `10080` = 7 days / weekly, `43200` = 30 days).
    pub window_minutes: Option<u32>,
    /// Unix epoch seconds when this quota window resets (server-reported or rolling oldest + span).
    pub resets_at_unix: Option<i64>,
    /// Real input/output tokens counted inside this window from local session logs.
    pub tokens: TokenUsage,
    /// Number of assistant turns / sessions counted inside this window.
    pub turns: u32,
}

/// Multi-window usage, provider rate-limit telemetry, and local auth status for one account profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileUsageReport {
    /// 5-hour rolling window (e.g. ChatGPT Plus / Codex primary window, Claude 5h rolling window).
    pub window_5h: WindowQuota,
    /// Weekly (7-day) rolling window (or 30-day window on plans that report a monthly window).
    pub window_weekly: WindowQuota,
    /// Last 24-hour token totals (for aggregate telemetry).
    pub recent_24h: TokenUsage,
    /// Plan tier when reported by the provider (e.g. `"plus"`, `"pro"`, `"team"`, `"free"`).
    pub plan_type: Option<String>,
    /// Whether local auth credentials exist in this profile's config folder (`None` if indeterminate).
    pub signed_in: Option<bool>,
}

/// Atomically copies `src` to `dst` if `dst` is missing or older/different than `src`.
fn copy_if_newer(src: &Path, dst: &Path) -> bool {
    if dst.is_file() {
        let same_or_newer = modified(dst) >= modified(src)
            && fs::metadata(dst).ok().map(|m| m.len()) == fs::metadata(src).ok().map(|m| m.len());
        if same_or_newer {
            return true;
        }
    }
    let Some(dir) = dst.parent() else { return false };
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let tmp = dir.join(format!(
        ".tc-copy-{}-{}.tmp",
        std::process::id(),
        dst.file_name().and_then(|n| n.to_str()).unwrap_or("session")
    ));
    if fs::copy(src, &tmp).is_err() {
        let _ = fs::remove_file(&tmp);
        return false;
    }
    if fs::rename(&tmp, dst).is_ok() {
        true
    } else {
        // Windows rename onto an existing file fails: remove target and retry once.
        let _ = fs::remove_file(dst);
        let ok = fs::rename(&tmp, dst).is_ok();
        if !ok {
            let _ = fs::remove_file(&tmp);
        }
        ok
    }
}

/// Copies a session file between two isolated profile roots of the same provider (e.g. `claude1`
/// -> `claude2`) so `--resume <id>` preserves full conversation history on failover. Returns
/// whether the target now has the up-to-date session file.
pub fn copy_session_between_roots(agent: AgentKind, project: &Path, from_root: &Path, to_root: &Path, id: &str) -> bool {
    if !valid_session_id(id) {
        return false;
    }
    let same_root = same_folder(from_root, to_root);
    match agent {
        AgentKind::Claude => {
            let slug = claude_dir_name(project);
            let mut candidates = vec![from_root.join("projects").join(&slug).join(format!("{id}.jsonl"))];
            if let Ok(real) = fs::canonicalize(project) {
                let p = from_root.join("projects").join(claude_dir_name(&real)).join(format!("{id}.jsonl"));
                if !candidates.contains(&p) {
                    candidates.push(p);
                }
            }
            let Some(src) = candidates.into_iter().find(|p| p.is_file()) else {
                return false;
            };
            if same_root {
                return true;
            }
            let rel_dir = src.parent().and_then(|d| d.file_name()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(&slug));
            let dst = to_root.join("projects").join(rel_dir).join(format!("{id}.jsonl"));
            copy_if_newer(&src, &dst)
        }
        AgentKind::Codex => {
            let matches_codex_id = |file: &Path| -> bool {
                let Ok(f) = File::open(file) else { return false };
                let mut line = String::new();
                if BufReader::new(f).read_line(&mut line).is_err() {
                    return false;
                }
                let Ok(meta) = serde_json::from_str::<Value>(&line) else { return false };
                let payload = &meta["payload"];
                text(&payload["id"]).or_else(|| text(&payload["session_id"])).as_deref() == Some(id)
            };
            let mut found = None;
            let mut checked = 0usize;
            'search: for year in names_desc(&from_root.join("sessions")) {
                for month in names_desc(&year) {
                    for day in names_desc(&month) {
                        for file in names_desc(&day) {
                            if file.extension().is_none_or(|x| x != "jsonl") {
                                continue;
                            }
                            checked += 1;
                            if checked > 400 {
                                break 'search;
                            }
                            if matches_codex_id(&file) {
                                found = Some(file);
                                break 'search;
                            }
                        }
                    }
                }
            }
            let Some(src) = found else { return false };
            if same_root {
                return true;
            }
            let Ok(rel) = src.strip_prefix(from_root) else { return false };
            copy_if_newer(&src, &to_root.join(rel))
        }
        AgentKind::Gemini => {
            let Some(src_dir) = gemini_dir(project, from_root) else { return false };
            let src = names_desc(&src_dir.join("chats")).into_iter().find(|p| {
                if p.extension().is_none_or(|x| x != "jsonl") {
                    return false;
                }
                let (head, _) = edges(p, 8 * 1024, 0);
                head.first()
                    .and_then(|l| serde_json::from_str::<Value>(l).ok())
                    .and_then(|v| text(&v["sessionId"]))
                    .as_deref()
                    == Some(id)
            });
            let Some(src) = src else { return false };
            if same_root {
                return true;
            }
            let dst_base = if to_root.file_name().is_some_and(|n| n == ".gemini")
                || to_root.join("tmp").is_dir()
                || to_root.join("projects.json").is_file()
            {
                to_root.to_path_buf()
            } else {
                to_root.join(".gemini")
            };
            let proj_key = src_dir.file_name().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(claude_dir_name(project)));
            let dst_proj = dst_base.join("tmp").join(proj_key);
            let _ = fs::create_dir_all(&dst_proj);
            let _ = fs::write(dst_proj.join(".project_root"), project.to_string_lossy().as_bytes());
            let Some(fname) = src.file_name() else { return false };
            copy_if_newer(&src, &dst_proj.join("chats").join(fname))
        }
        _ => false,
    }
}

fn unix_secs(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Parses an ISO-8601 / RFC3339 timestamp (`YYYY-MM-DDTHH:MM:SS(.fff)(Z|+HH:MM|-HH:MM)`) into Unix
/// epoch seconds without external crate dependencies.
fn parse_iso_unix(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    let hh: i64 = s.get(11..13)?.parse().ok()?;
    let mm: i64 = s.get(14..16)?.parse().ok()?;
    let ss: i64 = s.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Howard Hinnant's civil-from-days inverse: days since 1970-01-01.
    let y_adj = y - i64::from(m <= 2);
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = y_adj - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let mut secs = days * 86_400 + hh * 3_600 + mm * 60 + ss;
    let rest = &s[19..];
    let tz = rest.find(['Z', 'z', '+', '-']).map(|i| &rest[i..]).unwrap_or("");
    if tz.len() >= 6 && (tz.starts_with('+') || tz.starts_with('-')) && tz.as_bytes()[3] == b':' {
        let sign = if tz.starts_with('+') { 1 } else { -1 };
        let th: i64 = tz.get(1..3)?.parse().ok()?;
        let tm: i64 = tz.get(4..6)?.parse().ok()?;
        secs -= sign * (th * 3_600 + tm * 60);
    }
    Some(secs)
}

fn non_empty_file(p: &Path) -> bool {
    fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 2)
}

fn apply_codex_window(report: &mut ProfileUsageReport, obj: &Value) {
    if !obj.is_object() {
        return;
    }
    let used = obj["used_percent"].as_f64().map(|v| (v as f32).clamp(0.0, 100.0));
    let mins = obj["window_minutes"].as_u64().and_then(|v| u32::try_from(v).ok());
    let resets = obj["resets_at"].as_i64();
    if used.is_none() && resets.is_none() {
        return;
    }
    let target = if mins.is_some_and(|m| m <= 720) {
        &mut report.window_5h
    } else {
        &mut report.window_weekly
    };
    if target.used_percent.is_none() {
        target.used_percent = used;
    }
    if target.window_minutes.is_none() {
        target.window_minutes = mins;
    }
    if target.resets_at_unix.is_none() {
        target.resets_at_unix = resets;
    }
}

/// Scans real multi-window usage (5-hour rolling, 24-hour, and 7-day weekly), provider-reported
/// server rate-limit quotas (Codex `rate_limits.primary`/`secondary`), and local sign-in status
/// under `root` for `agent`.
pub fn profile_usage(agent: AgentKind, root: &Path) -> ProfileUsageReport {
    const WIN_5H: i64 = 5 * 3_600;
    const WIN_24H: i64 = 24 * 3_600;
    const WIN_7D: i64 = 7 * 86_400;

    let now_unix = unix_secs(SystemTime::now());
    let cut_5h = now_unix - WIN_5H;
    let cut_24h = now_unix - WIN_24H;
    let cut_7d = now_unix - WIN_7D;
    let cutoff_7d_st = SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(WIN_7D as u64))
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut report = ProfileUsageReport::default();
    report.window_5h.window_minutes = Some(300);
    report.window_weekly.window_minutes = Some(10_080);

    match agent {
        AgentKind::Claude => {
            let parent_claude_json = root
                .file_name()
                .is_some_and(|n| n == ".claude")
                .then(|| root.parent().map(|p| p.join(".claude.json")))
                .flatten();
            let has_auth = non_empty_file(&root.join(".credentials.json"))
                || non_empty_file(&root.join("credentials.json"))
                || non_empty_file(&root.join(".claude.json"))
                || non_empty_file(&root.join("daemon-auth-status.json"))
                || non_empty_file(&root.join("stats-cache.json"))
                || parent_claude_json.as_deref().is_some_and(non_empty_file);
            report.signed_in = Some(has_auth);

            let mut files: Vec<(SystemTime, PathBuf)> = fs::read_dir(root.join("projects"))
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|d| fs::read_dir(d.path()).ok())
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
                .filter_map(|p| {
                    let m = modified(&p);
                    (m >= cutoff_7d_st).then_some((m, p))
                })
                .collect();
            files.sort_by_key(|f| std::cmp::Reverse(f.0));

            let mut oldest_5h: Option<i64> = None;
            let mut oldest_7d: Option<i64> = None;

            for (mtime, path) in files.into_iter().take(48) {
                let fallback_ts = unix_secs(mtime);
                let (head, tail) = edges(&path, 64 * 1024, 256 * 1024);
                let mut by_id: std::collections::HashMap<String, (TokenUsage, i64)> = std::collections::HashMap::new();
                let mut anons: Vec<(TokenUsage, i64)> = Vec::new();
                for line in head.iter().chain(tail.iter()) {
                    if !line.contains("\"usage\"") {
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                    let msg = &v["message"];
                    let u = if msg["usage"].is_object() { &msg["usage"] } else { &v["usage"] };
                    if !u.is_object() {
                        continue;
                    }
                    let ts = v["timestamp"].as_str().and_then(parse_iso_unix).unwrap_or(fallback_ts);
                    let entry = TokenUsage {
                        input: u["input_tokens"]
                            .as_u64()
                            .unwrap_or(0)
                            .saturating_add(u["cache_read_input_tokens"].as_u64().unwrap_or(0))
                            .saturating_add(u["cache_creation_input_tokens"].as_u64().unwrap_or(0)),
                        output: u["output_tokens"].as_u64().unwrap_or(0),
                    };
                    if let Some(id) = msg["id"].as_str() {
                        let slot = by_id.entry(id.to_string()).or_insert((TokenUsage::default(), ts));
                        slot.0.input = slot.0.input.max(entry.input);
                        slot.0.output = slot.0.output.max(entry.output);
                        slot.1 = slot.1.max(ts);
                    } else {
                        anons.push((entry, ts));
                    }
                }
                for (u, ts) in by_id.into_values().chain(anons) {
                    if ts >= cut_7d {
                        report.window_weekly.tokens.add_assign(u);
                        report.window_weekly.turns = report.window_weekly.turns.saturating_add(1);
                        oldest_7d = Some(oldest_7d.map_or(ts, |o| o.min(ts)));
                    }
                    if ts >= cut_24h {
                        report.recent_24h.add_assign(u);
                    }
                    if ts >= cut_5h {
                        report.window_5h.tokens.add_assign(u);
                        report.window_5h.turns = report.window_5h.turns.saturating_add(1);
                        oldest_5h = Some(oldest_5h.map_or(ts, |o| o.min(ts)));
                    }
                }
            }
            if let Some(o) = oldest_5h {
                report.window_5h.resets_at_unix = Some(o + WIN_5H);
            }
            if let Some(o) = oldest_7d {
                report.window_weekly.resets_at_unix = Some(o + WIN_7D);
            }
        }
        AgentKind::Codex => {
            report.signed_in = Some(non_empty_file(&root.join("auth.json")));
            let mut files: Vec<(SystemTime, PathBuf)> = Vec::new();
            let mut scanned = 0usize;
            'days: for year in names_desc(&root.join("sessions")) {
                for month in names_desc(&year) {
                    for day in names_desc(&month) {
                        for file in names_desc(&day) {
                            if file.extension().is_none_or(|x| x != "jsonl") {
                                continue;
                            }
                            scanned += 1;
                            if scanned > 400 {
                                break 'days;
                            }
                            let m = modified(&file);
                            if m < cutoff_7d_st {
                                continue;
                            }
                            files.push((m, file));
                        }
                    }
                }
            }
            files.sort_by_key(|f| std::cmp::Reverse(f.0));

            let mut got_rate_limits = false;
            let mut oldest_5h: Option<i64> = None;
            let mut oldest_7d: Option<i64> = None;
            let mut server_5h = WindowQuota::default();
            let mut server_weekly = WindowQuota::default();

            for (mtime, path) in files.into_iter().take(48) {
                let ts = unix_secs(mtime);
                let (_, tail) = edges(&path, 0, 128 * 1024);
                let mut got_tokens = false;
                for line in tail.iter().rev() {
                    if !line.contains("token_usage") && !line.contains("rate_limits") {
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                    let payload = &v["payload"];
                    if !got_rate_limits && payload["rate_limits"].is_object() {
                        let rl = &payload["rate_limits"];
                        if let Some(pt) = rl["plan_type"].as_str().filter(|s| !s.is_empty()) {
                            report.plan_type = Some(pt.to_string());
                        }
                        let mut tmp = ProfileUsageReport::default();
                        apply_codex_window(&mut tmp, &rl["primary"]);
                        apply_codex_window(&mut tmp, &rl["secondary"]);
                        if tmp.window_5h.used_percent.is_some() || tmp.window_weekly.used_percent.is_some() {
                            server_5h = tmp.window_5h;
                            server_weekly = tmp.window_weekly;
                            got_rate_limits = true;
                        }
                    }
                    if !got_tokens {
                        let tu = &payload["info"]["total_token_usage"];
                        if tu.is_object() {
                            let u = TokenUsage {
                                input: tu["input_tokens"].as_u64().unwrap_or(0),
                                output: tu["output_tokens"].as_u64().unwrap_or(0),
                            };
                            if ts >= cut_7d {
                                report.window_weekly.tokens.add_assign(u);
                                report.window_weekly.turns = report.window_weekly.turns.saturating_add(1);
                                oldest_7d = Some(oldest_7d.map_or(ts, |o| o.min(ts)));
                            }
                            if ts >= cut_24h {
                                report.recent_24h.add_assign(u);
                            }
                            if ts >= cut_5h {
                                report.window_5h.tokens.add_assign(u);
                                report.window_5h.turns = report.window_5h.turns.saturating_add(1);
                                oldest_5h = Some(oldest_5h.map_or(ts, |o| o.min(ts)));
                            }
                            got_tokens = true;
                        }
                    }
                    if got_tokens && got_rate_limits {
                        break;
                    }
                }
            }
            if server_5h.used_percent.is_some() || server_5h.resets_at_unix.is_some() {
                report.window_5h.used_percent = server_5h.used_percent;
                report.window_5h.window_minutes = server_5h.window_minutes.or(Some(300));
                report.window_5h.resets_at_unix = server_5h.resets_at_unix;
            } else if let Some(o) = oldest_5h {
                report.window_5h.resets_at_unix = Some(o + WIN_5H);
            }
            if server_weekly.used_percent.is_some() || server_weekly.resets_at_unix.is_some() {
                report.window_weekly.used_percent = server_weekly.used_percent;
                report.window_weekly.window_minutes = server_weekly.window_minutes.or(Some(10_080));
                report.window_weekly.resets_at_unix = server_weekly.resets_at_unix;
            } else if let Some(o) = oldest_7d {
                report.window_weekly.resets_at_unix = Some(o + WIN_7D);
            }
        }
        AgentKind::Gemini => {
            let state_root = gemini_state_root(root);
            let has_auth = non_empty_file(&state_root.join("oauth_creds.json"))
                || non_empty_file(&state_root.join("gemini-credentials.json"))
                || non_empty_file(&state_root.join("google_accounts.json"));
            report.signed_in = Some(has_auth);

            let mut files: Vec<(SystemTime, PathBuf)> = fs::read_dir(state_root.join("tmp"))
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|d| fs::read_dir(d.path().join("chats")).ok())
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
                .filter_map(|p| {
                    let m = modified(&p);
                    (m >= cutoff_7d_st).then_some((m, p))
                })
                .collect();
            files.sort_by_key(|f| std::cmp::Reverse(f.0));

            let mut oldest_5h: Option<i64> = None;
            let mut oldest_7d: Option<i64> = None;
            for (mtime, path) in files.into_iter().take(48) {
                let fallback_ts = unix_secs(mtime);
                let (head, tail) = edges(&path, 64 * 1024, 128 * 1024);
                for line in head.iter().chain(tail.iter()) {
                    if !line.contains("\"tokens\"") && !line.contains("\"usageMetadata\"") {
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                    let t = if v["tokens"].is_object() { &v["tokens"] } else { &v["usageMetadata"] };
                    if t.is_object() {
                        let ts = v["timestamp"].as_str().and_then(parse_iso_unix).unwrap_or(fallback_ts);
                        let u = TokenUsage {
                            input: t["input"].as_u64().or_else(|| t["promptTokenCount"].as_u64()).unwrap_or(0),
                            output: t["output"].as_u64().or_else(|| t["candidatesTokenCount"].as_u64()).unwrap_or(0),
                        };
                        if ts >= cut_7d {
                            report.window_weekly.tokens.add_assign(u);
                            report.window_weekly.turns = report.window_weekly.turns.saturating_add(1);
                            oldest_7d = Some(oldest_7d.map_or(ts, |o| o.min(ts)));
                        }
                        if ts >= cut_24h {
                            report.recent_24h.add_assign(u);
                        }
                        if ts >= cut_5h {
                            report.window_5h.tokens.add_assign(u);
                            report.window_5h.turns = report.window_5h.turns.saturating_add(1);
                            oldest_5h = Some(oldest_5h.map_or(ts, |o| o.min(ts)));
                        }
                    }
                }
            }
            if let Some(o) = oldest_5h {
                report.window_5h.resets_at_unix = Some(o + WIN_5H);
            }
            if let Some(o) = oldest_7d {
                report.window_weekly.resets_at_unix = Some(o + WIN_7D);
            }
        }
        AgentKind::Antigravity => {
            let has_auth = non_empty_file(&root.join("settings.json"))
                || non_empty_file(&root.join("history.jsonl"))
                || antigravity_history(root).is_some();
            report.signed_in = Some(has_auth);
        }
        AgentKind::OpenCode => {
            let has_auth = non_empty_file(&root.join("data/opencode/auth.json"))
                || non_empty_file(&root.join("auth.json"));
            report.signed_in = Some(has_auth);
        }
        _ => {}
    }
    report
}

/// Aggregates real token usage reported in recent session files (last 24 hours) under `root` for
/// `agent`.
pub fn recent_usage(agent: AgentKind, root: &Path) -> TokenUsage {
    profile_usage(agent, root).recent_24h
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

    #[test]
    fn copy_session_between_roots_and_recent_usage() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("claude1");
        let to = dir.path().join("claude2");
        let proj = Path::new("/work/app");
        let sid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let src = from.join("projects/-work-app").join(format!("{sid}.jsonl"));
        write(
            &src,
            concat!(
                "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":1200,\"cache_read_input_tokens\":300,\"output_tokens\":450}}}\n",
                "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":800,\"output_tokens\":250}}}\n",
            ),
        );
        let usage = recent_usage(AgentKind::Claude, &from);
        assert_eq!(usage.input, 2300);
        assert_eq!(usage.output, 700);
        assert_eq!(usage.total(), 3000);
        assert!(copy_session_between_roots(AgentKind::Claude, proj, &from, &to, sid));
        let dst = to.join("projects/-work-app").join(format!("{sid}.jsonl"));
        assert!(dst.is_file());
        // Round-trip failover: updating `dst` to be newer and copying back overwrites stale `src`!
        let newer = concat!(
            "{\"type\":\"assistant\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":500,\"output_tokens\":10}}}\n",
            "{\"type\":\"assistant\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":500,\"output_tokens\":120}}}\n",
        );
        fs::write(&dst, newer).unwrap();
        age(&src, 60);
        assert!(copy_session_between_roots(AgentKind::Claude, proj, &to, &from, sid));
        assert!(fs::read_to_string(&src).unwrap().contains("\"output_tokens\":120"));
        // Deduplicating by message.id keeps the FINAL/max output tokens (120, not 10!).
        let dedup = recent_usage(AgentKind::Claude, &from);
        assert_eq!((dedup.input, dedup.output), (500, 120));

        // Leading dash (CLI flag injection) and path traversal must be rejected.
        assert!(!copy_session_between_roots(AgentKind::Claude, proj, &from, &to, "../evil"));
        assert!(!copy_session_between_roots(AgentKind::Claude, proj, &from, &to, "--model"));
        assert!(resume_command(AgentKind::Claude, "claude", "--dangerously-skip-permissions").is_none());

        // Gemini session copy between isolated profile roots.
        let gfrom = dir.path().join("gemini1");
        let gto = dir.path().join("gemini2");
        write(&gfrom.join(".gemini/projects.json"), "{\"projects\":{\"/work/app\":\"app\"}}");
        write(
            &gfrom.join(".gemini/tmp/app/chats/session-2026-11-23-g123.jsonl"),
            "{\"sessionId\":\"g123-abcd\",\"kind\":\"main\"}\n{\"type\":\"user\",\"content\":[{\"text\":\"hello\"}]}\n",
        );
        assert!(copy_session_between_roots(AgentKind::Gemini, proj, &gfrom, &gto, "g123-abcd"));
        assert!(gto.join(".gemini/tmp/app/chats/session-2026-11-23-g123.jsonl").is_file());
    }

    #[test]
    fn profile_usage_parses_5h_and_weekly_windows_and_codex_rate_limits() {
        let dir = tempfile::tempdir().unwrap();
        let now = unix_secs(SystemTime::now());
        // Codex: auth.json + server-reported 5h primary & 7d weekly secondary rate_limits.
        let codex_root = dir.path().join("codex-plus");
        write(&codex_root.join("auth.json"), "{\"access_token\":\"ok\"}");
        write(
            &codex_root.join("sessions/2026/11/23/rollout-2026-11-23-test.jsonl"),
            &format!(
                concat!(
                    "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"input_tokens\":5000,\"output_tokens\":1200}}}},",
                    "\"rate_limits\":{{\"plan_type\":\"plus\",\"primary\":{{\"used_percent\":64.0,\"window_minutes\":300,\"resets_at\":{}}},\"secondary\":{{\"used_percent\":28.5,\"window_minutes\":10080,\"resets_at\":{}}}}}}}}}\n"
                ),
                now + 7200,
                now + 3 * 86_400
            ),
        );
        let rep = profile_usage(AgentKind::Codex, &codex_root);
        assert_eq!(rep.signed_in, Some(true));
        assert_eq!(rep.plan_type.as_deref(), Some("plus"));
        assert_eq!(rep.window_5h.used_percent, Some(64.0));
        assert_eq!(rep.window_5h.window_minutes, Some(300));
        assert_eq!(rep.window_5h.resets_at_unix, Some(now + 7200));
        assert_eq!(rep.window_weekly.used_percent, Some(28.5));
        assert_eq!(rep.window_weekly.window_minutes, Some(10080));
        assert_eq!(rep.window_weekly.resets_at_unix, Some(now + 3 * 86_400));
        assert_eq!(rep.window_5h.tokens.total(), 6200);
        assert_eq!(rep.window_weekly.tokens.total(), 6200);

        // Unsigned isolated Codex profile reports signed_in = Some(false).
        let empty_codex = dir.path().join("codex-empty");
        fs::create_dir_all(&empty_codex).unwrap();
        assert_eq!(profile_usage(AgentKind::Codex, &empty_codex).signed_in, Some(false));

        // ISO timestamp parser sanity check.
        assert_eq!(parse_iso_unix("1970-01-01T00:00:00Z"), Some(0));
        assert!(parse_iso_unix("2026-11-20T04:44:38.403Z").is_some());
        let t_utc = parse_iso_unix("2026-11-20T04:00:00Z").unwrap();
        let t_off = parse_iso_unix("2026-11-20T11:00:00+07:00").unwrap();
        assert_eq!(t_utc, t_off);

        // Antigravity CLI session listing & resume from history.jsonl.
        let agy_home = dir.path().join("agy-home");
        write(
            &agy_home.join(".gemini/antigravity-cli/history.jsonl"),
            concat!(
                "{\"display\":\"fix sidebar scroll\",\"timestamp\":1795378000000,\"workspace\":\"/work/app\",\"conversationId\":\"4a5903a6-3a74-4a07-873a-9905821e63e8\"}\n",
                "{\"display\":\"p\",\"timestamp\":1795378500000,\"workspace\":\"/work/app\",\"conversationId\":\"4a5903a6-3a74-4a07-873a-9905821e63e8\"}\n",
                "{\"display\":\"other repo\",\"timestamp\":1795379000000,\"workspace\":\"/work/other\",\"conversationId\":\"99999999-1111-2222-3333-444444444444\"}\n",
            ),
        );
        let agy_sessions = list(AgentKind::Antigravity, Path::new("/work/app"), &Roots::under(&agy_home), 10);
        assert_eq!(agy_sessions.len(), 1);
        assert_eq!(agy_sessions[0].id, "4a5903a6-3a74-4a07-873a-9905821e63e8");
        assert_eq!(agy_sessions[0].title, "fix sidebar scroll");
        assert_eq!(
            resume_command(AgentKind::Antigravity, "agy", "4a5903a6-3a74-4a07-873a-9905821e63e8").as_deref(),
            Some("agy --conversation 4a5903a6-3a74-4a07-873a-9905821e63e8")
        );
    }
}
