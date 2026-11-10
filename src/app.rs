//! The application: projects and panes, the messages from PTYs and hooks, statuses and the
//! attention queue, notifications, saving, and every action.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use eframe::egui::{self, Ui};
use tc_agent::integrate::{self, Hooks, InstallPlan, InstallRecord};
use tc_agent::server::HookServer;
use tc_core::agent::{AgentKind, Preset};
use tc_core::attention::{self, Entry, Jumps};
use tc_core::model::{Integration, PaneSave, Settings, View, Workspace, WORKSPACE_VERSION};
use tc_core::status::Status;
use tc_core::{PaneId, ProjectId};
use tc_term::layout::{Layout, SplitDir};
use tc_term::scheme;

use crate::actions::Action;
use crate::i18n::{t, tf};
use crate::pane::{Pane, SpawnCtx, UiMsg};
use crate::project::{GitInfo, Project, Tab};
use crate::{paths, platform, ui};

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

pub fn mark_process_start() {
    let _ = PROCESS_START.set(Instant::now());
}

pub fn since_start() -> Option<Duration> {
    PROCESS_START.get().map(Instant::elapsed)
}

/// Extra messages to the UI thread besides the panes' own.
pub enum AppMsg {
    Git(ProjectId, Option<GitInfo>, crate::project::Kind),
    /// The files of a project, for quick open.
    ProjectFiles(ProjectId, Vec<PathBuf>),
    /// Files changed under a project folder.
    Files(ProjectId, Vec<PathBuf>),
    Review(crate::ui::review::ReviewMsg),
    /// The working tree at the start of an agent run.
    Snapshot(PaneId, AgentKind, Result<tc_git::Snapshot, String>),
    /// A message of a language server, and which start of it sent it (a restarted server's
    /// old process still reports its exit).
    Lsp(crate::lsp::ServerKey, u64, tc_lsp::client::Incoming),
    /// The user's PATH (read from their login shell), for finding language servers.
    LspPath(std::ffi::OsString),
}

/// A line of a confirmation's list: a program that would be stopped, or a file whose changes
/// would be lost.
pub struct Busy {
    pub text: String,
    /// An editor file with changes not saved.
    pub file: bool,
}

pub enum Dialog {
    AddProject { path: String, name: String },
    ConfirmQuit(Vec<Busy>),
    /// Restarting to install an update would stop these programs.
    ConfirmUpdate(Vec<Busy>),
    ConfirmRemove(ProjectId, Vec<Busy>),
    /// Moving a file or folder to the Trash.
    ConfirmTrash(PathBuf),
    /// Closing a tab (index in the active project) would stop these programs.
    ConfirmCloseTab(Vec<PaneId>, Vec<Busy>),
    /// ⌘W on a pane where a program runs.
    ConfirmClosePane(PaneId, String),
    Rename(ProjectId, String),
    RenamePane(PaneId, String),
    Settings(ui::settings::SettingsPage),
    Install(Box<InstallPlan>, String),
    Message(String, String),
}

/// The hook program agents run: the one next to the app, except in an AppImage, which is
/// mounted only while it runs. Hooks written into an agent's settings must outlive that, so it
/// is copied into the data folder (again whenever it changes).
fn hook_program() -> Hooks {
    let beside = Hooks::beside_current_exe();
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_some() {
        let copy = paths::data_dir().join("bin").join("ternarycode-hook");
        let current = std::fs::read(&beside.bin).ok();
        let fresh = current.is_some() && std::fs::read(&copy).ok() == current;
        let copied = fresh
            || current.is_some_and(|bytes| {
                use std::os::unix::fs::PermissionsExt;
                let tmp = copy.with_extension(format!("new-{}", std::process::id()));
                copy.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok())
                    && std::fs::write(&tmp, bytes).is_ok()
                    && std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).is_ok()
                    && std::fs::rename(&tmp, &copy).is_ok()
            });
        if copied {
            return Hooks { bin: copy };
        }
    }
    beside
}

