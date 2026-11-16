//! Pop-up windows a page opens: `window.open` with a size, right after the user's click in the
//! page ("Continue with Google", "Sign in with Apple", a payment). Each shows in a small window
//! of its own over the app, as browsers show them, built on the configuration WebKit hands for
//! it, so it stays tied to the page that opened it: the pop-up reports back through
//! `window.opener` and the page goes on signed in. It closes when its page is done
//! (`window.close()`), on ⌘W, or with its close button. Its title bar names the site it is on, so
//! a sign-in shows where a password goes.
//!
//! A link to a new tab, or a window opened by a script on its own, is not a pop-up: it opens as a
//! page in the panel (`Event::OpenHere`).

use std::cell::RefCell;
use std::ptr::NonNull;
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSBackingStoreType, NSEvent, NSEventMask, NSEventModifierFlags, NSMenu, NSWindow, NSWindowDelegate, NSWindowOrderingMode, NSWindowStyleMask};
use objc2_foundation::{NSNotification, NSPoint, NSRect, NSSize, NSString};
use objc2_web_kit::{WKNavigation, WKNavigationAction, WKNavigationDelegate, WKUIDelegate, WKUserContentController, WKWebView, WKWebViewConfiguration, WKWindowFeatures};

use crate::browser::{Event, Sink};

/// How long after the user's click (or key) in a page a window it opens is taken as theirs: a
/// sign-in button may ask its server first.
const USER_ACT: Duration = Duration::from_secs(5);

/// The smallest pop-up, and its distance from the screen's edges.
const MIN_SIZE: (f64, f64) = (320.0, 240.0);
const SCREEN_MARGIN: f64 = 20.0;

struct Popup {
    window: Retained<NSWindow>,
    web: Retained<WKWebView>,
    /// Where the page in the panel that opened it (or opened its opener) hears of new tabs.
    sink: Sink,
}

thread_local! {
    /// The pop-ups open now (main thread only, as AppKit is).
    static POPUPS: RefCell<Vec<Popup>> = const { RefCell::new(Vec::new()) };
    /// One delegate answers for every pop-up (window, page and navigation), kept for the app's
    /// life: AppKit and WebKit do not keep their delegates.
    static DELEGATE: RefCell<Option<Retained<PopupDelegate>>> = const { RefCell::new(None) };
}

/// Whether a request for a new window becomes a pop-up: the page asked for a window of a size
/// or at a place (a link to a new tab does not), right after the user clicked in it or typed.
pub fn wanted(sized: bool, user_acted: bool) -> bool {
    sized && user_acted
}

/// Whether the user just clicked in this page, or typed in it, now.
pub fn user_acted_in(page: &WKWebView) -> bool {
    crate::browser::clicked_in_view(page, USER_ACT) || (crate::browser::typed_within(USER_ACT) && crate::browser::view_has_keyboard(page))
}

/// The size of a pop-up asked for `asked` points (the page's own size when it gave none), kept
/// on a screen of `room` points.
pub fn fit(asked: Option<(f64, f64)>, room: (f64, f64)) -> (f64, f64) {
    let (w, h) = asked.unwrap_or((500.0, 640.0));
    let max_w = (room.0 - 2.0 * SCREEN_MARGIN).max(MIN_SIZE.0);
    let max_h = (room.1 - 2.0 * SCREEN_MARGIN).max(MIN_SIZE.1);
    (w.clamp(MIN_SIZE.0, max_w), h.clamp(MIN_SIZE.1, max_h))
}

/// The title of a pop-up's window: the site it is on, said to be not secure off https.
pub fn title_for(url: &str) -> String {
    let secure = url.starts_with("https://");
    if !secure && !url.starts_with("http://") {
        return url.to_string();
    }
    let host = crate::browser::host_of(url);
    // This machine's own server (a dev server's sign-in) is not flagged.
    let here = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]") || host.ends_with(".localhost");
    if secure || here { host } else { format!("{} — {host}", crate::i18n::t("Not Secure")) }
}

