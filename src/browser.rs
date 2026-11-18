//! Browser panes: a web page beside the terminals. A browser pane is a pane like a terminal: it
//! belongs to a project, lives in a tab or a split, and is saved with the workspace (its
//! address). Its page is the system's web view (WebKit on macOS), laid over the window where
//! the pane is drawn. Out of view, or under something drawn over it (a dialog, the palette, a
//! menu), it is hidden but keeps running; after a while out of view and unused its page is let
//! go (asleep) and loaded again when it is shown. Logins are kept: one profile, on disk, for
//! every project.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tc_core::{PaneId, ProjectId};

/// What a page reports, taken in by the app each frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The page is at this address (a load began or ended there).
    Url(String),
    Title(String),
    Loading(bool),
    /// The page was clicked: it has the keyboard.
    Focused,
    /// The page asked for a new window that is not a pop-up (a link with target=_blank, a
    /// script's window.open): a new page.
    OpenHere(String),
    /// Lines the page wrote to its console.
    Console(Vec<ConsoleLine>),
    /// The page opened a dialog (alert, confirm, prompt); it waits for the answer.
    Dialog(Dialog),
    /// A download began, to this file.
    DownloadStarted(String, std::path::PathBuf),
    /// The download of this address ended (or failed).
    DownloadDone(String, bool),
    /// A key of the app's, pressed while the page had the keyboard (Windows, Linux: the page's
    /// window has the keys there, not the app's).
    Key(eframe::egui::Key, eframe::egui::Modifiers),
    /// What the page does that sleeping would end, as its helper tells (each new page tells it
    /// once, then each change).
    State(PageState),
}

/// What a page does that sleeping would end (`browser_agent.js` tells it): such a page stays
/// awake out of view, and its tab shows a speaker while it plays sound.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageState {
    /// Audio or video plays, not muted by the page (the tab's mute still counts it).
    pub audible: bool,
    /// The camera, the microphone or the screen is in use.
    pub capturing: bool,
    /// Text typed in a field of a form, not sent yet.
    pub editing: bool,
    /// The tab's mute is on in the page.
    pub muted: bool,
}

/// A page's `state:` message.
pub fn page_state(json: &str) -> Option<PageState> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let f = |k: &str| v[k].as_bool().unwrap_or(false);
    Some(PageState { audible: f("a"), capturing: f("c"), editing: f("e"), muted: f("m") })
}

/// The script that turns the tab's mute on or off in a page (its media, and media to come).
pub fn mute_script(on: bool) -> String {
    format!("{AGENT_JS}\nreturn JSON.stringify(await window.__tc.run('mute', {{ on: {on} }}));")
}

/// The page's web view (wry's), and what the agents' steps act on (a page or a pop-up).
#[cfg(target_os = "macos")]
pub type View = wry::WebView;
#[cfg(target_os = "macos")]
pub type Page = objc2_web_kit::WKWebView;
#[cfg(target_os = "macos")]
pub type PageRc = objc2::rc::Retained<Page>;
#[cfg(windows)]
pub use crate::browser_win::{
    PageRc, View, back_in, bounds, build_view, clear_data, clicked_in, css_size, dim, edit_command, has_keyboard, is_loading, loading_in, page_key, print, run_js, run_js_in,
    snapshot_at, snapshot_view, turn_back, url_in, view_has_keyboard, web_pid, web_view_of,
};
#[cfg(windows)]
pub use crate::browser_win::Page;
#[cfg(target_os = "linux")]
pub use crate::browser_linux::{
    Page, PageRc, View, back_in, bounds, build_view, clear_data, clicked_in, css_size, dim, edit_command, has_keyboard, is_loading, loading_in, page_key, print, run_js, run_js_in,
    snapshot_at, snapshot_view, turn_back, url_in, view_has_keyboard, web_pid, web_view_of,
};

/// Whether pages show in TernaryCode on this machine: on macOS always; on Windows with the
/// WebView2 Runtime (part of Windows 11, and of Windows 10 with Edge); on Linux in an X11 window
/// with the page helper (`ternarycode-web`) and WebKitGTK there.
pub fn supported() -> bool {
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(windows)]
    {
        static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *OK.get_or_init(|| wry::webview_version().is_ok())
    }
    #[cfg(target_os = "linux")]
    {
        crate::browser_linux::available()
    }
}

/// Why pages do not show in TernaryCode here (`supported`).
pub const UNSUPPORTED: &str = if cfg!(windows) {
    "Pages need the Microsoft Edge WebView2 Runtime, which is not installed: get it from microsoft.com/edge/webview2, or the page opens in your default browser."
} else {
    "Pages show in TernaryCode in an X11 session with WebKitGTK 4.1 (libwebkit2gtk-4.1) installed; here they open in your default browser."
};

/// Whether a key pressed in a page is the app's (Windows, Linux): Ctrl+Shift is the app's own
/// (as ⌘ on macOS), and a browser's Ctrl+F, Ctrl+L, Ctrl+R and zoom keys act on the page as in
/// a browser.
pub fn app_key(key: eframe::egui::Key, mods: eframe::egui::Modifiers) -> bool {
    use eframe::egui::Key;
    if !mods.ctrl {
        return false;
    }
    // The page's own editing keys stay its (Ctrl+Shift+V pastes as plain text, Ctrl+Shift+Z redoes).
    if mods.shift {
        return !matches!(key, Key::C | Key::V | Key::X | Key::A | Key::Z);
    }
    !mods.alt && matches!(key, Key::F | Key::L | Key::R | Key::Equals | Key::Plus | Key::Minus | Key::Num0)
}

/// The kinds of a page's JavaScript dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    /// A message, with OK.
    Alert,
    /// A question, OK or Cancel.
    Confirm,
    /// A question with a text to give.
    Prompt,
}

/// A page's dialog waiting for an answer (`browser_dialog`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
    pub id: u64,
    pub kind: DialogKind,
    /// The site asking.
    pub host: String,
    pub message: String,
    /// What the prompt's field holds.
    pub text: String,
}

/// The find bar of a page: what is looked for, how many matches, which one is selected.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageFind {
    pub query: String,
    /// Matches in the page (`None` until the page answered).
    pub count: Option<usize>,
    /// The selected match, from 1 (0: none).
    pub index: usize,
    /// The field takes the keyboard next frame.
    pub focus: bool,
    /// The search the page is answering: 0 from the top, 1 the next match, -1 the one before.
    pub step: i32,
}

impl PageFind {
    /// Takes in the page's answer to the last search: `count` matches, one `found` (selected).
    pub fn answered(&mut self, count: usize, found: bool) {
        self.count = Some(count);
        self.index = match (found, count) {
            (false, _) | (_, 0) => 0,
            _ if self.step == 0 || self.index == 0 => 1,
            _ if self.step > 0 => self.index % count + 1,
            _ => (self.index + count - 2) % count + 1,
        };
    }
}

