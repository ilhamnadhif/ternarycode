//! Everything the user can ask for, from the menu, a shortcut, the palette or a button.

use std::path::PathBuf;

use tc_core::agent::AgentKind;
use tc_core::model::View;
use tc_core::{PaneId, ProjectId};

/// Which session an agent starts with.
#[derive(Clone, Debug, PartialEq)]
pub enum Launch {
    New,
    /// Its last session (the preset's resume command, `claude --continue`).
    Last,
    /// An earlier session, by the agent's id for it.
    Session(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    AddProject,
    AddProjectPath(PathBuf),
    /// `ternarycode <target>` from a terminal: a folder (absolute) or a project's name.
    OpenTarget(String),
    SelectProject(ProjectId),
    SelectProjectIndex(usize),
    RemoveProject(ProjectId),
    RemoveProjectNow(ProjectId),
    RenameProject(ProjectId, String),
    RenamePane(PaneId, String),
    SetProjectColor(ProjectId, u8),
    SetProjectPreset(ProjectId, Option<String>),
    RevealProject(ProjectId),
    NewTab,
    SelectTab(usize),
    NextTab,
    PrevTab,
    /// Moves tab `from` to position `to` (before the tab now at `to`).
    MoveTab { from: usize, to: usize },
    /// Drops tab `tab` beside pane `target` of the active tab, which then holds both.
    MergeTab {
        tab: usize,
        target: PaneId,
        dir: tc_term::layout::SplitDir,
        before: bool,
    },
    /// Closes tab `i` of the active project (asks first when something runs in it).
    CloseTab(usize),
    CloseTabNow(usize),
    /// Closes these panes (a tab confirmed in a dialog: by its panes, not its place, which a
    /// switch of project could change meanwhile).
    ClosePanesNow(Vec<PaneId>),
    /// Moves a pane of the tab in view beside another one.
    MovePane {
        pane: PaneId,
        target: PaneId,
        dir: tc_term::layout::SplitDir,
        before: bool,
    },
    /// Takes a pane out of its split into a tab of its own.
    PaneToTab(PaneId),
    SplitRight,
    SplitDown,
    ClosePane,
    ClosePaneId(PaneId),
    FocusPane(PaneId),
    NextPane,
    PrevPane,
    /// Starts a preset in a new pane of the active project.
    RunPreset(String),
    /// From the Run Agent dialog: the preset, which session (a new one, its last, or one chosen),
    /// where (0 new tab, 1 split right, 2 split down).
    RunAgent(String, Launch, usize),
    /// Starts a preset although its program was not found on the PATH (an alias or a shell
    /// function the check can't see): as `RunAgent` with a place, else as `RunPreset`.
    RunPresetAnyway(String, Launch, Option<usize>, Option<String>),
    /// Types a command in the project's shell waiting at its prompt, or in a new tab (an
    /// agent's install command).
    TypeInTerminal(String),
    /// Run Agent → New worktree: makes a worktree of the project in view (called `name`,
    /// starting at the branch `base`) and starts the preset in it.
    NewWorktree { preset: String, name: String, base: String },
    /// Folds or unfolds a project's worktrees in the sidebar.
    ToggleWorktrees(ProjectId),
    /// Asks how to remove a worktree (and its branch).
    RemoveWorktree(ProjectId),
    /// Removes a worktree now: its folder, and its branch when true.
    RemoveWorktreeNow(ProjectId, bool),
    /// Asks before merging a worktree's branch into its base.
    MergeWorktree(ProjectId),
    /// Types the merge in a new tab of the worktree's project.
    MergeWorktreeNow(ProjectId),
    /// Commits uncommitted changes in a worktree (off the UI thread) and then merges it into its base.
    CommitAndMergeWorktree(ProjectId, String),
    /// Opens what a new worktree of a project gets.
    WorktreeSetup(ProjectId),
    SetWorktreeSetup(ProjectId, tc_core::model::WorktreeSetup),
    Resume(PaneId),
    /// Forgets the session a restored pane could resume (its banner's Dismiss).
    DismissResume(PaneId),
    RestartShell(PaneId),
    /// Start the shell of a pane restored asleep.
    WakePane(PaneId),
    /// Grid: only projects where something runs or waits (true), or all.
    /// Which projects the Grid shows.
    GridFilter(tc_core::model::GridFilter),
    /// Tiles across the Grid (0 = automatic).
    GridColumns(u32),
    /// The Grid tile that has the keyboard opens in Focus (⌘↵).
    OpenGridTile,
    /// A Grid tile shows this tab of its project (a chip of its strip clicked): kept until
    /// something new waits in another tab, or the Grid is left.
    GridShowTab(ProjectId, usize),
    /// An editor command served by a language server (definition, rename, completion…).
    Ide(crate::lsp::IdeCmd),
    /// Opens a file with the caret at a line and char column (both 0-based).
    GotoLocation(PathBuf, usize, usize),
    NextAttention,
    BackAttention,
    ToggleAttention,
    /// Takes a pane out of the attention queue without going to it.
    MarkSeen(PaneId),
    MarkAllSeen,
    ToggleSidebar,
    /// The sidebar narrow (project tiles only) or full.
    ToggleCompactSidebar,
    Palette,
    SwitchProject,
    PickPreset,
    SetView(View),
    ToggleGrid,
    Settings,
    Integrations,
    PlanInstall(AgentKind),
    ConfirmInstall,
    Uninstall(AgentKind),
    UninstallNow(AgentKind),
    OpenFile(PathBuf, Option<usize>),
    CloseFile(usize),
    /// Closes a project's file and drops its unsaved edits ("Don't Save").
    DiscardAndCloseFile(ProjectId, PathBuf),
    /// Saves a project's file, then closes it ("Save" when closing a file with unsaved edits).
    SaveAndCloseFile(ProjectId, PathBuf),
    SaveFile,
    ForceSave,
    ReloadFile,
    ToggleEditor,
    /// Shows or folds the Files section of the sidebar.
    ToggleFiles,
    /// Types a file's path (quoted, relative to the pane's folder when inside it) into the
    /// focused pane.
    InsertPath(PathBuf),
    /// Shows a file or folder in Finder.
    /// Opens a file (an HTML page, an SVG) in the default web browser.
    OpenInBrowser(PathBuf),
    RevealPath(PathBuf),
    /// Starts typing the name of a new file or folder in a folder of the file tree.
    NewFileIn(PathBuf),
    NewFolderIn(PathBuf),
    /// Starts renaming a file or folder in the tree.
    RenamePath(PathBuf),
    /// A file or folder was renamed: open files follow it.
    PathRenamed { old: PathBuf, new: PathBuf },
    DuplicatePath(PathBuf),
    /// Copies files/folders onto the OS clipboard so they can be pasted into folders or Finder.
    CopyFiles(Vec<PathBuf>),
    /// Pastes files/folders currently on the OS clipboard (e.g. copied from Finder) into this folder.
    PasteFilesInto(PathBuf),
    /// Asks, then moves a file or folder (or several selected items) to the Trash.
    TrashPath(PathBuf),
    TrashPathNow(PathBuf),
    TrashPaths(Vec<PathBuf>),
    TrashPathsNow(Vec<PathBuf>),
    /// A new terminal tab of the active project, in this folder.
    NewTabAt(PathBuf),
    QuickOpen,
    Find,
    Review,
    /// Opens Commit History & GitHub/GitLab integration in the Code stage.
    History,
    /// ⇧⌘C: the commit box of the project in view (Review's Changes, its message focused).
    Commit,
    /// The pull request of the branch in view, in the Browser panel.
    OpenPullRequest,
    /// Review of what an agent's finished run changed (its Review button, the waiting list):
    /// that run's changes when it made some, else those since the last commit.
    ReviewAgent,
    /// A new page of the user's in the project's Browser panel (at the project's local server,
    /// when it has one).
    NewBrowserTab,
    /// ⇧⌘T: the page closed last opens again (of the project in view, else of any).
    ReopenClosedPage,
    /// The Browser button and ⇧⌘B: shows or hides the project's Browser panel (a page made in
    /// it when it has none: `App::show_browser`).
    ShowBrowser,
    /// The terminals beside a project's Browser panel stay side by side (not stacked).
    PanelSideBySide(tc_core::ProjectId),
    /// ⌘L: the address field of the project's browser (shown, or made, first).
    BrowserAddress,
    /// ⌘R: the page in view loads again.
    BrowserReload,
    /// Opens a web address: in the project's browser pane (made beside the terminal or in a
    /// tab when it has none), or with ⌥ held (or the setting off) in the default browser.
    OpenWeb(String),
    /// Opens a file (an HTML page, an SVG) in the default web browser.
    OpenInDefaultBrowser(PathBuf),
    FindAll,
    RevealMatch(PaneId, tc_term::alacritty_terminal::term::search::Match),
    Copy,
    Paste,
    SelectAll,
    ClearScrollback,
    FontBigger,
    FontSmaller,
    FontReset,
    Quit,
    QuitNow,
    /// "Check for Updates…": looks now and shows the update dialog.
    CheckForUpdates,
    /// Settings → About (Windows: Help → About TernaryCode).
    About,
    /// The status bar's update item: shows the update dialog.
    ShowUpdate,
    /// Quits and installs the downloaded update (after asking when programs run).
    RestartToUpdate,
    RestartToUpdateNow,
    /// One-keystroke Focus / Zen Mode: hides peripheral panels so the Code Editor fills the window.
    ToggleZenMode,
    /// Shows or collapses the secondary drawer (Terminals, Agent Logs, Problems).
    ToggleDrawer,
    /// ⇧⌘E: the right panel fills the body, or goes back beside the terminals.
    TogglePanelExpanded,
    /// The project moves one place up (true) or down in the sidebar (⌘1–9 follow it).
    MoveProject(ProjectId, bool),
    /// The project in view moves up (true) or down (⌥⌘↑ / ⌥⌘↓).
    MoveActiveProject(bool),
    /// Docks the secondary drawer at the Bottom (editor 100% width) or Right (editor >=60% width).
    SetDrawerDock(tc_core::model::DrawerDock),
    /// Switches the active tab inside the secondary drawer.
    SetDrawerTab(tc_core::model::DrawerTab),
    /// Opens the dedicated Multi-Account Profiles & Failover view.
    OpenProfiles,
    /// Opens the unified All-Agent Usage & Status Monitor view.
    OpenMonitor,
    /// Opens the Phone Pairing & Mobile Companion view.
    OpenPhonePairing,
    /// Expands or collapses nested agent session history under a project in the sidebar tree.
    ToggleProjectSessions(ProjectId),
    /// Resumes a historical or focuses a live agent session directly from the project tree.
    ResumeProjectSession {
        project: ProjectId,
        agent: AgentKind,
        session_id: String,
        profile_id: Option<String>,
    },
    /// Starts a fresh agent session inside a project using the selected profile.
    NewProjectSession {
        project: ProjectId,
        profile_id: Option<String>,
    },
    /// Switches the active account profile bound to a pane (without forcing a restart unless paused).
    SetPaneProfile(PaneId, String),
    /// Switches a rate-limited or running pane to another profile and seamlessly resumes its task.
    ResumeWithProfile {
        pane: PaneId,
        profile_id: String,
    },
    /// Dismisses a pane's inline rate-limit recovery banner.
    DismissRateLimit(PaneId),
    /// Dismisses a pane's active notice banner (`NeedsInput`, `Done`, `Failed`, or `RateLimit`).
    DismissNotice(PaneId),
    /// Tests CLI & auth readiness for a configured profile.
    TestProfile(String),
    /// Opens an interactive login pane using a profile's isolated auth/config directory.
    LoginProfile(String),
    /// Clears a profile's rate-limit / cooldown timer manually.
    ClearProfileCooldown(String),
    /// Starts or stops the embedded Mobile Companion HTTP/SSE server.
    ToggleCompanionServer,
    /// Enables or disables zero-setup Cloudflare Relay remote access.
    ToggleCompanionRelay,
    /// Generates a fresh pairing QR token (existing paired devices remain authorised).
    RegeneratePairingToken,
    /// Immediately revokes a paired phone/device token.
    RevokeDevice(String),
}
