//! Setting up each agent's hooks (PRD section 8, "Cara memasang integrasi").
//!
//! 1. Per launch: a preset types the agent's command with flags that add TernaryCode's hooks for
//!    that run only. The flags point at environment variables every pane has, so the typed line
//!    stays short (`claude --settings "$TC_CLAUDE_SETTINGS"`). Checked on 9 Nov 2026: Claude
//!    Code merges `--settings` hooks with the user's; Codex takes `-c key=value` per run (and
//!    asks the user to trust new hooks once, in its own dialog).
//! 2. Installed: hooks written into the agent's own configuration, for agents typed by hand.
//!    Opt-in, shown as a diff first, backed up, and undone byte for byte when the file was not
//!    changed since.
//!
//! Every hook runs `ternarycode-hook`, which does nothing outside a TernaryCode pane.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tc_core::agent::AgentKind;
use tc_core::model::Integration;

pub const ENV_CLAUDE_SETTINGS: &str = "TC_CLAUDE_SETTINGS";
pub const ENV_CODEX_HOOKS: &str = "TC_CODEX_HOOKS";
pub const ENV_CODEX_NOTIFY: &str = "TC_CODEX_NOTIFY";

pub const CLAUDE_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Notification",
    "Stop",
    "StopFailure",
    "SessionEnd",
];

pub const CODEX_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "Stop",
];

/// Single-quotes a word for POSIX shells (and fish, which reads `'…'` the same way when the
/// word has no `'` or `\`).
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-+:@%".contains(&b))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The hook program's path as the first word of a hook command. Agents run hook commands with
/// a POSIX shell, except on Windows, where it may be `cmd.exe` or Git Bash: a double-quoted
/// path reads the same in both (Windows paths have no `"`, and their backslashes stand before
/// letters, which Bash keeps inside double quotes).
pub fn command_quote(path: &str) -> String {
    if cfg!(windows) {
        format!("\"{path}\"")
    } else {
        shell_quote(path)
    }
}

/// An environment variable in a command typed into a pane's shell, in double quotes: `$NAME`
/// for POSIX shells and fish, `$env:NAME` for PowerShell (the shell on Windows).
pub fn env_ref(name: &str) -> String {
    if cfg!(windows) {
        format!("\"$env:{name}\"")
    } else {
        format!("\"${name}\"")
    }
}

/// The hook program and the commands that run it.
#[derive(Clone, Debug)]
pub struct Hooks {
    pub bin: PathBuf,
}

impl Hooks {
    /// `ternarycode-hook` next to the running executable (the app bundle's `MacOS` folder, or
    /// `target/<profile>` during development).
    pub fn beside_current_exe() -> Self {
        let dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        Self {
            bin: dir.join(format!("ternarycode-hook{}", std::env::consts::EXE_SUFFIX)),
        }
    }

    pub fn command(&self, agent: AgentKind, event: &str) -> String {
        format!(
            "{} {} {}",
            command_quote(&self.bin.to_string_lossy()),
            agent.slug(),
            event
        )
    }

    /// `{"Event": [{"hooks": [{"type": "command", "command": …}]}], …}` for Claude Code.
    pub fn claude_hooks(&self) -> Value {
        let mut map = serde_json::Map::new();
        for e in CLAUDE_EVENTS {
            map.insert(
                e.to_string(),
                json!([{"hooks": [{"type": "command", "command": self.command(AgentKind::Claude, e)}]}]),
            );
        }
        Value::Object(map)
    }

    /// The file passed with `claude --settings` (hooks only; the user's other settings stay).
    pub fn claude_settings(&self) -> String {
        serde_json::to_string_pretty(&json!({ "hooks": self.claude_hooks() })).unwrap_or_default()
    }

    fn toml_str(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }

    /// `hooks={SessionStart=[{hooks=[{type="command",command="…"}]}],…}` for `codex -c`.
    pub fn codex_hooks_override(&self) -> String {
        let events: Vec<String> = CODEX_EVENTS
            .iter()
            .map(|e| {
                format!(
                    "{e}=[{{hooks=[{{type=\"command\",command={}}}]}}]",
                    Self::toml_str(&self.command(AgentKind::Codex, e))
                )
            })
            .collect();
        format!("hooks={{{}}}", events.join(","))
    }

    /// `notify=["…/ternarycode-hook","codex","notify"]` for `codex -c`.
    pub fn codex_notify_override(&self) -> String {
        format!(
            "notify=[{},\"codex\",\"notify\"]",
            Self::toml_str(&self.bin.to_string_lossy())
        )
    }

