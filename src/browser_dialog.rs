//! A page's JavaScript dialogs: `alert()`, `confirm()` and `prompt()` (and "Leave site?"). The
//! system's web view shows none by itself (confirm() answered "Cancel" at once, alert() nothing),
//! so the app's pages are given WebKit's dialog methods here: a dialog becomes a bar above the
//! page (`Event::Dialog`), answered by the user or by the agent using the page, and the page
//! goes on with the answer. Only that page waits; the app does not.
//!
//! wry's own delegate class (`WryWebViewUIDelegate`) answers the rest of WebKit's questions; the
//! three dialog methods, which it lacks, are added to it once, before the first page is made.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use block2::{Block, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::sel;
use objc2_foundation::NSString;
use objc2_web_kit::{WKFrameInfo, WKWebView};

use crate::browser::{Dialog, DialogKind, Event, Sink};

/// How a waiting dialog is answered (WebKit's completion handler for it).
enum Reply {
    Alert(RcBlock<dyn Fn()>),
    Confirm(RcBlock<dyn Fn(Bool)>),
    Prompt(RcBlock<dyn Fn(*mut NSString)>),
}

struct Waiting {
    /// The page's web view (its address, as a number).
    web: usize,
    reply: Reply,
}

thread_local! {
    /// The pages' web views and where each reports (main thread only).
    static PAGES: RefCell<HashMap<usize, Sink>> = RefCell::new(HashMap::new());
    static WAITING: RefCell<HashMap<u64, Waiting>> = RefCell::new(HashMap::new());
}

static NEXT: AtomicU64 = AtomicU64::new(1);

fn key(web: &WKWebView) -> usize {
    std::ptr::from_ref(web) as usize
}

/// Gives a page just made the dialog methods: added to its delegate's class (wry's) once, then
/// the delegate is set again, for WebKit reads which methods a delegate has when it is set.
pub fn install(web: &WKWebView) {
    // SAFETY: the page's own delegate, on the main thread.
    let Some(delegate) = (unsafe { web.UIDelegate() }) else { return };
    let class: *const AnyClass = unsafe { objc2::msg_send![&*delegate, class] };
    static DONE: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
    let mut done = DONE.lock().unwrap_or_else(|e| e.into_inner());
    if !done.contains(&(class as usize)) {
        done.push(class as usize);
        // SAFETY: each function has the signature its selector and type encoding say (void
        // return; self, _cmd, then objects, the last a block); wry's class lacks these methods
        // (class_addMethod leaves a method a class has).
        unsafe {
            let add = |sel: Sel, imp: Imp, types: &std::ffi::CStr| {
                objc2::ffi::class_addMethod(class.cast_mut(), sel, imp, types.as_ptr());
            };
            add(
                sel!(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:),
                std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut WKWebView, *mut NSString, *mut WKFrameInfo, *mut Block<dyn Fn()>), Imp>(alert),
                c"v@:@@@@?",
            );
            add(
                sel!(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:),
                std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut WKWebView, *mut NSString, *mut WKFrameInfo, *mut Block<dyn Fn(Bool)>), Imp>(confirm),
                c"v@:@@@@?",
            );
            add(
                sel!(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:),
                std::mem::transmute::<unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut WKWebView, *mut NSString, *mut NSString, *mut WKFrameInfo, *mut Block<dyn Fn(*mut NSString)>), Imp>(prompt),
                c"v@:@@@@@?",
            );
        }
    }
    drop(done);
    // SAFETY: the same delegate, set again on the main thread.
    unsafe { web.setUIDelegate(Some(&delegate)) };
}

/// A page's web view, made: its dialogs are reported to `sink`.
pub fn register(web: &WKWebView, sink: Sink) {
    PAGES.with_borrow_mut(|p| p.insert(key(web), sink));
}

