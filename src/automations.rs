//! Scheduled and recurring background Automations for TernaryCode.
//!
//! Each automation runs on a local cron/interval/one-shot schedule (or manual `Run now`) in a
//! managed background session off the UI thread — never cluttering the user's main project tabs.
//! Supports isolated Git worktrees (auto-cleaned when clean, preserved for 1-click Review/Focus
//! when changes exist), multi-account rate-limit auto-failover, quiet-when-clean Inbox triage,
//! overlap guards, max-runtime timeouts, and clean cancellation.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Local, LocalResult, TimeZone, Timelike};
use eframe::egui;
use serde::{Deserialize, Serialize};
use tc_core::agent::AgentKind;
use tc_core::model::{AgentProfile, WorktreeSetup};
use tc_core::ProjectId;

use crate::app::{App, AppMsg, ToastKind};
use crate::i18n::{t, tf};
use crate::paths;
use crate::profiles::{self, LogLevel};

/// Maximum historical runs stored per automation in `automations.json`.
pub const MAX_RUNS_PER_AUTOMATION: usize = 30;
/// Maximum log lines retained per run.
pub const MAX_LOG_LINES_PER_RUN: usize = 400;

/// How an automation is scheduled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleKind {
    Interval { every_minutes: u32 },
    Daily { hour: u8, minute: u8 },
    Weekdays { hour: u8, minute: u8 },
    /// `weekday`: 0 = Mon .. 6 = Sun.
    Weekly { weekday: u8, hour: u8, minute: u8 },
    Cron { expr: String },
    Once { at_epoch_secs: i64 },
}

impl Default for ScheduleKind {
    fn default() -> Self {
        Self::Daily { hour: 2, minute: 0 }
    }
}

impl ScheduleKind {
    pub fn weekday_short(weekday: u8) -> &'static str {
        match weekday % 7 {
            0 => "Mon",
            1 => "Tue",
            2 => "Wed",
            3 => "Thu",
            4 => "Fri",
            5 => "Sat",
            _ => "Sun",
        }
    }

    pub fn cron_string(&self) -> String {
        match self {
            Self::Interval { every_minutes } => {
                let m = (*every_minutes).max(1);
                if m < 60 {
                    format!("*/{m} * * * *")
                } else if m % 60 == 0 {
                    format!("0 */{} * * *", (m / 60).max(1))
                } else {
                    format!("every {m}m")
                }
            }
            Self::Daily { hour, minute } => format!("{minute} {hour} * * *"),
            Self::Weekdays { hour, minute } => format!("{minute} {hour} * * 1-5"),
            Self::Weekly { weekday, hour, minute } => {
                let dow = (weekday % 7 + 1) % 7; // 0=Mon -> 1, ..., 6=Sun -> 0
                format!("{minute} {hour} * * {dow}")
            }
            Self::Cron { expr } => expr.trim().to_string(),
            Self::Once { at_epoch_secs } => format_local_hm(*at_epoch_secs),
        }
    }

    pub fn short_label(&self) -> String {
        match self {
            Self::Interval { every_minutes } => {
                let m = (*every_minutes).max(1);
                if m % 60 == 0 {
                    format!("Every {}h", m / 60)
                } else {
                    format!("Every {m}m")
                }
            }
            Self::Daily { hour, minute } => format!("Daily · {:02}:{:02}", hour % 24, minute % 60),
            Self::Weekdays { hour, minute } => format!("Weekdays · {:02}:{:02}", hour % 24, minute % 60),
            Self::Weekly { weekday, hour, minute } => {
                format!("Weekly {} · {:02}:{:02}", Self::weekday_short(*weekday), hour % 24, minute % 60)
            }
            Self::Cron { expr } => format!("Cron · {}", expr.trim()),
            Self::Once { at_epoch_secs } => format!("Once · {}", format_local_short(*at_epoch_secs)),
        }
    }

    pub fn preview_sentence(&self) -> String {
        let tz = local_tz_label();
        match self {
            Self::Interval { every_minutes } => {
                let m = (*every_minutes).max(1);
                if m % 60 == 0 {
                    let h = m / 60;
                    if h == 1 {
                        format!("Runs every hour ({tz})")
                    } else {
                        format!("Runs every {h} hours ({tz})")
                    }
                } else {
                    format!("Runs every {m} minutes ({tz})")
                }
            }
            Self::Daily { hour, minute } => format!("Runs every day at {:02}:{:02} ({tz})", hour % 24, minute % 60),
            Self::Weekdays { hour, minute } => format!("Runs Mon–Fri at {:02}:{:02} ({tz})", hour % 24, minute % 60),
            Self::Weekly { weekday, hour, minute } => {
                format!("Runs every {} at {:02}:{:02} ({tz})", Self::weekday_short(*weekday), hour % 24, minute % 60)
            }
            Self::Cron { expr } => match CronExpr::parse(expr) {
                Ok(_) => format!("Custom cron `{}` ({tz})", expr.trim()),
                Err(e) => format!("Invalid cron: {e}"),
            },
            Self::Once { at_epoch_secs } => format!("Runs once at {} ({tz}), then pauses", format_local_short(*at_epoch_secs)),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Interval { every_minutes } if *every_minutes == 0 => Err("Interval must be at least 1 minute.".into()),
            Self::Daily { hour, minute } | Self::Weekdays { hour, minute } | Self::Weekly { hour, minute, .. }
                if *hour > 23 || *minute > 59 =>
            {
                Err("Time must be HH:MM (00:00–23:59).".into())
            }
            Self::Cron { expr } => CronExpr::parse(expr).map(|_| ()),
            _ => Ok(()),
        }
    }

    /// Computes the next Unix epoch seconds strictly after `after_epoch` in the local timezone.
    pub fn next_after(&self, after_epoch: i64) -> Option<i64> {
        match self {
            Self::Interval { every_minutes } => {
                let step = i64::from((*every_minutes).max(1)) * 60;
                let base = (after_epoch / 60) * 60;
                Some(base + step)
            }
            Self::Daily { hour, minute } => {
                CronExpr::parse(&format!("{} {} * * *", minute % 60, hour % 24)).ok()?.next_after(after_epoch)
            }
            Self::Weekdays { hour, minute } => {
                CronExpr::parse(&format!("{} {} * * 1-5", minute % 60, hour % 24)).ok()?.next_after(after_epoch)
            }
            Self::Weekly { weekday, hour, minute } => {
                let dow = (weekday % 7 + 1) % 7;
                CronExpr::parse(&format!("{} {} * * {dow}", minute % 60, hour % 24)).ok()?.next_after(after_epoch)
            }
            Self::Cron { expr } => CronExpr::parse(expr).ok()?.next_after(after_epoch),
            Self::Once { at_epoch_secs } => (*at_epoch_secs > after_epoch).then_some(*at_epoch_secs),
        }
    }
}

/// Standard 5-field cron expression: `min hour dom month dow`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CronExpr {
    minutes: BTreeSet<u32>,
    hours: BTreeSet<u32>,
    doms: BTreeSet<u32>,
    months: BTreeSet<u32>,
    dows: BTreeSet<u32>,
    dom_any: bool,
    dow_any: bool,
}

impl CronExpr {
    pub fn parse(input: &str) -> Result<Self, String> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.len() != 5 {
            return Err("Expected 5 fields: min hour dom month dow".into());
        }
        let minutes = parse_cron_field(parts[0], 0, 59, false)?;
        let hours = parse_cron_field(parts[1], 0, 23, false)?;
        let dom_any = parts[2] == "*";
        let doms = parse_cron_field(parts[2], 1, 31, false)?;
        let months = parse_cron_field(parts[3], 1, 12, false)?;
        let dow_any = parts[4] == "*";
        let dows = parse_cron_field(parts[4], 0, 7, true)?;
        Ok(Self { minutes, hours, doms, months, dows, dom_any, dow_any })
    }

    pub fn matches_day(&self, month: u32, dom: u32, dow_sun0: u32) -> bool {
        if !self.months.contains(&month) {
            return false;
        }
        if self.dom_any && self.dow_any {
            true
        } else if self.dom_any {
            self.dows.contains(&dow_sun0)
        } else if self.dow_any {
            self.doms.contains(&dom)
        } else {
            // Standard Vixie cron: if both dom and dow are restricted, either matching fires.
            self.doms.contains(&dom) || self.dows.contains(&dow_sun0)
        }
    }

    /// Finds the next local timestamp strictly after `after_epoch` (bounded to 4 years).
    pub fn next_after(&self, after_epoch: i64) -> Option<i64> {
        let start_epoch = (after_epoch / 60 + 1) * 60;
        let start = Local.timestamp_opt(start_epoch, 0).single()?;
        let mut year = start.year();
        let max_year = year + 4;
        let mut month = start.month();
        let mut day = start.day();
        let mut hour = start.hour();
        let mut minute = start.minute();

        while year <= max_year {
            if !self.months.contains(&month) {
                month += 1;
                if month > 12 {
                    month = 1;
                    year += 1;
                }
                day = 1;
                hour = 0;
                minute = 0;
                continue;
            }
            let days_in_m = days_in_month(year, month);
            if day > days_in_m {
                month += 1;
                if month > 12 {
                    month = 1;
                    year += 1;
                }
                day = 1;
                hour = 0;
                minute = 0;
                continue;
            }
            let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
            let dow_sun0 = date.weekday().num_days_from_sunday();
            if !self.matches_day(month, day, dow_sun0) {
                day += 1;
                hour = 0;
                minute = 0;
                continue;
            }
            let Some(&h) = self.hours.range(hour..).next() else {
                day += 1;
                hour = 0;
                minute = 0;
                continue;
            };
            if h > hour {
                hour = h;
                minute = 0;
            }
            let Some(&m) = self.minutes.range(minute..).next() else {
                hour += 1;
                minute = 0;
                if hour > 23 {
                    day += 1;
                    hour = 0;
                }
                continue;
            };
            if let Some(naive) = date.and_hms_opt(hour, m, 0) {
                let ts = match Local.from_local_datetime(&naive) {
                    LocalResult::Single(dt) => Some(dt.timestamp()),
                    LocalResult::Ambiguous(a, _) => Some(a.timestamp()),
                    LocalResult::None => None,
                };
                if let Some(ts) = ts
                    && ts > after_epoch
                {
                    return Some(ts);
                }
            }
            minute = m + 1;
            if minute > 59 {
                minute = 0;
                hour += 1;
                if hour > 23 {
                    hour = 0;
                    day += 1;
                }
            }
        }
        None
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let next = chrono::NaiveDate::from_ymd_opt(ny, nm, 1).unwrap();
    (next - first).num_days() as u32
}