/// The zoom steps of a page (as Safari's).
pub const ZOOMS: [f32; 13] = [0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

/// The zoom one step in (`dir` 1) or out (-1) from `z`.
pub fn zoom_step(z: f32, dir: i32) -> f32 {
    if dir > 0 {
        ZOOMS.iter().copied().find(|s| *s > z + 0.001).unwrap_or(ZOOMS[ZOOMS.len() - 1])
    } else {
        ZOOMS.iter().rev().copied().find(|s| *s < z - 0.001).unwrap_or(ZOOMS[0])
    }
}

/// The script that looks for `query` in the page: the number of matches (case-insensitive, in
/// the page's text), and the next one selected (`back`: the one before; `fresh`: from the top).
pub fn find_script(query: &str, back: bool, fresh: bool) -> String {
    let q = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".into());
    format!(
        "const q = {q}; if (!q) {{ getSelection().removeAllRanges(); return JSON.stringify([0, false]); }} const text = (document.body ? document.body.innerText : '').toLowerCase(); const n = text.split(q.toLowerCase()).length - 1; if ({fresh}) getSelection().removeAllRanges(); const found = n > 0 && window.find(q, false, {back}, true, false, true, false); return JSON.stringify([n, found]);"
    )
}

/// A file the page downloaded (or is downloading).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub url: String,
    pub path: std::path::PathBuf,
    /// Ended: whether it worked; `None` while it runs.
    pub done: Option<bool>,
    pub at: Instant,
}

/// Downloads a page keeps in its list (the oldest go first).
const DOWNLOADS: usize = 20;

/// A line of a page's console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleLine {
    /// `error`, `warn`, `info`, `log` or `debug`.
    pub level: String,
    pub text: String,
    /// The file and line it came from (`CartSummary.tsx:41`), when known.
    pub src: String,
}

impl ConsoleLine {
    pub fn is_error(&self) -> bool {
        self.level == "error"
    }

    pub fn is_warning(&self) -> bool {
        self.level == "warn"
    }
}

/// Lines a page's console keeps (the oldest go first).
const CONSOLE_LINES: usize = 500;

/// An agent using a page (its browser tools, `browser_agent`).
#[derive(Clone, Debug)]
pub struct Driver {
    /// The agent's pane.
    pub pane: PaneId,
    /// Its name ("Claude Code").
    pub who: String,
    /// What it does now ("Clicking “Add to cart” button").
    pub doing: String,
    /// When its last tool call began or ended.
    pub last: Instant,
    /// The user took the browser over: the agent's calls are answered "the user has it".
    pub paused: bool,
}

/// A site an agent's step would open, not on the project's list: the user is asked.
#[derive(Clone, Debug)]
pub struct Ask {
    pub url: String,
    pub host: String,
    /// A click leads there (else the agent asked to open it).
    pub click: bool,
    pub answer: Option<Answer>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Once,
    /// And from now on, for the project.
    Always,
    Block,
}

/// A page closed lately: where it was, to open it again (Reopen Closed Page).
#[derive(Clone, Debug)]
pub struct ClosedPage {
    pub project: ProjectId,
    pub url: String,
    pub title: String,
}

/// Closed pages kept to reopen (the oldest go first).
pub const CLOSED_PAGES: usize = 25;

/// Where a page's callbacks put what they report, and wake the window.
#[derive(Clone)]
pub struct Sink {
    events: Arc<Mutex<Vec<Event>>>,
    ctx: eframe::egui::Context,
    /// The page it reports for.
    pub page: PaneId,
}

impl Sink {
    /// The window it wakes.
    pub fn ctx(&self) -> &eframe::egui::Context {
        &self.ctx
    }

    pub fn push(&self, e: Event) {
        self.events.lock().push(e);
        self.ctx.request_repaint();
    }
}

pub struct Browser {
    pub id: PaneId,
    pub project: ProjectId,
    /// The page's address (saved with the workspace, loaded again after sleeping).
    pub url: String,
    pub title: String,
    pub loading: bool,
    /// What the address field holds.
    pub address: String,
    /// When it was last in view (or busy out of view: playing sound, a form being filled in…).
    pub last_seen: Instant,
    /// The page was let go to save memory (loaded again when shown).
    pub asleep: bool,
    /// Since when it has had no web view (asleep, or never shown yet).
    pub asleep_since: Option<Instant>,
    /// What the page does that sleeping would end (sound, camera, a form being filled in).
    pub state: PageState,
    /// The user muted it from its tab (kept through loads and sleep).
    pub muted: bool,
    /// The agent pane this browser was opened for or taken by: its tools act here.
    pub owner: Option<PaneId>,
    /// An agent using the page now.
    pub driver: Option<Driver>,
    /// A site waiting for the user's OK.
    pub ask: Option<Ask>,
    /// A host the user let the agent open once: its next load is not stopped.
    pub pass: Option<String>,
    /// The user handed the page back to its agent: its next call is told where it is.
    pub handed_back: bool,
    /// What the page wrote to its console since it loaded.
    pub console: std::collections::VecDeque<ConsoleLine>,
    /// The console drawer is open.
    pub console_open: bool,
    /// A dialog of the page waiting for an answer.
    pub dialog: Option<Dialog>,
    /// What the page downloaded, the newest last.
    pub downloads: Vec<Download>,
    /// The list of downloads is open.
    pub downloads_open: bool,
    /// The find bar (⌘F), when open.
    pub find: Option<PageFind>,
    /// What the page's last search found: (matches, found one), filled by the page's answer.
    pub(crate) find_result: Arc<Mutex<Option<(usize, bool)>>>,
    /// The zoom the user chose for the page (⌘= / ⌘−), on top of `zoom_want`.
    pub user_zoom: f32,
    /// Pictures of the page, decoded and waiting to become a texture.
    pub(crate) picture: Picture,
    /// When a picture was asked for (one at a time).
    pub(crate) picture_asked: Option<Instant>,
    /// The page's last picture, and when it came: shown in its place while something covers
    /// it, and small in the Grid while an agent uses it.
    pub still: Option<(eframe::egui::TextureHandle, Instant)>,
    /// Since when something is drawn over the page (its picture shows meanwhile).
    pub(crate) covered_since: Option<Instant>,
    /// Its server (on this machine) does not answer: the pane says so instead of a blank page.
    pub down: bool,
    /// The terminal last seen serving the page.
    pub served_by: Option<PaneId>,
    /// Whether the server answered, from the last check (filled by another thread).
    probe: Arc<Mutex<Option<bool>>>,
    probing: bool,
    /// When to check again while it is down.
    probe_due: Option<Instant>,
    /// The page was loading in the last frame (a load that ends is checked).
    was_loading: bool,
    events: Arc<Mutex<Vec<Event>>>,
    pub(crate) view: Option<View>,
    /// Drawn (shown on screen) after the last frame.
    pub(crate) shown: bool,
    /// Where its web view was last laid (moved only when that changes).
    pub(crate) placed: Option<eframe::egui::Rect>,
    /// Laid outside the window: out of view while an agent uses it (it keeps drawing, so its
    /// screenshots and timers work).
    pub(crate) offscreen: bool,
    /// Dimmed under a question.
    pub(crate) dimmed: bool,
    /// The zoom its page is drawn at (1: as in Focus; smaller in a Grid tile, which keeps the
    /// layout it has in Focus), and the zoom where it is drawn this frame.
    pub(crate) zoom: f32,
    pub zoom_want: f32,
}

