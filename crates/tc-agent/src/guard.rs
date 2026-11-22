//! Keeps an agent in a TernaryCode pane on TernaryCode's own browser. Claude Code's and Codex's
//! `PreToolUse` hook (`ternarycode-hook <agent> PreToolUse`) refuses a tool call that would open
//! a page in another browser: Chrome, Safari and the others (headless too), Playwright or
//! Puppeteer opening a page, a script that drives a browser, another browser tool. The agent
//! hears why and uses the ternarycode tools, where the user sees the page beside the terminal.
//! Not when the user named that browser in their latest prompt, nor when the agent has no
//! browser tools (Settings). A project's tests (`playwright test`, `npm test`) run as usual.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::{Value, json};
use tc_core::agent::AgentKind;

/// Another browser a call would use: its name for the agent, and the words that name it in a
/// prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Other {
    pub name: &'static str,
    words: &'static [&'static str],
}

const CHROME: Other = Other { name: "Chrome", words: &["chrome", "chromium", "cdp"] };
const SAFARI: Other = Other { name: "Safari", words: &["safari"] };
const FIREFOX: Other = Other { name: "Firefox", words: &["firefox"] };
const EDGE: Other = Other { name: "Edge", words: &["edge", "msedge"] };
const BRAVE: Other = Other { name: "Brave", words: &["brave"] };
const ARC: Other = Other { name: "Arc", words: &["arc"] };
const ZEN: Other = Other { name: "Zen", words: &["zen"] };
const OPERA: Other = Other { name: "Opera", words: &["opera"] };
const VIVALDI: Other = Other { name: "Vivaldi", words: &["vivaldi"] };
const PLAYWRIGHT: Other = Other { name: "Playwright", words: &["playwright"] };
const PUPPETEER: Other = Other { name: "Puppeteer", words: &["puppeteer"] };
const SELENIUM: Other = Other { name: "Selenium", words: &["selenium", "webdriver"] };
const BROWSER_TOOL: Other = Other { name: "another browser tool", words: &[] };
/// The system's opener given a web address (`start https://…`, `xdg-open …`).
const DEFAULT: Other = Other { name: "the default browser", words: &["default"] };

/// A web address as an opener is given it.
fn is_url(arg: &str) -> bool {
    let a = arg.to_lowercase();
    ["http://", "https://", "localhost:", "127.0.0.1:", "www."].iter().any(|p| a.starts_with(p))
}

/// Programs that open a web address in the system's default browser.
const OPENERS: &[&str] = &["xdg-open", "sensible-browser", "x-www-browser", "www-browser", "gnome-open", "kde-open", "kde-open5", "wslview", "explorer", "explorer.exe", "rundll32", "rundll32.exe"];

/// The browser a program (its file name, lowercase) is.
fn program_browser(base: &str) -> Option<Other> {
    let base = base.strip_suffix(".exe").unwrap_or(base);
    Some(match base {
        "google-chrome" | "google-chrome-stable" | "google-chrome-beta" | "google-chrome-unstable" | "chromium" | "chromium-browser" | "chrome" | "chromedriver" => CHROME,
        "msedge" | "microsoft-edge" | "microsoft-edge-stable" | "msedgedriver" => EDGE,
        "firefox" | "firefox-esr" | "geckodriver" => FIREFOX,
        "safaridriver" => SAFARI,
        "brave" | "brave-browser" => BRAVE,
        "zen" | "zen-browser" => ZEN,
        "opera" => OPERA,
        "vivaldi" => VIVALDI,
        _ => return None,
    })
}

/// The browser an app's name or bundle id (lowercase, as in `Google Chrome.app`, `open -a
/// Safari`, `com.google.Chrome`) is.
fn app_browser(name: &str) -> Option<Other> {
    if name.contains("edgemac") {
        return Some(EDGE);
    }
    if name.contains("thebrowser") {
        return Some(ARC);
    }
    let has = |w: &str| name.split(|c: char| !c.is_alphanumeric()).any(|x| x == w);
    [
        ("chrome", CHROME),
        ("chromium", CHROME),
        ("safari", SAFARI),
        ("firefox", FIREFOX),
        ("edge", EDGE),
        ("brave", BRAVE),
        ("arc", ARC),
        ("zen", ZEN),
        ("opera", OPERA),
        ("vivaldi", VIVALDI),
    ]
    .into_iter()
    .find(|(w, _)| has(w))
    .map(|(_, b)| b)
}