fn parse_cron_field(field: &str, min: u32, max: u32, map_seven_to_zero: bool) -> Result<BTreeSet<u32>, String> {
    let mut set = BTreeSet::new();
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err("Empty cron segment".into());
        }
        let (range_part, step) = match part.split_once('/') {
            Some((r, s)) => {
                let st: u32 = s.parse().map_err(|_| format!("Invalid step `{s}`"))?;
                if st == 0 {
                    return Err("Step cannot be 0".into());
                }
                (r, st)
            }
            None => (part, 1),
        };
        let (start, end) = if range_part == "*" {
            (min, if map_seven_to_zero { 6 } else { max })
        } else if let Some((a, b)) = range_part.split_once('-') {
            let a: u32 = a.parse().map_err(|_| format!("Invalid number `{a}`"))?;
            let b: u32 = b.parse().map_err(|_| format!("Invalid number `{b}`"))?;
            if a < min || b > max || a > b {
                return Err(format!("Range `{range_part}` out of bounds {min}–{max}"));
            }
            (a, b)
        } else {
            let v: u32 = range_part.parse().map_err(|_| format!("Invalid number `{range_part}`"))?;
            if v < min || v > max {
                return Err(format!("Value `{v}` out of bounds {min}–{max}"));
            }
            if part.contains('/') {
                (v, if map_seven_to_zero { 6 } else { max })
            } else {
                (v, v)
            }
        };
        let mut cur = start;
        while cur <= end {
            set.insert(if map_seven_to_zero && cur == 7 { 0 } else { cur });
            cur = cur.saturating_add(step);
        }
    }
    if set.is_empty() {
        return Err(format!("No values matched in `{field}`"));
    }
    Ok(set)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    #[default]
    Fresh,
    Continue,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMode {
    #[default]
    Worktree,
    ProjectDir,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapPolicy {
    #[default]
    Skip,
    Queue,
    CancelPrevious,
}

impl OverlapPolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Skip => "Skip if running",
            Self::Queue => "Queue next",
            Self::CancelPrevious => "Cancel previous",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissedPolicy {
    #[default]
    Skip,
    RunOnWake,
}

impl MissedPolicy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Skip => "Skip while asleep",
            Self::RunOnWake => "Run once on wake",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTrigger {
    #[default]
    Scheduled,
    Manual,
    OnWake,
}

impl RunTrigger {
    pub fn label(self) -> &'static str {
        match self {
            Self::Scheduled => "Scheduled",
            Self::Manual => "Manual",
            Self::OnWake => "On wake",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    NeedsReview,
    #[default]
    Clean,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::NeedsReview => "Needs review",
            Self::Clean => "Clean",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}

/// One historical or active execution of an automation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomationRun {
    pub id: String,
    pub run_number: u32,
    pub trigger: RunTrigger,
    pub status: RunStatus,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    pub duration_secs: u64,
    pub account_used: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub workspace_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_base_commit: Option<String>,
    pub files_changed: usize,
    pub additions: u32,
    pub deletions: u32,
    pub summary_title: String,
    pub summary_body: String,
    pub log_lines: Vec<String>,
    pub reviewed: bool,
    pub notified_inbox: bool,
}

impl AutomationRun {
    /// Whether this run currently demands the user's attention in Inbox / Automations.
    pub fn unreviewed_alert(&self) -> bool {
        self.notified_inbox
            && !self.reviewed
            && matches!(self.status, RunStatus::NeedsReview | RunStatus::Failed)
    }
}

/// Saved automation definition + bounded run history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Automation {
    pub id: String,
    pub name: String,
    pub project_path: PathBuf,
    pub project_name: String,
    pub agent: AgentKind,
    /// `None` = Auto-failover across enabled profiles for `agent`; `Some(id)` = pinned profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    pub schedule: ScheduleKind,
    pub prompt: String,
    pub enabled: bool,
    pub session_mode: SessionMode,
    pub workspace_mode: WorkspaceMode,
    pub quiet_when_clean: bool,
    pub overlap: OverlapPolicy,
    pub timeout_mins: u32,
    pub missed: MissedPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_triggered_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<i64>,
    pub runs: Vec<AutomationRun>,
}

impl Default for Automation {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            project_path: PathBuf::new(),
            project_name: String::new(),
            agent: AgentKind::Claude,
            profile_id: None,
            schedule: ScheduleKind::default(),
            prompt: String::new(),
            enabled: false,
            session_mode: SessionMode::Fresh,
            workspace_mode: WorkspaceMode::Worktree,
            quiet_when_clean: true,
            overlap: OverlapPolicy::Skip,
            timeout_mins: 30,
            missed: MissedPolicy::Skip,
            last_session_id: None,
            last_triggered_at: None,
            next_run_at: None,
            runs: Vec::new(),
        }
    }
}

impl Automation {
    pub fn latest_run(&self) -> Option<&AutomationRun> {
        self.runs.first()
    }

    pub fn is_running(&self) -> bool {
        self.runs.first().is_some_and(|r| r.status == RunStatus::Running)
    }

    pub fn has_unreviewed_alert(&self) -> bool {
        self.runs.iter().any(AutomationRun::unreviewed_alert)
    }

    pub fn status(&self, active_running: bool) -> AutomationStatus {
        if active_running || self.is_running() {
            AutomationStatus::Running
        } else {
            self.display_status()
        }
    }

    pub fn display_status(&self) -> AutomationStatus {
        if self.is_running() {
            AutomationStatus::Running
        } else if self.runs.first().is_some_and(|r| r.status == RunStatus::Failed && !r.reviewed) {
            AutomationStatus::Failed
        } else if self.has_unreviewed_alert() {
            AutomationStatus::NeedsReview
        } else if !self.enabled {
            AutomationStatus::Paused
        } else {
            AutomationStatus::Clean
        }
    }

    pub fn recompute_next(&mut self, now_epoch: i64) {
        if !self.enabled {
            self.next_run_at = None;
        } else {
            self.next_run_at = self.schedule.next_after(now_epoch);
        }
    }

    pub fn next_run_number(&self) -> u32 {
        self.runs.iter().map(|r| r.run_number).max().unwrap_or(0) + 1
    }

