//! Browser panes on Windows: the page is WebView2 (Microsoft Edge's engine, part of Windows),
//! laid over the window as a child window, as WebKit is on macOS (`browser`). What the app asks
//! of a page goes to WebView2 here: its scripts and their answers, its picture, its dialogs
//! (which show as the app's bar above the page, not Edge's own boxes), and an agent's clicks and
//! keys, which are real input (the DevTools Protocol's `Input` domain): trusted events that
//! reach into frames of other sites and open file choosers, as a person's do.
//!
//! This module is also `browser_dialog` and `browser_input` on Windows (see `lib.rs`): the same
//! calls as on macOS.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Deref;
use std::rc::Rc;
use std::time::{Duration, Instant};

use tc_core::PaneId;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    AcceleratorKeyPressedEventHandler, CallDevToolsProtocolMethodCompletedHandler, CapturePreviewCompletedHandler, DevToolsProtocolEventReceivedEventHandler,
    ExecuteScriptCompletedHandler, FocusChangedEventHandler, NavigationCompletedEventHandler, NavigationStartingEventHandler, ScriptDialogOpeningEventHandler,
};
use windows::Win32::Foundation::HWND;
use windows::core::{HSTRING, Interface, PWSTR};
use wry::WebViewExtWindows;

use crate::browser::{ConsoleLine, Dialog, DialogKind, Event, Sink};

/// A page's web view: wry's, and WebView2's own objects for what wry does not do.
pub struct View {
    web: wry::WebView,
    page: Rc<Page>,
    devtools: Cell<bool>,
}

impl Deref for View {
    type Target = wry::WebView;
    fn deref(&self) -> &wry::WebView {
        &self.web
    }
}

impl View {
    /// Opens the Web Inspector (Edge's DevTools, a window of its own).
    pub fn open_devtools(&self) {
        self.devtools.set(true);
        self.web.open_devtools();
    }

    /// The Web Inspector was opened (WebView2 does not tell when it closes; wry always says no).
    pub fn is_devtools_open(&self) -> bool {
        self.devtools.get()
    }
}

/// What the agents' steps act on (a page): WebView2's objects for it.
pub struct Page {
    wv: ICoreWebView2,
    controller: ICoreWebView2Controller,
    /// wry's window holding the page (a child of the app's).
    hwnd: HWND,
    /// From the request of a load on, until it ends (or fails).
    loading: Rc<Cell<bool>>,
    /// The page this is.
    id: PaneId,
    /// Focus emulation is on (keys reach the page without the window's keyboard).
    emulating: Cell<bool>,
    /// The page has the keyboard (WebView2 says when it gets and loses it).
    keyboard: Rc<Cell<bool>>,
}

pub type PageRc = Rc<Page>;

impl Drop for Page {
    fn drop(&mut self) {
        let key = page_key(self);
        let _ = KEYS.try_with(|k| k.borrow_mut().remove(&key));
        let _ = UPLOADS.try_with(|u| u.borrow_mut().remove(&key));
    }
}

/// The page of a view (the same one each time).
pub fn web_view_of(view: &View) -> PageRc {
    view.page.clone()
}

/// A number naming the page (for "is it the same one").
pub fn page_key(page: &Page) -> usize {
    std::ptr::from_ref(page) as usize
}

/// The default name WebView2 gives is Edge's; sites know it.
const DATA: &str = "WebView2";

/// What gets a script's answer.
type Answer = Box<dyn FnOnce(Result<String, String>)>;

thread_local! {
    /// The pages' profile (logins, cookies) in the app's data folder, for every page.
    static CONTEXT: RefCell<wry::WebContext> = RefCell::new(wry::WebContext::new(Some(crate::paths::data_dir().join(DATA))));
    /// Scripts waiting for their answer (the page posts it back), by their token.
    static RETURNS: RefCell<HashMap<u64, (Instant, Answer)>> = RefCell::new(HashMap::new());
    /// Dialogs waiting for an answer.
    static WAITING: RefCell<HashMap<u64, (PaneId, ICoreWebView2ScriptDialogOpeningEventArgs, ICoreWebView2Deferral)>> = RefCell::new(HashMap::new());
    /// Files an agent's step gives the next file chooser of a page.
    static UPLOADS: RefCell<HashMap<usize, (Instant, Vec<std::path::PathBuf>)>> = RefCell::new(HashMap::new());
}

