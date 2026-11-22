//! macOS: each page's web view lives in a window of its own, a borderless child window laid
//! exactly over its pane in the app's window (it moves, hides and goes full screen with it), so
//! the page looks as if it were in the app's window.
//!
//! Why: a window has one keyboard (its first responder), and WebKit takes it for its web view at
//! every click in the page, an agent's included (its `mouseDown` makes the view first responder).
//! With the page in the app's window, an agent's click took the keyboard from the terminal the
//! user typed in. In a window of its own the page takes only its own window's keyboard, and that
//! window becomes the key window (gets the user's keys) only when the user clicks in the page or
//! the app gives it to the page; an agent's clicks and keys go to the web view itself and leave
//! the key window as it is. The page also keeps its focus and caret while an agent types in it:
//! its web view stays first responder of its own window, whichever window the user types in.
//!
//! To WebKit the page's window is the key window while the app's is (`isKeyWindow`), as when the
//! page was in the app's window: WebKit takes the pointer (hover, the cursor over a link) and an
//! agent's events only in a key window. AppKit's own idea of the key window, where the user's keys
//! go, stays the real one (`is_key`). The real mouse's moves come to the page through a tracking
//! area of its own, active whenever the app is in front.
//!
//! `View` wraps wry's web view with its window: `set_bounds`, `bounds`, `set_visible`, `focus` and
//! `focus_parent` are the window's here (wry's would act on the web view in the app's window);
//! the rest is wry's (`Deref`).

use std::cell::Cell;
use std::ops::Deref;

use objc2::rc::{Allocated, Retained, Weak};
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSAutoresizingMaskOptions, NSBackingStoreType, NSEvent, NSResponder, NSView, NSWindow, NSWindowOrderingMode, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use objc2_web_kit::WKWebView;

/// A page placed farther left than this is out of the window (an agent works in it unseen).
const PARKED_X: f64 = -10000.0;

pub struct PageWindowIvars {
    /// The window may become the key window: the user clicked in the page, or the app gave it the
    /// keyboard (`View::focus`). Cleared when it stops being key, so AppKit never picks it as the
    /// key window on its own (when another window closes).
    may_key: Cell<bool>,
    /// The page it holds (`browser::page_key`).
    page: Cell<usize>,
    /// `isKeyWindow` says the truth for now (while AppKit makes it the key window).
    honest: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TernaryCodePageWindow"]
    #[ivars = PageWindowIvars]
    pub struct PageWindow;

    impl PageWindow {
        /// To WebKit (and the rest), key while the app's window is: see the module's comment.
        #[unsafe(method(isKeyWindow))]
        fn says_key(&self) -> bool {
            is_key(self) || (!self.ivars().honest.get() && self.parentWindow().is_some_and(|p| is_key(&p)))
        }

        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            self.ivars().may_key.get()
        }

        /// The app's window stays the main window (its title bar as it was).
        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            false
        }

        #[unsafe(method(resignKeyWindow))]
        fn resign_key(&self) {
            // SAFETY: NSWindow's own method, on the main thread.
            unsafe {
                let _: () = msg_send![super(self), resignKeyWindow];
            }
            self.ivars().may_key.set(false);
        }

        // A key the user pressed in the page that the page did not take: on to the app's window,
        // as when the page was in it (its responder chain went on to the app's view): egui hands
        // it to the app (a shortcut of its own). An agent's never gets here
        // (`browser_input::agents_key`).
        #[unsafe(method(keyDown:))]
        fn key_down(&self, ev: &NSEvent) {
            if let Some(v) = self.app_view() {
                v.keyDown(ev);
            }
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, ev: &NSEvent) {
            if let Some(v) = self.app_view() {
                v.keyUp(ev);
            }
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, ev: &NSEvent) {
            if let Some(v) = self.app_view() {
                v.flagsChanged(ev);
            }
        }
    }
);

