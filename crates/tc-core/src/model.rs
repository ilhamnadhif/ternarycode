//! What TernaryCode saves: the workspace (projects, their tabs and split layouts, each pane's
//! directory and what ran in it) and the user's settings. Every field has a default so older
//! files keep loading as fields are added.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent::{AgentKind, Preset};
use crate::{PaneId, ProjectId};

pub const WORKSPACE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    /// One project: wide code editor + collapsible terminal/agent drawer.
    #[default]
    Focus,
    /// All projects at once. (Spotlight, one project large and the others as a strip, was
    /// removed: workspaces saved in it open in the Grid.)
    #[serde(alias = "spotlight")]
    Grid,
    /// Multi-account agent profiles and rate-limit failover configuration.
    Profiles,
    /// Unified real-time usage and status monitor across all agents and profiles.
    Monitor,
    /// Phone pairing and mobile companion server management.
    Pairing,
}

/// Where the secondary drawer (terminals, raw agent logs, inspectors) docks in Focus view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawerDock {
    #[default]
    Bottom,
    Right,
}

/// Which tab is active in the secondary workspace drawer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawerTab {
    #[default]
    Terminals,
    Logs,
    Problems,
}

/// What happens when an active agent account hits its usage/rate limit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailoverMode {
    /// Automatically switch to the next healthy profile in the fallback chain and resume.
    #[default]
    Auto,
    /// Pause with an inline recovery banner for one-click profile selection and resume.
    Manual,
}

/// An independent agent account profile (e.g. `claude1`, `claude2`, `chatgpt1`, `gemini1`),
/// isolated via its own auth/config directory and optional API key / environment overrides.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub agent: AgentKind,
    /// Isolated config/auth folder (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_CLI_HOME`, …).
    /// `None` uses the provider's default folder for the first profile or `<data>/profiles/<id>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<PathBuf>,
    /// Optional API key injected for this profile only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Extra environment variables injected for launches using this profile.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra_env: BTreeMap<String, String>,
    /// Priority in the fallback chain (lower runs first).
    pub fallback_order: u32,
    /// Whether this profile is active in the switcher and fallback chain.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Optional token budget used to compute remaining quota percentage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_token_budget: Option<u64>,
    /// Uses the system default auth folder (`~/.claude`, `~/.codex`, `~/.gemini`) rather than an
    /// isolated subfolder.
    #[serde(default)]
    pub use_default_auth: bool,
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            agent: AgentKind::Claude,
            config_dir: None,
            api_key: None,
            extra_env: BTreeMap::new(),
            fallback_order: 0,
            enabled: true,
            daily_token_budget: None,
            use_default_auth: false,
        }
    }
}

/// Multi-account rate-limit failover settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FailoverSettings {
    pub mode: FailoverMode,
    /// When every profile of the current provider is rate-limited, fail over to the next healthy
    /// profile from another provider.
    pub cross_provider: bool,
    /// Fallback cooldown duration (minutes) when the agent output does not specify a reset time.
    pub default_cooldown_mins: u32,
}

impl Default for FailoverSettings {
    fn default() -> Self {
        Self {
            mode: FailoverMode::Auto,
            cross_provider: true,
            default_cooldown_mins: 30,
        }
    }
}

/// A paired smartphone/tablet authorised to control this desktop ADE via the Mobile Companion.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    /// SHA-256 hex digest of the device's bearer token (the raw token is never stored in plain text).
    pub token_hash: String,
    pub ip: String,
    pub paired_at: i64,
    pub last_seen: i64,
}

/// Embedded local HTTP/SSE companion server settings for Phone Pairing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompanionSettings {
    pub enabled: bool,
    pub port: u16,
    /// Optional external tunnel URL (e.g. Cloudflare Tunnel / Tailscale Funnel / ngrok) used in
    /// the QR code instead of the LAN IP.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tunnel_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<PairedDevice>,
}

impl Default for CompanionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 7890,
            tunnel_url: String::new(),
            devices: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub version: u32,
    pub projects: Vec<ProjectSave>,
    pub active: Option<ProjectId>,
    pub view: View,
    /// Columns of the Grid view (0 = automatic).
    pub grid_columns: u32,
    /// The Grid shows only projects where something runs or waits (written for older
    /// versions; `grid_filter` says it now).
    pub grid_active_only: bool,
    /// Which projects the Grid shows. Files written before it existed have only
    /// `grid_active_only`.
    pub grid_filter: Option<GridFilter>,
    pub next_id: u64,
    /// The sidebar's groups folded away ("active", "idle").
    pub sidebar_folded: Vec<String>,
}

