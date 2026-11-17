//! Browser panes on Linux: the pages are WebKitGTK web views in a helper program,
//! `ternarycode-web` (crates/tc-web), laid in the app's own X11 window as child windows of it.
//! The app itself needs no GTK or WebKit, so it starts on any distribution; pages show where
//! WebKitGTK 4.1 is installed and the window is an X11 one (Wayland has no windows of one
//! program inside another's: there, pages open in the default browser).
//!
//! The helper starts with the first page and ends with the app. The app sends it commands (a
//! JSON object a line) and hears what the pages do the same way; each frame takes in what came
//! (`pump`). This module is also `browser_dialog` and `browser_input` on Linux (`lib.rs`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::rc::Rc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tc_core::PaneId;

use crate::browser::{ConsoleLine, Dialog, DialogKind, Event, Sink};

/// Where a page is laid: pixels of the screen from the top left of the window's content (GTK's
/// scale is a whole number; the window's, from the X server's DPI, need not be: the helper
/// takes pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A rectangle in egui points as the page's bounds (`_zoom` is in egui's pixels per point).
pub fn bounds(rect: eframe::egui::Rect, _zoom: f32) -> Bounds {
    let ppp = f64::from(f32::from_bits(PPP.load(Ordering::Relaxed))).max(0.25);
    let r = |v: f32| (f64::from(v) * ppp).round();
    Bounds { x: r(rect.left()), y: r(rect.top()), w: r(rect.right()) - r(rect.left()), h: r(rect.bottom()) - r(rect.top()) }
}

/// egui's pixels per point (the window's scale and zoom), for the pages' bounds.
static PPP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// What the app knows of a page in the helper.
pub struct Page {
    /// Its number in the helper.
    n: u64,
    url: RefCell<String>,
    loading: Cell<bool>,
    keyboard: Cell<bool>,
    clicked: Cell<Option<Instant>>,
    /// Its size in CSS pixels (its bounds over its zoom).
    size: Cell<(f64, f64)>,
    zoom: Cell<f64>,
    /// The helper is gone (the page with it).
    dead: Cell<bool>,
    devtools: Cell<bool>,
}

pub type PageRc = Rc<Page>;

/// A page's web view, as wry's calls (the helper makes them).
pub struct View {
    page: Rc<Page>,
}

/// wry's error, as a page's calls return it here.
pub type Error = String;

impl View {
    fn send(&self, op: &str, mut rest: Value) {
        rest["op"] = op.into();
        rest["id"] = self.page.n.into();
        send(rest);
    }

    pub fn set_visible(&self, v: bool) -> Result<(), Error> {
        self.send("show", json!({ "v": v }));
        Ok(())
    }

    pub fn set_bounds(&self, b: Bounds) -> Result<(), Error> {
        let z = self.page.zoom.get().max(0.01);
        self.page.size.set((b.w / z, b.h / z));
        self.send("bounds", json!({ "x": b.x, "y": b.y, "w": b.w, "h": b.h }));
        Ok(())
    }

    pub fn zoom(&self, z: f64) -> Result<(), Error> {
        let (w, h) = self.page.size.get();
        let old = self.page.zoom.replace(z.max(0.01));
        self.page.size.set((w * old / z.max(0.01), h * old / z.max(0.01)));
        self.send("zoom", json!({ "z": z }));
        Ok(())
    }

    pub fn focus(&self) -> Result<(), Error> {
        self.send("focus", json!({}));
        Ok(())
    }

    pub fn focus_parent(&self) -> Result<(), Error> {
        self.page.keyboard.set(false);
        self.send("blur", json!({}));
        Ok(())
    }

    pub fn load_url(&self, url: &str) -> Result<(), Error> {
        self.page.loading.set(true);
        self.send("load", json!({ "url": url }));
        Ok(())
    }

    pub fn reload(&self) -> Result<(), Error> {
        self.page.loading.set(true);
        self.send("reload", json!({}));
        Ok(())
    }

    pub fn go_back(&self) -> Result<(), Error> {
        self.send("back", json!({}));
        Ok(())
    }

    pub fn go_forward(&self) -> Result<(), Error> {
        self.send("forward", json!({}));
        Ok(())
    }

    pub fn url(&self) -> Result<String, Error> {
        Ok(self.page.url.borrow().clone())
    }