impl PageWindow {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(PageWindowIvars { may_key: Cell::new(false), page: Cell::new(0), honest: Cell::new(false) });
        // SAFETY: NSWindow's initializer, on the main thread.
        unsafe { msg_send![super(this), initWithContentRect: frame, styleMask: NSWindowStyleMask::Borderless, backing: NSBackingStoreType::Buffered, defer: false] }
    }

    /// The view of the app's window (winit's, where egui gets keys).
    fn app_view(&self) -> Option<Retained<NSView>> {
        self.parentWindow()?.contentView()
    }

    /// The page it holds (`browser::page_key`).
    pub fn page(&self) -> usize {
        self.ivars().page.get()
    }

    /// The user clicked in the page (seen before AppKit hands the click on): its window becomes
    /// the key window first, so the click is no click that only brings a window forward, and
    /// the user's keys go to the page.
    pub fn take_user_click(&self) {
        if !is_key(self) {
            self.become_key();
        }
    }

    /// Becomes AppKit's key window (it would not, saying it is one already).
    fn become_key(&self) {
        self.ivars().may_key.set(true);
        self.ivars().honest.set(true);
        self.makeKeyWindow();
        self.ivars().honest.set(false);
    }
}

define_class!(
    /// Holds the page window's keyboard while the page has not got it: AppKit would hand a
    /// window's keyboard to its first view that takes one (the web view) when it comes on screen
    /// again, and WebKit, given it that way, puts the focus on the page's first field with the
    /// caret at its start.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TernaryCodePageKeyHolder"]
    struct KeyHolder;

    impl KeyHolder {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }
    }
);

pub struct PageMouseIvars {
    web: Weak<WKWebView>,
}

define_class!(
    /// The real mouse in a page's window, handed to its web view (WebKit's own tracking of it is
    /// only while its window is AppKit's key window).
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TernaryCodePageMouse"]
    #[ivars = PageMouseIvars]
    struct PageMouse;

    impl PageMouse {
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, ev: &NSEvent) {
            self.hand_on(ev);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, ev: &NSEvent) {
            self.hand_on(ev);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, ev: &NSEvent) {
            self.hand_on(ev);
        }
    }
);

impl PageMouse {
    /// The real mouse's move in the page's window, to its web view, unless WebKit tracks it
    /// itself (its window is AppKit's key window).
    fn hand_on(&self, ev: &NSEvent) {
        let Some(web) = self.ivars().web.load() else { return };
        if web.window().is_some_and(|w| is_key(&w)) {
            return;
        }
        crate::browser_input::simulate_pointer(&web, ev);
    }

    /// Watches the mouse over `content` (the page's window) for `web`, while the app is in front.
    fn watch(mtm: MainThreadMarker, content: &NSView, web: &WKWebView) -> (Retained<Self>, Retained<NSObject>) {
        let this = mtm.alloc::<Self>().set_ivars(PageMouseIvars { web: Weak::from(web) });
        // SAFETY: NSObject's init, on the main thread.
        let mouse: Retained<Self> = unsafe { msg_send![super(this), init] };
        // Entered and exited, moved, while the app is active, over the whole view.
        let options: usize = 0x01 | 0x02 | 0x40 | 0x200;
        // SAFETY: NSTrackingArea's initializer (its owner is not retained: `View` keeps it) and
        // NSView's addTrackingArea:, on the main thread.
        let area: Retained<NSObject> = unsafe {
            let area: Allocated<NSObject> = msg_send![objc2::class!(NSTrackingArea), alloc];
            let area: Retained<NSObject> = msg_send![area, initWithRect: NSRect::ZERO, options: options, owner: &*mouse, userInfo: std::ptr::null::<AnyObject>()];
            let _: () = msg_send![content, addTrackingArea: &*area];
            area
        };
        (mouse, area)
    }
}

