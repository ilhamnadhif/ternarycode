//! Multi-account agent profiles (`claude1`, `claude2`, `chatgpt1`, `agy1`, …), real usage
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

/// Local authentication readiness for an account profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthStatus {
    ApiKey,
    SignedIn,
    NotSignedIn,
    Unknown,
}

/// Live runtime telemetry for one profile (not persisted; rebuilt from panes & session files).
#[derive(Clone, Debug, Default)]
pub struct ProfileRuntime {
    pub state: ProfileState,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// Multi-window usage, server-reported 5h / weekly quotas, plan tier, and sign-in status.
    pub usage: tc_agent::sessions::ProfileUsageReport,
    pub reset_at: Option<Instant>,
    pub reset_reason: Option<String>,
    pub last_error: Option<String>,
    pub active_panes: Vec<(PaneId, ProjectId)>,
    pub connection: Option<ConnectionTest>,
    pub usage_updated: Option<Instant>,
}

impl ProfileRuntime {
    /// Percentage of the 5-hour rolling window already USED (`0.0..=100.0`).
    /// Prefers server-reported quota (e.g. Codex `rate_limits.primary`), else computes from
    /// `limit_5h_tokens` (or legacy `daily_token_budget`).
    pub fn quota_5h_used_pct(&self, prof: &AgentProfile) -> Option<f32> {
        if self.state == ProfileState::RateLimited {
            return Some(100.0);
        }
        if let Some(used) = self.usage.window_5h.used_percent {
            return Some(used.clamp(0.0, 100.0));
        }
        let budget = prof.limit_5h_tokens.or(prof.daily_token_budget).filter(|&b| b > 0)?;
        let used = self.usage.window_5h.tokens.total();
        Some(((used as f64 / budget as f64) * 100.0).clamp(0.0, 100.0) as f32)
    }

    /// Percentage of the weekly (7-day) window already USED (`0.0..=100.0`).
    /// Prefers server-reported quota (e.g. Codex `rate_limits.secondary`), else computes from
    /// `limit_weekly_tokens`.
    pub fn quota_weekly_used_pct(&self, prof: &AgentProfile) -> Option<f32> {
        if let Some(used) = self.usage.window_weekly.used_percent {
            return Some(used.clamp(0.0, 100.0));
        }
        let budget = prof.limit_weekly_tokens.filter(|&b| b > 0)?;
        let used = self.usage.window_weekly.tokens.total();
        Some(((used as f64 / budget as f64) * 100.0).clamp(0.0, 100.0) as f32)
    }

    /// Seconds remaining until the 5-hour window resets (active cooldown wins, else rolling reset).
    pub fn window_5h_reset_secs(&self, now_unix: i64, now: Instant) -> Option<u64> {
        if let Some(s) = self.reset_remaining_secs(now) {
            return Some(s);
        }
        let target = self.usage.window_5h.resets_at_unix?;
        let rem = target.saturating_sub(now_unix);
        (rem > 0).then_some(rem as u64)
    }

    /// Seconds remaining until the weekly (or monthly) window resets.
    pub fn window_weekly_reset_secs(&self, now_unix: i64) -> Option<u64> {
        let target = self.usage.window_weekly.resets_at_unix?;
        let rem = target.saturating_sub(now_unix);
        (rem > 0).then_some(rem as u64)
    }

    /// Whether this profile actually has a short (e.g. 5h) quota window to display.
    /// (Plans like ChatGPT Free only have a single 30d window and no 5h quota.)
    pub fn has_window_5h(&self, prof: &AgentProfile) -> bool {
        self.quota_5h_used_pct(prof).is_some()
            || self.usage.window_5h.window_minutes.is_some()
            || self.usage.window_5h.resets_at_unix.is_some()
    }

    /// Whether this profile actually has a long (Weekly / 30d) quota window to display.
    pub fn has_window_weekly(&self, prof: &AgentProfile) -> bool {
        self.quota_weekly_used_pct(prof).is_some()
            || self.usage.window_weekly.window_minutes.is_some()
            || self.usage.window_weekly.resets_at_unix.is_some()
    }

    /// Short label for the primary short window (`"5h"` by default, or provider-reported hours).
    pub fn window_5h_label(&self) -> String {
        match self.usage.window_5h.window_minutes {
            Some(m) if m > 0 && m != 300 => {
                if m % 60 == 0 { format!("{}h", m / 60) } else { format!("{m}m") }
            }
            _ => "5h".into(),
        }
    }