/// Answers a waiting dialog: OK (`accept`, with `text` for a prompt) or Cancel. Returns whether
/// it was waiting.
pub fn answer(id: u64, accept: bool, text: &str) -> bool {
    let Some(w) = WAITING.with_borrow_mut(|d| d.remove(&id)) else { return false };
    match w.reply {
        Reply::Alert(r) => r.call(()),
        Reply::Confirm(r) => r.call((Bool::new(accept),)),
        Reply::Prompt(r) => {
            let s = accept.then(|| NSString::from_str(text));
            r.call((s.as_ref().map_or(std::ptr::null_mut(), |s| Retained::as_ptr(s).cast_mut()),));
        }
    }
    true
}

/// Forgets the pages whose web views are gone (`live`: those up now); their waiting dialogs are
/// answered Cancel (WebKit wants every one answered).
pub fn tidy(live: &[&WKWebView]) {
    let live: Vec<usize> = live.iter().map(|w| key(w)).collect();
    PAGES.with_borrow_mut(|p| p.retain(|k, _| live.contains(k)));
    let gone: Vec<u64> = WAITING.with_borrow(|d| d.iter().filter(|(_, w)| !live.contains(&w.web)).map(|(id, _)| *id).collect());
    for id in gone {
        answer(id, false, "");
    }
}

/// Reports a dialog of `web` to its page (one not the app's is answered at once, as WebKit does
/// without a delegate).
fn open(web: *mut WKWebView, kind: DialogKind, message: *mut NSString, default: *mut NSString, reply: Reply) {
    // SAFETY: WebKit hands live objects (or null) for the length of the call.
    let (web, message, default) = unsafe { (web.as_ref(), message.as_ref(), default.as_ref()) };
    let sink = web.and_then(|w| PAGES.with_borrow(|p| p.get(&key(w)).cloned()));
    let (Some(web), Some(sink)) = (web, sink) else {
        // Not a page of the app's: as WebKit does without a delegate.
        match reply {
            Reply::Alert(r) => r.call(()),
            Reply::Confirm(r) => r.call((Bool::NO,)),
            Reply::Prompt(r) => r.call((std::ptr::null_mut(),)),
        }
        return;
    };
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    WAITING.with_borrow_mut(|d| d.insert(id, Waiting { web: key(web), reply }));
    let host = crate::browser::host_of(&crate::browser::url_in(web));
    let message = message.map(|m| m.to_string()).unwrap_or_default();
    let text = default.map(|m| m.to_string()).unwrap_or_default();
    sink.push(Event::Dialog(Dialog { id, kind, host, message, text }));
}

unsafe extern "C-unwind" fn alert(_this: *mut AnyObject, _cmd: Sel, web: *mut WKWebView, message: *mut NSString, _frame: *mut WKFrameInfo, done: *mut Block<dyn Fn()>) {
    // SAFETY: WebKit hands a live block; it is copied to be kept until answered.
    let Some(done) = (unsafe { done.as_ref() }) else { return };
    open(web, DialogKind::Alert, message, std::ptr::null_mut(), Reply::Alert(done.copy()));
}

unsafe extern "C-unwind" fn confirm(_this: *mut AnyObject, _cmd: Sel, web: *mut WKWebView, message: *mut NSString, _frame: *mut WKFrameInfo, done: *mut Block<dyn Fn(Bool)>) {
    // SAFETY: as in `alert`.
    let Some(done) = (unsafe { done.as_ref() }) else { return };
    open(web, DialogKind::Confirm, message, std::ptr::null_mut(), Reply::Confirm(done.copy()));
}

unsafe extern "C-unwind" fn prompt(_this: *mut AnyObject, _cmd: Sel, web: *mut WKWebView, message: *mut NSString, default: *mut NSString, _frame: *mut WKFrameInfo, done: *mut Block<dyn Fn(*mut NSString)>) {
    // SAFETY: as in `alert`.
    let Some(done) = (unsafe { done.as_ref() }) else { return };
    open(web, DialogKind::Prompt, message, default, Reply::Prompt(done.copy()));
}