/// Code (a script's text, lowercase) that opens pages in a browser of its own.
fn code_browser(code: &str) -> Option<Other> {
    if code.contains("remote-debugging-port") || code.contains("chrome-remote-interface") {
        return Some(CHROME);
    }
    if code.contains("puppeteer") {
        return Some(PUPPETEER);
    }
    // A Playwright test file runs with the project's tests.
    if code.contains("playwright") && !code.contains("@playwright/test") {
        return Some(PLAYWRIGHT);
    }
    if code.contains("selenium") {
        return Some(SELENIUM);
    }
    None
}

/// An MCP tool (`mcp__<server>__<tool>`) of a browser other than TernaryCode's.
fn mcp_browser(server: &str, tool: &str) -> Option<Other> {
    if server == "ternarycode" {
        return None;
    }
    let s = format!("{server} {tool}").to_lowercase();
    if s.contains("playwright") {
        return Some(PLAYWRIGHT);
    }
    if s.contains("puppeteer") {
        return Some(PUPPETEER);
    }
    if s.contains("selenium") {
        return Some(SELENIUM);
    }
    let server = server.to_lowercase();
    app_browser(&server).or_else(|| server.contains("browser").then_some(BROWSER_TOOL))
}

/// The simple commands of a command line: split at `;`, `|`, `&`, line breaks, parentheses and
/// backquotes outside quotes.
fn segments(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in command.chars() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                cur.push(c);
            }
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                cur.push(c);
            }
            None if ";|&\n()`".contains(c) => out.push(std::mem::take(&mut cur)),
            None => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// The words of a shell command, quotes taken off (good enough to find its program).
fn words(segment: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in segment.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                any = true;
            }
            None if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            None => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

const WRAPPERS: &[&str] = &["env", "exec", "nohup", "sudo", "command", "time", "nice", "caffeinate", "xvfb-run"];
const INTERPRETERS: &[&str] = &["node", "python", "python3", "deno", "bun", "tsx", "ts-node", "ruby"];

