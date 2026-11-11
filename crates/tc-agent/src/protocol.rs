//! The message `ternarycode-hook` sends over the socket (one JSON object per line), and how each
//! agent's hook events map to a status signal (PRD section 8, adapter table).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tc_core::agent::AgentKind;
use tc_core::status::HookSignal;

/// Longest message kept from a payload.
pub const MAX_MESSAGE: usize = 240;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookMsg {
    pub pane: u64,
    pub token: String,
    /// Agent slug (`claude`, `codex`, …).
    pub agent: String,
    /// Event name as the agent calls it (`Stop`, `notify`, `session.idle`, …).
    pub event: String,
    /// Sub-kind: Claude's `notification_type`, Codex `notify`'s `type`, OpenCode's status.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

/// Maps an agent's hook event to a status signal; `None` for events that say nothing about
/// whether the user is needed.
pub fn classify(agent: AgentKind, event: &str, kind: Option<&str>) -> Option<HookSignal> {
    use HookSignal::*;
    match agent {
        AgentKind::Claude => match event {
            "SessionStart" => Some(SessionStart),
            "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure"
            | "SubagentStart" | "PreCompact" => Some(Working),
            "PermissionRequest" | "Elicitation" => Some(NeedsInput),
            "Notification" => match kind? {
                "permission_prompt"
                | "elicitation_dialog"
                | "elicitation_url_dialog"
                | "agent_needs_input" => Some(NeedsInput),
                "idle_prompt" | "agent_completed" => Some(TurnDone),
                _ => None,
            },
            "Stop" | "StopFailure" => Some(TurnDone),
            "SessionEnd" => Some(SessionEnd),
            _ => None,
        },
        AgentKind::Codex => match event {
            "SessionStart" => Some(SessionStart),
            "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "SubagentStart" => Some(Working),
            "PermissionRequest" => Some(NeedsInput),
            "Stop" => Some(TurnDone),
            "SessionEnd" => Some(SessionEnd),
            "notify" => match kind? {
                "agent-turn-complete" => Some(TurnDone),
                "approval-requested" => Some(NeedsInput),
                _ => None,
            },
            _ => None,
        },
        AgentKind::Gemini => match event {
            "SessionStart" => Some(SessionStart),
            "BeforeAgent" | "BeforeTool" | "AfterTool" | "BeforeModel" => Some(Working),
            "Notification" => Some(NeedsInput),
            "AfterAgent" => Some(TurnDone),
            "SessionEnd" => Some(SessionEnd),
            _ => None,
        },
        AgentKind::OpenCode => match event {
            "session.status" => match kind? {
                "busy" | "retry" => Some(Working),
                "idle" => Some(TurnDone),
                _ => None,
            },
            "session.idle" | "session.error" => Some(TurnDone),
            "permission.asked" | "permission.updated" => Some(NeedsInput),
            "permission.replied" => Some(Working),
            _ => None,
        },
        // A plain vocabulary for agents without their own (documented for custom presets).
        AgentKind::Antigravity | AgentKind::Custom => match event {
            "start" => Some(SessionStart),
            "working" => Some(Working),
            "needs-input" => Some(NeedsInput),
            "done" => Some(TurnDone),
            "end" => Some(SessionEnd),
            _ => None,
        },
    }
}

fn text(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string).filter(|s| !s.trim().is_empty())
}

fn shorten(s: String) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_MESSAGE {
        return one_line;
    }
    let mut out: String = one_line.chars().take(MAX_MESSAGE - 1).collect();
    out.push('…');
    out
}