impl Browser {
    /// A browser pane at `url` ("" for none yet): asleep until it is first drawn.
    pub fn new(id: PaneId, project: ProjectId, url: &str) -> Self {
        Self {
            id,
            project,
            url: url.to_string(),
            title: String::new(),
            loading: false,
            address: url.to_string(),
            last_seen: Instant::now(),
            asleep: true,
            asleep_since: Some(Instant::now()),
            state: PageState::default(),
            muted: false,
            owner: None,
            driver: None,
            ask: None,
            pass: None,
            handed_back: false,
            console: Default::default(),
            console_open: false,
            dialog: None,
            downloads: Vec::new(),
            downloads_open: false,
            find: None,
            find_result: Arc::default(),
            user_zoom: 1.0,
            picture: Arc::default(),
            picture_asked: None,
            still: None,
            covered_since: None,
            down: false,
            served_by: None,
            probe: Arc::default(),
            probing: false,
            probe_due: None,
            was_loading: false,
            events: Arc::default(),
            view: None,
            shown: false,
            placed: None,
            offscreen: false,
            dimmed: false,
            zoom: 1.0,
            zoom_want: 1.0,
        }
    }

    /// Asks for a picture of the page (`width` points wide, else as it shows), unless one is on
    /// its way.
    pub fn ask_picture(&mut self, ctx: &eframe::egui::Context, width: Option<f64>) {
        let Some(v) = &self.view else { return };
        if self.picture_asked.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            return;
        }
        self.picture_asked = Some(Instant::now());
        take_picture(v, width, self.picture.clone(), ctx.clone());
    }

    /// Takes in a picture that came: it becomes the page's still.
    pub fn pick_up_picture(&mut self, ctx: &eframe::egui::Context) {
        let Some(rgba) = self.picture.lock().take() else { return };
        self.picture_asked = None;
        let image = eframe::egui::ColorImage::from_rgba_unmultiplied([rgba.width as usize, rgba.height as usize], &rgba.pixels);
        match &mut self.still {
            Some((tex, at)) => {
                tex.set(image, eframe::egui::TextureOptions::LINEAR);
                *at = Instant::now();
            }
            None => self.still = Some((ctx.load_texture(format!("browser-still-{}", self.id), image, eframe::egui::TextureOptions::LINEAR), Instant::now())),
        }
    }

    /// Follows whether the page's server (on this machine) answers: checked when a load ends,
    /// and every 2 s while it does not; once it answers again, the page loads again.
    pub fn watch_server(&mut self, loading: bool, ctx: &eframe::egui::Context) {
        let ended = self.was_loading && !loading;
        self.was_loading = loading;
        if let Some(up) = self.probe.lock().take() {
            self.probing = false;
            if up {
                if self.down {
                    self.down = false;
                    self.reload();
                }
            } else {
                self.down = true;
                self.probe_due = Some(Instant::now() + Duration::from_secs(2));
            }
        }
        let due = self.down && self.probe_due.is_some_and(|t| Instant::now() >= t);
        if (ended || due) && !self.probing {
            match local_server(&self.url) {
                Some(addr) => {
                    self.probing = true;
                    self.probe_due = None;
                    let (slot, ctx) = (self.probe.clone(), ctx.clone());
                    let _ = std::thread::Builder::new().name("browser-probe".into()).spawn(move || {
                        let up = answers(&addr);
                        *slot.lock() = Some(up);
                        ctx.request_repaint();
                    });
                }
                None => self.down = false,
            }
        }
        if self.down && !self.probing {
            ctx.request_repaint_after(Duration::from_secs(2));
        }
    }

    /// An agent uses it now (the user has not taken it over).
    pub fn driven(&self) -> bool {
        self.driver.as_ref().is_some_and(|d| !d.paused)
    }

    pub fn sink(&self, ctx: &eframe::egui::Context) -> Sink {
        Sink { events: self.events.clone(), ctx: ctx.clone(), page: self.id }
    }

    /// Its name in a tab: the page's title, else its host.
    pub fn label(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.trim().to_string();
        }
        let (host, _) = split_url(&self.url);
        if host.is_empty() { crate::i18n::t("Browser").to_string() } else { host }
    }

    /// Takes in what the page reported; returns whether anything changed.
    pub fn take_events(&mut self) -> Vec<Event> {
        let events = std::mem::take(&mut *self.events.lock());
        for e in &events {
            match e {
                Event::Url(u) if !u.is_empty() && u != "about:blank" => {
                    self.url = u.clone();
                }
                // A new page: its console starts empty (as a browser's does).
                Event::Loading(true) => {
                    self.loading = true;
                    self.console.clear();
                }
                Event::Console(lines) => {
                    self.console.extend(lines.iter().cloned());
                    while self.console.len() > CONSOLE_LINES {
                        self.console.pop_front();
                    }
                }
                Event::Title(t) => self.title = t.clone(),
                Event::Loading(l) => self.loading = *l,
                Event::State(st) => {
                    self.state = *st;
                    // A new page (or one loaded again) gets the tab's mute.
                    if st.muted != self.muted
                        && let Some(v) = &self.view
                    {
                        run_js(v, &mute_script(self.muted), |_| {});
                    }
                }
                _ => {}
            }
        }
        events
    }

    /// Mutes the page from its tab, or lets it play sound again.
    pub fn set_muted(&mut self, on: bool) {
        self.muted = on;
        if let Some(v) = &self.view {
            run_js(v, &mute_script(on), |_| {});
        }
    }

    /// The page does something sleeping would end: it plays sound, uses the camera, microphone
    /// or screen, has a form being filled in, waits for a dialog's answer, or downloads. (A
    /// pop-up it opened counts too: `App::page_busy`.)
    pub fn busy(&self) -> bool {
        self.state.audible || self.state.capturing || self.state.editing || self.dialog.is_some() || self.downloads.iter().any(|d| d.done.is_none())
    }

    /// Goes to `address` (as typed: see [`normalize`]).
    pub fn navigate(&mut self, address: &str) {
        let url = normalize(address);
        if url.is_empty() {
            return;
        }
        self.url = url.clone();
        self.address = url.clone();
        self.title.clear();
        if let Some(v) = &self.view {
            let _ = v.load_url(&url);
        }
    }

    pub fn back(&self) {
        if let Some(v) = &self.view {
            let _ = v.go_back();
        }
    }

    pub fn forward(&self) {
        if let Some(v) = &self.view {
            let _ = v.go_forward();
        }
    }

    /// Loads the page again; a view that never got its page (its server did not answer) loads
    /// the address.
    pub fn reload(&self) {
        if let Some(v) = &self.view {
            // The view shows the address (not the page it was on when a load of it failed).
            let shows_it = v.url().is_ok_and(|u| u.trim_end_matches('/') == self.url.trim_end_matches('/'));
            if shows_it || self.url.is_empty() {
                let _ = v.reload();
            } else {
                let _ = v.load_url(&self.url);
            }
        }
    }

    /// Notes a download that began, or ended.
    pub fn note_download(&mut self, e: &Event) {
        match e {
            Event::DownloadStarted(url, path) => {
                self.downloads.push(Download { url: url.clone(), path: path.clone(), done: None, at: Instant::now() });
                if self.downloads.len() > DOWNLOADS {
                    self.downloads.remove(0);
                }
            }
            Event::DownloadDone(url, ok) => {
                if let Some(d) = self.downloads.iter_mut().rev().find(|d| &d.url == url && d.done.is_none()) {
                    d.done = Some(*ok);
                }
            }
            _ => {}
        }
    }

    /// Lets the page go (memory); it loads again when shown.
    pub fn sleep(&mut self) {
        self.view = None;
        self.placed = None;
        self.shown = false;
        self.offscreen = false;
        self.dimmed = false;
        self.still = None;
        self.covered_since = None;
        self.asleep = true;
        self.asleep_since = Some(Instant::now());
        self.state = PageState::default();
        self.loading = false;
        self.zoom = 1.0;
        // Its dialog went with the page (answered Cancel, `browser_dialog::tidy`).
        self.dialog = None;
    }

    /// The page has a web view (it is not asleep).
    pub fn live(&self) -> bool {
        self.view.is_some()
    }
}