/// The browser a shell command would open a page in, run in `cwd` (scripts it runs are read).
fn command_browser(command: &str, cwd: Option<&Path>) -> Option<Other> {
    // Where the command is when it runs each part (`cd` moves it).
    let mut dir = cwd.map(Path::to_path_buf);
    for segment in segments(command) {
        let w = words(&segment);
        let mut i = 0;
        while i < w.len() {
            let x = w[i].as_str();
            // `VAR=value` before the program, and programs that run the next one (with their options).
            if x.contains('=') && !x.starts_with('-') && !x.contains('/') {
                i += 1;
            } else if WRAPPERS.contains(&x) {
                i += 1;
                while i < w.len() && w[i].starts_with('-') {
                    let opt = w[i].as_str();
                    i += 1;
                    if opt == "--" {
                        break;
                    }
                    let takes_val = matches!(opt, "-u" | "-g" | "-n" | "-C" | "-a" | "-t" | "-w" | "-e" | "-f" | "-p" | "-s" | "--unset" | "--chdir" | "--user" | "--group" | "--adjustment");
                    if takes_val && i < w.len() {
                        i += 1;
                    }
                }
            } else if x == "timeout" {
                i += 1;
                while i < w.len() && w[i].starts_with('-') {
                    let opt = w[i].as_str();
                    i += 1;
                    if opt == "--" {
                        break;
                    }
                    if matches!(opt, "-k" | "-s" | "--kill-after" | "--signal") && i < w.len() {
                        i += 1;
                    }
                }
                // Duration argument after timeout options.
                if i < w.len() {
                    i += 1;
                }
            } else {
                break;
            }
        }
        let Some(program) = w.get(i) else { continue };
        let args = &w[i + 1..];
        let lower = program.to_lowercase();
        let base = lower.rsplit(['/', '\\']).next().unwrap_or(&lower).to_string();
        let lc = |k: usize| args.get(k).map(|a| a.to_lowercase()).unwrap_or_default();
        if base == "cd" {
            if let Some(to) = args.first() {
                let to = match (to.strip_prefix("~/"), std::env::var_os("HOME")) {
                    (Some(rest), Some(home)) => Path::new(&home).join(rest),
                    _ => Path::new(to).to_path_buf(),
                };
                dir = Some(match &dir {
                    Some(d) if to.is_relative() => d.join(to),
                    _ => to,
                });
            }
            continue;
        }
        // A browser run by its own program, or its app's (`…/Google Chrome.app/Contents/MacOS/…`).
        if let Some(b) = program_browser(&base) {
            return Some(b);
        }
        if let Some(app) = lower.split(".app/contents/").next().filter(|_| lower.contains(".app/contents/"))
            && let Some(b) = app_browser(app.rsplit('/').next().unwrap_or(app))
        {
            return Some(b);
        }
        // macOS `open -a Safari …` / `open -b com.google.Chrome …`, or `open https://…`.
        if base == "open" {
            for (k, a) in args.iter().enumerate() {
                if (a == "-a" || a == "-b" || a == "--bundle-identifier") && let Some(b) = app_browser(&lc(k + 1)) {
                    return Some(b);
                }
            }
            if args.iter().any(|a| is_url(a)) {
                return Some(DEFAULT);
            }
        }
        // The system's opener given a web address (`xdg-open`, `gio open`, `explorer`, `rundll32
        // url.dll,…`, `python -m webbrowser`): the default browser.
        if (OPENERS.contains(&base.as_str()) || (base == "gio" && lc(0) == "open")) && args.iter().any(|a| is_url(a)) {
            return Some(DEFAULT);
        }
        if INTERPRETERS.contains(&base.as_str()) && args.windows(2).any(|p| p[0] == "-m" && p[1] == "webbrowser") {
            return Some(DEFAULT);
        }
        // Windows `start chrome …`, `Start-Process msedge …`, `cmd /c start "" https://…`.
        let is_shell_cmd = matches!(base.as_str(), "cmd" | "cmd.exe" | "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe");
        let start_at = if base == "start" || base == "start-process" {
            Some(i)
        } else if is_shell_cmd {
            w[i + 1..].iter().position(|a| {
                let a = a.to_lowercase();
                a == "start" || a == "start-process"
            }).map(|p| i + 1 + p)
        } else {
            None
        };
        if let Some(k) = start_at {
            let next = w[k + 1..].iter().find(|x| !x.is_empty() && !x.starts_with('/') && !x.starts_with('-'));
            if let Some(next) = next {
                let lower = next.to_lowercase();
                let nb = lower.rsplit(['/', '\\']).next().unwrap_or(&lower).to_string();
                if let Some(b) = program_browser(&nb) {
                    return Some(b);
                }
                if is_url(next) {
                    return Some(DEFAULT);
                }
            }
        }
        // The Playwright command line opening a page (its tests and installs are fine).
        if let Some(k) = w.iter().position(|a| a.to_lowercase().rsplit(['/', '\\']).next() == Some("playwright"))
            && matches!(w.get(k + 1).map(|s| s.to_lowercase()).as_deref(), Some("open" | "codegen" | "screenshot" | "pdf" | "cr" | "ff" | "wk"))
        {
            return Some(PLAYWRIGHT);
        }
        // A script that drives a browser: given inline, or a file the interpreter runs.
        if INTERPRETERS.contains(&base.as_str()) {
            if let Some(k) = args.iter().position(|a| a == "-e" || a == "-c" || a == "--eval" || a == "-p")
                && let Some(b) = code_browser(&lc(k + 1))
            {
                return Some(b);
            }
            let file = args.iter().find(|a| !a.starts_with('-') && *a != "run");
            if let Some(file) = file {
                let path = Path::new(file);
                let path = match &dir {
                    Some(d) if path.is_relative() => d.join(path),
                    _ => path.to_path_buf(),
                };
                let name = path.to_string_lossy().to_lowercase();
                let test = ["test", "spec"].iter().any(|t| name.rsplit(['/', '\\']).next().unwrap_or(&name).contains(t));
                if !test
                    && std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() < 512 * 1024)
                    && let Ok(text) = std::fs::read_to_string(&path)
                    && let Some(b) = code_browser(&text.to_lowercase())
                {
                    return Some(b);
                }
            }
        }
    }
    None
}

