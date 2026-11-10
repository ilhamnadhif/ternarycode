//! `ternarycode-hook <agent> <event> [json]`: run by an agent's hook, it tells the TernaryCode
//! pane it runs in what happened, and always exits 0 quickly so the agent is never held up or
//! shown an error.
//!
//! Outside TernaryCode (no `TERNARYCODE_*` variables) it does nothing. The payload comes from
//! stdin (Claude Code, Codex hooks, Gemini CLI) or as the last argument (Codex `notify`, the
//! OpenCode plugin); only the fields in `protocol::extract` are sent on.

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

/// Entry point of `ternarycode-hook`; returns the exit code (always 0).
pub fn run(args: &[String]) -> i32 {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    match message(args, env, read_stdin) {
        Some((sock, msg)) => {
            let _ = send(&sock, &msg);
        }
        None => {
            // Not in a TernaryCode pane: still take the payload so the agent's write succeeds.
            let _ = read_stdin();
        }
    }
    0
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
}
