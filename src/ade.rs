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
use tc_core::model::{AgentProfile, DrawerDock, DrawerTab, FailoverMode, Integration, View};
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

/// Saved layout snapshot restored when exiting Focus / Zen Mode.
#[derive(Clone, Copy, Debug)]
pub struct ZenSaved {
    pub sidebar_hidden: bool,
    pub sidebar_compact: bool,
    pub drawer_open: bool,
    pub editor_visible: bool,
}

/// Editable form draft in the Profiles & Accounts management view.
#[derive(Clone, Debug)]
pub struct ProfileDraft {
    /// Original profile ID when editing an existing profile (`None` when creating a new one).
    pub editing_id: Option<String>,
    pub id: String,
    pub name: String,
    pub agent: AgentKind,
    pub config_dir: String,
    pub api_key: String,
    pub reveal_key: bool,
    pub extra_env_text: String,
    pub budget_text: String,
    pub use_default_auth: bool,
    pub enabled: bool,
}

impl ProfileDraft {
    pub fn new_for(agent: AgentKind, existing: &[AgentProfile]) -> Self {
        let prefix = match agent {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "chatgpt",
            AgentKind::Gemini => "gemini",
            AgentKind::OpenCode => "opencode",
            AgentKind::Antigravity => "agy",
            AgentKind::Custom => "agent",
        };
        let mut idx = 1u32;
        while existing.iter().any(|p| p.id.eq_ignore_ascii_case(&format!("{prefix}{idx}"))) {
            idx += 1;
        }
        let id = format!("{prefix}{idx}");
        Self {
            editing_id: None,
            name: format!("{} {idx}", agent.label()),
            id,
            agent,
            config_dir: String::new(),
            api_key: String::new(),
            reveal_key: false,
            extra_env_text: String::new(),
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
            agent: p.agent,
            config_dir: p.config_dir.as_ref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
            api_key: p.api_key.clone().unwrap_or_default(),
            reveal_key: false,
            extra_env_text,
            budget_text: p.daily_token_budget.map(|b| b.to_string()).unwrap_or_default(),
            use_default_auth: p.use_default_auth,
            enabled: p.enabled,
        }
    }

    pub fn into_profile(self, fallback_order: u32) -> AgentProfile {
        let id = profiles::sanitize_id(self.id.trim());
        let name = if self.name.trim().is_empty() { id.clone() } else { self.name.trim().to_string() };
        let mut extra_env = BTreeMap::new();
        for line in self.extra_env_text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                if !k.is_empty() {
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
        let daily_token_budget = self.budget_text.trim().replace([',', '_', ' '], "").parse::<u64>().ok().filter(|&v| v > 0);
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
            use_default_auth: self.use_default_auth,
        }
    }
}

impl App {
    /// Configured multi-account profiles, falling back to starter defaults (`claude1`, `chatgpt1`,
    /// `gemini1`) until customized.
    pub fn effective_profiles(&self) -> Vec<AgentProfile> {
        let mut list = if self.settings.profiles.is_empty() {
            profiles::default_profiles()
        } else {
            self.settings.profiles.clone()
        };
        list.sort_by_key(|p| (p.fallback_order, p.id.clone()));
        list
    }

    /// Materializes default profiles into `settings.profiles` before mutating.
    pub fn ensure_mutable_profiles(&mut self) {
        if self.settings.profiles.is_empty() {
            self.settings.profiles = profiles::default_profiles();
        }
    }

    pub fn profile_by_id(&self, id: &str) -> Option<AgentProfile> {
        self.effective_profiles().into_iter().find(|p| p.id == id)
    }

    /// Picks the highest-priority enabled healthy profile for `agent` (or first enabled profile).
    pub fn preferred_profile_for(&self, agent: AgentKind, project: Option<ProjectId>) -> Option<AgentProfile> {
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

    /// Builds a shell launch command for `preset` combining hooks, permissions flag, preset env,
    /// and isolated account profile env (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_CLI_HOME`, API keys).
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
            command = format!("{command} {flag}");
        }
        let command = integrate::launch_command_with(&command, preset.agent, integration, self.codex_hooks, self.codex_own_notify);
        let mut env = preset.env.clone();
        if let Some(profile) = profile {
            let penv = profiles::profile_env(profile, &paths::home_dir(), paths::data_dir());
            env.extend(penv);
        }
        integrate::with_env(&command, &env)
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

