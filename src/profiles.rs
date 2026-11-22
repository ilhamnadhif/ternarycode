//! Multi-account agent profiles (`claude1`, `claude2`, `chatgpt1`, `gemini1`, …), real usage
//! telemetry, rate-limit detection, failover chain selection, and project-nested session history.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use tc_core::agent::AgentKind;
use tc_core::model::{AgentProfile, FailoverSettings};
use tc_core::status::Status;
use tc_core::{PaneId, ProjectId};

/// Real-time execution state of a configured agent account profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProfileState {
    #[default]
    Idle,
    Running,
    RateLimited,
    Cooldown,
    Error,
}

impl ProfileState {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::RateLimited => "rate_limited",
            Self::Cooldown => "cooldown",
            Self::Error => "error",
        }
    }
}

/// Async connection & auth health check state for one profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionTest {
    Testing,
    Ok(String),
    Err(String),
}

/// Live runtime telemetry for one profile (not persisted; rebuilt from panes & session files).
#[derive(Clone, Debug, Default)]
pub struct ProfileRuntime {
    pub state: ProfileState,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub reset_at: Option<Instant>,
    pub reset_reason: Option<String>,
    pub last_error: Option<String>,
    pub active_panes: Vec<(PaneId, ProjectId)>,
    pub connection: Option<ConnectionTest>,
    pub usage_updated: Option<Instant>,
}

impl ProfileRuntime {
    /// Remaining quota percentage if a budget is configured (or `Some(0.0)` while rate-limited).
    pub fn quota_remaining_pct(&self, budget: Option<u64>) -> Option<f32> {
        if self.state == ProfileState::RateLimited {
            return Some(0.0);
        }
        let budget = budget.filter(|&b| b > 0)?;
        let used = self.tokens_in.saturating_add(self.tokens_out);
        let pct = 100.0 * (1.0 - (used as f64 / budget as f64)).clamp(0.0, 1.0);
        Some(pct as f32)
    }

    /// Seconds remaining until rate-limit / cooldown reset, if any.
    pub fn reset_remaining_secs(&self, now: Instant) -> Option<u64> {
        self.reset_at
            .and_then(|t| t.checked_duration_since(now))
            .map(|d| d.as_secs())
            .filter(|&s| s > 0)
    }

    pub fn is_healthy(&self, now: Instant) -> bool {
        match self.state {
            ProfileState::RateLimited | ProfileState::Cooldown => {
                self.reset_at.is_some_and(|t| now >= t)
            }
            ProfileState::Error => false,
            ProfileState::Idle | ProfileState::Running => true,
        }
    }
}

/// Inline recovery / failover banner shown inside a pane when an account hits its rate limit.
#[derive(Clone, Debug)]
pub struct RateLimitBanner {
    pub from_profile: String,
    pub agent: AgentKind,
    pub reason: String,
    pub reset_at: Instant,
    pub session_id: Option<String>,
    /// Populated when Auto-Resume already failed over to another profile (brief notice).
    pub auto_switched_to: Option<String>,
}

/// One structured entry in the Agent Logs drawer & Monitor event feed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Err,
    Failover,
    Tool,
}

#[derive(Clone, Debug)]
pub struct AgentLogEntry {
    pub at: chrono::DateTime<chrono::Local>,
    pub project: Option<ProjectId>,
    pub pane: Option<PaneId>,
    pub profile: Option<String>,
    pub level: LogLevel,
    pub message: String,
}

/// One session row rendered directly inside an expanded project node in the sidebar tree.
#[derive(Clone, Debug)]
pub struct ProjectSessionItem {
    pub id: String,
    pub title: String,
    pub agent: AgentKind,
    pub profile_id: Option<String>,
    pub status: Status,
    pub updated: SystemTime,
    pub branch: Option<String>,
    pub live_pane: Option<PaneId>,
}

/// Built-in default starter profiles when the user has not customized `settings.profiles` yet.
/// Each default profile uses the provider's standard auth folder so existing logins work instantly.
pub fn default_profiles() -> Vec<AgentProfile> {
    vec![
        AgentProfile {
            id: "claude1".into(),
            name: "Claude 1".into(),
            agent: AgentKind::Claude,
            config_dir: None,
            api_key: None,
            extra_env: BTreeMap::new(),
            fallback_order: 1,
            enabled: true,
            daily_token_budget: None,
            use_default_auth: true,
        },
        AgentProfile {
            id: "chatgpt1".into(),
            name: "ChatGPT / Codex 1".into(),
            agent: AgentKind::Codex,
            config_dir: None,
            api_key: None,
            extra_env: BTreeMap::new(),
            fallback_order: 2,
            enabled: true,
            daily_token_budget: None,
            use_default_auth: true,
        },
        AgentProfile {
            id: "gemini1".into(),
            name: "Gemini 1".into(),
            agent: AgentKind::Gemini,
            config_dir: None,
            api_key: None,
            extra_env: BTreeMap::new(),
            fallback_order: 3,
            enabled: true,
            daily_token_budget: None,
            use_default_auth: true,
        },
    ]
}