    /// Short label for the secondary long window (`"Weekly"` by default, or `"30d"` if monthly).
    pub fn window_weekly_label(&self) -> String {
        match self.usage.window_weekly.window_minutes {
            Some(m) if m >= 40_000 => "30d".into(),
            Some(m) if m > 0 && m != 10_080 && m % 1440 == 0 => format!("{}d", m / 1440),
            _ => "Weekly".into(),
        }
    }

    /// Whether credentials are ready for this profile (API key or isolated OAuth sign-in).
    pub fn auth_status(&self, prof: &AgentProfile) -> AuthStatus {
        if prof.api_key.as_deref().is_some_and(|k| !k.trim().is_empty()) {
            return AuthStatus::ApiKey;
        }
        match self.usage.signed_in {
            Some(true) => AuthStatus::SignedIn,
            Some(false) => AuthStatus::NotSignedIn,
            None => AuthStatus::Unknown,
        }
    }

    /// Remaining quota percentage if a budget or server quota is known (or `Some(0.0)` while rate-limited).
    pub fn quota_remaining_pct(&self, prof: &AgentProfile) -> Option<f32> {
        if self.state == ProfileState::RateLimited {
            return Some(0.0);
        }
        if let Some(used) = self.quota_5h_used_pct(prof).or_else(|| self.quota_weekly_used_pct(prof)) {
            return Some((100.0 - used).clamp(0.0, 100.0));
        }
        let budget = prof.daily_token_budget.filter(|&b| b > 0)?;
        let used = self.tokens_in.saturating_add(self.tokens_out);
        let pct = 100.0 * (1.0 - (used as f64 / budget as f64)).clamp(0.0, 1.0);
        Some(pct as f32)
    }