/// Which projects the Grid shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridFilter {
    #[default]
    All,
    /// Where something runs or waits.
    Active,
    /// Only what waits for the user (the attention queue).
    Waiting,
}

impl Workspace {
    /// The Grid's filter, from the field older versions wrote when it is missing.
    pub fn filter(&self) -> GridFilter {
        self.grid_filter.unwrap_or(if self.grid_active_only { GridFilter::Active } else { GridFilter::All })
    }
}

/// The shape of the terminal cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorShape {
    #[default]
    Block,
    Beam,
    Underline,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectSave {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,
    /// Index into the project colour palette.
    pub color: u8,
    /// Preset started by "New agent" in this project.
    pub preset: Option<String>,
    pub tabs: Vec<TabSave>,
    pub active_tab: usize,
    pub panes: Vec<PaneSave>,
    /// Share of the area the editor takes beside the panes (0 = default).
    pub editor_width: f32,
    /// The project this one is a worktree of: it is listed under it.
    pub parent: Option<ProjectId>,
    /// A worktree TernaryCode made for an agent run: its branch and where that started.
    pub worktree: Option<WorktreeSave>,
    /// What a new worktree of this project gets (files git ignores, a command).
    pub worktree_setup: Option<WorktreeSetup>,
    /// Its worktrees are folded away in the sidebar.
    pub folded: bool,
    /// Run Agent last chose to work in a new worktree here.
    pub new_worktree: bool,
    /// Sites (hosts) its agents may open in its browser without asking, besides the ones every
    /// project's agents may (Settings).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub browser_sites: Vec<String>,
    /// Its Browser panel (its pages are in `panes`, in the panel's order).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_panel: Option<PanelSave>,
    /// What its commit button does, when not the default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ship: Option<ShipPrefs>,
    /// Whether nested agent session history is expanded under this project in the sidebar tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sessions_expanded: Option<bool>,
    /// Preferred agent profile id for this project (e.g. `claude1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<String>,
}

/// What a project's commit button does (its ▾ menu), and whether its pull requests start as
/// drafts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShipPrefs {
    pub action: ShipAction,
    pub draft: bool,
}

/// How far the commit button goes. On the branch a pull request would go into (main), it
/// pushes there instead of opening one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShipAction {
    Commit,
    CommitPush,
    #[default]
    PullRequest,
}

/// A project's Browser panel: shown or not, its width, the page in front.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelSave {
    pub open: bool,
    /// Share of the area beside the sidebar it takes (0 = default).
    pub width: f32,
    pub front: Option<PaneId>,
}

/// A worktree made for an agent run (Run Agent → New worktree).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorktreeSave {
    /// Its own branch (`tc/generics`).
    pub branch: String,
    /// The branch it started from, which its work is reviewed against and merged into.
    pub base: String,
    /// The commit it started at (a branch still there is not "merged" before it has commits).
    pub start: String,
}

