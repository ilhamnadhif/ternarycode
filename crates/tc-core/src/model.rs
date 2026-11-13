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
    /// One project, its split panes.
    #[default]
    Focus,
    /// All projects at once. (Spotlight, one project large and the others as a strip, was
    /// removed: workspaces saved in it open in the Grid.)
    #[serde(alias = "spotlight")]
    Grid,
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
            skipped_version: None,
            last_update_check: None,
            window: None,
            sidebar_width: 248.0,
            sidebar_hidden: false,
            files_height: 0.0,
            files_hidden: false,
            ui_rev: UI_REV,
            lsp: true,
            lsp_on_demand: true,
            auto_complete: true,
            format_on_save: false,
            cursor_shape: CursorShape::Block,
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
        self.scrollback = self.scrollback.clamp(100, 1_000_000);
        self.scrollback_budget_mb = self.scrollback_budget_mb.clamp(16, 16_384);
        self.long_command_secs = self.long_command_secs.clamp(1, 86_400);
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

    /// The presets in use: built-ins (with the user's edits) followed by the user's own.
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
                None => out.push(p.clone()),
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
    }
}