/// The page as a web view laid over the window at `bounds` (logical points from the top left of
/// the window's content).
pub fn build_view(window: &impl raw_window_handle::HasWindowHandle, url: &str, bounds: wry::Rect, sink: Sink) -> Option<View> {
    use wry::{PageLoadEvent, PermissionResponse, WebViewBuilder};
    let (s1, s2, s4, s5, s6) = (sink.clone(), sink.clone(), sink.clone(), sink.clone(), sink.clone());
    let url = if url.is_empty() { "about:blank" } else { url };
    let owner = match window.window_handle().ok()?.as_raw() {
        raw_window_handle::RawWindowHandle::Win32(h) => h.hwnd.get(),
        _ => return None,
    };
    let web = CONTEXT.with_borrow_mut(|ctx| {
        WebViewBuilder::new_with_web_context(ctx)
            .with_url(url)
            .with_bounds(bounds)
            .with_initialization_script(crate::browser::FOCUS_SCRIPT)
            .with_initialization_script(crate::browser::AGENT_JS)
            .with_document_title_changed_handler(move |t| s1.push(Event::Title(t)))
            .with_on_page_load_handler(move |e, u| {
                s2.push(Event::Url(u));
                s2.push(Event::Loading(matches!(e, PageLoadEvent::Started)));
            })
            .with_ipc_handler(move |req| {
                let body = req.body();
                if let Some(rest) = body.strip_prefix("ret:") {
                    returned(rest);
                } else if body == "focus" {
                    s4.push(Event::Focused);
                } else if let Some(json) = body.strip_prefix("console:") {
                    let lines: Vec<(String, String, String)> = serde_json::from_str(json).unwrap_or_default();
                    let lines: Vec<ConsoleLine> = lines.into_iter().map(|(level, text, src)| ConsoleLine { level, text, src }).collect();
                    if !lines.is_empty() {
                        s4.push(Event::Console(lines));
                    }
                } else if let Some(state) = body.strip_prefix("state:").and_then(crate::browser::page_state) {
                    s4.push(Event::State(state));
                }
            })
            .with_permission_handler(|_| PermissionResponse::Default)
            .with_devtools(true)
            .with_download_started_handler(move |url, path| {
                #[cfg(debug_assertions)]
                if let (Some(dir), Some(name)) = (std::env::var_os("TERNARYCODE_DOWNLOAD_DIR"), path.file_name().map(|n| n.to_owned())) {
                    *path = std::path::PathBuf::from(dir).join(name);
                }
                s5.push(Event::DownloadStarted(url, path.clone()));
                true
            })
            .with_download_completed_handler(move |url, _, ok| s6.push(Event::DownloadDone(url, ok)))
            .build_as_child(window)
            .ok()
    })?;
    let (wv, controller, hwnd) = (web.webview(), web.controller(), web.hwnd());
    let loading = Rc::new(Cell::new(true));
    let keyboard = Rc::new(Cell::new(false));
    let page = Rc::new(Page { wv, controller, hwnd, loading: loading.clone(), id: sink.page, emulating: Cell::new(false), keyboard: keyboard.clone() });
    KEYS.with_borrow_mut(|k| k.insert(page_key(&page), sink.page));
    new_windows(&page, HWND(owner as *mut _), web.environment(), sink.clone());
    watch(&page, loading, Some(sink));
    Some(View { web, page, devtools: Cell::new(false) })
}

/// WebView2's events wry does not hand on: loads from their request on, dialogs, keys the app
/// has (⌃F, ⌃L…), file choosers for agents. A pop-up (no `sink`) keeps Edge's own dialogs and
/// keys.
fn watch(page: &Page, loading: Rc<Cell<bool>>, sink: Option<Sink>) {
    let mut token = 0i64;
    let (l1, l2) = (loading.clone(), loading);
    // SAFETY: WebView2 calls on the window's thread; each handler lives as long as the page.
    unsafe {
        let _ = page.wv.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, _| {
                l1.set(true);
                Ok(())
            })),
            &mut token,
        );
        let _ = page.wv.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, _| {
                l2.set(false);
                Ok(())
            })),
            &mut token,
        );
        // Whether the page has the keyboard: its windows are the browser process's.
        let (k1, k2) = (page.keyboard.clone(), page.keyboard.clone());
        let _ = page.controller.add_GotFocus(
            &FocusChangedEventHandler::create(Box::new(move |_, _| {
                k1.set(true);
                Ok(())
            })),
            &mut token,
        );
        let _ = page.controller.add_LostFocus(
            &FocusChangedEventHandler::create(Box::new(move |_, _| {
                k2.set(false);
                Ok(())
            })),
            &mut token,
        );
        if let Some(sink) = sink {
            // Dialogs: the app's bar above the page (Edge's boxes would block the window).
            if let Ok(settings) = page.wv.Settings() {
                let _ = settings.SetAreDefaultScriptDialogsEnabled(false);
            }
            let s = sink.clone();
            let id = page.id;
            let _ = page.wv.add_ScriptDialogOpening(
                &ScriptDialogOpeningEventHandler::create(Box::new(move |wv, args| {
                    let Some(args) = args else { return Ok(()) };
                    dialog_opening(id, wv, args, &s);
                    Ok(())
                })),
                &mut token,
            );
            // Keys of the app's while the page has the keyboard: the app takes them.
            let _ = page.controller.add_AcceleratorKeyPressed(
                &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    accelerator(&args, &sink);
                    Ok(())
                })),
                &mut token,
            );
        }
        // A file chooser an agent's click opened: the agent's files go in.
        let key = page_key(page);
        let wv = page.wv.clone();
        if let Ok(receiver) = page.wv.GetDevToolsProtocolEventReceiver(&HSTRING::from("Page.fileChooserOpened")) {
            let _ = receiver.add_DevToolsProtocolEventReceived(
                &DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut json = PWSTR::null();
                    args.ParameterObjectAsJson(&mut json)?;
                    let json = webview2_com::take_pwstr(json);
                    chooser_opened(&wv, key, &json);
                    Ok(())
                })),
                &mut token,
            );
        }
    }
}