/// The other browser a tool call (a `PreToolUse` payload) would use.
pub fn other_browser(payload: &Value) -> Option<Other> {
    let tool = payload["tool_name"].as_str().unwrap_or("");
    if let Some(rest) = tool.strip_prefix("mcp__") {
        let (server, name) = rest.split_once("__").unwrap_or((rest, ""));
        return mcp_browser(server, name);
    }
    let input = &payload["tool_input"];
    let command = match &input["command"] {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        _ => return None,
    };
    let cwd = payload["cwd"].as_str().map(Path::new);
    command_browser(&command, cwd)
}

/// The user's latest prompt in a transcript (Claude Code's or Codex's JSONL), read from its end.
pub fn last_prompt(transcript: &Path) -> Option<String> {
    const TAIL: u64 = 512 * 1024;
    let mut f = std::fs::File::open(transcript).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut buf = Vec::new();
    f.take(TAIL).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    text.lines().rev().find_map(|line| prompt_of(&serde_json::from_str(line).ok()?))
}

/// A transcript line that is the user's own prompt: its text.
fn prompt_of(v: &Value) -> Option<String> {
    let real = |s: &str| {
        let t = s.trim();
        (!t.is_empty() && !t.starts_with('<')).then(|| t.to_string())
    };
    // Claude Code: {"type":"user","message":{"content":"…" | [{"type":"text","text":"…"}, …]}}
    // (a tool's result comes back as a user line too: not a prompt).
    if v["type"] == "user" && v["isMeta"] != true {
        return match &v["message"]["content"] {
            Value::String(s) => real(s),
            Value::Array(a) if a.iter().all(|p| p["type"] != "tool_result") => {
                a.iter().filter(|p| p["type"] == "text").filter_map(|p| p["text"].as_str()).find_map(real)
            }
            _ => None,
        };
    }
    // Codex: {"type":"response_item","payload":{"type":"message","role":"user","content":
    // [{"type":"input_text","text":"…"}]}} (its environment's context comes so too, in <…>), or
    // {"type":"event_msg","payload":{"type":"user_message","message":"…"}}.
    let p = &v["payload"];
    if v["type"] == "response_item" && p["type"] == "message" && p["role"] == "user" {
        return p["content"].as_array()?.iter().filter(|c| c["type"] == "input_text").filter_map(|c| c["text"].as_str()).find_map(real);
    }
    if v["type"] == "event_msg" && p["type"] == "user_message" {
        return p["message"].as_str().and_then(real);
    }
    None
}

/// Whether `prompt` names `browser`.
fn names(prompt: &str, browser: Other) -> bool {
    let lower = prompt.to_lowercase();
    lower.split(|c: char| !c.is_alphanumeric()).any(|w| browser.words.contains(&w))
}

/// What the hook prints for a `PreToolUse` call of `agent` that would use another browser: the
/// agent is refused, and told why. `tools`: the agent has TernaryCode's browser tools.
/// `prompt`: the user's latest prompt (read only when needed).
pub fn decision(agent: AgentKind, event: &str, payload: &Value, tools: bool, prompt: impl FnOnce() -> Option<String>) -> Option<String> {
    if event != "PreToolUse" || !tools || !matches!(agent, AgentKind::Claude | AgentKind::Codex) {
        return None;
    }
    let other = other_browser(payload)?;
    if prompt().is_some_and(|p| names(&p, other)) {
        return None;
    }
    let reason = format!(
        "TernaryCode keeps web pages in its own browser, beside this terminal, where the user sees them: use the ternarycode browser tools (browser_navigate first, browser_take_screenshot for a picture of the page) instead of {}. Use {} only when the user names it in their request.",
        other.name, other.name
    );
    Some(json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason } }).to_string())
}