/// A page's web view (wry's) in its window over the app's.
pub struct View {
    web: wry::WebView,
    wk: Retained<WKWebView>,
    /// `None` only while it is let go (`Drop`).
    host: Option<Retained<PageWindow>>,
    /// The app's window it sits over.
    app: Retained<NSWindow>,
    /// The real mouse over it (kept: its tracking area does not keep it; autoreleased on drop).
    mouse: Option<Retained<PageMouse>>,
    /// The tracking area attached to `host`'s content view (removed on drop).
    area: Option<Retained<NSObject>>,
    /// Has the page window's keyboard while the page has not (`KeyHolder`).
    holder: Retained<NSView>,
    /// It is over the app's window (a child of it, on screen or out of the window).
    shown: Cell<bool>,
    /// It is out of the window, where an agent works in it unseen.
    parked: Cell<bool>,
}

impl Deref for View {
    type Target = wry::WebView;
    fn deref(&self) -> &wry::WebView {
        &self.web
    }
}

impl View {
    /// Moves wry's web view (just built in the app's window, at its place) into a window of its
    /// own at the same place.
    pub fn new(web: wry::WebView) -> Option<Self> {
        use wry::WebViewExtMacOS;
        let mtm = MainThreadMarker::new()?;
        let wk: Retained<WKWebView> = Retained::into_super(web.webview());
        let app = wk.window()?;
        let frame = app.convertRectToScreen(wk.convertRect_toView(wk.bounds(), None));
        let host = PageWindow::new(mtm, frame);
        host.ivars().page.set(crate::browser::page_key(&wk));
        // SAFETY: plain NSWindow setters on the main thread. It is not released on close: `View`
        // owns it. No shadow, no background (the page draws its own), no animation in or out, no
        // window tabs, not in the Window menu or ⌘`.
        unsafe {
            host.setReleasedWhenClosed(false);
            host.setOpaque(false);
            host.setHasShadow(false);
            let clear: *mut AnyObject = msg_send![objc2::class!(NSColor), clearColor];
            let _: () = msg_send![&*host, setBackgroundColor: clear];
            let _: () = msg_send![&*host, setAnimationBehavior: 2isize]; // NSWindowAnimationBehaviorNone
            let _: () = msg_send![&*host, setTabbingMode: 2isize]; // NSWindowTabbingModeDisallowed
            let _: () = msg_send![&*host, setCollectionBehavior: 1usize << 6]; // IgnoresCycle
        }
        host.setExcludedFromWindowsMenu(true);
        host.setAcceptsMouseMovedEvents(true);
        let content = NSView::initWithFrame(mtm.alloc(), NSRect::new(NSPoint::new(0.0, 0.0), frame.size));
        host.setContentView(Some(&content));
        wk.removeFromSuperview();
        wk.setFrame(content.bounds());
        wk.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        let holder: Retained<NSView> = {
            let this = mtm.alloc::<KeyHolder>();
            // SAFETY: NSView's initializer, on the main thread.
            let h: Retained<KeyHolder> = unsafe { msg_send![this, initWithFrame: NSRect::ZERO] };
            Retained::into_super(h)
        };
        content.addSubview(&holder);
        content.addSubview(&wk);
        host.makeFirstResponder(Some(&holder));
        let (mouse, area) = PageMouse::watch(mtm, &content, &wk);
        // The app's view keeps the app window's keyboard (the web view may have had it there).
        let app_view = app.contentView();
        let first_is_app = app.firstResponder().and_then(|f| f.downcast::<NSView>().ok()).zip(app_view.as_ref()).is_some_and(|(f, v)| f.isDescendantOf(v));
        if !first_is_app && let Some(v) = &app_view {
            app.makeFirstResponder(Some(v));
        }
        // SAFETY: both windows are alive, on the main thread.
        unsafe { app.addChildWindow_ordered(&host, NSWindowOrderingMode::Above) };
        let view = Self { web, wk, host: Some(host), app, mouse: Some(mouse), area: Some(area), holder, shown: Cell::new(true), parked: Cell::new(false) };
        view.note_parked(frame.origin.x - view.app.frame().origin.x < PARKED_X);
        tell(view.host());
        crate::browser_popup::raise();
        Some(view)
    }

    fn host(&self) -> &PageWindow {
        self.host.as_deref().expect("a page's window lives as long as its view")
    }

    /// The page's web view.
    pub fn page(&self) -> &WKWebView {
        &self.wk
    }