/// Keys the app has (⌃F find, ⌃L address, ⌃R reload, ⌃= ⌃− zoom, ⌃T, ⌃W…): WebView2 tells of
/// every key with ⌃ or Alt; the app's own go to the app (`Event::Key`), the rest stay the page's.
fn accelerator(args: &ICoreWebView2AcceleratorKeyPressedEventArgs, sink: &Sink) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
    let (mut kind, mut vk) = (COREWEBVIEW2_KEY_EVENT_KIND::default(), 0u32);
    // SAFETY: the event's own getters, during the event.
    unsafe {
        if args.KeyEventKind(&mut kind).is_err() || args.VirtualKey(&mut vk).is_err() {
            return;
        }
    }
    if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
        return;
    }
    // SAFETY: plain reads of the keyboard's state (the system's: the key went to the browser
    // process's window, not to this thread's queue).
    let held = |k: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe { GetAsyncKeyState(i32::from(k.0)) } < 0;
    let mods = eframe::egui::Modifiers { ctrl: held(VK_CONTROL), alt: held(VK_MENU), shift: held(VK_SHIFT), command: held(VK_CONTROL), mac_cmd: false };
    let Some(key) = egui_key(vk) else { return };
    if crate::browser::app_key(key, mods) {
        // SAFETY: the event's setter, during the event.
        unsafe {
            let _ = args.SetHandled(true);
        }
        sink.push(Event::Key(key, mods));
    }
}

/// A Windows virtual key as egui's.
fn egui_key(vk: u32) -> Option<eframe::egui::Key> {
    use eframe::egui::Key;
    Some(match vk {
        0x30..=0x39 => Key::from_name(&char::from(vk as u8).to_string())?,
        0x41..=0x5A => Key::from_name(&char::from(vk as u8).to_string())?,
        0x70..=0x7B => [Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7, Key::F8, Key::F9, Key::F10, Key::F11, Key::F12][(vk - 0x70) as usize],
        0xBB => Key::Equals,
        0xBD => Key::Minus,
        0xBC => Key::Comma,
        0xBE => Key::Period,
        0xBF => Key::Slash,
        0xDB => Key::OpenBracket,
        0xDD => Key::CloseBracket,
        0xC0 => Key::Backtick,
        0x09 => Key::Tab,
        0x0D => Key::Enter,
        0x1B => Key::Escape,
        0x6B => Key::Plus,
        0x6D => Key::Minus,
        _ => return None,
    })
}

/// A rectangle in egui points as the web view's bounds.
pub fn bounds(rect: eframe::egui::Rect, zoom: f32) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(f64::from(rect.left() * zoom), f64::from(rect.top() * zoom)).into(),
        size: wry::dpi::LogicalSize::new(f64::from(rect.width() * zoom), f64::from(rect.height() * zoom)).into(),
    }
}

/// A token no page guesses, for a script's answer.
fn token() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    h.write_u128(Instant::now().elapsed().as_nanos());
    h.finish() | 1
}

/// Runs `body` (the body of an async function) in the page and hands what it returns (a string)
/// to `done`, once. The page posts the answer back with the token of the call.
pub fn run_js_in(page: &Page, body: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    let t = token();
    let script = format!(
        "(async () => {{\n{body}\n}})().then(r => window.ipc.postMessage('ret:{t}:' + (typeof r === 'string' ? '1:' + r : '0:the page returned nothing')), e => window.ipc.postMessage('ret:{t}:0:' + String(e && e.message || e)));"
    );
    RETURNS.with_borrow_mut(|r| {
        // Answers that never came (the page went away): forgotten.
        r.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(120));
        r.insert(t, (Instant::now(), Box::new(done)));
    });
    let handler = ExecuteScriptCompletedHandler::create(Box::new(move |error, _| {
        if let Err(e) = error
            && let Some((_, done)) = RETURNS.with_borrow_mut(|r| r.remove(&t))
        {
            done(Err(e.message()));
        }
        Ok(())
    }));
    // SAFETY: WebView2's script call on the window's thread.
    if let Err(e) = unsafe { page.wv.ExecuteScript(&HSTRING::from(script), &handler) }
        && let Some((_, done)) = RETURNS.with_borrow_mut(|r| r.remove(&t))
    {
        done(Err(e.message()));
    }
}

/// [`run_js_in`] of a view.
pub fn run_js(view: &View, body: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    run_js_in(&view.page, body, done);
}

/// The answer of a script (`token:ok:text`, posted by the page).
fn returned(rest: &str) {
    let mut parts = rest.splitn(3, ':');
    let (Some(t), Some(ok), Some(text)) = (parts.next().and_then(|t| t.parse::<u64>().ok()), parts.next(), parts.next()) else { return };
    if let Some((_, done)) = RETURNS.with_borrow_mut(|r| r.remove(&t)) {
        done(if ok == "1" { Ok(text.to_string()) } else { Err(text.to_string()) });
    }
}

/// The page as it shows now, as a PNG, handed to `done` (once).
pub fn snapshot_view(page: &Page, _width: Option<f64>, done: impl FnOnce(Option<Vec<u8>>) + 'static) {
    use windows::Win32::UI::Shell::SHCreateMemStream;
    // SAFETY: a new stream in memory.
    let Some(stream) = (unsafe { SHCreateMemStream(None) }) else { return done(None) };
    let s = stream.clone();
    let done = RefCell::new(Some(done));
    let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
        let png = result.ok().and_then(|_| read_stream(&s));
        if let Some(f) = done.borrow_mut().take() {
            f(png);
        }
        Ok(())
    }));
    // SAFETY: WebView2's capture on the window's thread, into the stream.
    let _ = unsafe { page.wv.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &handler) };
}

