//! `ternarycode-hook <agent> <event> [json]`: run by an agent's hook, it tells the TernaryCode
//! pane it runs in what happened, and always exits 0 quickly so the agent is never held up or
//! shown an error.
//!
//! Outside TernaryCode (no `TERNARYCODE_*` variables) it does nothing. The payload comes from
//! stdin (Claude Code, Codex hooks, Gemini CLI) or as the last argument (Codex `notify`, the
//! OpenCode plugin); only the fields in `protocol::extract` (and `protocol::prompt`'s one line) are sent on.

use std::io::{IsTerminal, Read, Write};
use std::time::Duration;

use serde_json::Value;
use tc_core::agent::AgentKind;

use crate::protocol::{HookMsg, extract};
use crate::{ENV_PANE, ENV_SOCK, ENV_TOKEN};

/// Largest payload read.
const MAX_PAYLOAD: u64 = 1024 * 1024;
const STDIN_WAIT: Duration = Duration::from_millis(300);
const SOCKET_TIMEOUT: Duration = Duration::from_millis(300);

/// Reads stdin when it is a pipe, without waiting longer than `STDIN_WAIT` for it.
fn read_stdin() -> Option<Vec<u8>> {
    if std::io::stdin().is_terminal() {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::stdin().take(MAX_PAYLOAD).read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx.recv_timeout(STDIN_WAIT).ok()
}

/// Builds the message for `args` = `[agent, event, json?]`, or `None` when this is not a
/// TernaryCode pane or the arguments make no sense.
pub fn message(
    args: &[String],
    env: impl Fn(&str) -> Option<String>,
    stdin: impl FnOnce() -> Option<Vec<u8>>,
) -> Option<(String, HookMsg)> {
    let sock = env(ENV_SOCK)?;
    let pane = env(ENV_PANE)?.parse().ok()?;
    let token = env(ENV_TOKEN)?;
    let agent_slug = args.first()?;
    let agent = AgentKind::from_slug(agent_slug)?;
    let mut event = args.get(1)?.clone();
    let raw = match args.get(2) {
        Some(json) => Some(json.clone().into_bytes()),
        None => stdin(),
    };
    let payload: Value = raw
        .and_then(|r| serde_json::from_slice(&r).ok())
        .unwrap_or(Value::Null);
    // Hooks configured with a generic event name learn the real one from the payload.
    if (event.is_empty() || event == "-")
        && let Some(e) = payload["hook_event_name"].as_str()
    {
        event = e.to_string();
    }
    let (kind, message, session, cwd) = extract(agent, &event, &payload);
    let prompt = crate::protocol::prompt(agent, &event, &payload);
    Some((
        sock,
        HookMsg {
            pane,
            token,
            agent: agent.slug().to_string(),
            event,
            kind,
            message,
            session,
            cwd,
            prompt,
        },
    ))
}

/// Writes `msg` to the pane's socket. Connecting can wait too (on Linux, while the app takes no
/// connections and its backlog is full), and has no timeout of its own: it runs on a thread
/// that is given up after `SOCKET_TIMEOUT` (and left behind).
#[cfg(unix)]
pub(crate) fn send(sock: &str, msg: &HookMsg) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(msg)?;
    line.push(b'\n');
    let sock = sock.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = std::os::unix::net::UnixStream::connect(&sock).and_then(|mut stream| {
            stream.set_write_timeout(Some(SOCKET_TIMEOUT))?;
            stream.write_all(&line)
        });
        let _ = tx.send(result);
    });
    rx.recv_timeout(SOCKET_TIMEOUT)
        .unwrap_or_else(|_| Err(std::io::ErrorKind::TimedOut.into()))
}

/// Writes `msg` to the pane's named pipe. Every instance of it can be busy for a moment (the
/// app makes the next one as a client connects): that is retried. A pipe that takes no input is
/// given up after `SOCKET_TIMEOUT` (the write runs on a thread this process leaves behind).
#[cfg(windows)]
pub(crate) fn send(sock: &str, msg: &HookMsg) -> std::io::Result<()> {
    use std::io::ErrorKind;
    const ERROR_PIPE_BUSY: i32 = 231;
    let mut line = serde_json::to_vec(msg)?;
    line.push(b'\n');
    let sock = sock.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let result = loop {
            match std::fs::OpenOptions::new().write(true).open(&sock) {
                Ok(mut pipe) => break pipe.write_all(&line),
                Err(e)
                    if (e.raw_os_error() == Some(ERROR_PIPE_BUSY) || e.kind() == ErrorKind::NotFound)
                        && start.elapsed() < SOCKET_TIMEOUT =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => break Err(e),
            }
        };
        let _ = tx.send(result);
    });
    rx.recv_timeout(SOCKET_TIMEOUT)
        .unwrap_or_else(|_| Err(ErrorKind::TimedOut.into()))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn send(_sock: &str, _msg: &HookMsg) -> std::io::Result<()> {
    Ok(())
}

