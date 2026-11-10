//! AI coding agents: which ones TernaryCode knows, their presets, and recognising one from the
//! foreground process of a pane.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Claude,
    Codex,
    OpenCode,
    Gemini,
    Antigravity,
    /// A program the user marked as an agent in a custom preset.
    Custom,
}

impl AgentKind {
    pub const ALL: [AgentKind; 6] = [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::OpenCode,
        AgentKind::Gemini,
        AgentKind::Antigravity,
        AgentKind::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude Code",
            AgentKind::Codex => "Codex",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Gemini => "Gemini CLI",
            AgentKind::Antigravity => "Antigravity CLI",
            AgentKind::Custom => "Agent",
        }
    }

    /// The name used on the command line of `ternarycode hook <agent> <event>`.
    pub fn slug(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::OpenCode => "opencode",
            AgentKind::Gemini => "gemini",
            AgentKind::Antigravity => "antigravity",
            AgentKind::Custom => "custom",
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.slug() == s)
    }
}

/// A way to start an agent (or any program) in a new pane.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub agent: Option<AgentKind>,
    /// Typed into the pane's shell, as the user would.
    pub command: String,
    /// Typed by the Resume button after a restart (`claude --continue`); none: the command.
    #[serde(default)]
    pub resume: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Ships with the app (it can be edited but not deleted).
    #[serde(default)]
    pub builtin: bool,
}

/// Presets every installation starts with. Resume commands were checked against the installed
/// CLIs on 9 Nov 2026 (Claude Code 2.1.319, Codex 0.162.1, Gemini CLI 0.43.0, OpenCode 1.19.5).
pub fn builtin_presets() -> Vec<Preset> {
    let p = |id: &str, name: &str, agent, command: &str, resume: &str| Preset {
        id: id.into(),
        name: name.into(),
        agent: Some(agent),
        command: command.into(),
        resume: Some(resume.into()),
        env: BTreeMap::new(),
        builtin: true,
    };
    vec![
        p(
            "claude",
            "Claude Code",
            AgentKind::Claude,
            "claude",
            "claude --continue",
        ),
        p(
            "codex",
            "Codex",
            AgentKind::Codex,
            "codex",
            "codex resume --last",
        ),
        p(
            "opencode",
            "OpenCode",
            AgentKind::OpenCode,
            "opencode",
            "opencode --continue",
        ),
        p(
            "gemini",
            "Gemini CLI",
            AgentKind::Gemini,
            "gemini",
            "gemini --resume latest",
        ),
    ]
}

/// The file name in a path, also a Windows one, without `.exe`.
fn basename(s: &str) -> &str {
    let name = s.rsplit(['/', '\\']).next().unwrap_or(s);
    match name.len().checked_sub(4) {
        Some(i) if name.is_char_boundary(i) && name[i..].eq_ignore_ascii_case(".exe") => &name[..i],
        _ => name,
    }
}

/// Which agent a process is, from its argument vector and executable path. Some agents run as
/// `node`/`bun` scripts, so the script path is looked at too; Claude Code's native build runs
/// from `…/claude/versions/<version>`.
pub fn recognize(argv: &[String], exe: Option<&str>) -> Option<AgentKind> {
    let by_name = |name: &str| -> Option<AgentKind> {
        match name {
            "claude" => Some(AgentKind::Claude),
            "codex"
            | "codex-aarch64-apple-darwin"
            | "codex-x86_64-apple-darwin"
            | "codex-x86_64-pc-windows-msvc"
            | "codex-aarch64-pc-windows-msvc" => {
                Some(AgentKind::Codex)
            }
            "opencode" => Some(AgentKind::OpenCode),
            "gemini" => Some(AgentKind::Gemini),
            "agy" | "antigravity" => Some(AgentKind::Antigravity),
            _ => None,
        }
    };
    let by_path = |path: &str| -> Option<AgentKind> {
        let path = path.replace('\\', "/");
        if path.contains("/@anthropic-ai/claude-code/") || path.contains("/claude/versions/") {
            Some(AgentKind::Claude)
        } else if path.contains("/@openai/codex/") {
            Some(AgentKind::Codex)
        } else if path.contains("/@google/gemini-cli/") {
            Some(AgentKind::Gemini)
        } else if path.contains("/opencode-ai/") || path.contains("/opencode/") {
            Some(AgentKind::OpenCode)
        } else {
            None
        }
    };
    let first = argv.first().map(|s| basename(s.trim_start_matches('-')));
    if let Some(a) = first.and_then(by_name) {
        return Some(a);
    }
    if let Some(a) = exe.and_then(by_path) {
        return Some(a);
    }
    // Interpreters: the script is the first argument that is not an option.
    if matches!(first, Some("node" | "bun" | "deno" | "nodejs")) {
        for arg in argv.iter().skip(1).filter(|a| !a.starts_with('-')).take(2) {
            if let Some(a) = by_name(basename(arg)).or_else(|| by_path(arg)) {
                return Some(a);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn recognizes_agents_however_they_are_installed() {
        assert_eq!(recognize(&v(&["claude"]), None), Some(AgentKind::Claude));
        // Windows: native builds and npm installs run through node.exe.
        assert_eq!(
            recognize(&v(&[r"C:\Users\me\.local\bin\claude.exe"]), None),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            recognize(
                &v(&[
                    r"C:\Program Files\nodejs\node.exe",
                    r"C:\Users\me\AppData\Roaming\npm\node_modules\@openai\codex\bin\codex.js"
                ]),
                None
            ),
            Some(AgentKind::Codex)
        );
        // Native build: argv[0] "claude", executable under versions/.
        assert_eq!(
            recognize(
                &v(&["claude", "--continue"]),
                Some("/Users/me/.local/share/claude/versions/2.1.319")
            ),
            Some(AgentKind::Claude)
        );
        // npm install, run through node.
        assert_eq!(
            recognize(
                &v(&[
                    "node",
                    "/opt/homebrew/lib/node_modules/@anthropic-ai/claude-code/cli.js"
                ]),
                Some("/opt/homebrew/bin/node")
            ),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            recognize(&v(&["/opt/homebrew/bin/codex", "resume"]), None),
            Some(AgentKind::Codex)
        );
        assert_eq!(
            recognize(
                &v(&["node", "--no-warnings", "/usr/lib/node_modules/@google/gemini-cli/dist/index.js"]),
                None
            ),
            Some(AgentKind::Gemini)
        );
        assert_eq!(recognize(&v(&["opencode"]), None), Some(AgentKind::OpenCode));
        // A login shell's argv[0] starts with '-'.
        assert_eq!(recognize(&v(&["-zsh"]), Some("/bin/zsh")), None);
        assert_eq!(recognize(&v(&["cargo", "build"]), None), None);
        assert_eq!(recognize(&v(&["node", "server.js"]), None), None);
    }

    #[test]
    fn slugs_round_trip_and_presets_are_complete() {
        for a in AgentKind::ALL {
            assert_eq!(AgentKind::from_slug(a.slug()), Some(a));
        }
        let presets = builtin_presets();
        assert_eq!(presets.len(), 4);
        assert!(presets.iter().all(|p| p.resume.is_some() && p.builtin));
    }
}
