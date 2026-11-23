//! Runtime orchestration for the redesigned ADE:
//! - Multi-account agent profile resolution & isolated environment injection
//! - Automatic & manual rate-limit failover + state-preserving session resume
//! - Real token usage telemetry & project-nested session history indexing
//! - Focus / Zen Mode & secondary drawer state management
//! - Embedded Mobile Companion server lifecycle, live snapshot publishing, and remote command dispatch

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;
use tc_agent::integrate;
use tc_core::agent::{AgentKind, Preset};
use tc_core::model::{AgentProfile, DrawerDock, FailoverMode, Integration, View};
use tc_core::status::Status;
use tc_core::{PaneId, ProjectId};

use crate::actions::{Action, Launch};
use crate::app::{App, AppMsg, ToastKind};
use crate::companion::{
    CompanionAlert, CompanionCmd, CompanionHandle, CompanionPane, CompanionProfile,
    CompanionProject, CompanionRateLimit, CompanionSession, CompanionSnapshot, QuickRemoteAction,
};
use crate::i18n::{t, tf};
use crate::paths;
use crate::profiles::{
    self, AgentLogEntry, ConnectionTest, LogLevel, ProfileState, ProjectSessionItem,
    RateLimitBanner, RateLimitHit,
};


/// Editable form draft in the Profiles & Accounts management view.
#[derive(Clone, Debug)]
pub struct ProfileDraft {
    /// Original profile ID when editing an existing profile (`None` when creating a new one).
    pub editing_id: Option<String>,
    pub id: String,
    pub name: String,
    pub account_hint: String,
    pub agent: AgentKind,
    pub config_dir: String,
    pub api_key: String,
    pub reveal_key: bool,
    pub extra_env_text: String,
    pub limit_5h_text: String,
    pub limit_weekly_text: String,
    pub budget_text: String,
    pub use_default_auth: bool,
    pub enabled: bool,
}

impl ProfileDraft {
    pub fn suggest_id_and_name(agent: AgentKind, existing: &[AgentProfile]) -> (String, String) {
        let prefix = match agent {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "chatgpt",
            AgentKind::Gemini => "gemini",
            AgentKind::OpenCode => "opencode",
            AgentKind::Antigravity => "agy",
            AgentKind::Custom => "agent",
        };
        let label = match agent {
            AgentKind::Claude => "Claude",
            AgentKind::Codex => "ChatGPT / Codex",
            AgentKind::Antigravity => "Antigravity",
            _ => agent.label(),
        };
        let mut idx = 1u32;
        while existing.iter().any(|p| p.id.eq_ignore_ascii_case(&format!("{prefix}{idx}"))) {
            idx += 1;
        }
        (format!("{prefix}{idx}"), format!("{label} {idx}"))
    }

    pub fn new_for(agent: AgentKind, existing: &[AgentProfile]) -> Self {
        let agent = if profiles::supports_multi_profile(agent) { agent } else { AgentKind::Claude };
        let (id, name) = Self::suggest_id_and_name(agent, existing);
        Self {
            editing_id: None,
            name,
            id,
            account_hint: String::new(),
            agent,
            config_dir: String::new(),
            api_key: String::new(),
            reveal_key: false,
            extra_env_text: String::new(),
            limit_5h_text: String::new(),
            limit_weekly_text: String::new(),
            budget_text: String::new(),
            use_default_auth: false,
            enabled: true,
        }
    }

    pub fn from_profile(p: &AgentProfile) -> Self {
        let extra_env_text = p
            .extra_env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            editing_id: Some(p.id.clone()),
            id: p.id.clone(),
            name: p.name.clone(),
            account_hint: p.account_hint.clone().unwrap_or_default(),
            agent: p.agent,
            config_dir: p.config_dir.as_ref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
            api_key: p.api_key.clone().unwrap_or_default(),
            reveal_key: false,
            extra_env_text,
            limit_5h_text: p.limit_5h_tokens.map(|b| b.to_string()).unwrap_or_default(),
            limit_weekly_text: p.limit_weekly_tokens.map(|b| b.to_string()).unwrap_or_default(),
            budget_text: p.daily_token_budget.map(|b| b.to_string()).unwrap_or_default(),
            use_default_auth: p.use_default_auth,
            enabled: p.enabled,
        }
    }

    pub fn into_profile(self, fallback_order: u32) -> AgentProfile {
        let id = profiles::sanitize_id(self.id.trim());
        let name = if self.name.trim().is_empty() { id.clone() } else { self.name.trim().to_string() };
        let account_hint = {
            let trimmed = self.account_hint.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        };
        let mut extra_env = BTreeMap::new();
        for line in self.extra_env_text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                let valid_key = !k.is_empty()
                    && !k.as_bytes()[0].is_ascii_digit()
                    && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
                if valid_key {
                    extra_env.insert(k.to_string(), v.trim().to_string());
                }
            }
        }
        let config_dir = {
            let trimmed = self.config_dir.trim();
            (!trimmed.is_empty()).then(|| paths::expand_tilde(trimmed))
        };
        let api_key = {
            let trimmed = self.api_key.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        };
        let parse_cap = |s: &str| {
            let clean = s.trim().to_ascii_lowercase().replace([',', '_', ' '], "");
            if let Some(m) = clean.strip_suffix('m') {
                m.parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v > 0.0)
                    .map(|v| (v * 1_000_000.0).round() as u64)
                    .filter(|&v| v > 0)
            } else if let Some(k) = clean.strip_suffix('k') {
                k.parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v > 0.0)
                    .map(|v| (v * 1_000.0).round() as u64)
                    .filter(|&v| v > 0)
            } else {
                clean.parse::<u64>().ok().filter(|&v| v > 0)
            }
        };
        let limit_5h_tokens = parse_cap(&self.limit_5h_text);
        let limit_weekly_tokens = parse_cap(&self.limit_weekly_text);
        let daily_token_budget = parse_cap(&self.budget_text);
        AgentProfile {
            id,
            name,
            agent: self.agent,
            config_dir,
            api_key,
            extra_env,
            fallback_order,
            enabled: self.enabled,
            daily_token_budget,
            limit_5h_tokens,
            limit_weekly_tokens,
            account_hint,
            use_default_auth: self.use_default_auth,
        }
    }
}

impl App {
    /// Configured multi-account profiles, falling back to starter defaults (`claude1`, `chatgpt1`)
    /// until customized. Restricted to providers that support isolated multi-account auth.
    pub fn effective_profiles(&self) -> Vec<AgentProfile> {
        let mut list: Vec<AgentProfile> = self
            .settings
            .profiles
            .iter()
            .filter(|p| profiles::supports_multi_profile(p.agent))
            .cloned()
            .collect();
        if list.is_empty() {
            list = profiles::default_profiles();
        }
        list.sort_by_key(|p| (p.fallback_order, p.id.clone()));
        list
    }

