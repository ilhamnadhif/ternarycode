//! Everything the user can ask for, from the menu, a shortcut, the palette or a button.

use std::path::PathBuf;

use tc_core::agent::AgentKind;
use tc_core::model::View;
use tc_core::{PaneId, ProjectId};

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
    MoveProject { from: usize, to: usize },
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
    /// Starts the project's default preset (or asks which one).
    RunAgent,
    Resume(PaneId),
    RestartShell(PaneId),
    /// Start the shell of a pane restored asleep.
    WakePane(PaneId),
    /// Grid: only projects where something runs or waits (true), or all.
    GridActiveOnly(bool),
    /// An editor command served by a language server (definition, rename, completion…).
    Ide(crate::lsp::IdeCmd),
    /// Opens a file with the caret at a line and char column (both 0-based).
    GotoLocation(PathBuf, usize, usize),
    NextAttention,
    BackAttention,
    ToggleAttention,
    ToggleSidebar,
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
    OpenFile(PathBuf, Option<usize>),
    CloseFile(usize),
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
    RevealPath(PathBuf),
    /// Starts typing the name of a new file or folder in a folder of the file tree.
    NewFileIn(PathBuf),
    NewFolderIn(PathBuf),
    /// Starts renaming a file or folder in the tree.
    RenamePath(PathBuf),
    /// A file or folder was renamed: open files follow it.
    PathRenamed { old: PathBuf, new: PathBuf },
    DuplicatePath(PathBuf),
    /// Asks, then moves a file or folder to the Trash.
    TrashPath(PathBuf),
    TrashPathNow(PathBuf),
    /// A new terminal tab of the active project, in this folder.
    NewTabAt(PathBuf),
    QuickOpen,
    Find,
    Review,
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
    About,
    /// "Check for Updates…": looks now and shows the update dialog.
    CheckForUpdates,
    /// The status bar's update item: shows the update dialog.
    ShowUpdate,
    /// Quits and installs the downloaded update (after asking when programs run).
    RestartToUpdate,
    RestartToUpdateNow,
}