    /// Seconds remaining until rate-limit / cooldown reset, if any.
    pub fn reset_remaining_secs(&self, now: Instant) -> Option<u64> {
        let d = self.reset_at?.checked_duration_since(now)?;
        let secs = d.as_secs() + u64::from(d.subsec_nanos() > 0);
        (secs > 0).then_some(secs)
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
    /// Timestamp when the banner was created (for auto-dismissing auto-switched notices).
    pub created_at: Instant,
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

/// Whether `agent` supports isolated multi-account profiles & failover.
/// Only Claude Code (`CLAUDE_CONFIG_DIR`) and ChatGPT / Codex (`CODEX_HOME` + file-backed auth)
/// support true per-profile OAuth isolation without colliding in the global OS Keychain.
pub fn supports_multi_profile(agent: AgentKind) -> bool {
    matches!(agent, AgentKind::Claude | AgentKind::Codex)
}

/// Built-in default starter profiles when the user has not customized `settings.profiles` yet.
/// Each default profile uses the provider's standard auth folder so existing logins work instantly.
pub fn default_profiles() -> Vec<AgentProfile> {
    vec![
        AgentProfile {
            id: "claude1".into(),
            name: "Claude 1".into(),
            agent: AgentKind::Claude,
            fallback_order: 1,
            enabled: true,
            use_default_auth: true,
            ..Default::default()
        },
        AgentProfile {
            id: "chatgpt1".into(),
            name: "ChatGPT / Codex 1".into(),
            agent: AgentKind::Codex,
            fallback_order: 2,
            enabled: true,
            use_default_auth: true,
            ..Default::default()
        },
    ]
}

pub fn has_custom_config_dir(profile: &AgentProfile) -> bool {
    profile.config_dir.as_ref().is_some_and(|d| !d.as_os_str().is_empty())
}

/// Resolves the config/auth root folder for `profile`.
/// - Explicit non-empty `profile.config_dir` wins.
/// - `use_default_auth` maps to `~/.claude`, `~/.codex`, `~/.gemini/antigravity-cli` (honoring standard env vars).
/// - Otherwise isolates cleanly into `<data_dir>/profiles/<sanitized_id>`.
pub fn resolved_config_dir(profile: &AgentProfile, home: &Path, data_dir: &Path) -> PathBuf {
    if let Some(dir) = &profile.config_dir
        && !dir.as_os_str().is_empty()
    {
        return dir.clone();
    }
    if profile.use_default_auth {
        let env_path = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        return match profile.agent {
            AgentKind::Claude => env_path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
            AgentKind::Codex => env_path("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
            AgentKind::Gemini => env_path("GEMINI_CONFIG_DIR")
                .or_else(|| env_path("GEMINI_CLI_HOME").map(|h| h.join(".gemini")))
                .unwrap_or_else(|| home.join(".gemini")),
            AgentKind::OpenCode => home.join(".config").join("opencode"),
            AgentKind::Antigravity => env_path("AGY_DATA_DIR")
                .unwrap_or_else(|| home.join(".gemini").join("antigravity-cli")),
            AgentKind::Custom => data_dir.join("profiles").join(sanitize_id(&profile.id)),
        };
    }
    data_dir.join("profiles").join(sanitize_id(&profile.id))
}

pub fn sanitize_id(id: &str) -> String {
    let s: String = id
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '_' })
        .take(32)
        .collect();
    if s.chars().any(|c| c.is_ascii_alphanumeric()) { s } else { "profile".into() }
}

/// Environment variables injected when launching an agent under `profile` so credentials, auth
/// tokens, and session state stay strictly isolated per account.
pub fn profile_env(profile: &AgentProfile, home: &Path, data_dir: &Path) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    let dir = resolved_config_dir(profile, home, data_dir);
    let dir_s = dir.to_string_lossy().into_owned();
    if !profile.use_default_auth || has_custom_config_dir(profile) {
        let _ = tc_core::persist::ensure_private_dir(&dir);
        match profile.agent {
            AgentKind::Claude => {
                env.insert("CLAUDE_CONFIG_DIR".into(), dir_s);
            }
            AgentKind::Codex => {
                // Force file-backed auth inside isolated CODEX_HOME so multiple ChatGPT Plus/Pro
                // accounts (`chatgpt1`, `chatgpt2`, `chatgpt3`) never collide in the OS keychain.
                let cfg_path = dir.join("config.toml");
                if !cfg_path.exists() {
                    let _ = std::fs::write(&cfg_path, "cli_auth_credentials_store = \"file\"\n");
                }
                env.insert("CODEX_HOME".into(), dir_s);
            }
            AgentKind::Antigravity => {
                env.insert("AGY_DATA_DIR".into(), dir_s);
            }
            AgentKind::Gemini => {
                env.insert("GEMINI_CLI_HOME".into(), dir_s.clone());
                env.insert("GEMINI_CONFIG_DIR".into(), dir_s);
            }
            AgentKind::OpenCode => {
                let cfg = dir.join("config");
                let dat = dir.join("data");
                let _ = tc_core::persist::ensure_private_dir(&cfg);
                let _ = tc_core::persist::ensure_private_dir(&dat);
                env.insert("XDG_CONFIG_HOME".into(), cfg.to_string_lossy().into_owned());
                env.insert("XDG_DATA_HOME".into(), dat.to_string_lossy().into_owned());
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

/// Provider auth/API-key env variables that must be cleared when switching to `profile` if not
/// explicitly set by `profile_env` (prevents env leakage across profile switches, especially on
/// PowerShell where `$env:KEY=...` persists for the shell lifetime).
pub fn profile_unset_env(profile: &AgentProfile) -> Vec<&'static str> {
    let mut unset = Vec::new();
    let uses_default = profile.use_default_auth && !has_custom_config_dir(profile);
    let has_key = profile.api_key.as_deref().is_some_and(|k| !k.trim().is_empty());
    match profile.agent {
        AgentKind::Claude => {
            if uses_default {
                unset.push("CLAUDE_CONFIG_DIR");
            }
            if !has_key {
                unset.push("ANTHROPIC_API_KEY");
            }
        }
        AgentKind::Codex => {
            if uses_default {
                unset.push("CODEX_HOME");
            }
            if !has_key {
                unset.push("OPENAI_API_KEY");
            }
        }
        AgentKind::Antigravity => {
            if uses_default {
                unset.push("AGY_DATA_DIR");
            }
            if !has_key {
                unset.push("GEMINI_API_KEY");
            }
        }
        AgentKind::Gemini => {
            if uses_default {
                unset.push("GEMINI_CLI_HOME");
                unset.push("GEMINI_CONFIG_DIR");
            }
            if !has_key {
                unset.push("GEMINI_API_KEY");
            }
        }
        AgentKind::OpenCode => {
            if !has_key {
                unset.push("TC_API_KEY");
            }
        }
        AgentKind::Custom => {
            if !has_key {
                unset.push("TC_API_KEY");
            }
        }
    }
    unset
}

/// Redacts plaintext API keys, bearer tokens, and configured profile secrets from terminal text
/// before persisting handoff files or streaming output over Mobile Companion HTTP/SSE.
pub fn redact_secrets(text: &str, known_secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in known_secrets {
        let s = secret.trim();
        if s.len() >= 8 && out.contains(s) {
            out = out.replace(s, "[REDACTED]");
        }
    }
    let mut scrubbed = Vec::new();
    for line in out.lines() {
        let mut s = line.to_string();
        for key_prefix in [
            "ANTHROPIC_API_KEY=",
            "OPENAI_API_KEY=",
            "GEMINI_API_KEY=",
            "TC_API_KEY=",
            "API_KEY=",
            "SECRET_KEY=",
            "ACCESS_TOKEN=",
            "AUTH_TOKEN=",
        ] {
            let mut search_from = 0;
            while let Some(rel) = s[search_from..].find(key_prefix) {
                let start = search_from + rel + key_prefix.len();
                let end = if s[start..].starts_with('\'') {
                    s[start + 1..].find('\'').map(|i| start + 1 + i + 1).unwrap_or(s.len())
                } else if s[start..].starts_with('"') {
                    s[start + 1..].find('"').map(|i| start + 1 + i + 1).unwrap_or(s.len())
                } else {
                    s[start..].find(|c: char| c.is_whitespace() || c == ';').map(|i| start + i).unwrap_or(s.len())
                };
                s.replace_range(start..end, "'[REDACTED]'");
                search_from = start + "'[REDACTED]'".len();
            }
        }
        for auth_prefix in ["Bearer ", "bearer ", "BEARER "] {
            let mut search_from = 0;
            while let Some(rel) = s[search_from..].find(auth_prefix) {
                let start = search_from + rel + auth_prefix.len();
                let end = s[start..]
                    .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '=' | '+' | '/')))
                    .map(|i| start + i)
                    .unwrap_or(s.len());
                if end - start >= 12 {
                    s.replace_range(start..end, "[REDACTED]");
                    search_from = start + "[REDACTED]".len();
                } else {
                    search_from = end.max(start);
                }
            }
        }
        for tok_prefix in ["sk-ant-", "sk-proj-", "AIza", "ghp_", "gho_", "github_pat_"] {
            let mut search_from = 0;
            while let Some(rel) = s[search_from..].find(tok_prefix) {
                let start = search_from + rel;
                let end = s[start..]
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                    .map(|i| start + i)
                    .unwrap_or(s.len());
                if end - start >= tok_prefix.len() + 6 {
                    s.replace_range(start..end, "[REDACTED]");
                    search_from = start + "[REDACTED]".len();
                } else {
                    search_from = end;
                }
            }
        }
        scrubbed.push(s);
    }
    scrubbed.join("\n")
}