/// Set when the language changes (the native menu bar is then built again).
static MENU_STALE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub struct Palette {
    /// Files of the active project (quick open).
    pub files: Vec<PathBuf>,
    /// Search across panes: the query searched and its results.
    pub searched: String,
    pub results: Vec<(PaneId, tc_term::alacritty_terminal::term::search::Match, String)>,
    pub mode: PaletteMode,
    pub query: String,
    pub selected: usize,
    pub opened: bool,
    /// Locations mode: its title and places (references, symbols, problems…).
    pub title: String,
    pub locations: Vec<crate::lsp::LocItem>,
    /// Opened while the editor had the keyboard: it gets it back on closing.
    pub from_editor: bool,
    /// The project's files are still being listed (off the UI thread).
    pub loading: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteMode {
    All,
    Projects,
    Presets,
    Files,
    Search,
    Locations,
}

/// The find bar of one pane (⌘F).
pub struct PaneSearch {
    pub pane: PaneId,
    pub query: String,
    pub current: Option<tc_term::alacritty_terminal::term::search::Match>,
    pub opened: bool,
    pub missing: bool,
}

pub struct App {
    pub rt: tokio::runtime::Runtime,
    /// The UI's context (to wake it from other threads).
    pub ctx: egui::Context,
    pub tx: Sender<UiMsg>,
    rx: Receiver<UiMsg>,
    pub app_tx: Sender<AppMsg>,
    app_rx: Receiver<AppMsg>,
    pub hooks: Option<HookServer>,
    pub hook_bin: Hooks,
    /// Variables every pane gets (hook flags).
    pub base_env: Vec<(String, String)>,
    /// Whether Codex may get per-launch hooks (the user has none of their own).
    pub codex_hooks: bool,
    /// The per-launch settings file was written (its variables are in `base_env`).
    per_launch: bool,
    pub settings: Settings,
    pub projects: Vec<Project>,
    pub active: Option<ProjectId>,
    pub panes: HashMap<PaneId, Pane>,
    next_id: u64,
    pub view: View,
    pub jumps: Jumps,
    pub dialog: Option<Dialog>,
    pub palette: Option<Palette>,
    pub attention_open: bool,
    /// Move keyboard focus to this pane's terminal when it is drawn next.
    pub focus_request: Option<PaneId>,
    pub actions: Vec<Action>,
    save_due: Option<Instant>,
    saver: std::sync::Arc<paths::Saver>,
    /// The workspace as last written (to skip writing it unchanged).
    saved_workspace: Vec<u8>,
    settings_dirty: bool,
    pub window_focused: bool,
    /// The ask (see `Tracker::ask`) each pane was last notified for, or seen at.
    notified: HashMap<PaneId, u64>,
    badge: usize,
    #[cfg(target_os = "macos")]
    menu: Option<(ui::menu::AppMenu, Receiver<muda::MenuEvent>)>,
    instance_rx: Option<Receiver<Vec<String>>>,
    pub allow_close: bool,
    pub toast: Option<(String, Instant)>,
    pub updater: crate::ui::update_ui::Updater,
    pub mem_bytes: (Option<u64>, Option<Instant>),
    pub first_frame: bool,
    pub sidebar_drag: Option<usize>,
    /// Terminal tab being dragged (index in the active project).
    pub tab_drag: Option<usize>,
    /// Pane being dragged by its header.
    pub pane_drag: Option<PaneId>,
    /// The editor (its text or find field) had the keyboard in the last frame: ⌘F searches the
    /// file instead of the terminal.
    pub editor_keyboard: bool,
    /// Set while drawing when the editor has the keyboard.
    pub editor_seen: bool,
    /// The panes area of the Focus view and the terminal cell size, as last drawn.
    pub last_body: Option<egui::Rect>,
    /// The Focus view's area below the header, editor included (the same for every project).
    pub last_full_body: Option<egui::Rect>,
    /// The pointer is over a terminal tab (or one is being dragged) this frame.
    pub tabs_hot: bool,
    window_movable: Option<bool>,
    pub cell: Option<egui::Vec2>,
    /// A name typed in the Add Project dialog, for the project about to be added.
    pub pending_rename: Option<(PathBuf, String)>,
    shells_ready_logged: bool,
    /// Frames in a row whose logic, or drawing, panicked (each retry waits longer).
    logic_panics: u32,
    ui_panics: u32,
    /// The pane taking the keyboard in the Grid.
    pub grid_focus: Option<PaneId>,
    /// Columns of the Grid (0 = automatic).
    pub grid_columns: u32,
    /// The Grid shows only projects where something runs or waits.
    pub grid_active_only: bool,
    /// Language servers for the editor (IDE features).
    pub lsp: crate::lsp::Lsp,
    /// A buffer change waits to go to its server: look again then.
    pub lsp_repaint_after: Option<Duration>,
    /// When the editor was last on screen (its colouring memory goes after a while without).
    pub editor_seen_at: Instant,
    /// That memory was let go since the editor was last on screen (once per time away).
    editor_released: bool,
    /// The scheme whose colours the terminals answer OSC 4/10/11 queries with.
    term_scheme: &'static str,
    /// The scrollback setting the panes were last given.
    scrollback_applied: usize,
    pub editors: HashMap<ProjectId, crate::editor::Editor>,
    /// The file tree of each project shown so far.
    pub explorers: HashMap<ProjectId, crate::explorer::Explorer>,
    pub pane_search: Option<PaneSearch>,
    pub review: Option<crate::ui::review::Review>,
    /// Where each pane was drawn last frame (for overlays).
    pub pane_rects: HashMap<PaneId, egui::Rect>,
    /// How each pane is drawn this frame (see [`crate::pane::Shown`]).
    pub shown_now: HashMap<PaneId, crate::pane::Shown>,
    watchers: crate::watch::Watchers,
    git_due: HashMap<ProjectId, Instant>,
    git_running: HashSet<ProjectId>,
    git_again: HashSet<ProjectId>,
    conflict_warned: HashMap<ProjectId, Instant>,
    budget_checked: Instant,
    /// Clicked notifications (pane ids), when native notifications are available.
    clicked_rx: Option<Receiver<PaneId>>,
    #[cfg(debug_assertions)]
    pub devtest: crate::devtest::Script,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        open: Vec<PathBuf>,
        inbox: Option<crate::instance::Inbox>,
    ) -> Self {
        let ctx = &cc.egui_ctx;
        let mut settings: Settings = tc_core::persist::load_json(&paths::settings_file());
        if settings.migrate() {
            let _ = tc_core::persist::save_json(&paths::settings_file(), &settings);
        }
        crate::i18n::set_language(settings.language);
        tc_term::set_translator(crate::i18n::t);
        apply_fonts(ctx, &settings);
        ui::style::apply(ctx, settings.theme);

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ternarycode-io")
            .enable_all()
            .build()
            .expect("tokio runtime");
        let (tx, rx) = channel();
        let (app_tx, app_rx) = channel();

        // The hook socket.
        let hook_bin = hook_program();
        let hooks = {
            let tx = tx.clone();
            let ctx = ctx.clone();
            HookServer::start(
                rt.handle(),
                tc_agent::server::socket_path(paths::data_dir()),
                std::sync::Arc::new(move |m| {
                    let _ = tx.send(UiMsg::Hook(m));
                    ctx.request_repaint();
                }),
            )
            .ok()
        };
        let pane_env = hook_bin.pane_env(&paths::integrations_dir());
        let per_launch = pane_env.is_ok();
        let mut base_env = pane_env.unwrap_or_default();
        // Shell integration (OSC 133): scripts in the data folder, activated through the
        // environment only; the user's dotfiles load as usual.
        if settings.shell_integration {
            let dir = paths::shell_dir();
            if tc_agent::shell::install(&dir).is_ok() {
                #[cfg(windows)]
                let shell = tc_term::pty::default_shell().to_string();
                #[cfg(not(windows))]
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
                base_env.extend(tc_agent::shell::env(&dir, &shell, |k| std::env::var(k).ok()));
            }
        }
        // `ternarycode` in every pane (the app's own folder on PATH); and in other terminals
        // once it is linked onto the user's PATH (off the UI thread: it asks the login shell).
        if let Some(path) = crate::cli::pane_path() {
            base_env.retain(|(k, _)| k != "PATH");
            base_env.push(("PATH".into(), path.to_string_lossy().into_owned()));
        }
        if !cfg!(test) {
            let _ = std::thread::Builder::new().name("ternarycode-cli".into()).spawn(crate::cli::ensure_installed);
        }
        let codex_config = paths::home_dir().join(".codex/config.toml");
        let codex_hooks = !std::fs::read_to_string(codex_config)
            .is_ok_and(|s| integrate::codex_has_own_hooks(&s));

        #[cfg(target_os = "macos")]
        let menu = Some((ui::menu::AppMenu::install(), ui::menu::event_channel(ctx)));
        crate::i18n::on_language_change(|| MENU_STALE.store(true, std::sync::atomic::Ordering::Relaxed));

        let mut app = Self {
            rt,
            ctx: ctx.clone(),
            tx,
            rx,
            app_tx,
            app_rx,
            hooks,
            hook_bin,
            base_env,
            codex_hooks,
            per_launch,
            scrollback_applied: settings.scrollback,
            settings,
            projects: Vec::new(),
            active: None,
            panes: HashMap::new(),
            next_id: 1,
            view: View::Focus,
            jumps: Jumps::default(),
            dialog: None,
            palette: None,
            attention_open: false,
            focus_request: None,
            actions: Vec::new(),
            save_due: None,
            saver: Default::default(),
            saved_workspace: Vec::new(),
            settings_dirty: false,
            window_focused: true,
            notified: HashMap::new(),
            badge: 0,
            #[cfg(target_os = "macos")]
            menu,
            instance_rx: inbox.map(|i| i.attach(ctx)),
            allow_close: false,
            toast: None,
            updater: Default::default(),
            mem_bytes: (None, None),
            first_frame: true,
            sidebar_drag: None,
            tab_drag: None,
            pane_drag: None,
            editor_keyboard: false,
            editor_seen: false,
            last_body: None,
            last_full_body: None,
            tabs_hot: false,
            window_movable: None,
            cell: None,
            pending_rename: None,
            shells_ready_logged: false,
            logic_panics: 0,
            ui_panics: 0,
            grid_focus: None,
            grid_columns: 0,
            grid_active_only: false,
            lsp: crate::lsp::Lsp::default(),
            lsp_repaint_after: None,
            editor_seen_at: Instant::now(),
            editor_released: false,
            term_scheme: "",
            editors: HashMap::new(),
            explorers: HashMap::new(),
            pane_search: None,
            review: None,
            pane_rects: HashMap::new(),
            shown_now: HashMap::new(),
            watchers: crate::watch::Watchers::default(),
            git_due: HashMap::new(),
            git_running: HashSet::new(),
            git_again: HashSet::new(),
            conflict_warned: HashMap::new(),
            budget_checked: Instant::now(),
            clicked_rx: None,
            #[cfg(debug_assertions)]
            devtest: crate::devtest::Script::from_env(),
        };
        #[cfg(target_os = "macos")]
        {
            let (tx, rx) = channel();
            if crate::notify_native::init(tx, ctx) {
                app.clicked_rx = Some(rx);
            }
        }
        app.restore(ctx);
        app.lsp_find_path();
        for path in open {
            app.actions.push(Action::OpenTarget(path.to_string_lossy().into_owned()));
        }
        app
    }

    // ----- spawning ---------------------------------------------------------------------------

    /// What a shell starts with; `pane` registers it with the hook server (none for a pane that
    /// is created asleep: it registers when it wakes).
    fn spawn_ctx<'a>(&'a self, ctx: &'a egui::Context, pane: Option<PaneId>) -> SpawnCtx<'a> {
        let dark = ctx.global_style().visuals.dark_mode;
        let scheme = self.scheme(dark);
        SpawnCtx {
            rt: self.rt.handle(),
            tx: &self.tx,
            ctx,
            env: &self.base_env,
            hook: pane.and_then(|pane| {
                self.hooks.as_ref().map(|h| {
                    (
                        h.path().to_string_lossy().into_owned(),
                        h.register(pane),
                    )
                })
            }),
            scrollback: self.settings.scrollback,
            palette: scheme.into(),
            cursor: tc_term::prefs::CursorStyle::Block,
            blink: self.settings.cursor_blink,
            long_command: Duration::from_secs(self.settings.long_command_secs.max(1)),
        }
    }

    pub fn scheme(&self, dark: bool) -> &'static scheme::Scheme {
        let name = if dark {
            &self.settings.scheme_dark
        } else {
            &self.settings.scheme_light
        };
        scheme::find(name, dark)
    }

    fn new_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// The grid pane `pane` gets when `layout` is drawn in the Focus view.
    fn grid_in(&self, layout: &Layout, pane: PaneId) -> tc_term::core::GridSize {
        let body = self.last_body.unwrap_or_else(|| {
            // Before the first frame: the saved window size less the sidebar, bars and header.
            let [_, _, w, h] = self.settings.window.unwrap_or([0.0, 0.0, 1280.0, 800.0]);
            let left = if self.settings.sidebar_hidden { 0.0 } else { self.settings.sidebar_width };
            egui::Rect::from_min_max(
                egui::pos2(left, ui::focus::HEADER_H),
                egui::pos2(w.max(left + 200.0), (h - ui::view::STATUS_H).max(ui::focus::HEADER_H + 100.0)),
            )
        });
        let (rects, _) = layout.arrange(body);
        let single = rects.len() == 1;
        let rect = rects.iter().find(|(id, _)| *id == pane).map_or(body, |(_, r)| *r);
        let size = self.settings.font_size;
        let cell = self.cell.unwrap_or(egui::vec2(size * 0.6, size * 1.25));
        tc_term::widget::grid_for(ui::focus::term_rect(rect, single), cell)
    }

    /// Where the editor panel starts in the Focus view's `body`, when the project shows it.
    pub fn editor_split(&self, project: ProjectId, body: egui::Rect) -> Option<f32> {
        let e = self.editors.get(&project).filter(|e| e.visible)?;
        let ratio = if e.width > 0.0 { e.width } else { 0.45 }.clamp(0.2, 0.8);
        Some((body.right() - body.width() * ratio).round())
    }

    /// Gives panes of a project the grid they have in the Focus view (that of its tab in view,
    /// or of all its tabs): after panes were closed, split or moved elsewhere (Ctrl+D in a Grid
    /// tile), and for the panes a Grid tile shows (restored with a size guessed from the saved
    /// window, or last seen in a smaller window). The sizes are worked out as the Focus view
    /// does, so opening the project resizes nothing more. In the Focus view the terminal widget
    /// does this itself.
    pub fn fit_to_focus(&mut self, ctx: &egui::Context, project: ProjectId, tab_in_view: bool) {
        if self.view == View::Focus && self.active == Some(project) {
            return;
        }
        // Not shown in the Focus view since the app started: nothing to measure against.
        let Some(full) = self.last_full_body else { return };
        let body = match self.editor_split(project, full) {
            Some(x) => egui::Rect::from_min_max(full.min, egui::pos2(x, full.bottom())),
            None => full,
        };
        let cell = tc_term::widget::cell_size(ctx, self.settings.font_size);
        let Some(p) = self.project(project) else { return };
        let mut sizes = Vec::new();
        let tabs: Vec<&crate::project::Tab> = if tab_in_view { p.tab().into_iter().collect() } else { p.tabs.iter().collect() };
        for tab in tabs {
            let (rects, _) = tab.layout.arrange(body);
            let single = rects.len() == 1;
            for (id, r) in rects {
                sizes.push((id, tc_term::widget::grid_for(ui::focus::term_rect(r, single), cell)));
            }
        }
        self.resize_panes(ctx, sizes);
    }

    /// Gives panes these grid sizes (terminal and PTY); a pane already that size is left alone.
    pub fn resize_panes(&mut self, ctx: &egui::Context, sizes: Vec<(PaneId, tc_term::core::GridSize)>) {
        let cell = tc_term::widget::cell_size(ctx, self.settings.font_size);
        let ppp = ctx.pixels_per_point();
        let cell_px = ((cell.x * ppp).round(), (cell.y * ppp).round());
        for (id, size) in sizes {
            let Some(pane) = self.panes.get_mut(&id) else { continue };
            let now = {
                use tc_term::alacritty_terminal::grid::Dimensions;
                let term = pane.shared.term.lock();
                tc_term::core::GridSize { cols: term.columns(), rows: term.screen_lines() }
            };
            if now != size {
                tc_term::core::resize(&pane.shared, size, cell_px.0, cell_px.1);
                pane.resize_pty(size, size.cols as u32 * cell_px.0 as u32, size.rows as u32 * cell_px.1 as u32);
            }
        }
    }

    /// Back from the Grid: every project's panes take their Focus size again, so switching
    /// projects in the Focus view never resizes a terminal.
    fn fit_all_to_focus(&mut self, ctx: &egui::Context) {
        let ids: Vec<ProjectId> = self.projects.iter().map(|p| p.id).collect();
        for id in ids {
            self.fit_to_focus(ctx, id, false);
        }
    }

    /// Every way into the Focus view: from the Grid, the panes get their Focus size back (Gate
    /// C) and the Grid's keyboard tile is forgotten. Set `active` first (its panes are sized as
    /// they are drawn).
    fn show_focus(&mut self) {
        if std::mem::replace(&mut self.view, View::Focus) == View::Grid {
            let ctx = self.ctx.clone();
            self.fit_all_to_focus(&ctx);
        }
        self.grid_focus = None;
    }

    fn new_pane(&mut self, ctx: &egui::Context, project: ProjectId, cwd: PathBuf, size: tc_term::core::GridSize) -> PaneId {
        let id = self.new_id();
        let pane = Pane::new(id, project, cwd, size, &self.spawn_ctx(ctx, Some(id)), true);
        self.panes.insert(id, pane);
        id
    }

    /// Starts the shell of a pane restored asleep (shown, or about to be typed into).
    pub fn wake(&mut self, ctx: &egui::Context, id: PaneId) {
        if self.panes.get(&id).is_some_and(Pane::dormant) {
            self.restart_shell(ctx, id);
        }
    }

    // ----- workspace --------------------------------------------------------------------------

    fn restore(&mut self, ctx: &egui::Context) {
        let ws: Workspace = tc_core::persist::load_json(&paths::workspace_file());
        self.next_id = ws.next_id.max(1);
        self.view = ws.view;
        self.grid_columns = ws.grid_columns;
        self.grid_active_only = ws.grid_active_only;
        for p in &ws.projects {
            self.next_id = self.next_id.max(p.id + 1);
            for pane in &p.panes {
                self.next_id = self.next_id.max(pane.id + 1);
            }
        }
        let presets = self.settings.presets();
        for saved in ws.projects {
            let mut project = Project {
                id: saved.id,
                name: saved.name.clone(),
                path: saved.path.clone(),
                color: saved.color,
                preset: saved.preset.clone(),
                tabs: Vec::new(),
                active_tab: 0,
                git: None,
                kind: Default::default(),
            };
            let pane_info: HashMap<PaneId, PaneSave> =
                saved.panes.iter().map(|p| (p.id, p.clone())).collect();
            for tab in &saved.tabs {
                let layout = crate::project::from_saved(&tab.layout);
                for id in layout.panes() {
                    let info = pane_info.get(&id).cloned().unwrap_or_default();
                    let cwd = info.cwd.clone().unwrap_or_else(|| saved.path.clone());
                    let size = self.grid_in(&layout, id);
                    // Asleep: a shell starts when the pane is first shown (or typed into).
                    let mut pane = Pane::new(id, saved.id, cwd, size, &self.spawn_ctx(ctx, None), false);
                    pane.title = info.title.clone();
                    pane.preset = info.preset.clone();
                    if let Some(agent) = info.agent {
                        let preset = info
                            .preset
                            .as_ref()
                            .and_then(|pid| presets.iter().find(|p| &p.id == pid))
                            .or_else(|| presets.iter().find(|p| p.agent == Some(agent)));
                        if let Some(command) =
                            preset.and_then(|p| p.resume.clone().or(Some(p.command.clone())))
                        {
                            pane.resume = Some((agent, command));
                            pane.last_agent = Some(agent);
                        }
                    }
                    self.panes.insert(id, pane);
                }
                let focused = if layout.contains(tab.focused) {
                    tab.focused
                } else {
                    layout.panes()[0]
                };
                project.tabs.push(Tab {
                    layout,
                    focused,
                    title: tab.title.clone(),
                });
            }
            if project.tabs.is_empty() {
                let id = self.new_id();
                let size = self.grid_in(&Layout::Pane(id), id);
                let pane = Pane::new(id, project.id, project.path.clone(), size, &self.spawn_ctx(ctx, None), false);
                self.panes.insert(id, pane);
                project.tabs.push(Tab {
                    layout: Layout::Pane(id),
                    focused: id,
                    title: None,
                });
            }
            project.active_tab = saved.active_tab.min(project.tabs.len() - 1);
            if saved.editor_width > 0.0 {
                self.editors.insert(
                    project.id,
                    crate::editor::Editor {
                        width: saved.editor_width.clamp(0.2, 0.8),
                        ..Default::default()
                    },
                );
            }
            self.watchers
                .watch(project.id, project.path.clone(), self.app_tx.clone(), ctx.clone());
            let id = project.id;
            self.projects.push(project);
            self.refresh_git(id);
        }
        self.active = ws
            .active
            .filter(|a| self.projects.iter().any(|p| p.id == *a))
            .or_else(|| self.projects.first().map(|p| p.id));
        self.focus_request = self.current_pane();
    }

    pub fn workspace(&self) -> Workspace {
        Workspace {
            version: WORKSPACE_VERSION,
            projects: self
                .projects
                .iter()
                .map(|p| {
                    let panes = p
                        .panes()
                        .into_iter()
                        .filter_map(|id| self.panes.get(&id))
                        .map(|pane| PaneSave {
                            id: pane.id,
                            cwd: Some(pane.cwd.clone()),
                            title: pane.title.clone(),
                            agent: pane.last_agent,
                            preset: pane.preset.clone(),
                        })
                        .collect();
                    let mut saved = p.save(panes);
                    saved.editor_width = self.editors.get(&p.id).map_or(0.0, |e| e.width);
                    saved
                })
                .collect(),
            active: self.active,
            view: self.view,
            grid_columns: self.grid_columns,
            grid_active_only: self.grid_active_only,
            next_id: self.next_id,
        }
    }

    pub fn mark_dirty(&mut self) {
        if self.save_due.is_none() {
            self.save_due = Some(Instant::now() + Duration::from_millis(800));
        }
    }

    /// Saves the workspace (and settings, when changed) off the UI thread; an unchanged
    /// workspace is not written again.
    pub fn save_now(&mut self) {
        self.save_due = None;
        if let Ok(data) = serde_json::to_vec_pretty(&self.workspace())
            && data != self.saved_workspace
        {
            self.saver.queue(&self.rt, paths::workspace_file(), data.clone());
            self.saved_workspace = data;
        }
        if self.settings_dirty {
            self.settings_dirty = false;
            let long = Duration::from_secs(self.settings.long_command_secs.max(1));
            for p in self.panes.values_mut() {
                p.tracker.long_command = long;
            }
            // A new scrollback length is every pane's (the budget trims those out of view
            // again when it must).
            if self.settings.scrollback != self.scrollback_applied {
                let (lines, blink) = (self.settings.scrollback, self.settings.cursor_blink);
                self.scrollback_applied = lines;
                for p in self.panes.values_mut() {
                    p.scrollback_limit = lines;
                    tc_term::core::limit_scrollback(&p.shared, lines, tc_term::prefs::CursorStyle::Block, blink);
                }
            }
            if let Ok(data) = serde_json::to_vec_pretty(&self.settings) {
                self.saver.queue(&self.rt, paths::settings_file(), data);
            }
        }
    }

    /// Keeps the window's place and size for the next start (not while it is full screen,
    /// zoomed or minimized: the size to come back to is the ordinary one).
    fn remember_window(&mut self, ctx: &egui::Context) {
        let (outer, inner, special) = ctx.input(|i| {
            let v = i.viewport();
            let special = v.fullscreen.unwrap_or(false) || v.maximized.unwrap_or(false) || v.minimized.unwrap_or(false);
            (v.outer_rect, v.inner_rect, special)
        });
        let (Some(outer), Some(inner)) = (outer, inner) else { return };
        if special || inner.width() < 200.0 || inner.height() < 150.0 {
            return;
        }
        let now = [outer.min.x.round(), outer.min.y.round(), inner.width().round(), inner.height().round()];
        if self.settings.window != Some(now) {
            self.settings.window = Some(now);
            self.settings_changed();
        }
    }

    /// Saves now and waits for the writes (quitting).
    fn save_and_wait(&mut self) {
        self.save_now();
        self.saver.flush();
    }

    pub fn settings_changed(&mut self) {
        self.settings_dirty = true;
        self.mark_dirty();
    }

    /// Refreshes a project's git summary off the UI thread. One `git status` per project runs
    /// at a time: asking while one runs repeats it once when it is done (big repositories can
    /// take longer than the changes that ask for it).
    pub fn refresh_git(&mut self, id: ProjectId) {
        if self.git_running.contains(&id) {
            self.git_again.insert(id);
            return;
        }
        let Some(path) = self.project(id).map(|p| p.path.clone()) else {
            return;
        };
        self.git_running.insert(id);
        let tx = self.app_tx.clone();
        self.rt.spawn_blocking(move || {
            let kind = crate::project::Kind::detect(&path);
            let _ = tx.send(AppMsg::Git(id, crate::project::git_info(&path), kind));
        });
    }

    // ----- lookups ----------------------------------------------------------------------------

    pub fn project(&self, id: ProjectId) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    fn project_mut(&mut self, id: ProjectId) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| p.id == id)
    }

    pub fn active_project(&self) -> Option<&Project> {
        self.active.and_then(|id| self.project(id))
    }

    pub fn current_pane(&self) -> Option<PaneId> {
        self.active_project().and_then(Project::current_pane)
    }

    /// The pane commands act on: in the Grid, the tile being typed into; else the focused pane
    /// of the open project.
    pub fn target_pane(&self) -> Option<PaneId> {
        match self.view {
            View::Grid => self.grid_focus,
            View::Focus => self.current_pane(),
        }
    }

    /// The pane "this pane" commands act on (split, find, where ⌘J starts from): the target
    /// pane, else (in the Grid with no tile typed into) the open project's focused pane.
    fn command_pane(&self) -> Option<PaneId> {
        self.target_pane().or_else(|| self.current_pane())
    }

    /// The project "this project" commands act on (new tab, agents): the command pane's (a
    /// Grid tile's project is not made the open one).
    fn command_project(&self) -> Option<ProjectId> {
        self.command_pane().and_then(|id| self.panes.get(&id)).map(|p| p.project).or(self.active)
    }

    /// A widget other than a terminal pane has the keyboard (a text field, the editor).
    fn text_field_focused(&self, ctx: &egui::Context) -> bool {
        match ctx.memory(|m| m.focused()) {
            Some(id) => !self.panes.keys().any(|p| tc_term::widget::pane_widget_id(*p) == id),
            None => false,
        }
    }

    /// The editor's text has the keyboard.
    fn editor_has_keyboard(&self, ctx: &egui::Context) -> bool {
        let Some(path) = self.active.and_then(|id| self.editors.get(&id)).filter(|e| e.visible).and_then(|e| e.file()).map(|f| f.path.clone()) else {
            return false;
        };
        ctx.memory(|m| m.focused()) == Some(egui::Id::new(("editor-text", path)))
    }

    /// The pane the user is looking at: the focused pane of the visible project, while the
    /// window is active and no dialog covers it.
    fn looking_at(&self) -> Option<PaneId> {
        if !self.window_focused || self.dialog.is_some() || self.review.is_some() {
            return None;
        }
        match self.view {
            View::Focus => self.current_pane(),
            View::Grid => self.grid_focus,
        }
    }

    pub fn attention_queue(&self) -> Vec<Entry> {
        let mut q: Vec<Entry> = self
            .panes
            .values()
            .filter(|p| p.tracker.wants_attention())
            .map(|p| Entry {
                pane: p.id,
                status: p.tracker.status,
                since: p.tracker.since,
            })
            .collect();
        attention::order(&mut q);
        q
    }

    /// The most urgent status among a project's panes: in the attention queue's order (the
    /// sidebar and ⌘J agree), then a pane at work.
    pub fn project_status(&self, project: &Project) -> Option<(Status, bool)> {
        let rank = |s: Status| match (s.priority(), s) {
            (Some(p), _) => Some((0, p)),
            (None, Status::Running) => Some((1, 0)),
            (None, Status::Unknown) => Some((1, 1)),
            _ => None,
        };
        project
            .panes()
            .iter()
            .filter_map(|id| self.panes.get(id))
            .map(|p| (p.tracker.status, p.tracker.wants_attention()))
            .filter(|(s, _)| rank(*s).is_some())
            .min_by_key(|(s, attention)| (!attention, rank(*s)))
    }

    // ----- focus ------------------------------------------------------------------------------

    pub fn focus_pane(&mut self, pane: PaneId) {
        let Some(project) = self.panes.get(&pane).map(|p| p.project) else {
            return;
        };
        if let Some(p) = self.project_mut(project)
            && let Some(tab) = p.tab_of(pane)
        {
            p.active_tab = tab;
            p.tabs[tab].focused = pane;
        }
        self.active = Some(project);
        self.show_focus();
        self.focus_request = Some(pane);
        self.mark_dirty();
    }

    // ----- messages ---------------------------------------------------------------------------

    fn process_messages(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let mut git_refresh = Vec::new();
        let mut ended = Vec::new();
        // The agent a pane would offer to resume changed: saved soon, so a crash keeps it.
        let mut resume_changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                UiMsg::Pane(id, generation, event) => {
                    // An earlier shell of the pane is not its terminal's any more.
                    if let Some(pane) = self.panes.get_mut(&id).filter(|p| p.generation == generation) {
                        let was_program = !matches!(pane.tracker.fg, tc_core::status::Foreground::Shell);
                        let exited = matches!(event, tc_term::pty::SessionEvent::Exited(_));
                        pane.on_session_event(event, now);
                        if was_program && pane.exit.is_some() {
                            git_refresh.push(pane.project);
                        }
                        if exited && pane.ended_by_user(now) {
                            ended.push(id);
                        }
                    }
                }
                UiMsg::Hook(msg) => {
                    if let Some(pane) = self.panes.get_mut(&msg.pane) {
                        let turn_done = matches!(msg.event.as_str(), "Stop" | "AfterAgent" | "session.idle")
                            || msg.kind.as_deref() == Some("agent-turn-complete");
                        let agent = pane.last_agent;
                        pane.on_hook(&msg, now);
                        if turn_done {
                            git_refresh.push(pane.project);
                        }
                        resume_changed |= pane.last_agent != agent;
                    }
                }
            }
        }
        while let Ok(msg) = self.app_rx.try_recv() {
            match msg {
                AppMsg::ProjectFiles(id, files) => {
                    if self.active == Some(id)
                        && let Some(pal) = self.palette.as_mut().filter(|p| p.mode == PaletteMode::Files && p.loading)
                    {
                        pal.files = files;
                        pal.loading = false;
                    }
                }
                AppMsg::Git(id, info, kind) => {
                    if let Some(p) = self.project_mut(id) {
                        p.git = info.map(std::sync::Arc::new);
                        p.kind = kind;
                    }
                    self.git_running.remove(&id);
                    if self.git_again.remove(&id) {
                        self.refresh_git(id);
                    }
                }
                AppMsg::Files(id, paths) => {
                    self.lsp_files_changed(&paths);
                    self.on_files_changed(id, paths, now)
                }
                AppMsg::Lsp(key, generation, msg) => self.lsp_message(key, generation, msg),
                AppMsg::LspPath(path) => self.lsp_path_found(ctx, path),
                AppMsg::Review(msg) => self.on_review_msg(msg),
                AppMsg::Snapshot(pane, agent, result) => {
                    if let Some(p) = self.panes.get_mut(&pane) {
                        p.snapshot_pending = false;
                        if let Ok(s) = result {
                            p.run_snapshot = Some((s.tree, now, agent));
                        }
                    }
                }
            }
        }
        // Ctrl+D or `exit` in a shell closes its pane.
        for id in ended {
            self.close_pane(ctx, id);
        }
        // Foreground checks that are due; wake up for the next one.
        let mut next_due: Option<Instant> = None;
        for pane in self.panes.values_mut() {
            let was_program = !matches!(pane.tracker.fg, tc_core::status::Foreground::Shell);
            let agent = pane.last_agent;
            let checked = pane.check_foreground(now);
            resume_changed |= pane.last_agent != agent;
            if checked
                && was_program
                && matches!(pane.tracker.fg, tc_core::status::Foreground::Shell)
            {
                git_refresh.push(pane.project);
                self.save_due.get_or_insert(now + Duration::from_millis(800));
            }
            if let Some(d) = pane.fg_due {
                next_due = Some(next_due.map_or(d, |n| n.min(d)));
            }
            // A Running that only Enter set, and the agent never confirmed.
            pane.tracker.expire(now);
            if let Some(d) = pane.tracker.deadline() {
                next_due = Some(next_due.map_or(d, |n| n.min(d)));
            }
        }
        if let Some(d) = next_due {
            ctx.request_repaint_after(d.saturating_duration_since(now));
        }
        if resume_changed {
            self.mark_dirty();
        }
        // Soon rather than now: a script running many short commands asks once.
        for id in git_refresh {
            self.git_due.entry(id).or_insert(now + Duration::from_millis(300));
        }
    }

    /// Files changed in a project: open files follow (FR-E3), the git summary is refreshed soon,
    /// and two agents editing the same tree get a warning (FR-A6).
    fn on_files_changed(&mut self, project: ProjectId, paths: Vec<PathBuf>, now: Instant) {
        if let Some(explorer) = self.explorers.get_mut(&project) {
            explorer.invalidate(&paths);
        }
        if let Some(editor) = self.editors.get_mut(&project) {
            for f in editor.files.iter_mut() {
                if paths.contains(&f.path) {
                    crate::editor::on_disk_change(f);
                }
            }
        }
        self.git_due.entry(project).or_insert(now + Duration::from_millis(800));
        let working: Vec<String> = self
            .project(project)
            .map(|p| p.panes())
            .unwrap_or_default()
            .iter()
            .filter_map(|id| self.panes.get(id))
            .filter(|p| p.tracker.agent.is_some() && p.tracker.status == Status::Running)
            .map(|p| p.label())
            .collect();
        if working.len() >= 2
            && self
                .conflict_warned
                .get(&project)
                .is_none_or(|t| now.duration_since(*t) > Duration::from_secs(120))
        {
            self.conflict_warned.insert(project, now);
            let name = self.project(project).map(|p| p.name.clone()).unwrap_or_default();
            self.toast(tf(
                "{} agents are changing files in {} at the same time ({}). They may overwrite each other's work.",
                &[&working.len().to_string(), &name, &working.join(", ")],
            ));
        }
    }

    fn editor_mut(&mut self) -> Option<&mut crate::editor::Editor> {
        let id = self.active?;
        Some(self.editors.entry(id).or_insert_with(|| crate::editor::Editor {
            width: 0.45,
            ..Default::default()
        }))
    }

    /// The file tree of a project, made when first needed.
    pub fn explorer_mut(&mut self, project: ProjectId) -> Option<&mut crate::explorer::Explorer> {
        let root = self.project(project)?.path.clone();
        Some(
            self.explorers
                .entry(project)
                .or_insert_with(|| crate::explorer::Explorer::new(root)),
        )
    }

    /// The editor's colouring (syntect's compiled regexes, per-line colours) is let go a minute
    /// after the editor was last on screen; showing it again colours the file anew.
    fn release_editor_memory(&mut self, ctx: &egui::Context) {
        const AFTER: Duration = Duration::from_secs(60);
        let shown = self.view == View::Focus
            && self
                .active
                .and_then(|id| self.editors.get(&id))
                .is_some_and(|e| e.visible && !e.files.is_empty());
        if shown {
            self.editor_seen_at = Instant::now();
            self.editor_released = false;
            return;
        }
        // Once per time away: a document stays open (`doc` is kept) after its memory went.
        if self.editor_released {
            return;
        }
        let docs = self.editors.values().flat_map(|e| e.files.iter()).any(|f| f.doc.is_some());
        if !crate::editor::highlighting_loaded() && !docs {
            return;
        }
        let idle = self.editor_seen_at.elapsed();
        if idle < AFTER {
            ctx.request_repaint_after(AFTER - idle);
            return;
        }
        self.free_editor_memory();
        self.editor_released = true;
    }

    pub fn free_editor_memory(&mut self) {
        crate::editor::release_highlighting();
        for f in self.editors.values_mut().flat_map(|e| e.files.iter_mut()) {
            f.highlight = Default::default();
            f.index = None;
            if let Some(d) = f.doc.as_mut() {
                d.release();
            }
        }
        platform::return_free_memory();
    }

    /// Opens a file with the caret at `line`, `col` (0-based, char column).
    pub fn goto_location(&mut self, path: PathBuf, line: usize, col: usize) {
        self.open_file(path.clone(), Some(line + 1));
        if let Some(e) = self.editor_mut()
            && let Some(f) = e.files.iter_mut().find(|f| f.path == path)
        {
            let ix = tc_lsp::pos::LineIndex::new(&f.text);
            let at = (ix.line_start(line) + col).min(ix.line_start(line + 1).max(ix.line_start(line)));
            f.select = Some((at, at));
            e.refocus = true;
        }
    }

    fn open_file(&mut self, path: PathBuf, line: Option<usize>) {
        // The open project when the file is in it (a project inside another one included).
        let Some(project) = self
            .active
            .filter(|id| self.project(*id).is_some_and(|p| path.starts_with(&p.path)))
            .or_else(|| self.projects.iter().find(|p| path.starts_with(&p.path)).map(|p| p.id))
            .or(self.active)
        else {
            return;
        };
        self.active = Some(project);
        // The editor is in the Focus view.
        self.show_focus();
        if let Some(explorer) = self.explorer_mut(project) {
            explorer.reveal(&path);
        }
        let existing = self
            .editor_mut()
            .and_then(|e| e.files.iter().position(|f| f.path == path));
        match existing {
            Some(i) => {
                if let Some(e) = self.editor_mut() {
                    e.active = i;
                    e.visible = true;
                    e.files[i].goto_line = line;
                }
            }
            None if tc_doc::Kind::of(&path).is_some() => {
                let f = crate::editor::OpenFile::document(path.clone(), &self.ctx);
                if let Some(e) = self.editor_mut() {
                    e.files.push(f);
                    e.active = e.files.len() - 1;
                    e.visible = true;
                }
            }
            None => match crate::editor::load(&path) {
                Ok(mut f) => {
                    f.goto_line = line;
                    if let Some(e) = self.editor_mut() {
                        e.files.push(f);
                        e.active = e.files.len() - 1;
                        e.visible = true;
                    }
                }
                Err(crate::editor::OpenError::Binary) => {
                    self.toast(tf("{} is a binary file.", &[&paths::contract_tilde(&path)]))
                }
                Err(crate::editor::OpenError::Io(e)) => self.toast(e),
            },
        }
    }

    /// Who is looked at, notifications for what needs the user, the Dock badge.
    fn update_attention(&mut self) {
        let now = Instant::now();
        let looking = self.looking_at();
        for pane in self.panes.values_mut() {
            pane.tracker.set_focused(Some(pane.id) == looking, now);
        }
        // Notifications (FR-N4): when the window is not active, or the pane is in another project.
        let n = self.settings.notifications.clone();
        let active = self.active;
        let mut posts = Vec::new();
        for pane in self.panes.values() {
            // Asked while the user looks at the pane: no notification for it later either.
            if pane.tracker.is_focused() {
                self.notified.insert(pane.id, pane.tracker.ask);
                continue;
            }
            if !pane.tracker.wants_attention() {
                continue;
            }
            // Once per ask (a second question in a status that stays NeedsInput is one too).
            if self.notified.get(&pane.id) == Some(&pane.tracker.ask) {
                continue;
            }
            self.notified.insert(pane.id, pane.tracker.ask);
            let wanted = match pane.tracker.status {
                Status::NeedsInput => n.needs_input,
                Status::Failed(_) => n.failed,
                Status::Done => n.done,
                Status::CommandDone => n.command_done,
                _ => false,
            };
            if wanted && (!self.window_focused || Some(pane.project) != active) {
                posts.push(pane.id);
            }
        }
        for id in posts {
            let Some(pane) = self.panes.get(&id) else { continue };
            let project = self.project(pane.project).map(|p| p.name.clone()).unwrap_or_default();
            let body = pane
                .tracker
                .message
                .clone()
                .unwrap_or_else(|| ui::status::text(pane.tracker.status));
            let subtitle = format!("{project} · {}", pane.label());
            // Scripted UI checks log notifications instead of showing them.
            if cfg!(debug_assertions) && std::env::var_os("TERNARYCODE_DEV_SCRIPT").is_some() {
                eprintln!("DEVTEST LOG: notify {subtitle}: {body}");
            } else if self.clicked_rx.is_some() {
                #[cfg(target_os = "macos")]
                crate::notify_native::post("TernaryCode", &subtitle, &body, id);
            } else {
                platform::notify("TernaryCode", &subtitle, &body);
            }
        }
        self.notified.retain(|id, _| self.panes.contains_key(id));
        let badge = if self.settings.notifications.dock_badge {
            self.panes
                .values()
                .filter(|p| p.tracker.wants_attention() && p.tracker.status == Status::NeedsInput)
                .count()
        } else {
            0
        };
        if badge != self.badge {
            self.badge = badge;
            platform::set_badge(badge);
        }
    }

    // ----- actions ----------------------------------------------------------------------------

    pub fn presets(&self) -> Vec<Preset> {
        self.settings.presets()
    }

    fn add_project(&mut self, ctx: &egui::Context, path: PathBuf, name: Option<String>) {
        let path = paths::canonical(&path);
        if let Some(existing) = self.projects.iter().find(|p| p.path == path).map(|p| p.id) {
            self.active = Some(existing);
            self.focus_request = self.current_pane();
            return;
        }
        if !path.is_dir() {
            self.toast(tf("{} is not a folder.", &[&path.display().to_string()]));
            return;
        }
        let id = self.new_id();
        let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        });
        let size = self.grid_in(&Layout::Pane(0), 0);
        let pane = self.new_pane(ctx, id, path.clone(), size);
        let project = Project {
            id,
            name,
            path,
            color: (self.projects.len() % crate::project::COLORS.len()) as u8,
            preset: None,
            tabs: vec![Tab {
                layout: Layout::Pane(pane),
                focused: pane,
                title: None,
            }],
            active_tab: 0,
            git: None,
            kind: Default::default(),
        };
        self.watchers
            .watch(project.id, project.path.clone(), self.app_tx.clone(), ctx.clone());
        self.projects.push(project);
        self.refresh_git(id);
        self.active = Some(id);
        self.show_focus();
        self.focus_request = Some(pane);
        self.mark_dirty();
    }

    /// `ternarycode <target>` from a terminal (or Finder): a folder becomes a project (or opens
    /// the one it is), a file opens in the editor with its folder as the project (as when
    /// dropped); a name opens the project called that (its name, else its folder's name, else
    /// the one name it starts or contains), else adds the folder of that name found beside the
    /// projects there already.
    fn open_target(&mut self, ctx: &egui::Context, target: &str) {
        let path = Path::new(target);
        if path.is_file()
            && path.is_absolute()
            && let Some(dir) = path.parent()
        {
            self.add_project(ctx, dir.to_path_buf(), None);
            self.dispatch(ctx, Action::OpenFile(paths::canonical(path), None));
            return;
        }
        if path.is_absolute() {
            self.add_project(ctx, path.to_path_buf(), None);
            return;
        }
        let want = target.to_lowercase();
        let folder = |p: &Project| p.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let tests: [&dyn Fn(&Project) -> bool; 4] = [
            &|p| p.name.to_lowercase() == want,
            &|p| folder(p) == want,
            &|p| p.name.to_lowercase().starts_with(&want) || folder(p).starts_with(&want),
            &|p| p.name.to_lowercase().contains(&want) || folder(p).contains(&want),
        ];
        for (i, test) in tests.iter().enumerate() {
            let found: Vec<ProjectId> = self.projects.iter().filter(|p| test(p)).map(|p| p.id).collect();
            // Exact names: the first; partial ones only when they point at one project.
            if (i < 2 && !found.is_empty()) || found.len() == 1 {
                self.dispatch(ctx, Action::SelectProject(found[0]));
                return;
            }
        }
        let parents: std::collections::BTreeSet<PathBuf> =
            self.projects.iter().filter_map(|p| p.path.parent().map(Path::to_path_buf)).collect();
        let beside: Vec<PathBuf> = parents.iter().map(|d| d.join(target)).filter(|d| d.is_dir()).collect();
        if let [dir] = beside.as_slice() {
            self.add_project(ctx, dir.clone(), None);
        } else {
            self.toast(tf("No project or folder named {}.", &[target]));
        }
    }

    fn remove_project(&mut self, id: ProjectId) {
        let Some(idx) = self.projects.iter().position(|p| p.id == id) else {
            return;
        };
        let project = self.projects.remove(idx);
        self.watchers.unwatch(id);
        self.editors.remove(&id);
        self.explorers.remove(&id);
        self.git_due.remove(&id);
        self.git_again.remove(&id);
        self.conflict_warned.remove(&id);
        for pane in project.panes() {
            self.drop_pane(pane);
        }
        if self.active == Some(id) {
            self.active = self
                .projects
                .get(idx.min(self.projects.len().saturating_sub(1)))
                .map(|p| p.id);
            self.focus_request = self.current_pane();
        }
        self.mark_dirty();
    }

    fn drop_pane(&mut self, id: PaneId) {
        if let Some(mut pane) = self.panes.remove(&id) {
            pane.hang_up();
        }
        if let Some(h) = &self.hooks {
            h.unregister(id);
        }
        self.jumps.forget(id);
        self.notified.remove(&id);
        if self.grid_focus == Some(id) {
            self.grid_focus = None;
        }
    }

    fn split(&mut self, ctx: &egui::Context, dir: SplitDir) -> Option<PaneId> {
        // In the Grid, the tile typed into (its project need not be the open one).
        let current = self.command_pane()?;
        let (project, cwd) = self.panes.get(&current).map(|p| (p.project, p.cwd.clone()))?;
        let tab_ix = self.project(project)?.tab_of(current)?;
        // The new pane's size in the split layout, before its shell starts.
        const PLACEHOLDER: PaneId = PaneId::MAX;
        let mut planned = self.project(project)?.tabs[tab_ix].layout.clone();
        planned.split(current, PLACEHOLDER, dir);
        let size = self.grid_in(&planned, PLACEHOLDER);
        let new = self.new_pane(ctx, project, cwd, size);
        let p = self.project_mut(project)?;
        let tab = &mut p.tabs[tab_ix];
        tab.layout.split(current, new, dir);
        tab.focused = new;
        self.focus_request = Some(new);
        self.mark_dirty();
        self.fit_to_focus(ctx, project, false);
        Some(new)
    }

    fn new_tab(&mut self, ctx: &egui::Context) -> Option<PaneId> {
        self.new_tab_in(ctx, None)
    }

    /// A new tab whose shell starts in `cwd` (the project folder by default), in the project
    /// commands act on (in the Grid, that of the tile typed into).
    fn new_tab_in(&mut self, ctx: &egui::Context, cwd: Option<PathBuf>) -> Option<PaneId> {
        let project = self.command_project()?;
        let cwd = cwd.filter(|d| d.is_dir()).unwrap_or(self.project(project)?.path.clone());
        let size = self.grid_in(&Layout::Pane(0), 0);
        let new = self.new_pane(ctx, project, cwd, size);
        let p = self.project_mut(project)?;
        p.tabs.push(Tab {
            layout: Layout::Pane(new),
            focused: new,
            title: None,
        });
        p.active_tab = p.tabs.len() - 1;
        self.focus_request = Some(new);
        self.mark_dirty();
        Some(new)
    }

    fn close_pane(&mut self, ctx: &egui::Context, id: PaneId) {
        let Some(project_id) = self.panes.get(&id).map(|p| p.project) else {
            return;
        };
        let Some(p) = self.project_mut(project_id) else {
            return;
        };
        let Some(tab_idx) = p.tab_of(id) else { return };
        let tab = &mut p.tabs[tab_idx];
        if tab.layout.remove(id) {
            if tab.focused == id {
                tab.focused = tab.layout.panes()[0];
            }
        } else if p.tabs.len() > 1 {
            p.tabs.remove(tab_idx);
            // The tab in view stays in view when one before it closes.
            if tab_idx < p.active_tab {
                p.active_tab -= 1;
            }
            p.active_tab = p.active_tab.min(p.tabs.len() - 1);
        } else {
            // The project's last pane: keep the project, give it a fresh shell.
            self.drop_pane(id);
            let cwd = self.project(project_id).map(|p| p.path.clone()).unwrap_or_default();
            let size = self.grid_in(&Layout::Pane(0), 0);
            let new = self.new_pane(ctx, project_id, cwd, size);
            if let Some(p) = self.project_mut(project_id) {
                p.tabs[0] = Tab {
                    layout: Layout::Pane(new),
                    focused: new,
                    title: None,
                };
            }
            self.focus_request = Some(new);
            self.mark_dirty();
            return;
        }
        self.drop_pane(id);
        self.focus_request = self.current_pane();
        self.mark_dirty();
        self.fit_to_focus(ctx, project_id, false);
    }

    /// Starts a preset in a new pane of the active project: beside the current pane while the
    /// tab is not crowded, otherwise in a new tab.
    fn run_preset(&mut self, ctx: &egui::Context, preset_id: &str) {
        let Some(preset) = self.presets().into_iter().find(|p| p.id == preset_id) else {
            return;
        };
        if self.active.is_none() {
            self.toast(t("Add a project first.").into());
            return;
        }
        let crowded = self
            .command_project()
            .and_then(|id| self.project(id))
            .and_then(Project::tab)
            .is_some_and(|tab| tab.layout.panes().len() >= 3);
        let pane = if crowded {
            self.new_tab(ctx)
        } else {
            self.split(ctx, SplitDir::Horizontal)
        };
        let Some(pane) = pane else { return };
        let command = self.launch_command(&preset, &preset.command);
        let mut project = None;
        if let Some(p) = self.panes.get_mut(&pane) {
            p.preset = Some(preset.id.clone());
            p.type_command(command);
            project = Some(p.project);
        }
        if let Some(project) = project {
            self.actions
                .push(Action::SetProjectPreset(project, Some(preset.id)));
        }
    }

    pub fn launch_command(&self, preset: &Preset, command: &str) -> String {
        // Without the per-launch settings file (it could not be written) the flags would pass
        // empty arguments: the agent starts as typed.
        let integration = preset
            .agent
            .filter(|_| self.per_launch)
            .map(|a| self.settings.integration(a))
            .unwrap_or(Integration::Off);
        integrate::launch_command(command, preset.agent, integration, self.codex_hooks)
    }

    fn resume(&mut self, ctx: &egui::Context, pane: PaneId) {
        self.wake(ctx, pane);
        let presets = self.presets();
        let Some(p) = self.panes.get(&pane) else { return };
        let Some((agent, command)) = p.resume.clone() else {
            return;
        };
        let preset = p
            .preset
            .as_ref()
            .and_then(|id| presets.iter().find(|x| &x.id == id))
            .or_else(|| presets.iter().find(|x| x.agent == Some(agent)))
            .cloned();
        let typed = match preset {
            Some(preset) => self.launch_command(&preset, &command),
            None => command,
        };
        if let Some(p) = self.panes.get_mut(&pane) {
            p.resume = None;
            p.type_command(typed);
        }
        self.focus_pane(pane);
    }

    fn restart_shell(&mut self, ctx: &egui::Context, id: PaneId) {
        // Only a pane without a shell: a sleeping pane drawn as Enter is pressed asks both to
        // wake and to restart.
        if self.panes.get(&id).is_none_or(Pane::alive) {
            return;
        }
        let spawn_hook = self.hooks.as_ref().map(|h| {
            (
                h.path().to_string_lossy().into_owned(),
                h.register(id),
            )
        });
        let dark = ctx.global_style().visuals.dark_mode;
        let palette = self.scheme(dark).into();
        let spawn = SpawnCtx {
            rt: self.rt.handle(),
            tx: &self.tx,
            ctx,
            env: &self.base_env,
            hook: spawn_hook,
            scrollback: self.settings.scrollback,
            palette,
            cursor: tc_term::prefs::CursorStyle::Block,
            blink: self.settings.cursor_blink,
            long_command: Duration::from_secs(self.settings.long_command_secs.max(1)),
        };
        if let Some(p) = self.panes.get_mut(&id) {
            p.start_shell(&spawn);
        }
    }

    pub fn toast(&mut self, text: String) {
        self.toast = Some((text, Instant::now()));
    }

    /// Editor files with changes not saved ("main.rs · not saved"), of one project or all.
    fn unsaved_files(&self, project: Option<ProjectId>) -> Vec<Busy> {
        self.editors
            .iter()
            .filter(|(id, _)| project.is_none_or(|p| p == **id))
            .flat_map(|(_, e)| e.files.iter())
            .filter(|f| f.dirty() && !f.read_only)
            .map(|f| Busy { text: tf("{} · not saved", &[&f.name()]), file: true })
            .collect()
    }

    /// Programs that quitting would stop.
    fn busy_panes(&self) -> Vec<Busy> {
        let mut out = Vec::new();
        for project in &self.projects {
            for id in project.panes() {
                if let Some(what) = self.panes.get(&id).and_then(Pane::busy_with) {
                    out.push(Busy { text: format!("{} · {what}", project.name), file: false });
                }
            }
        }
        out
    }

    /// The editor shows a document and no terminal has the keyboard (⌘F then searches it).
    fn doc_has_keyboard(&self, ctx: &egui::Context) -> bool {
        let Some(e) = self.active.and_then(|id| self.editors.get(&id)) else { return false };
        if !e.visible || e.file().and_then(|f| f.doc.as_ref()).is_none() {
            return false;
        }
        let focused = ctx.memory(|m| m.focused());
        !self.panes.keys().any(|id| focused == Some(tc_term::widget::pane_widget_id(*id)))
    }

    /// Starts the update's installer and quits; the new version opens by itself and restores
    /// the workspace.
    fn install_update(&mut self, ctx: &egui::Context) {
        match self.updater.launch_installer() {
            Ok(()) => self.quit_now(ctx),
            Err(e) => {
                self.dialog = Some(Dialog::Message(
                    t("Software Update").to_string(),
                    tf("The update could not be installed: {}", &[&e]),
                ))
            }
        }
    }

    fn quit_now(&mut self, ctx: &egui::Context) {
        self.save_and_wait();
        self.lsp_stop_all();
        for pane in self.panes.values_mut() {
            pane.hang_up();
        }
        self.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn install_record_path(agent: AgentKind) -> PathBuf {
        paths::integrations_dir().join(format!("{}-install.json", agent.slug()))
    }

    fn agent_config_path(agent: AgentKind) -> Option<PathBuf> {
        let home = paths::home_dir();
        match agent {
            AgentKind::Claude => Some(home.join(".claude/settings.json")),
            AgentKind::Codex => Some(home.join(".codex/config.toml")),
            AgentKind::Gemini => Some(home.join(".gemini/settings.json")),
            AgentKind::OpenCode => Some(
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".config"))
                    .join("opencode/plugins/ternarycode.js"),
            ),
            _ => None,
        }
    }

    pub fn installed(&self, agent: AgentKind) -> Option<InstallRecord> {
        let data = std::fs::read(Self::install_record_path(agent)).ok()?;
        serde_json::from_slice(&data).ok()
    }

    pub fn dispatch(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::AddProject => {
                self.dialog = Some(Dialog::AddProject {
                    path: String::new(),
                    name: String::new(),
                })
            }
            Action::AddProjectPath(path) => {
                let name = self
                    .pending_rename
                    .take()
                    .filter(|(p, _)| *p == path)
                    .map(|(_, n)| n);
                self.add_project(ctx, path, name);
            }
            Action::OpenTarget(target) => self.open_target(ctx, &target),
            Action::SelectProject(id) => {
                if let Some(path) = self.project(id).map(|p| p.path.clone()) {
                    // A folder that could not be watched (not there at launch) is tried again.
                    self.watchers.watch(id, path, self.app_tx.clone(), ctx.clone());
                    self.active = Some(id);
                    self.show_focus();
                    self.focus_request = self.current_pane();
                    self.mark_dirty();
                }
            }
            Action::SelectProjectIndex(i) => {
                if let Some(id) = self.projects.get(i).map(|p| p.id) {
                    self.dispatch(ctx, Action::SelectProject(id));
                }
            }
            Action::RemoveProject(id) => {
                let mut busy: Vec<Busy> = self
                    .project(id)
                    .map(|p| {
                        p.panes()
                            .iter()
                            .filter_map(|pid| self.panes.get(pid).and_then(Pane::busy_with))
                            .map(|text| Busy { text, file: false })
                            .collect()
                    })
                    .unwrap_or_default();
                busy.extend(self.unsaved_files(Some(id)));
                if busy.is_empty() {
                    self.remove_project(id);
                } else {
                    self.dialog = Some(Dialog::ConfirmRemove(id, busy));
                }
            }
            Action::RemoveProjectNow(id) => self.remove_project(id),
            Action::RenameProject(id, name) => {
                if let Some(p) = self.project_mut(id)
                    && !name.trim().is_empty()
                {
                    p.name = name.trim().to_string();
                }
                self.mark_dirty();
            }
            Action::RenamePane(id, title) => {
                let mut project = None;
                if let Some(p) = self.panes.get_mut(&id) {
                    p.title = Some(title.trim().to_string()).filter(|t| !t.is_empty());
                    project = Some(p.project);
                }
                // A tab shows its own title before its focused pane's: a stale one (from the saved
                // workspace) would hide the rename.
                if let Some(p) = project.and_then(|pid| self.project_mut(pid)) {
                    for tab in p.tabs.iter_mut().filter(|t| t.focused == id) {
                        tab.title = None;
                    }
                }
                self.mark_dirty();
            }
            Action::SetProjectColor(id, c) => {
                if let Some(p) = self.project_mut(id) {
                    p.color = c;
                }
                self.mark_dirty();
            }
            Action::SetProjectPreset(id, preset) => {
                if let Some(p) = self.project_mut(id) {
                    p.preset = preset;
                }
                self.mark_dirty();
            }
            Action::MoveProject { from, to } => {
                if from < self.projects.len() && to <= self.projects.len() && from != to {
                    let p = self.projects.remove(from);
                    let to = if to > from { to - 1 } else { to };
                    self.projects.insert(to.min(self.projects.len()), p);
                    self.mark_dirty();
                }
            }
            Action::RevealProject(id) => {
                if let Some(p) = self.project(id) {
                    platform::reveal(&p.path);
                }
            }
            Action::NewTab => {
                self.new_tab(ctx);
            }
            Action::SelectTab(i) => {
                if let Some(p) = self.active.and_then(|id| self.project_mut(id))
                    && i < p.tabs.len()
                {
                    p.active_tab = i;
                    self.focus_request = self.current_pane();
                    self.mark_dirty();
                }
            }
            Action::NextTab | Action::PrevTab => {
                if let Some(p) = self.active.and_then(|id| self.project_mut(id))
                    && !p.tabs.is_empty()
                {
                    let n = p.tabs.len();
                    p.active_tab = if action == Action::NextTab {
                        (p.active_tab + 1) % n
                    } else {
                        (p.active_tab + n - 1) % n
                    };
                    self.focus_request = self.current_pane();
                    self.mark_dirty();
                }
            }
            Action::MoveTab { from, to } => {
                if let Some(p) = self.active.and_then(|id| self.project_mut(id))
                    && from < p.tabs.len()
                    && to <= p.tabs.len()
                    && to != from
                    && to != from + 1
                {
                    let active = p.tabs.get(p.active_tab).map(|t| t.focused);
                    let tab = p.tabs.remove(from);
                    let to = if to > from { to - 1 } else { to };
                    p.tabs.insert(to.min(p.tabs.len()), tab);
                    if let Some(f) = active {
                        p.active_tab = p.tabs.iter().position(|t| t.focused == f).unwrap_or(0);
                    }
                    self.mark_dirty();
                }
            }
            Action::MergeTab { tab, target, dir, before } => {
                if let Some(p) = self.active.and_then(|id| self.project_mut(id))
                    && tab < p.tabs.len()
                    && tab != p.active_tab
                    && p.tab().is_some_and(|t| t.layout.contains(target))
                {
                    let moved = p.tabs.remove(tab);
                    if tab < p.active_tab {
                        p.active_tab -= 1;
                    }
                    let focused = moved.focused;
                    if let Some(t) = p.tab_mut()
                        && t.layout.insert(target, moved.layout, dir, before)
                    {
                        t.focused = focused;
                    }
                    self.focus_request = Some(focused);
                    self.mark_dirty();
                }
            }
            Action::CloseTab(i) => {
                let busy: Vec<Busy> = self
                    .active_project()
                    .and_then(|p| p.tabs.get(i))
                    .map(|tab| {
                        tab.layout
                            .panes()
                            .iter()
                            .filter_map(|id| self.panes.get(id).and_then(Pane::busy_with))
                            .map(|text| Busy { text, file: false })
                            .collect()
                    })
                    .unwrap_or_default();
                if busy.is_empty() {
                    self.dispatch(ctx, Action::CloseTabNow(i));
                } else {
                    let panes = self.active_project().and_then(|p| p.tabs.get(i)).map(|t| t.layout.panes()).unwrap_or_default();
                    self.dialog = Some(Dialog::ConfirmCloseTab(panes, busy));
                }
            }
            Action::ClosePanesNow(panes) => {
                for id in panes {
                    if self.panes.contains_key(&id) {
                        self.close_pane(ctx, id);
                    }
                }
            }
            Action::CloseTabNow(i) => {
                let panes = self
                    .active_project()
                    .and_then(|p| p.tabs.get(i))
                    .map(|t| t.layout.panes())
                    .unwrap_or_default();
                // Closing each pane keeps the project's last tab with a fresh shell, as ⌘W does.
                for id in panes {
                    self.close_pane(ctx, id);
                }
            }
            Action::MovePane { pane, target, dir, before } => {
                if let Some(p) = self.active.and_then(|id| self.project_mut(id))
                    && let Some(tab) = p.tab_mut()
                    && pane != target
                    && tab.layout.contains(pane)
                    && tab.layout.contains(target)
                    && tab.layout.remove(pane)
                {
                    tab.layout.insert(target, Layout::Pane(pane), dir, before);
                    tab.focused = pane;
                    self.focus_request = Some(pane);
                    self.mark_dirty();
                }
            }
            Action::PaneToTab(pane) => {
                if let Some(p) = self
                    .panes
                    .get(&pane)
                    .map(|x| x.project)
                    .and_then(|id| self.projects.iter_mut().find(|p| p.id == id))
                    && let Some(i) = p.tab_of(pane)
                    && p.tabs[i].layout.remove(pane)
                {
                    if p.tabs[i].focused == pane {
                        p.tabs[i].focused = p.tabs[i].layout.panes()[0];
                    }
                    p.tabs.insert(
                        i + 1,
                        Tab {
                            layout: Layout::Pane(pane),
                            focused: pane,
                            title: None,
                        },
                    );
                    p.active_tab = i + 1;
                    self.focus_request = Some(pane);
                    self.mark_dirty();
                }
            }
            Action::SplitRight => {
                self.split(ctx, SplitDir::Horizontal);
            }
            Action::SplitDown => {
                self.split(ctx, SplitDir::Vertical);
            }
            Action::ClosePane => {
                // ⌘W closes what has the keyboard: the file in the editor, else the pane typed
                // into (in the Grid, the tile), asking first when a program runs there.
                if self.editor_has_keyboard(ctx)
                    && let Some(i) = self.editor_mut().filter(|e| !e.files.is_empty()).map(|e| e.active)
                {
                    self.dispatch(ctx, Action::CloseFile(i));
                    return;
                }
                if let Some(id) = self.target_pane() {
                    match self.panes.get(&id).and_then(Pane::busy_with) {
                        Some(what) => self.dialog = Some(Dialog::ConfirmClosePane(id, what)),
                        None => self.close_pane(ctx, id),
                    }
                }
            }
            Action::ClosePaneId(id) => self.close_pane(ctx, id),
            Action::FocusPane(id) => self.focus_pane(id),
            Action::NextPane | Action::PrevPane => {
                if let Some(p) = self.active.and_then(|id| self.project_mut(id))
                    && let Some(tab) = p.tab_mut()
                {
                    tab.focused = tab.layout.neighbor(tab.focused, action == Action::NextPane);
                    let f = tab.focused;
                    self.focus_request = Some(f);
                    self.mark_dirty();
                }
            }
            Action::RunPreset(id) => self.run_preset(ctx, &id),
            Action::Resume(id) => self.resume(ctx, id),
            Action::WakePane(id) => self.wake(ctx, id),
            Action::GridActiveOnly(only) => {
                self.grid_active_only = only;
                self.mark_dirty();
            }
            Action::RestartShell(id) => self.restart_shell(ctx, id),
            Action::NextAttention => {
                let queue = self.attention_queue();
                match self.jumps.next(self.command_pane(), &queue) {
                    Some(pane) => self.focus_pane(pane),
                    None => self.toast(t("No agent is waiting for you.").into()),
                }
            }
            Action::BackAttention => {
                if let Some(pane) = self.jumps.back().filter(|p| self.panes.contains_key(p)) {
                    self.focus_pane(pane);
                }
            }
            Action::ToggleAttention => self.attention_open = !self.attention_open,
            Action::ToggleSidebar => {
                self.settings.sidebar_hidden = !self.settings.sidebar_hidden;
                self.settings_changed();
            }
            Action::Palette => self.open_palette(PaletteMode::All),
            Action::SwitchProject => self.open_palette(PaletteMode::Projects),
            Action::PickPreset => self.open_palette(PaletteMode::Presets),
            Action::SetView(v) => {
                if v == View::Focus {
                    self.show_focus();
                } else {
                    self.view = v;
                }
                self.grid_focus = None;
                self.focus_request = if v == View::Focus { self.current_pane() } else { None };
                self.mark_dirty();
            }
            Action::ToggleGrid => {
                let v = if self.view == View::Grid { View::Focus } else { View::Grid };
                self.dispatch(ctx, Action::SetView(v));
            }
            Action::Settings => {
                self.dialog = Some(Dialog::Settings(ui::settings::SettingsPage::General))
            }
            Action::Integrations => {
                self.dialog = Some(Dialog::Settings(ui::settings::SettingsPage::Integrations))
            }
            Action::PlanInstall(agent) => {
                let Some(path) = Self::agent_config_path(agent) else {
                    return;
                };
                let plan = match agent {
                    AgentKind::Claude => integrate::plan_claude(&self.hook_bin, &path),
                    AgentKind::Codex => integrate::plan_codex(&self.hook_bin, &path),
                    AgentKind::Gemini => integrate::plan_gemini(&self.hook_bin, &path),
                    AgentKind::OpenCode => integrate::plan_opencode(&self.hook_bin, &path),
                    _ => Err(t("This agent cannot be installed yet.").to_string()),
                };
                match plan {
                    Ok(plan) => {
                        let diff = integrate::diff(&plan);
                        self.dialog = Some(Dialog::Install(Box::new(plan), diff));
                    }
                    Err(e) => {
                        self.dialog = Some(Dialog::Message(t("Cannot install hooks").into(), e))
                    }
                }
            }
            Action::ConfirmInstall => {
                if let Some(Dialog::Install(plan, _)) = self.dialog.take() {
                    match integrate::apply(&plan, &paths::integrations_dir().join("backup")) {
                        Ok(record) => {
                            let saved = tc_core::persist::save_json(
                                &Self::install_record_path(plan.agent),
                                &record,
                            );
                            self.settings
                                .integrations
                                .insert(plan.agent.slug().into(), Integration::Installed);
                            self.settings_changed();
                            self.toast(match saved {
                                Ok(()) => tf("Hooks installed for {}.", &[plan.agent.label()]),
                                // Without the record, Uninstall cannot put the file back.
                                Err(e) => tf(
                                    "Hooks installed for {}, but their record could not be saved ({}): Uninstall will not restore {}.",
                                    &[plan.agent.label(), &e.to_string(), &paths::contract_tilde(&plan.path)],
                                ),
                            });
                        }
                        Err(e) => {
                            self.dialog = Some(Dialog::Message(
                                t("Cannot install hooks").into(),
                                e.to_string(),
                            ))
                        }
                    }
                    if self.dialog.is_none() {
                        self.dialog =
                            Some(Dialog::Settings(ui::settings::SettingsPage::Integrations));
                    }
                }
            }
            Action::Uninstall(agent) => {
                if let Some(record) = self.installed(agent) {
                    match integrate::uninstall(&record, &self.hook_bin) {
                        Ok(how) => {
                            let _ = std::fs::remove_file(Self::install_record_path(agent));
                            self.toast(match how {
                                integrate::Uninstalled::Restored => tf(
                                    "{} restored exactly as it was.",
                                    &[&paths::contract_tilde(&record.path)],
                                ),
                                integrate::Uninstalled::Removed => tf(
                                    "TernaryCode's hooks removed from {}; your later changes were kept.",
                                    &[&paths::contract_tilde(&record.path)],
                                ),
                            });
                        }
                        Err(e) => {
                            // The hooks may still be in the file: the agent stays as installed
                            // (per-launch flags on top would report every event twice).
                            self.toast(e);
                            return;
                        }
                    }
                }
                self.settings
                    .integrations
                    .insert(agent.slug().into(), Integration::PerLaunch);
                self.settings_changed();
            }
            Action::OpenFile(path, line) => {
                self.open_file(path.clone(), line);
                self.lsp_open(ctx, &path);
            }
            Action::GotoLocation(path, line, col) => {
                self.goto_location(path.clone(), line, col);
                self.lsp_open(ctx, &path);
            }
            Action::Ide(cmd) => self.ide_cmd(ctx, cmd),
            Action::CloseFile(i) => {
                if let Some(e) = self.editor_mut()
                    && i < e.files.len()
                {
                    // Unsaved edits are kept until saved or reloaded: closing asks nothing but
                    // never drops a dirty buffer silently.
                    if e.files[i].dirty() {
                        e.active = i;
                        let name = e.files[i].name();
                        self.toast(tf(
                            "{} has unsaved changes. Save them ({}) or reload the file first.",
                            &[&name, ui::kit::keys(&Action::SaveFile).unwrap_or("⌘S")],
                        ));
                    } else {
                        e.close(i);
                    }
                }
            }
            Action::SaveFile | Action::ForceSave => {
                // A document view has nothing to save.
                if self.editor_mut().and_then(|e| e.file_mut()).is_some_and(|f| f.doc.is_some()) {
                    return;
                }
                let force = action == Action::ForceSave;
                // Formatted first (the answer saves) when asked for and a server can.
                if !force && self.settings.lsp && self.settings.format_on_save && self.format_then_save() {
                    return;
                }
                let result = self.editor_mut().and_then(|e| e.file_mut()).map(|f| (crate::editor::save(f, force), f.name(), f.path.clone()));
                if let Some((Ok(()), _, path)) = &result {
                    self.lsp_saved(path);
                }
                match result.map(|(r, n, _)| (r, n)) {
                    Some((Ok(()), name)) => self.toast(tf("Saved {}.", &[&name])),
                    Some((Err(crate::editor::SaveError::ChangedOnDisk), _)) => {
                        self.toast(t("Not saved: the file changed on disk. Choose what to keep in the editor.").into())
                    }
                    Some((Err(crate::editor::SaveError::Io(e)), _)) => self.toast(e),
                    None => {}
                }
            }
            Action::ReloadFile => {
                if let Some(f) = self.editor_mut().and_then(|e| e.file_mut()) {
                    crate::editor::reload(f);
                }
            }
            Action::ToggleEditor => {
                if let Some(e) = self.editor_mut() {
                    e.visible = !e.visible;
                }
                self.show_focus();
                if !self.editors.get(&self.active.unwrap_or_default()).is_some_and(|e| e.visible) {
                    self.focus_request = self.current_pane();
                }
            }
            Action::ToggleFiles => {
                // With the sidebar hidden, this shows it with the files open.
                if self.settings.sidebar_hidden {
                    self.settings.sidebar_hidden = false;
                    self.settings.files_hidden = false;
                } else {
                    self.settings.files_hidden = !self.settings.files_hidden;
                }
                self.settings_changed();
            }
            Action::InsertPath(path) => {
                if let Some(id) = self.current_pane() {
                    self.wake(ctx, id);
                }
                if let Some(id) = self.current_pane()
                    && let Some(pane) = self.panes.get_mut(&id)
                {
                    let shown = path
                        .strip_prefix(&pane.cwd)
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|_| path.clone());
                    let text = format!("{} ", platform::shell_quote(&shown.to_string_lossy()));
                    pane.input(text.into_bytes());
                    self.show_focus();
                    self.focus_request = Some(id);
                }
            }
            Action::RevealPath(path) => platform::reveal_in_folder(&path),
            Action::NewFileIn(ref dir) | Action::NewFolderIn(ref dir) => {
                let folder = matches!(action, Action::NewFolderIn(_));
                let dir = dir.clone();
                if let Some(pid) = self.active {
                    self.settings.files_hidden = false;
                    self.settings.sidebar_hidden = false;
                    if let Some(ex) = self.explorer_mut(pid) {
                        ex.start_new(&dir, folder);
                    }
                }
            }
            Action::RenamePath(path) => {
                if let Some(ex) = self.active.and_then(|pid| self.explorer_mut(pid)) {
                    ex.start_rename(&path);
                }
            }
            Action::PathRenamed { old, new } => {
                let mut moved = Vec::new();
                for e in self.editors.values_mut() {
                    for f in e.files.iter_mut() {
                        if let Ok(rest) = f.path.strip_prefix(&old) {
                            f.path = if rest.as_os_str().is_empty() { new.clone() } else { new.join(rest) };
                            f.syntax = crate::editor::syntax_for(&f.path);
                            f.highlight = Default::default();
                            moved.push(f.path.clone());
                        }
                    }
                }
                // The language server closes the old name (the next sync) and opens the new.
                for path in moved {
                    self.lsp_open(ctx, &path);
                }
            }
            Action::DuplicatePath(path) => {
                let copy = crate::explorer::duplicate_name(&path);
                match std::fs::copy(&path, &copy) {
                    Ok(_) => {
                        if let Some(ex) = self.active.and_then(|pid| self.explorer_mut(pid)) {
                            ex.invalidate(std::slice::from_ref(&copy));
                            ex.reveal(&copy);
                        }
                    }
                    Err(e) => self.toast(e.to_string()),
                }
            }
            Action::TrashPath(path) => self.dialog = Some(Dialog::ConfirmTrash(path)),
            Action::TrashPathNow(path) => match platform::trash(&path) {
                Ok(()) => {
                    if let Some(ex) = self.active.and_then(|pid| self.explorer_mut(pid)) {
                        ex.invalidate(std::slice::from_ref(&path));
                    }
                    // Open files that went with it close, unless they hold unsaved edits.
                    for e in self.editors.values_mut() {
                        let gone: Vec<usize> = e
                            .files
                            .iter()
                            .enumerate()
                            .filter(|(_, f)| f.path.starts_with(&path) && !f.dirty())
                            .map(|(i, _)| i)
                            .collect();
                        for i in gone.into_iter().rev() {
                            e.close(i);
                        }
                    }
                    self.toast(tf("{} moved to the Trash.", &[&paths::contract_tilde(&path)]));
                }
                Err(e) => self.toast(e),
            },
            Action::NewTabAt(dir) => {
                self.show_focus();
                self.new_tab_in(ctx, Some(dir));
            }
            Action::QuickOpen => self.open_palette(PaletteMode::Files),
            // A document shown in the editor, with the keyboard not in a terminal: search it.
            Action::Find if self.view == View::Focus && self.doc_has_keyboard(ctx) => {
                if let Some(v) = self.active.and_then(|id| self.editors.get_mut(&id)).and_then(|e| e.file_mut()).and_then(|f| f.doc.as_deref_mut()) {
                    v.open_find();
                }
            }
            Action::Find if self.editor_keyboard && self.view == View::Focus => {
                if let Some(e) = self.active.and_then(|id| self.editors.get_mut(&id)) {
                    e.find.get_or_insert_with(Default::default).focus = true;
                }
            }
            Action::Find => {
                if let Some(pane) = self.command_pane() {
                    // From the Grid: the tile typed into, in the Focus view (its find bar is there).
                    if self.view == View::Grid {
                        self.focus_pane(pane);
                    }
                    self.pane_search = Some(PaneSearch {
                        pane,
                        query: String::new(),
                        current: None,
                        opened: true,
                        missing: false,
                    });
                }
            }
            Action::FindAll => self.open_palette(PaletteMode::Search),
            Action::Review => self.open_review(None),
            Action::RevealMatch(pane, m) => {
                self.focus_pane(pane);
                if let Some(p) = self.panes.get(&pane) {
                    tc_term::search::reveal(&p.shared, &m);
                }
            }
            Action::Copy => {
                // The menu takes ⌘C before any widget: a text field (the editor, a dialog, the
                // palette) gets it as egui's own copy; else the terminal copies its selection.
                if self.text_field_focused(ctx) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy);
                } else if let Some(p) = self.target_pane().and_then(|id| self.panes.get(&id)) {
                    tc_term::widget::copy_selection(ctx, &p.shared);
                }
            }
            Action::Paste => {
                // A terminal pastes the clipboard as text (on Windows and Linux a paste event
                // without Shift would reach it as a typed Ctrl+V); a text field gets egui's own.
                match self.target_pane().filter(|_| !self.text_field_focused(ctx)) {
                    Some(id) if let Some(p) = self.panes.get_mut(&id) => p.view.request_paste(ctx),
                    _ => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
                }
            }
            Action::SelectAll => {
                if self.text_field_focused(ctx) {
                    // The key the menu took, given to the text field (actions run before the
                    // widgets of this frame).
                    ctx.input_mut(|i| {
                        i.events.push(egui::Event::Key {
                            key: egui::Key::A,
                            physical_key: Some(egui::Key::A),
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::COMMAND,
                        })
                    });
                } else if let Some(p) = self.target_pane().and_then(|id| self.panes.get(&id)) {
                    tc_term::widget::select_all(&p.shared);
                }
            }
            Action::ClearScrollback => {
                if let Some(p) = self.target_pane().and_then(|id| self.panes.get(&id)) {
                    tc_term::widget::clear_scrollback(&p.shared);
                }
            }
            Action::FontBigger | Action::FontSmaller | Action::FontReset => {
                self.settings.font_size = match action {
                    Action::FontBigger => (self.settings.font_size + 1.0).min(32.0),
                    Action::FontSmaller => (self.settings.font_size - 1.0).max(8.0),
                    _ => 13.0,
                };
                self.settings_changed();
            }
            Action::Quit => {
                let mut busy = self.busy_panes();
                busy.extend(self.unsaved_files(None));
                if busy.is_empty() {
                    self.quit_now(ctx);
                } else {
                    self.dialog = Some(Dialog::ConfirmQuit(busy));
                }
            }
            Action::QuitNow => self.quit_now(ctx),
            Action::CheckForUpdates => self.updater.check_now(ctx),
            Action::ShowUpdate => self.updater.open_dialog(),
            Action::RestartToUpdate => {
                let mut busy = self.busy_panes();
                busy.extend(self.unsaved_files(None));
                if busy.is_empty() {
                    self.install_update(ctx);
                } else {
                    self.dialog = Some(Dialog::ConfirmUpdate(busy));
                }
            }
            Action::RestartToUpdateNow => self.install_update(ctx),
        }
    }

    pub fn open_palette(&mut self, mode: PaletteMode) {
        self.pane_search = None;
        if mode == PaletteMode::Files && self.active.is_none() {
            return;
        }
        // Listing a big repository's files (`git ls-files`) takes a while: off the UI thread.
        let loading = mode == PaletteMode::Files;
        if let Some(p) = self.active_project().filter(|_| loading) {
            let (tx, root, id) = (self.app_tx.clone(), p.path.clone(), p.id);
            let ctx = self.ctx.clone();
            self.rt.spawn_blocking(move || {
                let _ = tx.send(AppMsg::ProjectFiles(id, crate::editor::project_files(&root, 20_000)));
                ctx.request_repaint();
            });
        }
        self.palette = Some(Palette {
            loading,
            files: Vec::new(),
            searched: String::new(),
            results: Vec::new(),
            mode,
            query: String::new(),
            selected: 0,
            opened: true,
            title: String::new(),
            locations: Vec::new(),
            from_editor: self.editor_keyboard || mode == PaletteMode::Locations,
        });
    }

    // ----- input ------------------------------------------------------------------------------

    fn handle_menu(&mut self) {
        // The language changed: the menu bar is built again in it.
        #[cfg(target_os = "macos")]
        if MENU_STALE.swap(false, std::sync::atomic::Ordering::Relaxed) {
            self.menu = Some((ui::menu::AppMenu::install(), ui::menu::event_channel(&self.ctx)));
        }
        #[cfg(target_os = "macos")]
        if let Some((menu, rx)) = &self.menu {
            while let Ok(event) = rx.try_recv() {
                if let Some(a) = menu.action(event.id()) {
                    self.actions.push(a);
                }
            }
        }
    }

    /// Shortcuts egui handles itself: everything on Windows and Linux (Ctrl+Shift, since plain
    /// Ctrl belongs to the terminal); on macOS the menu has them, except ⌘1–9.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let cmd = if cfg!(target_os = "macos") {
            Modifiers::MAC_CMD
        } else {
            Modifiers::CTRL | Modifiers::SHIFT
        };
        let digits = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (i, key) in digits.iter().enumerate() {
            if ctx.input_mut(|inp| inp.consume_shortcut(&KeyboardShortcut::new(cmd, *key))) {
                self.actions.push(Action::SelectProjectIndex(i));
            }
        }
        // Shift+1 reads as "!" where egui knows that key (the other digits fall back to the
        // physical key).
        if !cfg!(target_os = "macos")
            && ctx.input_mut(|inp| inp.consume_shortcut(&KeyboardShortcut::new(cmd, Key::Exclamationmark)))
        {
            self.actions.push(Action::SelectProjectIndex(0));
        }
        if cfg!(target_os = "macos") {
            return;
        }
        let map = [
            (Key::J, Action::NextAttention),
            (Key::K, Action::Palette),
            (Key::P, Action::SwitchProject),
            (Key::A, Action::PickPreset),
            (Key::B, Action::ToggleSidebar),
            (Key::T, Action::NewTab),
            (Key::W, Action::ClosePane),
            (Key::D, Action::SplitRight),
            (Key::E, Action::SplitDown),
            (Key::C, Action::Copy),
            (Key::V, Action::Paste),
            (Key::N, Action::AddProject),
            (Key::O, Action::QuickOpen),
            (Key::Comma, Action::Settings),
            (Key::Q, Action::Quit),
            (Key::Num0, Action::ToggleGrid),
            (Key::S, Action::SaveFile),
            (Key::F, Action::Find),
            (Key::G, Action::FindAll),
            (Key::R, Action::Review),
            (Key::Y, Action::ToggleEditor),
        ];
        for (key, action) in map {
            if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(cmd, key))) {
                self.actions.push(action);
            }
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL | Modifiers::ALT, Key::J))
        }) {
            self.actions.push(Action::BackAttention);
        }
    }

    /// Closing the window hides the app (FR-L1); only Quit ends the panes.
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !platform::hide_app() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
    }

    /// Folders dropped on the window become projects (FR-P1).
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        for path in dropped {
            let dir = if path.is_dir() {
                path
            } else {
                match path.parent() {
                    Some(p) => p.to_path_buf(),
                    None => continue,
                }
            };
            self.actions.push(Action::AddProjectPath(dir));
        }
    }

    /// A notification was clicked: open its pane (FR-N4).
    fn handle_notification_clicks(&mut self) {
        let clicked: Vec<PaneId> = self
            .clicked_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for pane in clicked {
            if self.panes.contains_key(&pane) {
                self.actions.push(Action::FocusPane(pane));
            }
        }
    }

    fn handle_instance(&mut self) {
        if let Some(rx) = &self.instance_rx {
            while let Ok(args) = rx.try_recv() {
                for a in args {
                    self.actions.push(Action::OpenTarget(a));
                }
            }
        }
    }

    /// Everything that must go on while the window is hidden (FR-L1): messages from the PTYs and
    /// hooks, foreground checks, statuses, notifications, the badge, saving. eframe calls this
    /// before each frame and, while the window is hidden, whenever a repaint is requested.
    fn frame_logic(&mut self, ctx: &egui::Context) {
        self.window_focused = window_looked_at(ctx);
        self.sync_term_palette(ctx);
        self.handle_menu();
        self.handle_instance();
        self.handle_notification_clicks();
        self.process_messages(ctx);
        // The shells started at launch (those of the panes in view; the rest sleep).
        let started = self.panes.values().filter(|p| !p.dormant()).count();
        if !self.shells_ready_logged
            && started > 0
            && self.panes.values().all(|p| p.ready() || p.exit.is_some() || p.dormant())
        {
            self.shells_ready_logged = true;
            if std::env::var_os("TERNARYCODE_STARTUP_LOG").is_some()
                && let Some(d) = since_start()
            {
                eprintln!("{started} shells ready after {} ms", d.as_millis());
            }
        }
        #[cfg(debug_assertions)]
        self.run_devtest(ctx);
        for action in std::mem::take(&mut self.actions) {
            self.dispatch(ctx, action);
        }
        self.update_attention();
        self.take_run_snapshots();
        self.lsp_tick(ctx);
        if let Some(d) = self.lsp_repaint_after.take() {
            ctx.request_repaint_after(d);
        }
        self.release_editor_memory(ctx);
        self.enforce_scrollback_budget();
        self.schedule_wakeups(ctx);
    }

    /// Terminals answer colour queries (OSC 4/10/11) with the scheme in use: programs that ask
    /// after a theme switch, or after the system turned dark under "System", get the new one.
    fn sync_term_palette(&mut self, ctx: &egui::Context) {
        let scheme = self.scheme(ctx.global_style().visuals.dark_mode);
        if self.term_scheme != scheme.name {
            self.term_scheme = scheme.name;
            for p in self.panes.values() {
                *p.shared.palette.lock() = scheme.into();
            }
        }
    }

    /// When an agent appears in a pane, the working tree is recorded (off the UI thread, outside
    /// the user's index) so its changes can be reviewed later.
    fn take_run_snapshots(&mut self) {
        let mut todo = Vec::new();
        for pane in self.panes.values_mut() {
            match pane.tracker.agent {
                None => pane.snapshot_armed = true,
                Some(agent) if pane.snapshot_armed && !pane.snapshot_pending => {
                    pane.snapshot_armed = false;
                    pane.snapshot_pending = true;
                    todo.push((pane.id, pane.project, agent));
                }
                _ => {}
            }
        }
        for (pane, project, agent) in todo {
            let Some(folder) = self.project(project).map(|p| p.path.clone()) else {
                if let Some(p) = self.panes.get_mut(&pane) {
                    p.snapshot_pending = false;
                }
                continue;
            };
            let tx = self.app_tx.clone();
            // Finding the repository runs git too: all of it off the UI thread.
            self.rt.spawn_blocking(move || {
                let result = match tc_git::repo_root(&folder) {
                    Some(root) => tc_git::snapshot(&root),
                    None => Err("not a git repository".into()),
                };
                let _ = tx.send(AppMsg::Snapshot(pane, agent, result));
            });
        }
    }

    /// Keeps all panes' scrollback within the budget (NFR "Memori"): panes out of view lose
    /// their oldest lines first, the least recently active first. A pane back on screen may
    /// keep the configured length again.
    fn enforce_scrollback_budget(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.budget_checked) < Duration::from_secs(10) {
            return;
        }
        self.budget_checked = now;
        let blink = self.settings.cursor_blink;
        let configured = self.settings.scrollback;
        // On screen in the last frame (a Grid tile too), not only the open project's tab.
        let shown = |p: &Pane| p.shown.load(std::sync::atomic::Ordering::Relaxed) != crate::pane::Shown::Hidden as u8;
        for pane in self.panes.values_mut().filter(|p| shown(p) && p.scrollback_limit < configured) {
            pane.scrollback_limit = configured;
            tc_term::core::limit_scrollback(&pane.shared, configured, tc_term::prefs::CursorStyle::Block, blink);
        }
        let budget = self.settings.scrollback_budget_mb.max(16) * 1024 * 1024;
        let mut total: usize = self
            .panes
            .values()
            .map(|p| tc_term::core::scrollback_bytes(&p.shared))
            .sum();
        if total <= budget {
            return;
        }
        let mut order: Vec<(Instant, PaneId)> = self
            .panes
            .values()
            .filter(|p| !shown(p))
            .map(|p| (p.last_output, p.id))
            .collect();
        order.sort();
        for (_, id) in order {
            if total <= budget {
                break;
            }
            let Some(pane) = self.panes.get_mut(&id) else { continue };
            let before = tc_term::core::scrollback_bytes(&pane.shared);
            let lines = (pane.scrollback_limit / 2).max(1000);
            if lines >= pane.scrollback_limit {
                continue;
            }
            pane.scrollback_limit = lines;
            tc_term::core::limit_scrollback(&pane.shared, lines, tc_term::prefs::CursorStyle::Block, blink);
            total = total + tc_term::core::scrollback_bytes(&pane.shared) - before;
        }
    }

    fn schedule_wakeups(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let mut resize_due: Option<Instant> = None;
        for pane in self.panes.values_mut() {
            if let Some(d) = pane.apply_resize(now) {
                resize_due = Some(resize_due.map_or(d, |x| x.min(d)));
            }
        }
        if let Some(d) = resize_due {
            ctx.request_repaint_after(d.saturating_duration_since(now));
        }
        let due: Vec<ProjectId> = self.git_due.iter().filter(|(_, t)| **t <= now).map(|(id, _)| *id).collect();
        for id in due {
            self.git_due.remove(&id);
            self.refresh_git(id);
        }
        if let Some(t) = self.git_due.values().min() {
            ctx.request_repaint_after(t.saturating_duration_since(now));
        }
        if let Some(due) = self.panes.values().filter_map(|p| p.fg_due).min() {
            ctx.request_repaint_after(due.saturating_duration_since(now));
        }
        // A write that failed (disk full, folder gone) is tried again a little later.
        for path in self.saver.take_failed() {
            if path == paths::workspace_file() {
                self.saved_workspace.clear();
            } else if path == paths::settings_file() {
                self.settings_dirty = true;
            }
            self.save_due.get_or_insert(now + Duration::from_secs(5));
        }
        if let Some(due) = self.save_due {
            if now >= due {
                self.save_now();
            } else {
                ctx.request_repaint_after(due - now);
            }
        }
    }

    fn frame_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        #[cfg(debug_assertions)]
        crate::devtest::begin_frame();
        static DEBUG_FRAMES: std::sync::LazyLock<bool> =
            std::sync::LazyLock::new(|| std::env::var_os("TERNARYCODE_DEBUG_FRAMES").is_some());
        if *DEBUG_FRAMES {
            let (events, wants) = ctx.input(|i| (i.raw.events.len(), i.pointer.has_pointer()));
            eprintln!(
                "frame at {:?}: {events} events, pointer {wants}, cause {:?}",
                since_start().unwrap_or_default(),
                ctx.repaint_causes()
            );
        }
        self.window_focused = window_looked_at(&ctx);
        self.remember_window(&ctx);
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);
        self.handle_close_request(&ctx);
        if self.first_frame
            && std::env::var_os("TERNARYCODE_STARTUP_LOG").is_some()
            && let Some(d) = since_start()
        {
            eprintln!("first frame after {} ms", d.as_millis());
        }
        #[cfg(debug_assertions)]
        self.devtest_screenshot(&ctx);
        for action in std::mem::take(&mut self.actions) {
            self.dispatch(&ctx, action);
        }

        #[cfg(debug_assertions)]
        let drawing = Instant::now();
        self.tabs_hot = false;
        // Where panes are is learned again as they are drawn (a pane not drawn has no place:
        // its find bar closes instead of floating over another view).
        self.pane_rects.clear();
        // A drag the view it began in no longer sees (⌘0 or ⌘B while dragging) ends with the
        // button: otherwise its ghost would follow the pointer, repainting, until a later click.
        if !ctx.input(|i| i.pointer.any_down() || i.pointer.any_released()) {
            self.tab_drag = None;
            self.pane_drag = None;
            self.sidebar_drag = None;
        }
        self.draw(ui);
        // The title bar region moves the window, except over the tabs (they can be dragged).
        let movable = !self.tabs_hot && self.tab_drag.is_none() && self.pane_drag.is_none();
        if self.window_movable != Some(movable) {
            platform::set_window_movable(movable);
            self.window_movable = Some(movable);
        }
        #[cfg(debug_assertions)]
        crate::devtest_run::frame_drawn(&ctx, drawing.elapsed());
        // Panes say how they were drawn as they are drawn; the reader threads see it once the
        // frame is done (so a read during the frame still sees the last one).
        for (id, p) in self.panes.iter_mut() {
            let how = self.shown_now.get(id).copied().unwrap_or(crate::pane::Shown::Hidden);
            p.shown.store(how as u8, std::sync::atomic::Ordering::Relaxed);
            // Not drawn any more (its tab or project left) while it had the keyboard: a program
            // that asked for focus reports hears that it lost focus.
            if how == crate::pane::Shown::Hidden {
                let alive = p.alive();
                let out = p.view.blur(&p.shared, alive);
                p.write(out);
            }
        }
        self.shown_now.clear();

        for action in std::mem::take(&mut self.actions) {
            self.dispatch(&ctx, action);
        }
        // Focus changes made while drawing count at once (no notification for a pane just
        // looked at).
        self.update_attention();
        self.schedule_wakeups(&ctx);
    }
}