    pub fn open_devtools(&self) {
        self.page.devtools.set(true);
        self.send("devtools", json!({}));
    }

    /// The Web Inspector was opened (WebKitGTK's own window; it tells not when it closes).
    pub fn is_devtools_open(&self) -> bool {
        self.page.devtools.get()
    }

    /// The helper went away: the page is gone (the pane loads it again).
    pub fn dead(&self) -> bool {
        self.page.dead.get()
    }
}

impl Drop for View {
    fn drop(&mut self) {
        PAGES.with_borrow_mut(|p| p.remove(&self.page.n));
        if !self.page.dead.get() {
            self.send("close", json!({}));
        }
    }
}

/// The page of a view (the same one each time).
pub fn web_view_of(view: &View) -> PageRc {
    view.page.clone()
}

/// A number naming the page (for "is it the same one").
pub fn page_key(page: &Page) -> usize {
    page.n as usize
}

/// What gets a script's answer, and a picture.
type Answer = Box<dyn FnOnce(Result<String, String>)>;
type Shot = Box<dyn FnOnce(Option<Vec<u8>>)>;

struct Helper {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
}

thread_local! {
    static HELPER: RefCell<Option<Helper>> = const { RefCell::new(None) };
    /// The pages: where each reports, and what the app knows of it.
    static PAGES: RefCell<HashMap<u64, (Sink, Rc<Page>)>> = RefCell::new(HashMap::new());
    static RETURNS: RefCell<HashMap<u64, (Instant, Answer)>> = RefCell::new(HashMap::new());
    static SHOTS: RefCell<HashMap<u64, (Instant, String, Shot)>> = RefCell::new(HashMap::new());
    /// Dialogs waiting: the page's, and the helper's number of it.
    static WAITING: RefCell<HashMap<u64, (PaneId, u64)>> = RefCell::new(HashMap::new());
    /// Sign-in pop-ups open, by their number in the helper: the page that opened each.
    static POPUPS: RefCell<Vec<(u64, u64, Rc<Page>)>> = const { RefCell::new(Vec::new()) };
    /// A pop-up an agent's step opened to a site it may not open, held back: its address, by
    /// the page's number.
    static BLOCKED: RefCell<HashMap<u64, String>> = RefCell::new(HashMap::new());
}

fn new_page(n: u64, url: &str) -> Rc<Page> {
    Rc::new(Page {
        n,
        url: RefCell::new(url.to_string()),
        loading: Cell::new(true),
        keyboard: Cell::new(false),
        clicked: Cell::new(None),
        size: Cell::new((800.0, 600.0)),
        zoom: Cell::new(1.0),
        dead: Cell::new(false),
        devtools: Cell::new(false),
    })
}

/// What the helper said, waiting for the next frame.
static INBOX: Mutex<Vec<Value>> = Mutex::new(Vec::new());
/// The window to wake when it says something.
static WAKE: Mutex<Option<eframe::egui::Context>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

fn send(v: Value) {
    HELPER.with_borrow_mut(|h| {
        if let Some(h) = h
            && writeln!(h.stdin, "{v}").and_then(|_| h.stdin.flush()).is_err()
        {
            // It is gone: the next page starts another.
        }
    });
}

/// The helper beside the app (or in its lib folder, as the packages install it).
fn helper_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("ternarycode-web"), dir.join("../lib/ternarycode/ternarycode-web")].into_iter().find(|p| p.is_file())
}

/// The kind of the app's window: 0 not known yet, 1 X11, 2 another (Wayland).
static WINDOW: AtomicU8 = AtomicU8::new(0);

/// Notes what kind of window the app has, and its scale (each frame; cheap).
pub fn note_window(window: &impl raw_window_handle::HasWindowHandle, ctx: &eframe::egui::Context) {
    PPP.store(ctx.pixels_per_point().to_bits(), Ordering::Relaxed);
    if WINDOW.load(Ordering::Relaxed) != 0 {
        return;
    }
    let x11 = window.window_handle().is_ok_and(|h| matches!(h.as_raw(), raw_window_handle::RawWindowHandle::Xlib(_) | raw_window_handle::RawWindowHandle::Xcb(_)));
    WINDOW.store(if x11 { 1 } else { 2 }, Ordering::Relaxed);
}