/// Whether the pane's agent has TernaryCode's browser tools, from the pane's variables (as the
/// agent was started with them): Claude Code's `--mcp-config` file names the server; Codex's
/// override does not disable it.
pub fn tools_on(agent: AgentKind, env: impl Fn(&str) -> Option<String>) -> bool {
    match agent {
        AgentKind::Claude => env(crate::integrate::ENV_CLAUDE_MCP)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .is_some_and(|t| t.contains("\"ternarycode\"")),
        AgentKind::Codex => env(crate::integrate::ENV_CODEX_MCP).is_some_and(|v| v.contains("ternarycode") && !v.contains("enabled=false")),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash(cmd: &str) -> Value {
        json!({"tool_name": "Bash", "tool_input": {"command": cmd}})
    }

    fn other(cmd: &str) -> Option<&'static str> {
        other_browser(&bash(cmd)).map(|b| b.name)
    }

    #[test]
    fn finds_other_browsers_in_commands() {
        assert_eq!(other("\"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome\" --headless --screenshot https://x.dev"), Some("Chrome"));
        assert_eq!(other("cd /tmp && google-chrome --headless=new --dump-dom https://x.dev"), Some("Chrome"));
        assert_eq!(other("open -a Safari https://x.dev"), Some("Safari"));
        assert_eq!(other("open -a 'Google Chrome' https://x.dev"), Some("Chrome"));
        assert_eq!(other("open -b com.microsoft.edgemac https://x.dev"), Some("Edge"));
        assert_eq!(other("npx playwright screenshot https://x.dev shot.png"), Some("Playwright"));
        assert_eq!(other("FOO=1 nohup chromium --remote-debugging-port=9222 &"), Some("Chrome"));
        assert_eq!(other("cmd /c start chrome https://x.dev"), Some("Chrome"));
        assert_eq!(other("Start-Process msedge https://x.dev"), Some("Edge"));
        assert_eq!(other("node -e \"require('puppeteer').launch()\""), Some("Puppeteer"));
        assert_eq!(other("python3 -c 'from playwright.sync_api import sync_playwright'"), Some("Playwright"));
        // The system's opener given a web address: the default browser, outside TernaryCode.
        for cmd in [
            "open https://x.dev",
            "xdg-open http://localhost:3000",
            "gio open https://x.dev",
            "start https://x.dev",
            "cmd /c start \"\" https://x.dev",
            "Start-Process 'https://x.dev'",
            "explorer.exe https://x.dev",
            "rundll32 url.dll,FileProtocolHandler https://x.dev",
            "python3 -m webbrowser https://x.dev",
        ] {
            assert_eq!(other(cmd), Some("the default browser"), "{cmd}");
        }
    }

    #[test]
    fn leaves_other_commands_alone() {
        for cmd in [
            "open .",
            "open README.md",
            "xdg-open report.pdf",
            "explorer .",
            "Start-Process notepad",
            "npm start",
            "npx playwright test",
            "npx playwright install chromium",
            "npm test",
            "ls ~/.cache/chromium && rm -rf build",
            "grep -r chrome src/",
            "cargo build --release",
            "curl -s https://x.dev",
            "echo 'open -a Safari'",
        ] {
            assert_eq!(other(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn reads_the_scripts_it_runs() {
        let dir = std::env::temp_dir().join(format!("tc-guard-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("ref")).unwrap();
        std::fs::write(dir.join("ref/cdp.mjs"), "spawn(chrome, ['--headless=new', '--remote-debugging-port=0'])").unwrap();
        std::fs::write(dir.join("build.mjs"), "console.log('hi')").unwrap();
        std::fs::write(dir.join("login.spec.ts"), "import { test } from '@playwright/test'").unwrap();
        let at = |cmd: &str| other_browser(&json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "cwd": dir.to_string_lossy()})).map(|b| b.name);
        // As it was run: in a folder `cd` went to, several at once in a subshell.
        assert_eq!(at("cd ref && export S=$PWD; (node cdp.mjs https://orca.dev orca & node cdp.mjs https://t3.codes t3)"), Some("Chrome"));
        assert_eq!(at("node cdp.mjs https://orca.dev"), None);
        assert_eq!(at("node build.mjs"), None);
        assert_eq!(at("node login.spec.ts"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_other_browser_tools() {
        let mcp = |t: &str| other_browser(&json!({"tool_name": t, "tool_input": {}})).map(|b| b.name);
        assert_eq!(mcp("mcp__claude-in-chrome__navigate"), Some("Chrome"));
        assert_eq!(mcp("mcp__zen__computer"), Some("Zen"));
        assert_eq!(mcp("mcp__playwright__browser_navigate"), Some("Playwright"));
        assert_eq!(mcp("mcp__browserbase__open"), Some("another browser tool"));
        assert_eq!(mcp("mcp__ternarycode__browser_navigate"), None);
        assert_eq!(mcp("mcp__github__search"), None);
        assert_eq!(mcp("WebFetch"), None);
    }

    #[test]
    fn refuses_unless_the_user_named_it() {
        let p = bash("open -a 'Google Chrome' https://x.dev");
        let d = decision(AgentKind::Claude, "PreToolUse", &p, true, || Some("ambil screenshot situs referensi".into())).unwrap();
        let v: Value = serde_json::from_str(&d).unwrap();
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(v["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().contains("ternarycode browser tools"));
        assert!(decision(AgentKind::Codex, "PreToolUse", &p, true, || None).is_some());
        // Named by the user: allowed.
        assert!(decision(AgentKind::Claude, "PreToolUse", &p, true, || Some("buka lewat Google Chrome ya".into())).is_none());
        // No browser tools, another event, another agent: nothing said.
        assert!(decision(AgentKind::Claude, "PreToolUse", &p, false, || None).is_none());
        assert!(decision(AgentKind::Claude, "PostToolUse", &p, true, || None).is_none());
        assert!(decision(AgentKind::Gemini, "PreToolUse", &p, true, || None).is_none());
        // "Open it in my default browser" lets the system's opener through.
        let start = bash("start https://x.dev");
        assert!(decision(AgentKind::Codex, "PreToolUse", &start, true, || Some("buka di default browser saya".into())).is_none());
        assert!(decision(AgentKind::Codex, "PreToolUse", &start, true, || Some("open the shop".into())).is_some());
        // "search" is not Arc.
        let arc = bash("open -a Arc https://x.dev");
        assert!(decision(AgentKind::Claude, "PreToolUse", &arc, true, || Some("search the docs".into())).is_some());
    }

    #[test]
    fn finds_the_latest_prompt() {
        let dir = std::env::temp_dir().join(format!("tc-guard-t-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let claude = dir.join("c.jsonl");
        std::fs::write(
            &claude,
            [
                json!({"type": "user", "message": {"role": "user", "content": "first prompt"}}),
                json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "ok"}]}}),
                json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "pakai safari"}]}}),
                json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "content": "x"}]}}),
                json!({"type": "user", "isMeta": true, "message": {"role": "user", "content": "meta"}}),
            ]
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
        )
        .unwrap();
        assert_eq!(last_prompt(&claude).as_deref(), Some("pakai safari"));
        let codex = dir.join("x.jsonl");
        std::fs::write(
            &codex,
            [
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "open it in firefox"}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "assistant"}}),
            ]
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
        )
        .unwrap();
        assert_eq!(last_prompt(&codex).as_deref(), Some("open it in firefox"));
        // Codex 0.162: the prompt as a response item, after the environment's context.
        let rollout = dir.join("r.jsonl");
        std::fs::write(
            &rollout,
            [
                json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "<environment_context>cwd</environment_context>"}]}}),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "screenshot it in safari"}]}}),
                json!({"type": "response_item", "payload": {"type": "function_call", "name": "exec"}}),
            ]
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
        )
        .unwrap();
        assert_eq!(last_prompt(&rollout).as_deref(), Some("screenshot it in safari"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrapper_flags_and_start_positions() {
        assert_eq!(command_browser("echo start chrome", None), None);
        assert!(command_browser("sudo -u root google-chrome https://x.dev", None).is_some());
        assert!(command_browser("timeout -s KILL 5 firefox https://x.dev", None).is_some());
        assert!(command_browser("cmd /c start https://x.dev", None).is_some());
    }
}