/// Computes the JSON stdout response required by Antigravity CLI hooks.
/// `PreToolUse` requires a `"decision"` field (`"allow"` / `"deny"` / `"ask"`); returning `{}`
/// causes `agy` to deny the tool call with an empty reason, so non-denied `PreToolUse` invocations
/// ALWAYS return `{"decision":"allow"}` both inside and outside TernaryCode panes.
fn antigravity_hook_output(event: &str, in_pane: bool, tools: bool, payload: &Value) -> String {
    match event {
        "SessionStart" if in_pane && tools => serde_json::json!({
            "injectSteps": [{
                "systemMessage": {
                    "systemMessage": crate::integrate::ANTIGRAVITY_GUIDE,
                }
            }]
        })
        .to_string(),
        "PreToolUse" => {
            if in_pane {
                let prompt = || payload["lastUserInput"].as_str().map(str::to_string);
                if let Some(out) = crate::guard::decision(AgentKind::Antigravity, event, payload, tools, prompt) {
                    return out;
                }
            }
            r#"{"decision":"allow"}"#.to_string()
        }
        _ => "{}".to_string(),
    }
}

/// Entry point of `ternarycode-hook`; returns the exit code (always 0). `ternarycode-hook mcp`
/// is the agents' browser tools instead (`crate::mcp`).
pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("mcp") => return crate::mcp::run(&args[1..]),
        Some("browse") => return browse(&args[1..]),
        _ => {}
    }
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let is_antigravity = args.first().and_then(|a| AgentKind::from_slug(a)) == Some(AgentKind::Antigravity);
    // The payload comes from stdin unless it is the last argument; taken either way so the
    // agent's write succeeds.
    let stdin = if args.len() < 3 { read_stdin() } else { None };
    let raw = args.get(2).map(|j| j.clone().into_bytes()).or_else(|| stdin.clone());
    if let Some((sock, msg)) = message(args, env, || stdin) {
        let _ = send(&sock, &msg);
        if is_antigravity {
            let tools = crate::guard::tools_on(AgentKind::Antigravity, env);
            let payload: Value = raw.and_then(|r| serde_json::from_slice(&r).ok()).unwrap_or(Value::Null);
            println!("{}", antigravity_hook_output(&msg.event, true, tools, &payload));
            return 0;
        }
        // A call that would open a page in another browser: refused (Claude Code, Codex).
        if msg.event == "PreToolUse"
            && let Some(agent) = AgentKind::from_slug(&msg.agent)
            && matches!(agent, AgentKind::Claude | AgentKind::Codex)
        {
            let payload: Value = raw.and_then(|r| serde_json::from_slice(&r).ok()).unwrap_or(Value::Null);
            if crate::guard::other_browser(&payload).is_some() {
                let transcript = payload["transcript_path"].as_str().map(std::path::PathBuf::from);
                let tools = crate::guard::tools_on(agent, env);
                let prompt = || transcript.as_deref().and_then(crate::guard::last_prompt);
                if let Some(out) = crate::guard::decision(agent, &msg.event, &payload, tools, prompt) {
                    println!("{out}");
                }
            }
        }
    } else if is_antigravity {
        // Antigravity CLI hooks always expect a JSON object on stdout, even outside TernaryCode,
        // and `PreToolUse` specifically requires `"decision":"allow"` so tools are never blocked.
        let event = args.get(1).map(String::as_str).unwrap_or("");
        println!("{}", antigravity_hook_output(event, false, false, &Value::Null));
    }
    0
}

/// `ternarycode-hook browse <address>` (and `ternarycode browse`, and `$BROWSER` in panes):
/// the page opens in TernaryCode, in the browser of the project the terminal belongs to. Outside
/// a TernaryCode pane, or when TernaryCode leaves web pages to the default browser (or does not
/// answer), the default browser opens it. Returns the exit code.
pub fn browse(args: &[String]) -> i32 {
    let Some(url) = args.iter().find(|a| !a.starts_with('-')) else {
        eprintln!("usage: ternarycode browse <address>");
        return 2;
    };
    if let Some(pane) = crate::mcp::pane_from_env(None) {
        let reply = crate::mcp::call_app(&pane, "open", &serde_json::json!({ "url": url }));
        if reply["text"].is_string() {
            return 0;
        }
    }
    open_default(url)
}

/// Opens `url` in the system's default browser.
fn open_default_command(url: &str) -> Option<std::process::Command> {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    let allowed = lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") || lower.starts_with("file://");
    if !allowed || url.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return None;
    }
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("/usr/bin/open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    // Avoid infinite recursion on Linux where `xdg-open` falls back to `$BROWSER` (which points
    // to TernaryCode's `browse.js` wrapper -> `ternarycode-hook browse` -> `open_default`).
    cmd.env_remove("BROWSER").env_remove(ENV_SOCK).arg(url);
    Some(cmd)
}