    /// Lays the page at `bounds` (logical points from the top left of the app window's
    /// content), out of the window when far to the left.
    pub fn set_bounds(&self, bounds: wry::Rect) -> wry::Result<()> {
        let scale = self.app.backingScaleFactor();
        let p = bounds.position.to_logical::<f64>(scale);
        let s = bounds.size.to_logical::<f64>(scale);
        let Some(content) = self.app.contentView() else { return Ok(()) };
        let height = content.frame().size.height;
        let y = if content.isFlipped() { p.y } else { height - p.y - s.height };
        let local = NSRect::new(NSPoint::new(p.x, y), NSSize::new(s.width, s.height));
        let frame = self.app.convertRectToScreen(content.convertRect_toView(local, None));
        self.host().setFrame_display(frame, true);
        self.note_parked(p.x < PARKED_X);
        Ok(())
    }

    /// Where the page is (logical points from the top left of the app window's content): its
    /// window's place over the app's.
    pub fn bounds(&self) -> wry::Result<wry::Rect> {
        let frame = self.app.convertRectFromScreen(self.host().frame());
        let (local, height, flipped) = match self.app.contentView() {
            Some(content) => (content.convertRect_fromView(frame, None), content.frame().size.height, content.isFlipped()),
            None => (frame, self.app.frame().size.height, false),
        };
        let y = if flipped { local.origin.y } else { height - local.origin.y - local.size.height };
        Ok(wry::Rect {
            position: wry::dpi::LogicalPosition::new(local.origin.x, y).into(),
            size: wry::dpi::LogicalSize::new(local.size.width, local.size.height).into(),
        })
    }

    /// Out of the window, the page goes on as if seen (WebKit would slow a page whose window is
    /// off screen down, its timers and drawing): an agent works in it.
    fn note_parked(&self, parked: bool) {
        if self.parked.replace(parked) == parked {
            return;
        }
        let sel = sel!(_setWindowOcclusionDetectionEnabled:);
        // SAFETY: WKWebView's own setter (macOS 10.13+, not public, so asked for first), taking a BOOL.
        unsafe {
            let responds: bool = msg_send![&*self.wk, respondsToSelector: sel];
            if responds {
                let _: () = msg_send![&*self.wk, _setWindowOcclusionDetectionEnabled: !parked];
            }
        }
    }

    /// Shows the page over the app's window, or takes it away.
    pub fn set_visible(&self, visible: bool) -> wry::Result<()> {
        if self.shown.replace(visible) == visible {
            return Ok(());
        }
        let host = self.host();
        if visible {
            // SAFETY: both windows are alive, on the main thread.
            unsafe { self.app.addChildWindow_ordered(host, NSWindowOrderingMode::Above) };
            tell(host);
            crate::browser_popup::raise();
        } else {
            if is_key(host) {
                self.app.makeKeyWindow();
            }
            self.app.removeChildWindow(host);
            host.orderOut(None);
        }
        Ok(())
    }

    /// The page takes the user's keyboard (its window becomes the key window).
    pub fn focus(&self) -> wry::Result<()> {
        let host = self.host();
        host.makeFirstResponder(Some(&self.wk));
        if self.shown.get() && !self.parked.get() && !is_key(host) {
            host.become_key();
        }
        Ok(())
    }

    /// The page lets go of the keyboard, as when it was in the app's window and the app took it:
    /// the user's goes back to the app's window if the page had it, and the page's own window's
    /// keyboard goes too (the page is no longer focused: its caret and focus ring go, as in a
    /// browser window in the back).
    pub fn focus_parent(&self) -> wry::Result<()> {
        let host = self.host();
        if is_key(host) {
            self.app.makeKeyWindow();
        }
        host.makeFirstResponder(Some(&self.holder));
        Ok(())
    }

    /// The page has the keyboard: of its own window (an agent typing in it), or the user's (its
    /// window is then the key window too).
    pub fn has_keyboard(&self) -> bool {
        crate::browser::holds_window_keys(&self.wk)
    }