/// What a new worktree of a project gets: files git ignores (they are not in a new worktree)
/// copied from the project, and a command run in it before the agent starts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorktreeSetup {
    pub copy: Vec<String>,
    pub run: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabSave {
    pub layout: SavedLayout,
    pub focused: PaneId,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedDir {
    /// Side by side.
    Horizontal,
    /// Stacked.
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedLayout {
    Pane(PaneId),
    Split {
        dir: SavedDir,
        ratio: f32,
        a: Box<SavedLayout>,
        b: Box<SavedLayout>,
    },
}

impl SavedLayout {
    pub fn panes(&self) -> Vec<PaneId> {
        match self {
            SavedLayout::Pane(p) => vec![*p],
            SavedLayout::Split { a, b, .. } => {
                let mut v = a.panes();
                v.extend(b.panes());
                v
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneSave {
    pub id: PaneId,
    /// The shell's directory when last seen.
    pub cwd: Option<PathBuf>,
    pub title: Option<String>,
    /// The agent that was running when the workspace was saved, and the preset that started it.
    pub agent: Option<AgentKind>,
    pub preset: Option<String>,
    /// A page of the project's Browser panel: its address (older versions: a browser pane in a
    /// tab).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
    /// The agent pane a page belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_owner: Option<PaneId>,
    /// Active multi-account profile id bound to this pane (e.g. `claude1`, `chatgpt2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    System,
    Light,
    #[default]
    Dark,
}

/// The revision of the UI defaults a settings file was written with (see [`Settings::migrate`]).
pub const UI_REV: u32 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    System,
    English,
    Indonesian,
}

/// How hooks reach TernaryCode for one agent (PRD section 8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Integration {
    /// Flags added when a preset starts the agent; no file is changed.
    #[default]
    PerLaunch,
    /// Hooks installed in the agent's own configuration (works when typed by hand too).
    Installed,
    /// Neither: foreground process and OSC only.
    Off,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub needs_input: bool,
    pub failed: bool,
    pub done: bool,
    pub command_done: bool,
    pub dock_badge: bool,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            needs_input: true,
            failed: true,
            done: true,
            command_done: true,
            dock_badge: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    pub theme: ThemeMode,
    pub scheme_dark: String,
    pub scheme_light: String,
    pub font_name: String,
    pub font_path: Option<String>,
    pub font_size: f32,
    /// Scrollback lines per pane.
    pub scrollback: usize,
    /// All panes' scrollback together, in MB (oldest trimmed first in panes out of view).
    pub scrollback_budget_mb: usize,
    pub cursor_blink: bool,
    pub option_as_meta: bool,
    pub copy_on_select: bool,
    /// Programs in the panes may put text on the clipboard (OSC 52). Reading it is never
    /// allowed: any program could take what was copied elsewhere.
    #[serde(default = "yes")]
    pub program_copy: bool,
    /// Commands shorter than this never trigger "command finished".
    pub long_command_secs: u64,
    pub notifications: Notifications,
    /// User presets and edits of the built-in ones, by id.
    pub presets: Vec<Preset>,
    pub integrations: BTreeMap<String, Integration>,
    /// Agents (by slug) Run Agent starts without their permission prompts (Claude Code's
    /// `--dangerously-skip-permissions`, …), as last chosen there.
    pub skip_permissions: Vec<String>,
    /// Shell integration (OSC 133) injected into zsh, bash and fish.
    pub shell_integration: bool,
    /// Look for a new version on GitHub at launch and every few hours.
    pub check_updates: bool,
    /// A version the user chose not to be told about again.
    #[serde(default)]
    pub skipped_version: Option<String>,
    /// When the last check answered (Unix seconds).
    #[serde(default)]
    pub last_update_check: Option<i64>,
    pub window: Option<[f32; 4]>,
    pub sidebar_width: f32,
    pub sidebar_hidden: bool,
    /// The sidebar as a narrow column of project tiles (⌥⌘B).
    #[serde(default)]
    pub sidebar_compact: bool,
    /// Height of the Files section of the sidebar.
    pub files_height: f32,
    pub files_hidden: bool,
    /// UI defaults revision; a file without it was written before revision 1.
    #[serde(default)]
    pub ui_rev: u32,
    /// The editor's language features (language servers: definitions, rename, completion…).
    #[serde(default = "yes")]
    pub lsp: bool,
    /// A language server starts only when a file it serves is edited or a language feature is
    /// asked for (definition, references, rename…), not when a file is only opened to be read.
    #[serde(default = "yes")]
    pub lsp_on_demand: bool,
    /// Completion pops up while typing (else only with ⌃Space).
    #[serde(default = "yes")]
    pub auto_complete: bool,
    /// Format a file with its language server when saving it.
    #[serde(default)]
    pub format_on_save: bool,
    /// The shape of the terminal cursor.
    #[serde(default)]
    pub cursor_shape: CursorShape,
    /// Where new worktrees go and what their branches are called.
    #[serde(default)]
    pub worktrees: WorktreeSettings,
    /// The sidebar lists projects where something runs or waits first, under "Active".
    #[serde(default = "yes")]
    pub group_active: bool,
    /// Web pages (server chips, links clicked in terminals, HTML files) open in a browser pane
    /// beside the terminals, not in the default browser.
    #[serde(default = "yes")]
    pub browser_here: bool,
    /// Minutes a browser pane out of view and unused keeps its page before letting it go.
    #[serde(default = "ten")]
    pub browser_sleep_minutes: u32,
    /// Agents in TernaryCode panes may use their project's browser (its tools).
    #[serde(default = "yes")]
    pub browser_agents: bool,
    /// An agent's page shows the project's Browser panel (else only the Browser button says so).
    #[serde(default = "yes")]
    pub browser_agent_beside: bool,
    /// Pages in Grid tiles keep the layout they have in Focus, zoomed out (else they fit the tile).
    #[serde(default = "yes")]
    pub browser_grid_zoom: bool,
    /// Agents ask the user before opening a site not on the lists (else they open any site).
    #[serde(default)]
    pub browser_agent_ask: bool,
    /// Sites any agent may open without asking, when it asks (`*.local`: that domain and its
    /// subdomains).
    #[serde(default = "agent_sites")]
    pub browser_agent_sites: Vec<String>,
    /// Agents may run their own JavaScript in pages (`browser_evaluate`).
    #[serde(default)]
    pub browser_agent_js: bool,
    /// Multi-account agent profiles (e.g. `claude1`, `claude2`, `chatgpt1`, `gemini1`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<AgentProfile>,
    /// Rate-limit failover policy (auto-resume vs manual recovery banner).
    #[serde(default)]
    pub failover: FailoverSettings,
    /// Where the secondary terminal/logs drawer docks (`bottom` or `right`).
    #[serde(default)]
    pub drawer_dock: DrawerDock,
    /// Whether the secondary drawer is open.
    #[serde(default = "yes")]
    pub drawer_open: bool,
    /// Fraction of the workspace allocated to the secondary drawer (`0.18..=0.70`, default `0.36`
    /// so the Code Editor keeps >= 60% of the primary viewport by default).
    #[serde(default = "default_drawer_size")]
    pub drawer_size: f32,
    /// Code editor & diff viewer commands the primary viewport by default.
    #[serde(default = "yes")]
    pub editor_first: bool,
    /// Phone pairing and mobile companion server settings.
    #[serde(default)]
    pub companion: CompanionSettings,
}

/// The sites agents open without asking unless the user changes the list: this machine's.
pub fn agent_sites() -> Vec<String> {
    vec!["localhost".into(), "127.0.0.1".into(), "*.local".into()]
}

/// Where Run Agent puts a new worktree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreePlace {
    /// Beside the repository: `‹repo›.worktrees/‹name›`.
    #[default]
    Beside,
    /// In TernaryCode's data folder.
    DataFolder,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorktreeSettings {
    pub place: WorktreePlace,
    /// What a worktree's branch name starts with (`tc/`).
    pub prefix: String,
    /// When a worktree's branch is merged, offer to remove the worktree.
    pub offer_remove: bool,
}

impl Default for WorktreeSettings {
    fn default() -> Self {
        Self { place: WorktreePlace::Beside, prefix: "tc/".into(), offer_remove: true }
    }
}

fn ten() -> u32 {
    10
}

fn default_drawer_size() -> f32 {
    0.36
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::System,
            theme: ThemeMode::Dark,
            scheme_dark: String::new(),
            scheme_light: String::new(),
            font_name: String::new(),
            font_path: None,
            font_size: 13.0,
            scrollback: 10_000,
            scrollback_budget_mb: 512,
            cursor_blink: true,
            option_as_meta: true,
            copy_on_select: false,
            program_copy: true,
            long_command_secs: 10,
            notifications: Notifications::default(),
            presets: Vec::new(),
            integrations: BTreeMap::new(),
            skip_permissions: Vec::new(),
            shell_integration: true,
            check_updates: true,
            group_active: true,
            browser_here: true,
            browser_sleep_minutes: 10,
            browser_agents: true,
            browser_agent_beside: true,
            browser_agent_ask: false,
            browser_agent_sites: agent_sites(),
            browser_agent_js: false,
            browser_grid_zoom: true,
            skipped_version: None,
            last_update_check: None,
            window: None,
            sidebar_width: 248.0,
            sidebar_hidden: false,
            sidebar_compact: false,
            files_height: 0.0,
            files_hidden: false,
            ui_rev: UI_REV,
            lsp: true,
            lsp_on_demand: true,
            auto_complete: true,
            format_on_save: false,
            cursor_shape: CursorShape::Block,
            worktrees: WorktreeSettings::default(),
            profiles: Vec::new(),
            failover: FailoverSettings::default(),
            drawer_dock: DrawerDock::Bottom,
            drawer_open: true,
            drawer_size: default_drawer_size(),
            editor_first: true,
            companion: CompanionSettings::default(),
        }
    }
}

impl Settings {
    /// Brings values typed into the file by hand into range (a font size of 0 would make the
    /// app unusable before Settings could be opened to fix it). Returns whether one changed.
    pub fn sanitize(&mut self) -> bool {
        let before = self.clone();
        let d = Settings::default();
        let fix = |v: f32, lo: f32, hi: f32, default: f32| if v.is_finite() { v.clamp(lo, hi) } else { default };
        self.font_size = fix(self.font_size, 6.0, 72.0, d.font_size);
        self.sidebar_width = fix(self.sidebar_width, 180.0, 420.0, d.sidebar_width);
        self.files_height = fix(self.files_height, 0.0, 4000.0, d.files_height);
        self.drawer_size = fix(self.drawer_size, 0.18, 0.70, d.drawer_size);
        self.scrollback = self.scrollback.clamp(100, 1_000_000);
        self.scrollback_budget_mb = self.scrollback_budget_mb.clamp(16, 16_384);
        self.long_command_secs = self.long_command_secs.clamp(1, 86_400);
        self.failover.default_cooldown_mins = self.failover.default_cooldown_mins.clamp(1, 1440);
        self.companion.port = self.companion.port.clamp(1024, 65535);
        // Sanitize & deduplicate hand-edited profiles so IDs, paths, keys, and env names are valid.
        let mut seen_ids = std::collections::HashSet::new();
        for (idx, pr) in self.profiles.iter_mut().enumerate() {
            let mut id: String = pr
                .id
                .trim()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .take(32)
                .collect::<String>()
                .to_ascii_lowercase();
            if !id.chars().any(|c| c.is_ascii_alphanumeric()) {
                id = format!("profile{}", idx + 1);
            }
            if !seen_ids.insert(id.clone()) {
                let mut n = 2u32;
                while !seen_ids.insert(format!("{id}-{n}")) {
                    n += 1;
                }
                id = format!("{id}-{n}");
            }
            pr.id = id;
            if pr.name.trim().is_empty() {
                pr.name = pr.id.clone();
            }
            if pr.config_dir.as_ref().is_some_and(|d| d.as_os_str().is_empty()) {
                pr.config_dir = None;
            }
            if let Some(k) = pr.api_key.as_ref() {
                let t = k.trim();
                if t.is_empty() {
                    pr.api_key = None;
                } else if t.len() != k.len() {
                    pr.api_key = Some(t.to_string());
                }
            }
            if pr.daily_token_budget == Some(0) {
                pr.daily_token_budget = None;
            }
            pr.extra_env.retain(|k, _| {
                k.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
        }
        self.profiles.sort_by_key(|p| (p.fallback_order, p.id.clone()));
        *self != before
    }

    /// Brings settings written by an older version to the current UI defaults (and into range,
    /// see [`Settings::sanitize`]). Revision 1 made the dark theme the default: "System" saved
    /// before it was the old default, not a choice. Returns whether anything changed (the file
    /// should be saved).
    pub fn migrate(&mut self) -> bool {
        let fixed = self.sanitize();
        if self.ui_rev >= UI_REV {
            return fixed;
        }
        if self.ui_rev < 1 && self.theme == ThemeMode::System {
            self.theme = ThemeMode::Dark;
        }
        // Revision 2: update checks exist (the setting was saved off before they did).
        if self.ui_rev < 2 {
            self.check_updates = true;
        }
        // Revision 3: the sidebar is 248 wide by default, and the Files section became the
        // Files column of the Code drawer (shown by default).
        if self.ui_rev < 3 {
            if self.sidebar_width == 240.0 {
                self.sidebar_width = 248.0;
            }
            self.files_hidden = false;
        }
        self.ui_rev = UI_REV;
        true
    }

    /// The presets in use: built-ins (with the user's edits) followed by the user's own. A
    /// built-in the user edited that is no longer one (Gemini CLI's) stays, as their own.
    pub fn presets(&self) -> Vec<Preset> {
        let mut out = crate::agent::builtin_presets();
        for p in &self.presets {
            match out.iter_mut().find(|b| b.id == p.id) {
                Some(b) => {
                    *b = Preset {
                        builtin: true,
                        ..p.clone()
                    }
                }
                None => out.push(Preset { builtin: false, ..p.clone() }),
            }
        }
        out
    }

    pub fn integration(&self, agent: AgentKind) -> Integration {
        self.integrations
            .get(agent.slug())
            .copied()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_partial_files_load_with_defaults() {
        let w: Workspace = serde_json::from_str("{}").unwrap();
        assert!(w.projects.is_empty());
        let s: Settings = serde_json::from_str(r#"{"font_size": 15}"#).unwrap();
        assert_eq!(s.font_size, 15.0);
        assert_eq!(s.scrollback, 10_000);
        assert!(s.notifications.needs_input);
        assert!(s.program_copy, "programs may copy unless turned off");
    }

    #[test]
    fn a_workspace_saved_in_spotlight_opens_in_the_grid() {
        let ws: Workspace = serde_json::from_str(r#"{"version": 1, "view": "spotlight"}"#).unwrap();
        assert_eq!(ws.view, View::Grid);
    }

    #[test]
    fn grid_shows_every_project_unless_active_only_was_chosen() {
        // Workspaces saved before the choice existed (or with an older field) show all.
        let ws: Workspace = serde_json::from_str(r#"{"version": 1, "grid_columns": 2, "grid_all": false}"#).unwrap();
        assert!(!ws.grid_active_only);
        let only = Workspace { grid_active_only: true, ..Default::default() };
        let back: Workspace = serde_json::from_str(&serde_json::to_string(&only).unwrap()).unwrap();
        assert!(back.grid_active_only);
    }

    #[test]
    fn old_settings_move_to_the_dark_default_once() {
        // Written before revision 1: "system" was the default, not a choice.
        let mut s: Settings = serde_json::from_str(r#"{"theme": "system"}"#).unwrap();
        assert_eq!(s.ui_rev, 0);
        assert!(s.migrate());
        assert_eq!(s.theme, ThemeMode::Dark);
        assert_eq!(s.ui_rev, UI_REV);
        assert!(s.check_updates, "update checks start on");
        // Saved off before update checks existed: on once; turned off after: stays off.
        let mut before: Settings = serde_json::from_str(r#"{"check_updates": false, "ui_rev": 1}"#).unwrap();
        assert!(before.migrate() && before.check_updates);
        let mut after: Settings = serde_json::from_str(r#"{"check_updates": false, "ui_rev": 2}"#).unwrap();
        assert!(after.migrate() && !after.check_updates);
        // Revision 3: the old default sidebar width becomes the new one; a width chosen stays.
        let mut narrow: Settings = serde_json::from_str(r#"{"sidebar_width": 240, "files_hidden": true, "ui_rev": 2}"#).unwrap();
        assert!(narrow.migrate());
        assert_eq!(narrow.sidebar_width, 248.0);
        assert!(!narrow.files_hidden, "the Files column shows in the drawer");
        let mut wide: Settings = serde_json::from_str(r#"{"sidebar_width": 300, "ui_rev": 2}"#).unwrap();
        assert!(wide.migrate());
        assert_eq!(wide.sidebar_width, 300.0);
        let mut now: Settings = serde_json::from_str(r#"{"check_updates": false, "ui_rev": 3}"#).unwrap();
        assert!(!now.migrate() && !now.check_updates);
        // A light theme chosen before is kept.
        let mut light: Settings = serde_json::from_str(r#"{"theme": "light"}"#).unwrap();
        assert!(light.migrate());
        assert_eq!(light.theme, ThemeMode::Light);
        // "System" chosen after the migration stays.
        s.theme = ThemeMode::System;
        let mut back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(!back.migrate());
        assert_eq!(back.theme, ThemeMode::System);
        // New installs start dark and need nothing.
        let mut fresh = Settings::default();
        assert_eq!(fresh.theme, ThemeMode::Dark);
        assert!(!fresh.migrate());
    }

    #[test]
    fn workspace_round_trips() {
        let w = Workspace {
            version: WORKSPACE_VERSION,
            projects: vec![ProjectSave {
                id: 1,
                name: "api".into(),
                path: "/tmp/api".into(),
                tabs: vec![TabSave {
                    layout: SavedLayout::Split {
                        dir: SavedDir::Horizontal,
                        ratio: 0.6,
                        a: Box::new(SavedLayout::Pane(10)),
                        b: Box::new(SavedLayout::Pane(11)),
                    },
                    focused: 11,
                    title: None,
                }],
                panes: vec![PaneSave {
                    id: 10,
                    agent: Some(AgentKind::Claude),
                    preset: Some("claude".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            active: Some(1),
            ..Default::default()
        };
        let json = serde_json::to_string(&w).unwrap();
        let back: Workspace = serde_json::from_str(&json).unwrap();
        assert_eq!(back, w);
        assert_eq!(back.projects[0].tabs[0].layout.panes(), [10, 11]);
    }

    #[test]
    fn worktrees_load_from_older_files_and_round_trip() {
        // Written before worktrees: none, and the defaults.
        let p: ProjectSave = serde_json::from_str(r#"{"id": 3, "name": "api", "path": "/tmp/api"}"#).unwrap();
        assert!(p.parent.is_none() && p.worktree.is_none() && !p.folded && !p.new_worktree);
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.worktrees.prefix, "tc/");
        assert_eq!(s.worktrees.place, WorktreePlace::Beside);
        assert!(s.worktrees.offer_remove);
        let wt = ProjectSave {
            id: 4,
            name: "generics".into(),
            path: "/tmp/api.worktrees/generics".into(),
            parent: Some(3),
            worktree: Some(WorktreeSave { branch: "tc/generics".into(), base: "main".into(), start: "abc".into() }),
            worktree_setup: Some(WorktreeSetup { copy: vec![".env".into()], run: "npm install".into() }),
            ..Default::default()
        };
        let back: ProjectSave = serde_json::from_str(&serde_json::to_string(&wt).unwrap()).unwrap();
        assert_eq!(back, wt);
    }

    #[test]
    fn preset_edits_override_builtins() {
        let mut s = Settings::default();
        let mut claude = crate::agent::builtin_presets().remove(0);
        claude.command = "claude --model opus".into();
        claude.builtin = false;
        s.presets.push(claude);
        s.presets.push(Preset {
            id: "aider".into(),
            name: "Aider".into(),
            agent: Some(AgentKind::Custom),
            command: "aider".into(),
            resume: None,
            env: Default::default(),
            builtin: false,
        });
        let all = s.presets();
        assert_eq!(all.len(), 5);
        assert_eq!(all[0].command, "claude --model opus");
        assert!(all[0].builtin);
        assert_eq!(all[4].id, "aider");
        // An edited built-in that is no longer one (Gemini CLI's, saved before Antigravity CLI
        // replaced it): the user's own now, so it can be deleted.
        s.presets.push(Preset {
            id: "gemini".into(),
            name: "Gemini CLI".into(),
            agent: Some(AgentKind::Gemini),
            command: "gemini --model pro".into(),
            resume: Some("gemini --resume latest".into()),
            env: Default::default(),
            builtin: true,
        });
        let all = s.presets();
        assert_eq!(all.len(), 6);
        assert!(all.iter().any(|p| p.id == "antigravity" && p.command == "agy" && p.builtin));
        assert!(all.iter().any(|p| p.id == "gemini" && !p.builtin));
    }

    #[test]
    fn ade_redesign_settings_and_profiles_round_trip() {
        let mut s = Settings::default();
        assert!(s.editor_first);
        assert!(s.drawer_open);
        assert_eq!(s.drawer_dock, DrawerDock::Bottom);
        assert_eq!(s.failover.mode, FailoverMode::Auto);
        s.drawer_dock = DrawerDock::Right;
        s.drawer_size = 0.99;
        s.companion.port = 80;
        s.profiles.push(AgentProfile {
            id: "claude2".into(),
            name: "Claude Secondary".into(),
            agent: AgentKind::Claude,
            config_dir: Some("/tmp/tc-claude2".into()),
            api_key: Some("sk-ant-test".into()),
            extra_env: Default::default(),
            enabled: true,
            fallback_order: 2,
            daily_token_budget: Some(2_000_000),
            use_default_auth: false,
        });
        s.failover.mode = FailoverMode::Manual;
        s.companion.enabled = true;
        s.companion.devices.push(PairedDevice {
            id: "dev-1".into(),
            name: "iPhone 16 Pro".into(),
            token_hash: "deadbeef".into(),
            ip: "192.168.1.42".into(),
            paired_at: 1_700_000_000,
            last_seen: 1_700_000_120,
        });
        s.sanitize();
        assert!(s.drawer_size <= 0.70);
        assert!(s.companion.port >= 1024);
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        // Views serialize cleanly and fall back gracefully on unknown strings.
        for v in [View::Focus, View::Grid, View::Profiles, View::Monitor, View::Pairing] {
            let enc = serde_json::to_string(&v).unwrap();
            let dec: View = serde_json::from_str(&enc).unwrap();
            assert_eq!(dec, v);
        }
    }
}
