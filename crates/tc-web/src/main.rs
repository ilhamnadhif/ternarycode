//! `ternarycode-web`: TernaryCode's pages on Linux. The app (which needs no GTK or WebKit, so
//! it starts on any distribution) runs this helper when a browser pane first shows a page. Each
//! page is a WebKitGTK web view laid in the app's own X11 window (a child window of it), placed,
//! shown and driven by the app's commands: one JSON object a line on stdin. What the pages do
//! comes back the same way on stdout. The helper ends with the app (its stdin closes).
//!
//! Commands: `new` (a page: id, parent window, address, bounds), `bounds`, `show`, `load`,
//! `reload`, `back`, `forward`, `stop_back`, `zoom`, `focus`, `blur`, `js` (a script; it posts
//! its answer itself), `shot` (a PNG of the page, into a file), `devtools`, `print`, `close`,
//! `clear`, `dialog` (an answer), `upload` (files for the next file chooser), `mouse`, `key`,
//! `text` (an agent's input, as real GDK events).
//!
//! Events: `url`, `title`, `loading`, `ipc` (the page's messages), `focus` (has the keyboard or
//! not), `clicked` (the user's click), `open_here` (a link to a new window), `download_started`,
//! `download_done`, `dialog`, `key` (a key of the app's), `shot`, `chooser`, `closed`.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("ternarycode-web runs on Linux only.");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
fn main() {
    linux::main();
}

#[cfg(target_os = "linux")]
mod linux {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::rc::Rc;

    use gtk::glib;
    use gtk::prelude::*;
    use serde_json::{Value, json};
    use webkit2gtk::{FileChooserRequestExt, ScriptDialog, ScriptDialogType, SnapshotOptions, SnapshotRegion, WebViewExt, WindowPropertiesExt};
    use wry::{WebViewBuilder, WebViewExtUnix};

    /// The app's window, as wry takes a parent.
    struct Parent(std::ffi::c_ulong);