fn read_stream(stream: &windows::Win32::System::Com::IStream) -> Option<Vec<u8>> {
    use windows::Win32::System::Com::{STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET};
    // SAFETY: the stream's own calls; the buffer is as long as the read.
    unsafe {
        let mut stat = STATSTG::default();
        stream.Stat(&mut stat, STATFLAG_NONAME).ok()?;
        let len = usize::try_from(stat.cbSize).ok()?;
        stream.Seek(0, STREAM_SEEK_SET, None).ok()?;
        let mut buf = vec![0u8; len];
        let mut read = 0u32;
        stream.Read(buf.as_mut_ptr().cast(), u32::try_from(len).ok()?, Some(&mut read)).ok().ok()?;
        buf.truncate(read as usize);
        (!buf.is_empty()).then_some(buf)
    }
}

/// [`snapshot_view`] of a view.
pub fn snapshot_at(view: &View, width: Option<f64>, done: impl FnOnce(Option<Vec<u8>>) + 'static) {
    snapshot_view(&view.page, width, done);
}

/// Whether a page is loading (from the request on).
pub fn loading_in(page: &Page) -> bool {
    page.loading.get()
}

/// [`loading_in`] of a view.
pub fn is_loading(view: &View) -> bool {
    view.page.loading.get()
}

/// A page's address ("" before it has one).
pub fn url_in(page: &Page) -> String {
    let mut uri = PWSTR::null();
    // SAFETY: WebView2's getter; the string is taken (freed) after.
    if unsafe { page.wv.Source(&mut uri) }.is_err() {
        return String::new();
    }
    webview2_com::take_pwstr(uri)
}

/// Goes back a page.
pub fn back_in(page: &Page) {
    // SAFETY: WebView2's own call on the window's thread.
    let _ = unsafe { page.wv.GoBack() };
}

/// The page's size in CSS pixels.
pub fn css_size(page: &Page) -> (f64, f64) {
    let mut r = windows::Win32::Foundation::RECT::default();
    let (mut zoom, mut scale) = (1.0f64, 1.0f64);
    // SAFETY: the controller's getters on the window's thread.
    unsafe {
        let _ = page.controller.Bounds(&mut r);
        let _ = page.controller.ZoomFactor(&mut zoom);
        if let Ok(c3) = page.controller.cast::<ICoreWebView2Controller3>() {
            let _ = c3.RasterizationScale(&mut scale);
        }
    }
    let f = (zoom * scale).max(0.01);
    (f64::from(r.right - r.left) / f, f64::from(r.bottom - r.top) / f)
}

/// Takes the page back from a site an agent may not open yet.
pub fn turn_back(view: &View) {
    // SAFETY: WebView2's own calls on the window's thread.
    unsafe {
        let _ = view.page.wv.Stop();
        let _ = view.page.wv.GoBack();
    }
}

/// Prints the page (the system's print dialog).
pub fn print(view: &View) {
    let _ = view.web.print();
}

/// Whether the page has the keyboard (the focused window is in it).
pub fn view_has_keyboard(page: &Page) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    use windows::Win32::UI::WindowsAndMessaging::IsChild;
    if page.keyboard.get() {
        return true;
    }
    // SAFETY: plain window queries.
    unsafe {
        let f = GetFocus();
        !f.is_invalid() && (f == page.hwnd || IsChild(page.hwnd, f).as_bool())
    }
}

/// [`view_has_keyboard`] of a view.
pub fn has_keyboard(view: &View) -> bool {
    view_has_keyboard(&view.page)
}

/// Whether the user's click put the keyboard in the page (a page saying it was clicked is
/// believed only then).
pub fn clicked_in(view: &View) -> bool {
    has_keyboard(view)
}

/// Dims the page under a question: not on Windows (a child window has no opacity).
pub fn dim(_view: &View, _dimmed: bool) {}

/// Editing commands go to the page by its own keys (it has the keyboard).
pub fn edit_command(view: &View, command: &str) {
    let js = format!("document.execCommand({}); return '';", serde_json::to_string(command).unwrap_or_default());
    run_js(view, &js, |_| {});
}

/// The process the page draws in: WebView2's are shared, not the page's own.
pub fn web_pid(_view: &View) -> Option<i32> {
    None
}

/// Forgets the pages' cookies, logins, cache and storage (the one profile every page shares).
pub fn clear_data(views: &[&View]) {
    if let Some(v) = views.first() {
        let _ = v.web.clear_all_browsing_data();
    } else {
        // No page up: the profile's folder goes (made again with the next page).
        let _ = std::fs::remove_dir_all(crate::paths::data_dir().join(DATA));
    }
}

// Dialogs (`browser_dialog` on Windows).

fn dialog_opening(id: PaneId, wv: Option<ICoreWebView2>, args: ICoreWebView2ScriptDialogOpeningEventArgs, sink: &Sink) {
    let mut kind = COREWEBVIEW2_SCRIPT_DIALOG_KIND::default();
    let (mut message, mut text, mut uri) = (PWSTR::null(), PWSTR::null(), PWSTR::null());
    // SAFETY: the event's own getters, during the event; the strings are taken after.
    let deferral = unsafe {
        let _ = args.Kind(&mut kind);
        let _ = args.Message(&mut message);
        let _ = args.DefaultText(&mut text);
        let _ = args.Uri(&mut uri);
        args.GetDeferral()
    };
    let (message, text, uri) = (webview2_com::take_pwstr(message), webview2_com::take_pwstr(text), webview2_com::take_pwstr(uri));
    let _ = wv;
    let Ok(deferral) = deferral else { return };
    let kind = match kind {
        COREWEBVIEW2_SCRIPT_DIALOG_KIND_CONFIRM | COREWEBVIEW2_SCRIPT_DIALOG_KIND_BEFOREUNLOAD => DialogKind::Confirm,
        COREWEBVIEW2_SCRIPT_DIALOG_KIND_PROMPT => DialogKind::Prompt,
        _ => DialogKind::Alert,
    };
    let message = if message.is_empty() && kind == DialogKind::Confirm { crate::i18n::t("Leave this page? Changes you made may not be saved.").to_string() } else { message };
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let did = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    WAITING.with_borrow_mut(|d| d.insert(did, (id, args, deferral)));
    sink.push(Event::Dialog(Dialog { id: did, kind, host: crate::browser::host_of(&uri), message, text }));
}