/// Interactive login command for signing into an isolated profile inside a terminal pane.
pub fn login_command(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Claude => "claude auth login",
        AgentKind::Codex => "codex login",
        AgentKind::Gemini => "gemini",
        AgentKind::OpenCode => "opencode auth login",
        AgentKind::Antigravity => "agy",
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

/// Returns true if a terminal line clearly looks like source code, diff output, or test logs
/// rather than a genuine CLI rate-limit error banner.
fn looks_like_code_or_test(trimmed: &str) -> bool {
    trimmed.starts_with("+//")
        || trimmed.starts_with("-//")
        || trimmed.starts_with("//")
        || trimmed.starts_with("+ ")
        || trimmed.starts_with("- ")
        || trimmed.starts_with("| ")
        || trimmed.starts_with('"')
        || trimmed.starts_with('\'')
        || trimmed.starts_with("test ")
        || trimmed.starts_with("assert")
        || trimmed.contains("fn ")
        || trimmed.contains("let ")
        || trimmed.contains("pub fn ")
        || trimmed.contains("detect_rate_limit")
}

/// Scans recent agent terminal lines for genuine rate-limit / usage-limit pauses across Claude
/// Code, Codex / ChatGPT, and Antigravity CLI, supporting 2-line wrapped error messages while skipping
/// source code / diff / test false positives.
pub fn detect_rate_limit(tail: &str) -> Option<RateLimitHit> {
    let patterns = [
        "you've hit your limit",
        "you have hit your limit",
        "hit your usage limit",
        "hit your limit",
        "usage limit reached",
        "usage_limit_reached",
        "claude usage limit",
        "rate limit exceeded",
        "rate_limit_error",
        "rate_limit_reached",
        "429 too many requests",
        "status 429",
        "http 429",
        "error 429",
        "error: 429",
        "too many requests",
        "quota exceeded",
        "insufficient_quota",
        "resource_exhausted",
        "resource exhausted",
        "exceeded your current quota",
        "spending cap",
        "credit balance is too low",
        "overloaded_error",
        "limit reached · resets",
        "limit reached. resets",
    ];
    let lines: Vec<&str> = tail
        .lines()
        .rev()
        .take(24)
        .map(str::trim)
        .filter(|l| !l.is_empty() && !looks_like_code_or_test(l))
        .collect();
    for (idx, trimmed) in lines.iter().enumerate() {
        let lower = trimmed.to_ascii_lowercase();
        if patterns.iter().any(|p| lower.contains(p)) {
            // Check both the matched line and the following line (idx - 1 in reversed order) in
            // case the reset timer wrapped across a narrow terminal width.
            let combined = if idx > 0 {
                format!("{lower} {}", lines[idx - 1].to_ascii_lowercase())
            } else {
                lower.clone()
            };
            let retry_after = parse_retry_duration(&combined);
            let reason: String = trimmed.chars().take(120).collect();
            return Some(RateLimitHit { reason, retry_after });
        }
    }
    None
}

/// Parses human durations after reset/retry clauses like `"resets in 1h 25m"`, `"try again in 45 seconds"`,
/// `"retry after 12 min"`, or wall-clock times like `"resets 5pm"` / `"resets at 14:30"`.
pub fn parse_retry_duration(lower: &str) -> Option<Duration> {
    // Isolate the clause after reset/retry keywords so error codes like `429 Some...` or
    // `5 hour limit · resets in 12m` are never misparsed as cooldown durations.
    let slice = [
        "resets in ",
        "reset in ",
        "try again in ",
        "retry in ",
        "retry after ",
        "wait ",
        "resets at ",
        "resets ",
        "after ",
    ]
    .iter()
    .filter_map(|kw| lower.rfind(kw).map(|pos| &lower[pos + kw.len()..]))
    .next()
    .unwrap_or(lower);

    let is_unit_end = |s: &str, len: usize| s.as_bytes().get(len).is_none_or(|b| !b.is_ascii_alphabetic());
    let mut total_secs = 0u64;
    let bytes = slice.as_bytes();
    let mut i = 0;
    let mut found = false;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let Ok(num) = slice[start..i].parse::<u64>() else { continue };
            let rest = slice[i..].trim_start();
            let unit_hours = ["hours", "hour", "hrs", "hr", "h"].iter().find(|u| rest.starts_with(*u) && is_unit_end(rest, u.len()));
            let unit_mins = ["minutes", "minute", "mins", "min", "m"].iter().find(|u| rest.starts_with(*u) && is_unit_end(rest, u.len()));
            let unit_secs = ["seconds", "second", "secs", "sec", "s"].iter().find(|u| rest.starts_with(*u) && is_unit_end(rest, u.len()));
            if let Some(u) = unit_hours {
                total_secs = total_secs.saturating_add(num.saturating_mul(3600));
                i = slice.len() - rest.len() + u.len();
                found = true;
            } else if let Some(u) = unit_mins {
                total_secs = total_secs.saturating_add(num.saturating_mul(60));
                i = slice.len() - rest.len() + u.len();
                found = true;
            } else if let Some(u) = unit_secs {
                total_secs = total_secs.saturating_add(num);
                i = slice.len() - rest.len() + u.len();
                found = true;
            }
        } else {
            i += 1;
        }
    }
    if found && total_secs > 0 && total_secs <= 86_400 {
        return Some(Duration::from_secs(total_secs));
    }
    parse_wall_clock_reset(slice)
}