/// Opens a pop-up for `opener`'s request of `url`, `size` points (as the page asked), on the
/// configuration WebKit handed for it (that keeps the two tied). Returns its web view for WebKit
/// to load the request in; `None` when there is no window to show it over.
pub fn open(opener: &WKWebView, url: &str, config: &WKWebViewConfiguration, size: Option<(f64, f64)>, sink: Sink) -> Option<Retained<WKWebView>> {
    let mtm = MainThreadMarker::new()?;
    let parent = opener.window()?;
    let screen = parent.screen()?;
    let room = screen.visibleFrame();
    let (w, h) = fit(size, (room.size.width, room.size.height));
    // Centred over the page that opened it, on its screen.
    let page = parent.convertRectToScreen(opener.convertRect_toView(opener.bounds(), None));
    let x = (page.origin.x + (page.size.width - w) / 2.0).clamp(room.origin.x + SCREEN_MARGIN, (room.origin.x + room.size.width - w - SCREEN_MARGIN).max(room.origin.x));
    let y = (page.origin.y + (page.size.height - h) / 2.0).clamp(room.origin.y + SCREEN_MARGIN, (room.origin.y + room.size.height - h - SCREEN_MARGIN).max(room.origin.y));
    // Not minimized on its own: it rides on the app's window.
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable;
    let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
    // SAFETY: a new window on the main thread. It is not released on close: the list owns it.
    let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(mtm.alloc(), frame, style, NSBackingStoreType::Buffered, false) };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setContentMinSize(NSSize::new(MIN_SIZE.0, MIN_SIZE.1));
    // SAFETY: the configuration WebKit handed for this request (a copy of the opener's, tied to
    // it), given scripts of its own: the panel's (the agents' helper, the click report) are for
    // the panel's pages, and would report this page's console as the opener's.
    let web = unsafe {
        config.setUserContentController(&WKUserContentController::new(mtm));
        WKWebView::initWithFrame_configuration(mtm.alloc(), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h)), config)
    };
    let delegate = delegate(mtm);
    // SAFETY: plain setters on the main thread; the delegate lives as long as the app.
    unsafe {
        web.setCustomUserAgent(Some(&NSString::from_str(crate::browser::USER_AGENT)));
        web.setUIDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        web.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    }
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    window.setContentView(Some(&web));
    window.setTitle(&NSString::from_str(&title_for(url)));
    // Over the app's window, moving with it (and in its full screen space).
    // SAFETY: both windows are alive, on the main thread.
    unsafe { parent.addChildWindow_ordered(&window, NSWindowOrderingMode::Above) };
    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(&web));
    watch_keys();
    POPUPS.with_borrow_mut(|p| p.push(Popup { window, web: web.clone(), sink }));
    Some(web)
}

/// How many pop-ups are open (for tests).
#[cfg(debug_assertions)]
pub fn count() -> usize {
    POPUPS.with_borrow(Vec::len)
}

/// The address and title of the first pop-up's page (for tests).
#[cfg(debug_assertions)]
pub fn first_page() -> Option<(String, String)> {
    // SAFETY: a plain WKWebView getter on the main thread.
    POPUPS.with_borrow(|p| p.first().map(|p| (url_of(&p.web), unsafe { p.web.title() }.map(|t| t.to_string()).unwrap_or_default())))
}

/// The first pop-up's window title (for tests).
#[cfg(debug_assertions)]
pub fn first_title() -> Option<String> {
    POPUPS.with_borrow(|p| p.first().map(|p| p.window.title().to_string()))
}

/// A picture of the first pop-up's page, as a PNG, handed to `done` (for tests).
#[cfg(debug_assertions)]
pub fn snapshot_first(done: impl FnOnce(Option<Vec<u8>>) + 'static) {
    match POPUPS.with_borrow(|p| p.first().map(|p| p.web.clone())) {
        Some(web) => crate::browser::snapshot_view(&web, None, done),
        None => done(None),
    }
}