/// Resolves the config/auth root folder for `profile`.
/// - Explicit `profile.config_dir` wins.
/// - `use_default_auth` maps to `~/.claude`, `~/.codex`, `~/.gemini`, etc.
/// - Otherwise isolates cleanly into `<data_dir>/profiles/<sanitized_id>`.
pub fn resolved_config_dir(profile: &AgentProfile, home: &Path, data_dir: &Path) -> PathBuf {
    if let Some(dir) = &profile.config_dir
        && !dir.as_os_str().is_empty()
    {
        return dir.clone();
    }
    if profile.use_default_auth {
        return match profile.agent {
            AgentKind::Claude => home.join(".claude"),
            AgentKind::Codex => home.join(".codex"),
            AgentKind::Gemini => home.join(".gemini"),
            AgentKind::OpenCode => home.join(".config").join("opencode"),
            AgentKind::Antigravity => home.join(".gemini").join("antigravity"),
            AgentKind::Custom => data_dir.join("profiles").join(sanitize_id(&profile.id)),
        };
    }
    data_dir.join("profiles").join(sanitize_id(&profile.id))
}

pub fn sanitize_id(id: &str) -> String {
    let s: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '_' })
        .collect();
    if s.is_empty() { "profile".into() } else { s }
}

/// Environment variables injected when launching an agent under `profile` so credentials, auth
/// tokens, and session state stay strictly isolated per account.
pub fn profile_env(profile: &AgentProfile, home: &Path, data_dir: &Path) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    let dir = resolved_config_dir(profile, home, data_dir);
    let dir_s = dir.to_string_lossy().into_owned();
    if !profile.use_default_auth || profile.config_dir.is_some() {
        let _ = std::fs::create_dir_all(&dir);
        match profile.agent {
            AgentKind::Claude => {
                env.insert("CLAUDE_CONFIG_DIR".into(), dir_s);
            }
            AgentKind::Codex => {
                env.insert("CODEX_HOME".into(), dir_s);
            }
            AgentKind::Gemini | AgentKind::Antigravity => {
                env.insert("GEMINI_CLI_HOME".into(), dir_s.clone());
                env.insert("GEMINI_CONFIG_DIR".into(), dir_s);
            }
            AgentKind::OpenCode => {
                env.insert("XDG_CONFIG_HOME".into(), dir.join("config").to_string_lossy().into_owned());
                env.insert("XDG_DATA_HOME".into(), dir.join("data").to_string_lossy().into_owned());
            }
            AgentKind::Custom => {
                env.insert("TC_PROFILE_DIR".into(), dir_s);
            }
        }
    }
    if let Some(key) = profile.api_key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        match profile.agent {
            AgentKind::Claude => {
                env.insert("ANTHROPIC_API_KEY".into(), key.into());
            }
            AgentKind::Codex => {
                env.insert("OPENAI_API_KEY".into(), key.into());
            }
            AgentKind::Gemini | AgentKind::Antigravity => {
                env.insert("GEMINI_API_KEY".into(), key.into());
            }
            AgentKind::OpenCode | AgentKind::Custom => {
                env.insert("TC_API_KEY".into(), key.into());
            }
        }
    }
    for (k, v) in &profile.extra_env {
        if !k.trim().is_empty() {
            env.insert(k.trim().to_string(), v.clone());
        }
    }
    env.insert("TC_AGENT_PROFILE".into(), profile.id.clone());
    env
}

/// Interactive login command for signing into an isolated profile inside a terminal pane.
pub fn login_command(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Claude => "claude /login",
        AgentKind::Codex => "codex login",
        AgentKind::Gemini => "gemini",
        AgentKind::OpenCode => "opencode auth login",
        AgentKind::Antigravity => "agy login",
        AgentKind::Custom => "echo 'Configure credentials for this custom profile'",
    }
}

/// Default CLI command for an agent provider.
pub fn default_command(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
        AgentKind::Gemini => "gemini",
        AgentKind::OpenCode => "opencode",
        AgentKind::Antigravity => "agy",
        AgentKind::Custom => "claude",
    }
}

/// Detected rate-limit event parsed from agent terminal output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateLimitHit {
    pub reason: String,
    pub retry_after: Option<Duration>,
}