    pub fn agent_label(&self) -> &'static str {
        automation_agent_label(self.agent)
    }

    pub fn account_label(&self) -> String {
        if self.agent == AgentKind::Custom {
            return "Shell".into();
        }
        match &self.profile_id {
            Some(id) if !id.is_empty() => id.clone(),
            _ if profiles::supports_profile(self.agent) => "Auto-failover".into(),
            _ => "Default".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationStatus {
    Running,
    NeedsReview,
    Failed,
    Clean,
    Paused,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomationsSave {
    pub version: u32,
    pub items: Vec<Automation>,
}

/// Starter template preset shown in Automations → Templates.
#[derive(Clone, Copy, Debug)]
pub struct AutomationTemplate {
    pub name: &'static str,
    pub desc: &'static str,
    pub agent: AgentKind,
    pub cadence_kind: CadenceKind,
    pub time_text: &'static str,
    pub session_mode: SessionMode,
    pub workspace_mode: WorkspaceMode,
    pub quiet_when_clean: bool,
    pub prompt: &'static str,
}

impl AutomationTemplate {
    pub fn cadence_summary(&self) -> String {
        match self.cadence_kind {
            CadenceKind::Interval => format!("Every {}m", self.time_text),
            CadenceKind::Daily => format!("Daily {}", self.time_text),
            CadenceKind::Weekdays => format!("Weekdays {}", self.time_text),
            CadenceKind::Weekly => format!("Weekly {}", self.time_text),
            CadenceKind::Cron => self.time_text.to_string(),
            CadenceKind::Once => format!("Once {}", self.time_text),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CadenceKind {
    Interval,
    #[default]
    Daily,
    Weekdays,
    Weekly,
    Cron,
    Once,
}

impl CadenceKind {
    pub const ALL: [Self; 6] = [
        Self::Interval,
        Self::Daily,
        Self::Weekdays,
        Self::Weekly,
        Self::Cron,
        Self::Once,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Interval => "Interval",
            Self::Daily => "Daily",
            Self::Weekdays => "Weekdays",
            Self::Weekly => "Weekly",
            Self::Cron => "Custom cron",
            Self::Once => "One-shot",
        }
    }
}

pub const STARTER_TEMPLATES: &[AutomationTemplate] = &[
    AutomationTemplate {
        name: "Morning PR & CI Triage",
        desc: "Checks open pull requests, failing CI workflows, and unanswered review comments every weekday morning.",
        agent: AgentKind::Antigravity,
        cadence_kind: CadenceKind::Weekdays,
        time_text: "09:00",
        session_mode: SessionMode::Fresh,
        workspace_mode: WorkspaceMode::ProjectDir,
        quiet_when_clean: true,
        prompt: "Check open pull requests and failed GitHub Actions workflows for this repository. Summarize actionable failures, root causes, and reviewers waiting on a response. Do not edit files; finish quietly if all checks are green.",
    },
    AutomationTemplate {
        name: "Nightly Test & Flaky Fix",
        desc: "Runs the full test and lint suite overnight in an isolated Git worktree and drafts minimal fixes for failures.",
        agent: AgentKind::Claude,
        cadence_kind: CadenceKind::Daily,
        time_text: "02:00",
        session_mode: SessionMode::Fresh,
        workspace_mode: WorkspaceMode::Worktree,
        quiet_when_clean: true,
        prompt: "Run the repository's lint and test suite. If any warning or test fails, diagnose the root cause, fix it minimally in this isolated worktree, re-run verification, and summarize the diff. If everything passes, report clean with no file changes.",
    },
    AutomationTemplate {
        name: "Weekly Dependency & CVE Audit",
        desc: "Audits outdated packages and security advisories every Monday morning in a safe worktree branch.",
        agent: AgentKind::Codex,
        cadence_kind: CadenceKind::Weekly,
        time_text: "08:30",
        session_mode: SessionMode::Fresh,
        workspace_mode: WorkspaceMode::Worktree,
        quiet_when_clean: true,
        prompt: "Audit repository dependencies for known vulnerabilities and security updates. Group findings by severity. Apply safe patch-level security fixes inside this isolated worktree and verify builds and tests pass.",
    },
    AutomationTemplate {
        name: "Staging Error Rate Watch",
        desc: "Stateful recurring check that continues the same session to track error trends and regressions.",
        agent: AgentKind::Claude,
        cadence_kind: CadenceKind::Interval,
        time_text: "240",
        session_mode: SessionMode::Continue,
        workspace_mode: WorkspaceMode::ProjectDir,
        quiet_when_clean: true,
        prompt: "Inspect recent deployment and error logs. Compare against the previous run in this session and report only new anomalies, rising 5xx rates, or regressions.",
    },
    AutomationTemplate {
        name: "Weekly Changelog & Docs Sync",
        desc: "Reviews merged commits from the past 7 days and drafts updated README / changelog entries in a worktree.",
        agent: AgentKind::Claude,
        cadence_kind: CadenceKind::Weekly,
        time_text: "17:00",
        session_mode: SessionMode::Fresh,
        workspace_mode: WorkspaceMode::Worktree,
        quiet_when_clean: false,
        prompt: "Review git commits merged in the last 7 days. Identify user-facing features or CLI changes not yet documented in README / CHANGELOG and draft concise updates in this isolated worktree.",
    },
];

/// Modal form draft for creating or editing an automation.
#[derive(Clone, Debug)]
pub struct AutomationDraft {
    pub editing_id: Option<String>,
    pub nl_mode: bool,
    pub nl_input: String,
    pub name: String,
    pub project_path: PathBuf,
    pub project_name: String,
    pub agent: AgentKind,
    pub profile_id: Option<String>,
    pub cadence_kind: CadenceKind,
    /// `HH:MM` for Daily/Weekdays/Weekly, minutes (e.g. `"240"`) for Interval, cron string for
    /// Cron, or minutes-from-now / `HH:MM` for Once.
    pub time_text: String,
    pub weekday: u8,
    pub prompt: String,
    pub session_mode: SessionMode,
    pub workspace_mode: WorkspaceMode,
    pub quiet_when_clean: bool,
    pub overlap: OverlapPolicy,
    pub timeout_mins: u32,
    pub missed: MissedPolicy,
    pub enabled: bool,
}

impl AutomationDraft {
    pub fn new_for(project_path: PathBuf, project_name: String) -> Self {
        Self {
            editing_id: None,
            nl_mode: false,
            nl_input: String::new(),
            name: String::new(),
            project_path,
            project_name,
            agent: AgentKind::Claude,
            profile_id: None,
            cadence_kind: CadenceKind::Daily,
            time_text: "02:00".into(),
            weekday: 0,
            prompt: String::new(),
            session_mode: SessionMode::Fresh,
            workspace_mode: WorkspaceMode::Worktree,
            quiet_when_clean: true,
            overlap: OverlapPolicy::Skip,
            timeout_mins: 30,
            missed: MissedPolicy::Skip,
            enabled: false,
        }
    }

    pub fn apply_template(&mut self, tpl: &AutomationTemplate) {
        self.name = tpl.name.into();
        self.agent = tpl.agent;
        self.cadence_kind = tpl.cadence_kind;
        self.time_text = tpl.time_text.into();
        self.session_mode = tpl.session_mode;
        self.workspace_mode = tpl.workspace_mode;
        self.quiet_when_clean = tpl.quiet_when_clean;
        self.prompt = tpl.prompt.into();
        self.nl_mode = false;
    }

    pub fn from_template(tpl: &AutomationTemplate, project_path: PathBuf, project_name: String) -> Self {
        let mut d = Self::new_for(project_path, project_name);
        d.name = tpl.name.into();
        d.agent = tpl.agent;
        d.cadence_kind = tpl.cadence_kind;
        d.time_text = tpl.time_text.into();
        d.session_mode = tpl.session_mode;
        d.workspace_mode = tpl.workspace_mode;
        d.quiet_when_clean = tpl.quiet_when_clean;
        d.prompt = tpl.prompt.into();
        d
    }

    pub fn from_automation(a: &Automation) -> Self {
        let (cadence_kind, time_text, weekday) = match &a.schedule {
            ScheduleKind::Interval { every_minutes } => (CadenceKind::Interval, every_minutes.to_string(), 0),
            ScheduleKind::Daily { hour, minute } => (CadenceKind::Daily, format!("{hour:02}:{minute:02}"), 0),
            ScheduleKind::Weekdays { hour, minute } => (CadenceKind::Weekdays, format!("{hour:02}:{minute:02}"), 0),
            ScheduleKind::Weekly { weekday, hour, minute } => (CadenceKind::Weekly, format!("{hour:02}:{minute:02}"), *weekday),
            ScheduleKind::Cron { expr } => (CadenceKind::Cron, expr.clone(), 0),
            ScheduleKind::Once { at_epoch_secs } => (CadenceKind::Once, format_local_hm(*at_epoch_secs), 0),
        };
        Self {
            editing_id: Some(a.id.clone()),
            nl_mode: false,
            nl_input: String::new(),
            name: a.name.clone(),
            project_path: a.project_path.clone(),
            project_name: a.project_name.clone(),
            agent: a.agent,
            profile_id: a.profile_id.clone(),
            cadence_kind,
            time_text,
            weekday,
            prompt: a.prompt.clone(),
            session_mode: a.session_mode,
            workspace_mode: a.workspace_mode,
            quiet_when_clean: a.quiet_when_clean,
            overlap: a.overlap,
            timeout_mins: a.timeout_mins,
            missed: a.missed,
            enabled: a.enabled,
        }
    }

    pub fn build_schedule(&self, now_epoch: i64) -> Result<ScheduleKind, String> {
        let txt = self.time_text.trim();
        match self.cadence_kind {
            CadenceKind::Interval => {
                let mins = parse_interval_minutes(txt)?;
                Ok(ScheduleKind::Interval { every_minutes: mins })
            }
            CadenceKind::Daily => {
                let (h, m) = parse_hm(txt)?;
                Ok(ScheduleKind::Daily { hour: h, minute: m })
            }
            CadenceKind::Weekdays => {
                let (h, m) = parse_hm(txt)?;
                Ok(ScheduleKind::Weekdays { hour: h, minute: m })
            }
            CadenceKind::Weekly => {
                let (h, m) = parse_hm(txt)?;
                Ok(ScheduleKind::Weekly { weekday: self.weekday % 7, hour: h, minute: m })
            }
            CadenceKind::Cron => {
                CronExpr::parse(txt)?;
                Ok(ScheduleKind::Cron { expr: txt.to_string() })
            }
            CadenceKind::Once => {
                let at = parse_once_epoch(txt, now_epoch)?;
                Ok(ScheduleKind::Once { at_epoch_secs: at })
            }
        }
    }

    /// Populates structured fields from a natural-language description (Antigravity-style prompt builder).
    pub fn apply_natural_language(&mut self) {
        let raw = self.nl_input.trim();
        if raw.is_empty() {
            return;
        }
        let lower = raw.to_ascii_lowercase();
        // Agent hint
        if lower.contains("codex") || lower.contains("chatgpt") {
            self.agent = AgentKind::Codex;
        } else if lower.contains("antigravity") || lower.contains("agy") {
            self.agent = AgentKind::Antigravity;
        } else if lower.contains("opencode") {
            self.agent = AgentKind::OpenCode;
        } else if lower.contains("shell") || lower.contains("bash") || lower.contains("script") {
            self.agent = AgentKind::Custom;
        } else if lower.contains("claude") {
            self.agent = AgentKind::Claude;
        }
        // Cadence & time
        if let Some(mins) = extract_nl_interval(&lower) {
            self.cadence_kind = CadenceKind::Interval;
            self.time_text = mins.to_string();
        } else if lower.contains("weekday") || lower.contains("mon-fri") || lower.contains("hari kerja") {
            self.cadence_kind = CadenceKind::Weekdays;
            self.time_text = extract_nl_time(&lower).unwrap_or_else(|| "09:00".into());
        } else if let Some(wd) = extract_nl_weekday(&lower) {
            self.cadence_kind = CadenceKind::Weekly;
            self.weekday = wd;
            self.time_text = extract_nl_time(&lower).unwrap_or_else(|| "09:00".into());
        } else if lower.contains("once") || lower.contains("sekali") {
            self.cadence_kind = CadenceKind::Once;
            self.time_text = extract_nl_time(&lower).unwrap_or_else(|| "30m".into());
        } else {
            self.cadence_kind = CadenceKind::Daily;
            let default_t = if lower.contains("night") || lower.contains("malam") { "02:00" } else { "09:00" };
            self.time_text = extract_nl_time(&lower).unwrap_or_else(|| default_t.into());
        }
        // Workspace & session isolation
        let edits = ["fix", "edit", "refactor", "update", "worktree", "commit", "upgrade", "changelog", "perbaiki"]
            .iter()
            .any(|k| lower.contains(k));
        self.workspace_mode = if edits { WorkspaceMode::Worktree } else { WorkspaceMode::ProjectDir };
        let stateful = ["continue", "previous run", "compare against", "stateful", "lanjutkan sesi"]
            .iter()
            .any(|k| lower.contains(k));
        self.session_mode = if stateful { SessionMode::Continue } else { SessionMode::Fresh };
        // Name
        if self.name.trim().is_empty() {
            let clean: String = raw
                .split_whitespace()
                .take(5)
                .collect::<Vec<_>>()
                .join(" ");
            self.name = clean.chars().take(42).collect();
        }
        // Self-contained prompt
        self.prompt = if self.agent == AgentKind::Custom {
            raw.to_string()
        } else {
            format!("{raw}\n\nKeep changes minimal, verify before finishing, and report quietly if everything is already clean.")
        };
        self.nl_mode = false;
    }
}

fn parse_hm(s: &str) -> Result<(u8, u8), String> {
    let (h, m) = s.split_once(':').ok_or_else(|| "Use HH:MM (e.g. 09:00)".to_string())?;
    let h: u8 = h.trim().parse().map_err(|_| "Invalid hour".to_string())?;
    let m: u8 = m.trim().parse().map_err(|_| "Invalid minute".to_string())?;
    if h > 23 || m > 59 {
        return Err("Time must be 00:00–23:59".into());
    }
    Ok((h, m))
}

fn parse_interval_minutes(s: &str) -> Result<u32, String> {
    let t = s.trim().to_ascii_lowercase();
    if let Some(h) = t.strip_suffix('h') {
        let v: u32 = h.trim().parse().map_err(|_| "Use minutes or hours (e.g. 60 or 4h)".to_string())?;
        return (v > 0).then_some(v.saturating_mul(60)).ok_or_else(|| "Must be > 0".into());
    }
    let digits = t.strip_suffix('m').unwrap_or(&t).trim();
    let v: u32 = digits.parse().map_err(|_| "Use minutes or hours (e.g. 30 or 4h)".to_string())?;
    if v == 0 {
        return Err("Interval must be at least 1 minute".into());
    }
    Ok(v)
}

fn parse_once_epoch(s: &str, now_epoch: i64) -> Result<i64, String> {
    let t = s.trim().to_ascii_lowercase();
    if t.ends_with('m') || t.ends_with('h') {
        let mins = parse_interval_minutes(&t)?;
        return Ok(now_epoch + i64::from(mins) * 60);
    }
    let (h, m) = parse_hm(&t)?;
    CronExpr::parse(&format!("{m} {h} * * *"))?.next_after(now_epoch).ok_or_else(|| "Could not resolve time".into())
}

fn extract_nl_interval(lower: &str) -> Option<u32> {
    let words: Vec<&str> = lower.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        if (*w == "every" || *w == "tiap" || *w == "setiap")
            && let Some(next) = words.get(i + 1)
        {
            if *next == "hour" || *next == "jam" {
                return Some(60);
            }
            if let Some(v) = next.strip_suffix('h').and_then(|s| s.parse::<u32>().ok()).filter(|v| *v > 0) {
                return Some(v * 60);
            }
            if let Some(v) = next.strip_suffix('m').and_then(|s| s.parse::<u32>().ok()).filter(|v| *v > 0) {
                return Some(v);
            }
            if let Ok(v) = next.parse::<u32>()
                && v > 0
                && let Some(unit) = words.get(i + 2)
            {
                if unit.starts_with("hour") || unit.starts_with("jam") {
                    return Some(v * 60);
                }
                if unit.starts_with("min") || unit.starts_with("menit") {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn extract_nl_weekday(lower: &str) -> Option<u8> {
    for (i, keys) in [
        (0u8, &["monday", "senin"][..]),
        (1, &["tuesday", "selasa"]),
        (2, &["wednesday", "rabu"]),
        (3, &["thursday", "kamis"]),
        (4, &["friday", "jumat"]),
        (5, &["saturday", "sabtu"]),
        (6, &["sunday", "minggu"]),
    ] {
        if keys.iter().any(|k| lower.contains(k)) {
            return Some(i);
        }
    }
    lower.contains("weekly").then_some(0)
}

fn extract_nl_time(lower: &str) -> Option<String> {
    for tok in lower.split_whitespace() {
        let clean = tok.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != ':');
        if let Ok((h, m)) = parse_hm(clean) {
            return Some(format!("{h:02}:{m:02}"));
        }
        if let Some(h) = clean.strip_suffix("am").and_then(|s| s.parse::<u8>().ok()).filter(|h| *h >= 1 && *h <= 12) {
            let h24 = if h == 12 { 0 } else { h };
            return Some(format!("{h24:02}:00"));
        }
        if let Some(h) = clean.strip_suffix("pm").and_then(|s| s.parse::<u8>().ok()).filter(|h| *h >= 1 && *h <= 12) {
            let h24 = if h == 12 { 12 } else { h + 12 };
            return Some(format!("{h24:02}:00"));
        }
    }
    None
}

pub fn automation_agent_label(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Claude => "Claude Code",
        AgentKind::Codex => "Codex",
        AgentKind::Antigravity => "Antigravity CLI",
        AgentKind::OpenCode => "OpenCode",
        AgentKind::Custom => "Shell command",
        AgentKind::Gemini => "Gemini CLI",
    }
}

pub fn now_epoch_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

pub fn local_tz_label() -> String {
    Local::now().format("UTC%:z").to_string()
}

pub fn format_local_hm(epoch: i64) -> String {
    Local
        .timestamp_opt(epoch, 0)
        .single()
        .map_or_else(|| "09:00".into(), |dt| dt.format("%H:%M").to_string())
}

pub fn format_local_short(epoch: i64) -> String {
    let Some(dt) = Local.timestamp_opt(epoch, 0).single() else {
        return String::new();
    };
    let today = Local::now().date_naive();
    let d = dt.date_naive();
    if d == today {
        format!("Today {}", dt.format("%H:%M"))
    } else if Some(d) == today.pred_opt() {
        format!("Yesterday {}", dt.format("%H:%M"))
    } else if Some(d) == today.succ_opt() {
        format!("Tomorrow {}", dt.format("%H:%M"))
    } else {
        dt.format("%b %d %H:%M").to_string()
    }
}

pub fn format_countdown(next_epoch: i64, now_epoch: i64) -> String {
    let diff = next_epoch.saturating_sub(now_epoch);
    if diff < 60 {
        "in <1m".into()
    } else if diff < 3600 {
        format!("in {}m", diff / 60)
    } else if diff < 86400 {
        format!("in {}h {}m", diff / 3600, (diff % 3600) / 60)
    } else {
        format!("in {}d", diff / 86400)
    }
}

pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Strips ANSI CSI/OSC escape sequences so streamed CLI output reads cleanly in the run log.
pub fn strip_ansi(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                i = (i + 1).min(bytes.len());
                continue;
            }
            if i + 1 < bytes.len() && bytes[i + 1] == b']' {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            i += 1;
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        if ch != '\r' {
            out.push(ch);
        }
        i += ch.len_utf8();
    }
    out
}

/// Handle for an actively executing background automation run.
pub struct ActiveAutomationRun {
    pub run_id: String,
    pub started: Instant,
    pub timeout: Duration,
    pub cancel: Arc<AtomicBool>,
    pub child_pid: Arc<Mutex<Option<u32>>>,
    pub queued: Option<RunTrigger>,
}

impl ActiveAutomationRun {
    pub fn cancel_now(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(pid) = *self.child_pid.lock().unwrap_or_else(|p| p.into_inner()) {
            kill_pid(pid);
        }
    }
}

/// Messages sent from background automation runner threads to the UI thread.
#[derive(Debug)]
pub enum AutomationMsg {
    Log {
        auto_id: String,
        run_id: String,
        line: String,
    },
    WorktreeReady {
        auto_id: String,
        run_id: String,
        path: PathBuf,
        branch: String,
        base_commit: String,
    },
    AccountSwitched {
        auto_id: String,
        run_id: String,
        account_used: String,
        limited_profile: String,
        retry_after: Option<Duration>,
    },
    Finished {
        auto_id: String,
        run_id: String,
        outcome: RunOutcome,
    },
}

#[derive(Debug)]
pub struct RunOutcome {
    pub status: RunStatus,
    pub duration_secs: u64,
    pub account_used: String,
    pub session_id: Option<String>,
    pub workspace_path: PathBuf,
    pub worktree_branch: Option<String>,
    pub worktree_base_commit: Option<String>,
    pub files_changed: usize,
    pub additions: u32,
    pub deletions: u32,
    pub summary_title: String,
    pub summary_body: String,
    pub notified_inbox: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutomationFilter {
    #[default]
    All,
    Active,
    NeedsReview,
    Paused,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutomationDetailTab {
    #[default]
    Runs,
    Config,
    Templates,
}

#[derive(Clone)]
struct RunSpec {
    auto_id: String,
    run_id: String,
    run_number: u32,
    name: String,
    project_path: PathBuf,
    agent: AgentKind,
    prompt: String,
    session_mode: SessionMode,
    last_session_id: Option<String>,
    workspace_mode: WorkspaceMode,
    worktree_setup: WorktreeSetup,
    quiet_when_clean: bool,
    auto_failover: bool,
    candidates: Vec<Option<AgentProfile>>,
    secrets: Vec<String>,
    path_env: Option<std::ffi::OsString>,
    home: PathBuf,
    data_dir: PathBuf,
    worktree_prefix: String,
    cancel: Arc<AtomicBool>,
    child_pid: Arc<Mutex<Option<u32>>>,
}

fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY: sending signals to the spawned child process group / pid.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
            libc::kill(pid as i32, libc::SIGTERM);
        }
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
                libc::kill(pid as i32, libc::SIGKILL);
            }
        });
    }
    #[cfg(windows)]
    {
        let _ = crate::platform::command("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn copy_worktree_path(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_worktree_path(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else if meta.file_type().is_symlink() {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(std::fs::read_link(from)?, to)
        }
        #[cfg(not(unix))]
        {
            std::fs::copy(from, to).map(|_| ())
        }
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

fn build_command(agent: AgentKind, prompt: &str, session_mode: SessionMode, last_session: Option<&str>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = match agent {
        AgentKind::Custom => {
            #[cfg(windows)]
            {
                let mut c = crate::platform::command("cmd");
                c.args(["/C", prompt]);
                c
            }
            #[cfg(not(windows))]
            {
                let mut c = crate::platform::command("/bin/sh");
                c.args(["-c", prompt]);
                c
            }
        }
        AgentKind::Claude => {
            let mut c = crate::platform::command("claude");
            if session_mode == SessionMode::Continue {
                if let Some(sid) = last_session {
                    c.args(["--resume", sid]);
                } else {
                    c.arg("--continue");
                }
            }
            c.args(["-p", prompt, "--output-format", "text"]);
            c
        }
        AgentKind::Codex => {
            let mut c = crate::platform::command("codex");
            c.arg("exec");
            if session_mode == SessionMode::Continue {
                c.arg("resume");
                if let Some(sid) = last_session {
                    c.arg(sid);
                } else {
                    c.arg("--last");
                }
            }
            c.arg(prompt);
            c
        }
        AgentKind::Antigravity => {
            let mut c = crate::platform::command("agy");
            if session_mode == SessionMode::Continue {
                if let Some(sid) = last_session {
                    c.args(["--resume", sid]);
                } else {
                    c.arg("--continue");
                }
            }
            c.args(["-p", prompt, "--output-format", "text", "--mode", "auto"]);
            c
        }
        AgentKind::OpenCode => {
            let mut c = crate::platform::command("opencode");
            c.args(["run", prompt]);
            c
        }
        AgentKind::Gemini => {
            let mut c = crate::platform::command("gemini");
            c.args(["-p", prompt]);
            c
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // SAFETY: `setpgid(0, 0)` is async-signal-safe; puts child in its own process group so
        // timeout / Stop kills the entire subtree cleanly.
        unsafe {
            cmd.pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }
    }
    cmd
}

fn execute_run(spec: RunSpec, tx: std::sync::mpsc::Sender<AppMsg>, ctx: egui::Context) {
    let started = Instant::now();
    let emit_log = |line: String| {
        let clean = profiles::redact_secrets(&strip_ansi(&line), &spec.secrets);
        let _ = tx.send(AppMsg::Automation(AutomationMsg::Log {
            auto_id: spec.auto_id.clone(),
            run_id: spec.run_id.clone(),
            line: clean,
        }));
        ctx.request_repaint();
    };

    if !spec.project_path.is_dir() {
        let _ = tx.send(AppMsg::Automation(AutomationMsg::Finished {
            auto_id: spec.auto_id,
            run_id: spec.run_id,
            outcome: RunOutcome {
                status: RunStatus::Failed,
                duration_secs: started.elapsed().as_secs(),
                account_used: "none".into(),
                session_id: spec.last_session_id,
                workspace_path: spec.project_path.clone(),
                worktree_branch: None,
                worktree_base_commit: None,
                files_changed: 0,
                additions: 0,
                deletions: 0,
                summary_title: "Project folder not found".into(),
                summary_body: format!("Directory `{}` does not exist.", paths::contract_tilde(&spec.project_path)),
                notified_inbox: true,
            },
        }));
        ctx.request_repaint();
        return;
    }

    // 1. Provision isolated Git worktree if requested and repo is a git repo.
    let mut work_dir = spec.project_path.clone();
    let mut wt_repo_root: Option<PathBuf> = None;
    let mut wt_branch: Option<String> = None;
    let mut wt_base_commit: Option<String> = None;

    if spec.workspace_mode == WorkspaceMode::Worktree
        && let Some(repo) = tc_git::repo_root(&spec.project_path)
    {
        let slug = profiles::sanitize_id(&spec.name);
        let proj_slug = spec
            .project_path
            .file_name()
            .map(|n| profiles::sanitize_id(&n.to_string_lossy()))
            .unwrap_or_else(|| "project".into());
        let suffix = format!("{slug}-r{}", spec.run_number);
        let branch = format!("{}auto-{suffix}", spec.worktree_prefix.trim());
        let wt_dir = paths::automation_worktrees_dir().join(&proj_slug).join(&suffix);
        let base = tc_git::worktree::current_branch(&repo).unwrap_or_else(|| "HEAD".into());
        if let Some(parent) = wt_dir.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        emit_log(format!("Provisioning isolated worktree ({branch})…"));
        match tc_git::worktree::add(&repo, &wt_dir, &branch, &base) {
            Ok(commit) => {
                for rel in &spec.worktree_setup.copy {
                    let from = repo.join(rel);
                    if from.exists() {
                        let _ = copy_worktree_path(&from, &wt_dir.join(rel));
                    }
                }
                if !spec.worktree_setup.run.trim().is_empty() {
                    emit_log(format!("Running worktree setup: {}", spec.worktree_setup.run.trim()));
                    let mut sc = build_command(AgentKind::Custom, spec.worktree_setup.run.trim(), SessionMode::Fresh, None);
                    sc.current_dir(&wt_dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
                    if let Some(p) = &spec.path_env {
                        sc.env("PATH", p);
                    }
                    let _ = sc.status();
                }
                work_dir = wt_dir.clone();
                wt_repo_root = Some(repo);
                wt_branch = Some(branch.clone());
                wt_base_commit = Some(commit.clone());
                let _ = tx.send(AppMsg::Automation(AutomationMsg::WorktreeReady {
                    auto_id: spec.auto_id.clone(),
                    run_id: spec.run_id.clone(),
                    path: wt_dir,
                    branch,
                    base_commit: commit,
                }));
                ctx.request_repaint();
            }
            Err(e) => {
                emit_log(format!("Worktree setup failed ({e}); falling back to project folder."));
            }
        }
    }

    // Take baseline git snapshot so we can detect exact changed files after the run.
    let base_snap = tc_git::snapshot(&work_dir).ok();

    // 2. Execute command across candidate accounts (supports automatic rate-limit failover).
    let mut collected_lines: Vec<String> = Vec::new();
    let mut accounts_chain: Vec<String> = Vec::new();
    let mut exit_ok = false;
    let mut last_err: Option<String> = None;

    let candidates = if spec.candidates.is_empty() { vec![None] } else { spec.candidates.clone() };
    for (attempt_idx, cand) in candidates.iter().enumerate() {
        if spec.cancel.load(Ordering::SeqCst) {
            break;
        }
        let acc_name = match cand {
            Some(p) => p.id.clone(),
            None if spec.agent == AgentKind::Custom => "shell".into(),
            None => "default".into(),
        };
        if !accounts_chain.contains(&acc_name) {
            accounts_chain.push(acc_name.clone());
        }
        emit_log(format!(
            "Starting {} ({acc_name}, {} session) in {}",
            automation_agent_label(spec.agent),
            if spec.session_mode == SessionMode::Fresh { "fresh" } else { "continue" },
            paths::contract_tilde(&work_dir)
        ));

        let mut cmd = build_command(spec.agent, &spec.prompt, spec.session_mode, spec.last_session_id.as_deref());
        cmd.current_dir(&work_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("TERNARYCODE_AUTOMATION", &spec.auto_id)
            .env("TERNARYCODE_AUTOMATION_RUN", &spec.run_id);
        if let Some(p) = &spec.path_env {
            cmd.env("PATH", p);
        }
        if let Some(prof) = cand {
            for k in profiles::profile_unset_env(prof) {
                cmd.env_remove(k);
            }
            for (k, v) in profiles::profile_env(prof, &spec.home, &spec.data_dir) {
                cmd.env(k, v);
            }
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("Failed to start {}: {e}", automation_agent_label(spec.agent));
                emit_log(msg.clone());
                last_err = Some(msg);
                break;
            }
        };
        *spec.child_pid.lock().unwrap_or_else(|p| p.into_inner()) = Some(child.id());

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let attempt_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let spawn_reader = |stream: Box<dyn std::io::Read + Send>,
                            lines_acc: Arc<Mutex<Vec<String>>>,
                            tx_r: std::sync::mpsc::Sender<AppMsg>,
                            ctx_r: egui::Context,
                            auto_id: String,
                            run_id: String,
                            secrets: Vec<String>| {
            std::thread::spawn(move || {
                let reader = BufReader::new(stream);
                for line in reader.lines().map_while(Result::ok) {
                    let clean = profiles::redact_secrets(&strip_ansi(&line), &secrets);
                    {
                        let mut acc = lines_acc.lock().unwrap_or_else(|p| p.into_inner());
                        if acc.len() >= MAX_LOG_LINES_PER_RUN {
                            acc.remove(0);
                        }
                        acc.push(clean.clone());
                    }
                    let _ = tx_r.send(AppMsg::Automation(AutomationMsg::Log {
                        auto_id: auto_id.clone(),
                        run_id: run_id.clone(),
                        line: clean,
                    }));
                    ctx_r.request_repaint();
                }
            })
        };

        let h_out = stdout.map(|s| {
            spawn_reader(
                Box::new(s),
                attempt_lines.clone(),
                tx.clone(),
                ctx.clone(),
                spec.auto_id.clone(),
                spec.run_id.clone(),
                spec.secrets.clone(),
            )
        });
        let h_err = stderr.map(|s| {
            spawn_reader(
                Box::new(s),
                attempt_lines.clone(),
                tx.clone(),
                ctx.clone(),
                spec.auto_id.clone(),
                spec.run_id.clone(),
                spec.secrets.clone(),
            )
        });

        let status = child.wait();
        *spec.child_pid.lock().unwrap_or_else(|p| p.into_inner()) = None;
        if let Some(h) = h_out {
            let _ = h.join();
        }
        if let Some(h) = h_err {
            let _ = h.join();
        }
        let lines_now = attempt_lines.lock().unwrap_or_else(|p| p.into_inner()).clone();
        collected_lines.extend(lines_now.iter().cloned());

        if spec.cancel.load(Ordering::SeqCst) {
            break;
        }
        match status {
            Ok(st) if st.success() => {
                exit_ok = true;
                break;
            }
            Ok(st) => {
                let tail_text = lines_now.iter().rev().take(24).rev().cloned().collect::<Vec<_>>().join("\n");
                if spec.auto_failover
                    && attempt_idx + 1 < candidates.len()
                    && let Some(hit) = profiles::detect_rate_limit(&tail_text)
                {
                    let next_name = candidates[attempt_idx + 1]
                        .as_ref()
                        .map_or_else(|| "next".into(), |p| p.id.clone());
                    emit_log(format!("Rate limit on {acc_name} ({}) → failing over to {next_name}…", hit.reason));
                    let _ = tx.send(AppMsg::Automation(AutomationMsg::AccountSwitched {
                        auto_id: spec.auto_id.clone(),
                        run_id: spec.run_id.clone(),
                        account_used: format!("{acc_name} → {next_name}"),
                        limited_profile: acc_name,
                        retry_after: hit.retry_after,
                    }));
                    ctx.request_repaint();
                    continue;
                }
                let code = st.code().map_or_else(|| "signal".into(), |c| format!("exit {c}"));
                last_err = Some(format!("Exited with {code}"));
                break;
            }
            Err(e) => {
                last_err = Some(format!("Wait failed: {e}"));
                break;
            }
        }
    }

    // 3. Inspect git diff & auto-clean untouched worktrees.
    let mut files_changed = 0usize;
    let mut additions = 0u32;
    let mut deletions = 0u32;
    let mut has_new_commits = false;

    if let Some(before) = &base_snap
        && let Ok(after) = tc_git::snapshot(&work_dir)
    {
        if let Ok(changes) = tc_git::changes(&work_dir, &before.tree, &after.tree) {
            files_changed = changes.len();
            additions = changes.iter().filter_map(|c| c.additions).sum();
            deletions = changes.iter().filter_map(|c| c.deletions).sum();
        }
        if let (Some(base_c), Some(now_c)) = (&wt_base_commit, tc_git::worktree::head(&work_dir))
            && now_c != *base_c
        {
            has_new_commits = true;
            if files_changed == 0
                && let Ok((fc, a, d)) = tc_git::worktree::diff_stat(&work_dir, base_c, &now_c)
            {
                files_changed = fc;
                additions = a;
                deletions = d;
            }
        }
    }

    if let (Some(repo), Some(branch)) = (&wt_repo_root, &wt_branch)
        && files_changed == 0
        && !has_new_commits
    {
        let _ = tc_git::worktree::remove(repo, &work_dir, true);
        let _ = tc_git::worktree::delete_branch(repo, branch, true);
        emit_log("No file changes — auto-cleaned empty worktree.".into());
        wt_branch = None;
        wt_base_commit = None;
        work_dir = spec.project_path.clone();
    }

    // 4. Derive status, summary, and Inbox routing.
    let cancelled = spec.cancel.load(Ordering::SeqCst);
    let non_empty: Vec<&str> = collected_lines
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let tail_snippet = non_empty.iter().rev().take(2).rev().copied().collect::<Vec<_>>().join(" · ");

    let (status, notified_inbox, summary_title, summary_body) = if cancelled {
        (
            RunStatus::Cancelled,
            false,
            "Run stopped by user".into(),
            if tail_snippet.is_empty() { "Execution cancelled before completion.".into() } else { tail_snippet },
        )
    } else if !exit_ok {
        let title = last_err.unwrap_or_else(|| "Run failed".into());
        (
            RunStatus::Failed,
            true,
            title,
            if tail_snippet.is_empty() { "Check the terminal log below for error details.".into() } else { tail_snippet },
        )
    } else if files_changed > 0 || has_new_commits {
        let title = if files_changed > 0 {
            format!(
                "Completed with {} changed {} (+{additions} -{deletions})",
                files_changed,
                if files_changed == 1 { "file" } else { "files" }
            )
        } else {
            "Completed with new commits in worktree".into()
        };
        let body = if tail_snippet.is_empty() {
            "Isolated changes are ready for Review diff or Open in Focus.".into()
        } else {
            tail_snippet
        };
        (RunStatus::NeedsReview, true, title, body)
    } else if !spec.quiet_when_clean {
        (
            RunStatus::NeedsReview,
            true,
            "Completed — report ready".into(),
            if tail_snippet.is_empty() { "Finished cleanly with no file changes.".into() } else { tail_snippet },
        )
    } else {
        (
            RunStatus::Clean,
            false,
            "Clean run — no action needed (quiet)".into(),
            if tail_snippet.is_empty() { "Finished cleanly; Inbox stayed quiet.".into() } else { tail_snippet },
        )
    };

    let session_id = spec
        .last_session_id
        .filter(|_| spec.session_mode == SessionMode::Continue)
        .or_else(|| Some(format!("auto-{}-r{}", profiles::sanitize_id(&spec.auto_id), spec.run_number)));

    let _ = tx.send(AppMsg::Automation(AutomationMsg::Finished {
        auto_id: spec.auto_id,
        run_id: spec.run_id,
        outcome: RunOutcome {
            status,
            duration_secs: started.elapsed().as_secs(),
            account_used: accounts_chain.join(" → "),
            session_id,
            workspace_path: work_dir,
            worktree_branch: wt_branch,
            worktree_base_commit: wt_base_commit,
            files_changed,
            additions,
            deletions,
            summary_title,
            summary_body,
            notified_inbox,
        },
    }));
    ctx.request_repaint();
}

impl App {
    /// Loads saved automations from `<data_dir>/automations.json`, marking any run left
    /// `Running` from a previous crash/quit as `Cancelled` and recomputing next schedule slots.
    pub fn load_automations() -> Vec<Automation> {
        let save: AutomationsSave = tc_core::persist::load_json(&paths::automations_file());
        let now = now_epoch_secs();
        let mut items = save.items;
        for a in &mut items {
            for r in &mut a.runs {
                if r.status == RunStatus::Running {
                    r.status = RunStatus::Cancelled;
                    r.finished_at = Some(now);
                    r.summary_title = "Interrupted when TernaryCode closed".into();
                }
            }
            if a.enabled && a.next_run_at.is_none() {
                a.recompute_next(now);
            }
        }
        items
    }

    pub fn mark_automations_dirty(&mut self) {
        self.automations_dirty = true;
        self.mark_dirty();
    }

    pub fn save_automations_if_dirty(&mut self) {
        if !self.automations_dirty {
            return;
        }
        self.automations_dirty = false;
        let save = AutomationsSave { version: 1, items: self.automations.clone() };
        if let Ok(data) = serde_json::to_vec_pretty(&save) {
            self.saver.queue(&self.rt, paths::automations_file(), data);
        }
    }

    /// Number of automations currently executing in the background.
    pub fn running_automations_count(&self) -> usize {
        self.active_automation_runs.len()
    }

    /// Number of unreviewed Automation alerts (findings, diffs, failures) waiting in Inbox.
    pub fn unreviewed_automation_alerts_count(&self) -> usize {
        self.automations
            .iter()
            .flat_map(|a| a.runs.iter())
            .filter(|r| r.unreviewed_alert())
            .count()
    }

    /// Cancels all running automation child processes (called on app quit).
    pub fn stop_all_automations(&mut self) {
        for active in self.active_automation_runs.values() {
            active.cancel_now();
        }
        self.active_automation_runs.clear();
    }

    /// Periodic scheduler tick called from `frame_logic`: checks timeouts and scheduled triggers.
    pub fn automations_tick(&mut self, ctx: &egui::Context) {
        let now_inst = Instant::now();
        // 1. Check active run timeouts.
        let timed_out: Vec<String> = self
            .active_automation_runs
            .iter()
            .filter(|(_, r)| now_inst.duration_since(r.started) >= r.timeout)
            .map(|(id, _)| id.clone())
            .collect();
        for id in timed_out {
            if let Some(active) = self.active_automation_runs.get(&id) {
                active.cancel_now();
            }
            if let Some(a) = self.automations.iter_mut().find(|x| x.id == id)
                && let Some(r) = a.runs.first_mut()
                && r.status == RunStatus::Running
            {
                r.log_lines.push(format!("Timed out after {}m — terminating run.", a.timeout_mins));
            }
        }
        if !self.active_automation_runs.is_empty() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }

        // 2. Check scheduled triggers once per second.
        if self.automations_checked_at.is_some_and(|t| now_inst.duration_since(t) < Duration::from_secs(1)) {
            return;
        }
        self.automations_checked_at = Some(now_inst);
        let now_epoch = now_epoch_secs();

        let mut to_fire: Vec<(String, RunTrigger)> = Vec::new();
        let mut next_wake_secs: Option<i64> = None;
        let mut dirty = false;

        for a in &mut self.automations {
            if !a.enabled {
                continue;
            }
            let due = match a.next_run_at {
                Some(t) => t,
                None => {
                    a.recompute_next(now_epoch);
                    dirty = true;
                    continue;
                }
            };
            if due <= now_epoch {
                let late_secs = now_epoch - due;
                let trigger = if late_secs > 120 {
                    match a.missed {
                        MissedPolicy::Skip => {
                            a.recompute_next(now_epoch);
                            dirty = true;
                            continue;
                        }
                        MissedPolicy::RunOnWake => RunTrigger::OnWake,
                    }
                } else {
                    RunTrigger::Scheduled
                };
                if matches!(a.schedule, ScheduleKind::Once { .. }) {
                    a.enabled = false;
                    a.next_run_at = None;
                } else {
                    a.recompute_next(now_epoch);
                }
                a.last_triggered_at = Some(now_epoch);
                dirty = true;
                to_fire.push((a.id.clone(), trigger));
            } else {
                let left = due - now_epoch;
                next_wake_secs = Some(next_wake_secs.map_or(left, |w| w.min(left)));
            }
        }

        if dirty {
            self.mark_automations_dirty();
        }
        for (id, trigger) in to_fire {
            self.start_automation_run(&id, trigger);
        }
        if let Some(secs) = next_wake_secs {
            ctx.request_repaint_after(Duration::from_secs((secs.clamp(1, 30)) as u64));
        }
    }

    /// Starts a run of automation `auto_id`, honoring its `OverlapPolicy`.
    pub fn start_automation_run(&mut self, auto_id: &str, trigger: RunTrigger) {
        let Some(idx) = self.automations.iter().position(|a| a.id == auto_id) else {
            return;
        };
        let overlap = self.automations[idx].overlap;
        if let Some(active) = self.active_automation_runs.get_mut(auto_id) {
            match overlap {
                OverlapPolicy::Skip => {
                    if trigger == RunTrigger::Manual {
                        self.toast(t("This automation is already running.").into());
                    }
                    return;
                }
                OverlapPolicy::Queue => {
                    active.queued = Some(trigger);
                    if trigger == RunTrigger::Manual {
                        self.toast(t("Queued next run after the active one finishes.").into());
                    }
                    return;
                }
                OverlapPolicy::CancelPrevious => {
                    active.cancel_now();
                    self.active_automation_runs.remove(auto_id);
                }
            }
        }

        let now_epoch = now_epoch_secs();
        let run_number = self.automations[idx].next_run_number();
        let run_id = format!("run-{run_number}-{now_epoch}");
        let auto = self.automations[idx].clone();

        // Build candidate profiles (pinned or auto-failover sorted by fallback order, healthy first).
        let now_inst = Instant::now();
        let all_profiles = self.effective_profiles();
        let mut candidates: Vec<Option<AgentProfile>> = Vec::new();
        let mut auto_failover = false;
        if profiles::supports_profile(auto.agent) {
            if let Some(pid) = &auto.profile_id {
                if let Some(p) = all_profiles.iter().find(|p| p.id == *pid && p.enabled) {
                    candidates.push(Some(p.clone()));
                }
            } else {
                auto_failover = true;
                let mut healthy: Vec<AgentProfile> = all_profiles
                    .iter()
                    .filter(|p| p.enabled && p.agent == auto.agent && self.profiles_rt.get(&p.id).is_none_or(|rt| rt.is_healthy(now_inst)))
                    .cloned()
                    .collect();
                healthy.sort_by_key(|p| p.fallback_order);
                let mut cross: Vec<AgentProfile> = if self.settings.failover.cross_provider {
                    all_profiles
                        .iter()
                        .filter(|p| p.enabled && p.agent != auto.agent && self.profiles_rt.get(&p.id).is_none_or(|rt| rt.is_healthy(now_inst)))
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                };
                cross.sort_by_key(|p| p.fallback_order);
                for p in healthy.into_iter().chain(cross) {
                    candidates.push(Some(p));
                }
            }
        }
        if candidates.is_empty() {
            candidates.push(None);
        }

        let account_used = candidates[0]
            .as_ref()
            .map_or_else(|| if auto.agent == AgentKind::Custom { "shell".into() } else { "default".into() }, |p| p.id.clone());

        let worktree_setup = self
            .projects
            .iter()
            .find(|p| p.path == auto.project_path)
            .map(|p| self.worktree_setup_of(p.id))
            .unwrap_or_default();

        let initial_run = AutomationRun {
            id: run_id.clone(),
            run_number,
            trigger,
            status: RunStatus::Running,
            started_at: now_epoch,
            finished_at: None,
            duration_secs: 0,
            account_used,
            session_id: auto.last_session_id.clone().filter(|_| auto.session_mode == SessionMode::Continue),
            workspace_path: auto.project_path.clone(),
            worktree_branch: None,
            worktree_base_commit: None,
            files_changed: 0,
            additions: 0,
            deletions: 0,
            summary_title: format!("Running #{} in background…", run_number),
            summary_body: if auto.session_mode == SessionMode::Fresh {
                "Fresh isolated session streaming live output below.".into()
            } else {
                "Continuing previous session context.".into()
            },
            log_lines: Vec::new(),
            reviewed: false,
            notified_inbox: false,
        };

        let slot = &mut self.automations[idx];
        slot.runs.insert(0, initial_run);
        if slot.runs.len() > MAX_RUNS_PER_AUTOMATION {
            slot.runs.truncate(MAX_RUNS_PER_AUTOMATION);
        }
        slot.last_triggered_at = Some(now_epoch);
        self.automation_sel = Some(auto.id.clone());
        self.automation_run_ix = 0;
        self.mark_automations_dirty();

        let cancel = Arc::new(AtomicBool::new(false));
        let child_pid = Arc::new(Mutex::new(None));
        let timeout = {
            #[cfg(debug_assertions)]
            let override_ms = std::env::var("TERNARYCODE_AUTO_TIMEOUT_MS").ok().and_then(|v| v.parse::<u64>().ok());
            #[cfg(not(debug_assertions))]
            let override_ms: Option<u64> = None;
            override_ms.map(Duration::from_millis).unwrap_or_else(|| Duration::from_secs(u64::from(auto.timeout_mins.max(1)) * 60))
        };

        self.active_automation_runs.insert(
            auto.id.clone(),
            ActiveAutomationRun {
                run_id: run_id.clone(),
                started: Instant::now(),
                timeout,
                cancel: cancel.clone(),
                child_pid: child_pid.clone(),
                queued: None,
            },
        );

        let mut secrets = Vec::new();
        for p in &all_profiles {
            if let Some(k) = &p.api_key
                && k.len() >= 8
            {
                secrets.push(k.clone());
            }
        }

        let spec = RunSpec {
            auto_id: auto.id.clone(),
            run_id,
            run_number,
            name: auto.name.clone(),
            project_path: auto.project_path,
            agent: auto.agent,
            prompt: auto.prompt,
            session_mode: auto.session_mode,
            last_session_id: auto.last_session_id,
            workspace_mode: auto.workspace_mode,
            worktree_setup,
            quiet_when_clean: auto.quiet_when_clean,
            auto_failover,
            candidates,
            secrets,
            path_env: self.lsp.path_env.clone(),
            home: paths::home_dir(),
            data_dir: paths::data_dir().to_path_buf(),
            worktree_prefix: self.settings.worktrees.prefix.clone(),
            cancel,
            child_pid,
        };

        let tx = self.app_tx.clone();
        let ctx = self.ctx.clone();
        self.rt.spawn_blocking(move || execute_run(spec, tx, ctx));
        self.push_agent_log(
            LogLevel::Info,
            None,
            None,
            None,
            format!("Started automation \"{}\" (#{run_number})", auto.name),
        );
    }

    pub fn stop_automation_run(&mut self, auto_id: &str) {
        if let Some(active) = self.active_automation_runs.get(auto_id) {
            active.cancel_now();
            self.toast(t("Stopping automation run…").into());
        }
    }

    pub fn on_automation_msg(&mut self, msg: AutomationMsg) {
        match msg {
            AutomationMsg::Log { auto_id, run_id, line } => {
                if let Some(a) = self.automations.iter_mut().find(|x| x.id == auto_id)
                    && let Some(r) = a.runs.iter_mut().find(|r| r.id == run_id)
                {
                    if r.log_lines.len() >= MAX_LOG_LINES_PER_RUN {
                        r.log_lines.remove(0);
                    }
                    r.log_lines.push(line);
                }
            }
            AutomationMsg::WorktreeReady { auto_id, run_id, path, branch, base_commit } => {
                if let Some(a) = self.automations.iter_mut().find(|x| x.id == auto_id)
                    && let Some(r) = a.runs.iter_mut().find(|r| r.id == run_id)
                {
                    r.workspace_path = path;
                    r.worktree_branch = Some(branch);
                    r.worktree_base_commit = Some(base_commit);
                }
            }
            AutomationMsg::AccountSwitched { auto_id, run_id, account_used, limited_profile, retry_after } => {
                let now = Instant::now();
                let cooldown = retry_after.unwrap_or_else(|| Duration::from_secs(u64::from(self.settings.failover.default_cooldown_mins) * 60));
                let rt = self.profiles_rt.entry(limited_profile.clone()).or_default();
                rt.state = profiles::ProfileState::RateLimited;
                rt.reset_at = Some(now + cooldown);
                if let Some(a) = self.automations.iter_mut().find(|x| x.id == auto_id)
                    && let Some(r) = a.runs.iter_mut().find(|r| r.id == run_id)
                {
                    r.account_used = account_used;
                }
            }
            AutomationMsg::Finished { auto_id, run_id, outcome } => {
                let queued = self
                    .active_automation_runs
                    .get(&auto_id)
                    .filter(|act| act.run_id == run_id)
                    .and_then(|act| act.queued);
                if self.active_automation_runs.get(&auto_id).is_some_and(|act| act.run_id == run_id) {
                    self.active_automation_runs.remove(&auto_id);
                }

                let mut notify_info: Option<(String, String, String)> = None;
                if let Some(a) = self.automations.iter_mut().find(|x| x.id == auto_id) {
                    if let Some(sid) = &outcome.session_id {
                        a.last_session_id = Some(sid.clone());
                    }
                    if let Some(r) = a.runs.iter_mut().find(|r| r.id == run_id) {
                        r.status = outcome.status;
                        r.finished_at = Some(now_epoch_secs());
                        r.duration_secs = outcome.duration_secs;
                        r.account_used = outcome.account_used;
                        r.session_id = outcome.session_id;
                        r.workspace_path = outcome.workspace_path;
                        r.worktree_branch = outcome.worktree_branch;
                        r.worktree_base_commit = outcome.worktree_base_commit;
                        r.files_changed = outcome.files_changed;
                        r.additions = outcome.additions;
                        r.deletions = outcome.deletions;
                        r.summary_title = outcome.summary_title.clone();
                        r.summary_body = outcome.summary_body.clone();
                        r.notified_inbox = outcome.notified_inbox;
                        if outcome.notified_inbox {
                            notify_info = Some((a.name.clone(), a.project_name.clone(), outcome.summary_title));
                        }
                    }
                }
                self.mark_automations_dirty();
                let notify_ok = self.settings.notifications.done || self.settings.notifications.failed;
                if let Some((name, proj, title)) = notify_info
                    && !self.window_focused
                    && notify_ok
                {
                    crate::platform::notify(&name, &proj, &title);
                }
                if let Some(next_trigger) = queued {
                    self.start_automation_run(&auto_id, next_trigger);
                }
            }
        }
    }

    pub fn mark_automation_run_reviewed(&mut self, auto_id: &str, run_id: &str) {
        if let Some(a) = self.automations.iter_mut().find(|x| x.id == auto_id)
            && let Some(r) = a.runs.iter_mut().find(|r| r.id == run_id)
        {
            r.reviewed = true;
            if r.status == RunStatus::NeedsReview {
                r.status = RunStatus::Clean;
            }
            self.mark_automations_dirty();
        }
    }

    pub fn mark_all_automation_alerts_reviewed(&mut self) {
        let mut changed = false;
        for a in &mut self.automations {
            for r in &mut a.runs {
                if r.unreviewed_alert() {
                    r.reviewed = true;
                    if r.status == RunStatus::NeedsReview {
                        r.status = RunStatus::Clean;
                    }
                    changed = true;
                }
            }
        }
        if changed {
            self.mark_automations_dirty();
        }
    }

    /// Registers/selects the automation run's worktree (or project) and opens Focus or Review.
    pub fn open_automation_run_workspace(&mut self, ctx: &egui::Context, auto_id: &str, run_id: &str, open_review: bool) {
        let Some((proj_path, run)) = self.automations.iter().find(|a| a.id == auto_id).and_then(|a| {
            let r = a.runs.iter().find(|r| r.id == run_id)?.clone();
            Some((a.project_path.clone(), r))
        }) else {
            return;
        };
        let raw_dir = if run.workspace_path.is_dir() { run.workspace_path.clone() } else { proj_path.clone() };
        let target_dir = crate::paths::canonical(&raw_dir);
        let canonical_proj = crate::paths::canonical(&proj_path);
        let pid: ProjectId = if let Some(existing) = self
            .projects
            .iter()
            .find(|p| crate::paths::canonical(&p.path) == target_dir)
            .map(|p| p.id)
        {
            existing
        } else if let (Some(branch), Some(base_commit)) = (&run.worktree_branch, &run.worktree_base_commit)
            && let Some(root_id) = self
                .projects
                .iter()
                .find(|p| crate::paths::canonical(&p.path) == canonical_proj)
                .map(|p| p.id)
        {
            // Attach this automation's worktree as a real child project so Review / Ship / Terminal all work natively!
            let id = self.new_id();
            let color = self.project(root_id).map_or(0, |p| p.color);
            let canonical_dir = target_dir.clone();
            let wt_save = tc_core::model::WorktreeSave {
                branch: branch.clone(),
                base: tc_git::worktree::current_branch(&proj_path).unwrap_or_else(|| "main".into()),
                start: base_commit.clone(),
            };
            let size = self.grid_in(&tc_term::layout::Layout::Pane(0), 0);
            let pane_id = self.new_pane(ctx, id, canonical_dir.clone(), size);
            let mut proj = crate::project::Project::from_save(&tc_core::model::ProjectSave {
                id,
                name: branch.clone(),
                path: canonical_dir,
                color,
                parent: Some(root_id),
                worktree: Some(wt_save),
                ..Default::default()
            });
            proj.tabs = vec![crate::project::Tab {
                layout: tc_term::layout::Layout::Pane(pane_id),
                focused: pane_id,
                title: None,
            }];
            self.watch_project(ctx, &proj);
            self.projects.push(proj);
            if let Some(parent) = self.project_mut(root_id) {
                parent.folded = false;
            }
            self.sort_projects();
            self.refresh_git(id);
            self.mark_dirty();
            id
        } else if let Some(root_id) = self
            .projects
            .iter()
            .find(|p| crate::paths::canonical(&p.path) == canonical_proj)
            .map(|p| p.id)
        {
            root_id
        } else {
            self.toast(t("Project is no longer in the workspace.").into());
            return;
        };

        self.active = Some(pid);
        self.view = tc_core::model::View::Focus;
        self.ensure_terminals_visible();
        if open_review {
            self.actions.push(crate::actions::Action::Review);
        } else {
            self.focus_request = self.current_pane();
        }
        self.toast_with(
            ToastKind::Ok,
            tf("Opened run #{} in {}", &[&run.run_number.to_string(), &self.project_label(pid)]),
            None,
        );
    }

    pub fn toggle_automation_enabled(&mut self, auto_id: &str) {
        let now = now_epoch_secs();
        if let Some(a) = self.automations.iter_mut().find(|x| x.id == auto_id) {
            a.enabled = !a.enabled;
            a.recompute_next(now);
            let msg = if a.enabled {
                tf("Enabled automation \"{}\".", &[&a.name])
            } else {
                tf("Paused automation \"{}\".", &[&a.name])
            };
            self.mark_automations_dirty();
            self.toast_ok(msg);
        }
    }

    pub fn delete_automation(&mut self, auto_id: &str) {
        if let Some(active) = self.active_automation_runs.remove(auto_id) {
            active.cancel_now();
        }
        let name = self.automations.iter().find(|a| a.id == auto_id).map(|a| a.name.clone());
        self.automations.retain(|a| a.id != auto_id);
        if self.automation_sel.as_deref() == Some(auto_id) {
            self.automation_sel = self.automations.first().map(|a| a.id.clone());
            self.automation_run_ix = 0;
        }
        self.mark_automations_dirty();
        if let Some(name) = name {
            self.toast(tf("Deleted automation \"{}\".", &[&name]));
        }
    }

    pub fn save_automation_draft(&mut self, draft: AutomationDraft, run_now: bool) -> Result<String, String> {
        let now = now_epoch_secs();
        let name = draft.name.trim();
        if name.is_empty() {
            return Err("Give this automation a name.".into());
        }
        if draft.prompt.trim().is_empty() {
            return Err("Write a prompt or command for this automation.".into());
        }
        if draft.project_path.as_os_str().is_empty() {
            return Err("Choose a project folder first.".into());
        }
        let schedule = draft.build_schedule(now)?;
        schedule.validate()?;

        let id = if let Some(edit_id) = draft.editing_id.clone() {
            if let Some(slot) = self.automations.iter_mut().find(|a| a.id == edit_id) {
                slot.name = name.to_string();
                slot.project_path = draft.project_path;
                slot.project_name = draft.project_name;
                slot.agent = draft.agent;
                slot.profile_id = draft.profile_id;
                slot.schedule = schedule;
                slot.prompt = draft.prompt.trim().to_string();
                slot.session_mode = draft.session_mode;
                slot.workspace_mode = draft.workspace_mode;
                slot.quiet_when_clean = draft.quiet_when_clean;
                slot.overlap = draft.overlap;
                slot.timeout_mins = draft.timeout_mins.max(1);
                slot.missed = draft.missed;
                if run_now {
                    slot.enabled = true;
                }
                slot.recompute_next(now);
            }
            edit_id
        } else {
            let new_id = format!("auto-{}-{}", profiles::sanitize_id(name), now);
            let mut item = Automation {
                id: new_id.clone(),
                name: name.to_string(),
                project_path: draft.project_path,
                project_name: draft.project_name,
                agent: draft.agent,
                profile_id: draft.profile_id,
                schedule,
                prompt: draft.prompt.trim().to_string(),
                enabled: run_now || draft.enabled,
                session_mode: draft.session_mode,
                workspace_mode: draft.workspace_mode,
                quiet_when_clean: draft.quiet_when_clean,
                overlap: draft.overlap,
                timeout_mins: draft.timeout_mins.max(1),
                missed: draft.missed,
                last_session_id: None,
                last_triggered_at: None,
                next_run_at: None,
                runs: Vec::new(),
            };
            item.recompute_next(now);
            self.automations.insert(0, item);
            new_id
        };

        self.automation_sel = Some(id.clone());
        self.automation_run_ix = 0;
        self.automation_tab = AutomationDetailTab::Runs;
        self.mark_automations_dirty();

        if run_now {
            self.start_automation_run(&id, RunTrigger::Manual);
        } else {
            self.toast_ok(tf("Saved automation \"{}\".", &[name]));
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_parser_handles_ranges_steps_lists_and_rejects_invalid() {
        let c = CronExpr::parse("*/15 9-17 * * 1-5").unwrap();
        assert!(c.minutes.contains(&0) && c.minutes.contains(&15) && c.minutes.contains(&45) && !c.minutes.contains(&10));
        assert!(c.hours.contains(&9) && c.hours.contains(&17) && !c.hours.contains(&18));
        assert!(c.dows.contains(&1) && c.dows.contains(&5) && !c.dows.contains(&0));

        // Sunday as 7 maps cleanly to 0.
        let sun = CronExpr::parse("30 8 * * 7").unwrap();
        assert!(sun.dows.contains(&0));

        assert!(CronExpr::parse("0 25 * * *").is_err());
        assert!(CronExpr::parse("60 2 * * *").is_err());
        assert!(CronExpr::parse("* * *").is_err());
        assert!(CronExpr::parse("*/0 * * * *").is_err());

        let now = now_epoch_secs();
        let next = c.next_after(now).expect("must find next weekday slot");
        assert!(next > now);
        let dt = Local.timestamp_opt(next, 0).single().unwrap();
        assert!(c.minutes.contains(&dt.minute()));
        assert!(c.hours.contains(&dt.hour()));
        assert!(c.dows.contains(&dt.weekday().num_days_from_sunday()));
    }

    #[test]
    fn schedule_kinds_compute_future_slots_and_summaries() {
        let now = now_epoch_secs();
        for s in [
            ScheduleKind::Interval { every_minutes: 30 },
            ScheduleKind::Daily { hour: 2, minute: 0 },
            ScheduleKind::Weekdays { hour: 9, minute: 0 },
            ScheduleKind::Weekly { weekday: 0, hour: 8, minute: 30 },
            ScheduleKind::Cron { expr: "0 */4 * * *".into() },
            ScheduleKind::Once { at_epoch_secs: now + 600 },
        ] {
            assert!(s.validate().is_ok(), "{s:?}");
            let next = s.next_after(now).expect("future slot");
            assert!(next > now, "{s:?}");
            assert!(!s.short_label().is_empty());
            assert!(!s.preview_sentence().is_empty());
        }
        // Past one-shot returns None.
        assert_eq!(ScheduleKind::Once { at_epoch_secs: now - 10 }.next_after(now), None);
    }

    #[test]
    fn natural_language_parser_extracts_cadence_time_and_isolation() {
        let mut d = AutomationDraft::new_for(PathBuf::from("/tmp/demo"), "demo".into());
        d.nl_input = "Every weekday at 9am check open PRs and fix failing clippy lints in a worktree using claude".into();
        d.apply_natural_language();
        assert_eq!(d.agent, AgentKind::Claude);
        assert_eq!(d.cadence_kind, CadenceKind::Weekdays);
        assert_eq!(d.time_text, "09:00");
        assert_eq!(d.workspace_mode, WorkspaceMode::Worktree);
        assert_eq!(d.session_mode, SessionMode::Fresh);
        assert!(!d.name.is_empty());

        let mut d2 = AutomationDraft::new_for(PathBuf::from("/tmp/demo"), "demo".into());
        d2.nl_input = "Every 4 hours continue previous run to watch staging logs".into();
        d2.apply_natural_language();
        assert_eq!(d2.cadence_kind, CadenceKind::Interval);
        assert_eq!(d2.time_text, "240");
        assert_eq!(d2.workspace_mode, WorkspaceMode::ProjectDir);
        assert_eq!(d2.session_mode, SessionMode::Continue);
    }

    #[test]
    fn strip_ansi_cleans_csi_and_osc_sequences() {
        let raw = "\x1b[32mPASS\x1b[0m \x1b]0;Title\x07done\r\n";
        assert_eq!(strip_ansi(raw), "PASS done\n");
    }

    #[test]
    fn real_shell_automation_in_isolated_worktree_preserves_changes_and_cleans_empty_runs() {
        // Create a real temp git repo so we exercise real worktree add, diff stats, and cleanup.
        let root = std::env::temp_dir().join(format!("tc-auto-test-{}", now_epoch_secs()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let git = |args: &[&str]| {
            let st = Command::new("git")
                .args(args)
                .current_dir(&root)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .status()
                .unwrap();
            assert!(st.success(), "git {args:?} failed");
        };
        git(&["init", "-b", "main"]);
        std::fs::write(root.join("hello.txt"), "hello\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "init"]);

        // 1. Clean run with no edits: worktree must be auto-cleaned and status == Clean (quiet).
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = egui::Context::default();
        execute_run(
            RunSpec {
                auto_id: "a1".into(),
                run_id: "r1".into(),
                run_number: 1,
                name: "Clean Check".into(),
                project_path: root.clone(),
                agent: AgentKind::Custom,
                prompt: "echo all-ok".into(),
                session_mode: SessionMode::Fresh,
                last_session_id: None,
                workspace_mode: WorkspaceMode::Worktree,
                worktree_setup: WorktreeSetup::default(),
                quiet_when_clean: true,
                auto_failover: false,
                candidates: vec![None],
                secrets: Vec::new(),
                path_env: std::env::var_os("PATH"),
                home: paths::home_dir(),
                data_dir: paths::data_dir().to_path_buf(),
                worktree_prefix: "tc/".into(),
                cancel: Arc::new(AtomicBool::new(false)),
                child_pid: Arc::new(Mutex::new(None)),
            },
            tx,
            ctx.clone(),
        );
        let fin1 = rx
            .try_iter()
            .find_map(|m| match m {
                AppMsg::Automation(AutomationMsg::Finished { outcome, .. }) => Some(outcome),
                _ => None,
            })
            .expect("finished outcome");
        assert_eq!(fin1.status, RunStatus::Clean);
        assert!(!fin1.notified_inbox);
        assert!(fin1.worktree_branch.is_none(), "empty worktree must be auto-cleaned");

        // 2. Run that edits a file: worktree must be preserved with diff stats & NeedsReview!
        let (tx2, rx2) = std::sync::mpsc::channel();
        execute_run(
            RunSpec {
                auto_id: "a1".into(),
                run_id: "r2".into(),
                run_number: 2,
                name: "Edit Fix".into(),
                project_path: root.clone(),
                agent: AgentKind::Custom,
                prompt: "echo fixed >> hello.txt && echo patched-file".into(),
                session_mode: SessionMode::Fresh,
                last_session_id: None,
                workspace_mode: WorkspaceMode::Worktree,
                worktree_setup: WorktreeSetup::default(),
                quiet_when_clean: true,
                auto_failover: false,
                candidates: vec![None],
                secrets: Vec::new(),
                path_env: std::env::var_os("PATH"),
                home: paths::home_dir(),
                data_dir: paths::data_dir().to_path_buf(),
                worktree_prefix: "tc/".into(),
                cancel: Arc::new(AtomicBool::new(false)),
                child_pid: Arc::new(Mutex::new(None)),
            },
            tx2,
            ctx,
        );
        let fin2 = rx2
            .try_iter()
            .find_map(|m| match m {
                AppMsg::Automation(AutomationMsg::Finished { outcome, .. }) => Some(outcome),
                _ => None,
            })
            .expect("finished outcome 2");
        assert_eq!(fin2.status, RunStatus::NeedsReview);
        assert!(fin2.notified_inbox);
        assert_eq!(fin2.files_changed, 1);
        assert_eq!(fin2.additions, 1);
        assert!(fin2.worktree_branch.is_some(), "dirty worktree must be kept for review");
        assert_eq!(std::fs::read_to_string(root.join("hello.txt")).unwrap(), "hello\n", "main repo untouched!");

        // Cleanup test worktree + temp repo.
        if let Some(br) = &fin2.worktree_branch {
            let _ = tc_git::worktree::remove(&root, &fin2.workspace_path, true);
            let _ = tc_git::worktree::delete_branch(&root, br, true);
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