    /// Rounds the page's corners (`radius` points): its window's content clips it, WebKit's
    /// drawing included.
    pub fn set_corner_radius(&self, radius: f64) {
        let Some(content) = self.host().contentView() else { return };
        content.setWantsLayer(true);
        // SAFETY: the layer of a layer-backed view, and CALayer's own setters, on the main thread.
        unsafe {
            let layer: *mut AnyObject = msg_send![&*content, layer];
            if !layer.is_null() {
                let _: () = msg_send![layer, setCornerRadius: radius];
                let _: () = msg_send![layer, setMasksToBounds: radius > 0.0];
            }
        }
    }

    /// The page's window shows where the app's does (on screen, in its Space: full screen too).
    pub fn on_screen(&self) -> bool {
        let host = self.host();
        host.isVisible() && host.isOnActiveSpace() && self.app.isOnActiveSpace()
    }

    /// The page has the user's keyboard: its window is AppKit's key window.
    pub fn has_user_keyboard(&self) -> bool {
        is_key(self.host())
    }
}

impl Drop for View {
    fn drop(&mut self) {
        let Some(host) = self.host.take() else { return };
        if let (Some(content), Some(area)) = (host.contentView(), self.area.take()) {
            // SAFETY: removing the tracking area we added to `content`, on the main thread.
            unsafe {
                let _: () = msg_send![&*content, removeTrackingArea: &*area];
            }
        }
        if is_key(&host) {
            self.app.makeKeyWindow();
        }
        self.app.removeChildWindow(&host);
        host.orderOut(None);
        // Let go once AppKit is done with this turn of the event loop (wry takes the web view
        // out of the window right after this; any queued tracking events still reference `mouse`).
        if let Some(mouse) = self.mouse.take() {
            let _ = Retained::autorelease_ptr(mouse);
        }
        let _ = Retained::autorelease_ptr(host);
    }
}

/// Whether `window` is AppKit's key window, the one the user's keys go to (a page's window says
/// it is key to WebKit while the app's window is).
pub fn is_key(window: &NSWindow) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    NSApplication::sharedApplication(mtm).keyWindow().is_some_and(|k| std::ptr::eq(&*k, window))
}

thread_local! {
    /// AppKit's key window when WebKit was last told of it (`tell_webkit`).
    static TOLD: Cell<usize> = const { Cell::new(0) };
}

/// AppKit's key window changed (the user went from the app's window to a page's, from the app to
/// another, or back): each page's WebKit hears again whether its window is key (it hears only of
/// its own window, and a page's says it is key while the app's is). Called each frame.
pub fn tell_webkit() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let key = app.keyWindow().map_or(0, |k| Retained::as_ptr(&k) as usize);
    if TOLD.with(|t| t.replace(key)) == key {
        return;
    }
    let windows = app.windows();
    for w in (0..windows.count()).map(|i| windows.objectAtIndex(i)) {
        let Some(page) = w.downcast_ref::<PageWindow>() else { continue };
        tell(page);
    }
}

/// WebKit hears whether the page's window is key (as it says): it asks only when told so of its
/// window (and when the web view comes into it, before the window is over the app's).
fn tell(page: &PageWindow) {
    let name = NSString::from_str(if page.isKeyWindow() { "NSWindowDidBecomeKeyNotification" } else { "NSWindowDidResignKeyNotification" });
    // SAFETY: posting a notification on the default center, on the main thread.
    unsafe {
        let center: *mut AnyObject = msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
        let _: () = msg_send![center, postNotificationName: &*name, object: page];
    }
}

/// The window of the app a web view's page sits over (a page's own window is a child of it).
pub fn app_window_of(window: &NSWindow) -> Retained<NSWindow> {
    match window.downcast_ref::<PageWindow>().and_then(|w| w.parentWindow()) {
        Some(app) => app,
        None => objc2::Message::retain(window),
    }
}

/// The page's window, when `window` is one.
pub fn page_window(window: &NSWindow) -> Option<&PageWindow> {
    window.downcast_ref::<PageWindow>()
}