/// Someone may be looking at the window: it has the keyboard and is on screen (not hidden,
/// minimized or covered). Asked of the window itself, not told by frames drawn: an idle window
/// draws none, and the logic also runs while it is hidden (FR-L1).
fn window_looked_at(ctx: &egui::Context) -> bool {
    // The viewport's own facts first: while hidden, the rest of the input is the last frame's.
    ctx.input(|i| i.raw.focused && i.viewport().focused != Some(false) && i.viewport().visible() != Some(false))
}

/// How long the next try waits after `n` frames in a row panicked: none for a one-off, then
/// longer each time, up to two seconds (a bug hit on every frame must not spin the CPU).
fn panic_backoff(n: u32) -> Duration {
    match n {
        0..=2 => Duration::ZERO,
        _ => Duration::from_millis(50 << (n - 3).min(6)).min(Duration::from_secs(2)),
    }
}

pub fn apply_fonts(ctx: &egui::Context, settings: &Settings) {
    let choice = tc_term::prefs::FontChoice {
        name: settings.font_name.clone(),
        path: settings.font_path.clone(),
        index: 0,
    };
    tc_term::fonts::apply(ctx, &tc_term::fonts::effective_choice(&choice), false);
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.frame_logic(ctx)));
        if result.is_err() {
            self.logic_panics = self.logic_panics.saturating_add(1);
            ctx.request_repaint_after(panic_backoff(self.logic_panics));
        } else {
            self.logic_panics = 0;
        }
    }

    #[cfg(debug_assertions)]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        raw_input.events.extend(crate::devtest::take_injected());
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        // A bug while drawing must not take every pane down: the panic hook wrote a report; the
        // frame is skipped and the app carries on.
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.frame_ui(ui)));
        if result.is_err() {
            self.toast(tf(
                "Something went wrong and TernaryCode recovered. A report was saved in {}.",
                &[&paths::contract_tilde(&paths::logs_dir())],
            ));
            self.ui_panics = self.ui_panics.saturating_add(1);
            ui.ctx().request_repaint_after(panic_backoff(self.ui_panics));
        } else {
            self.ui_panics = 0;
        }
        self.first_frame = false;
    }

    #[cfg(target_os = "macos")]
    fn on_exit(&mut self) {
        self.save_and_wait();
        // Language servers too, whichever way the app ends (they would outlive it).
        self.lsp_stop_all();
        for pane in self.panes.values_mut() {
            pane.hang_up();
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_and_wait();
        self.lsp_stop_all();
        for pane in self.panes.values_mut() {
            pane.hang_up();
        }
    }
}

/// A path given on the command line, made absolute (the running instance has another
/// directory).
pub fn absolute(arg: &str) -> PathBuf {
    let p = paths::expand_tilde(arg);
    let p = if p.is_absolute() {
        p
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    };
    paths::canonical(&p)
}