    /// Writes the `--settings` file and returns the variables every pane gets.
    pub fn pane_env(&self, integrations_dir: &Path) -> io::Result<Vec<(String, String)>> {
        let file = integrations_dir.join("claude-settings.json");
        tc_core::persist::write_private(&file, self.claude_settings().as_bytes())?;
        Ok(vec![
            (
                ENV_CLAUDE_SETTINGS.into(),
                file.to_string_lossy().into_owned(),
            ),
            (ENV_CODEX_HOOKS.into(), self.codex_hooks_override()),
            (ENV_CODEX_NOTIFY.into(), self.codex_notify_override()),
        ])
    }
}

/// Whether `s` starts with the bare TOML key `key` (not `key_x`), followed by one of `next`.
fn toml_key(s: &str, key: &str, next: &[char]) -> bool {
    s.trim_start()
        .strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with(next))
}

/// `hooks = …`, `hooks.Stop = …`, `[hooks]`, `[hooks.Stop]` or `[[hooks.Stop]]`.
fn defines_hooks(line: &str) -> bool {
    toml_key(line, "hooks", &['=', '.'])
        || line.trim_start().strip_prefix('[').is_some_and(|h| {
            toml_key(h.strip_prefix('[').unwrap_or(h), "hooks", &[']', '.'])
        })
}

/// Whether the user's Codex configuration has hooks of its own: a `-c hooks=…` override would
/// hide them for that run, so then only `notify` is added. The hooks in TernaryCode's own block
/// are not the user's.
pub fn codex_has_own_hooks(config_toml: &str) -> bool {
    let mut ours = false;
    config_toml.lines().any(|l| {
        match l {
            MARK_BEGIN => ours = true,
            MARK_END => ours = false,
            _ => {}
        }
        !ours && defines_hooks(l)
    })
}

/// The command a preset types for this launch: the user's command with the hook flags added
/// right after the program (or at the end when the command starts differently).
pub fn launch_command(
    command: &str,
    agent: Option<AgentKind>,
    integration: Integration,
    codex_hooks: bool,
) -> String {
    if integration != Integration::PerLaunch {
        return command.to_string();
    }
    let flags = match agent {
        Some(AgentKind::Claude) => format!("--settings {}", env_ref(ENV_CLAUDE_SETTINGS)),
        Some(AgentKind::Codex) if codex_hooks => format!(
            "-c {} -c {}",
            env_ref(ENV_CODEX_HOOKS),
            env_ref(ENV_CODEX_NOTIFY)
        ),
        Some(AgentKind::Codex) => format!("-c {}", env_ref(ENV_CODEX_NOTIFY)),
        _ => return command.to_string(),
    };
    let program = agent.map(AgentKind::slug).unwrap_or_default();
    let trimmed = command.trim();
    // The first word ends at a space or a shell operator (`claude; …`, `claude&&…`); its file
    // name may follow a Windows path (`C:\…\claude.exe`).
    let end = trimmed
        .find(|c: char| c.is_whitespace() || ";|&<>()".contains(c))
        .unwrap_or(trimmed.len());
    let (first, rest) = trimmed.split_at(end);
    let name = first.rsplit(['/', '\\']).next().unwrap_or(first);
    if name.eq_ignore_ascii_case(program) || name.eq_ignore_ascii_case(&format!("{program}.exe")) {
        format!("{first} {flags}{rest}")
    } else {
        format!("{trimmed} {flags}")
    }
}

// ----- installed hooks ------------------------------------------------------------------------

const MARK_BEGIN: &str = "# >>> TernaryCode hooks (managed; remove in TernaryCode > Integrations)";
const MARK_END: &str = "# <<< TernaryCode hooks";

/// A change to one agent configuration file, shown to the user before it is made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallPlan {
    pub agent: AgentKind,
    pub path: PathBuf,
    /// The file now (`None`: it does not exist).
    pub before: Option<String>,
    pub after: String,
    /// `before` in the same formatting as `after`, so the diff shows only real changes (the
    /// backup keeps the original bytes).
    pub before_normalized: Option<String>,
}

/// Recorded at install time, for an exact uninstall.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallRecord {
    pub agent: AgentKind,
    pub path: PathBuf,
    /// Copy of the original file (`None`: there was none).
    pub backup: Option<PathBuf>,
    /// FNV-1a of what TernaryCode wrote.
    pub written_hash: u64,
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The file's text; `None` when there is no file. Any other failure (no permission, not UTF-8)
/// is an error: such a file must not be taken for a missing one and overwritten.
fn read_existing(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| format!("{} is not UTF-8 text; TernaryCode leaves it as it is", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{} cannot be read: {e}", path.display())),
    }
}

fn is_ours(command: &str, hooks: &Hooks) -> bool {
    command.contains(&*hooks.bin.to_string_lossy()) || command.contains("ternarycode-hook")
}