    /// Materializes default profiles into `settings.profiles` before mutating.
    pub fn ensure_mutable_profiles(&mut self) {
        self.settings.profiles.retain(|p| profiles::supports_multi_profile(p.agent));
        if self.settings.profiles.is_empty() {
            self.settings.profiles = profiles::default_profiles();
        }
    }

    pub fn profile_by_id(&self, id: &str) -> Option<AgentProfile> {
        self.effective_profiles().into_iter().find(|p| p.id == id)
    }

    /// Resolves the canonical active profile ID for `pane` running `agent`: uses `pane.active_profile`
    /// when valid for `agent`, otherwise falls back to the enabled default-auth profile for `agent`
    /// (never misattributing a manual `~/.claude` shell to an isolated profile like `claude2`).
    pub fn resolved_pane_profile_id(
        &self,
        pane: &crate::pane::Pane,
        agent: AgentKind,
        profiles: &[AgentProfile],
    ) -> Option<String> {
        if !profiles::supports_multi_profile(agent) {
            return None;
        }
        if let Some(id) = pane.active_profile.as_deref()
            && profiles.iter().any(|p| p.id == id && p.agent == agent)
        {
            return Some(id.to_string());
        }
        profiles
            .iter()
            .find(|p| p.agent == agent && p.enabled && p.use_default_auth && !profiles::has_custom_config_dir(p))
            .or_else(|| profiles.iter().find(|p| p.agent == agent && p.enabled))
            .map(|p| p.id.clone())
    }

    /// Picks the highest-priority enabled healthy profile for `agent` (or first enabled profile).
    pub fn preferred_profile_for(&self, agent: AgentKind, project: Option<ProjectId>) -> Option<AgentProfile> {
        if !profiles::supports_multi_profile(agent) {
            return None;
        }
        let now = Instant::now();
        let all = self.effective_profiles();
        if let Some(pid) = project
            && let Some(pref) = self.project(pid).and_then(|p| p.active_profile.as_deref())
            && let Some(prof) = all.iter().find(|p| p.id == pref && p.agent == agent && p.enabled)
            && self.profiles_rt.get(&prof.id).is_none_or(|rt| rt.is_healthy(now))
        {
            return Some(prof.clone());
        }
        all.iter()
            .find(|p| p.agent == agent && p.enabled && self.profiles_rt.get(&p.id).is_none_or(|rt| rt.is_healthy(now)))
            .or_else(|| all.iter().find(|p| p.agent == agent && p.enabled))
            .cloned()
    }

    /// Returns a preset for `agent`, synthesizing a default command preset when none is configured
    /// (for example Antigravity CLI or Custom accounts).
    pub fn preset_for_agent(&self, agent: AgentKind) -> Preset {
        self.presets()
            .into_iter()
            .find(|p| p.agent == Some(agent))
            .unwrap_or_else(|| Preset {
                id: agent.slug().into(),
                name: agent.label().into(),
                agent: Some(agent),
                command: profiles::default_command(agent).into(),
                resume: None,
                env: BTreeMap::new(),
                builtin: false,
            })
    }

    /// Builds a shell launch command for `preset` combining hooks, permissions flag, preset env,
    /// and isolated account profile env (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `AGY_DATA_DIR`, API keys).
    pub fn launch_command_for_profile(&self, preset: &Preset, command: &str, profile: Option<&AgentProfile>) -> String {
        let integration = preset
            .agent
            .filter(|_| self.per_launch)
            .map(|a| self.settings.integration(a))
            .unwrap_or(Integration::Off);
        let mut command = command.trim().to_string();
        if let Some(flag) = preset.agent.filter(|a| self.skips_permissions(*a)).and_then(tc_agent::sessions::skip_permissions_flag)
            && !command.split_whitespace().any(|w| w == flag)
        {
            command = integrate::insert_flag(&command, flag);
        }
        let command = integrate::launch_command_with(&command, preset.agent, integration, self.codex_hooks, self.codex_own_notify);
        let mut env = preset.env.clone();
        let mut unset = Vec::new();
        if let Some(profile) = profile {
            let penv = profiles::profile_env(profile, &paths::home_dir(), paths::data_dir());
            env.extend(penv);
            unset = profiles::profile_unset_env(profile);
            unset.retain(|k| !env.contains_key(*k));
        }
        integrate::with_env_and_unset(&command, &env, &unset)
    }

    /// Appends one structured entry to the Agent Logs drawer & Monitor event feed.
    pub fn push_agent_log(
        &mut self,
        level: LogLevel,
        project: Option<ProjectId>,
        pane: Option<PaneId>,
        profile: Option<String>,
        message: impl Into<String>,
    ) {
        if self.agent_logs.len() >= 300 {
            self.agent_logs.pop_front();
        }
        self.agent_logs.push_back(AgentLogEntry {
            at: chrono::Local::now(),
            project,
            pane,
            profile,
            level,
            message: message.into(),
        });
    }

    /// Ensures the project's editor exists (hidden: Files opens it in the right panel).
    pub fn ensure_editor_for_project(&mut self, id: ProjectId, saved_width: f32) {
        let visible = false;
        let width = if saved_width > 0.0 { saved_width.clamp(0.45, 0.85) } else { 0.64 };
        self.editors.entry(id).or_insert_with(|| crate::editor::Editor {
            visible,
            width,
            ..Default::default()
        });
    }

    /// Leaves Zen: the sidebar, the project bar and the panel come back as they were.
    pub fn exit_zen_mode(&mut self) {
        self.zen_mode = false;
    }

    /// Enters or exits one-keystroke Focus / Zen Mode without mutating persisted settings.
    pub fn toggle_zen_mode(&mut self) {
        if self.zen_mode {
            self.exit_zen_mode();
            return;
        }
        if self.projects.is_empty() {
            return;
        }
        // Zen: only the terminals (the panel stays as it was, out of view).
        self.zen_mode = true;
        self.drawer_maximized = false;
        self.monitor_popover = false;
        self.attention_open = false;
        self.show_focus();
        self.release_page_keyboard();
        self.focus_request = self.current_pane();
    }

    /// Periodic ADE background tick: updates profile states, expires cooldowns, scans agent panes
    /// for rate limits, refreshes real token telemetry & project-nested session history, and
    /// publishes updates to connected Mobile Companions.
    pub fn ade_tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let profiles = self.effective_profiles();

        // 1. Recompute active panes per profile & expire finished rate-limit cooldowns across ALL entries.
        let mut active_by_profile: BTreeMap<String, Vec<(PaneId, ProjectId, Status)>> = BTreeMap::new();
        for pane in self.panes.values() {
            if !pane.alive() {
                continue;
            }
            let Some(agent) = pane.tracker.agent.filter(|a| profiles::supports_multi_profile(*a)) else {
                continue;
            };
            if let Some(prof_id) = self.resolved_pane_profile_id(pane, agent, &profiles) {
                active_by_profile.entry(prof_id).or_default().push((pane.id, pane.project, pane.tracker.status));
            }
        }