/// Parses wall-clock reset times like `"5pm"`, `"5:00 pm"`, `"3:30am"`, or `"14:00"` into a relative duration.
fn parse_wall_clock_reset(slice: &str) -> Option<Duration> {
    use chrono::Timelike;
    let mut words = slice.split_whitespace();
    let token = words.next()?.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != ':');
    let next_word = words
        .next()
        .map(|w| w.trim_matches(|c: char| !c.is_ascii_alphabetic()).to_ascii_lowercase());
    let (time_part, ampm) = if let Some(t) = token.strip_suffix("am") {
        (t, Some(false))
    } else if let Some(t) = token.strip_suffix("pm") {
        (t, Some(true))
    } else if next_word.as_deref() == Some("am") {
        (token, Some(false))
    } else if next_word.as_deref() == Some("pm") {
        (token, Some(true))
    } else {
        (token, None)
    };
    let (h_str, m_str) = match time_part.split_once(':') {
        Some((h, m)) => (h, m),
        None if ampm.is_some() => (time_part, "0"),
        None => return None,
    };
    let mut hour: u32 = h_str.parse().ok()?;
    let min: u32 = m_str.parse().ok()?;
    if min >= 60 {
        return None;
    }
    match ampm {
        Some(false) => {
            if hour == 0 || hour > 12 { return None; }
            if hour == 12 { hour = 0; }
        }
        Some(true) => {
            if hour == 0 || hour > 12 { return None; }
            if hour < 12 { hour += 12; }
        }
        None => {
            if hour >= 24 { return None; }
        }
    }
    let now = chrono::Local::now();
    let now_secs = now.num_seconds_from_midnight();
    let target_secs = hour * 3600 + min * 60;
    let diff = if target_secs > now_secs {
        target_secs - now_secs
    } else {
        86_400 - now_secs + target_secs
    };
    (diff > 0 && diff <= 86_400).then(|| Duration::from_secs(u64::from(diff)))
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