/// Whether pages can show here: an X11 window, and the helper starts (WebKitGTK is there).
pub fn available() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if WINDOW.load(Ordering::Relaxed) == 2 {
        return false;
    }
    *OK.get_or_init(|| {
        helper_path().is_some_and(|p| {
            std::process::Command::new(p)
                .arg("--check")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    })
}

/// Starts the helper unless it runs.
fn ensure_helper() -> bool {
    HELPER.with_borrow_mut(|h| {
        if let Some(helper) = h
            && helper.child.try_wait().ok().flatten().is_none()
        {
            return true;
        }
        *h = None;
        let Some(path) = helper_path() else { return false };
        let child = std::process::Command::new(path)
            // GDK on the X server the app's window is on (not Wayland, under XWayland).
            .env("GDK_BACKEND", "x11")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn();
        let Ok(mut child) = child else { return false };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else { return false };
        let _ = std::thread::Builder::new().name("browser-helper".into()).spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    INBOX.lock().push(v);
                    if let Some(ctx) = &*WAKE.lock() {
                        ctx.request_repaint();
                    }
                }
            }
            INBOX.lock().push(json!({ "ev": "gone" }));
            if let Some(ctx) = &*WAKE.lock() {
                ctx.request_repaint();
            }
        });
        *h = Some(Helper { child, stdin });
        true
    })
}

/// The page as a web view laid over the window at `bounds`.
pub fn build_view(window: &impl raw_window_handle::HasWindowHandle, url: &str, bounds: Bounds, sink: Sink) -> Option<View> {
    let parent = match window.window_handle().ok()?.as_raw() {
        raw_window_handle::RawWindowHandle::Xlib(h) => h.window,
        raw_window_handle::RawWindowHandle::Xcb(h) => u64::from(h.window.get()),
        _ => return None,
    };
    if !ensure_helper() {
        return None;
    }
    *WAKE.lock() = Some(sink.ctx().clone());
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let page = new_page(n, url);
    page.size.set((bounds.w, bounds.h));
    PAGES.with_borrow_mut(|p| p.insert(n, (sink, page.clone())));
    let downloads = if cfg!(debug_assertions) { std::env::var("TERNARYCODE_DOWNLOAD_DIR").unwrap_or_default() } else { String::new() };
    send(json!({
        "op": "new", "id": n, "parent": parent, "url": url,
        "x": bounds.x, "y": bounds.y, "w": bounds.w, "h": bounds.h,
        "data": crate::paths::data_dir().join("WebKitGTK").to_string_lossy(),
        "downloads": downloads,
        "scripts": [crate::browser::AGENT_JS],
    }));
    Some(View { page })
}