fn url_of(web: &WKWebView) -> String {
    // SAFETY: a plain WKWebView getter on the main thread.
    unsafe { web.URL() }.and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default()
}

fn delegate(mtm: MainThreadMarker) -> Retained<PopupDelegate> {
    DELEGATE.with_borrow_mut(|d| {
        d.get_or_insert_with(|| {
            let this = mtm.alloc::<PopupDelegate>().set_ivars(());
            // SAFETY: NSObject's init.
            unsafe { msg_send![super(this), init] }
        })
        .clone()
    })
}

/// The pop-up showing `web`, as (its window, the sink of the page that opened it).
fn find(web: &WKWebView) -> Option<(Retained<NSWindow>, Sink)> {
    POPUPS.with_borrow(|p| p.iter().find(|p| std::ptr::eq(&*p.web, web)).map(|p| (p.window.clone(), p.sink.clone())))
}

/// The pop-up whose window is `window`: its page.
fn web_in(window: &NSWindow) -> Option<Retained<WKWebView>> {
    POPUPS.with_borrow(|p| p.iter().find(|p| std::ptr::eq(&*p.window, window)).map(|p| p.web.clone()))
}

/// Names the site in the pop-up's title bar (and the page's title under it).
fn retitle(web: &WKWebView) {
    let Some((window, _)) = find(web) else { return };
    let url = url_of(web);
    if !url.is_empty() {
        window.setTitle(&NSString::from_str(&title_for(&url)));
    }
    // SAFETY: a plain WKWebView getter on the main thread.
    let title = unsafe { web.title() }.map(|t| t.to_string()).unwrap_or_default();
    window.setSubtitle(&NSString::from_str(&title));
}

/// Takes a closed pop-up out of the list. Its window and page are let go once AppKit is done
/// closing it (at the end of this turn of the event loop), not in the middle of it.
fn forget(window: &NSWindow) {
    let gone = POPUPS.with_borrow_mut(|p| p.iter().position(|p| std::ptr::eq(&*p.window, window)).map(|i| p.remove(i)));
    let Some(gone) = gone else { return };
    // SAFETY: plain WebKit and AppKit calls on the main thread, the objects alive.
    unsafe { gone.web.stopLoading() };
    let parent = gone.window.parentWindow();
    if let Some(parent) = &parent {
        parent.removeChildWindow(&gone.window);
    }
    let _ = Retained::autorelease_ptr(gone.web);
    let _ = Retained::autorelease_ptr(gone.window);
    // The keyboard goes back to the app's window.
    if let Some(parent) = parent {
        parent.makeKeyWindow();
    }
}

/// Keys with ⌘ while a pop-up has the keyboard. The app's menu takes those it has first and
/// acts in its own window (⌘V would paste into the page behind, ⌘W close a pane, ⌘K open the
/// palette), so here: the editing ones act in the pop-up's page, ⌘W closes the pop-up, ⌘R loads
/// it again, Quit and Hide stay the app's, the menu's other keys do nothing, and the rest go on
/// to the pop-up's page as typed (⌘← in a field, ⌘[ to go back).
fn watch_keys() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let block = block2::RcBlock::new(|e: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit hands the monitor a live event, on the main thread.
            let ev = unsafe { e.as_ref() };
            let Some(mtm) = MainThreadMarker::new() else { return e.as_ptr() };
            let flags = ev.modifierFlags() & MODIFIERS;
            if !flags.contains(NSEventModifierFlags::Command) {
                return e.as_ptr();
            }
            let Some(window) = ev.window(mtm) else { return e.as_ptr() };
            let Some(web) = web_in(&window) else { return e.as_ptr() };
            let key = ev.charactersIgnoringModifiers().map(|s| s.to_string().to_lowercase()).unwrap_or_default();
            match popup_key(&key, flags == NSEventModifierFlags::Command, flags == NSEventModifierFlags::Command | NSEventModifierFlags::Shift) {
                PopupKey::App => return e.as_ptr(),
                PopupKey::Close => window.performClose(None),
                PopupKey::Action(action) => {
                    window.makeFirstResponder(Some(&web));
                    // SAFETY: a standard action sent down the pop-up's responder chain (its page
                    // first), on the main thread.
                    unsafe { NSApplication::sharedApplication(mtm).sendAction_to_from(action, None, None) };
                }
                // The page hands a key it does not use on to the app's menu: one the menu has
                // would act behind the pop-up.
                PopupKey::Page if menu_has(mtm, &key, flags) => {}
                PopupKey::Page => return e.as_ptr(),
            }
            std::ptr::null_mut()
        });
        // SAFETY: the block returns the event it was given, or nothing for a key it used. The
        // monitor stays for the app's life.
        let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block) };
        std::mem::forget(monitor);
    });
}