/// Current Unix epoch seconds (monotonic-safe helper for comparing `resets_at_unix`).
pub fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Formats a reset countdown (`3d 14h`, `4h 12m`, `42m 10s`, `18s`).
pub fn format_countdown(secs: u64) -> String {
    if secs >= 86_400 {
        format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3600)
    } else if secs >= 3600 {
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
    let dir = resolved_config_dir(profile, home, data_dir);
    let has_key = profile.api_key.as_deref().is_some_and(|k| !k.trim().is_empty());
    let has_dir = dir.is_dir();
    let has_files = has_dir
        && std::fs::read_dir(&dir)
            .ok()
            .is_some_and(|mut it| it.next().is_some());
    if profile.agent == AgentKind::Custom {
        return Ok(if has_key {
            "Custom profile ready · API key configured".into()
        } else if has_files {
            "Custom profile ready · isolated config folder present".into()
        } else {
            "Custom profile configured".into()
        });
    }
    let program = default_command(profile.agent);
    let found = match path_env {
        Some(p) => tc_lsp::servers::resolve(program, p).is_some(),
        None => std::env::var_os("PATH").is_some_and(|p| tc_lsp::servers::resolve(program, &p).is_some()),
    };
    if !found {
        return Err(format!("CLI `{program}` not found on PATH"));
    }
    if has_key {
        Ok(format!("`{program}` ready · API key configured"))
    } else if tc_agent::sessions::is_signed_in(profile.agent, &dir) == Some(true) {
        Ok(format!("`{program}` ready · auth session found"))
    } else if tc_agent::sessions::is_signed_in(profile.agent, &dir).is_none() && has_files {
        Ok(format!("`{program}` ready · system keyring configured"))
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
        assert_eq!(env.get("CLAUDE_CONFIG_DIR").map(PathBuf::from), Some(data.join("profiles").join("claude2")));
        assert_eq!(env.get("ANTHROPIC_API_KEY").map(String::as_str), Some("sk-ant-test"));

        let codex = AgentProfile {
            id: "chatgpt2".into(),
            name: "ChatGPT 2".into(),
            agent: AgentKind::Codex,
            api_key: Some("sk-openai".into()),
            ..Default::default()
        };
        let cenv = profile_env(&codex, home, data);
        assert_eq!(cenv.get("CODEX_HOME").map(PathBuf::from), Some(data.join("profiles").join("chatgpt2")));
        assert_eq!(cenv.get("OPENAI_API_KEY").map(String::as_str), Some("sk-openai"));

        assert!(supports_multi_profile(AgentKind::Claude));
        assert!(supports_multi_profile(AgentKind::Codex));
        assert!(!supports_multi_profile(AgentKind::Antigravity));
        assert!(!supports_multi_profile(AgentKind::OpenCode));
    }

    #[test]
    fn rate_limit_detector_parses_limits_and_reset_timers() {
        let hit = detect_rate_limit("Working...\nYou've hit your limit · resets in 1h 25m\n").unwrap();
        assert_eq!(hit.retry_after, Some(Duration::from_secs(3600 + 25 * 60)));

        let hit2 = detect_rate_limit("Error: 429 Too Many Requests. Please try again in 45 seconds.").unwrap();
        assert_eq!(hit2.retry_after, Some(Duration::from_secs(45)));

        // Wrapped 2-line error where reset duration is on the next line.
        let hit3 = detect_rate_limit("You've hit your usage limit.\nResets in 2h 10m").unwrap();
        assert_eq!(hit3.retry_after, Some(Duration::from_secs(2 * 3600 + 10 * 60)));

        // Must NOT parse unrelated numbers like "429" or "2026" as retry durations.
        let hit4 = detect_rate_limit("HTTP 429 Too Many Requests from api.example.com").unwrap();
        assert_eq!(hit4.retry_after, None);

        // Must NOT trigger false positives on source code, git diffs, or cargo test output!
        assert!(detect_rate_limit("Running cargo test --workspace\nok 42 passed").is_none());
        assert!(detect_rate_limit("+ // Error: 429 Too Many Requests\n- let s = \"rate limit exceeded\";").is_none());
        assert!(detect_rate_limit("test profiles::tests::rate_limit_exceeded ... ok").is_none());
        assert!(detect_rate_limit("fn detect_rate_limit() { \"usage limit reached\" }").is_none());
    }

    #[test]
    fn redact_secrets_strips_api_keys_and_tokens() {
        let input = "ANTHROPIC_API_KEY=sk-ant-api03-secretvalue1234567890 Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig custom=my-super-secret-val";
        let out = redact_secrets(input, &["my-super-secret-val".into()]);
        assert!(!out.contains("sk-ant-api03-secretvalue1234567890"), "{out}");
        assert!(!out.contains("eyJhbGciOiJIUzI1NiJ9"), "{out}");
        assert!(!out.contains("my-super-secret-val"), "{out}");
        assert!(out.contains("[REDACTED]"), "{out}");
    }

    #[test]
    fn failover_chain_prefers_same_provider_then_cross_provider() {
        let now = Instant::now();
        let profiles = vec![
            AgentProfile { id: "claude1".into(), agent: AgentKind::Claude, fallback_order: 1, enabled: true, ..Default::default() },
            AgentProfile { id: "claude2".into(), agent: AgentKind::Claude, fallback_order: 2, enabled: true, ..Default::default() },
            AgentProfile { id: "chatgpt1".into(), agent: AgentKind::Codex, fallback_order: 3, enabled: true, ..Default::default() },
        ];
        let mut rt = std::collections::HashMap::new();
        rt.insert("claude1".into(), ProfileRuntime { state: ProfileState::RateLimited, reset_at: Some(now + Duration::from_secs(600)), ..Default::default() });
        let policy = FailoverSettings { cross_provider: true, ..Default::default() };

        // Same provider healthy: claude2 wins.
        let pick = pick_failover_profile(&profiles, &rt, "claude1", AgentKind::Claude, &policy, now).unwrap();
        assert_eq!(pick.id, "claude2");

        // All Claude profiles limited: falls over to chatgpt1.
        rt.insert("claude2".into(), ProfileRuntime { state: ProfileState::RateLimited, reset_at: Some(now + Duration::from_secs(600)), ..Default::default() });
        let pick2 = pick_failover_profile(&profiles, &rt, "claude1", AgentKind::Claude, &policy, now).unwrap();
        assert_eq!(pick2.id, "chatgpt1");

        // Cross-provider disabled: returns None.
        let strict = FailoverSettings { cross_provider: false, ..Default::default() };
        assert!(pick_failover_profile(&profiles, &rt, "claude1", AgentKind::Claude, &strict, now).is_none());
    }
}