    impl raw_window_handle::HasWindowHandle for Parent {
        fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
            let h = raw_window_handle::XlibWindowHandle::new(self.0);
            // SAFETY: the app's window lives as long as the app, and the helper ends with it.
            Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw_window_handle::RawWindowHandle::Xlib(h)) })
        }
    }

    struct Page {
        web: wry::WebView,
        /// The app's window (the keyboard goes back there).
        parent: std::ffi::c_ulong,
        /// Files an agent gave the next file chooser.
        upload: Option<Vec<String>>,
    }

    /// A sign-in pop-up a page opened: its web view, its window, the page.
    struct Popup {
        view: webkit2gtk::WebView,
        window: gtk::Window,
        opener: u64,
    }

    /// An agent acting in a page now: the sites it may open (a pop-up its step opens elsewhere is
    /// held back, and the app asks the user).
    struct Acting {
        page: u64,
        sites: Vec<String>,
        pass: Option<String>,
        at: std::time::Instant,
    }

    thread_local! {
        static PAGES: RefCell<HashMap<u64, Page>> = RefCell::new(HashMap::new());
        static POPUPS: RefCell<HashMap<u64, Popup>> = RefCell::new(HashMap::new());
        static ACTING: RefCell<Option<Acting>> = const { RefCell::new(None) };
        static DIALOGS: RefCell<HashMap<u64, ScriptDialog>> = RefCell::new(HashMap::new());
        static CONTEXT: RefCell<Option<wry::WebContext>> = const { RefCell::new(None) };
        static XLIB: Option<x11_dl::xlib::Xlib> = x11_dl::xlib::Xlib::open().ok();
    }

    /// Tells the app.
    fn send(v: Value) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{v}");
        let _ = out.flush();
    }

    fn ev(id: u64, ev: &str, rest: Value) {
        let mut v = json!({ "id": id, "ev": ev });
        if let (Some(o), Value::Object(r)) = (v.as_object_mut(), rest) {
            o.extend(r);
        }
        send(v);
    }

    pub fn main() {
        if std::env::args().nth(1).as_deref() == Some("--check") {
            // The libraries loaded (else the program would not have started): pages can show.
            println!("ok");
            return;
        }
        // GDK talks to the X server the app's window is on.
        if gtk::init().is_err() {
            eprintln!("ternarycode-web: no X11 display");
            std::process::exit(1);
        }
        let mut pending = Vec::new();
        let stdin = std::mem::ManuallyDrop::new(unsafe {
            use std::os::fd::FromRawFd;
            // SAFETY: fd 0 is the helper's stdin, kept open (ManuallyDrop) for its whole life.
            std::fs::File::from_raw_fd(0)
        });
        let stdin = RefCell::new(stdin);
        glib::unix_fd_add_local(0, glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR, move |_, _| {
            let mut buf = [0u8; 65536];
            let n = match stdin.borrow_mut().read(&mut buf) {
                Ok(n) => n,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock) => {
                    return glib::ControlFlow::Continue;
                }
                Err(_) => 0,
            };
            if n == 0 {
                // The app is gone.
                gtk::main_quit();
                return glib::ControlFlow::Break;
            }
            pending.extend_from_slice(&buf[..n]);
            while let Some(i) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=i).collect();
                if let Ok(cmd) = serde_json::from_slice::<Value>(&line) {
                    handle(&cmd);
                }
            }
            glib::ControlFlow::Continue
        });
        send(json!({ "ev": "ready" }));
        gtk::main();
    }

    /// The bounds the app gives (pixels of the screen), as wry takes them (GTK's units: pixels
    /// over GTK's whole-number scale).
    fn rect(c: &Value) -> wry::Rect {
        let f = |k: &str| c[k].as_f64().unwrap_or(0.0);
        // X11 places windows within ±32767 (an agent's page out of view sits far left).
        let x = f("x").max(-30000.0);
        let scale = gdk::Display::default().and_then(|d| d.monitor(0)).map_or(1, |m| m.scale_factor()).max(1);
        let s = f64::from(scale);
        wry::Rect { position: wry::dpi::LogicalPosition::new(x / s, f("y") / s).into(), size: wry::dpi::LogicalSize::new((f("w") / s).max(1.0), (f("h") / s).max(1.0)).into() }
    }

    fn with<R>(id: u64, f: impl FnOnce(&mut Page) -> R) -> Option<R> {
        PAGES.with_borrow_mut(|p| p.get_mut(&id).map(f))
    }

    /// The web view of a page, or of a pop-up (the agents' steps act in either).
    fn view_of(id: u64) -> Option<webkit2gtk::WebView> {
        PAGES.with_borrow(|p| p.get(&id).map(|p| p.web.webview())).or_else(|| POPUPS.with_borrow(|p| p.get(&id).map(|p| p.view.clone())))
    }

    fn handle(c: &Value) {
        let id = c["id"].as_u64().unwrap_or(0);
        let s = |k: &str| c[k].as_str().unwrap_or("").to_string();
        match c["op"].as_str().unwrap_or("") {
            "new" => new_page(id, c),
            "bounds" => {
                with(id, |p| p.web.set_bounds(rect(c)));
            }
            "show" => {
                with(id, |p| p.web.set_visible(c["v"].as_bool().unwrap_or(true)));
            }
            "load" => {
                with(id, |p| p.web.load_url(&s("url")));
            }
            "reload" => {
                with(id, |p| p.web.reload());
            }
            "back" => {
                with(id, |p| p.web.go_back());
            }
            "forward" => {
                with(id, |p| p.web.go_forward());
            }
            "stop_back" => {
                with(id, |p| {
                    let wv = p.web.webview();
                    wv.stop_loading();
                    if wv.can_go_back() {
                        wv.go_back();
                    }
                });
            }
            "zoom" => {
                with(id, |p| p.web.zoom(c["z"].as_f64().unwrap_or(1.0)));
            }
            "focus" => {
                with(id, |p| {
                    let wv = p.web.webview();
                    take_keyboard(&wv);
                    wv.grab_focus();
                });
            }
            "blur" => {
                with(id, |p| {
                    keys_to(&p.web.webview(), false);
                    give_keyboard(p.parent);
                });
            }
            "js" => match with(id, |p| p.web.evaluate_script(&s("script"))) {
                Some(_) => {}
                None => {
                    if let Some(v) = view_of(id) {
                        #[allow(deprecated)]
                        v.run_javascript(&s("script"), None::<&gtk::gio::Cancellable>, |_| {});
                    }
                }
            },
            "shot" => shot(id, c["token"].as_u64().unwrap_or(0), &s("path")),
            "devtools" => {
                with(id, |p| p.web.open_devtools());
            }
            "print" => {
                with(id, |p| p.web.print());
            }
            "close" => {
                PAGES.with_borrow_mut(|p| p.remove(&id));
            }
            "clear" => {
                let done = PAGES.with_borrow(|p| p.values().next().map(|p| p.web.clear_all_browsing_data().is_ok()));
                if done != Some(true) {
                    let _ = std::fs::remove_dir_all(s("data"));
                }
            }
            "dialog" => answer(c["did"].as_u64().unwrap_or(0), c["accept"].as_bool().unwrap_or(false), &s("text")),
            "upload" => {
                let files: Vec<String> = c["files"].as_array().map(|a| a.iter().filter_map(|f| f.as_str().map(str::to_string)).collect()).unwrap_or_default();
                with(id, |p| p.upload = (!files.is_empty()).then_some(files));
            }
            "mouse" => {
                if let Some(wv) = view_of(id) {
                    let f = |k: &str| c[k].as_f64().unwrap_or(0.0);
                    match c["kind"].as_str().unwrap_or("") {
                        "move" => input::moved(&wv, f("x"), f("y"), false),
                        "click" => {
                            take_keyboard(&wv);
                            input::click(&wv, f("x"), f("y"), c["count"].as_i64().unwrap_or(1).max(1) as i32);
                        }
                        "drag" => input::drag(&wv, (f("x"), f("y")), (f("x2"), f("y2"))),
                        _ => {}
                    }
                }
            }
            "key" => {
                if let Some(wv) = view_of(id) {
                    take_keyboard(&wv);
                    input::key(&wv, &s("key"));
                }
            }
            "text" => {
                if let Some(wv) = view_of(id) {
                    take_keyboard(&wv);
                    for ch in s("text").chars() {
                        input::char(&wv, ch);
                    }
                }
            }
            "agent" => {
                let sites = c["sites"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                ACTING.with_borrow_mut(|a| *a = Some(Acting { page: id, sites, pass: c["pass"].as_str().map(str::to_string), at: std::time::Instant::now() }));
            }
            "agent_stop" => ACTING.with_borrow_mut(|a| *a = None),
            "close_popups" => {
                let gone: Vec<gtk::Window> = POPUPS.with_borrow(|p| p.values().filter(|p| p.opener == id).map(|p| p.window.clone()).collect());
                for w in gone {
                    w.close();
                }
            }
            _ => {}
        }
    }

    fn new_page(id: u64, c: &Value) {
        let s = |k: &str| c[k].as_str().unwrap_or("").to_string();
        let parent = c["parent"].as_u64().unwrap_or(0) as std::ffi::c_ulong;
        let url = Some(s("url")).filter(|u| !u.is_empty()).unwrap_or_else(|| "about:blank".into());
        let dl = Some(s("downloads")).filter(|d| !d.is_empty());
        let built = CONTEXT.with_borrow_mut(|ctx| {
            let ctx = ctx.get_or_insert_with(|| wry::WebContext::new(Some(std::path::PathBuf::from(s("data")))));
            let mut b = WebViewBuilder::new_with_web_context(ctx)
                .with_url(&url)
                .with_bounds(rect(c))
                .with_visible(c["visible"].as_bool().unwrap_or(true))
                .with_devtools(true)
                .with_document_title_changed_handler(move |t| ev(id, "title", json!({ "v": t })))
                .with_on_page_load_handler(move |_, u| ev(id, "url", json!({ "v": u })))
                .with_ipc_handler(move |req| ev(id, "ipc", json!({ "v": req.body() })))
                .with_download_started_handler(move |url, path| {
                    if let (Some(dir), Some(name)) = (&dl, path.file_name().map(|n| n.to_owned())) {
                        *path = std::path::PathBuf::from(dir).join(name);
                    }
                    ev(id, "download_started", json!({ "url": url, "path": path.to_string_lossy() }));
                    true
                })
                .with_download_completed_handler(move |url, _, ok| ev(id, "download_done", json!({ "url": url, "ok": ok })))
                .with_new_window_req_handler(move |u, f| new_window(id, u, &f.opener.webview));
            for script in c["scripts"].as_array().into_iter().flatten() {
                b = b.with_initialization_script(script.as_str().unwrap_or(""));
            }
            b.build_as_child(&Parent(parent))
        });
        let web = match built {
            Ok(web) => web,
            Err(e) => {
                ev(id, "closed", json!({ "error": e.to_string() }));
                return;
            }
        };
        let wv = web.webview();
        // The page's window inside the app's (wry's, made by Xlib) hears the pointer: without
        // asking, X hands it to the app's window around it. Keys only once it has the keyboard
        // (`keys_to`).
        if let Some(w) = wv.toplevel().and_then(|t| t.window()) {
            use gdk::EventMask as M;
            w.set_events(
                (w.events()
                    | M::BUTTON_PRESS_MASK
                    | M::BUTTON_RELEASE_MASK
                    | M::POINTER_MOTION_MASK
                    | M::SCROLL_MASK
                    | M::SMOOTH_SCROLL_MASK
                    | M::ENTER_NOTIFY_MASK
                    | M::LEAVE_NOTIFY_MASK
                    | M::FOCUS_CHANGE_MASK
                    | M::STRUCTURE_MASK)
                    & !(M::KEY_PRESS_MASK | M::KEY_RELEASE_MASK),
            );
        }
        // No keys until it has the keyboard, WebKit's window too (made when the page first shows).
        keys_to(&wv, false);
        wv.connect_realize(|wv| keys_to(wv, false));
        watch(id, &wv);
        PAGES.with_borrow_mut(|p| p.insert(id, Page { web, parent, upload: None }));
    }

    /// WebKit's signals wry does not hand on.
    fn watch(id: u64, wv: &webkit2gtk::WebView) {
        wv.connect_is_loading_notify(move |wv| ev(id, "loading", json!({ "v": wv.is_loading() })));
        // The user's click: the page takes the keyboard (X11 gives it to the window clicked only
        // when told: the app's window is the one the window manager focused).
        wv.connect_button_press_event(move |wv, e| {
            if !e.is_send_event() {
                take_keyboard(wv);
                ev(id, "clicked", json!({}));
            }
            glib::Propagation::Proceed
        });
        wv.connect_focus_in_event(move |_, _| {
            ev(id, "focus", json!({ "v": true }));
            glib::Propagation::Proceed
        });
        wv.connect_focus_out_event(move |_, _| {
            ev(id, "focus", json!({ "v": false }));
            glib::Propagation::Proceed
        });
        wv.connect_key_press_event(move |_, e| {
            if e.is_send_event() {
                return glib::Propagation::Proceed;
            }
            match app_key(e) {
                Some((key, ctrl, shift, alt)) => {
                    ev(id, "key", json!({ "key": key, "ctrl": ctrl, "shift": shift, "alt": alt }));
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            }
        });
        wv.connect_script_dialog(move |_, d| {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let did = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let kind = match d.dialog_type() {
                ScriptDialogType::Confirm | ScriptDialogType::BeforeUnloadConfirm => "confirm",
                ScriptDialogType::Prompt => "prompt",
                _ => "alert",
            };
            let message = d.message().map(|m| m.to_string()).unwrap_or_default();
            let text = if kind == "prompt" { d.prompt_get_default_text().map(|m| m.to_string()).unwrap_or_default() } else { String::new() };
            DIALOGS.with_borrow_mut(|m| m.insert(did, d.clone()));
            ev(id, "dialog", json!({ "did": did, "kind": kind, "message": message, "text": text, "before_unload": d.dialog_type() == ScriptDialogType::BeforeUnloadConfirm }));
            true
        });
        wv.connect_run_file_chooser(move |_, req| {
            let files = PAGES.with_borrow_mut(|p| p.get_mut(&id).and_then(|p| p.upload.take()));
            let Some(files) = files else { return false };
            let take = if req.selects_multiple() { files.len() } else { 1 };
            let names: Vec<&str> = files.iter().take(take).map(String::as_str).collect();
            req.select_files(&names);
            ev(id, "chooser", json!({}));
            true
        });
    }

    fn answer(did: u64, accept: bool, text: &str) {
        let Some(d) = DIALOGS.with_borrow_mut(|m| m.remove(&did)) else { return };
        match d.dialog_type() {
            ScriptDialogType::Confirm | ScriptDialogType::BeforeUnloadConfirm => d.confirm_set_confirmed(accept),
            ScriptDialogType::Prompt if accept => d.prompt_set_text(text),
            _ => {}
        }
        d.close();
    }

    /// A key with Ctrl the app has: a browser's Ctrl+F, Ctrl+L, Ctrl+R and zoom keys, and every
    /// Ctrl+Shift key (the app's own). As (egui's name of the key, ctrl, shift, alt).
    fn app_key(e: &gdk::EventKey) -> Option<(String, bool, bool, bool)> {
        let state = e.state();
        let (ctrl, shift, alt) = (state.contains(gdk::ModifierType::CONTROL_MASK), state.contains(gdk::ModifierType::SHIFT_MASK), state.contains(gdk::ModifierType::MOD1_MASK));
        if !ctrl {
            return None;
        }
        // The key as without Shift (Ctrl+Shift+1 is "1", not "!").
        let keyval = gdk::Display::default()
            .and_then(|d| gdk::Keymap::for_display(&d))
            .and_then(|k| k.translate_keyboard_state(u32::from(e.hardware_keycode()), state - gdk::ModifierType::SHIFT_MASK, i32::from(e.group())))
            .map_or(e.keyval(), |(k, ..)| k.into());
        let name = match keyval.to_unicode() {
            Some(c) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase().to_string(),
            Some(c @ ('=' | '+' | '-' | ',' | '.' | '/' | '[' | ']' | '`')) => c.to_string(),
            _ => match keyval.name().as_deref() {
                Some("Return") | Some("KP_Enter") => "Enter".into(),
                Some("Page_Up") => "PageUp".into(),
                Some("Page_Down") => "PageDown".into(),
                Some("BackSpace") => "Backspace".into(),
                Some("Tab") | Some("ISO_Left_Tab") => "Tab".into(),
                _ => return None,
            },
        };
        let browser = !alt && matches!(name.as_str(), "F" | "L" | "R" | "=" | "+" | "-" | "0");
        // The page's own editing keys stay its (Ctrl+Shift+V pastes as plain text).
        let editing = matches!(name.as_str(), "C" | "V" | "X" | "A" | "Z");
        ((shift && !editing) || (!shift && browser)).then_some((name, ctrl, shift, alt))
    }

    /// The window of the page (wry's, inside the app's).
    fn page_window(wv: &webkit2gtk::WebView) -> Option<std::ffi::c_ulong> {
        let top = wv.toplevel()?;
        let w = top.window()?;
        let x = w.downcast::<gdkx11::X11Window>().ok()?;
        Some(x.xid() as std::ffi::c_ulong)
    }

    fn x_display() -> Option<*mut x11_dl::xlib::Display> {
        let d = gdk::Display::default()?;
        let d = d.downcast::<gdkx11::X11Display>().ok()?;
        // SAFETY: GDK's own connection to the X server, open while GTK runs.
        Some(unsafe { gdkx11::ffi::gdk_x11_display_get_xdisplay(d.as_ptr() as *mut _) } as *mut x11_dl::xlib::Display)
    }

    fn set_focus(window: std::ffi::c_ulong) {
        let Some(display) = x_display() else { return };
        let Some(gd) = gdk::Display::default().and_then(|d| d.downcast::<gdkx11::X11Display>().ok()) else { return };
        XLIB.with(|x| {
            if let Some(x) = x {
                // SAFETY: a plain request on GDK's connection; an error (a window not shown any
                // more) is trapped and ignored, not taken for the program's end.
                unsafe {
                    gdkx11::ffi::gdk_x11_display_error_trap_push(gd.as_ptr() as *mut _);
                    (x.XSetInputFocus)(display, window, x11_dl::xlib::RevertToParent, x11_dl::xlib::CurrentTime);
                    (x.XFlush)(display);
                    gdkx11::ffi::gdk_x11_display_error_trap_pop_ignored(gd.as_ptr() as *mut _);
                }
            }
        });
    }

    /// The page takes the keyboard (its window, inside the app's).
    fn take_keyboard(wv: &webkit2gtk::WebView) {
        keys_to(wv, true);
        if let Some(w) = page_window(wv) {
            set_focus(w);
        }
    }

    /// The page's window asks X for keys only while it has the keyboard. X hands a key to the
    /// window under the pointer when that is inside the window with the keyboard: with the app's
    /// window focused (the user types in a terminal) and the pointer resting on a page that asks
    /// for keys, the typing would go into the page.
    fn keys_to(wv: &webkit2gtk::WebView, on: bool) {
        use gdk::EventMask as M;
        let keys = M::KEY_PRESS_MASK | M::KEY_RELEASE_MASK;
        // Both of the page's windows: WebKit's own (under the pointer) and the one wry puts it in.
        for w in [wv.window(), wv.toplevel().and_then(|t| t.window())].into_iter().flatten() {
            w.set_events(if on { w.events() | keys } else { w.events() & !keys });
        }
    }

    /// The keyboard goes back to the app's window.
    fn give_keyboard(parent: std::ffi::c_ulong) {
        set_focus(parent);
    }

    /// A PNG of the page as it shows, into `path`; the app hears where.
    fn shot(id: u64, token: u64, path: &str) {
        let Some(wv) = view_of(id) else {
            ev(id, "shot", json!({ "token": token, "ok": false }));
            return;
        };
        let path = path.to_string();
        wv.snapshot(SnapshotRegion::Visible, SnapshotOptions::NONE, None::<&gtk::gio::Cancellable>, move |r| {
            let ok = r.ok().and_then(|surface| {
                let img = gtk::cairo::ImageSurface::try_from(surface).ok()?;
                let (w, h) = (img.width(), img.height());
                let pix = gdk::pixbuf_get_from_surface(&img, 0, 0, w, h)?;
                let png = pix.save_to_bufferv("png", &[]).ok()?;
                std::fs::write(&path, png).ok()
            });
            ev(id, "shot", json!({ "token": token, "ok": ok.is_some() }));
        });
    }

    /// A window the page opens: a sign-in pop-up (a window of a size) shows as its own window,
    /// tied to the page; a link to a new window becomes a new page in the app.
    /// Whether an agent may open `url` without asking (as the app's `agent_may_open`): local
    /// files and blank pages, its sites, the host the user let it open once.
    fn agent_may_open(url: &str, sites: &[String], pass: Option<&str>) -> bool {
        if url.bytes().any(|b| b < 0x20 || b == 0x7f) {
            return false;
        }
        let Some((scheme, after)) = url.split_once(':') else { return false };
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "http" | "https") {
            return matches!(scheme.as_str(), "about" | "file" | "data" | "blob");
        }
        let Some(rest) = after.strip_prefix("//") else { return false };
        let host = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
        let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
        let host = if let Some(v6) = host.strip_prefix('[') { format!("[{}]", v6.split(']').next().unwrap_or("")) } else { host.split(':').next().unwrap_or("").to_ascii_lowercase() };
        if host.is_empty() {
            return false;
        }
        if pass == Some(host.as_str()) {
            return true;
        }
        sites.iter().any(|s| {
            if s.trim() == "*" {
                return true;
            }
            let s = s.trim().trim_start_matches("*.").to_ascii_lowercase();
            !s.is_empty() && (host == s || host.ends_with(&format!(".{s}")))
        })
    }

    fn new_window(id: u64, url: String, opener: &webkit2gtk::WebView) -> wry::NewWindowResponse {
        // An agent's step a moment ago opened it on this page (or its pop-up): to its sites only
        // (else the user is asked).
        let agent = ACTING.with_borrow(|a| {
            a.as_ref()
                .filter(|a| {
                    a.at.elapsed() < std::time::Duration::from_secs(3)
                        && (a.page == id || POPUPS.with_borrow(|p| p.get(&a.page).is_some_and(|pop| pop.opener == id)))
                })
                .map(|a| agent_may_open(&url, &a.sites, a.pass.as_deref()))
        });
        if agent == Some(false) {
            ev(id, "blocked", json!({ "url": url }));
            return wry::NewWindowResponse::Deny;
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 32);
        let pid = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let popup = webkit2gtk::WebView::builder().related_view(opener).build();
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title(&url);
        window.add(&popup);
        let shown = Rc::new(std::cell::Cell::new(false));
        let (w, s, u) = (window.clone(), shown.clone(), url.clone());
        popup.connect_ready_to_show(move |pv| {
            // A window opened with a size (`window.open(url, name, "width=480,height=560")`) is a
            // pop-up; a link to a new window comes with WebKit's 100×100 (or none).
            let geometry = pv.window_properties().map(|p| p.geometry());
            match geometry.filter(|g| g.width() > 100 || g.height() > 100) {
                Some(g) => {
                    w.set_default_size(g.width().clamp(320, 1400), g.height().clamp(240, 1000));
                    w.set_position(gtk::WindowPosition::Center);
                    // An agent's pop-up does not take the keyboard from the user's terminal.
                    w.set_focus_on_map(agent.is_none());
                    w.show_all();
                    s.set(true);
                    POPUPS.with_borrow_mut(|p| p.insert(pid, Popup { view: pv.clone(), window: w.clone(), opener: id }));
                    ev(id, "popup", json!({ "pid": pid, "url": u }));
                }
                None if !u.is_empty() && u != "about:blank" => {
                    // Not a pop-up: the app opens it as a new page.
                    ev(id, "open_here", json!({ "url": u }));
                    w.close();
                }
                None => {
                    w.set_default_size(600, 700);
                    w.set_focus_on_map(agent.is_none());
                    w.show_all();
                    s.set(true);
                    POPUPS.with_borrow_mut(|p| p.insert(pid, Popup { view: pv.clone(), window: w.clone(), opener: id }));
                    ev(id, "popup", json!({ "pid": pid, "url": u }));
                }
            }
        });
        let w = window.clone();
        popup.connect_close(move |_| w.close());
        window.connect_destroy(move |_| {
            if POPUPS.with_borrow_mut(|p| p.remove(&pid)).is_some() {
                ev(id, "popup_closed", json!({ "pid": pid }));
            }
        });
        popup.connect_uri_notify(move |pv| ev(pid, "url", json!({ "v": pv.uri().map(|u| u.to_string()).unwrap_or_default() })));
        popup.connect_is_loading_notify(move |pv| ev(pid, "loading", json!({ "v": pv.is_loading() })));
        popup.connect_title_notify(move |pv| {
            if let Some(t) = pv.title() {
                window.set_title(&t);
            }
        });
        wry::NewWindowResponse::Create { webview: popup }
    }

    /// An agent's input, as GDK events from the window system (WebKit takes them as the user's:
    /// they reach into frames of other sites and open file choosers).
    mod input {
        use gtk::prelude::*;

        fn target(wv: &webkit2gtk::WebView) -> Option<(gdk::Window, gdk::Device, f64, f64)> {
            let win = wv.window()?;
            let seat = gdk::Display::default()?.default_seat()?;
            let pointer = seat.pointer()?;
            let alloc = wv.allocation();
            Some((win, pointer, f64::from(alloc.x()), f64::from(alloc.y())))
        }

        /// The page's zoom: points of the page are CSS pixels.
        fn zoom(wv: &webkit2gtk::WebView) -> f64 {
            use webkit2gtk::WebViewExt;
            wv.zoom_level().max(0.01)
        }

        fn button(wv: &webkit2gtk::WebView, ty: gdk::EventType, x: f64, y: f64, n: i32) {
            let Some((win, device, ox, oy)) = target(wv) else { return };
            let z = zoom(wv);
            let (x, y) = (ox + x * z, oy + y * z);
            // SAFETY: a new event, filled in as GDK's own (window and device referenced), handed
            // to GTK and freed.
            unsafe {
                let e = gdk::ffi::gdk_event_new(ty.into_glib());
                let b = e as *mut gdk::ffi::GdkEventButton;
                (*b).window = win.as_ptr() as *mut _;
                gtk::glib::gobject_ffi::g_object_ref((*b).window as *mut _);
                (*b).send_event = 1;
                (*b).time = gtk::current_event_time();
                (*b).x = x;
                (*b).y = y;
                (*b).button = 1;
                (*b).state = if ty == gdk::EventType::ButtonRelease { gdk::ffi::GDK_BUTTON1_MASK } else { 0 };
                let (_, rx, ry) = win.origin();
                (*b).x_root = f64::from(rx) + x;
                (*b).y_root = f64::from(ry) + y;
                gdk::ffi::gdk_event_set_device(e, device.as_ptr());
                let _ = n;
                gtk::ffi::gtk_main_do_event(e);
                gdk::ffi::gdk_event_free(e);
            }
        }

        pub fn moved(wv: &webkit2gtk::WebView, x: f64, y: f64, held: bool) {
            let Some((win, device, ox, oy)) = target(wv) else { return };
            let z = zoom(wv);
            let (x, y) = (ox + x * z, oy + y * z);
            // SAFETY: as in `button`.
            unsafe {
                let e = gdk::ffi::gdk_event_new(gdk::ffi::GDK_MOTION_NOTIFY);
                let m = e as *mut gdk::ffi::GdkEventMotion;
                (*m).window = win.as_ptr() as *mut _;
                gtk::glib::gobject_ffi::g_object_ref((*m).window as *mut _);
                (*m).send_event = 1;
                (*m).time = gtk::current_event_time();
                (*m).x = x;
                (*m).y = y;
                (*m).state = if held { gdk::ffi::GDK_BUTTON1_MASK } else { 0 };
                let (_, rx, ry) = win.origin();
                (*m).x_root = f64::from(rx) + x;
                (*m).y_root = f64::from(ry) + y;
                gdk::ffi::gdk_event_set_device(e, device.as_ptr());
                gtk::ffi::gtk_main_do_event(e);
                gdk::ffi::gdk_event_free(e);
            }
        }

        pub fn click(wv: &webkit2gtk::WebView, x: f64, y: f64, count: i32) {
            moved(wv, x, y, false);
            for n in 1..=count {
                button(wv, gdk::EventType::ButtonPress, x, y, n);
                if n == 2 {
                    button(wv, gdk::EventType::DoubleButtonPress, x, y, n);
                }
                button(wv, gdk::EventType::ButtonRelease, x, y, n);
            }
        }

        pub fn drag(wv: &webkit2gtk::WebView, from: (f64, f64), to: (f64, f64)) {
            moved(wv, from.0, from.1, false);
            button(wv, gdk::EventType::ButtonPress, from.0, from.1, 1);
            const STEPS: usize = 12;
            for i in 1..=STEPS {
                let f = i as f64 / STEPS as f64;
                moved(wv, from.0 + (to.0 - from.0) * f, from.1 + (to.1 - from.1) * f, true);
            }
            button(wv, gdk::EventType::ButtonRelease, to.0, to.1, 1);
        }

        fn press(wv: &webkit2gtk::WebView, keyval: u32) {
            let Some(win) = wv.window() else { return };
            let Some(keymap) = gdk::Display::default().and_then(|d| gdk::Keymap::for_display(&d)) else { return };
            let entries = keymap.entries_for_keyval(keyval);
            let (code, shift) = entries.first().map_or((0u16, false), |k| (k.keycode() as u16, k.level() == 1));
            let device = gdk::Display::default().and_then(|d| d.default_seat()).and_then(|s| s.keyboard());
            for ty in [gdk::ffi::GDK_KEY_PRESS, gdk::ffi::GDK_KEY_RELEASE] {
                // SAFETY: as in `button`, for a key event.
                unsafe {
                    let e = gdk::ffi::gdk_event_new(ty);
                    let k = e as *mut gdk::ffi::GdkEventKey;
                    (*k).window = win.as_ptr() as *mut _;
                    gtk::glib::gobject_ffi::g_object_ref((*k).window as *mut _);
                    (*k).send_event = 1;
                    (*k).time = gtk::current_event_time();
                    (*k).keyval = keyval;
                    (*k).hardware_keycode = code;
                    (*k).state = if shift { gdk::ffi::GDK_SHIFT_MASK } else { 0 };
                    if let Some(d) = &device {
                        gdk::ffi::gdk_event_set_device(e, d.as_ptr());
                    }
                    gtk::ffi::gtk_main_do_event(e);
                    gdk::ffi::gdk_event_free(e);
                }
            }
        }

        /// A named key (`Enter`, `Tab`, `ArrowDown`…) or one character.
        pub fn key(wv: &webkit2gtk::WebView, key: &str) {
            let named = match key {
                "Enter" => Some(gdk::keys::constants::Return),
                "Tab" => Some(gdk::keys::constants::Tab),
                "Escape" | "Esc" => Some(gdk::keys::constants::Escape),
                "Backspace" => Some(gdk::keys::constants::BackSpace),
                "Delete" => Some(gdk::keys::constants::Delete),
                " " | "Space" => Some(gdk::keys::constants::space),
                "ArrowUp" => Some(gdk::keys::constants::Up),
                "ArrowDown" => Some(gdk::keys::constants::Down),
                "ArrowLeft" => Some(gdk::keys::constants::Left),
                "ArrowRight" => Some(gdk::keys::constants::Right),
                "Home" => Some(gdk::keys::constants::Home),
                "End" => Some(gdk::keys::constants::End),
                "PageUp" => Some(gdk::keys::constants::Page_Up),
                "PageDown" => Some(gdk::keys::constants::Page_Down),
                _ => None,
            };
            match named {
                Some(k) => press(wv, *k),
                None => {
                    if let Some(c) = key.chars().next() {
                        char(wv, c);
                    }
                }
            }
        }

        pub fn char(wv: &webkit2gtk::WebView, c: char) {
            match c {
                '\n' => press(wv, *gdk::keys::constants::Return),
                '\t' => press(wv, *gdk::keys::constants::Tab),
                c => press(wv, *gdk::keys::Key::from_unicode(c)),
            }
        }

        use gtk::glib::translate::IntoGlib;
    }
}