/// Takes in what the helper said (each frame): the pages' events, scripts' answers, pictures.
pub fn pump() {
    let inbox = std::mem::take(&mut *INBOX.lock());
    for v in inbox {
        let n = v["id"].as_u64().unwrap_or(0);
        let ev = v["ev"].as_str().unwrap_or("");
        if ev == "gone" {
            // Its pages went with it: each loads again in a new helper when shown.
            PAGES.with_borrow(|p| p.values().for_each(|(_, page)| page.dead.set(true)));
            HELPER.with_borrow_mut(|h| *h = None);
            continue;
        }
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        // A pop-up's own address and loading (its page's stay as they are).
        if let Some(popup) = POPUPS.with_borrow(|p| p.iter().find(|(pid, ..)| *pid == n).map(|(.., page)| page.clone())) {
            match ev {
                "url" => *popup.url.borrow_mut() = s("v"),
                "loading" => popup.loading.set(v["v"].as_bool().unwrap_or(false)),
                _ => {}
            }
            continue;
        }
        let Some((sink, page)) = PAGES.with_borrow(|p| p.get(&n).cloned()) else { continue };
        match ev {
            "url" => {
                let u = s("v");
                *page.url.borrow_mut() = u.clone();
                sink.push(Event::Url(u));
            }
            "title" => sink.push(Event::Title(s("v"))),
            "loading" => {
                let l = v["v"].as_bool().unwrap_or(false);
                page.loading.set(l);
                sink.push(Event::Loading(l));
            }
            "focus" => page.keyboard.set(v["v"].as_bool().unwrap_or(false)),
            "clicked" => {
                page.clicked.set(Some(Instant::now()));
                page.keyboard.set(true);
                sink.push(Event::Focused);
            }
            "ipc" => ipc(&sink, &s("v")),
            "open_here" => sink.push(Event::OpenHere(s("url"))),
            "download_started" => sink.push(Event::DownloadStarted(s("url"), std::path::PathBuf::from(s("path")))),
            "download_done" => sink.push(Event::DownloadDone(s("url"), v["ok"].as_bool().unwrap_or(false))),
            "dialog" => {
                static DID: AtomicU64 = AtomicU64::new(1);
                let id = DID.fetch_add(1, Ordering::Relaxed);
                WAITING.with_borrow_mut(|w| w.insert(id, (sink.page, v["did"].as_u64().unwrap_or(0))));
                let kind = match v["kind"].as_str() {
                    Some("confirm") => DialogKind::Confirm,
                    Some("prompt") => DialogKind::Prompt,
                    _ => DialogKind::Alert,
                };
                let message = if v["before_unload"].as_bool().unwrap_or(false) && s("message").is_empty() {
                    crate::i18n::t("Leave this page? Changes you made may not be saved.").to_string()
                } else {
                    s("message")
                };
                let host = crate::browser::host_of(&page.url.borrow());
                sink.push(Event::Dialog(Dialog { id, kind, host, message, text: s("text") }));
            }
            "key" => {
                let key = eframe::egui::Key::from_name(&s("key"));
                let (ctrl, shift, alt) = (v["ctrl"].as_bool().unwrap_or(false), v["shift"].as_bool().unwrap_or(false), v["alt"].as_bool().unwrap_or(false));
                if let Some(key) = key {
                    let mods = eframe::egui::Modifiers { ctrl, shift, alt, command: ctrl, mac_cmd: false };
                    sink.push(Event::Key(key, mods));
                }
            }
            "shot" => {
                let token = v["token"].as_u64().unwrap_or(0);
                if let Some((_, path, done)) = SHOTS.with_borrow_mut(|m| m.remove(&token)) {
                    let png = v["ok"].as_bool().unwrap_or(false).then(|| std::fs::read(&path).ok()).flatten();
                    let _ = std::fs::remove_file(&path);
                    done(png);
                }
            }
            "chooser" => {
                UPLOADS.with_borrow_mut(|u| u.remove(&n));
            }
            "closed" => page.dead.set(true),
            "popup" => {
                let pid = v["pid"].as_u64().unwrap_or(0);
                POPUPS.with_borrow_mut(|p| p.push((pid, n, new_page(pid, &s("url")))));
            }
            "popup_closed" => {
                let pid = v["pid"].as_u64().unwrap_or(0);
                POPUPS.with_borrow_mut(|p| p.retain(|(id, ..)| *id != pid));
            }
            "blocked" => {
                BLOCKED.with_borrow_mut(|b| b.insert(n, s("url")));
            }
            _ => {}
        }
    }
    let old = |at: &Instant| at.elapsed() > Duration::from_secs(120);
    RETURNS.with_borrow_mut(|r| r.retain(|_, (at, _)| !old(at)));
    SHOTS.with_borrow_mut(|r| r.retain(|_, (at, ..)| !old(at)));
}

/// A message the page posted (`window.ipc`): a script's answer, its console.
fn ipc(sink: &Sink, body: &str) {
    if let Some(rest) = body.strip_prefix("ret:") {
        let mut parts = rest.splitn(3, ':');
        let (Some(t), Some(ok), Some(text)) = (parts.next().and_then(|t| t.parse::<u64>().ok()), parts.next(), parts.next()) else { return };
        if let Some((_, done)) = RETURNS.with_borrow_mut(|r| r.remove(&t)) {
            done(if ok == "1" { Ok(text.to_string()) } else { Err(text.to_string()) });
        }
    } else if let Some(json) = body.strip_prefix("console:") {
        let lines: Vec<(String, String, String)> = serde_json::from_str(json).unwrap_or_default();
        let lines: Vec<ConsoleLine> = lines.into_iter().map(|(level, text, src)| ConsoleLine { level, text, src }).collect();
        if !lines.is_empty() {
            sink.push(Event::Console(lines));
        }
    } else if let Some(state) = body.strip_prefix("state:").and_then(crate::browser::page_state) {
        sink.push(Event::State(state));
    }
}

/// A token no page guesses, for a script's answer.
fn token() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(NEXT.fetch_add(1, Ordering::Relaxed));
    h.finish() | 1
}