        let mut next_wake: Option<Duration> = None;
        let mut wake_at = |d: Duration| {
            next_wake = Some(next_wake.map_or(d, |w| w.min(d)));
        };
        let timer_visible = matches!(self.view, View::Profiles | View::Monitor)
            || self.monitor_popover
            || self.companion.is_some();

        for rt in self.profiles_rt.values_mut() {
            if let Some(reset_at) = rt.reset_at {
                if now >= reset_at {
                    rt.reset_at = None;
                    rt.reset_reason = None;
                    if matches!(rt.state, ProfileState::RateLimited | ProfileState::Cooldown) {
                        rt.state = ProfileState::Idle;
                    }
                } else {
                    let rem = reset_at.saturating_duration_since(now);
                    wake_at(if timer_visible { rem.min(Duration::from_secs(1)) } else { rem });
                }
            }
        }

        for prof in &profiles {
            let rt = self.profiles_rt.entry(prof.id.clone()).or_default();
            let panes = active_by_profile.get(&prof.id);
            rt.active_panes = panes.map(|v| v.iter().map(|(a, b, _)| (*a, *b)).collect()).unwrap_or_default();
            if !matches!(rt.state, ProfileState::RateLimited | ProfileState::Cooldown) {
                let running = panes.is_some_and(|v| v.iter().any(|(_, _, s)| matches!(s, Status::Running | Status::NeedsInput | Status::Unknown)));
                let failed = panes.is_some_and(|v| v.iter().any(|(_, _, s)| matches!(s, Status::Failed(_))));
                rt.state = if running {
                    ProfileState::Running
                } else if failed {
                    ProfileState::Error
                } else {
                    ProfileState::Idle
                };
            }
        }

        // Expire old pane rate-limit banners (auto-switched info banners dismiss after 12s; manual banners expire on reset).
        for pane in self.panes.values_mut() {
            if let Some(banner) = &pane.rate_limit {
                if banner.auto_switched_to.is_some() {
                    let age = now.duration_since(banner.created_at);
                    if age >= Duration::from_secs(12) {
                        pane.rate_limit = None;
                    } else {
                        wake_at(Duration::from_secs(12) - age);
                    }
                } else if now >= banner.reset_at {
                    pane.rate_limit = None;
                } else {
                    wake_at(banner.reset_at.saturating_duration_since(now).min(Duration::from_secs(1)));
                }
            }
        }

        // 2. Scan live agent panes (currently running an agent with multi-account profile support) for rate-limit output.
        let mut detected: Vec<(PaneId, RateLimitHit)> = Vec::new();
        for pane in self.panes.values_mut() {
            let in_agent = pane.alive()
                && pane.tracker.agent.is_some_and(profiles::supports_multi_profile)
                && !matches!(pane.tracker.fg, tc_core::status::Foreground::Shell | tc_core::status::Foreground::Gone);
            if !in_agent {
                continue;
            }
            if let Some(until) = pane.limit_ignore_until {
                if now < until {
                    wake_at(until.saturating_duration_since(now));
                    continue;
                }
                pane.limit_ignore_until = None;
            }
            let elapsed = now.duration_since(pane.last_limit_check);
            if elapsed < Duration::from_millis(1200) {
                if pane.last_output > pane.last_limit_check {
                    wake_at(Duration::from_millis(1200) - elapsed);
                }
                continue;
            }
            pane.last_limit_check = now;
            let tail = pane.screen_tail(28);
            if let Some(hit) = profiles::detect_rate_limit(&tail) {
                if pane.last_limit_sig.as_deref() != Some(hit.reason.as_str()) {
                    pane.last_limit_sig = Some(hit.reason.clone());
                    detected.push((pane.id, hit));
                }
            } else {
                pane.last_limit_sig = None;
            }
        }
        for (pane_id, hit) in detected {
            self.handle_rate_limit(ctx, pane_id, hit);
        }