/// The parts of a hook payload worth keeping: (kind, message, session, cwd). Only these leave
/// the hook process; prompts and transcripts never do.
pub fn extract(
    agent: AgentKind,
    event: &str,
    payload: &Value,
) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    let kind = text(&payload["notification_type"])
        .or_else(|| text(&payload["type"]))
        .or_else(|| text(&payload["status"]["type"]))
        .or_else(|| text(&payload["status"]));
    let message = match (agent, event) {
        // A permission request: which tool, and what it would do.
        (_, "PermissionRequest") | (AgentKind::Gemini, "Notification") => {
            let tool = text(&payload["tool_name"])
                .or_else(|| text(&payload["details"]["tool_name"]))
                .or_else(|| text(&payload["details"]["title"]));
            let input = &payload["tool_input"];
            let what = text(&input["command"])
                .or_else(|| text(&input["file_path"]))
                .or_else(|| text(&input["url"]))
                .or_else(|| text(&input["pattern"]));
            match (tool, what) {
                (Some(t), Some(w)) => Some(format!("{t}: {w}")),
                (Some(t), None) => Some(t),
                (None, _) => text(&payload["message"]),
            }
        }
        (AgentKind::Codex, "notify") => text(&payload["last-assistant-message"]),
        // A finished turn: the agent's last words, when the payload has them (a notification
        // that it is idle says nothing worth showing).
        (AgentKind::Claude, "Stop" | "StopFailure") => text(&payload["last_assistant_message"]),
        (AgentKind::Claude, "Notification")
            if matches!(text(&payload["notification_type"]).as_deref(), Some("idle_prompt" | "agent_completed")) =>
        {
            None
        }
        (AgentKind::OpenCode, _) => text(&payload["title"]).or_else(|| text(&payload["message"])),
        _ => text(&payload["message"]),
    }
    .map(shorten);
    let session = text(&payload["session_id"])
        .or_else(|| text(&payload["thread-id"]))
        .or_else(|| text(&payload["sessionID"]));
    let cwd = text(&payload["cwd"]);
    (kind, message, session, cwd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_events() {
        let c = |e, k| classify(AgentKind::Claude, e, k);
        assert_eq!(c("SessionStart", None), Some(HookSignal::SessionStart));
        assert_eq!(c("PreToolUse", None), Some(HookSignal::Working));
        assert_eq!(c("PermissionRequest", None), Some(HookSignal::NeedsInput));
        assert_eq!(c("Notification", Some("permission_prompt")), Some(HookSignal::NeedsInput));
        assert_eq!(c("Notification", Some("idle_prompt")), Some(HookSignal::TurnDone));
        assert_eq!(c("Notification", Some("auth_success")), None);
        assert_eq!(c("Notification", None), None);
        assert_eq!(c("Stop", None), Some(HookSignal::TurnDone));
        assert_eq!(c("SessionEnd", None), Some(HookSignal::SessionEnd));
        assert_eq!(c("FileChanged", None), None);
    }

    #[test]
    fn other_agents() {
        assert_eq!(
            classify(AgentKind::Codex, "notify", Some("agent-turn-complete")),
            Some(HookSignal::TurnDone)
        );
        assert_eq!(
            classify(AgentKind::Codex, "PermissionRequest", None),
            Some(HookSignal::NeedsInput)
        );
        assert_eq!(
            classify(AgentKind::Gemini, "AfterAgent", None),
            Some(HookSignal::TurnDone)
        );
        assert_eq!(
            classify(AgentKind::OpenCode, "permission.asked", None),
            Some(HookSignal::NeedsInput)
        );
        assert_eq!(
            classify(AgentKind::OpenCode, "session.status", Some("busy")),
            Some(HookSignal::Working)
        );
        assert_eq!(
            classify(AgentKind::Custom, "needs-input", None),
            Some(HookSignal::NeedsInput)
        );
    }

    #[test]
    fn extracts_only_what_is_needed() {
        let p = json!({
            "session_id": "abc",
            "transcript_path": "/secret/transcript.jsonl",
            "cwd": "/repo",
            "hook_event_name": "PermissionRequest",
            "tool_name": "Bash",
            "tool_input": {"command": "rm -rf target\n  && cargo build"}
        });
        let (kind, msg, session, cwd) = extract(AgentKind::Claude, "PermissionRequest", &p);
        assert_eq!(kind, None);
        assert_eq!(msg.as_deref(), Some("Bash: rm -rf target && cargo build"));
        assert_eq!(session.as_deref(), Some("abc"));
        assert_eq!(cwd.as_deref(), Some("/repo"));

        let n = json!({"notification_type": "permission_prompt", "message": "Claude needs your permission to use Bash"});
        let (kind, msg, ..) = extract(AgentKind::Claude, "Notification", &n);
        assert_eq!(kind.as_deref(), Some("permission_prompt"));
        assert_eq!(msg.as_deref(), Some("Claude needs your permission to use Bash"));

        let codex = json!({"type": "agent-turn-complete", "thread-id": "t1", "last-assistant-message": "x".repeat(1000)});
        let (kind, msg, session, _) = extract(AgentKind::Codex, "notify", &codex);
        assert_eq!(kind.as_deref(), Some("agent-turn-complete"));
        assert_eq!(msg.unwrap().chars().count(), MAX_MESSAGE);
        assert_eq!(session.as_deref(), Some("t1"));
    }
}