    /// Ensures the project's editor exists and respects `settings.editor_first` (wide code editor
    /// open by default).
    pub fn ensure_editor_for_project(&mut self, id: ProjectId, saved_width: f32) {
        let visible = self.settings.editor_first;
        let width = if saved_width > 0.0 { saved_width.clamp(0.45, 0.85) } else { 0.64 };
        self.editors.entry(id).or_insert_with(|| crate::editor::Editor {
            visible,
            width,
            ..Default::default()
        });
    }

    /// Enters or exits one-keystroke Focus / Zen Mode, preserving exact prior panel visibility.
    pub fn toggle_zen_mode(&mut self) {
        if self.zen_mode {
            self.zen_mode = false;
            if let Some(saved) = self.zen_saved.take() {
                self.settings.sidebar_hidden = saved.sidebar_hidden;
                self.settings.sidebar_compact = saved.sidebar_compact;
                self.settings.drawer_open = saved.drawer_open;
                if let Some(e) = self.editor_mut() {
                    e.visible = saved.editor_visible;
                }
            }
        } else {
            let editor_visible = self.active.and_then(|id| self.editors.get(&id)).is_some_and(|e| e.visible);
            self.zen_saved = Some(ZenSaved {
                sidebar_hidden: self.settings.sidebar_hidden,
                sidebar_compact: self.settings.sidebar_compact,
                drawer_open: self.settings.drawer_open,
                editor_visible,
            });
            self.zen_mode = true;
            self.settings.sidebar_hidden = true;
            self.settings.drawer_open = false;
            self.monitor_popover = false;
            self.show_focus();
            if let Some(e) = self.editor_mut() {
                e.visible = true;
            }
        }
        self.settings_changed();
    }

    /// Periodic ADE background tick: updates profile states, expires cooldowns, scans agent panes
    /// for rate limits, refreshes real token telemetry & project-nested session history, and
    /// publishes updates to connected Mobile Companions.
    pub fn ade_tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let profiles = self.effective_profiles();

        // 1. Recompute active panes per profile & expire finished rate-limit cooldowns.
        let mut active_by_profile: BTreeMap<String, Vec<(PaneId, ProjectId, Status)>> = BTreeMap::new();
        for pane in self.panes.values() {
            let pid = pane.active_profile.clone().or_else(|| {
                pane.tracker.agent.and_then(|a| {
                    profiles.iter().find(|p| p.agent == a && p.enabled).map(|p| p.id.clone())
                })
            });
            if let Some(prof_id) = pid {
                active_by_profile.entry(prof_id).or_default().push((pane.id, pane.project, pane.tracker.status));
            }
        }