fn open_default(url: &str) -> i32 {
    let Some(mut cmd) = open_default_command(url) else {
        return 1;
    };
    match cmd.status() {
        Ok(s) if s.success() => 0,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    const PANE_ENV: &[(&str, &str)] = &[
        (ENV_SOCK, "/tmp/tc.sock"),
        (ENV_PANE, "42"),
        (ENV_TOKEN, "secret"),
    ];

    #[test]
    fn builds_messages_from_stdin_or_argument() {
        let args: Vec<String> = ["claude", "Notification"].iter().map(|s| s.to_string()).collect();
        let (sock, m) = message(&args, env_of(PANE_ENV), || {
            Some(br#"{"notification_type":"permission_prompt","message":"Allow?","session_id":"s"}"#.to_vec())
        })
        .unwrap();
        assert_eq!(sock, "/tmp/tc.sock");
        assert_eq!((m.pane, m.token.as_str(), m.agent.as_str()), (42, "secret", "claude"));
        assert_eq!(m.kind.as_deref(), Some("permission_prompt"));
        assert_eq!(m.message.as_deref(), Some("Allow?"));

        // Codex notify: JSON as the last argument; stdin is not touched.
        let args: Vec<String> = ["codex", "notify", r#"{"type":"agent-turn-complete"}"#]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (_, m) = message(&args, env_of(PANE_ENV), || panic!("stdin read")).unwrap();
        assert_eq!(m.kind.as_deref(), Some("agent-turn-complete"));

        // The event name may come from the payload.
        let args: Vec<String> = ["gemini", "-"].iter().map(|s| s.to_string()).collect();
        let (_, m) = message(&args, env_of(PANE_ENV), || {
            Some(br#"{"hook_event_name":"AfterAgent"}"#.to_vec())
        })
        .unwrap();
        assert_eq!(m.event, "AfterAgent");
    }

    #[test]
    fn does_nothing_outside_ternarycode_or_with_bad_arguments() {
        let args: Vec<String> = ["claude", "Stop"].iter().map(|s| s.to_string()).collect();
        assert!(message(&args, env_of(&[]), || None).is_none());
        let bad: Vec<String> = ["vim", "Stop"].iter().map(|s| s.to_string()).collect();
        assert!(message(&bad, env_of(PANE_ENV), || None).is_none());
        assert!(message(&[], env_of(PANE_ENV), || None).is_none());
    }

    #[test]
    fn antigravity_pre_tool_use_always_returns_explicit_decision() {
        // Non-browser tools inside a pane must return decision=allow (empty `{}` denies in agy).
        let read = serde_json::json!({"toolCall": {"name": "read_file", "args": {"Path": "/tmp/a"}}});
        let out: Value = serde_json::from_str(&antigravity_hook_output("PreToolUse", true, true, &read)).unwrap();
        assert_eq!(out["decision"], "allow");

        // Outside TernaryCode panes, PreToolUse must still return decision=allow.
        let out_outside: Value = serde_json::from_str(&antigravity_hook_output("PreToolUse", false, false, &Value::Null)).unwrap();
        assert_eq!(out_outside["decision"], "allow");

        // Blocked external browser subagent inside a pane returns decision=deny with reason.
        let sub = serde_json::json!({
            "toolCall": {"name": "invoke_subagent", "args": {"TypeName": "browser", "Prompt": "open http://localhost:3000"}},
            "lastUserInput": "check the preview"
        });
        let deny: Value = serde_json::from_str(&antigravity_hook_output("PreToolUse", true, true, &sub)).unwrap();
        assert_eq!(deny["decision"], "deny");
        assert!(deny["reason"].as_str().unwrap().contains("ternarycode browser tools"));

        // Explicit Chrome request passes through with decision=allow.
        let sub_chrome = serde_json::json!({
            "toolCall": {"name": "invoke_subagent", "args": {"TypeName": "browser", "Prompt": "open http://localhost:3000"}},
            "lastUserInput": "buka di chrome"
        });
        let allow_chrome: Value = serde_json::from_str(&antigravity_hook_output("PreToolUse", true, true, &sub_chrome)).unwrap();
        assert_eq!(allow_chrome["decision"], "allow");

        // Non-PreToolUse events return `{}` when no injection is needed.
        assert_eq!(antigravity_hook_output("PostToolUse", true, true, &Value::Null), "{}");
        assert_eq!(antigravity_hook_output("Stop", true, true, &Value::Null), "{}");
        assert_eq!(antigravity_hook_output("SessionStart", false, false, &Value::Null), "{}");
    }

    #[test]
    fn open_default_strips_browser_and_socket_env_to_prevent_recursion() {
        let cmd = open_default_command("https://example.com").expect("valid URL");
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(envs.contains(&(std::ffi::OsStr::new("BROWSER"), None)));
        assert!(envs.contains(&(std::ffi::OsStr::new(ENV_SOCK), None)));
        assert!(open_default_command("javascript:alert(1)").is_none());
    }
}