/// Answers a waiting dialog: OK (`accept`, with `text` for a prompt) or Cancel. Returns whether
/// it was waiting.
pub fn answer(id: u64, accept: bool, text: &str) -> bool {
    let Some((_, args, deferral)) = WAITING.with_borrow_mut(|d| d.remove(&id)) else { return false };
    // SAFETY: the dialog's own calls, then its deferral ends it.
    unsafe {
        if accept {
            let _ = args.SetResultText(&HSTRING::from(text));
            let _ = args.Accept();
        }
        let _ = deferral.Complete();
    }
    true
}

/// Dialogs of pages gone are answered Cancel (`live`: the pages up now); pop-ups their page closed
/// close.
pub fn tidy(live: &[PaneId]) {
    tidy_popups();
    let gone: Vec<u64> = WAITING.with_borrow(|d| d.iter().filter(|(_, (p, ..))| !live.contains(p)).map(|(id, _)| *id).collect());
    for id in gone {
        answer(id, false, "");
    }
}

// An agent's input (`browser_input` on Windows): the DevTools Protocol's, trusted as a person's.

/// Sends a DevTools Protocol call to the page (its answer is not waited for; calls go in order).
fn cdp(page: &Page, method: &str, params: serde_json::Value) {
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
    // SAFETY: WebView2's call on the window's thread.
    let _ = unsafe { page.wv.CallDevToolsProtocolMethod(&HSTRING::from(method), &HSTRING::from(params.to_string()), &handler) };
}

/// Keys reach the page's focused element without the window's keyboard (the user's typing stays
/// where it is).
fn emulate_focus(page: &Page) {
    if !page.emulating.replace(true) {
        cdp(page, "Emulation.setFocusEmulationEnabled", serde_json::json!({ "enabled": true }));
    }
}

fn mouse(page: &Page, ty: &str, x: f64, y: f64, button: &str, count: i32) {
    let buttons = if ty == "mouseReleased" || button == "none" { 0 } else { 1 };
    cdp(page, "Input.dispatchMouseEvent", serde_json::json!({ "type": ty, "x": x, "y": y, "button": button, "buttons": buttons, "clickCount": count }));
}

/// The pointer moves to (`x`, `y`) of the page (CSS pixels from its top left).
pub fn hover(page: &Page, x: f64, y: f64) -> bool {
    mouse(page, "mouseMoved", x - 1.0, y, "none", 0);
    mouse(page, "mouseMoved", x, y, "none", 0);
    true
}

/// A click (`count` 2: a double click) at (`x`, `y`) of the page.
pub fn click(page: &Page, x: f64, y: f64, count: isize) -> bool {
    emulate_focus(page);
    mouse(page, "mouseMoved", x, y, "none", 0);
    for n in 1..=count.max(1) as i32 {
        mouse(page, "mousePressed", x, y, "left", n);
        mouse(page, "mouseReleased", x, y, "left", n);
    }
    true
}

/// A drag with the button held from one point of the page to another, in steps.
pub fn drag(page: &Page, from: (f64, f64), to: (f64, f64)) -> bool {
    mouse(page, "mouseMoved", from.0, from.1, "none", 0);
    mouse(page, "mousePressed", from.0, from.1, "left", 1);
    const STEPS: usize = 12;
    for i in 1..=STEPS {
        let f = i as f64 / STEPS as f64;
        cdp(page, "Input.dispatchMouseEvent", serde_json::json!({ "type": "mouseMoved", "x": from.0 + (to.0 - from.0) * f, "y": from.1 + (to.1 - from.1) * f, "button": "left", "buttons": 1 }));
    }
    mouse(page, "mouseReleased", to.0, to.1, "left", 1);
    true
}

/// A named key (`Enter`, `Tab`, `ArrowDown`…) as the protocol has it: key, code, Windows key
/// code, text.
fn named(key: &str) -> Option<(&'static str, &'static str, u32, &'static str)> {
    Some(match key {
        "Enter" => ("Enter", "Enter", 13, "\r"),
        "Tab" => ("Tab", "Tab", 9, "\t"),
        "Escape" | "Esc" => ("Escape", "Escape", 27, ""),
        "Backspace" => ("Backspace", "Backspace", 8, ""),
        "Delete" => ("Delete", "Delete", 46, ""),
        " " | "Space" => (" ", "Space", 32, " "),
        "ArrowUp" => ("ArrowUp", "ArrowUp", 38, ""),
        "ArrowDown" => ("ArrowDown", "ArrowDown", 40, ""),
        "ArrowLeft" => ("ArrowLeft", "ArrowLeft", 37, ""),
        "ArrowRight" => ("ArrowRight", "ArrowRight", 39, ""),
        "Home" => ("Home", "Home", 36, ""),
        "End" => ("End", "End", 35, ""),
        "PageUp" => ("PageUp", "PageUp", 33, ""),
        "PageDown" => ("PageDown", "PageDown", 34, ""),
        _ => return None,
    })
}