/// Scans recent agent terminal lines for genuine rate-limit / usage-limit pauses across Claude
/// Code, Codex / ChatGPT, and Gemini CLI.
pub fn detect_rate_limit(tail: &str) -> Option<RateLimitHit> {
    let patterns = [
        "you've hit your limit",
        "you have hit your limit",
        "hit your usage limit",
        "usage limit reached",
        "claude usage limit",
        "rate limit exceeded",
        "rate_limit_error",
        "429 too many requests",
        "status 429",
        "http 429",
        "too many requests",
        "quota exceeded",
        "resource_exhausted",
        "exceeded your current quota",
        "limit reached · resets",
        "limit reached. resets",
    ];
    for line in tail.lines().rev().take(24) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if patterns.iter().any(|p| lower.contains(p)) {
            let retry_after = parse_retry_duration(&lower);
            let reason: String = trimmed.chars().take(120).collect();
            return Some(RateLimitHit { reason, retry_after });
        }
    }
    None
}

/// Parses human durations like `"in 45m"`, `"in 2h 15m"`, `"in 30 seconds"`, `"after 12 min"`.
pub fn parse_retry_duration(lower: &str) -> Option<Duration> {
    let mut total_secs = 0u64;
    let bytes = lower.as_bytes();
    let mut i = 0;
    let mut found = false;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let Ok(num) = lower[start..i].parse::<u64>() else { continue };
            let rest = lower[i..].trim_start();
            if rest.starts_with("hour") || rest.starts_with("hr") || rest.starts_with('h') && !rest.starts_with("http") {
                total_secs = total_secs.saturating_add(num.saturating_mul(3600));
                found = true;
            } else if rest.starts_with("min") || (rest.starts_with('m') && !rest.starts_with("ms") && !rest.starts_with("mb")) {
                total_secs = total_secs.saturating_add(num.saturating_mul(60));
                found = true;
            } else if rest.starts_with("sec") || (rest.starts_with('s') && !rest.starts_with("status")) {
                total_secs = total_secs.saturating_add(num);
                found = true;
            }
        } else {
            i += 1;
        }
    }
    (found && total_secs > 0 && total_secs <= 86_400).then(|| Duration::from_secs(total_secs))
}

/// Picks the next healthy enabled profile in fallback priority order when `current_id` hits its
/// limit. Prefers profiles of the same `AgentKind` first, then falls back across providers if
/// `policy.cross_provider` is enabled.
pub fn pick_failover_profile<'a>(
    profiles: &'a [AgentProfile],
    runtime: &std::collections::HashMap<String, ProfileRuntime>,
    current_id: &str,
    current_agent: AgentKind,
    policy: &FailoverSettings,
    now: Instant,
) -> Option<&'a AgentProfile> {
    let mut ordered: Vec<&AgentProfile> = profiles
        .iter()
        .filter(|p| p.enabled && p.id != current_id)
        .filter(|p| runtime.get(&p.id).is_none_or(|rt| rt.is_healthy(now)))
        .collect();
    ordered.sort_by_key(|p| (p.fallback_order, p.id.as_str()));
    // Same provider first.
    if let Some(same) = ordered.iter().copied().find(|p| p.agent == current_agent) {
        return Some(same);
    }
    if policy.cross_provider {
        return ordered.into_iter().next();
    }
    None
}

/// Writes a concise handoff note containing the interrupted task, recent terminal output, and
/// git state so a cross-account or cross-provider failover continues cleanly from the exact spot.
pub fn write_handoff_context(
    data_dir: &Path,
    pane: PaneId,
    from_profile: &str,
    task: Option<&str>,
    tail: &str,
) -> Option<PathBuf> {
    let dir = data_dir.join("handoffs");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("pane-{pane}.md"));
    let mut out = format!("# Failover Context Handoff\n\nPrevious profile: `{from_profile}`\n");
    if let Some(task) = task {
        out.push_str(&format!("Interrupted task: {task}\n"));
    }
    out.push_str("\n## Recent Terminal Output\n```\n");
    for line in tail.lines().rev().take(60).collect::<Vec<_>>().into_iter().rev() {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("```\n");
    tc_core::persist::write_private(&path, out.as_bytes()).ok()?;
    Some(path)
}