/// How long a browser out of view and unused keeps its page.
pub fn sleep_after(minutes: u32) -> Duration {
    // Tests wait seconds, not minutes.
    #[cfg(debug_assertions)]
    if let Some(secs) = std::env::var("TERNARYCODE_SLEEP_SECS").ok().and_then(|s| s.parse::<u64>().ok()) {
        return Duration::from_secs(secs);
    }
    Duration::from_secs(u64::from(minutes.max(1)) * 60)
}

/// How long pages out of view and unused stay awake while memory runs low.
pub const PRESSURE_SLEEP: Duration = Duration::from_secs(60);

/// How long an agent's page stays asleep after its agent ended before it closes.
pub fn agent_page_keep() -> Duration {
    #[cfg(debug_assertions)]
    if let Some(secs) = std::env::var("TERNARYCODE_AGENT_PAGE_SECS").ok().and_then(|s| s.parse::<u64>().ok()) {
        return Duration::from_secs(secs);
    }
    Duration::from_secs(60 * 60)
}

/// The host (with its port) and the rest of an address, for the address field. The host ends
/// where WebKit ends it: at `/`, `?`, `#` or `\` (so `evil.example#@localhost` is not at
/// localhost).
pub fn split_url(url: &str) -> (String, String) {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    match rest.find(['/', '?', '#', '\\']) {
        Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
        None => (rest.to_string(), String::new()),
    }
}

/// The host of an address, without its port ("" for none).
pub fn host_of(url: &str) -> String {
    let (host, _) = split_url(url);
    let host = host.rsplit_once('@').map_or(host.as_str(), |(_, h)| h).to_string();
    match host.strip_prefix('[') {
        Some(v6) => format!("[{}]", v6.split(']').next().unwrap_or("")),
        None => host.split(':').next().unwrap_or("").to_ascii_lowercase(),
    }
}

/// Whether `host` is on a list of sites: `example.com` (or `*.example.com`) takes that host and
/// its subdomains; `*` takes every site.
pub fn site_listed(host: &str, sites: &[String]) -> bool {
    sites.iter().any(|s| {
        if s.trim() == "*" {
            return true;
        }
        let s = s.trim().trim_start_matches("*.").to_ascii_lowercase();
        !s.is_empty() && (host == s || host.ends_with(&format!(".{s}")))
    })
}

/// Whether an agent may open `url` without asking: local files and blank pages, the sites on
/// the lists (every project's, from Settings, and this project's own: `sites`), and `pass` (a
/// host the user let it open once).
pub fn agent_may_open(url: &str, sites: &[String], pass: Option<&str>) -> bool {
    let scheme = url.split_once(':').map_or("", |(s, _)| s).to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https") {
        return matches!(scheme.as_str(), "about" | "file" | "data" | "blob");
    }
    let host = host_of(url);
    pass == Some(host.as_str()) || site_listed(&host, sites)
}

/// A server on this machine an address is at (host, port), to check whether it answers.
fn local_server(url: &str) -> Option<(String, u16)> {
    let scheme = url.split_once("://")?.0.to_ascii_lowercase();
    let (hostport, _) = split_url(url);
    let host = host_of(url);
    if !local_host(&host) || host.ends_with(".local") {
        return None;
    }
    let port = hostport.rsplit_once(':').and_then(|(_, p)| p.parse().ok()).unwrap_or(if scheme == "https" { 443 } else { 80 });
    Some((host.trim_start_matches('[').trim_end_matches(']').to_string(), port))
}

/// Whether something listens at (host, port) on this machine.
fn answers((host, port): &(String, u16)) -> bool {
    use std::net::ToSocketAddrs;
    let host = if host == "0.0.0.0" { "127.0.0.1" } else { host.as_str() };
    (host, *port).to_socket_addrs().is_ok_and(|mut addrs| addrs.any(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_millis(400)).is_ok()))
}

/// Whether an address is on this machine (a dev server): http, not https, when typed bare.
fn local_host(host: &str) -> bool {
    if let Some(v6) = host.strip_prefix('[') {
        return v6.starts_with("::1]");
    }
    let host = host.split(':').next().unwrap_or(host);
    matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]") || host.ends_with(".local") || host.ends_with(".localhost")
}

