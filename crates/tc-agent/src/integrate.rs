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
/// The file passed with `claude --mcp-config`: TernaryCode's browser tools (`crate::mcp`).
pub const ENV_CLAUDE_MCP: &str = "TC_CLAUDE_MCP";
pub const ENV_CODEX_HOOKS: &str = "TC_CODEX_HOOKS";
pub const ENV_CODEX_NOTIFY: &str = "TC_CODEX_NOTIFY";
/// `-c mcp_servers.ternarycode={…}` for `codex`: the browser tools for one run.
pub const ENV_CODEX_MCP: &str = "TC_CODEX_MCP";
/// `-c developer_instructions=…` for `codex`: in a TernaryCode pane, its browser is the user's.
/// Codex reads no MCP server's instructions, and takes its own browser and computer-use tools
/// when nothing says so.
pub const ENV_CODEX_GUIDE: &str = "TC_CODEX_GUIDE";

/// OpenCode's inline configuration, merged over the user's own: TernaryCode's browser tools and
/// the guide saying its browser is the user's. In every pane (on macOS), so an `opencode` typed
/// by hand gets them too.
pub const ENV_OPENCODE_CONFIG: &str = "OPENCODE_CONFIG_CONTENT";
/// `"1"` while Settings → Browser → Agents can use the browser is on, `"0"` when off (checked by
/// installed MCP servers and hooks such as Antigravity CLI's plugin).
pub const ENV_BROWSER_AGENTS: &str = "TC_BROWSER_AGENTS";

/// What Antigravity CLI (`agy`) is injected with at `SessionStart` inside a TernaryCode pane so
/// TernaryCode's built-in browser is its primary browser unless the user explicitly names another.
pub const ANTIGRAVITY_GUIDE: &str = "You run in a terminal pane of TernaryCode, which has a browser \
built in, beside this terminal: it is the user's primary browser in this session, and the user sees \
every page and every click. Use the ternarycode browser MCP tools (browser_navigate first, \
browser_snapshot to read, browser_click/browser_type to act, browser_take_screenshot for a picture) \
for every web page you need, whether the user asks for it or your work needs it: to open, show, \
check or search something on a website (open YouTube, check it in the browser, a URL, a dev server), \
to look at a page or take a picture of it, to test a web app, to study a site's design. Do NOT use \
Antigravity's browser subagent (/browser, Chrome DevTools), do NOT start Chrome, Chromium, Edge, \
Firefox or Safari (headless included), do NOT use Playwright, Puppeteer, Selenium or a script that \
drives a browser, and do NOT run `open <url>` or `open -a` in a shell, unless the user explicitly \
names that other browser in their request. A project's own tests (npm test, playwright test) run as \
they always do. A pop-up a page opens (a sign-in such as Continue with Google) shows in a window of \
its own; the tools act in it until it closes. Never type the user's passwords or one-time codes: ask \
the user to type them there.";

/// What an agent that reads no MCP server's instructions (Codex, OpenCode) is told about
/// TernaryCode's browser (after the user's own instructions).
const CODEX_GUIDE: &str = "You run in a terminal pane of TernaryCode, which has a browser built in, \
beside this terminal: it is the user's browser in this session, and the user sees every page and \
every click. Use the ternarycode tools, browser_navigate first, for every web page you need, \
whether the user asks for it or your work needs it: to open, show, check or search something on a \
website (open YouTube, check it in the browser, a URL, a dev server), to look at a page or take a \
picture of it, to test a web app, to study a site's design. Do not start another browser for it: \
not Chrome, Chromium, Edge, Firefox or Safari (headless included), not Playwright, Puppeteer, \
Selenium or a script of your own that drives a browser, not `open <url>` or `open -a` in a shell, \
and not other browser or computer-use tools (Computer Use, Chrome, Safari, Playwright), unless the \
user names that other browser or tool. A project's own tests (npm test, playwright test) run as \
they always do. A \
pop-up a page opens (a sign-in such as Continue with Google) shows in a window of its own; the \
tools act in it until it closes. Never type the user's passwords or one-time codes: ask the user \
to type them there.";

/// The `developer_instructions` a Codex `config.toml` sets at its top level (a `-c` of the key
/// replaces them: TernaryCode's guide keeps them first).
pub fn codex_developer_instructions(config_toml: &str) -> Option<String> {
    let table: toml::Table = toml::from_str(config_toml).ok()?;
    table.get("developer_instructions")?.as_str().map(str::to_string).filter(|s| !s.trim().is_empty())
}

/// The variables `ternarycode-hook mcp` needs from the pane. Codex passes only those it is told
/// to; Gemini CLI drops any with TOKEN in its name unless the server's `env` names it.
const MCP_ENV: [&str; 3] = [crate::ENV_SOCK, crate::ENV_PANE, crate::ENV_TOKEN];

/// How long an agent waits for a browser tool: a page may load slowly, or the user be asked
/// whether the agent may open a site.
const MCP_TOOL_SECS: u32 = 900;

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