/// The child windows of `window`, bottom to top.
fn children(window: &NSWindow) -> Vec<Retained<NSWindow>> {
    window.childWindows().map(|c| (0..c.count()).map(|i| c.objectAtIndex(i)).collect()).unwrap_or_default()
}

/// Whether `window` holds pages over it (it is the app's window).
pub fn holds_pages(window: &NSWindow) -> bool {
    children(window).iter().any(|w| w.downcast_ref::<PageWindow>().is_some())
}

/// Whether the user types in a page now: the app is in front and a page's window is the key
/// window (the app's window is then not, though it is in use: its title bar and panes look as
/// they do while it has the keyboard).
pub fn typing_in_page() -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    let app = NSApplication::sharedApplication(mtm);
    app.isActive() && app.keyWindow().is_some_and(|w| w.downcast_ref::<PageWindow>().is_some())
}

/// A picture (PNG) of the app's window as it shows, the windows over it (its pages') included:
/// a test's look at the pages laid over their panes. Only this app's windows are drawn in it.
#[cfg(debug_assertions)]
pub fn window_picture() -> Option<Vec<u8>> {
    use objc2_app_kit::NSBitmapImageFileType;
    /// CoreGraphics' image (a CGImageRef points at one).
    #[repr(C)]
    struct CGImage {
        _private: [u8; 0],
    }
    // SAFETY: CGImageRef's encoding.
    unsafe impl objc2::encode::RefEncode for CGImage {
        const ENCODING_REF: objc2::encode::Encoding = objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Struct("CGImage", &[]));
    }
    unsafe extern "C" {
        // CoreGraphics; the image is the caller's to release.
        fn CGWindowListCreateImage(rect: NSRect, options: u32, window: u32, image: u32) -> *mut CGImage;
        fn CGImageRelease(image: *mut CGImage);
    }
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let windows = app.windows();
    let main = (0..windows.count())
        .map(|i| windows.objectAtIndex(i))
        .filter(|w| w.downcast_ref::<PageWindow>().is_none() && w.isVisible())
        .max_by(|a, b| (a.frame().size.width * a.frame().size.height).total_cmp(&(b.frame().size.width * b.frame().size.height)))?;
    let f = main.frame();
    let screen_h = main.screen()?.frame().size.height.max(NSApplication::sharedApplication(mtm).mainWindow()?.screen()?.frame().size.height);
    // CoreGraphics counts y from the top of the main screen.
    let rect = NSRect::new(NSPoint::new(f.origin.x, screen_h - f.origin.y - f.size.height), f.size);
    // On screen, above this window, and this window itself.
    let options = (1 << 1) | (1 << 3);
    // SAFETY: CoreGraphics' capture of this app's windows; the image is released once copied.
    unsafe {
        let image = CGWindowListCreateImage(rect, options, main.windowNumber() as u32, 0);
        if image.is_null() {
            return None;
        }
        let rep = mtm.alloc::<objc2_app_kit::NSBitmapImageRep>();
        let rep: Option<Retained<objc2_app_kit::NSBitmapImageRep>> = msg_send![rep, initWithCGImage: image];
        CGImageRelease(image);
        let props = objc2_foundation::NSDictionary::new();
        let data = rep?.representationUsingType_properties(NSBitmapImageFileType::PNG, &props)?;
        Some(data.to_vec())
    }
}

/// The page window over the app's window at `at` (a point in the app's window), with the point
/// in it: where a click there goes (tests post clicks as the mouse would land them).
pub fn page_window_at(app: &NSWindow, at: NSPoint) -> Option<(Retained<NSWindow>, NSPoint)> {
    let on_screen = app.convertPointToScreen(at);
    children(app).into_iter().rev().filter(|w| w.downcast_ref::<PageWindow>().is_some() && w.isVisible()).find_map(|w| {
        let f = w.frame();
        let inside = on_screen.x >= f.origin.x && on_screen.x < f.origin.x + f.size.width && on_screen.y >= f.origin.y && on_screen.y < f.origin.y + f.size.height;
        inside.then(|| {
            let local = w.convertPointFromScreen(on_screen);
            (w, local)
        })
    })
}