fn press_char(page: &Page, c: char) {
    let s = c.to_string();
    let (code, vk) = match c.to_ascii_uppercase() {
        u @ 'A'..='Z' => (format!("Key{u}"), u32::from(u)),
        d @ '0'..='9' => (format!("Digit{d}"), u32::from(d)),
        _ => (String::new(), 0),
    };
    cdp(page, "Input.dispatchKeyEvent", serde_json::json!({ "type": "keyDown", "key": s, "code": code, "windowsVirtualKeyCode": vk, "text": s, "unmodifiedText": s }));
    cdp(page, "Input.dispatchKeyEvent", serde_json::json!({ "type": "keyUp", "key": s, "code": code, "windowsVirtualKeyCode": vk }));
}

/// Presses a key (`Enter`, `Tab`, `a`…) in the page. Err: a key this does not know.
pub fn key(page: &Page, key: &str) -> Result<(), String> {
    emulate_focus(page);
    if let Some((k, code, vk, text)) = named(key) {
        let mut down = serde_json::json!({ "type": if text.is_empty() { "rawKeyDown" } else { "keyDown" }, "key": k, "code": code, "windowsVirtualKeyCode": vk });
        if !text.is_empty() {
            down["text"] = text.into();
        }
        cdp(page, "Input.dispatchKeyEvent", down);
        cdp(page, "Input.dispatchKeyEvent", serde_json::json!({ "type": "keyUp", "key": k, "code": code, "windowsVirtualKeyCode": vk }));
        return Ok(());
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => {
            press_char(page, c);
            Ok(())
        }
        _ => Err(format!("Unknown key {key:?}: use Enter, Tab, Escape, Backspace, Delete, Space, ArrowUp/Down/Left/Right, Home, End, PageUp, PageDown, or one character.")),
    }
}

/// Types `text` into what has the keyboard in the page, a key at a time (a line break is Enter).
pub fn type_text(page: &Page, text: &str) -> bool {
    emulate_focus(page);
    for c in text.chars() {
        match c {
            '\n' => {
                let _ = key(page, "Enter");
            }
            '\t' => {
                let _ = key(page, "Tab");
            }
            c => press_char(page, c),
        }
    }
    true
}

/// The next file chooser of `page` takes these files (an agent's upload): choosers are caught
/// (no Explorer dialog) until it comes.
pub fn upload_next(page: &Page, files: Vec<std::path::PathBuf>) {
    UPLOADS.with_borrow_mut(|u| u.insert(page_key(page), (Instant::now(), files)));
    cdp(page, "Page.enable", serde_json::json!({}));
    cdp(page, "Page.setInterceptFileChooserDialog", serde_json::json!({ "enabled": true }));
}

/// Whether files given to `page`'s next chooser are still waiting (no chooser came): they are
/// taken back, and choosers are the user's again.
pub fn upload_unused(page: &Page) -> bool {
    let unused = UPLOADS.with_borrow_mut(|u| u.remove(&page_key(page)).is_some());
    if unused {
        cdp(page, "Page.setInterceptFileChooserDialog", serde_json::json!({ "enabled": false }));
    }
    unused
}

fn chooser_opened(wv: &ICoreWebView2, key: usize, json: &str) {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let files = UPLOADS.with_borrow_mut(|u| u.remove(&key)).map(|(_, f)| f);
    let send = |method: &str, params: serde_json::Value| {
        let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
        // SAFETY: WebView2's call on the window's thread.
        let _ = unsafe { wv.CallDevToolsProtocolMethod(&HSTRING::from(method), &HSTRING::from(params.to_string()), &handler) };
    };
    if let Some(files) = files {
        let many = v["mode"].as_str() == Some("selectMultiple");
        let files: Vec<String> = files.iter().take(if many { usize::MAX } else { 1 }).map(|f| f.to_string_lossy().to_string()).collect();
        send("DOM.setFileInputFiles", serde_json::json!({ "files": files, "backendNodeId": v["backendNodeId"] }));
    }
    send("Page.setInterceptFileChooserDialog", serde_json::json!({ "enabled": false }));
}

// Sign-in pop-ups (`browser_popup` on Windows): windows of the app's, each with a WebView2 in the
// environment of the page that opened it (so `window.opener` ties them).

/// An agent acting in a page now: the sites it may open.
struct Acting {
    page: PaneId,
    sites: Vec<String>,
    pass: Option<String>,
    at: Instant,
}

struct Popup {
    opener: PaneId,
    /// The pop-up's window (the app's).
    window: HWND,
    controller: ICoreWebView2Controller,
    page: Rc<Page>,
}

thread_local! {
    static ACTING: RefCell<Option<Acting>> = const { RefCell::new(None) };
    static BLOCKED: RefCell<HashMap<PaneId, String>> = RefCell::new(HashMap::new());
    static POPUPS: RefCell<Vec<Popup>> = const { RefCell::new(Vec::new()) };
    /// The pages' panes, by their key (`page_key`).
    static KEYS: RefCell<HashMap<usize, PaneId>> = RefCell::new(HashMap::new());
}

/// The agent's step acts in `page` now: a pop-up it opens is the agent's, to its sites only.
pub fn agent_acting(page: &Page, sites: Vec<String>, pass: Option<String>) {
    ACTING.with_borrow_mut(|a| *a = Some(Acting { page: page.id, sites, pass, at: Instant::now() }));
}

