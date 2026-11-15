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
    /// The page asked for a new window: opened here instead.
    OpenHere(String),
}

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

/// Where a page's callbacks put what they report, and wake the window.
#[derive(Clone)]
pub struct Sink {
    events: Arc<Mutex<Vec<Event>>>,
    ctx: eframe::egui::Context,
}

impl Sink {
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
    /// When it was last in view.
    pub last_seen: Instant,
    /// The page was let go to save memory (loaded again when shown).
    pub asleep: bool,
    /// The agent pane this browser was opened for or taken by: its tools act here.
    pub owner: Option<PaneId>,
    /// An agent using the page now.
    pub driver: Option<Driver>,
    /// A site waiting for the user's OK.
    pub ask: Option<Ask>,
    /// A host the user let the agent open once: its next load is not stopped.
    pub pass: Option<String>,
    events: Arc<Mutex<Vec<Event>>>,
    #[cfg(target_os = "macos")]
    pub(crate) view: Option<wry::WebView>,
    /// Drawn (shown on screen) after the last frame.
    pub(crate) shown: bool,
    /// Where its web view was last laid (moved only when that changes).
    pub(crate) placed: Option<eframe::egui::Rect>,
    /// Laid outside the window: out of view while an agent uses it (it keeps drawing, so its
    /// screenshots and timers work).
    pub(crate) offscreen: bool,
    /// Dimmed under a question.
    pub(crate) dimmed: bool,
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
            owner: None,
            driver: None,
            ask: None,
            pass: None,
            events: Arc::default(),
            #[cfg(target_os = "macos")]
            view: None,
            shown: false,
            placed: None,
            offscreen: false,
            dimmed: false,
        }
    }

    /// An agent uses it now (the user has not taken it over).
    pub fn driven(&self) -> bool {
        self.driver.as_ref().is_some_and(|d| !d.paused)
    }

    pub fn sink(&self, ctx: &eframe::egui::Context) -> Sink {
        Sink { events: self.events.clone(), ctx: ctx.clone() }
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
                Event::Title(t) => self.title = t.clone(),
                Event::Loading(l) => self.loading = *l,
                _ => {}
            }
        }
        events
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
        #[cfg(target_os = "macos")]
        if let Some(v) = &self.view {
            let _ = v.load_url(&url);
        }
    }

    pub fn back(&self) {
        #[cfg(target_os = "macos")]
        if let Some(v) = &self.view {
            let _ = v.go_back();
        }
    }

    pub fn forward(&self) {
        #[cfg(target_os = "macos")]
        if let Some(v) = &self.view {
            let _ = v.go_forward();
        }
    }

    pub fn reload(&self) {
        #[cfg(target_os = "macos")]
        if let Some(v) = &self.view {
            let _ = v.reload();
        }
    }

    /// Lets the page go (memory); it loads again when shown.
    pub fn sleep(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.view = None;
        }
        self.placed = None;
        self.shown = false;
        self.offscreen = false;
        self.dimmed = false;
        self.asleep = true;
        self.loading = false;
    }

    /// The page has a web view (it is not asleep).
    pub fn live(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.view.is_some()
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}

/// How long a browser out of view and unused keeps its page.
pub fn sleep_after(minutes: u32) -> Duration {
    Duration::from_secs(u64::from(minutes.max(1)) * 60)
}