        let mut next_wake: Option<Duration> = None;
        for prof in &profiles {
            let rt = self.profiles_rt.entry(prof.id.clone()).or_default();
            if let Some(reset_at) = rt.reset_at {
                if now >= reset_at {
                    rt.reset_at = None;
                    rt.reset_reason = None;
                    if matches!(rt.state, ProfileState::RateLimited | ProfileState::Cooldown) {
                        rt.state = ProfileState::Idle;
                    }
                } else {
                    let rem = reset_at.saturating_duration_since(now).min(Duration::from_secs(1));
                    next_wake = Some(next_wake.map_or(rem, |w| w.min(rem)));
                }
            }
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

        // 2. Scan active agent panes for rate-limit / usage-limit output (~1.2 s interval).
        let mut detected: Vec<(PaneId, RateLimitHit)> = Vec::new();
        for pane in self.panes.values_mut() {
            let has_agent = pane.tracker.agent.is_some() || pane.active_profile.is_some();
            if !has_agent || !pane.alive() || now.duration_since(pane.last_limit_check) < Duration::from_millis(1200) {
                continue;
            }
            pane.last_limit_check = now;
            let tail = pane.screen_tail(28);
            if let Some(hit) = profiles::detect_rate_limit(&tail) {
                let sig = format!("{}:{}", pane.active_profile.as_deref().unwrap_or(""), hit.reason);
                if pane.last_limit_sig.as_deref() != Some(sig.as_str()) {
                    pane.last_limit_sig = Some(sig);
                    detected.push((pane.id, hit));
                }
            }
        }
        for (pane_id, hit) in detected {
            self.handle_rate_limit(ctx, pane_id, hit);
        }

        // 3. Background real token usage scan across every profile's config dir (every 20 s).
        if self.usage_scan_at.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(20)) {
            self.usage_scan_at = Some(now);
            let home = paths::home_dir();
            let data = paths::data_dir().to_path_buf();
            let tx = self.app_tx.clone();
            let ctx = ctx.clone();
            let scan_list: Vec<(String, AgentKind, std::path::PathBuf)> = profiles
                .iter()
                .map(|p| (p.id.clone(), p.agent, profiles::resolved_config_dir(p, &home, &data)))
                .collect();
            self.rt.spawn_blocking(move || {
                for (id, agent, root) in scan_list {
                    let usage = tc_agent::sessions::recent_usage(agent, &root);
                    let _ = tx.send(AppMsg::ProfileUsage(id, usage));
                }
                ctx.request_repaint();
            });
        }

        // 4. Refresh project-nested agent session history for expanded projects (every 15 s).
        let to_scan: Vec<(ProjectId, std::path::PathBuf)> = self
            .projects
            .iter()
            .filter(|p| !p.is_worktree() && p.sessions_expanded)
            .filter(|p| self.sessions_scan_at.get(&p.id).is_none_or(|t| now.duration_since(*t) >= Duration::from_secs(15)))
            .take(4)
            .map(|p| (p.id, p.path.clone()))
            .collect();
        for (pid, path) in to_scan {
            self.refresh_project_sessions(pid, path);
        }

        // 5. Publish live snapshot to connected Mobile Companion clients (throttled ~350 ms).
        if self.companion.is_some() && now.duration_since(self.companion_last_push) >= Duration::from_millis(350) {
            self.companion_last_push = now;
            let snap = self.build_companion_snapshot();
            if let Some(c) = &self.companion {
                c.publish(&snap);
            }
        }