/// Adds TernaryCode's hooks to Claude Code's `settings.json`, keeping everything else (and its
/// order) as it is.
pub fn plan_claude(hooks: &Hooks, path: &Path) -> Result<InstallPlan, String> {
    let before = read_existing(path)?;
    let mut root: Value = match before.as_deref().map(str::trim) {
        None | Some("") => json!({}),
        Some(text) => serde_json::from_str(text)
            .map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?,
    };
    let obj = root
        .as_object_mut()
        .ok_or_else(|| format!("{} is not a JSON object", path.display()))?;
    let hooks_obj = obj
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("\"hooks\" is not an object")?;
    for event in CLAUDE_EVENTS {
        let groups = hooks_obj
            .entry(event.to_string())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("\"hooks.{event}\" is not a list"))?;
        let present = groups.iter().any(|g| {
            g["hooks"].as_array().is_some_and(|hs| {
                hs.iter()
                    .any(|h| h["command"].as_str().is_some_and(|c| is_ours(c, hooks)))
            })
        });
        if !present {
            groups.push(json!({"hooks": [{"type": "command", "command": hooks.command(AgentKind::Claude, event)}]}));
        }
    }
    let mut after = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    after.push('\n');
    let before_normalized = before
        .as_deref()
        .and_then(|b| serde_json::from_str::<Value>(b).ok())
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .map(|s| s + "\n");
    Ok(InstallPlan {
        agent: AgentKind::Claude,
        path: path.to_path_buf(),
        before,
        after,
        before_normalized,
    })
}

/// Adds a managed block to Codex's `config.toml`: `notify` at the top (top-level keys must come
/// before any table) unless the user set one, and the hooks as arrays of tables at the end.
pub fn plan_codex(hooks: &Hooks, path: &Path) -> Result<InstallPlan, String> {
    let before = read_existing(path)?;
    let text = before.clone().unwrap_or_default();
    if text.contains(MARK_BEGIN) {
        return Ok(InstallPlan {
            agent: AgentKind::Codex,
            path: path.to_path_buf(),
            after: text.clone(),
            before,
            before_normalized: None,
        });
    }
    if codex_has_own_hooks(&text) {
        return Err(format!(
            "{} already defines hooks; add TernaryCode's by hand or use per-launch mode",
            path.display()
        ));
    }
    let has_notify = text
        .lines()
        .take_while(|l| !l.trim_start().starts_with('['))
        .any(|l| toml_key(l, "notify", &['=']));
    let mut head = String::new();
    if !has_notify {
        head = format!(
            "{MARK_BEGIN}\n{}\n{MARK_END}\n",
            hooks.codex_notify_override().replacen('=', " = ", 1)
        );
    }
    let mut tail = format!("\n{MARK_BEGIN}\n");
    for e in CODEX_EVENTS {
        tail.push_str(&format!(
            "[[hooks.{e}]]\nhooks = [{{ type = \"command\", command = {} }}]\n",
            Hooks::toml_str(&hooks.command(AgentKind::Codex, e))
        ));
    }
    tail.push_str(MARK_END);
    tail.push('\n');
    let mut after = head;
    after.push_str(&text);
    if !after.is_empty() && !after.ends_with('\n') {
        after.push('\n');
    }
    after.push_str(&tail);
    Ok(InstallPlan {
        agent: AgentKind::Codex,
        path: path.to_path_buf(),
        before,
        after,
        before_normalized: None,
    })
}

pub const GEMINI_EVENTS: &[&str] = &[
    "SessionStart",
    "BeforeAgent",
    "Notification",
    "AfterTool",
    "AfterAgent",
    "SessionEnd",
];

/// Adds TernaryCode's hooks to Gemini CLI's `settings.json` (`hooks.<Event>` →
/// `[{matcher, hooks: [{name, type: "command", command}]}]`, checked against Gemini CLI 0.43.0's
/// settings schema, where `hooksConfig.enabled` defaults to true).
pub fn plan_gemini(hooks: &Hooks, path: &Path) -> Result<InstallPlan, String> {
    let before = read_existing(path)?;
    let mut root: Value = match before.as_deref().map(str::trim) {
        None | Some("") => json!({}),
        Some(text) => serde_json::from_str(text).map_err(|e| {
            format!("{} is not plain JSON (comments are not supported): {e}", path.display())
        })?,
    };
    let obj = root
        .as_object_mut()
        .ok_or_else(|| format!("{} is not a JSON object", path.display()))?;
    let hooks_obj = obj
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("\"hooks\" is not an object")?;
    for event in GEMINI_EVENTS {
        let groups = hooks_obj
            .entry(event.to_string())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("\"hooks.{event}\" is not a list"))?;
        let present = groups.iter().any(|g| {
            g["hooks"].as_array().is_some_and(|hs| {
                hs.iter()
                    .any(|h| h["command"].as_str().is_some_and(|c| is_ours(c, hooks)))
            })
        });
        if !present {
            groups.push(json!({
                "matcher": "*",
                "hooks": [{
                    "name": "ternarycode",
                    "type": "command",
                    "command": hooks.command(AgentKind::Gemini, event),
                    "timeout": 2000
                }]
            }));
        }
    }
    let mut after = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    after.push('\n');
    let before_normalized = before
        .as_deref()
        .and_then(|b| serde_json::from_str::<Value>(b).ok())
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .map(|s| s + "\n");
    Ok(InstallPlan {
        agent: AgentKind::Gemini,
        path: path.to_path_buf(),
        before,
        after,
        before_normalized,
    })
}