pub const ANTIGRAVITY_EVENTS: &[&str] = &[
    "SessionStart",
    "PreInvocation",
    "PreToolUse",
    "PostToolUse",
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

/// A program's path as the first word of a command `cmd.exe` runs, without quotes: as it is when
/// `cmd.exe` reads it as one word, else (on Windows) its short 8.3 form. None when neither does.
fn cmd_word(path: &Path) -> Option<String> {
    let plain = |s: &str| !s.is_empty() && !s.chars().any(|c| c.is_whitespace() || c.is_control() || "\"&|<>^%!(),;=".contains(c));
    let s = path.to_string_lossy();
    if plain(&s) {
        return Some(s.into_owned());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut buf = vec![0u16; 1024];
        // SAFETY: `wide` ends with a NUL; `buf` is as long as said.
        let n = unsafe { windows_sys::Win32::Storage::FileSystem::GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n > 0 && n < buf.len() {
            let short = String::from_utf16(&buf[..n]).ok()?;
            return plain(&short).then_some(short);
        }
    }
    None
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

/// `command` with a preset's environment variables set for it, as the pane's shell reads it:
/// `KEY='value' command` (zsh, bash, fish), `$env:KEY='value'; command` in PowerShell.
/// Names that are not variable names are left out.
pub fn with_env(command: &str, env: &std::collections::BTreeMap<String, String>) -> String {
    with_env_for(command, env, cfg!(windows))
}

/// Like [`with_env`], additionally clearing `unset` variable names before launching `command` so
/// switching from an isolated/API-key profile back to default OAuth never leaks prior env vars
/// (especially in PowerShell where `$env:KEY=...` persists for the shell lifetime).
pub fn with_env_and_unset(
    command: &str,
    env: &std::collections::BTreeMap<String, String>,
    unset: &[&str],
) -> String {
    with_env_and_unset_for(command, env, unset, cfg!(windows))
}

fn with_env_for(command: &str, env: &std::collections::BTreeMap<String, String>, powershell: bool) -> String {
    with_env_and_unset_for(command, env, &[], powershell)
}

fn with_env_and_unset_for(
    command: &str,
    env: &std::collections::BTreeMap<String, String>,
    unset: &[&str],
    powershell: bool,
) -> String {
    let name_ok = |k: &str| k.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let vars: Vec<(&String, &String)> = env.iter().filter(|(k, _)| name_ok(k)).collect();
    let clears: Vec<&str> = unset.iter().copied().filter(|k| name_ok(k) && !env.contains_key(*k)).collect();
    if vars.is_empty() && clears.is_empty() {
        return command.to_string();
    }
    let fish = !powershell
        && std::env::var_os("SHELL")
            .and_then(|s| s.into_string().ok())
            .is_some_and(|s| s.ends_with("/fish") || s == "fish");
    // Single quotes: nothing in them is expanded; escape inner quotes (and `\` in fish).
    let quote = |v: &str| {
        if powershell {
            format!("'{}'", v.replace('\'', "''"))
        } else if fish {
            format!("'{}'", v.replace('\\', "\\\\").replace('\'', "\\'"))
        } else {
            format!("'{}'", v.replace('\'', "'\\''"))
        }
    };
    if powershell {
        let mut parts: Vec<String> = clears
            .iter()
            .map(|k| format!("Remove-Item Env:{k} -ErrorAction SilentlyContinue"))
            .collect();
        parts.extend(vars.iter().map(|(k, v)| format!("$env:{k}={}", quote(v))));
        format!("{}; {command}", parts.join("; "))
    } else if !clears.is_empty() {
        let mut parts: Vec<String> = vec!["env".into()];
        for k in &clears {
            parts.push(format!("-u {k}"));
        }
        for (k, v) in &vars {
            parts.push(format!("{k}={}", quote(v)));
        }
        format!("{} {command}", parts.join(" "))
    } else {
        let sets: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={}", quote(v))).collect();
        format!("{} {command}", sets.join(" "))
    }
}

/// Inserts `flag` right after the program binary name in `command` (unless already present).
pub fn insert_flag(command: &str, flag: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() || trimmed.split_whitespace().any(|w| w == flag) {
        return trimmed.to_string();
    }
    match trimmed.split_once(char::is_whitespace) {
        Some((prog, rest)) => format!("{prog} {flag} {}", rest.trim_start()),
        None => format!("{trimmed} {flag}"),
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

    /// The file passed with `claude --settings`: the hooks, and the browser tools allowed (they
    /// act only on the project's browser pane, and TernaryCode asks before another site opens).
    /// The user's other settings stay.
    pub fn claude_settings(&self) -> String {
        serde_json::to_string_pretty(&json!({
            "hooks": self.claude_hooks(),
            "permissions": { "allow": ["mcp__ternarycode"] },
        }))
        .unwrap_or_default()
    }

    /// The file passed with `claude --mcp-config`: `ternarycode-hook mcp claude`, the browser
    /// tools of the pane the agent runs in (`tools` off: none).
    pub fn claude_mcp(&self, tools: bool) -> String {
        let servers = if tools {
            json!({ "ternarycode": {
                "type": "stdio",
                "command": self.bin.to_string_lossy(),
                "args": ["mcp", "claude"],
            } })
        } else {
            json!({})
        };
        serde_json::to_string_pretty(&json!({ "mcpServers": servers })).unwrap_or_default()
    }

    /// `mcp_servers.ternarycode={…}` for `codex -c`: the browser tools for one run (`tools`
    /// off: there, but disabled).
    pub fn codex_mcp_override(&self, tools: bool) -> String {
        self.codex_mcp_for(tools, cfg!(windows))
    }

    fn codex_mcp_for(&self, tools: bool, win: bool) -> String {
        let s = |v: &str| Self::toml_arg(v, win);
        let env: Vec<String> = MCP_ENV.iter().map(|v| s(v)).collect();
        format!(
            "mcp_servers.ternarycode={{command={},args=[{},{}],env_vars=[{}],tool_timeout_sec={MCP_TOOL_SECS},default_tools_approval_mode={}{}}}",
            s(&self.bin.to_string_lossy()),
            s("mcp"),
            s("codex"),
            env.join(","),
            s("approve"),
            if tools { "" } else { ",enabled=false" }
        )
    }

    /// The browser tools' entry in Gemini CLI's `mcpServers` (installed with the hooks).
    pub fn gemini_mcp(&self) -> Value {
        json!({
            "command": self.bin.to_string_lossy(),
            "args": ["mcp", "gemini"],
            "env": { crate::ENV_TOKEN: format!("${}", crate::ENV_TOKEN) },
            "timeout": MCP_TOOL_SECS * 1000,
            "trust": true,
        })
    }

    /// `developer_instructions="…"` for `codex -c`: the user's own (`own`, from `config.toml`),
    /// then which browser is theirs in a TernaryCode pane. `tools` off: the browser tools'
    /// disabled entry again, a flag that changes nothing.
    /// On Windows, when the user's own text cannot go through PowerShell whole (it has a `"`),
    /// the guide is not passed and theirs stays as their config sets it.
    pub fn codex_guide_override(&self, tools: bool, own: Option<&str>) -> String {
        self.codex_guide_for(tools, own, cfg!(windows))
    }

    fn codex_guide_for(&self, tools: bool, own: Option<&str>, win: bool) -> String {
        let text = match own {
            Some(own) => format!("{own}\n\n{CODEX_GUIDE}"),
            None => CODEX_GUIDE.to_string(),
        };
        if !tools || (win && Self::toml_literal(&text).is_none()) {
            return self.codex_mcp_for(tools, win);
        }
        format!("developer_instructions={}", Self::toml_arg(&text, win))
    }

    /// `OPENCODE_CONFIG_CONTENT` for the panes: the browser tools' server (unless the user has
    /// one named so) and `guide` (a file) among the instructions, over `theirs` (the variable
    /// from the app's own environment). None when theirs is not a JSON object: theirs stays.
    pub fn opencode_config(&self, guide: &Path, theirs: Option<&str>) -> Option<String> {
        let mut config: Value = match theirs.filter(|t| !t.trim().is_empty()) {
            Some(t) => serde_json::from_str(t).ok().filter(Value::is_object)?,
            None => json!({}),
        };
        let root = config.as_object_mut()?;
        let mcp = root.entry("mcp").or_insert_with(|| json!({})).as_object_mut()?;
        mcp.entry("ternarycode").or_insert_with(|| {
            json!({
                "type": "local",
                "command": [self.bin.to_string_lossy(), "mcp", "opencode"],
                "enabled": true,
            })
        });
        let instructions = root.entry("instructions").or_insert_with(|| json!([])).as_array_mut()?;
        let guide = Value::from(guide.to_string_lossy());
        if !instructions.contains(&guide) {
            instructions.push(guide);
        }
        serde_json::to_string(&config).ok()
    }

    /// A TOML basic string.
    fn toml_str(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    /// A TOML literal string (`'…'`, `'''…'''` when it has a `'`) with no `"` in it, when `s`
    /// can be one.
    fn toml_literal(s: &str) -> Option<String> {
        let bad = |c: char| c == '"' || (c.is_control() && c != '\t' && c != '\n' && c != '\r');
        if s.chars().any(bad) || s.replace("\r\n", "\n").contains('\r') {
            return None;
        }
        if !s.contains(['\'', '\n']) {
            return Some(format!("'{s}'"));
        }
        // A line break right after `'''` would be dropped.
        (!s.contains("'''") && !s.starts_with(['\n', '\r'])).then(|| format!("'''{s}'''"))
    }

    /// A string in a `codex -c` value, typed into a pane's shell. On Windows (`win`) that is
    /// PowerShell, and Windows PowerShell (5.1) drops each `"` inside an argument it hands to a
    /// program, so there the string is a literal one when it can be (no `"`).
    fn toml_arg(s: &str, win: bool) -> String {
        win.then(|| Self::toml_literal(s)).flatten().unwrap_or_else(|| Self::toml_str(s))
    }

    /// `hooks={SessionStart=[{hooks=[{type="command",command="…"}]}],…}` for `codex -c`.
    pub fn codex_hooks_override(&self) -> String {
        self.codex_hooks_for(cfg!(windows))
    }

    fn codex_hooks_for(&self, win: bool) -> String {
        // Codex runs a hook's command with `cmd.exe /C` on Windows: the program's path goes
        // without quotes there (a `"` would not get through PowerShell).
        let bare = win.then(|| cmd_word(&self.bin)).flatten();
        let events: Vec<String> = CODEX_EVENTS
            .iter()
            .map(|e| {
                let command = match &bare {
                    Some(bin) => format!("{bin} {} {e}", AgentKind::Codex.slug()),
                    None => self.command(AgentKind::Codex, e),
                };
                format!(
                    "{e}=[{{hooks=[{{type={},command={}}}]}}]",
                    Self::toml_arg("command", win),
                    Self::toml_arg(&command, win)
                )
            })
            .collect();
        format!("hooks={{{}}}", events.join(","))
    }

    /// `notify=["…/ternarycode-hook","codex","notify"]` for `codex -c`.
    pub fn codex_notify_override(&self) -> String {
        self.codex_notify_for(cfg!(windows))
    }

    fn codex_notify_for(&self, win: bool) -> String {
        let s = |v: &str| Self::toml_arg(v, win);
        format!("notify=[{},{},{}]", s(&self.bin.to_string_lossy()), s("codex"), s("notify"))
    }

    /// Writes the `--settings` and `--mcp-config` files and returns the variables every pane
    /// gets. `tools`: agents get the browser tools (Settings → Browser); written again when that
    /// changes, the variables staying the same.
    /// `codex_config`: the text of Codex's `config.toml`, whose own instructions Codex's guide
    /// keeps.
    pub fn pane_env(&self, integrations_dir: &Path, tools: bool, codex_config: Option<&str>) -> io::Result<Vec<(String, String)>> {
        let mut env = self.agent_env(integrations_dir, tools, codex_config)?;
        // OpenCode reads no MCP server's instructions either: its guide is a file it includes.
        if tools {
            let guide = integrations_dir.join("opencode-browser.md");
            if std::fs::read(&guide).ok().as_deref() != Some(CODEX_GUIDE.as_bytes()) {
                tc_core::persist::write_private(&guide, CODEX_GUIDE.as_bytes())?;
            }
            if let Some(config) = self.opencode_config(&guide, std::env::var(ENV_OPENCODE_CONFIG).ok().as_deref()) {
                env.push((ENV_OPENCODE_CONFIG.into(), config));
            }
        }
        Ok(env)
    }

    fn agent_env(&self, integrations_dir: &Path, tools: bool, codex_config: Option<&str>) -> io::Result<Vec<(String, String)>> {
        let file = integrations_dir.join("claude-settings.json");
        let mcp = integrations_dir.join("claude-mcp.json");
        for (path, text) in [(&file, self.claude_settings()), (&mcp, self.claude_mcp(tools))] {
            // Unchanged since the last launch (the usual case): not written again (each write
            // syncs to disk).
            if std::fs::read(path).ok().as_deref() != Some(text.as_bytes()) {
                tc_core::persist::write_private(path, text.as_bytes())?;
            }
        }
        Ok(vec![
            (
                ENV_CLAUDE_SETTINGS.into(),
                file.to_string_lossy().into_owned(),
            ),
            (ENV_CLAUDE_MCP.into(), mcp.to_string_lossy().into_owned()),
            (ENV_CODEX_HOOKS.into(), self.codex_hooks_override()),
            (ENV_CODEX_NOTIFY.into(), self.codex_notify_override()),
            (ENV_CODEX_MCP.into(), self.codex_mcp_override(tools)),
            (ENV_CODEX_GUIDE.into(), self.codex_guide_override(tools, codex_config.and_then(codex_developer_instructions).as_deref())),
            (ENV_BROWSER_AGENTS.into(), if tools { "1" } else { "0" }.into()),
        ])
    }

    /// Manifest for the Antigravity CLI global plugin (`~/.gemini/config/plugins/ternarycode/plugin.json`).
    pub fn antigravity_plugin_json() -> String {
        let mut s = serde_json::to_string_pretty(&json!({
            "name": "ternarycode",
            "displayName": "TernaryCode",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Connects Antigravity CLI in TernaryCode panes to TernaryCode's built-in browser and pane status."
        }))
        .unwrap_or_default();
        s.push('\n');
        s
    }

    /// MCP config for the Antigravity CLI plugin (`mcp_config.json`): exposes TernaryCode's built-in
    /// browser tools when running inside a TernaryCode pane (outside TernaryCode, lists no tools).
    pub fn antigravity_mcp_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(&json!({
            "mcpServers": {
                "ternarycode": {
                    "command": self.bin.to_string_lossy(),
                    "args": ["mcp", "antigravity"],
                    "timeoutSeconds": MCP_TOOL_SECS,
                }
            }
        }))
        .unwrap_or_default();
        s.push('\n');
        s
    }

    /// Lifecycle hooks for the Antigravity CLI plugin (`hooks.json`): reports pane status, injects
    /// primary-browser instructions on `SessionStart`, and guards `PreToolUse` against external
    /// browsers unless the user explicitly names one.
    pub fn antigravity_hooks_json(&self) -> String {
        let mut hook = serde_json::Map::new();
        for &event in ANTIGRAVITY_EVENTS {
            let entry = json!({
                "type": "command",
                "command": self.command(AgentKind::Antigravity, event),
                "timeout": 5,
            });
            let value = if matches!(event, "PreToolUse" | "PostToolUse") {
                json!([{
                    "matcher": "*",
                    "hooks": [entry],
                }])
            } else {
                json!([entry])
            };
            hook.insert(event.to_string(), value);
        }
        let mut s = serde_json::to_string_pretty(&json!({ "ternarycode": Value::Object(hook) })).unwrap_or_default();
        s.push('\n');
        s
    }
}

/// The `$BROWSER` of TernaryCode's panes, written into `dir` (a folder whose path has no spaces:
/// some programs split `$BROWSER` at them): it runs `ternarycode-hook browse`. It is a shell
/// script and a Node.js script at once, since Vite and Create React App run a `$BROWSER` ending in
/// `.js` with node (and take any other one for an app's name), while Python, gh and git run it.
pub fn write_browse_script(dir: &Path, hooks: &Hooks) -> io::Result<PathBuf> {
    let path = dir.join("browse.js");
    let bin = hooks.bin.to_string_lossy();
    let text = format!(
        "#!/bin/sh\n//bin/true; exec {} browse \"$@\"\n// TernaryCode's $BROWSER: opens an address in the browser of the pane's project. The line\n// above runs for sh, the one below for node.\nrequire('child_process').spawnSync({}, ['browse', ...process.argv.slice(2)], {{ stdio: 'inherit' }});\n",
        shell_quote(&bin),
        serde_json::to_string(&*bin).unwrap_or_default()
    );
    if std::fs::read(&path).ok().as_deref() != Some(text.as_bytes()) {
        tc_core::persist::write_private(&path, text.as_bytes())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

/// Whether `s` starts with the bare TOML key `key` (not `key_x`), followed by one of `next`.
fn toml_key(s: &str, key: &str, next: &[char]) -> bool {
    s.trim_start()
        .strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with(next))
}

/// Whether the user's Codex configuration has a `notify` of its own (at the top, before any
/// table; the one in TernaryCode's block is not the user's).
pub fn codex_has_own_notify(config_toml: &str) -> bool {
    let mut ours = false;
    for l in config_toml.lines() {
        match l {
            MARK_BEGIN => ours = true,
            MARK_END => ours = false,
            _ if ours => {}
            _ if l.trim_start().starts_with('[') => return false,
            _ if toml_key(l, "notify", &['=']) => return true,
            _ => {}
        }
    }
    false
}

/// Whether the user's Codex configuration has hooks of its own: a `-c hooks=…` override would
/// hide them for that run, so then only `notify` is added. The hooks in TernaryCode's own block
/// are not the user's.
pub fn codex_has_own_hooks(config_toml: &str) -> bool {
    let mut ours = false;
    let mut in_table = false;
    config_toml.lines().any(|l| {
        match l {
            MARK_BEGIN => {
                ours = true;
                return false;
            }
            MARK_END => {
                ours = false;
                return false;
            }
            _ => {}
        }
        let table = toml_table(l);
        if table.is_some() {
            in_table = true;
        }
        if ours {
            return false;
        }
        if let Some(name) = table {
            name == "hooks" || name.starts_with("hooks.")
        } else {
            !in_table && toml_key(l, "hooks", &['=', '.'])
        }
    })
}

/// Claude Code's flags for one launch: the browser tools and the hooks. `--mcp-config` takes several files, so it goes first: the word after
/// it is a flag, never a prompt typed after the command.
/// Codex's browser tools for one launch, and that its browser is TernaryCode's.
fn codex_mcp_flag() -> String {
    format!(" -c {} -c {}", env_ref(ENV_CODEX_MCP), env_ref(ENV_CODEX_GUIDE))
}

fn claude_flags() -> String {
    let settings = format!("--settings {}", env_ref(ENV_CLAUDE_SETTINGS));
    format!("--mcp-config {} {settings}", env_ref(ENV_CLAUDE_MCP))
}

/// The command a preset types for this launch: the user's command with the hook flags added
/// right after the program (or at the end when the command starts differently).
pub fn launch_command(
    command: &str,
    agent: Option<AgentKind>,
    integration: Integration,
    codex_hooks: bool,
) -> String {
    launch_command_with(command, agent, integration, codex_hooks, false)
}

/// `launch_command`, where `codex_own_notify`: the user's Codex config has a `notify` of its
/// own. With TernaryCode's hooks added, theirs is kept for the run (the hooks say when a turn
/// ends), as an install keeps it.
pub fn launch_command_with(
    command: &str,
    agent: Option<AgentKind>,
    integration: Integration,
    codex_hooks: bool,
    codex_own_notify: bool,
) -> String {
    let flags = match (agent, integration) {
        (Some(AgentKind::Claude), Integration::PerLaunch) => claude_flags(),
        // Installed: hooks live in ~/.claude/settings.json; browser tools still go per launch so
        // TernaryCode's built-in browser remains the primary browser in both modes.
        (Some(AgentKind::Claude), Integration::Installed) => format!("--mcp-config {}", env_ref(ENV_CLAUDE_MCP)),
        (Some(AgentKind::Codex), Integration::PerLaunch) if codex_hooks && codex_own_notify => {
            format!("-c {}{}", env_ref(ENV_CODEX_HOOKS), codex_mcp_flag())
        }
        (Some(AgentKind::Codex), Integration::PerLaunch) if codex_hooks => format!(
            "-c {} -c {}{}",
            env_ref(ENV_CODEX_HOOKS),
            env_ref(ENV_CODEX_NOTIFY),
            codex_mcp_flag()
        ),
        (Some(AgentKind::Codex), Integration::PerLaunch) => format!("-c {}{}", env_ref(ENV_CODEX_NOTIFY), codex_mcp_flag()),
        // Installed: hooks and browser tools are in its config.toml; that its browser is
        // TernaryCode's is still said per launch.
        (Some(AgentKind::Codex), Integration::Installed) => format!("-c {}", env_ref(ENV_CODEX_GUIDE)),
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
    /// Companion files written alongside `path` (e.g. Antigravity plugin's `plugin.json` and
    /// `mcp_config.json`).
    pub extra_files: Vec<(PathBuf, String)>,
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
        extra_files: Vec::new(),
    })
}

/// Adds a managed block to Codex's `config.toml`: `notify` at the top (top-level keys must come
/// before any table) unless the user set one, and the hooks as arrays of tables at the end.
pub fn plan_codex(hooks: &Hooks, path: &Path) -> Result<InstallPlan, String> {
    let before = read_existing(path)?;
    // Installed already: planned again from the user's own lines, so that an older install
    // gets what is new (the same install plans no change).
    let had = before.as_deref().is_some_and(|t| t.contains(MARK_BEGIN));
    let text = match before.as_deref() {
        Some(t) if had => without_marked(t),
        other => other.unwrap_or_default().to_string(),
    };
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
            hooks.codex_notify_for(false).replacen('=', " = ", 1)
        );
    }
    let mut tail = format!("\n{MARK_BEGIN}\n");
    for e in CODEX_EVENTS {
        tail.push_str(&format!(
            "[[hooks.{e}]]\nhooks = [{{ type = \"command\", command = {} }}]\n",
            Hooks::toml_str(&hooks.command(AgentKind::Codex, e))
        ));
    }
    // The browser tools, unless the user has a server of that name.
    if !text.contains("[mcp_servers.ternarycode]") && !text.contains("mcp_servers.ternarycode") {
        let env: Vec<String> = MCP_ENV.iter().map(|v| Hooks::toml_str(v)).collect();
        tail.push_str(&format!(
            "[mcp_servers.ternarycode]\ncommand = {}\nargs = [\"mcp\", \"codex\"]\nenv_vars = [{}]\ntool_timeout_sec = {MCP_TOOL_SECS}\ndefault_tools_approval_mode = \"approve\"\n",
            Hooks::toml_str(&hooks.bin.to_string_lossy()),
            env.join(", ")
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
        extra_files: Vec::new(),
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
    // The browser tools (a server of that name that is not TernaryCode's stays).
    let servers = obj
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("\"mcpServers\" is not an object")?;
    let theirs = servers
        .get("ternarycode")
        .is_some_and(|s| !s["command"].as_str().is_some_and(|c| is_ours(c, hooks)));
    if !theirs {
        servers.insert("ternarycode".into(), hooks.gemini_mcp());
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
        extra_files: Vec::new(),
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
        extra_files: Vec::new(),
    })
}

/// Installs the Antigravity CLI global plugin (`~/.gemini/config/plugins/ternarycode/`):
/// `hooks.json` (status signals, `SessionStart` browser guide, `PreToolUse` browser guard),
/// `mcp_config.json` (TernaryCode built-in browser MCP tools), and `plugin.json`.
/// Outside a TernaryCode pane, both the MCP server and the hooks do nothing.
pub fn plan_antigravity(hooks: &Hooks, path: &Path) -> Result<InstallPlan, String> {
    let before = read_existing(path)?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(InstallPlan {
        agent: AgentKind::Antigravity,
        path: path.to_path_buf(),
        before,
        after: hooks.antigravity_hooks_json(),
        before_normalized: None,
        extra_files: vec![
            (dir.join("plugin.json"), Hooks::antigravity_plugin_json()),
            (dir.join("mcp_config.json"), hooks.antigravity_mcp_json()),
        ],
    })
}

/// Idempotently writes or updates the Antigravity CLI plugin directory
/// (`~/.gemini/config/plugins/ternarycode/`), touching disk only when a file changed.
/// Returns the [`InstallRecord`] matching the installed `hooks.json` so Settings → Agents can
/// display and cleanly uninstall it.
pub fn sync_antigravity_plugin(hooks: &Hooks, hooks_path: &Path) -> io::Result<InstallRecord> {
    let dir = hooks_path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let hooks_text = hooks.antigravity_hooks_json();
    for (path, text) in [
        (dir.join("plugin.json"), Hooks::antigravity_plugin_json()),
        (dir.join("mcp_config.json"), hooks.antigravity_mcp_json()),
        (hooks_path.to_path_buf(), hooks_text.clone()),
    ] {
        if std::fs::read(&path).ok().as_deref() != Some(text.as_bytes()) {
            tc_core::persist::write_private(&path, text.as_bytes())?;
        }
    }
    Ok(InstallRecord {
        agent: AgentKind::Antigravity,
        path: hooks_path.to_path_buf(),
        backup: None,
        written_hash: fnv(hooks_text.as_bytes()),
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

/// Lines prefixed `"  "`, `"+ "` or `"- "` (longest common subsequence after stripping shared
/// prefix/suffix, with a bounded fallback for huge files so it never OOMs).
pub fn diff_lines(before: &str, after: &str) -> String {
    let before: Vec<&str> = before.lines().collect();
    let after: Vec<&str> = after.lines().collect();
    let mut pre = 0;
    while pre < before.len() && pre < after.len() && before[pre] == after[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < before.len() - pre
        && suf < after.len() - pre
        && before[before.len() - 1 - suf] == after[after.len() - 1 - suf]
    {
        suf += 1;
    }
    let b_mid = &before[pre..before.len() - suf];
    let a_mid = &after[pre..after.len() - suf];
    let (n, m) = (b_mid.len(), a_mid.len());
    let mut out = String::new();
    for line in &before[..pre] {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }
    // Cap DP matrix at ~4M cells (~16 MB) so giant files fall back cleanly instead of OOMing.
    if n.saturating_mul(m) > 4_000_000 {
        for line in b_mid {
            out.push_str("- ");
            out.push_str(line);
            out.push('\n');
        }
        for line in a_mid {
            out.push_str("+ ");
            out.push_str(line);
            out.push('\n');
        }
    } else if n > 0 || m > 0 {
        let mut lcs = vec![0u32; (n + 1) * (m + 1)];
        let stride = m + 1;
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i * stride + j] = if b_mid[i] == a_mid[j] {
                    lcs[(i + 1) * stride + j + 1] + 1
                } else {
                    lcs[(i + 1) * stride + j].max(lcs[i * stride + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && b_mid[i] == a_mid[j] {
                out.push_str("  ");
                out.push_str(b_mid[i]);
                out.push('\n');
                i += 1;
                j += 1;
            } else if j < m && (i == n || lcs[i * stride + j + 1] >= lcs[(i + 1) * stride + j]) {
                out.push_str("+ ");
                out.push_str(a_mid[j]);
                out.push('\n');
                j += 1;
            } else {
                out.push_str("- ");
                out.push_str(b_mid[i]);
                out.push('\n');
                i += 1;
            }
        }
    }
    for line in &before[before.len() - suf..] {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
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
    for (extra_path, extra_text) in &plan.extra_files {
        if let Some(dir) = extra_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        tc_core::persist::write_private(extra_path, extra_text.as_bytes())?;
    }
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
    if record.agent == AgentKind::Antigravity {
        // The plugin directory is TernaryCode's own (`~/.gemini/config/plugins/ternarycode/`).
        if let Some(dir) = record.path.parent() {
            for name in ["hooks.json", "mcp_config.json", "plugin.json"] {
                let p = dir.join(name);
                if let Err(e) = std::fs::remove_file(&p)
                    && e.kind() != io::ErrorKind::NotFound
                {
                    return Err(e.to_string());
                }
            }
            let _ = std::fs::remove_dir(dir);
        }
        return Ok(Uninstalled::Restored);
    }
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

/// The name of the table a TOML line starts (`[a.b]`, `[[a.b]]`, quotes and spaces left out),
/// if it starts one.
fn toml_table(line: &str) -> Option<String> {
    let t = line.trim();
    let inner = t.strip_prefix('[')?;
    let close = inner.rfind(']')?;
    // After the `]`: nothing, or a comment (else it is a line of an array).
    if !inner[close + 1..].trim().is_empty() && !inner[close + 1..].trim_start().starts_with('#') {
        return None;
    }
    let name = inner[..close].strip_prefix('[').unwrap_or(&inner[..close]);
    let name = name.strip_suffix(']').unwrap_or(name);
    Some(name.chars().filter(|c| !matches!(c, '"' | '\'' | ' ' | '\t')).collect())
}

/// `text` without what TernaryCode wrote in its marked blocks (Codex's `config.toml`): the
/// `notify` at the top, the `[[hooks.…]]` tables and the browser tools' server. Codex edits the
/// file itself (toml_edit): a key it adds at the top goes right after our `notify`, and a table
/// it adds goes before the file's last comment, our closing mark. Those lines are the user's,
/// inside our marks, and they stay.
fn without_marked(text: &str) -> String {
    let mut out = String::new();
    // In a marked block; a table started yet (in the file); the table the line is in is ours.
    let (mut marked, mut in_table, mut ours) = (false, false, false);
    for line in text.lines() {
        if line == MARK_BEGIN {
            marked = true;
            continue;
        }
        if line == MARK_END {
            marked = false;
            continue;
        }
        if let Some(name) = toml_table(line) {
            in_table = true;
            ours = marked && (name == "hooks" || name.starts_with("hooks.") || name == "mcp_servers.ternarycode" || name.starts_with("mcp_servers.ternarycode."));
        }
        let drop = marked
            && if in_table { ours || line.trim().is_empty() } else { toml_key(line, "notify", &['=']) || line.trim().is_empty() };
        if !drop {
            out.push_str(line);
            out.push('\n');
        }
    }
    let trimmed = out.trim_end_matches('\n');
    if trimmed.is_empty() { String::new() } else { format!("{trimmed}\n") }
}

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
            // The browser tools' server (Gemini's), when it is TernaryCode's.
            if let Some(servers) = root.get_mut("mcpServers").and_then(Value::as_object_mut) {
                if servers.get("ternarycode").is_some_and(|s| s["command"].as_str().is_some_and(|c| is_ours(c, hooks))) {
                    servers.remove("ternarycode");
                }
                if servers.is_empty()
                    && let Some(o) = root.as_object_mut()
                {
                    o.remove("mcpServers");
                }
            }
            let mut s = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
            s.push('\n');
            s
        }
        _ => without_marked(text),
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
        // Claude Code: the browser tools first (`--mcp-config` takes several files).
        let claude = claude_flags();
        let mcp = codex_mcp_flag();
        assert_eq!(mcp, format!(" -c {} -c {}", env_ref("TC_CODEX_MCP"), env_ref("TC_CODEX_GUIDE")));
        assert_eq!(claude, format!("--mcp-config {} --settings {settings}", env_ref("TC_CLAUDE_MCP")));
        assert_eq!(
            launch_command("claude", Some(AgentKind::Claude), pl, true),
            format!("claude {claude}")
        );
        assert_eq!(
            launch_command("claude --continue", Some(AgentKind::Claude), pl, true),
            format!("claude {claude} --continue")
        );
        assert_eq!(
            launch_command("codex resume --last", Some(AgentKind::Codex), pl, true),
            format!("codex -c {hooks} -c {notify}{mcp} resume --last")
        );
        assert_eq!(
            launch_command("codex", Some(AgentKind::Codex), pl, false),
            format!("codex -c {notify}{mcp}")
        );
        assert_eq!(
            launch_command("claude", Some(AgentKind::Claude), Integration::Installed, true),
            format!("claude --mcp-config {}", env_ref("TC_CLAUDE_MCP"))
        );
        // Codex installed (hooks and browser tools in its config.toml): only that its browser is
        // TernaryCode's; nothing when off.
        assert_eq!(
            launch_command("codex", Some(AgentKind::Codex), Integration::Installed, true),
            format!("codex -c {}", env_ref("TC_CODEX_GUIDE"))
        );
        assert_eq!(launch_command("codex", Some(AgentKind::Codex), Integration::Off, true), "codex");
        assert_eq!(launch_command("opencode", Some(AgentKind::OpenCode), pl, true), "opencode");
        // The flags follow the program, whatever its path, and come before what the line runs
        // after it.
        assert_eq!(
            launch_command(r"C:\Users\Me\.local\bin\claude.exe --continue", Some(AgentKind::Claude), pl, true),
            format!(r"C:\Users\Me\.local\bin\claude.exe {claude} --continue")
        );
        assert_eq!(
            launch_command("/opt/bin/CLAUDE.EXE", Some(AgentKind::Claude), pl, true),
            format!("/opt/bin/CLAUDE.EXE {claude}")
        );
        assert_eq!(
            launch_command("claude; echo done", Some(AgentKind::Claude), pl, true),
            format!("claude {claude}; echo done")
        );
        assert_eq!(
            launch_command("codex&&say done", Some(AgentKind::Codex), pl, false),
            format!("codex -c {notify}{mcp}&&say done")
        );
        assert_eq!(
            launch_command("codex --full-auto | tee log", Some(AgentKind::Codex), pl, false),
            format!("codex -c {notify}{mcp} --full-auto | tee log")
        );
        // Not the program first: at the end, as before.
        assert_eq!(
            launch_command("npx claude", Some(AgentKind::Claude), pl, true),
            format!("npx claude {claude}")
        );
        assert_eq!(
            launch_command("claudette", Some(AgentKind::Claude), pl, true),
            format!("claudette {claude}")
        );
    }

    #[test]
    fn codex_overrides_are_valid_toml_shapes() {
        // As typed into a POSIX shell (Windows': `codex_values_get_through_windows_powershell`).
        let on = hooks().codex_mcp_for(true, false);
        assert!(on.starts_with("mcp_servers.ternarycode={command="));
        // Approved for the run: Codex asks before each call of a server that is not (and fails
        // it where it may not ask).
        assert!(on.ends_with(",args=[\"mcp\",\"codex\"],env_vars=[\"TERNARYCODE_SOCK\",\"TERNARYCODE_PANE\",\"TERNARYCODE_TOKEN\"],tool_timeout_sec=900,default_tools_approval_mode=\"approve\"}"));
        assert!(hooks().codex_mcp_for(false, false).ends_with(",default_tools_approval_mode=\"approve\",enabled=false}"));
        // Each parses as TOML (`codex -c` takes a value that does not as a plain string).
        let parses = |kv: &str| {
            let (k, v) = kv.split_once('=').unwrap();
            toml::from_str::<toml::Table>(&format!("{} = {v}", k.replace('.', "_"))).is_ok()
        };
        assert!(parses(&on));
        assert!(parses(&hooks().codex_hooks_for(false)));
        assert!(parses(&hooks().codex_notify_for(false)));
        let h = hooks().codex_hooks_for(false);
        #[cfg(unix)]
        assert!(h.starts_with("hooks={SessionStart=[{hooks=[{type=\"command\",command=\"'/Applications/Ternary Code.app/"));
        #[cfg(windows)]
        assert!(h.starts_with("hooks={SessionStart=[{hooks=[{type=\"command\",command=\"\\\"/Applications/Ternary Code.app/"));
        assert!(h.contains("PermissionRequest=[{hooks="));
        assert_eq!(
            hooks().codex_notify_for(false),
            "notify=[\"/Applications/Ternary Code.app/Contents/MacOS/ternarycode-hook\",\"codex\",\"notify\"]"
        );
    }

    /// Each `codex -c` value as Windows PowerShell hands it over: no `"` (5.1 drops them), and
    /// what Codex reads from it (`_x_ = <value>`, as its `-c` does) is what was meant.
    #[test]
    fn preset_environment_is_set_for_the_command() {
        let mut env = std::collections::BTreeMap::new();
        assert_eq!(with_env_for("claude", &env, false), "claude");
        env.insert("ANTHROPIC_MODEL".to_string(), "opus".to_string());
        env.insert("NOTE".to_string(), "it's here".to_string());
        env.insert("bad name".to_string(), "x".to_string());
        assert_eq!(with_env_for("claude", &env, false), "ANTHROPIC_MODEL='opus' NOTE='it'\\''s here' claude");
        assert_eq!(with_env_for("claude", &env, true), "$env:ANTHROPIC_MODEL='opus'; $env:NOTE='it''s here'; claude");
    }

    #[test]
    fn codex_values_get_through_windows_powershell() {
        let bin = r"C:\Users\ilham\AppData\Local\Programs\TernaryCode\ternarycode-hook.exe";
        let h = Hooks { bin: PathBuf::from(bin) };
        let read = |kv: &str, key: &str| -> toml::Value {
            assert!(!kv.contains('"'), "{kv}");
            let (k, v) = kv.split_once('=').unwrap();
            assert_eq!(k, key);
            toml::from_str::<toml::Table>(&format!("_x_ = {v}")).unwrap_or_else(|e| panic!("{kv}: {e}"))["_x_"].clone()
        };
        let mcp = read(&h.codex_mcp_for(true, true), "mcp_servers.ternarycode");
        assert_eq!(mcp["command"].as_str(), Some(bin));
        assert_eq!(mcp["args"], toml::Value::from(vec!["mcp", "codex"]));
        assert_eq!(mcp["env_vars"], toml::Value::from(MCP_ENV.to_vec()));
        assert_eq!(mcp["tool_timeout_sec"].as_integer(), Some(MCP_TOOL_SECS as i64));
        assert_eq!(mcp["default_tools_approval_mode"].as_str(), Some("approve"));
        assert!(mcp.get("enabled").is_none());
        assert_eq!(read(&h.codex_mcp_for(false, true), "mcp_servers.ternarycode")["enabled"].as_bool(), Some(false));
        // Hooks: run by `cmd.exe /C`, the path bare.
        let hooks = read(&h.codex_hooks_for(true), "hooks");
        for e in CODEX_EVENTS {
            let one = &hooks[*e][0]["hooks"][0];
            assert_eq!(one["type"].as_str(), Some("command"));
            assert_eq!(one["command"].as_str().unwrap(), format!("{bin} codex {e}"));
        }
        assert_eq!(read(&h.codex_notify_for(true), "notify"), toml::Value::from(vec![bin, "codex", "notify"]));
        // The guide (it has a `'`), and after the user's own, lines and all.
        assert_eq!(read(&h.codex_guide_for(true, None, true), "developer_instructions").as_str(), Some(CODEX_GUIDE));
        let own = "Answer in Indonesian.\nIt's C:\\x, kept.\r\n\tIndented.";
        let both = read(&h.codex_guide_for(true, Some(own), true), "developer_instructions");
        assert_eq!(both.as_str().unwrap(), format!("{own}\n\n{CODEX_GUIDE}"));
        // Theirs with a `"`: not passed (their config keeps it), a flag that changes nothing.
        assert_eq!(h.codex_guide_for(true, Some("Say \"hi\"."), true), h.codex_mcp_for(true, true));
        // A path with a space and no short form: quoted, as before.
        let spaced = Hooks { bin: PathBuf::from(r"C:\No Such Dir\ternarycode-hook.exe") }.codex_hooks_for(true);
        let v = toml::from_str::<toml::Table>(&format!("_x_ = {}", spaced.split_once('=').unwrap().1)).unwrap();
        let command = v["_x_"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap().to_string();
        assert_eq!(command, Hooks { bin: PathBuf::from(r"C:\No Such Dir\ternarycode-hook.exe") }.command(AgentKind::Codex, "Stop"));
        // Literal strings: whatever has no `"`.
        for s in ["plain", "it's", "x'", "'x", "a''", "tab\there", "two\nlines", "crlf\r\nok"] {
            let lit = Hooks::toml_literal(s).unwrap();
            assert_eq!(toml::from_str::<toml::Table>(&format!("x = {lit}")).unwrap()["x"].as_str(), Some(s), "{lit}");
        }
        for s in ["a\"b", "x'''y", "\nlead", "bell\u{7}", "lone\rcr"] {
            assert_eq!(Hooks::toml_literal(s), None, "{s:?}");
        }
    }

    #[test]
    fn codex_is_told_its_browser_after_the_users_own_instructions() {
        let read = |kv: &str| -> String {
            let (k, v) = kv.split_once('=').unwrap();
            assert_eq!(k, "developer_instructions");
            toml::from_str::<toml::Table>(&format!("x = {v}")).unwrap()["x"].as_str().unwrap().to_string()
        };
        let guide = read(&hooks().codex_guide_for(true, None, false));
        assert!(guide.starts_with("You run in a terminal pane of TernaryCode"));
        assert!(guide.contains("unless the user names that other browser"));
        // The user's own, with quotes, a backslash and lines, come first, whole.
        let own = "Answer in \"Indonesian\".\nPaths like C:\\x stay as written.\n\tIndented.";
        let both = read(&hooks().codex_guide_for(true, Some(own), false));
        assert_eq!(both, format!("{own}\n\n{CODEX_GUIDE}"));
        // Browser tools off: a flag that changes nothing.
        assert_eq!(hooks().codex_guide_override(false, Some(own)), hooks().codex_mcp_override(false));
        // Read from config.toml: top level only, any kind of string.
        assert_eq!(codex_developer_instructions("model = \"o3\"\ndeveloper_instructions = '''\nBe brief.'''\n").as_deref(), Some("Be brief."));
        assert_eq!(codex_developer_instructions("developer_instructions = \"Be \\\"brief\\\".\"\n[profiles.x]\n").as_deref(), Some("Be \"brief\"."));
        assert_eq!(codex_developer_instructions("[profiles.x]\ndeveloper_instructions = \"x\"\n"), None);
        assert_eq!(codex_developer_instructions("developer_instructions = \"  \"\n"), None);
        assert_eq!(codex_developer_instructions("not toml ["), None);
    }

    #[test]
    fn opencode_gets_the_browser_tools_over_the_users_own_config() {
        let guide = Path::new("/data/integrations/opencode-browser.md");
        let parse = |s: Option<String>| -> Value { serde_json::from_str(&s.unwrap()).unwrap() };
        // None of theirs: the server, and the guide among the instructions.
        let c = parse(hooks().opencode_config(guide, None));
        assert_eq!(c["mcp"]["ternarycode"]["command"], json!(["/Applications/Ternary Code.app/Contents/MacOS/ternarycode-hook", "mcp", "opencode"]));
        assert_eq!(c["mcp"]["ternarycode"]["type"], "local");
        assert_eq!(c["instructions"], json!([guide.to_string_lossy()]));
        assert_eq!(parse(hooks().opencode_config(guide, Some("  "))), c);
        // Theirs: kept whole, ours added (their own server of that name wins).
        let theirs = r#"{"model":"x/y","mcp":{"docs":{"type":"remote","url":"https://d"}},"instructions":["AGENTS.md"]}"#;
        let c = parse(hooks().opencode_config(guide, Some(theirs)));
        assert_eq!(c["model"], "x/y");
        assert_eq!(c["mcp"]["docs"]["url"], "https://d");
        assert!(c["mcp"]["ternarycode"].is_object());
        assert_eq!(c["instructions"], json!(["AGENTS.md", guide.to_string_lossy()]));
        let own = r#"{"mcp":{"ternarycode":{"type":"local","command":["mine"]}}}"#;
        assert_eq!(parse(hooks().opencode_config(guide, Some(own)))["mcp"]["ternarycode"]["command"], json!(["mine"]));
        // Not a JSON object: theirs stays as it is.
        assert_eq!(hooks().opencode_config(guide, Some("{model: x}")), None);
        assert_eq!(hooks().opencode_config(guide, Some("[1]")), None);
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
        // The browser tools, the token named (Gemini CLI drops variables named *TOKEN*).
        let server = &v["mcpServers"]["ternarycode"];
        assert_eq!(server["args"], json!(["mcp", "gemini"]));
        assert_eq!(server["env"]["TERNARYCODE_TOKEN"], "$TERNARYCODE_TOKEN");
        let record = apply(&plan, &dir.path().join("b")).unwrap();
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Restored));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        // Changed since: only TernaryCode's parts go (hooks and server), the user's stay.
        let plan = plan_gemini(&hooks(), &path).unwrap();
        let record = apply(&plan, &dir.path().join("b2")).unwrap();
        let mut v: Value = serde_json::from_str(&plan.after).unwrap();
        v["mcpServers"]["mine"] = json!({"command": "my-server"});
        v["theme"] = json!("Dracula");
        std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        assert!(uninstall(&record, &hooks()).is_ok());
        let left: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(left["mcpServers"].get("ternarycode").is_none());
        assert_eq!(left["mcpServers"]["mine"]["command"], "my-server");
        assert_eq!(left["hooks"]["AfterAgent"].as_array().unwrap().len(), 1);
        // A server of that name that is not TernaryCode's is left alone.
        std::fs::write(&path, "{\"mcpServers\": {\"ternarycode\": {\"command\": \"elsewhere\"}}}\n").unwrap();
        let v: Value = serde_json::from_str(&plan_gemini(&hooks(), &path).unwrap().after).unwrap();
        assert_eq!(v["mcpServers"]["ternarycode"]["command"], "elsewhere");
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
        // The browser tools, with the pane's variables passed through.
        assert!(plan.after.contains("[mcp_servers.ternarycode]\n"));
        assert!(plan.after.contains("args = [\"mcp\", \"codex\"]\nenv_vars = [\"TERNARYCODE_SOCK\", \"TERNARYCODE_PANE\", \"TERNARYCODE_TOKEN\"]\ntool_timeout_sec = 900\n"));
        let record = apply(&plan, &dir.path().join("b")).unwrap();
        // Installed: planned again, nothing changes.
        assert_eq!(plan_codex(&hooks(), &path).unwrap().after, plan.after);
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Restored));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        // An install from before the browser tools gets them, its old block replaced.
        let old = format!("{MARK_BEGIN}\nnotify = [\"old\"]\n{MARK_END}\nmodel = \"gpt-5\"\n\n{MARK_BEGIN}\n[[hooks.Stop]]\nhooks = []\n{MARK_END}\n");
        std::fs::write(&path, &old).unwrap();
        let up = plan_codex(&hooks(), &path).unwrap();
        assert!(!up.after.contains("\"old\""));
        assert!(up.after.contains("[mcp_servers.ternarycode]"));
        assert_eq!(up.after.matches(MARK_BEGIN).count(), 2);
        assert!(up.after.contains("model = \"gpt-5\""));
        // The user's own hooks or notify are respected.
        std::fs::write(&path, "[[hooks.Stop]]\nhooks = []\n").unwrap();
        assert!(plan_codex(&hooks(), &path).is_err());
        std::fs::write(&path, "notify = [\"x\"]\n").unwrap();
        let p = plan_codex(&hooks(), &path).unwrap();
        assert!(!p.after.starts_with(MARK_BEGIN), "the user's notify stays");
        assert!(codex_has_own_hooks("[hooks]\nStop = []\n"));
        assert!(!codex_has_own_hooks("model = \"x\"\n"));
        // The user's notify, not ours or one in a table.
        assert!(codex_has_own_notify("model = \"x\"\nnotify = [\"say\"]\n"));
        assert!(!codex_has_own_notify(&p.after.replace("notify = [\"x\"]", "")));
        assert!(!codex_has_own_notify(&plan_codex(&hooks(), &dir.path().join("none.toml")).unwrap().after));
        assert!(!codex_has_own_notify("[tui]\nnotify = true\n"));
        // Per launch, their notify stays when TernaryCode's hooks are added.
        let pl = Integration::PerLaunch;
        let kept = launch_command_with("codex", Some(AgentKind::Codex), pl, true, true);
        assert!(kept.contains("TC_CODEX_HOOKS") && !kept.contains("TC_CODEX_NOTIFY"), "{kept}");
        let without_hooks = launch_command_with("codex", Some(AgentKind::Codex), pl, false, true);
        assert!(without_hooks.contains("TC_CODEX_NOTIFY"), "{without_hooks}");
    }

    /// Codex writes into its config itself (toml_edit): a key it adds at the top lands after our
    /// `notify`, a table it adds lands before our closing mark. Uninstalling keeps them.
    #[test]
    fn codex_uninstall_keeps_what_codex_wrote_inside_our_marks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[tui]\nnotifications = true\n").unwrap();
        let plan = plan_codex(&hooks(), &path).unwrap();
        let record = apply(&plan, &dir.path().join("b")).unwrap();
        let installed = std::fs::read_to_string(&path).unwrap();
        let notify = installed.lines().nth(1).unwrap().to_string();
        assert!(notify.starts_with("notify = ["));
        let edited = installed.replacen(&format!("{notify}\n"), &format!("{notify}\nmodel = \"gpt-5.5\"\n"), 1);
        let last = edited.rfind(MARK_END).unwrap();
        let edited = format!(
            "{}\n[projects.\"/Users/me/new app\"]\ntrust_level = \"trusted\"\n\n[mcp_servers.ternarycode.tools.browser_click]\napproval_mode = \"approve\"\n{}",
            &edited[..last],
            &edited[last..]
        );
        std::fs::write(&path, &edited).unwrap();
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Removed));
        let left = std::fs::read_to_string(&path).unwrap();
        assert!(left.starts_with("model = \"gpt-5.5\"\n"), "{left}");
        assert!(left.contains("[tui]\nnotifications = true\n"), "{left}");
        assert!(left.contains("[projects.\"/Users/me/new app\"]\ntrust_level = \"trusted\"\n"), "{left}");
        assert!(!left.contains("notify") && !left.contains("hooks") && !left.contains("ternarycode") && !left.contains("approval_mode"), "{left}");
        assert!(!left.contains(MARK_BEGIN) && !left.contains(MARK_END), "{left}");
        // Installed again over it: the user's lines stay, ours come back once.
        std::fs::write(&path, &edited).unwrap();
        let again = plan_codex(&hooks(), &path).unwrap().after;
        assert!(again.contains("model = \"gpt-5.5\"") && again.contains("trust_level = \"trusted\""), "{again}");
        assert_eq!(again.matches("[mcp_servers.ternarycode]").count(), 1, "{again}");
        assert_eq!(again.matches("notify = [").count(), 1, "{again}");
    }

    /// The `$BROWSER` file runs `ternarycode-hook browse <address>` as a shell script and, where
    /// Node.js is installed, as a Node.js one (how Vite runs it).
    #[cfg(unix)]
    #[test]
    fn browser_script_runs_for_sh_and_node() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("args.txt");
        // A stand-in for the hook program, in a folder with a space and a quote in its name.
        let bin_dir = dir.path().join("Ternary Code's");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let bin = bin_dir.join("ternarycode-hook");
        std::fs::write(&bin, format!("#!/bin/sh\nprintf '%s|' \"$@\" >> '{}'\n", out.display())).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let script = write_browse_script(dir.path(), &Hooks { bin }).unwrap();
        assert!(script.to_string_lossy().ends_with("browse.js"));
        let ok = std::process::Command::new(&script).arg("http://localhost:5173/").status().unwrap();
        assert!(ok.success());
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "browse|http://localhost:5173/|");
        if std::process::Command::new("node").arg("--version").output().is_ok_and(|o| o.status.success()) {
            std::fs::remove_file(&out).unwrap();
            let ok = std::process::Command::new("node").arg(&script).arg("https://a.b/?q=1 2").status().unwrap();
            assert!(ok.success());
            assert_eq!(std::fs::read_to_string(&out).unwrap(), "browse|https://a.b/?q=1 2|");
        }
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

    #[test]
    fn codex_hooks_ignores_unrelated_tables_and_strips_empty_blocks() {
        assert!(!codex_has_own_hooks("[other]\nhooks = true\n"));
        assert!(codex_has_own_hooks("hooks = true\n"));
        assert!(codex_has_own_hooks("[hooks]\nStop = []\n"));
        assert_eq!(without_marked(&format!("{MARK_BEGIN}\nnotify = [\"ternarycode-hook\", \"codex\"]\n{MARK_END}\n")), "");
    }

    #[test]
    fn antigravity_plugin_installs_syncs_and_uninstalls_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("plugins/ternarycode");
        let hooks_path = plugin.join("hooks.json");
        let plan = plan_antigravity(&hooks(), &hooks_path).unwrap();
        assert_eq!(plan.extra_files.len(), 2);
        let record = apply(&plan, &dir.path().join("b")).unwrap();
        assert!(plugin.join("plugin.json").exists());
        assert!(plugin.join("mcp_config.json").exists());
        assert!(hooks_path.exists());
        let mcp: Value = serde_json::from_str(&std::fs::read_to_string(plugin.join("mcp_config.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["ternarycode"]["args"], json!(["mcp", "antigravity"]));
        let hooks_v: Value = serde_json::from_str(&std::fs::read_to_string(&hooks_path).unwrap()).unwrap();
        assert!(hooks_v["ternarycode"]["SessionStart"][0]["command"].as_str().unwrap().contains("antigravity SessionStart"));
        assert_eq!(hooks_v["ternarycode"]["PreToolUse"][0]["matcher"], "*");
        assert_eq!(uninstall(&record, &hooks()), Ok(Uninstalled::Restored));
        assert!(!plugin.exists());
        // Sync creates it idempotently too.
        let rec2 = sync_antigravity_plugin(&hooks(), &hooks_path).unwrap();
        assert!(plugin.join("plugin.json").exists());
        assert!(plugin.join("mcp_config.json").exists());
        assert!(hooks_path.exists());
        assert_eq!(uninstall(&rec2, &hooks()), Ok(Uninstalled::Restored));
        assert!(!plugin.exists());
    }
}