        // 3. Background real token usage scan across every profile's config dir (every 20 s, bounded to 1 in-flight).
        if !self.usage_scan_running && self.usage_scan_at.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(20)) {
            self.usage_scan_at = Some(now);
            self.usage_scan_running = true;
            let home = paths::home_dir();
            let data = paths::data_dir().to_path_buf();
            let tx = self.app_tx.clone();
            let ctx = ctx.clone();
            let scan_list: Vec<(String, AgentKind, std::path::PathBuf)> = profiles
                .iter()
                .map(|p| (p.id.clone(), p.agent, profiles::resolved_config_dir(p, &home, &data)))
                .collect();
            self.rt.spawn_blocking(move || {
                // Deduplicate identical `(agent, canonical_root)` folders inside the background thread.
                let mut cache: std::collections::HashMap<(AgentKind, std::path::PathBuf), tc_agent::sessions::ProfileUsageReport> =
                    std::collections::HashMap::new();
                let mut results = Vec::with_capacity(scan_list.len());
                for (id, agent, root) in scan_list {
                    let key = (agent, paths::canonical(&root));
                    let usage = cache
                        .entry(key)
                        .or_insert_with(|| tc_agent::sessions::profile_usage(agent, &root))
                        .clone();
                    results.push((id, usage));
                }
                let _ = tx.send(AppMsg::ProfileUsages(results));
                ctx.request_repaint();
            });
        }

        // 4. Refresh project-nested agent session history for expanded projects (bounded to 3 in-flight, every 45 s).
        if !self.settings.sidebar_hidden && !self.zen_mode && self.sessions_running.len() < 3 {
            let slots = 3usize.saturating_sub(self.sessions_running.len());
            let to_scan: Vec<(ProjectId, std::path::PathBuf)> = self
                .projects
                .iter()
                .filter(|p| !p.is_worktree() && p.sessions_expanded && !self.sessions_running.contains(&p.id))
                .filter(|p| self.sessions_scan_at.get(&p.id).is_none_or(|t| now.duration_since(*t) >= Duration::from_secs(45)))
                .take(slots)
                .map(|p| (p.id, p.path.clone()))
                .collect();
            for (pid, path) in to_scan {
                self.refresh_project_sessions(pid, path);
            }
        }

        // 5. Publish live snapshot to connected Mobile Companion clients (throttled ~350 ms).
        if self.companion.is_some() {
            let elapsed = now.duration_since(self.companion_last_push);
            if elapsed >= Duration::from_millis(350) {
                self.companion_last_push = now;
                let snap = self.build_companion_snapshot();
                if let Some(c) = &self.companion {
                    c.publish(&snap);
                }
            } else if self.panes.values().any(|p| p.last_output > self.companion_last_push) {
                wake_at(Duration::from_millis(350) - elapsed);
            }
        }

        if let Some(d) = next_wake {
            ctx.request_repaint_after(d);
        }
    }

    /// Scans historical sessions for `project` across both default roots and every configured
    /// profile's isolated auth/config folder so the sidebar project tree shows all sessions.
    pub fn refresh_project_sessions(&mut self, project_id: ProjectId, project_path: std::path::PathBuf) {
        if !self.sessions_running.insert(project_id) {
            return;
        }
        self.sessions_scan_at.insert(project_id, Instant::now());
        let profiles = self.effective_profiles();
        let home = paths::home_dir();
        let data = paths::data_dir().to_path_buf();
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        let agent_home_override = std::env::var_os("TERNARYCODE_AGENT_HOME").filter(|_| cfg!(debug_assertions));

        self.rt.spawn_blocking(move || {
            let mut items: Vec<ProjectSessionItem> = Vec::new();
            let mut seen: HashSet<(AgentKind, String)> = HashSet::new();

            // Scan default roots first (preserving E2E `TERNARYCODE_AGENT_HOME` override).
            let default_roots = match &agent_home_override {
                Some(h) => tc_agent::sessions::Roots::under(Path::new(h)),
                None => tc_agent::sessions::Roots::from_env(&home),
            };
            for agent in [AgentKind::Claude, AgentKind::Codex, AgentKind::Antigravity] {
                let default_prof = if profiles::supports_multi_profile(agent) {
                    profiles
                        .iter()
                        .find(|p| p.agent == agent && p.use_default_auth && !profiles::has_custom_config_dir(p))
                        .map(|p| p.id.clone())
                } else {
                    None
                };
                for s in tc_agent::sessions::list(agent, &project_path, &default_roots, 8) {
                    if seen.insert((agent, s.id.clone())) {
                        items.push(ProjectSessionItem {
                            id: s.id,
                            title: s.title,
                            agent,
                            profile_id: default_prof.clone(),
                            status: Status::Idle,
                            updated: s.updated,
                            branch: s.branch,
                            live_pane: None,
                        });
                    }
                }
            }

            // Scan every isolated multi-account profile config folder.
            for prof in &profiles {
                if prof.use_default_auth && !profiles::has_custom_config_dir(prof) {
                    continue;
                }
                if !profiles::supports_multi_profile(prof.agent) || !tc_agent::sessions::listable(prof.agent) {
                    continue;
                }
                let dir = profiles::resolved_config_dir(prof, &home, &data);
                let roots = tc_agent::sessions::Roots {
                    claude: dir.clone(),
                    codex: dir.clone(),
                    gemini: dir.clone(),
                    antigravity: dir,
                };
                for s in tc_agent::sessions::list(prof.agent, &project_path, &roots, 8) {
                    if seen.insert((prof.agent, s.id.clone())) {
                        items.push(ProjectSessionItem {
                            id: s.id,
                            title: s.title,
                            agent: prof.agent,
                            profile_id: Some(prof.id.clone()),
                            status: Status::Idle,
                            updated: s.updated,
                            branch: s.branch,
                            live_pane: None,
                        });
                    }
                }
            }

            items.sort_by_key(|s| std::cmp::Reverse(s.updated));
            items.truncate(20);
            let _ = tx.send(AppMsg::ProjectSessions(project_id, items));
            ctx.request_repaint();
        });
    }

    /// Zero-allocation count of sessions (live agent panes + cached disk sessions) for a collapsed project row.
    pub fn project_sessions_count(&self, proj: &crate::project::Project) -> usize {
        let mut live_count = 0usize;
        for pane_id in proj.panes() {
            if let Some(pane) = self.panes.get(&pane_id)
                && pane.alive()
                && pane.tracker.agent.is_some()
            {
                live_count += 1;
            }
        }
        let disk_count = self.project_sessions.get(&proj.id).map_or(0, |disk| {
            disk.iter()
                .filter(|item| {
                    !proj.panes().into_iter().any(|pid| {
                        self.panes.get(&pid).is_some_and(|p| {
                            p.alive()
                                && p.tracker.agent == Some(item.agent)
                                && p.agent_session.as_deref() == Some(item.id.as_str())
                        })
                    })
                })
                .count()
        });
        live_count + disk_count
    }

    /// Merged live panes + historical disk sessions for `project_id`, newest / live first.
    pub fn merged_project_sessions(&self, project_id: ProjectId) -> Vec<ProjectSessionItem> {
        let mut out: Vec<ProjectSessionItem> = Vec::new();
        let mut live_ids: HashSet<(AgentKind, String)> = HashSet::new();
        if let Some(proj) = self.project(project_id) {
            let branch = proj.git.as_ref().map(|g| g.branch.clone());
            for pane_id in proj.panes() {
                let Some(pane) = self.panes.get(&pane_id) else { continue };
                if !pane.alive() {
                    continue;
                }
                let Some(agent) = pane.tracker.agent else { continue };
                let sid = pane.agent_session.clone().unwrap_or_else(|| format!("pane-{pane_id}"));
                live_ids.insert((agent, sid.clone()));
                let title = pane
                    .title
                    .clone()
                    .or_else(|| pane.task.clone())
                    .or_else(|| pane.program_title.clone())
                    .unwrap_or_else(|| pane.label());
                out.push(ProjectSessionItem {
                    id: sid,
                    title,
                    agent,
                    profile_id: profiles::supports_multi_profile(agent).then(|| pane.active_profile.clone()).flatten(),
                    status: pane.tracker.status,
                    updated: SystemTime::now(),
                    branch: branch.clone(),
                    live_pane: Some(pane_id),
                });
            }
        }
        if let Some(disk) = self.project_sessions.get(&project_id) {
            for item in disk {
                if !live_ids.contains(&(item.agent, item.id.clone())) {
                    out.push(item.clone());
                }
            }
        }
        out
    }

    /// Collects secret values from configured profiles for redaction before writing handoff files
    /// or streaming pane tails to Mobile Companions.
    fn known_profile_secrets(&self) -> Vec<String> {
        let mut secrets = Vec::new();
        for p in self.effective_profiles() {
            if let Some(k) = p.api_key.filter(|k| k.len() >= 8) {
                secrets.push(k);
            }
            for (k, v) in p.extra_env {
                let ku = k.to_ascii_uppercase();
                if v.len() >= 8
                    && (ku.contains("KEY") || ku.contains("TOKEN") || ku.contains("SECRET") || ku.contains("PASS") || ku.contains("AUTH"))
                {
                    secrets.push(v);
                }
            }
        }
        secrets
    }

    /// Handles a newly detected rate-limit pause on `pane_id`.
    pub fn handle_rate_limit(&mut self, ctx: &egui::Context, pane_id: PaneId, hit: RateLimitHit) {
        let now = Instant::now();
        let profiles = self.effective_profiles();
        let Some((project_id, agent, from_profile, session_id)) = self.panes.get(&pane_id).and_then(|p| {
            let agent = p.tracker.agent.or(p.last_agent)?;
            if !profiles::supports_multi_profile(agent) {
                return None;
            }
            let prof = self.resolved_pane_profile_id(p, agent, &profiles)?;
            Some((p.project, agent, prof, p.agent_session.clone()))
        }) else {
            return;
        };

        let cooldown = hit
            .retry_after
            .unwrap_or_else(|| Duration::from_secs(u64::from(self.settings.failover.default_cooldown_mins) * 60));
        let reset_at = now + cooldown;

        let rt = self.profiles_rt.entry(from_profile.clone()).or_default();
        rt.state = ProfileState::RateLimited;
        rt.reset_at = Some(reset_at);
        rt.reset_reason = Some(hit.reason.clone());

        self.push_agent_log(
            LogLevel::Warn,
            Some(project_id),
            Some(pane_id),
            Some(from_profile.clone()),
            format!("Rate limit detected on {from_profile}: {}", hit.reason),
        );

        if self.settings.failover.mode == FailoverMode::Auto
            && let Some(next) = profiles::pick_failover_profile(
                &profiles,
                &self.profiles_rt,
                &from_profile,
                agent,
                &self.settings.failover,
                now,
            )
        {
            let next_id = next.id.clone();
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.rate_limit = Some(RateLimitBanner {
                    from_profile: from_profile.clone(),
                    agent,
                    reason: hit.reason.clone(),
                    reset_at,
                    created_at: now,
                    session_id,
                    auto_switched_to: Some(next_id.clone()),
                });
            }
            self.switch_pane_profile_and_resume(ctx, pane_id, &next_id, true);
            return;
        }

        // Manual Resume mode (or no healthy fallback profile left): pause cleanly with inline banner
        // and force NeedsInput even when hooks are active so desktop & mobile alerts fire.
        if let Some(pane) = self.panes.get_mut(&pane_id) {
            pane.rate_limit = Some(RateLimitBanner {
                from_profile: from_profile.clone(),
                agent,
                reason: hit.reason.clone(),
                reset_at,
                created_at: now,
                session_id,
                auto_switched_to: None,
            });
            let _ = pane.tracker.apply(
                tc_core::status::Event::RateLimited {
                    message: Some(format!("{from_profile} hit usage limit")),
                },
                now,
            );
        }
        self.toast_with(
            ToastKind::Warn,
            tf("{} hit its usage limit. Choose another account to resume.", &[&from_profile]),
            Some((t("Accounts").into(), Action::OpenProfiles)),
        );
        self.push_companion_now();
    }

    /// Switches `pane_id` to `target_profile_id`, copies session history or writes a context
    /// handoff file, interrupts any stalled run, and resumes execution from the stopping point.
    pub fn switch_pane_profile_and_resume(
        &mut self,
        ctx: &egui::Context,
        pane_id: PaneId,
        target_profile_id: &str,
        auto: bool,
    ) {
        let Some(target) = self.profile_by_id(target_profile_id).filter(|p| p.enabled) else {
            self.toast_err(tf("Profile {} not found or disabled.", &[target_profile_id]));
            return;
        };
        self.wake(ctx, pane_id);
        let profiles = self.effective_profiles();
        let Some((project_id, project_path, from_profile_id, from_agent, session_id, task, tail, running)) =
            self.panes.get(&pane_id).map(|p| {
                let path = self.project(p.project).map(|pr| pr.path.clone()).unwrap_or_else(|| p.cwd.clone());
                let from_agent = p.tracker.agent.or(p.last_agent).unwrap_or(target.agent);
                let from_id = self
                    .resolved_pane_profile_id(p, from_agent, &profiles)
                    .unwrap_or_else(|| from_agent.slug().to_string());
                let sid = p.agent_session.clone().or_else(|| p.rate_limit.as_ref().and_then(|b| b.session_id.clone()));
                (
                    p.project,
                    path,
                    from_id,
                    from_agent,
                    sid,
                    p.task.clone(),
                    p.screen_tail(50),
                    !matches!(p.tracker.fg, tc_core::status::Foreground::Shell),
                )
            })
        else {
            return;
        };

        let home = paths::home_dir();
        let data = paths::data_dir();
        let to_root = profiles::resolved_config_dir(&target, &home, data);

        // If switching between two accounts of the same provider (e.g. claude1 -> claude2), copy
        // the session file across isolated config roots so native `--resume <id>` keeps full history.
        let mut copied_session = false;
        if let Some(sid) = session_id.as_deref()
            && from_agent == target.agent
        {
            let from_root = self
                .profile_by_id(&from_profile_id)
                .map(|p| profiles::resolved_config_dir(&p, &home, data))
                .unwrap_or_else(|| {
                    let default_roots = tc_agent::sessions::Roots::from_env(&home);
                    match from_agent {
                        AgentKind::Claude => default_roots.claude,
                        AgentKind::Codex => default_roots.codex,
                        AgentKind::Antigravity => default_roots.antigravity,
                        AgentKind::Gemini => default_roots.gemini,
                        _ => to_root.clone(),
                    }
                });
            copied_session = tc_agent::sessions::copy_session_between_roots(
                target.agent,
                &project_path,
                &from_root,
                &to_root,
                sid,
            );
        }

        let preset = self.preset_for_agent(target.agent);
        let native_resume = session_id
            .as_deref()
            .filter(|_| from_agent == target.agent && copied_session)
            .and_then(|sid| tc_agent::sessions::resume_command(target.agent, &preset.command, sid));

        // Write a redacted handoff summary only when native same-provider resume is unavailable.
        let base_cmd = if let Some(rc) = native_resume {
            rc
        } else {
            let secrets = self.known_profile_secrets();
            let safe_tail = profiles::redact_secrets(&tail, &secrets);
            let safe_task = task.as_deref().map(|s| profiles::redact_secrets(s, &secrets));
            if let Some(handoff) = profiles::write_handoff_context(data, pane_id, &from_profile_id, safe_task.as_deref(), &safe_tail) {
                let prompt = format!(
                    "Continue the interrupted task from {} (handoff context saved at {})",
                    from_profile_id,
                    handoff.display()
                );
                #[cfg(windows)]
                let q = crate::platform::shell_quote(&prompt);
                #[cfg(not(windows))]
                let q = integrate::shell_quote(&prompt);
                match target.agent {
                    AgentKind::Claude | AgentKind::Codex | AgentKind::Gemini => format!("{} {q}", preset.command.trim()),
                    AgentKind::Antigravity => format!("{} -i {q}", preset.command.trim()),
                    _ => preset.command.clone(),
                }
            } else {
                preset.command.clone()
            }
        };

        let typed = self.launch_command_for_profile(&preset, &base_cmd, Some(&target));

        let now = Instant::now();
        if let Some(pane) = self.panes.get_mut(&pane_id) {
            if running {
                // Cleanly interrupt the paused/rate-limited CLI so the shell takes the failover launch.
                pane.input(vec![0x03, 0x03, b'\r']);
                pane.last_output = now;
            }
            pane.active_profile = Some(target.id.clone());
            pane.preset = Some(preset.id.clone());
            // Keep `last_limit_sig` and hold scanning for 6s so stale 429 output on screen never cascades!
            pane.limit_ignore_until = Some(now + Duration::from_secs(6));
            if !copied_session {
                pane.agent_session = None;
                pane.last_agent = Some(target.agent);
            }
            if !auto {
                pane.rate_limit = None;
            }
            pane.type_command(typed);
        }
        if let Some(proj) = self.project_mut(project_id) {
            proj.active_profile = Some(target.id.clone());
        }

        let msg = if auto {
            tf("{} hit limit → auto-resumed on {}", &[&from_profile_id, &target.id])
        } else {
            tf("Switched to {} and resumed session.", &[&target.id])
        };
        self.push_agent_log(
            LogLevel::Failover,
            Some(project_id),
            Some(pane_id),
            Some(target.id.clone()),
            msg.clone(),
        );
        if auto {
            self.toast_with(ToastKind::Ok, msg, Some((t("Show").into(), Action::FocusPane(pane_id))));
        } else {
            self.toast_with(ToastKind::Ok, msg, None);
            self.ensure_terminals_visible();
            self.focus_pane(pane_id);
        }
        self.push_companion_now();
    }

    /// Resumes a session directly from the project-nested sidebar tree (or Mobile Companion) in
    /// one click without a pre-launch modal.
    pub fn resume_project_session(
        &mut self,
        ctx: &egui::Context,
        project: ProjectId,
        agent: AgentKind,
        session_id: String,
        profile_id: Option<String>,
    ) {
        if self.project(project).is_none() || session_id.starts_with("pane-") {
            return;
        }
        // Focus live pane immediately if already open.
        if let Some(live) = self
            .panes
            .values()
            .find(|p| p.alive() && p.project == project && p.tracker.agent == Some(agent) && p.agent_session.as_deref() == Some(session_id.as_str()))
            .map(|p| p.id)
        {
            self.ensure_terminals_visible();
            self.focus_pane(live);
            return;
        }
        self.active = Some(project);
        self.show_focus();
        self.ensure_terminals_visible();
        let valid_profile_id = if profiles::supports_multi_profile(agent) {
            profile_id
                .as_deref()
                .and_then(|id| self.profile_by_id(id))
                .filter(|p| p.enabled && p.agent == agent)
                .map(|p| p.id)
        } else {
            None
        };
        if let Some(prof_id) = &valid_profile_id
            && let Some(p) = self.project_mut(project)
        {
            p.active_profile = Some(prof_id.clone());
        }
        let preset = self.preset_for_agent(agent);
        self.run_agent_preset_with_profile(ctx, preset, Launch::Session(session_id), Some(0), valid_profile_id);
    }

    /// Starts a new session directly inside `project` with an optional chosen profile.
    pub fn new_project_session(&mut self, ctx: &egui::Context, project: ProjectId, profile_id: Option<String>) {
        if self.project(project).is_none() {
            return;
        }
        self.active = Some(project);
        self.show_focus();
        self.ensure_terminals_visible();
        let now = Instant::now();
        let all = self.effective_profiles();
        let proj_pref = self.project(project).and_then(|p| p.active_profile.clone());
        let profile = profile_id
            .as_deref()
            .and_then(|id| all.iter().find(|p| p.id == id && p.enabled))
            .or_else(|| {
                proj_pref.as_deref().and_then(|id| {
                    all.iter().find(|p| {
                        p.id == id && p.enabled && self.profiles_rt.get(&p.id).is_none_or(|rt| rt.is_healthy(now))
                    })
                })
            })
            .or_else(|| {
                all.iter().find(|p| p.enabled && self.profiles_rt.get(&p.id).is_none_or(|rt| rt.is_healthy(now)))
            })
            .or_else(|| all.iter().find(|p| p.enabled))
            .cloned();
        let agent = profile.as_ref().map(|p| p.agent).unwrap_or(AgentKind::Claude);
        let preset = self.preset_for_agent(agent);
        self.run_agent_preset_with_profile(ctx, preset, Launch::New, Some(0), profile.map(|p| p.id));
    }

    /// Runs an interactive login session inside a terminal pane with `profile_id`'s isolated auth
    /// directory (`CLAUDE_CONFIG_DIR` / `CODEX_HOME` / `AGY_DATA_DIR`).
    pub fn login_profile(&mut self, ctx: &egui::Context, profile_id: &str) {
        let Some(profile) = self.profile_by_id(profile_id) else { return };
        if self.active.is_none() {
            self.toast(t("Add a project first.").into());
            return;
        }
        let env = profiles::profile_env(&profile, &paths::home_dir(), paths::data_dir());
        let mut unset = profiles::profile_unset_env(&profile);
        unset.retain(|k| !env.contains_key(*k));
        let cmd = integrate::with_env_and_unset(profiles::login_command(profile.agent), &env, &unset);
        self.show_focus();
        self.ensure_terminals_visible();
        if let Some(pane) = self.new_tab(ctx) {
            if let Some(p) = self.panes.get_mut(&pane) {
                p.active_profile = Some(profile.id.clone());
                p.title = Some(format!("Auth · {}", profile.id));
                p.type_command(cmd);
            }
            self.focus_pane(pane);
        }
    }

    /// Triggers an async connection & auth health check for `profile_id`.
    pub fn test_profile(&mut self, profile_id: String) {
        let Some(profile) = self.profile_by_id(&profile_id) else { return };
        self.profiles_rt.entry(profile_id.clone()).or_default().connection = Some(ConnectionTest::Testing);
        let home = paths::home_dir();
        let data = paths::data_dir().to_path_buf();
        #[cfg(windows)]
        let path_env = crate::cli::pane_path();
        #[cfg(not(windows))]
        let path_env = self.lsp.path_env.clone();
        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || {
            let res = profiles::test_profile_connection(&profile, &home, &data, path_env.as_ref());
            let _ = tx.send(AppMsg::ProfileTest(profile_id, res));
            ctx.request_repaint();
        });
    }

    /// Starts or stops the embedded Mobile Companion server based on `settings.companion.enabled`.
    pub fn sync_companion_server(&mut self) {
        if self.settings.companion.enabled {
            if self.companion.as_ref().is_some_and(|c| c.port != self.settings.companion.port) {
                self.companion = None;
            }
            if self.companion.is_none() {
                match CompanionHandle::start(
                    self.rt.handle(),
                    self.settings.companion.port,
                    &self.settings.companion.devices,
                    self.app_tx.clone(),
                    self.ctx.clone(),
                ) {
                    Ok(handle) => {
                        self.companion = Some(handle);
                        self.push_companion_now();
                    }
                    Err(e) => {
                        self.settings.companion.enabled = false;
                        self.toast_err(e);
                    }
                }
            } else if let Some(c) = &self.companion {
                c.sync_devices(&self.settings.companion.devices);
            }
        } else {
            self.companion = None;
        }
    }

    pub fn push_companion_now(&mut self) {
        if self.companion.is_some() {
            let snap = self.build_companion_snapshot();
            if let Some(c) = &self.companion {
                c.publish(&snap);
            }
            self.companion_last_push = Instant::now();
        }
    }

    /// Handles a remote command sent from a paired smartphone via the Mobile Companion API.
    pub fn handle_companion_cmd(&mut self, ctx: &egui::Context, cmd: CompanionCmd) {
        match cmd {
            CompanionCmd::SendInput { pane, text, enter } => {
                self.wake(ctx, pane);
                if let Some(p) = self.panes.get_mut(&pane) {
                    let mut bytes = text.into_bytes();
                    if enter {
                        bytes.push(b'\r');
                        p.entered(Instant::now());
                    }
                    p.input(bytes);
                }
                self.push_companion_now();
            }
            CompanionCmd::QuickAction { pane, kind } => {
                self.wake(ctx, pane);
                if let Some(p) = self.panes.get_mut(&pane) {
                    let now = Instant::now();
                    match kind {
                        QuickRemoteAction::Approve => {
                            p.entered(now);
                            p.input(b"\r".to_vec());
                        }
                        QuickRemoteAction::Reject => {
                            p.input(vec![0x1b]);
                        }
                        QuickRemoteAction::Interrupt => {
                            p.input(vec![0x03]);
                        }
                    }
                }
                self.push_companion_now();
            }
            CompanionCmd::SwitchProfile { pane, profile_id } => {
                self.switch_pane_profile_and_resume(ctx, pane, &profile_id, false);
            }
            CompanionCmd::ResumeSession { project, agent, session_id, profile_id } => {
                self.resume_project_session(ctx, project, agent, session_id, profile_id);
            }
            CompanionCmd::DevicePaired(dev) => {
                let name = dev.name.clone();
                self.settings.companion.devices.push(dev);
                self.settings_changed();
                if let Some(c) = &self.companion {
                    c.sync_devices(&self.settings.companion.devices);
                }
                self.toast_ok(tf("Paired mobile device: {}", &[&name]));
            }
            CompanionCmd::DeviceSeen { id, ip, at } => {
                let mut changed = false;
                if let Some(dev) = self.settings.companion.devices.iter_mut().find(|d| d.id == id) {
                    if dev.ip != ip || at.saturating_sub(dev.last_seen) >= 60 {
                        changed = true;
                    }
                    dev.ip = ip;
                    dev.last_seen = at;
                }
                if changed {
                    self.settings_changed();
                }
            }
        }
    }

    /// Builds a JSON-serializable snapshot of workspace projects, sessions, live streaming output,
    /// profile telemetry, and alerts for the Mobile Companion.
    pub fn build_companion_snapshot(&self) -> CompanionSnapshot {
        let now = Instant::now();
        let profiles_cfg = self.effective_profiles();
        let secrets = self.known_profile_secrets();
        let projects: Vec<CompanionProject> = self
            .projects
            .iter()
            .filter(|p| !p.is_worktree())
            .map(|pr| {
                let sessions = self
                    .merged_project_sessions(pr.id)
                    .into_iter()
                    .take(10)
                    .map(|s| CompanionSession {
                        id: s.id,
                        title: s.title,
                        agent: s.agent.slug().into(),
                        profile: s.profile_id,
                        status: status_slug(s.status).into(),
                        age: if s.live_pane.is_some() {
                            "live".into()
                        } else {
                            crate::ui::dialogs::ago(s.updated)
                        },
                        live_pane: s.live_pane,
                    })
                    .collect();
                CompanionProject {
                    id: pr.id,
                    name: pr.name.clone(),
                    branch: pr.git.as_ref().map(|g| g.branch.clone()),
                    sessions,
                }
            })
            .collect();

        let now_unix = profiles::now_unix_secs();
        let mut panes: Vec<CompanionPane> = self
            .panes
            .values()
            .map(|p| {
                let proj = self.project(p.project);
                let rate_limit = p
                    .rate_limit
                    .as_ref()
                    .filter(|b| b.auto_switched_to.is_none() && b.reset_at > now)
                    .map(|b| {
                        let rem = b.reset_at.saturating_duration_since(now);
                        let secs = rem.as_secs() + u64::from(rem.subsec_nanos() > 0);
                        CompanionRateLimit {
                            from_profile: b.from_profile.clone(),
                            reason: b.reason.clone(),
                            resets_at_unix: now_unix.saturating_add(secs as i64),
                        }
                    });
                let output = profiles::redact_secrets(&p.screen_tail(45), &secrets);
                let supports_profiles = p
                    .tracker
                    .agent
                    .or(p.last_agent)
                    .is_some_and(profiles::supports_multi_profile);
                CompanionPane {
                    id: p.id,
                    project_id: p.project,
                    project_name: self.project_label(p.project),
                    label: p.label(),
                    branch: proj.and_then(|x| x.git.as_ref()).map(|g| g.branch.clone()),
                    profile: supports_profiles.then(|| p.active_profile.clone()).flatten(),
                    supports_profiles,
                    status: status_slug(p.tracker.status).into(),
                    status_label: crate::ui::status::text(p.tracker.status),
                    message: p.tracker.message.clone(),
                    output,
                    rate_limit,
                }
            })
            .collect();
        panes.sort_by_key(|p| (p.rate_limit.is_none(), p.status != "needs_input", p.status != "running", p.id));

        let profiles: Vec<CompanionProfile> = profiles_cfg
            .iter()
            .map(|prof| {
                let rt = self.profiles_rt.get(&prof.id).cloned().unwrap_or_default();
                let active_session = rt.active_panes.first().map(|(_, pid)| self.project_label(*pid));
                let reset_unix_5h = rt
                    .usage
                    .window_5h
                    .resets_at_unix
                    .filter(|&r| r > now_unix)
                    .or_else(|| rt.reset_remaining_secs(now).map(|s| now_unix.saturating_add(s as i64)));
                let reset_unix_wk = rt.usage.window_weekly.resets_at_unix.filter(|&r| r > now_unix);
                let window_5h = rt.has_window_5h(prof).then(|| crate::companion::CompanionWindowQuota {
                    label: rt.window_5h_label(),
                    used_pct: rt.quota_5h_used_pct(prof),
                    tokens: rt.usage.window_5h.tokens.total(),
                    resets_at_unix: reset_unix_5h,
                });
                let window_weekly = rt.has_window_weekly(prof).then(|| crate::companion::CompanionWindowQuota {
                    label: rt.window_weekly_label(),
                    used_pct: rt.quota_weekly_used_pct(prof),
                    tokens: rt.usage.window_weekly.tokens.total(),
                    resets_at_unix: reset_unix_wk,
                });
                CompanionProfile {
                    id: prof.id.clone(),
                    name: prof.name.clone(),
                    provider: prof.agent.label().into(),
                    account_hint: prof.account_hint.clone(),
                    plan_type: rt.usage.plan_type.clone(),
                    signed_in: if prof.api_key.is_some() { Some(true) } else { rt.usage.signed_in },
                    state: rt.state.slug().into(),
                    enabled: prof.enabled,
                    tokens_in: rt.tokens_in,
                    tokens_out: rt.tokens_out,
                    quota_pct: rt.quota_remaining_pct(prof),
                    resets_at_unix: rt.reset_remaining_secs(now).map(|s| now_unix.saturating_add(s as i64)),
                    window_5h,
                    window_weekly,
                    active_session,
                }
            })
            .collect();

        let alerts: Vec<CompanionAlert> = self
            .agent_logs
            .iter()
            .rev()
            .take(30)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|e| CompanionAlert {
                time: e.at.format("%H:%M:%S").to_string(),
                level: match e.level {
                    LogLevel::Info => "info",
                    LogLevel::Tool => "tool",
                    LogLevel::Warn => "warn",
                    LogLevel::Err => "error",
                    LogLevel::Failover => "failover",
                }
                .into(),
                profile: e.profile.clone(),
                message: e.message.clone(),
            })
            .collect();

        CompanionSnapshot {
            failover_mode: match self.settings.failover.mode {
                FailoverMode::Auto => "auto".into(),
                FailoverMode::Manual => "manual".into(),
            },
            projects,
            panes,
            profiles,
            alerts,
        }
    }

    /// Recomputes and caches deduplicated token/window totals across enabled profiles off-frame
    /// (zero per-frame `fs::canonicalize` syscalls during UI rendering).
    pub fn recompute_usage_totals(&mut self) {
        let home = paths::home_dir();
        let data = paths::data_dir();
        let mut seen_roots: HashSet<(AgentKind, std::path::PathBuf)> = HashSet::new();
        let mut total_in = 0u64;
        let mut total_out = 0u64;
        let mut total_5h = 0u64;
        let mut total_weekly = 0u64;
        for p in self.effective_profiles() {
            if !p.enabled {
                continue;
            }
            let dir = profiles::resolved_config_dir(&p, &home, data);
            let canon = paths::canonical(&dir);
            if !seen_roots.insert((p.agent, canon)) {
                continue;
            }
            if let Some(rt) = self.profiles_rt.get(&p.id) {
                total_in = total_in.saturating_add(rt.tokens_in);
                total_out = total_out.saturating_add(rt.tokens_out);
                if rt.has_window_5h(&p) {
                    total_5h = total_5h.saturating_add(rt.usage.window_5h.tokens.total());
                }
                if rt.has_window_weekly(&p) {
                    total_weekly = total_weekly.saturating_add(rt.usage.window_weekly.tokens.total());
                }
            }
        }
        self.cached_usage_totals = (total_in, total_out, total_5h, total_weekly);
    }

    /// Cached deduplicated 24h input/output tokens across enabled profiles (O(1), zero syscalls).
    pub fn deduped_token_totals(&self) -> (u64, u64) {
        (self.cached_usage_totals.0, self.cached_usage_totals.1)
    }

    /// Cached deduplicated (5-hour, weekly/long) token totals across enabled profiles (O(1)).
    pub fn deduped_window_totals(&self) -> (u64, u64) {
        (self.cached_usage_totals.2, self.cached_usage_totals.3)
    }

    /// Picks the best next healthy pre-registered account to one-click switch to from `pane_id`
    /// (prefers signed-in accounts of the same provider first, e.g. `chatgpt1` -> `chatgpt2`).
    pub fn next_switch_profile(&self, pane_id: PaneId) -> Option<AgentProfile> {
        let now = Instant::now();
        let pane = self.panes.get(&pane_id)?;
        let agent = pane.tracker.agent.or(pane.last_agent)?;
        if !profiles::supports_multi_profile(agent) {
            return None;
        }
        let profiles = self.effective_profiles();
        let active_id = self.resolved_pane_profile_id(pane, agent, &profiles)?;
        let mut candidates: Vec<&AgentProfile> = profiles
            .iter()
            .filter(|p| p.enabled && p.id != active_id)
            .filter(|p| self.settings.failover.cross_provider || p.agent == agent)
            .filter(|p| self.profiles_rt.get(&p.id).is_none_or(|rt| rt.is_healthy(now)))
            .collect();
        candidates.sort_by_key(|p| {
            let rt = self.profiles_rt.get(&p.id);
            let not_signed_in = rt.is_some_and(|r| r.auth_status(p) == profiles::AuthStatus::NotSignedIn);
            let used_5h = rt.and_then(|r| r.quota_5h_used_pct(p)).unwrap_or(0.0).round() as u32;
            (p.agent != agent, not_signed_in, used_5h >= 95, p.fallback_order, p.id.as_str())
        });
        candidates.into_iter().next().cloned()
    }

    /// Convenience summary metrics for the top-bar Usage & Status telemetry pill.
    pub fn telemetry_summary(&self) -> (usize, usize, usize, u64) {
        let profiles = self.effective_profiles();
        let mut running = 0usize;
        let mut limited = 0usize;
        let mut errors = 0usize;
        for p in &profiles {
            if !p.enabled {
                continue;
            }
            if let Some(rt) = self.profiles_rt.get(&p.id) {
                match rt.state {
                    ProfileState::Running => running += 1,
                    ProfileState::RateLimited | ProfileState::Cooldown => limited += 1,
                    ProfileState::Error => errors += 1,
                    ProfileState::Idle => {}
                }
            }
        }
        // Also count live running agent panes even if not mapped to a custom profile.
        let active_panes = self
            .panes
            .values()
            .filter(|p| p.alive() && matches!(p.tracker.status, Status::Running | Status::NeedsInput) && p.tracker.agent.is_some())
            .count();
        let (win_5h, _) = self.deduped_window_totals();
        let (tin, tout) = self.deduped_token_totals();
        let tokens = if win_5h > 0 { win_5h } else { tin.saturating_add(tout) };
        (running.max(active_panes), limited, errors, tokens)
    }

    pub fn is_dock_right(&self) -> bool {
        self.settings.drawer_dock == DrawerDock::Right
    }
}

fn status_slug(s: Status) -> &'static str {
    match s {
        Status::Idle => "idle",
        Status::Running => "running",
        Status::NeedsInput => "needs_input",
        Status::Failed(_) => "failed",
        Status::Done => "done",
        Status::CommandDone => "done",
        Status::Unknown => "running",
        Status::Ended => "idle",
    }
}

/// Helper to check whether `View` is one of the ADE management views (Profiles / Monitor / Pairing).
pub fn is_management_view(v: View) -> bool {
    matches!(v, View::Profiles | View::Monitor | View::Pairing)
}