fn send_page(page: &Page, op: &str, mut rest: Value) {
    rest["op"] = op.into();
    rest["id"] = page.n.into();
    send(rest);
}

/// Runs `body` (the body of an async function) in the page and hands what it returns (a string)
/// to `done`, once.
pub fn run_js_in(page: &Page, body: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    if page.dead.get() {
        return done(Err("the page went away".into()));
    }
    let t = token();
    let script = format!(
        "(async () => {{\n{body}\n}})().then(r => window.ipc.postMessage('ret:{t}:' + (typeof r === 'string' ? '1:' + r : '0:the page returned nothing')), e => window.ipc.postMessage('ret:{t}:0:' + String(e && e.message || e)));"
    );
    RETURNS.with_borrow_mut(|r| r.insert(t, (Instant::now(), Box::new(done))));
    send_page(page, "js", json!({ "script": script }));
}

/// [`run_js_in`] of a view.
pub fn run_js(view: &View, body: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    run_js_in(&view.page, body, done);
}

/// The page as it shows now, as a PNG, handed to `done` (once).
pub fn snapshot_view(page: &Page, _width: Option<f64>, done: impl FnOnce(Option<Vec<u8>>) + 'static) {
    if page.dead.get() {
        return done(None);
    }
    let t = token();
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let path = dir.join(format!("ternarycode-page-{}-{t}.png", std::process::id())).to_string_lossy().to_string();
    SHOTS.with_borrow_mut(|m| m.insert(t, (Instant::now(), path.clone(), Box::new(done))));
    send_page(page, "shot", json!({ "token": t, "path": path }));
}

/// [`snapshot_view`] of a view.
pub fn snapshot_at(view: &View, width: Option<f64>, done: impl FnOnce(Option<Vec<u8>>) + 'static) {
    snapshot_view(&view.page, width, done);
}

pub fn loading_in(page: &Page) -> bool {
    page.loading.get()
}

pub fn is_loading(view: &View) -> bool {
    view.page.loading.get()
}

pub fn url_in(page: &Page) -> String {
    page.url.borrow().clone()
}

pub fn back_in(page: &Page) {
    send_page(page, "back", json!({}));
}

/// The page's size in CSS pixels.
pub fn css_size(page: &Page) -> (f64, f64) {
    page.size.get()
}

/// Takes the page back from a site an agent may not open yet.
pub fn turn_back(view: &View) {
    view.send("stop_back", json!({}));
}

/// Prints the page (the system's print dialog).
pub fn print(view: &View) {
    view.send("print", json!({}));
}

pub fn view_has_keyboard(page: &Page) -> bool {
    page.keyboard.get()
}

pub fn has_keyboard(view: &View) -> bool {
    view.page.keyboard.get()
}

/// Whether the user clicked the page a moment ago (the helper tells of real clicks only).
pub fn clicked_in(view: &View) -> bool {
    view.page.clicked.get().is_some_and(|at| at.elapsed() < Duration::from_secs(3))
}

/// Dims the page under a question: not here (a window in another program's has no opacity).
pub fn dim(_view: &View, _dimmed: bool) {}

/// Editing commands go to the page by its own keys (it has the keyboard).
pub fn edit_command(view: &View, command: &str) {
    let js = format!("document.execCommand({}); return '';", serde_json::to_string(command).unwrap_or_default());
    run_js(view, &js, |_| {});
}

pub fn web_pid(_view: &View) -> Option<i32> {
    None
}

/// Forgets the pages' cookies, logins, cache and storage (the one profile every page shares).
pub fn clear_data(_views: &[&View]) {
    let dir = crate::paths::data_dir().join("WebKitGTK");
    if HELPER.with_borrow(Option::is_some) {
        send(json!({ "op": "clear", "data": dir.to_string_lossy() }));
    } else {
        let _ = std::fs::remove_dir_all(dir);
    }
}

// Dialogs (`browser_dialog` on Linux).

/// Answers a waiting dialog: OK (`accept`, with `text` for a prompt) or Cancel.
pub fn answer(id: u64, accept: bool, text: &str) -> bool {
    let Some((_, did)) = WAITING.with_borrow_mut(|w| w.remove(&id)) else { return false };
    send(json!({ "op": "dialog", "did": did, "accept": accept, "text": text }));
    true
}