/// What was typed into the address field, as an address: with a scheme (http for this
/// machine's servers, https otherwise), or a web search for words.
pub fn normalize(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return String::new();
    }
    if s.contains("://") || s.starts_with("about:") || s.starts_with("data:") {
        return s.to_string();
    }
    let (host, _) = split_url(s);
    if local_host(&host) || host.parse::<std::net::Ipv4Addr>().is_ok() || host.split(':').next().is_some_and(|h| h.parse::<std::net::Ipv4Addr>().is_ok()) {
        return format!("http://{s}");
    }
    // A port alone (":3000") or a bare number: this machine.
    if let Some(port) = s.strip_prefix(':').or(Some(s)).filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) {
        return format!("http://localhost:{port}");
    }
    if !s.contains(' ') && host.contains('.') {
        return format!("https://{s}");
    }
    let q: String = s
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("https://www.google.com/search?q={q}")
}

/// A local file as an address.
pub fn file_url(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    let encoded: String = s
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("file://{encoded}")
}

/// Reports the user's click in the page (it takes the keyboard): the app then lets go of its
/// own keyboard focus. An agent's clicks are not the user's (they are not trusted events). On
/// Linux the page helper tells of the user's clicks itself.
#[cfg(not(target_os = "linux"))]
pub(crate) const FOCUS_SCRIPT: &str = "window.addEventListener('pointerdown', function (e) { if (!e.isTrusted) return; try { window.ipc.postMessage('focus'); } catch (x) {} }, true);";

/// The agents' helper in the page (`browser_agent.js`), from the start of every page so that
/// it hears the console from the first line.
pub const AGENT_JS: &str = include_str!("browser_agent.js");

/// Safari's own name: some sites refuse a web view that does not say it is Safari (Google's
/// sign-in among them).
#[cfg(target_os = "macos")]
pub(crate) const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.4 Safari/605.1.15";

/// The page as a web view laid over the window at `bounds` (logical points from the top left of
/// the window's content).
#[cfg(target_os = "macos")]
pub fn build_view(
    window: &impl raw_window_handle::HasWindowHandle,
    url: &str,
    bounds: wry::Rect,
    sink: Sink,
) -> Option<wry::WebView> {
    use wry::{NewWindowResponse, PageLoadEvent, PermissionResponse, WebViewBuilder};
    let (s1, s2, s3, s4, s5, s6) = (sink.clone(), sink.clone(), sink.clone(), sink.clone(), sink.clone(), sink.clone());
    let url = if url.is_empty() { "about:blank" } else { url };
    watch_clicks();
    let view = WebViewBuilder::new()
        .with_url(url)
        .with_bounds(bounds)
        .with_user_agent(USER_AGENT)
        .with_accept_first_mouse(true)
        .with_back_forward_navigation_gestures(true)
        .with_initialization_script(FOCUS_SCRIPT)
        .with_initialization_script(AGENT_JS)
        .with_document_title_changed_handler(move |t| s1.push(Event::Title(t)))
        .with_on_page_load_handler(move |e, u| {
            s2.push(Event::Url(u));
            s2.push(Event::Loading(matches!(e, PageLoadEvent::Started)));
        })
        .with_new_window_req_handler(move |u, f| {
            // A window of a size, opened by a click in the page (Continue with Google, Sign in
            // with Apple), the user's or an agent's: a pop-up tied to the page, which hears back
            // from it through `window.opener`. Anything else (a link to a new tab): a new page in
            // the panel.
            let sized = f.size.is_some() || f.position.is_some();
            let size = f.size.map(|s| (s.width, s.height));
            match crate::browser_popup::request(&f.opener.webview, &u, &f.opener.target_configuration, sized, size, &s3) {
                Some(Some(webview)) => NewWindowResponse::Create { webview },
                Some(None) => NewWindowResponse::Deny,
                None => {
                    s3.push(Event::OpenHere(u));
                    NewWindowResponse::Deny
                }
            }
        })
        .with_ipc_handler(move |req| {
            let body = req.body();
            if body == "focus" {
                s4.push(Event::Focused);
            } else if let Some(json) = body.strip_prefix("console:") {
                let lines: Vec<(String, String, String)> = serde_json::from_str(json).unwrap_or_default();
                let lines: Vec<ConsoleLine> = lines.into_iter().map(|(level, text, src)| ConsoleLine { level, text, src }).collect();
                if !lines.is_empty() {
                    s4.push(Event::Console(lines));
                }
            } else if let Some(state) = body.strip_prefix("state:").and_then(page_state) {
                s4.push(Event::State(state));
            }
        })
        // Camera and microphone: WebKit asks the user, per site (never granted unasked).
        .with_permission_handler(|_| PermissionResponse::Default)
        // Web Inspector (Inspect Element in the page's menu, or ⋯ → Inspect Element).
        .with_devtools(true)
        // Downloads go to the Downloads folder (a name taken gets a number); the page lists them.
        .with_download_started_handler(move |url, path| {
            // Tests keep the user's Downloads folder out of it.
            #[cfg(debug_assertions)]
            if let (Some(dir), Some(name)) = (std::env::var_os("TERNARYCODE_DOWNLOAD_DIR"), path.file_name().map(|n| n.to_owned())) {
                *path = std::path::PathBuf::from(dir).join(name);
            }
            s5.push(Event::DownloadStarted(url, path.clone()));
            true
        })
        .with_download_completed_handler(move |url, _, ok| s6.push(Event::DownloadDone(url, ok)))
        .build_as_child(window)
        .ok()?;
    let wk = web_view_of(&view);
    crate::browser_dialog::install(&wk);
    crate::browser_dialog::register(&wk, sink);
    Some(view)
}

/// A rectangle in egui points as the web view's bounds.
#[cfg(target_os = "macos")]
pub fn bounds(rect: eframe::egui::Rect, zoom: f32) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(f64::from(rect.left() * zoom), f64::from(rect.top() * zoom)).into(),
        size: wry::dpi::LogicalSize::new(f64::from(rect.width() * zoom), f64::from(rect.height() * zoom)).into(),
    }
}

/// Sends an editing command (copy, paste, cut, select all) to the page: the app's menu has the
/// keys, the page has the text.
#[cfg(target_os = "macos")]
pub fn edit_command(view: &wry::WebView, command: &str) {
    use objc2::runtime::AnyObject;
    use wry::WebViewExtMacOS;
    let wk = view.webview();
    let none: *const AnyObject = std::ptr::null();
    // SAFETY: WKWebView answers the standard editing actions (NSResponder), each taking a
    // sender and returning nothing.
    unsafe {
        match command {
            "copy" => objc2::msg_send![&*wk, copy: none],
            "paste" => objc2::msg_send![&*wk, paste: none],
            "cut" => objc2::msg_send![&*wk, cut: none],
            "selectAll" => objc2::msg_send![&*wk, selectAll: none],
            _ => {}
        }
    }
}