/// The host (with its port) and the rest of an address, for the address field.
pub fn split_url(url: &str) -> (String, String) {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    match rest.find('/') {
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

/// Whether an agent may open `url` without asking: this machine's addresses, local files and
/// blank pages, the project's own sites (`sites`, each with its subdomains), and `pass` (a
/// host the user let it open once).
pub fn agent_may_open(url: &str, sites: &[String], pass: Option<&str>) -> bool {
    let scheme = url.split_once(':').map_or("", |(s, _)| s).to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https") {
        return matches!(scheme.as_str(), "about" | "file" | "data" | "blob");
    }
    let host = host_of(url);
    local_host(&host)
        || pass == Some(host.as_str())
        || sites.iter().any(|s| host == *s || host.ends_with(&format!(".{s}")))
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
/// own keyboard focus. An agent's clicks are not the user's (they are not trusted events).
#[cfg(target_os = "macos")]
const FOCUS_SCRIPT: &str = "window.addEventListener('pointerdown', function (e) { if (!e.isTrusted) return; try { window.ipc.postMessage('focus'); } catch (x) {} }, true);";

/// The agents' helper in the page (`browser_agent.js`), from the start of every page so that
/// it hears the console from the first line.
pub const AGENT_JS: &str = include_str!("browser_agent.js");

/// Safari's own name: some sites refuse a web view that does not say it is Safari.
#[cfg(target_os = "macos")]
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.4 Safari/605.1.15";

/// The page as a web view laid over the window at `bounds` (logical points from the top left of
/// the window's content).
#[cfg(target_os = "macos")]
pub fn build_view(
    window: &impl raw_window_handle::HasWindowHandle,
    url: &str,
    bounds: wry::Rect,
    sink: Sink,
) -> Option<wry::WebView> {
    use wry::{NewWindowResponse, PageLoadEvent, WebViewBuilder};
    let (s1, s2, s3, s4) = (sink.clone(), sink.clone(), sink.clone(), sink);
    let url = if url.is_empty() { "about:blank" } else { url };
    WebViewBuilder::new()
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
        .with_new_window_req_handler(move |u, _| {
            s3.push(Event::OpenHere(u));
            NewWindowResponse::Deny
        })
        .with_ipc_handler(move |req| {
            if req.body() == "focus" {
                s4.push(Event::Focused);
            }
        })
        .build_as_child(window)
        .ok()
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
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError};
    use wry::WebViewExtMacOS;
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
    let wk = view.webview();
    // SAFETY: a snapshot of the whole visible page; the block lives as long as WebKit keeps it.
    unsafe { wk.takeSnapshotWithConfiguration_completionHandler(None, &block) };
}

/// Runs `body` (the body of an async function) in the page and hands what it returns, as a
/// string, to `done` (on the main thread, once). An error: the script failed to run, or the
/// page went away meanwhile (it navigated).
#[cfg(target_os = "macos")]
pub fn run_js(view: &wry::WebView, body: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSError, NSString};
    use objc2_web_kit::WKContentWorld;
    use wry::WebViewExtMacOS;
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
    let wk = view.webview();
    // SAFETY: on the main thread; the page's own world (where the helper lives), the main frame,
    // no arguments; the block lives as long as WebKit keeps it.
    unsafe {
        let world = WKContentWorld::pageWorld(mtm);
        wk.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(&NSString::from_str(body), None, None, &world, Some(&block));
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

/// Whether the page has the keyboard (its web view, or a view in it, is the window's first
/// responder).
#[cfg(target_os = "macos")]
pub fn has_keyboard(view: &wry::WebView) -> bool {
    use objc2_app_kit::NSView;
    use wry::WebViewExtMacOS;
    let wk = view.webview();
    let Some(window) = wk.window() else { return false };
    let Some(first) = window.firstResponder() else { return false };
    first.downcast_ref::<NSView>().is_some_and(|v| v.isDescendantOf(&wk))
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
pub fn clear_data() {
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
        let sites = vec!["stripe.com".to_string()];
        for ok in ["http://localhost:5173/cart", "http://127.0.0.1:8080", "http://app.local/", "http://[::1]:3000", "about:blank", "file:///tmp/a.html", "https://checkout.stripe.com/x", "https://stripe.com"] {
            assert!(agent_may_open(ok, &sites, None), "{ok}");
        }
        for asks in ["https://github.com/", "https://notstripe.com/", "https://stripe.com.evil.io/", "javascript:alert(1)", "ftp://x.org/"] {
            assert!(!agent_may_open(asks, &sites, None), "{asks}");
        }
        assert!(agent_may_open("https://github.com/x", &[], Some("github.com")));
    }
}