/// The events the OpenCode plugin forwards (names checked in the OpenCode 1.19.5 binary).
pub const OPENCODE_EVENTS: &[&str] = &[
    "session.status",
    "session.idle",
    "session.error",
    "permission.asked",
    "permission.updated",
    "permission.replied",
];

/// The OpenCode plugin: forwards session and permission events to `ternarycode-hook`, only
/// inside a TernaryCode pane.
pub fn opencode_plugin(hooks: &Hooks) -> String {
    let bin = serde_json::to_string(&hooks.bin.to_string_lossy()).unwrap_or_default();
    let events = serde_json::to_string(OPENCODE_EVENTS).unwrap_or_default();
    format!(
        r#"// TernaryCode: tells the TernaryCode pane this OpenCode session runs in what it is doing.
// Installed by TernaryCode (Settings > Integrations); it does nothing outside TernaryCode.
import {{ spawn }} from "node:child_process";

const HOOK = {bin};
const EVENTS = new Set({events});

export const TernaryCode = async () => ({{
  event: async ({{ event }}) => {{
    if (!process.env.TERNARYCODE_SOCK || !EVENTS.has(event?.type)) return;
    try {{
      const child = spawn(HOOK, ["opencode", event.type, JSON.stringify(event.properties ?? {{}})], {{
        stdio: "ignore",
      }});
      child.on("error", () => {{}});
      child.unref?.();
    }} catch {{}}
  }},
}});
"#
    )
}

/// Installs the OpenCode plugin file (`~/.config/opencode/plugins/ternarycode.js`).
pub fn plan_opencode(hooks: &Hooks, path: &Path) -> Result<InstallPlan, String> {
    let before = read_existing(path)?;
    Ok(InstallPlan {
        agent: AgentKind::OpenCode,
        path: path.to_path_buf(),
        before,
        after: opencode_plugin(hooks),
        before_normalized: None,
    })
}

/// A plain line diff of a plan, for the confirmation dialog.
pub fn diff(plan: &InstallPlan) -> String {
    let before = plan
        .before_normalized
        .as_deref()
        .or(plan.before.as_deref())
        .unwrap_or("");
    diff_lines(before, &plan.after)
}