/// The page as it shows now, as a PNG, handed to `done` (on the main thread, once).
#[cfg(target_os = "macos")]
pub fn snapshot(view: &wry::WebView, done: impl FnOnce(Option<Vec<u8>>) + 'static) {
    snapshot_at(view, None, done);
}

/// [`snapshot`], `width` points wide (a smaller picture costs less to make).
#[cfg(target_os = "macos")]
pub fn snapshot_at(view: &wry::WebView, width: Option<f64>, done: impl FnOnce(Option<Vec<u8>>) + 'static) {
    use wry::WebViewExtMacOS;
    snapshot_view(&view.webview(), width, done);
}

/// [`snapshot_at`] of a web view.
#[cfg(target_os = "macos")]
pub fn snapshot_view(wk: &objc2_web_kit::WKWebView, width: Option<f64>, done: impl FnOnce(Option<Vec<u8>>) + 'static) {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError};
    let done = std::cell::RefCell::new(Some(done));
    let block = block2::RcBlock::new(move |image: *mut NSImage, _error: *mut NSError| {
        // SAFETY: WebKit hands a valid image (or null) for the length of the call.
        let png = unsafe { image.as_ref() }.and_then(|img| {
            let tiff = img.TIFFRepresentation()?;
            let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
            let props = NSDictionary::new();
            // SAFETY: PNG with no properties is a valid request of a bitmap representation.
            let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }?;
            Some(data.to_vec())
        });
        if let Some(f) = done.borrow_mut().take() {
            f(png);
        }
    });
    let config = width.zip(objc2::MainThreadMarker::new()).map(|(w, mtm)| {
        // SAFETY: a new configuration, on the main thread.
        unsafe {
            let c = objc2_web_kit::WKSnapshotConfiguration::new(mtm);
            c.setSnapshotWidth(Some(&objc2_foundation::NSNumber::new_f64(w)));
            c
        }
    });
    // SAFETY: a snapshot of the whole visible page; the block lives as long as WebKit keeps it.
    unsafe { wk.takeSnapshotWithConfiguration_completionHandler(config.as_deref(), &block) };
}

/// A picture of a page, decoded (off the main thread) for egui.
pub type Picture = Arc<Mutex<Option<tc_doc::Rgba>>>;

/// Takes a picture of the page into `slot` (`width` points wide, else as it shows): the
/// snapshot on the main thread, decoding on another; the window is woken when it is there.
pub fn take_picture(view: &View, width: Option<f64>, slot: Picture, ctx: eframe::egui::Context) {
    snapshot_at(view, width, move |png| {
        let Some(png) = png else { return };
        let _ = std::thread::Builder::new().name("browser-picture".into()).spawn(move || {
            if let Ok(rgba) = tc_doc::img::decode(&png, 2400) {
                *slot.lock() = Some(rgba);
                ctx.request_repaint();
            }
        });
    });
}

/// Runs `body` (the body of an async function) in the page and hands what it returns, as a
/// string, to `done` (on the main thread, once). An error: the script failed to run, or the
/// page went away meanwhile (it navigated).
#[cfg(target_os = "macos")]
pub fn run_js(view: &wry::WebView, body: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    use wry::WebViewExtMacOS;
    run_js_in(&view.webview(), body, done);
}

/// [`run_js`] in a web view (a page's, or a pop-up's).
#[cfg(target_os = "macos")]
pub fn run_js_in(wk: &objc2_web_kit::WKWebView, body: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSError, NSString};
    use objc2_web_kit::WKContentWorld;
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return done(Err("not on the main thread".into()));
    };
    let done = std::cell::RefCell::new(Some(done));
    let block = block2::RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
        // SAFETY: WebKit hands a valid object (or null) for each, for the length of the call.
        let out = match unsafe { error.as_ref() } {
            Some(e) => Err(e.localizedDescription().to_string()),
            None => unsafe { result.as_ref() }
                .and_then(|r| r.downcast_ref::<NSString>())
                .map(|s| s.to_string())
                .ok_or_else(|| "the page returned nothing".to_string()),
        };
        if let Some(f) = done.borrow_mut().take() {
            f(out);
        }
    });
    // SAFETY: on the main thread; the page's own world (where the helper lives), the main frame,
    // no arguments; the block lives as long as WebKit keeps it.
    unsafe {
        let world = WKContentWorld::pageWorld(mtm);
        wk.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(&NSString::from_str(body), None, None, &world, Some(&block));
    }
}

/// Prints the page (the system's print sheet, on the page's window).
#[cfg(target_os = "macos")]
pub fn print(view: &wry::WebView) {
    use objc2_app_kit::NSPrintInfo;
    let wk = web_view_of(view);
    let Some(window) = wk.window() else { return };
    // SAFETY: WebKit's print operation of the page, run as a sheet on its window (main thread).
    unsafe {
        let op = wk.printOperationWithPrintInfo(&NSPrintInfo::sharedPrintInfo());
        // Without a frame the operation prints an empty page.
        if let Some(v) = op.view() {
            v.setFrame(wk.bounds());
        }
        op.runOperationModalForWindow_delegate_didRunSelector_contextInfo(&window, None, None, std::ptr::null_mut());
    }
}

/// Whether the page is loading: from the request on (wry reports a load only once it commits,
/// and a load that fails not at all).
#[cfg(target_os = "macos")]
pub fn is_loading(view: &wry::WebView) -> bool {
    use wry::WebViewExtMacOS;
    // SAFETY: a plain WKWebView getter on the main thread.
    unsafe { view.webview().isLoading() }
}

/// A page's web view, as WebKit's own type (for the calls a pop-up's takes too).
#[cfg(target_os = "macos")]
pub fn web_view_of(view: &wry::WebView) -> objc2::rc::Retained<objc2_web_kit::WKWebView> {
    use wry::WebViewExtMacOS;
    objc2::rc::Retained::into_super(view.webview())
}

/// A number naming a page (for "is it the same one").
#[cfg(target_os = "macos")]
pub fn page_key(page: &Page) -> usize {
    std::ptr::from_ref(page) as usize
}

/// Goes back a page.
#[cfg(target_os = "macos")]
pub fn back_in(page: &Page) {
    // SAFETY: a plain WKWebView method on the main thread.
    unsafe { page.goBack() };
}

/// The page's size in CSS pixels.
#[cfg(target_os = "macos")]
pub fn css_size(page: &Page) -> (f64, f64) {
    // SAFETY: a plain WKWebView getter on the main thread.
    let z = unsafe { page.pageZoom() }.max(0.01);
    let b = page.bounds();
    (b.size.width / z, b.size.height / z)
}

/// Whether a web view is loading (see [`is_loading`]).
#[cfg(target_os = "macos")]
pub fn loading_in(wk: &objc2_web_kit::WKWebView) -> bool {
    // SAFETY: a plain WKWebView getter on the main thread.
    unsafe { wk.isLoading() }
}