/// Dialogs of pages gone are answered Cancel (`live`: the pages up now).
pub fn tidy(live: &[PaneId]) {
    let gone: Vec<u64> = WAITING.with_borrow(|w| w.iter().filter(|(_, (p, _))| !live.contains(p)).map(|(id, _)| *id).collect());
    for id in gone {
        answer(id, false, "");
    }
}

thread_local! {
    static UPLOADS: RefCell<HashMap<u64, Instant>> = RefCell::new(HashMap::new());
}

/// The next file chooser of `page` takes these files (an agent's upload).
pub fn upload_next(page: &Page, files: Vec<std::path::PathBuf>) {
    UPLOADS.with_borrow_mut(|u| u.insert(page.n, Instant::now()));
    let files: Vec<String> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();
    send_page(page, "upload", json!({ "files": files }));
}

/// Whether files given to `page`'s next chooser are still waiting (no chooser came).
pub fn upload_unused(page: &Page) -> bool {
    let unused = UPLOADS.with_borrow_mut(|u| u.remove(&page.n).is_some());
    if unused {
        send_page(page, "upload", json!({ "files": [] }));
    }
    unused
}

// An agent's input (`browser_input` on Linux): GDK events in the helper, as the user's.

pub fn hover(page: &Page, x: f64, y: f64) -> bool {
    send_page(page, "mouse", json!({ "kind": "move", "x": x, "y": y }));
    true
}

pub fn click(page: &Page, x: f64, y: f64, count: isize) -> bool {
    send_page(page, "mouse", json!({ "kind": "click", "x": x, "y": y, "count": count }));
    true
}

pub fn drag(page: &Page, from: (f64, f64), to: (f64, f64)) -> bool {
    send_page(page, "mouse", json!({ "kind": "drag", "x": from.0, "y": from.1, "x2": to.0, "y2": to.1 }));
    true
}

const KEYS: [&str; 15] = ["Enter", "Tab", "Escape", "Esc", "Backspace", "Delete", "Space", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown"];

/// Presses a key (`Enter`, `Tab`, `a`…) in the page. Err: a key this does not know.
pub fn key(page: &Page, key: &str) -> Result<(), String> {
    if !KEYS.contains(&key) && key.chars().count() != 1 {
        return Err(format!("Unknown key {key:?}: use Enter, Tab, Escape, Backspace, Delete, Space, ArrowUp/Down/Left/Right, Home, End, PageUp, PageDown, or one character."));
    }
    send_page(page, "key", json!({ "key": key }));
    Ok(())
}

/// Types `text` into what has the keyboard in the page.
pub fn type_text(page: &Page, text: &str) -> bool {
    send_page(page, "text", json!({ "text": text }));
    true
}

// Sign-in pop-ups (`browser_popup` on Linux): the helper opens them as windows of their own, tied
// to their page; an agent's steps act in the newest one while it is open.

/// The agent's step acts in `page` now: a pop-up it opens is the agent's, to its sites only.
pub fn agent_acting(page: &Page, sites: Vec<String>, pass: Option<String>) {
    send_page(page, "agent", json!({ "sites": sites, "pass": pass }));
}

/// The user took the browser over: pop-ups are theirs.
pub fn agent_stopped() {
    send(json!({ "op": "agent_stop" }));
}

/// A pop-up the agent's step on `page` opened to a site it may not open (held back): its address.
pub fn take_blocked(page: usize) -> Option<String> {
    BLOCKED.with_borrow_mut(|b| b.remove(&(page as u64)))
}

/// The helper's number of the page of a pane.
fn number_of(pane: PaneId) -> Option<u64> {
    PAGES.with_borrow(|p| p.iter().find(|(_, (sink, _))| sink.page == pane).map(|(n, _)| *n))
}

/// Closes the pop-ups a pane's page opened (its agent went to another address).
pub fn close_of(pane: PaneId) {
    if let Some(n) = number_of(pane) {
        send(json!({ "op": "close_popups", "id": n }));
        POPUPS.with_borrow_mut(|p| p.retain(|(_, opener, _)| *opener != n));
    }
}

/// The newest pop-up a pane's page opened, still open.
pub fn popup_of(pane: PaneId) -> Option<PageRc> {
    let n = number_of(pane)?;
    POPUPS.with_borrow(|p| p.iter().rev().find(|(_, opener, _)| *opener == n).map(|(.., page)| page.clone()))
}