/// Whether the app's menu has an item for `key` (as typed without modifiers, lowercase) with
/// these modifiers.
fn menu_has(mtm: MainThreadMarker, key: &str, flags: NSEventModifierFlags) -> bool {
    fn has(menu: &NSMenu, key: &str, flags: NSEventModifierFlags) -> bool {
        menu.itemArray().iter().any(|item| {
            if item.submenu().is_some_and(|sub| has(&sub, key, flags)) {
                return true;
            }
            let eq = item.keyEquivalent().to_string();
            if eq.is_empty() {
                return false;
            }
            let mut mask = item.keyEquivalentModifierMask() & MODIFIERS;
            // An upper-case key equivalent means ⇧.
            if eq.chars().any(char::is_uppercase) {
                mask |= NSEventModifierFlags::Shift;
            }
            eq.to_lowercase() == key && mask == flags
        })
    }
    NSApplication::sharedApplication(mtm).mainMenu().is_some_and(|m| has(&m, key, flags))
}

/// The modifier keys a shortcut is told by.
const MODIFIERS: NSEventModifierFlags = NSEventModifierFlags::Command.union(NSEventModifierFlags::Shift).union(NSEventModifierFlags::Option).union(NSEventModifierFlags::Control);

/// What a key with ⌘ does in a pop-up.
#[derive(Debug, PartialEq, Eq)]
enum PopupKey {
    /// The app's: Quit, Hide.
    App,
    /// ⌘W.
    Close,
    /// An editing action (or Reload) for the page.
    Action(objc2::runtime::Sel),
    /// To the page as typed (unless the app's menu has it).
    Page,
}

/// `key` (as typed without modifiers) with ⌘ alone (`plain`) or ⇧⌘ (`shift`).
fn popup_key(key: &str, plain: bool, shift: bool) -> PopupKey {
    match key {
        "q" | "h" if plain => PopupKey::App,
        "w" if plain => PopupKey::Close,
        "c" if plain => PopupKey::Action(sel!(copy:)),
        "v" if plain => PopupKey::Action(sel!(paste:)),
        "x" if plain => PopupKey::Action(sel!(cut:)),
        "a" if plain => PopupKey::Action(sel!(selectAll:)),
        "z" if plain => PopupKey::Action(sel!(undo:)),
        "z" if shift => PopupKey::Action(sel!(redo:)),
        "r" if plain => PopupKey::Action(sel!(reload:)),
        _ => PopupKey::Page,
    }
}