/// The user took the browser over: pop-ups are theirs.
pub fn agent_stopped() {
    ACTING.with_borrow_mut(|a| *a = None);
}

/// A pop-up the agent's step on the page (`page_key`) opened to a site it may not open: its
/// address.
pub fn take_blocked(page: usize) -> Option<String> {
    let pane = KEYS.with_borrow(|k| k.get(&page).copied())?;
    BLOCKED.with_borrow_mut(|b| b.remove(&pane))
}

/// Pop-ups whose page closed them (`window.close()`: WebView2 closed wry's window in it) close.
fn tidy_popups() {
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    let closed: Vec<Popup> = POPUPS.with_borrow_mut(|p| {
        let mut gone = Vec::new();
        let mut i = 0;
        while i < p.len() {
            // SAFETY: plain window query on the window's thread.
            let open = unsafe { IsWindow(Some(p[i].window)).as_bool() };
            if open {
                i += 1;
            } else {
                gone.push(p.remove(i));
            }
        }
        gone
    });
    for pop in closed {
        // SAFETY: WebView2 COM call outside the POPUPS borrow.
        unsafe { let _ = pop.controller.Close(); }
    }
}

/// Closes the pop-ups a pane's page opened (its agent went to another address, or the pane closed).
pub fn close_of(pane: PaneId) {
    use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;
    let closing: Vec<Popup> = POPUPS.with_borrow_mut(|p| {
        let mut gone = Vec::new();
        let mut i = 0;
        while i < p.len() {
            if p[i].opener == pane {
                gone.push(p.remove(i));
            } else {
                i += 1;
            }
        }
        gone
    });
    for pop in closing {
        // SAFETY: closed outside POPUPS borrow so WM_SIZE during teardown cannot re-enter mutably.
        unsafe {
            let _ = pop.controller.Close();
            let _ = DestroyWindow(pop.window);
        }
    }
}

/// The newest pop-up a pane's page opened, still open.
pub fn popup_of(pane: PaneId) -> Option<PageRc> {
    tidy_popups();
    POPUPS.with_borrow(|p| p.iter().rev().find(|pop| pop.opener == pane).map(|pop| pop.page.clone()))
}

unsafe extern "system" fn popup_proc(hwnd: HWND, msg: u32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, GetClientRect, WM_SIZE};
    if msg == WM_SIZE {
        // The page fills the window.
        let c = POPUPS.with_borrow(|p| p.iter().find(|pop| pop.window == hwnd).map(|pop| pop.controller.clone()));
        if let Some(c) = c {
            let mut r = windows::Win32::Foundation::RECT::default();
            // SAFETY: the window's own size, and its page's bounds, on the window's thread.
            unsafe {
                let _ = GetClientRect(hwnd, &mut r);
                let _ = c.SetBounds(r);
            }
        }
    }
    // SAFETY: the default handling of the window's messages (WM_CLOSE destroys it).
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A page's new windows: a window of a size (Continue with Google, Sign in with Apple) is a
/// pop-up window of the app's, tied to the page (`window.opener`), where an agent's steps act
/// too; anything else (a link to a new tab) a new page in the panel. The pop-up's WebView2 is
/// made while the page waits (a deferral), not in the event: making it there blocks.
fn new_windows(page: &Page, owner: HWND, env: ICoreWebView2Environment, sink: Sink) {
    use webview2_com::NewWindowRequestedEventHandler;
    let opener = page.id;
    let mut token = 0i64;
    let handler = NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut uri = PWSTR::null();
        let (mut sized, mut placed, mut w, mut h) = (windows::core::BOOL::default(), windows::core::BOOL::default(), 0u32, 0u32);
        // SAFETY: the event's own getters, during the event.
        unsafe {
            args.Uri(&mut uri)?;
            if let Ok(f) = args.WindowFeatures() {
                let _ = f.HasSize(&mut sized);
                let _ = f.HasPosition(&mut placed);
                let _ = f.Width(&mut w);
                let _ = f.Height(&mut h);
            }
        }
        let url = webview2_com::take_pwstr(uri);
        if !sized.as_bool() && !placed.as_bool() {
            sink.push(Event::OpenHere(url));
            // SAFETY: the event's setter, during the event.
            return unsafe { args.SetHandled(true) };
        }
        // An agent's step a moment ago opened it: to its sites only (else the user is asked).
        let agent = ACTING.with_borrow(|a| a.as_ref().filter(|a| a.page == opener && a.at.elapsed() < Duration::from_secs(3)).map(|a| crate::browser::agent_may_open(&url, &a.sites, a.pass.as_deref())));
        if agent == Some(false) {
            BLOCKED.with_borrow_mut(|b| b.insert(opener, url));
            // SAFETY: as above.
            return unsafe { args.SetHandled(true) };
        }
        let size = sized.as_bool().then_some((f64::from(w), f64::from(h)));
        new_popup(owner, opener, &env, args, size, agent.is_some());
        Ok(())
    }));
    // SAFETY: the page's own event, on the window's thread; the handler lives with the page.
    unsafe {
        let _ = page.wv.add_NewWindowRequested(&handler, &mut token);
    }
}