/// A web view's address ("" before it has one).
#[cfg(target_os = "macos")]
pub fn url_in(wk: &objc2_web_kit::WKWebView) -> String {
    // SAFETY: a plain WKWebView getter on the main thread.
    unsafe { wk.URL() }.and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default()
}

/// Takes the page back from a site an agent may not open yet. wry tells of a load once it has
/// committed (the new page is in), so it is stopped and the page goes back to the last one.
#[cfg(target_os = "macos")]
pub fn turn_back(view: &wry::WebView) {
    use wry::WebViewExtMacOS;
    // SAFETY: plain WKWebView methods on the main thread.
    unsafe {
        let wk = view.webview();
        wk.stopLoading();
        if wk.canGoBack() {
            wk.goBack();
        }
    }
}

/// When and where (window number, point in the window) a mouse button last went down in the
/// app, seen before AppKit hands the click on.
#[cfg(target_os = "macos")]
static LAST_CLICK: Mutex<Option<(std::time::Instant, isize, f64, f64)>> = Mutex::new(None);

/// When a key last went down in the app.
#[cfg(target_os = "macos")]
static LAST_KEY: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Starts noting the app's clicks (`clicked_in`) and keys, once.
#[cfg(target_os = "macos")]
fn watch_clicks() {
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventType};
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let block = block2::RcBlock::new(|e: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit hands the monitor a live event, on the main thread.
            let ev = unsafe { e.as_ref() };
            let ty = ev.r#type();
            if matches!(ty, NSEventType::KeyDown | NSEventType::KeyUp) {
                // A key an agent pressed that its page did not take (WebKit hands it on to the
                // app): it goes nowhere, as in a browser, never into a terminal.
                if ty == NSEventType::KeyDown && crate::browser_input::agents_key(ev) {
                    return std::ptr::null_mut();
                }
                // The user's key while an agent's page has the keyboard (it types there): the
                // key goes to the app, where the user types.
                if to_app_while_agent_types(ev) {
                    return std::ptr::null_mut();
                }
                if ty == NSEventType::KeyDown {
                    *LAST_KEY.lock() = Some(std::time::Instant::now());
                }
            } else {
                let at = ev.locationInWindow();
                *LAST_CLICK.lock() = Some((std::time::Instant::now(), ev.windowNumber(), at.x, at.y));
            }
            // The event goes on as it came.
            e.as_ptr()
        });
        let mask = NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown | NSEventMask::KeyDown | NSEventMask::KeyUp;
        // SAFETY: the block returns the event it was given. The monitor stays for the app's life.
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
        std::mem::forget(monitor);
    });
}

/// The pages (`page_key`) whose keyboard is an agent's: it acts in them and the user has not
/// taken them. Set each frame (`App::sync_browsers`).
#[cfg(target_os = "macos")]
static AGENTS_PAGES: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Sets the pages whose keyboard is an agent's.
#[cfg(target_os = "macos")]
pub fn set_agents_pages(pages: Vec<usize>) {
    *AGENTS_PAGES.lock() = pages;
}

/// A key of the user's comes while an agent's page has the keyboard of its window: it goes to
/// the app (the page's parent view) as it would have, and the page keeps the keyboard. Taking
/// the keyboard from the page would clear its selection (WebKit does), and the agent's next
/// character would go to the start of the field. False: no agent's page had the keyboard.
#[cfg(target_os = "macos")]
fn to_app_while_agent_types(ev: &objc2_app_kit::NSEvent) -> bool {
    use objc2_app_kit::{NSEventType, NSView};
    let agents = AGENTS_PAGES.lock().clone();
    if agents.is_empty() {
        return false;
    }
    let Some(mtm) = objc2::MainThreadMarker::new() else { return false };
    let Some(window) = ev.window(mtm) else { return false };
    let Some(first) = window.firstResponder() else { return false };
    let Some(mut view) = first.downcast::<NSView>().ok() else { return false };
    loop {
        // SAFETY: a plain NSView getter on the main thread (the monitor runs there).
        let up = unsafe { view.superview() };
        if agents.contains(&(objc2::rc::Retained::as_ptr(&view) as usize)) {
            let Some(app) = up else { return false };
            if ev.r#type() == NSEventType::KeyDown {
                app.keyDown(ev);
            } else {
                app.keyUp(ev);
            }
            return true;
        }
        let Some(up) = up else { return false };
        view = up;
    }
}

/// Whether the user's last click, a moment ago, was on this page. A page saying it was clicked
/// (`Event::Focused`) is believed only then: any script in the page can post that message
/// (`window.ipc` is the page's), and the keyboard must not leave a terminal for it.
#[cfg(target_os = "macos")]
pub fn clicked_in(view: &wry::WebView) -> bool {
    use wry::WebViewExtMacOS;
    clicked_in_view(&view.webview(), Duration::from_secs(3))
}

/// Whether the user's last click, at most `within` ago, was on this page.
#[cfg(target_os = "macos")]
pub fn clicked_in_view(wk: &objc2_web_kit::WKWebView, within: Duration) -> bool {
    let Some((at, window, x, y)) = *LAST_CLICK.lock() else { return false };
    if at.elapsed() > within {
        return false;
    }
    if wk.window().is_none_or(|w| w.windowNumber() != window) {
        return false;
    }
    // The page's place in the window, as the click's point (from the window's bottom left).
    let r = wk.convertRect_toView(wk.bounds(), None);
    x >= r.origin.x && x <= r.origin.x + r.size.width && y >= r.origin.y && y <= r.origin.y + r.size.height
}

/// Whether the page has the keyboard (its web view, or a view in it, is the window's first
/// responder).
#[cfg(target_os = "macos")]
pub fn has_keyboard(view: &wry::WebView) -> bool {
    use wry::WebViewExtMacOS;
    view_has_keyboard(&view.webview())
}

/// [`has_keyboard`] of a web view.
#[cfg(target_os = "macos")]
pub fn view_has_keyboard(wk: &objc2_web_kit::WKWebView) -> bool {
    use objc2_app_kit::NSView;
    let Some(window) = wk.window() else { return false };
    let Some(first) = window.firstResponder() else { return false };
    first.downcast_ref::<NSView>().is_some_and(|v| v.isDescendantOf(wk))
}

/// Whether a key went down in the app at most `within` ago.
#[cfg(target_os = "macos")]
pub fn typed_within(within: Duration) -> bool {
    LAST_KEY.lock().is_some_and(|at| at.elapsed() <= within)
}