/// A pop-up's own new window: another pop-up when it is one (and the user's), else a page in the
/// panel of the page that opened the first.
fn new_window_of_popup(web: &WKWebView, config: &WKWebViewConfiguration, action: &WKNavigationAction, features: &WKWindowFeatures) -> Option<Retained<WKWebView>> {
    let (_, sink) = find(web)?;
    // SAFETY: plain getters of the objects WebKit hands, on the main thread.
    let (size, sized, url) = unsafe {
        let size = features.width().zip(features.height()).map(|(w, h)| (w.doubleValue(), h.doubleValue()));
        let sized = size.is_some() || features.x().is_some() || features.y().is_some();
        let url = action.request().URL().and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default();
        (size, sized, url)
    };
    if wanted(sized, user_acted_in(web)) {
        return open(web, &url, config, size, sink);
    }
    if !url.is_empty() {
        sink.push(Event::OpenHere(url));
    }
    None
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TernaryCodePopupDelegate"]
    struct PopupDelegate;

    unsafe impl NSObjectProtocol for PopupDelegate {}

    unsafe impl WKUIDelegate for PopupDelegate {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(&self, web: &WKWebView, config: &WKWebViewConfiguration, action: &WKNavigationAction, features: &WKWindowFeatures) -> Option<Retained<WKWebView>> {
            new_window_of_popup(web, config, action, features)
        }

        // The page closed itself (`window.close()`, a sign-in done).
        #[unsafe(method(webViewDidClose:))]
        fn web_view_did_close(&self, web: &WKWebView) {
            if let Some((window, _)) = find(web) {
                window.close();
            }
        }
    }

    unsafe impl WKNavigationDelegate for PopupDelegate {
        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, web: &WKWebView, _navigation: Option<&WKNavigation>) {
            retitle(web);
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, web: &WKWebView, _navigation: Option<&WKNavigation>) {
            retitle(web);
        }
    }

    unsafe impl NSWindowDelegate for PopupDelegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, note: &NSNotification) {
            if let Some(window) = note.object().and_then(|o| o.downcast::<NSWindow>().ok()) {
                forget(&window);
            }
        }
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popups_only_for_sized_windows_the_user_opened() {
        assert!(wanted(true, true));
        // A link to a new tab (no size), or a script opening windows on its own.
        assert!(!wanted(false, true));
        assert!(!wanted(true, false));
    }

    #[test]
    fn keys_in_a_popup_stay_in_it() {
        assert_eq!(popup_key("v", true, false), PopupKey::Action(sel!(paste:)));
        assert_eq!(popup_key("z", false, true), PopupKey::Action(sel!(redo:)));
        assert_eq!(popup_key("w", true, false), PopupKey::Close);
        assert_eq!(popup_key("q", true, false), PopupKey::App);
        // ⌘K, ⌘T, ⌘← (the app's palette, a new tab, a line's start): the page's, as typed.
        assert_eq!(popup_key("k", true, false), PopupKey::Page);
        assert_eq!(popup_key("t", true, false), PopupKey::Page);
        assert_eq!(popup_key("\u{f702}", true, false), PopupKey::Page);
        // ⌥⌘V is not Paste.
        assert_eq!(popup_key("v", false, false), PopupKey::Page);
    }

    #[test]
    fn popups_fit_the_screen() {
        assert_eq!(fit(Some((500.0, 600.0)), (1440.0, 875.0)), (500.0, 600.0));
        assert_eq!(fit(Some((100.0, 50.0)), (1440.0, 875.0)), (320.0, 240.0));
        assert_eq!(fit(Some((3000.0, 3000.0)), (1440.0, 875.0)), (1400.0, 835.0));
        assert_eq!(fit(None, (1440.0, 875.0)), (500.0, 640.0));
        // A screen smaller than the smallest pop-up.
        assert_eq!(fit(Some((500.0, 600.0)), (300.0, 200.0)), (320.0, 240.0));
    }

    #[test]
    fn titles_name_the_site() {
        assert_eq!(title_for("https://accounts.google.com/o/oauth2/v2/auth?client_id=x"), "accounts.google.com");
        assert_eq!(title_for("http://evil.example/login"), "Not Secure — evil.example");
        assert_eq!(title_for("http://localhost:5173/auth"), "localhost");
        assert_eq!(title_for("about:blank"), "about:blank");
        assert_eq!(title_for(""), "");
    }
}