/// Lines prefixed `"  "`, `"+ "` or `"- "` (longest common subsequence; meant for files of a
/// few thousand lines).
pub fn diff_lines(before: &str, after: &str) -> String {
    let before: Vec<&str> = before.lines().collect();
    let after: Vec<&str> = after.lines().collect();
    // Longest common subsequence on lines (files are small).
    let (n, m) = (before.len(), after.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if before[i] == after[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, String::new());
    while i < n || j < m {
        if i < n && j < m && before[i] == after[j] {
            out.push_str(&format!("  {}\n", before[i]));
            i += 1;
            j += 1;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            out.push_str(&format!("+ {}\n", after[j]));
            j += 1;
        } else {
            out.push_str(&format!("- {}\n", before[i]));
            i += 1;
        }
    }
    out
}

/// Makes the change: backs up the original (if any) into `backup_dir`, writes the new file, and
/// returns the record needed to undo it.
pub fn apply(plan: &InstallPlan, backup_dir: &Path) -> io::Result<InstallRecord> {
    // The file may have changed while the diff was shown (the agent itself edits it): then the
    // plan is out of date, and writing it would throw that change away.
    let now = read_existing(&plan.path).map_err(io::Error::other)?;
    if now != plan.before {
        return Err(io::Error::other(format!(
            "{} changed since the changes were shown; open Install again to see them anew",
            plan.path.display()
        )));
    }
    tc_core::persist::ensure_private_dir(backup_dir)?;
    let backup = match &plan.before {
        Some(text) => {
            // Never over an earlier backup: that one may be the true original.
            let mut b = backup_dir.join(format!("{}-original", plan.agent.slug()));
            let mut n = 2;
            while b.exists() {
                b = backup_dir.join(format!("{}-original-{n}", plan.agent.slug()));
                n += 1;
            }
            tc_core::persist::write_private(&b, text.as_bytes())?;
            Some(b)
        }
        None => None,
    };
    if let Some(dir) = plan.path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    tc_core::persist::write_private(&plan.path, plan.after.as_bytes())?;
    Ok(InstallRecord {
        agent: plan.agent,
        path: plan.path.clone(),
        backup,
        written_hash: fnv(plan.after.as_bytes()),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Uninstalled {
    /// The file is exactly what it was before installing (or gone again, if there was none).
    Restored,
    /// The file changed since (or had TernaryCode's hooks before installing already: installed
    /// again after the record was lost); only TernaryCode's hooks were taken out.
    Removed,
}

pub fn uninstall(record: &InstallRecord, hooks: &Hooks) -> Result<Uninstalled, String> {
    let current = match std::fs::read(&record.path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        // Not readable is not gone: the hooks may still be in it.
        Err(e) => return Err(format!("{} cannot be read: {e}", record.path.display())),
    };
    if current.as_deref().map(fnv) == Some(record.written_hash) {
        match &record.backup {
            Some(b) => {
                let original = std::fs::read(b).map_err(|e| e.to_string())?;
                let text = String::from_utf8_lossy(&original);
                // Installed again after the record was lost, the backup has our hooks too:
                // written back as it is, it would bring them back.
                if has_ours(record.agent, &text, hooks) {
                    remove_ours(record, &text, hooks)?;
                    return Ok(Uninstalled::Removed);
                }
                tc_core::persist::write_private(&record.path, &original)
                    .map_err(|e| e.to_string())?;
            }
            None => std::fs::remove_file(&record.path).map_err(|e| e.to_string())?,
        }
        return Ok(Uninstalled::Restored);
    }
    let Some(current) = current else {
        return Ok(Uninstalled::Removed);
    };
    remove_ours(record, &String::from_utf8_lossy(&current), hooks)?;
    Ok(Uninstalled::Removed)
}

/// Whether `text` (a file at `agent`'s path) has hooks TernaryCode installed.
fn has_ours(agent: AgentKind, text: &str, hooks: &Hooks) -> bool {
    match agent {
        AgentKind::Codex => text.lines().any(|l| l == MARK_BEGIN),
        _ => is_ours(text, hooks),
    }
}

/// Writes `text` without TernaryCode's hooks to the record's file.
fn remove_ours(record: &InstallRecord, text: &str, hooks: &Hooks) -> Result<(), String> {
    let cleaned = match record.agent {
        AgentKind::OpenCode => {
            // The plugin file is TernaryCode's own; changed or not, it goes.
            return std::fs::remove_file(&record.path).map_err(|e| e.to_string());
        }
        AgentKind::Claude | AgentKind::Gemini => {
            let mut root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
            if !root.is_object() {
                return Err(format!("{} is not a JSON object; nothing was changed", record.path.display()));
            }
            if let Some(hooks_obj) = root.get_mut("hooks").and_then(Value::as_object_mut) {
                for groups in hooks_obj.values_mut() {
                    if let Some(list) = groups.as_array_mut() {
                        // Our hooks go, the user's stay, even in a group with ours; a group
                        // left empty by that goes too.
                        list.retain_mut(|g| {
                            let Some(hs) = g.get_mut("hooks").and_then(Value::as_array_mut) else {
                                return true;
                            };
                            let before = hs.len();
                            hs.retain(|h| !h["command"].as_str().is_some_and(|c| is_ours(c, hooks)));
                            hs.len() == before || !hs.is_empty()
                        });
                    }
                }
                hooks_obj.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
            }
            if root.get("hooks").and_then(Value::as_object).is_some_and(|o| o.is_empty())
                && let Some(o) = root.as_object_mut()
            {
                o.remove("hooks");
            }
            let mut s = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
            s.push('\n');
            s
        }
        _ => {
            let mut out = String::new();
            let mut skipping = false;
            for line in text.lines() {
                if line == MARK_BEGIN {
                    skipping = true;
                    continue;
                }
                if line == MARK_END {
                    skipping = false;
                    continue;
                }
                if !skipping {
                    out.push_str(line);
                    out.push('\n');
                }
            }
            out.trim_end_matches('\n').to_string() + "\n"
        }
    };
    tc_core::persist::write_private(&record.path, cleaned.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hooks() -> Hooks {
        Hooks {
            bin: PathBuf::from("/Applications/Ternary Code.app/Contents/MacOS/ternarycode-hook"),
        }
    }

    #[test]
    fn quoting_and_commands() {
        assert_eq!(shell_quote("/usr/bin/x"), "/usr/bin/x");
        assert_eq!(shell_quote("/a b/c"), "'/a b/c'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        #[cfg(unix)]
        assert_eq!(
            hooks().command(AgentKind::Claude, "Stop"),
            "'/Applications/Ternary Code.app/Contents/MacOS/ternarycode-hook' claude Stop"
        );
        #[cfg(windows)]
        {
            let h = Hooks {
                bin: PathBuf::from(r"C:\Users\Me Too\AppData\Local\Programs\TernaryCode\ternarycode-hook.exe"),
            };
            assert_eq!(
                h.command(AgentKind::Claude, "Stop"),
                r#""C:\Users\Me Too\AppData\Local\Programs\TernaryCode\ternarycode-hook.exe" claude Stop"#
            );
            assert_eq!(env_ref("X"), "\"$env:X\"");
        }
        assert!(
            Hooks::beside_current_exe()
                .bin
                .to_string_lossy()
                .ends_with(&format!("ternarycode-hook{}", std::env::consts::EXE_SUFFIX))
        );
    }

    #[test]
    fn launch_commands_get_short_flags() {
        let pl = Integration::PerLaunch;
        let (settings, hooks, notify) = (
            env_ref("TC_CLAUDE_SETTINGS"),
            env_ref("TC_CODEX_HOOKS"),
            env_ref("TC_CODEX_NOTIFY"),
        );
        #[cfg(unix)]
        assert_eq!(settings, "\"$TC_CLAUDE_SETTINGS\"");
        assert_eq!(
            launch_command("claude", Some(AgentKind::Claude), pl, true),
            format!("claude --settings {settings}")
        );
        assert_eq!(
            launch_command("claude --continue", Some(AgentKind::Claude), pl, true),
            format!("claude --settings {settings} --continue")
        );
        assert_eq!(
            launch_command("codex resume --last", Some(AgentKind::Codex), pl, true),
            format!("codex -c {hooks} -c {notify} resume --last")
        );
        assert_eq!(
            launch_command("codex", Some(AgentKind::Codex), pl, false),
            format!("codex -c {notify}")
        );
        assert_eq!(
            launch_command("claude", Some(AgentKind::Claude), Integration::Installed, true),
            "claude"
        );
        assert_eq!(launch_command("opencode", Some(AgentKind::OpenCode), pl, true), "opencode");
        // The flags follow the program, whatever its path, and come before what the line runs
        // after it.
        assert_eq!(
            launch_command(r"C:\Users\Me\.local\bin\claude.exe --continue", Some(AgentKind::Claude), pl, true),
            format!(r"C:\Users\Me\.local\bin\claude.exe --settings {settings} --continue")
        );
        assert_eq!(
            launch_command("/opt/bin/CLAUDE.EXE", Some(AgentKind::Claude), pl, true),
            format!("/opt/bin/CLAUDE.EXE --settings {settings}")
        );
        assert_eq!(
            launch_command("claude; echo done", Some(AgentKind::Claude), pl, true),
            format!("claude --settings {settings}; echo done")
        );
        assert_eq!(
            launch_command("codex&&say done", Some(AgentKind::Codex), pl, false),
            format!("codex -c {notify}&&say done")
        );
        assert_eq!(
            launch_command("codex --full-auto | tee log", Some(AgentKind::Codex), pl, false),
            format!("codex -c {notify} --full-auto | tee log")
        );
        // Not the program first: at the end, as before.
        assert_eq!(
            launch_command("npx claude", Some(AgentKind::Claude), pl, true),
            format!("npx claude --settings {settings}")
        );
        assert_eq!(
            launch_command("claudette", Some(AgentKind::Claude), pl, true),
            format!("claudette --settings {settings}")
        );
    }

    #[test]
    fn codex_overrides_are_valid_toml_shapes() {
        let h = hooks().codex_hooks_override();
        #[cfg(unix)]
        assert!(h.starts_with("hooks={SessionStart=[{hooks=[{type=\"command\",command=\"'/Applications/Ternary Code.app/"));
        #[cfg(windows)]
        assert!(h.starts_with("hooks={SessionStart=[{hooks=[{type=\"command\",command=\"\\\"/Applications/Ternary Code.app/"));
        assert!(h.contains("PermissionRequest=[{hooks="));
        assert_eq!(
            hooks().codex_notify_override(),
            "notify=[\"/Applications/Ternary Code.app/Contents/MacOS/ternarycode-hook\",\"codex\",\"notify\"]"
        );
    }

    #[test]
    fn claude_install_keeps_the_users_settings_and_undoes_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = "{\n  \"model\": \"opus\",\n  \"hooks\": {\n    \"Stop\": [\n      {\n        \"hooks\": [\n          {\"type\": \"command\", \"command\": \"say done\"}\n        ]\n      }\n    ]\n  },\n  \"permissions\": {\"allow\": [\"Bash(ls)\"]}\n}\n";
        std::fs::write(&path, original).unwrap();
        let plan = plan_claude(&hooks(), &path).unwrap();
        let v: Value = serde_json::from_str(&plan.after).unwrap();
        // The user's own Stop hook stays, ours is added; key order is kept.
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "say done");
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["model", "hooks", "permissions"]);
        // The diff shows only what is added: the user's own Stop hook is not marked as new.
        let d = diff(&plan);
        assert!(d.contains("+ "));
        assert!(!d.lines().any(|l| l.starts_with("- ")), "{d}");
        assert!(!d.lines().any(|l| l.starts_with("+ ") && l.contains("say done")), "{d}");
        // Installing twice adds nothing.
        let record = apply(&plan, &dir.path().join("backup")).unwrap();
        let again = plan_claude(&hooks(), &path).unwrap();
        assert_eq!(again.after, plan.after);
        // Untouched since: restored byte for byte.
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Restored));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

        // Changed since: only our hooks are removed, the user's edit stays.
        let record = apply(&plan_claude(&hooks(), &path).unwrap(), &dir.path().join("backup")).unwrap();
        let mut edited: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        edited["theme"] = json!("dark");
        std::fs::write(&path, serde_json::to_string_pretty(&edited).unwrap()).unwrap();
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Removed));
        let left: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(left["theme"], "dark");
        assert_eq!(left["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(left["hooks"].get("PreToolUse").is_none());
    }

    #[test]
    fn claude_install_into_a_missing_file_and_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        let plan = plan_claude(&hooks(), &path).unwrap();
        assert!(plan.before.is_none());
        let record = apply(&plan, &dir.path().join("b")).unwrap();
        assert!(path.exists());
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Restored));
        assert!(!path.exists());
    }

    #[test]
    fn gemini_install_keeps_settings_and_undoes_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = "{\n  \"theme\": \"GitHub\",\n  \"hooks\": {\"AfterAgent\": [{\"matcher\": \"*\", \"hooks\": [{\"type\": \"command\", \"command\": \"notify-me\"}]}]}\n}\n";
        std::fs::write(&path, original).unwrap();
        let plan = plan_gemini(&hooks(), &path).unwrap();
        let v: Value = serde_json::from_str(&plan.after).unwrap();
        assert_eq!(v["hooks"]["AfterAgent"].as_array().unwrap().len(), 2);
        assert_eq!(v["hooks"]["Notification"][0]["matcher"], "*");
        assert_eq!(v["hooks"]["Notification"][0]["hooks"][0]["type"], "command");
        assert!(!diff(&plan).lines().any(|l| l.starts_with("- ")));
        let record = apply(&plan, &dir.path().join("b")).unwrap();
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Restored));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        // Comments are refused rather than lost.
        std::fs::write(&path, "{ // mine\n}").unwrap();
        assert!(plan_gemini(&hooks(), &path).is_err());
    }

    #[test]
    fn opencode_plugin_is_one_file_in_and_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins/ternarycode.js");
        let plan = plan_opencode(&hooks(), &path).unwrap();
        assert!(plan.after.contains("process.env.TERNARYCODE_SOCK"));
        assert!(plan.after.contains("\"permission.asked\""));
        assert!(plan.after.contains("Ternary Code.app/Contents/MacOS/ternarycode-hook"));
        let record = apply(&plan, &dir.path().join("b")).unwrap();
        assert!(path.exists());
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Restored));
        assert!(!path.exists());
    }

    #[test]
    fn codex_install_puts_notify_first_and_hooks_last() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = "model = \"gpt-5\"\n\n[tui]\nnotifications = true\n";
        std::fs::write(&path, original).unwrap();
        let plan = plan_codex(&hooks(), &path).unwrap();
        assert!(plan.after.starts_with(MARK_BEGIN));
        let notify_at = plan.after.find("notify = [").unwrap();
        assert!(notify_at < plan.after.find("[tui]").unwrap());
        assert!(plan.after.contains("[[hooks.PermissionRequest]]"));
        let record = apply(&plan, &dir.path().join("b")).unwrap();
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Restored));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        // The user's own hooks or notify are respected.
        std::fs::write(&path, "[[hooks.Stop]]\nhooks = []\n").unwrap();
        assert!(plan_codex(&hooks(), &path).is_err());
        std::fs::write(&path, "notify = [\"x\"]\n").unwrap();
        let p = plan_codex(&hooks(), &path).unwrap();
        assert!(!p.after.starts_with(MARK_BEGIN), "the user's notify stays");
        assert!(codex_has_own_hooks("[hooks]\nStop = []\n"));
        assert!(!codex_has_own_hooks("model = \"x\"\n"));
    }

    #[test]
    fn codex_keys_are_matched_exactly() {
        for own in [
            "hooks = {}",
            "  hooks= {}",
            "hooks.Stop = []",
            "[hooks]",
            "[ hooks ]",
            "[hooks.Stop]",
            "[[hooks.Stop]]",
        ] {
            assert!(codex_has_own_hooks(own), "{own}");
        }
        for other in ["hooks_dir = \"x\"", "[hooks_extra]", "[[hooksy.Stop]]", "# hooks = {}", "[tui]\nnotifications = true"] {
            assert!(!codex_has_own_hooks(other), "{other}");
        }
        // TernaryCode's own block is not the user's hooks; the user's next to it are.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "model = \"gpt-5\"\n").unwrap();
        let installed = plan_codex(&hooks(), &path).unwrap().after;
        assert!(!codex_has_own_hooks(&installed));
        assert!(codex_has_own_hooks(&format!("{installed}[[hooks.Stop]]\nhooks = []\n")));
        // `notify_…` is not `notify`: TernaryCode's notify is still added.
        std::fs::write(&path, "notify_sound = true\n").unwrap();
        assert!(plan_codex(&hooks(), &path).unwrap().after.starts_with(MARK_BEGIN));
    }

    #[test]
    fn files_that_cannot_be_read_or_changed_meanwhile_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // Not UTF-8: refused, not taken for a missing file.
        std::fs::write(&path, b"{\"x\": \"caf\xe9\"}").unwrap();
        assert!(plan_claude(&hooks(), &path).is_err());
        // Changed after the plan was made: not overwritten.
        std::fs::write(&path, "{}\n").unwrap();
        let plan = plan_claude(&hooks(), &path).unwrap();
        std::fs::write(&path, "{\"model\": \"opus\"}\n").unwrap();
        assert!(apply(&plan, &dir.path().join("b")).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"model\": \"opus\"}\n");
        // A second install keeps the first backup.
        let plan = plan_claude(&hooks(), &path).unwrap();
        let first = apply(&plan, &dir.path().join("b")).unwrap();
        let plan = plan_claude(&hooks(), &path).unwrap();
        let second = apply(&plan, &dir.path().join("b")).unwrap();
        assert_ne!(first.backup, second.backup);
        assert_eq!(std::fs::read_to_string(first.backup.unwrap()).unwrap(), "{\"model\": \"opus\"}\n");
    }

    #[test]
    #[cfg(unix)]
    fn a_linked_settings_file_stays_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles-settings.json");
        let link = dir.path().join("settings.json");
        std::fs::write(&real, "{}\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let record = apply(&plan_claude(&hooks(), &link).unwrap(), &dir.path().join("b")).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert!(std::fs::read_to_string(&real).unwrap().contains("hooks"));
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Restored));
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "{}\n");
    }

    #[test]
    fn uninstall_leaves_no_null_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{\"model\": \"opus\"}\n").unwrap();
        let record = apply(&plan_claude(&hooks(), &path).unwrap(), &dir.path().join("b")).unwrap();
        // The user took the hooks out by hand.
        std::fs::write(&path, "{\"model\": \"opus\", \"theme\": \"dark\"}\n").unwrap();
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Removed));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("null"));
    }

    #[test]
    fn a_file_that_cannot_be_read_is_not_taken_for_a_removed_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let record = apply(&plan_claude(&hooks(), &path).unwrap(), &dir.path().join("b")).unwrap();
        // A folder where the file was: reading it fails, on every system and even as root.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(uninstall(&record, &hooks()).unwrap_err().contains("cannot be read"));
        // Gone for real: nothing left to remove.
        std::fs::remove_dir(&path).unwrap();
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Removed));
    }

    #[test]
    fn uninstall_keeps_the_users_hooks_in_a_group_with_ours() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{}\n").unwrap();
        let record = apply(&plan_claude(&hooks(), &path).unwrap(), &dir.path().join("b")).unwrap();
        // The user added a hook of their own to the group with ours, and an empty group.
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["hooks"]["Stop"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "command", "command": "say done"}));
        v["hooks"]["Stop"].as_array_mut().unwrap().push(json!({"matcher": "x", "hooks": []}));
        std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Removed));
        let left: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            left,
            json!({"hooks": {"Stop": [
                {"hooks": [{"type": "command", "command": "say done"}]},
                {"matcher": "x", "hooks": []}
            ]}})
        );
    }

    #[test]
    fn installing_again_after_the_record_was_lost_uninstalls_without_our_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join("b");
        // Claude Code: the second backup has our hooks.
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{\"model\": \"opus\"}\n").unwrap();
        apply(&plan_claude(&hooks(), &path).unwrap(), &backups).unwrap();
        let record = apply(&plan_claude(&hooks(), &path).unwrap(), &backups).unwrap();
        assert!(std::fs::read_to_string(record.backup.as_ref().unwrap()).unwrap().contains("ternarycode-hook"));
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Removed));
        let left: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(left, json!({"model": "opus"}));
        // Codex: the managed block is taken out of the backup.
        let path = dir.path().join("config.toml");
        let original = "model = \"gpt-5\"\n";
        std::fs::write(&path, original).unwrap();
        apply(&plan_codex(&hooks(), &path).unwrap(), &backups).unwrap();
        let record = apply(&plan_codex(&hooks(), &path).unwrap(), &backups).unwrap();
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Removed));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        // OpenCode: the plugin was ours already, so it goes.
        let path = dir.path().join("plugins/ternarycode.js");
        apply(&plan_opencode(&hooks(), &path).unwrap(), &backups).unwrap();
        let record = apply(&plan_opencode(&hooks(), &path).unwrap(), &backups).unwrap();
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Removed));
        assert!(!path.exists());
    }
}