/// A pop-up window over the app's (`owner`), `size` (CSS pixels) or a sign-in window's; its
/// WebView2 (in the opener's environment) is handed to the page when it is made. An agent's
/// pop-up does not take the keyboard.
fn new_popup(owner: HWND, opener: PaneId, env: &ICoreWebView2Environment, args: ICoreWebView2NewWindowRequestedEventArgs, size: Option<(f64, f64)>, agent: bool) {
    use webview2_com::{AddScriptToExecuteOnDocumentCreatedCompletedHandler, CreateCoreWebView2ControllerCompletedHandler, DocumentTitleChangedEventHandler, WebMessageReceivedEventHandler, WindowCloseRequestedEventHandler};
    use windows::Win32::Foundation::RECT;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::*;
    // SAFETY: the event's deferral (the page waits for the new window until it completes).
    let Ok(deferral) = (unsafe { args.GetDeferral() }) else { return };
    // SAFETY: the window class is registered once; the window is made and shown on the window's
    // thread, owned by the app's window.
    let window = unsafe {
        let made = (|| {
            let module = GetModuleHandleW(None).ok()?;
            let class = windows::core::w!("TernaryCodePopup");
            static REGISTERED: std::sync::Once = std::sync::Once::new();
            REGISTERED.call_once(|| {
                let wc = WNDCLASSW { lpfnWndProc: Some(popup_proc), hInstance: module.into(), lpszClassName: class, hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(), ..Default::default() };
                RegisterClassW(&wc);
            });
            let scale = f64::from(GetDpiForWindow(owner).max(96)) / 96.0;
            let (w, h) = size.unwrap_or((500.0, 600.0));
            let (w, h) = ((w.clamp(320.0, 1400.0) * scale) as i32 + 16, (h.clamp(240.0, 1000.0) * scale) as i32 + 39);
            let mut r = RECT::default();
            let _ = GetWindowRect(owner, &mut r);
            let (x, y) = (r.left + ((r.right - r.left) - w) / 2, r.top + ((r.bottom - r.top) - h) / 2);
            let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), class, windows::core::w!(""), WS_OVERLAPPEDWINDOW, x, y, w, h, Some(owner), None, Some(module.into()), None).ok()?;
            let _ = ShowWindow(hwnd, if agent { SW_SHOWNOACTIVATE } else { SW_SHOW });
            Some(hwnd)
        })();
        match made {
            Some(w) => w,
            None => {
                let _ = deferral.Complete();
                return;
            }
        }
    };
    let done = deferral.clone();
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |result, controller| {
        let made = result.ok().and(controller).and_then(|c| {
            // SAFETY: the new controller's calls, on the window's thread.
            unsafe {
                let wv = c.CoreWebView2().ok()?;
                let mut r = RECT::default();
                let _ = GetClientRect(window, &mut r);
                let _ = c.SetBounds(r);
                let _ = c.SetIsVisible(true);
                // The agents' helper and `window.ipc` (scripts' answers), as wry gives its pages.
                let noop = || AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(|_, _| Ok(())));
                let _ = wv.AddScriptToExecuteOnDocumentCreated(&HSTRING::from("window.ipc = { postMessage: function (s) { window.chrome.webview.postMessage(s); } };"), &noop());
                let _ = wv.AddScriptToExecuteOnDocumentCreated(&HSTRING::from(crate::browser::AGENT_JS), &noop());
                let mut token = 0i64;
                let _ = wv.add_WebMessageReceived(
                    &WebMessageReceivedEventHandler::create(Box::new(|_, args| {
                        let Some(args) = args else { return Ok(()) };
                        let mut m = PWSTR::null();
                        if args.TryGetWebMessageAsString(&mut m).is_ok()
                            && let Some(rest) = webview2_com::take_pwstr(m).strip_prefix("ret:")
                        {
                            returned(rest);
                        }
                        Ok(())
                    })),
                    &mut token,
                );
                let _ = wv.add_DocumentTitleChanged(
                    &DocumentTitleChangedEventHandler::create(Box::new(move |wv, _| {
                        let mut t = PWSTR::null();
                        if let Some(wv) = wv
                            && wv.DocumentTitle(&mut t).is_ok()
                        {
                            let _ = SetWindowTextW(window, &HSTRING::from(webview2_com::take_pwstr(t)));
                        }
                        Ok(())
                    })),
                    &mut token,
                );
                // `window.close()` in the pop-up closes its window.
                let _ = wv.add_WindowCloseRequested(
                    &WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
                        let _ = DestroyWindow(window);
                        Ok(())
                    })),
                    &mut token,
                );
                let _ = args.SetNewWindow(&wv);
                let _ = args.SetHandled(true);
                Some((c, wv))
            }
        });
        match made {
            Some((controller, wv)) => {
                let loading = Rc::new(Cell::new(true));
                let page = Rc::new(Page { wv, controller: controller.clone(), hwnd: window, loading: loading.clone(), id: opener, emulating: Cell::new(false), keyboard: Rc::new(Cell::new(false)) });
                KEYS.with_borrow_mut(|k| k.insert(page_key(&page), opener));
                watch(&page, loading, None);
                POPUPS.with_borrow_mut(|p| p.push(Popup { opener, window, controller, page }));
            }
            None => {
                // SAFETY: the window just made, on its thread.
                unsafe {
                    let _ = DestroyWindow(window);
                }
            }
        }
        // SAFETY: the event's deferral, completed once.
        unsafe { done.Complete() }
    }));
    // SAFETY: the opener's environment makes the new controller in the pop-up's window.
    if unsafe { env.CreateCoreWebView2Controller(window, &handler) }.is_err() {
        // SAFETY: as above.
        unsafe {
            let _ = DestroyWindow(window);
            let _ = deferral.Complete();
        }
    }
}