/// Formats a compact human-readable token count (`12.4k`, `1.8M`, `840`).
pub fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Formats a reset countdown (`42m 10s`, `1h 12m`, `18s`).
pub fn format_countdown(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// Runs a non-blocking readiness check for `profile`: CLI binary presence, isolated config folder
/// state, and credentials / API key status.
pub fn test_profile_connection(
    profile: &AgentProfile,
    home: &Path,
    data_dir: &Path,
    path_env: Option<&std::ffi::OsString>,
) -> Result<String, String> {
    let program = default_command(profile.agent);
    let found = match path_env {
        Some(p) => tc_lsp::servers::resolve(program, p).is_some(),
        None => std::env::var_os("PATH").is_some_and(|p| tc_lsp::servers::resolve(program, &p).is_some()),
    };
    if !found {
        return Err(format!("CLI `{program}` not found on PATH"));
    }
    let dir = resolved_config_dir(profile, home, data_dir);
    let has_key = profile.api_key.as_deref().is_some_and(|k| !k.trim().is_empty());
    let has_dir = dir.is_dir();
    let has_files = has_dir
        && std::fs::read_dir(&dir)
            .ok()
            .is_some_and(|mut it| it.next().is_some());
    if has_key {
        Ok(format!("`{program}` ready · API key configured"))
    } else if has_files {
        Ok(format!("`{program}` ready · auth session found"))
    } else {
        Ok(format!("`{program}` found · sign in via Auth Login"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_env_isolates_auth_dirs_per_provider() {
        let home = Path::new("/Users/dev");
        let data = Path::new("/Users/dev/.ternarycode");
        let p = AgentProfile {
            id: "claude2".into(),
            name: "Claude Work".into(),
            agent: AgentKind::Claude,
            api_key: Some("sk-ant-test".into()),
            ..Default::default()
        };
        let env = profile_env(&p, home, data);
        assert_eq!(env.get("CLAUDE_CONFIG_DIR").map(String::as_str), Some("/Users/dev/.ternarycode/profiles/claude2"));
        assert_eq!(env.get("ANTHROPIC_API_KEY").map(String::as_str), Some("sk-ant-test"));

        let codex = AgentProfile {
            id: "chatgpt2".into(),
            name: "ChatGPT 2".into(),
            agent: AgentKind::Codex,
            api_key: Some("sk-openai".into()),
            ..Default::default()
        };
        let cenv = profile_env(&codex, home, data);
        assert_eq!(cenv.get("CODEX_HOME").map(String::as_str), Some("/Users/dev/.ternarycode/profiles/chatgpt2"));
        assert_eq!(cenv.get("OPENAI_API_KEY").map(String::as_str), Some("sk-openai"));
    }

    #[test]
    fn rate_limit_detector_parses_limits_and_reset_timers() {
        let hit = detect_rate_limit("Working...\nYou've hit your limit · resets in 1h 25m\n").unwrap();
        assert_eq!(hit.retry_after, Some(Duration::from_secs(3600 + 25 * 60)));

        let hit2 = detect_rate_limit("Error: 429 Too Many Requests. Please try again in 45 seconds.").unwrap();
        assert_eq!(hit2.retry_after, Some(Duration::from_secs(45)));

        assert!(detect_rate_limit("Running cargo test --workspace\nok 42 passed").is_none());
    }

    #[test]
    fn failover_chain_prefers_same_provider_then_cross_provider() {
        let now = Instant::now();
        let profiles = vec![
            AgentProfile { id: "claude1".into(), agent: AgentKind::Claude, fallback_order: 1, enabled: true, ..Default::default() },
            AgentProfile { id: "claude2".into(), agent: AgentKind::Claude, fallback_order: 2, enabled: true, ..Default::default() },
            AgentProfile { id: "gemini1".into(), agent: AgentKind::Gemini, fallback_order: 3, enabled: true, ..Default::default() },
        ];
        let mut rt = std::collections::HashMap::new();
        rt.insert("claude1".into(), ProfileRuntime { state: ProfileState::RateLimited, reset_at: Some(now + Duration::from_secs(600)), ..Default::default() });
        let policy = FailoverSettings { cross_provider: true, ..Default::default() };

        // Same provider healthy: claude2 wins.
        let pick = pick_failover_profile(&profiles, &rt, "claude1", AgentKind::Claude, &policy, now).unwrap();
        assert_eq!(pick.id, "claude2");

        // All Claude profiles limited: falls over to gemini1.
        rt.insert("claude2".into(), ProfileRuntime { state: ProfileState::RateLimited, reset_at: Some(now + Duration::from_secs(600)), ..Default::default() });
        let pick2 = pick_failover_profile(&profiles, &rt, "claude1", AgentKind::Claude, &policy, now).unwrap();
        assert_eq!(pick2.id, "gemini1");

        // Cross-provider disabled: returns None.
        let strict = FailoverSettings { cross_provider: false, ..Default::default() };
        assert!(pick_failover_profile(&profiles, &rt, "claude1", AgentKind::Claude, &strict, now).is_none());
    }
}