/// The process WebKit draws the page in (each page has its own), for the memory it takes.
#[cfg(target_os = "macos")]
pub fn web_pid(view: &wry::WebView) -> Option<i32> {
    use wry::WebViewExtMacOS;
    let wk = view.webview();
    let sel = objc2::sel!(_webProcessIdentifier);
    // SAFETY: a getter WKWebView has had since macOS 10.13 (not public, so asked whether it is
    // there first), returning a pid_t.
    unsafe {
        let responds: bool = objc2::msg_send![&*wk, respondsToSelector: sel];
        if !responds {
            return None;
        }
        let pid: i32 = objc2::msg_send![&*wk, _webProcessIdentifier];
        (pid > 0).then_some(pid)
    }
}

/// Dims the page (a question is asked over it) or shows it fully again.
#[cfg(target_os = "macos")]
pub fn dim(view: &wry::WebView, dimmed: bool) {
    use wry::WebViewExtMacOS;
    let alpha: f64 = if dimmed { 0.55 } else { 1.0 };
    // SAFETY: NSView's alphaValue setter, on the main thread.
    unsafe {
        let _: () = objc2::msg_send![&*view.webview(), setAlphaValue: alpha];
    }
}

/// Forgets the pages' cookies, logins, cache and storage: WebKit's default store, the one
/// profile every browser pane shares (live or asleep). Main thread only.
#[cfg(target_os = "macos")]
pub fn clear_data(_views: &[&View]) {
    use objc2_foundation::NSDate;
    use objc2_web_kit::WKWebsiteDataStore;
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    // SAFETY: on the main thread; every kind of data, since 1970, with a block that does nothing.
    unsafe {
        let store = WKWebsiteDataStore::defaultDataStore(mtm);
        let kinds = WKWebsiteDataStore::allWebsiteDataTypes(mtm);
        let since = NSDate::dateWithTimeIntervalSince1970(0.0);
        let done = block2::RcBlock::new(|| {});
        store.removeDataOfTypes_modifiedSince_completionHandler(&kinds, &since, &done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_counts_round() {
        let mut f = PageFind { query: "a".into(), ..Default::default() };
        f.answered(3, true);
        assert_eq!(f.index, 1);
        f.step = 1;
        f.answered(3, true);
        f.answered(3, true);
        assert_eq!(f.index, 3);
        f.answered(3, true);
        assert_eq!(f.index, 1);
        f.step = -1;
        f.answered(3, true);
        assert_eq!(f.index, 3);
        f.answered(0, false);
        assert_eq!((f.index, f.count), (0, Some(0)));
    }

    #[test]
    fn keys_a_page_hands_to_the_app() {
        use eframe::egui::{Key, Modifiers};
        let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
        let ctrl_shift = Modifiers { shift: true, ..ctrl };
        assert!(app_key(Key::F, ctrl) && app_key(Key::L, ctrl) && app_key(Key::Equals, ctrl));
        assert!(app_key(Key::K, ctrl_shift) && app_key(Key::T, ctrl_shift));
        assert!(!app_key(Key::C, ctrl) && !app_key(Key::V, ctrl_shift) && !app_key(Key::Z, ctrl_shift));
        assert!(!app_key(Key::F, Modifiers::default()) && !app_key(Key::K, Modifiers { alt: true, ..ctrl }));
    }

    #[test]
    fn zoom_steps_as_safari() {
        assert_eq!(zoom_step(1.0, 1), 1.1);
        assert_eq!(zoom_step(1.0, -1), 0.9);
        assert_eq!(zoom_step(3.0, 1), 3.0);
        assert_eq!(zoom_step(0.5, -1), 0.5);
        assert_eq!(zoom_step(1.05, -1), 1.0);
        let s = find_script("Linen \"shirt\"", true, false);
        assert!(s.starts_with("const q = \"Linen \\\"shirt\\\"\";"), "{s}");
        assert!(s.contains("window.find(q, false, true, true"));
    }

    #[test]
    fn addresses_as_typed() {
        assert_eq!(normalize("localhost:5173/cart"), "http://localhost:5173/cart");
        assert_eq!(normalize("127.0.0.1:8080"), "http://127.0.0.1:8080");
        assert_eq!(normalize(":3000"), "http://localhost:3000");
        assert_eq!(normalize("3000"), "http://localhost:3000");
        assert_eq!(normalize("github.com/x"), "https://github.com/x");
        assert_eq!(normalize("https://a.b/c"), "https://a.b/c");
        assert_eq!(normalize("rust egui"), "https://www.google.com/search?q=rust+egui");
        assert_eq!(normalize("  "), "");
        assert_eq!(split_url("http://localhost:4321/guides/install"), ("localhost:4321".into(), "/guides/install".into()));
        assert_eq!(file_url(std::path::Path::new("/tmp/a b.html")), "file:///tmp/a%20b.html");
    }

    #[test]
    fn agents_open_this_machine_and_the_project_sites() {
        assert_eq!(host_of("https://User@Checkout.Stripe.com:443/c/pay"), "checkout.stripe.com");
        assert_eq!(host_of("http://[::1]:3000/"), "[::1]");
        // A `@` after the host is in the query or the fragment, not before the host.
        assert_eq!(host_of("https://evil.example#@localhost/"), "evil.example");
        assert_eq!(host_of("https://evil.example?q=1@localhost"), "evil.example");
        assert_eq!(host_of("https://evil.example\\@localhost/"), "evil.example");
        assert!(!agent_may_open("https://evil.example#@localhost/", &["localhost".into()], None));
        assert_eq!(split_url("http://localhost:3000?x=1"), ("localhost:3000".into(), "?x=1".into()));
        let mut sites = tc_core::model::agent_sites();
        sites.push("stripe.com".to_string());
        for ok in ["http://localhost:5173/cart", "http://127.0.0.1:8080", "http://app.local/", "about:blank", "file:///tmp/a.html", "https://checkout.stripe.com/x", "https://stripe.com"] {
            assert!(agent_may_open(ok, &sites, None), "{ok}");
        }
        for asks in ["https://github.com/", "https://notstripe.com/", "https://stripe.com.evil.io/", "javascript:alert(1)", "ftp://x.org/", "http://localhost.evil.io/"] {
            assert!(!agent_may_open(asks, &sites, None), "{asks}");
        }
        assert!(agent_may_open("https://github.com/x", &[], Some("github.com")));
        // Agents that don't ask (the default) may open any site.
        assert!(agent_may_open("https://www.youtube.com/watch?v=1", &["*".into()], None));
        assert!(site_listed("example.com", &["*".into()]));
    }

    #[test]
    fn page_state_from_the_page() {
        let st = page_state(r#"{"a":true,"c":false,"e":true,"m":false}"#).unwrap();
        assert!(st.audible && st.editing && !st.capturing && !st.muted);
        assert_eq!(page_state("{}"), Some(PageState::default()));
        assert!(page_state("not json").is_none());
        assert!(mute_script(true).ends_with("await window.__tc.run('mute', { on: true }));"));
    }

}