        if let Some(d) = next_wake {
            ctx.request_repaint_after(d);
        }
    }

    /// Scans historical sessions for `project` across both default roots and every configured
    /// profile's isolated auth/config folder so the sidebar project tree shows all sessions.
    pub fn refresh_project_sessions(&mut self, project_id: ProjectId, project_path: std::path::PathBuf) {
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
            for agent in [AgentKind::Claude, AgentKind::Codex, AgentKind::Gemini] {
                let default_prof = profiles.iter().find(|p| p.agent == agent && p.use_default_auth).map(|p| p.id.clone());
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
                if prof.use_default_auth && prof.config_dir.is_none() {
                    continue;
                }
                if !tc_agent::sessions::listable(prof.agent) {
                    continue;
                }
                let dir = profiles::resolved_config_dir(prof, &home, &data);
                let roots = tc_agent::sessions::Roots {
                    claude: dir.clone(),
                    codex: dir.clone(),
                    gemini: dir,
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
            items.truncate(14);
            let _ = tx.send(AppMsg::ProjectSessions(project_id, items));
            ctx.request_repaint();
        });
    }

    /// Merged live panes + historical disk sessions for `project_id`, newest / live first.
    pub fn merged_project_sessions(&self, project_id: ProjectId) -> Vec<ProjectSessionItem> {
        let mut out: Vec<ProjectSessionItem> = Vec::new();
        let mut live_ids: HashSet<String> = HashSet::new();
        if let Some(proj) = self.project(project_id) {
            let branch = proj.git.as_ref().map(|g| g.branch.clone());
            for pane_id in proj.panes() {
                let Some(pane) = self.panes.get(&pane_id) else { continue };
                let Some(agent) = pane.tracker.agent.or(pane.last_agent) else { continue };
                let sid = pane.agent_session.clone().unwrap_or_else(|| format!("pane-{pane_id}"));
                live_ids.insert(sid.clone());
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
                    profile_id: pane.active_profile.clone(),
                    status: pane.tracker.status,
                    updated: SystemTime::now(),
                    branch: branch.clone(),
                    live_pane: Some(pane_id),
                });
            }
        }
        if let Some(disk) = self.project_sessions.get(&project_id) {
            for item in disk {
                if !live_ids.contains(&item.id) {
                    out.push(item.clone());
                }
            }
        }
        out
    }

    /// Handles a newly detected rate-limit pause on `pane_id`.
    pub fn handle_rate_limit(&mut self, ctx: &egui::Context, pane_id: PaneId, hit: RateLimitHit) {
        let now = Instant::now();
        let Some((project_id, agent, from_profile, session_id)) = self.panes.get(&pane_id).map(|p| {
            let agent = p.tracker.agent.or(p.last_agent).unwrap_or(AgentKind::Claude);
            let prof = p.active_profile.clone().unwrap_or_else(|| {
                self.preferred_profile_for(agent, Some(p.project))
                    .map(|x| x.id)
                    .unwrap_or_else(|| agent.slug().to_string())
            });
            (p.project, agent, prof, p.agent_session.clone())
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

        let profiles = self.effective_profiles();
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
                    session_id,
                    auto_switched_to: Some(next_id.clone()),
                });
            }
            self.switch_pane_profile_and_resume(ctx, pane_id, &next_id, true);
            return;
        }

        // Manual Resume mode (or no healthy fallback profile left): pause cleanly with inline banner.
        if let Some(pane) = self.panes.get_mut(&pane_id) {
            pane.rate_limit = Some(RateLimitBanner {
                from_profile: from_profile.clone(),
                agent,
                reason: hit.reason.clone(),
                reset_at,
                session_id,
                auto_switched_to: None,
            });
            let _ = pane.tracker.apply(
                tc_core::status::Event::Notify {
                    message: Some(format!("{from_profile} hit usage limit")),
                },
                now,
            );
        }
        self.toast_with(
            ToastKind::Warn,
            tf("{} hit its usage limit. Choose another profile to resume.", &[&from_profile]),
            Some((t("Profiles").into(), Action::OpenProfiles)),
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
        let Some(target) = self.profile_by_id(target_profile_id) else {
            self.toast_err(tf("Profile {} not found.", &[target_profile_id]));
            return;
        };
        self.wake(ctx, pane_id);
        let Some((project_id, project_path, from_profile_id, from_agent, session_id, task, tail, running)) =
            self.panes.get(&pane_id).map(|p| {
                let path = self.project(p.project).map(|pr| pr.path.clone()).unwrap_or_else(|| p.cwd.clone());
                let from_agent = p.tracker.agent.or(p.last_agent).unwrap_or(target.agent);
                let from_id = p.active_profile.clone().unwrap_or_else(|| from_agent.slug().to_string());
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
        // the session JSONL file across isolated config roots so `--resume <id>` keeps full history.
        let mut copied_session = false;
        if let Some(sid) = session_id.as_deref()
            && from_agent == target.agent
        {
            let from_root = self
                .profile_by_id(&from_profile_id)
                .map(|p| profiles::resolved_config_dir(&p, &home, data))
                .unwrap_or_else(|| match from_agent {
                    AgentKind::Claude => home.join(".claude"),
                    AgentKind::Codex => home.join(".codex"),
                    AgentKind::Gemini => home.join(".gemini"),
                    _ => to_root.clone(),
                });
            copied_session = tc_agent::sessions::copy_session_between_roots(
                target.agent,
                &project_path,
                &from_root,
                &to_root,
                sid,
            );
        }

        // Write a concise handoff summary for cross-provider failover or fresh sessions.
        let handoff_file = profiles::write_handoff_context(data, pane_id, &from_profile_id, task.as_deref(), &tail);

        let presets = self.presets();
        let preset = presets
            .into_iter()
            .find(|p| p.agent == Some(target.agent))
            .unwrap_or(Preset {
                id: target.agent.slug().into(),
                name: target.agent.label().into(),
                agent: Some(target.agent),
                command: profiles::default_command(target.agent).into(),
                resume: None,
                env: BTreeMap::new(),
                builtin: false,
            });

        let base_cmd = if let Some(sid) = session_id.as_deref().filter(|_| from_agent == target.agent && copied_session)
            && let Some(rc) = tc_agent::sessions::resume_command(target.agent, &preset.command, sid)
        {
            rc
        } else if let Some(handoff) = &handoff_file {
            let prompt = format!(
                "Continue the interrupted task from {} (handoff context saved at {})",
                from_profile_id,
                handoff.display()
            );
            let q = integrate::shell_quote(&prompt);
            match target.agent {
                AgentKind::Claude | AgentKind::Codex | AgentKind::Gemini => format!("{} {q}", preset.command.trim()),
                _ => preset.command.clone(),
            }
        } else {
            preset.command.clone()
        };

        let typed = self.launch_command_for_profile(&preset, &base_cmd, Some(&target));

        if let Some(pane) = self.panes.get_mut(&pane_id) {
            if running {
                // Cleanly interrupt the paused/rate-limited CLI so the shell takes the failover launch.
                pane.input(vec![0x03, 0x03, b'\r']);
            }
            pane.active_profile = Some(target.id.clone());
            pane.preset = Some(preset.id.clone());
            pane.last_limit_sig = None;
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
        self.toast_with(ToastKind::Ok, msg, None);
        self.settings.drawer_open = true;
        self.drawer_tab = DrawerTab::Terminals;
        self.focus_pane(pane_id);
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
        // Focus live pane immediately if already open.
        if let Some(live) = self
            .panes
            .values()
            .find(|p| p.project == project && p.agent_session.as_deref() == Some(session_id.as_str()))
            .map(|p| p.id)
        {
            self.settings.drawer_open = true;
            self.drawer_tab = DrawerTab::Terminals;
            self.focus_pane(live);
            return;
        }
        self.active = Some(project);
        self.show_focus();
        self.settings.drawer_open = true;
        self.drawer_tab = DrawerTab::Terminals;
        if let Some(prof_id) = &profile_id
            && let Some(p) = self.project_mut(project)
        {
            p.active_profile = Some(prof_id.clone());
        }
        let preset_id = self
            .presets()
            .into_iter()
            .find(|p| p.agent == Some(agent))
            .map(|p| p.id)
            .unwrap_or_else(|| agent.slug().to_string());
        self.run_preset_with_profile(ctx, &preset_id, Launch::Session(session_id), Some(0), profile_id);
    }

    /// Starts a new session directly inside `project` with an optional chosen profile.
    pub fn new_project_session(&mut self, ctx: &egui::Context, project: ProjectId, profile_id: Option<String>) {
        self.active = Some(project);
        self.show_focus();
        self.settings.drawer_open = true;
        self.drawer_tab = DrawerTab::Terminals;
        let profile = profile_id
            .as_deref()
            .and_then(|id| self.profile_by_id(id))
            .or_else(|| self.effective_profiles().into_iter().find(|p| p.enabled));
        let agent = profile.as_ref().map(|p| p.agent).unwrap_or(AgentKind::Claude);
        let preset_id = self
            .presets()
            .into_iter()
            .find(|p| p.agent == Some(agent))
            .map(|p| p.id)
            .unwrap_or_else(|| "claude".into());
        self.run_preset_with_profile(ctx, &preset_id, Launch::New, Some(0), profile.map(|p| p.id));
    }

    /// Runs an interactive login session inside a terminal pane with `profile_id`'s isolated auth
    /// directory (`CLAUDE_CONFIG_DIR` / `CODEX_HOME` / `GEMINI_CLI_HOME`).
    pub fn login_profile(&mut self, ctx: &egui::Context, profile_id: &str) {
        let Some(profile) = self.profile_by_id(profile_id) else { return };
        if self.active.is_none() {
            self.toast(t("Add a project first.").into());
            return;
        }
        let env = profiles::profile_env(&profile, &paths::home_dir(), paths::data_dir());
        let cmd = integrate::with_env(profiles::login_command(profile.agent), &env);
        self.show_focus();
        self.settings.drawer_open = true;
        self.drawer_tab = DrawerTab::Terminals;
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
            if self.companion.is_none() {
                match CompanionHandle::start(
                    self.rt.handle(),
                    self.settings.companion.port,
                    &self.settings.companion.devices,
                    self.app_tx.clone(),
                    self.ctx.clone(),
                ) {
                    Ok(handle) => {
                        self.settings.companion.port = handle.port;
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
                if let Some(dev) = self.settings.companion.devices.iter_mut().find(|d| d.id == id) {
                    dev.ip = ip;
                    dev.last_seen = at;
                }
            }
        }
    }

    /// Builds a JSON-serializable snapshot of workspace projects, sessions, live streaming output,
    /// profile telemetry, and alerts for the Mobile Companion.
    pub fn build_companion_snapshot(&self) -> CompanionSnapshot {
        let now = Instant::now();
        let profiles_cfg = self.effective_profiles();
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

        let mut panes: Vec<CompanionPane> = self
            .panes
            .values()
            .map(|p| {
                let proj = self.project(p.project);
                let rate_limit = p.rate_limit.as_ref().filter(|b| b.auto_switched_to.is_none()).map(|b| CompanionRateLimit {
                    from_profile: b.from_profile.clone(),
                    reason: b.reason.clone(),
                    reset_secs: b.reset_at.saturating_duration_since(now).as_secs(),
                });
                CompanionPane {
                    id: p.id,
                    project_id: p.project,
                    project_name: self.project_label(p.project),
                    label: p.label(),
                    branch: proj.and_then(|x| x.git.as_ref()).map(|g| g.branch.clone()),
                    profile: p.active_profile.clone(),
                    status: status_slug(p.tracker.status).into(),
                    status_label: crate::ui::status::text(p.tracker.status),
                    message: p.tracker.message.clone(),
                    output: p.screen_tail(45),
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
                CompanionProfile {
                    id: prof.id.clone(),
                    name: prof.name.clone(),
                    provider: prof.agent.label().into(),
                    state: rt.state.slug().into(),
                    enabled: prof.enabled,
                    tokens_in: rt.tokens_in,
                    tokens_out: rt.tokens_out,
                    quota_pct: rt.quota_remaining_pct(prof.daily_token_budget),
                    reset_secs: rt.reset_remaining_secs(now).unwrap_or(0),
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
                    LogLevel::Info => "idle",
                    LogLevel::Tool => "running",
                    LogLevel::Warn => "needs_input",
                    LogLevel::Err => "error",
                    LogLevel::Failover => "done",
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

    /// Convenience summary metrics for the top-bar Usage & Status telemetry pill.
    pub fn telemetry_summary(&self) -> (usize, usize, usize, u64) {
        let profiles = self.effective_profiles();
        let mut running = 0usize;
        let mut limited = 0usize;
        let mut errors = 0usize;
        let mut total_tokens = 0u64;
        for p in &profiles {
            let rt = self.profiles_rt.get(&p.id);
            if let Some(rt) = rt {
                total_tokens = total_tokens.saturating_add(rt.tokens_in).saturating_add(rt.tokens_out);
                match rt.state {
                    ProfileState::Running => running += 1,
                    ProfileState::RateLimited | ProfileState::Cooldown => limited += 1,
                    ProfileState::Error => errors += 1,
                    ProfileState::Idle => {}
                }
            }
        }
        // Also count running agent panes even if not mapped to a custom profile.
        let active_panes = self
            .panes
            .values()
            .filter(|p| matches!(p.tracker.status, Status::Running | Status::NeedsInput) && p.tracker.agent.is_some())
            .count();
        (running.max(active_panes), limited, errors, total_tokens)
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
