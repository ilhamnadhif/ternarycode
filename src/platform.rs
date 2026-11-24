//! Small platform services: desktop notifications, the Dock badge, hiding the app when its window
//! is closed (FR-L1), and the cleaned-up environment panes inherit.

/// Variables that mark the process as running inside an agent session. When TernaryCode is
/// started from such a session they would leak into every pane and make agents there behave as
/// nested sessions (seen in Gate A with `CLAUDE_CODE_CHILD_SESSION`).
const INHERITED_MARKERS: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SSE_PORT",
    "CODEX_SANDBOX",
    "CODEX_SANDBOX_NETWORK_DISABLED",
    "TERNARYCODE_SOCK",
    "TERNARYCODE_PANE",
    "TERNARYCODE_TOKEN",
    "TERNARYCODE_HOOK",
    // Set on the copy started apart from a terminal (`cli::launch_detached`; `main` reads it
    // first): a TernaryCode started from a pane is not that copy.
    "TERNARYCODE_DETACHED",
];

/// Removes the markers above from this process's environment. Call at the start of `main`,
/// before any thread exists.
pub fn clean_inherited_env() {
    for name in INHERITED_MARKERS {
        if std::env::var_os(name).is_some() {
            // SAFETY: called at the very start of main, before other threads run.
            unsafe { std::env::remove_var(name) };
        }
    }
    // A pane's `$BROWSER` (TernaryCode's own, next to its hook socket) is not the user's.
    if std::env::var("BROWSER").is_ok_and(|b| b.ends_with("/browse.js") && b.contains("/ternarycode-")) {
        // SAFETY: as above.
        unsafe { std::env::remove_var("BROWSER") };
    }
}

#[cfg(target_os = "macos")]
fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A command for one of the system's tools, without a console window flashing up on Windows
/// (the app has no console of its own there).
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    no_window(&mut cmd);
    cmd
}

/// Windows: `cmd` runs without a console window. Nothing elsewhere.
pub fn no_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Shows a desktop notification. Runs the system's tool on a separate thread. (Windows: none;
/// the app's own attention list and the taskbar show what needs the user.)
pub fn notify(title: &str, subtitle: &str, body: &str) {
    let (title, subtitle, body) = (title.to_string(), subtitle.to_string(), body.to_string());
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = (title, subtitle, body);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    std::thread::spawn(move || {
        #[cfg(target_os = "macos")]
        {
            let script = format!(
                "display notification {} with title {} subtitle {}",
                applescript_string(&body),
                applescript_string(&title),
                applescript_string(&subtitle)
            );
            let _ = command("/usr/bin/osascript")
                .args(["-e", &script])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        #[cfg(target_os = "linux")]
        {
            let summary = if subtitle.is_empty() {
                title
            } else {
                format!("{title} · {subtitle}")
            };
            let _ = command("notify-send")
                .args(["--app-name=TernaryCode", &summary, &body])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    });
}

/// Gives memory the allocator keeps after frees back to the system (after letting a large
/// cache go): macOS keeps freed pages for reuse, so the process would not look any smaller.
#[cfg(target_os = "macos")]
pub fn return_free_memory() {
    unsafe extern "C" {
        fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
    }
    // SAFETY: a null zone means every zone; goal 0 means as much as possible.
    unsafe {
        malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
    }
}

#[cfg(target_os = "linux")]
pub fn return_free_memory() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> i32;
    }
    // SAFETY: glibc's malloc_trim only releases free memory.
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn return_free_memory() {}

/// Where the pointer is in the window, in egui points, asked of the system: while files are
/// dragged over the window the app hears of no pointer movement.
#[cfg(target_os = "macos")]
pub fn pointer_in_window(_ctx: &eframe::egui::Context) -> Option<eframe::egui::Pos2> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent};
    let mtm = MainThreadMarker::new()?;
    let windows = NSApplication::sharedApplication(mtm).windows();
    let w = (0..windows.count()).map(|i| windows.objectAtIndex(i)).max_by(|a, b| {
        let (ra, rb) = (a.contentLayoutRect(), b.contentLayoutRect());
        (ra.size.width * ra.size.height).total_cmp(&(rb.size.width * rb.size.height))
    })?;
    let p = w.convertPointFromScreen(NSEvent::mouseLocation());
    let height = w.contentView().map(|v| v.frame().size.height)?;
    Some(eframe::egui::pos2(p.x as f32, (height - p.y) as f32))
}

#[cfg(windows)]
pub fn pointer_in_window(ctx: &eframe::egui::Context) -> Option<eframe::egui::Pos2> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: a plain call filling a POINT owned here.
    if unsafe { GetCursorPos(&mut p) } == 0 {
        return None;
    }
    let (inner, ppp) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().native_pixels_per_point));
    let ppp = ppp.unwrap_or(1.0);
    Some(eframe::egui::pos2(p.x as f32 / ppp, p.y as f32 / ppp) - inner?.min.to_vec2())
}

/// Elsewhere the last position the window heard of.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn pointer_in_window(ctx: &eframe::egui::Context) -> Option<eframe::egui::Pos2> {
    ctx.input(|i| i.pointer.latest_pos())
}

/// A dropped file's path as the terminals of this system type it: with a backslash before
/// spaces and the characters a shell reads (as Terminal.app, iTerm2 and Ghostty do); in double
/// quotes on Windows when it has a space.
pub fn dropped_path(path: &str) -> String {
    if cfg!(windows) {
        return if path.contains(' ') { format!("\"{path}\"") } else { path.to_string() };
    }
    let mut out = String::with_capacity(path.len() + 8);
    for c in path.chars() {
        if c.is_whitespace() || "\\'\"`$&|;<>()[]{}*?!#~^".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Turns off macOS's automatic window tabbing (it would add "Show Tab Bar" and "Show All Tabs"
/// to the View menu; TernaryCode has its own tabs). Call before the window opens.
#[cfg(target_os = "macos")]
pub fn disable_window_tabbing() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSWindow;
    if let Some(mtm) = MainThreadMarker::new() {
        NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn disable_window_tabbing() {}

/// Whether dragging the top of the window moves it. Off while the pointer is over the terminal
/// tabs: they sit where the (hidden) title bar is, and macOS would move the window instead of
/// letting the tab be dragged.
#[cfg(target_os = "macos")]
pub fn set_window_movable(movable: bool) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let Some(mtm) = MainThreadMarker::new() else { return };
    let windows = NSApplication::sharedApplication(mtm).windows();
    for i in 0..windows.count() {
        let w = windows.objectAtIndex(i);
        if w.isMovable() != movable {
            w.setMovable(movable);
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_window_movable(_movable: bool) {}

/// Windows: the app draws the title bar itself (mark, name, menus and the window buttons, above
/// the sidebar and the top bar; `ui::caption`) and its resize edges: the system's caption is off
/// (`with_decorations(false)`).
pub fn custom_caption() -> bool {
    cfg!(windows)
}

/// The window, for the system menu (Windows).
#[cfg(windows)]
static WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Windows: keeps the window for `show_system_menu`, and asks Windows 11 for rounded corners
/// (a window without the system caption gets square ones; Windows 10 ignores it). egui-winit
/// gives it the shadow.
#[cfg(windows)]
pub fn init_window_frame(window: &impl raw_window_handle::HasWindowHandle) {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
    let Ok(handle) = window.window_handle() else { return };
    let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() else { return };
    let hwnd = h.hwnd.get();
    WINDOW.store(hwnd, std::sync::atomic::Ordering::Relaxed);
    let round = DWMWCP_ROUND;
    unsafe {
        DwmSetWindowAttribute(hwnd as _, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&raw const round).cast(), size_of_val(&round) as u32);
    }
}

/// Windows: the window's system menu (Restore, Move, Size, Minimise, Maximise, Close) where the
/// pointer is, as a right click on a title bar shows it; the item chosen is carried out.
#[cfg(windows)]
pub fn show_system_menu(maximized: bool) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnableMenuItem, GetCursorPos, GetSystemMenu, MF_BYCOMMAND, MFS_DISABLED, MFS_ENABLED, PostMessageW, SC_CLOSE, SC_MAXIMIZE,
        SC_MINIMIZE, SC_MOVE, SC_RESTORE, SC_SIZE, SetMenuDefaultItem, TPM_LEFTALIGN, TPM_RETURNCMD, TrackPopupMenu, WM_SYSCOMMAND,
    };
    let hwnd = WINDOW.load(std::sync::atomic::Ordering::Relaxed);
    if hwnd == 0 {
        return;
    }
    let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let menu = GetSystemMenu(hwnd, 0);
        if menu.is_null() {
            return;
        }
        let state = |on: bool| MF_BYCOMMAND | if on { MFS_ENABLED } else { MFS_DISABLED };
        EnableMenuItem(menu, SC_RESTORE, state(maximized));
        EnableMenuItem(menu, SC_MOVE, state(!maximized));
        EnableMenuItem(menu, SC_SIZE, state(!maximized));
        EnableMenuItem(menu, SC_MINIMIZE, state(true));
        EnableMenuItem(menu, SC_MAXIMIZE, state(!maximized));
        EnableMenuItem(menu, SC_CLOSE, state(true));
        SetMenuDefaultItem(menu, SC_CLOSE, 0);
        let mut at = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut at) == 0 {
            return;
        }
        let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN, at.x, at.y, 0, hwnd, std::ptr::null());
        if chosen != 0 {
            PostMessageW(hwnd, WM_SYSCOMMAND, chosen as usize, 0);
        }
    }
}

#[cfg(not(windows))]
pub fn show_system_menu(_maximized: bool) {}

/// macOS: the window buttons (close, minimise, zoom) centred in the top row, `row_h` points
/// high (the sidebar's and the top bar's), not in the 28 of a title bar. AppKit lays the title
/// bar out again on some changes (a resize, full screen): called every frame, it only moves
/// what is not where it belongs.
#[cfg(target_os = "macos")]
pub fn place_window_buttons(window: &impl raw_window_handle::HasWindowHandle, row_h: f64) {
    use objc2_app_kit::{NSView, NSWindowButton, NSWindowStyleMask};
    let Ok(handle) = window.window_handle() else { return };
    let raw_window_handle::RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    // SAFETY: the handle's view is the window's content view, alive while the window is.
    let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
    let Some(win) = view.window() else { return };
    if win.styleMask().contains(NSWindowStyleMask::FullScreen) {
        return;
    }
    let kinds = [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton];
    let Some(close) = win.standardWindowButton(kinds[0]) else { return };
    // The buttons' view, and the title bar's container around it: made as tall as the row.
    let Some(bar) = (unsafe { close.superview() }) else { return };
    let Some(container) = (unsafe { bar.superview() }) else { return };
    let window_h = win.frame().size.height;
    let mut cf = container.frame();
    if (cf.size.height - row_h).abs() > 0.5 || (cf.origin.y - (window_h - row_h)).abs() > 0.5 {
        cf.size.height = row_h;
        cf.origin.y = window_h - row_h;
        container.setFrame(cf);
    }
    let mut bf = bar.frame();
    if (bf.size.height - row_h).abs() > 0.5 || bf.origin.y.abs() > 0.5 {
        bf.size.height = row_h;
        bf.origin.y = 0.0;
        bar.setFrame(bf);
    }
    let bh = close.frame().size.height;
    let y = ((row_h - bh) / 2.0).round();
    for kind in kinds {
        if let Some(b) = win.standardWindowButton(kind) {
            let mut o = b.frame().origin;
            if (o.y - y).abs() > 0.5 {
                o.y = y;
                b.setFrameOrigin(o);
            }
        }
    }
}

/// Room the window buttons (close, minimise, zoom) take at the top left when the content runs
/// under the title bar (macOS, not in full screen).
pub fn traffic_light_inset(fullscreen: bool) -> f32 {
    if cfg!(target_os = "macos") && !fullscreen { 76.0 } else { 0.0 }
}

/// The number on the Dock icon (`0` clears it).
#[cfg(target_os = "macos")]
pub fn set_badge(count: usize) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let tile = NSApplication::sharedApplication(mtm).dockTile();
    let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
    tile.setBadgeLabel(label.as_deref());
}

#[cfg(not(target_os = "macos"))]
pub fn set_badge(_count: usize) {}

/// Hides the app (like ⌘H): the window and every pane keep running; clicking the Dock icon brings
/// it back.
#[cfg(target_os = "macos")]
pub fn hide_app() -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    NSApplication::sharedApplication(mtm).hide(None);
    true
}

#[cfg(not(target_os = "macos"))]
pub fn hide_app() -> bool {
    false
}

/// Opens a folder in the file manager, a file in its default app, or a web link in the browser.
pub fn reveal(path: &std::path::Path) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer.exe"
    } else {
        "xdg-open"
    };
    let _ = command(program).arg(path).spawn();
}

/// Whether `url` is an address `open_url` opens: a web page or an e-mail address. Other schemes
/// (which could start any app) are not.
pub fn openable_url(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
}

/// Opens a web address in the default browser (or an e-mail address in the mail app).
pub fn open_url(url: &str) {
    if !openable_url(url) {
        return;
    }
    // The end-to-end tests note it instead.
    #[cfg(debug_assertions)]
    if let Some(file) = std::env::var_os("TERNARYCODE_E2E_OPENED") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
            let _ = writeln!(f, "{url}");
        }
        return;
    }
    let mut cmd = if cfg!(target_os = "macos") {
        command("/usr/bin/open")
    } else if cfg!(windows) {
        let mut c = command("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        command("xdg-open")
    };
    let _ = cmd
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Whether a file is a page a web browser shows (HTML, SVG): its menu offers Open in Browser.
pub fn browser_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "html" | "htm" | "xhtml" | "svg"))
}

/// Opens a local file (a static HTML page, an SVG) in the default web browser.
pub fn open_in_browser(path: &std::path::Path) {
    // The end-to-end tests note it instead.
    #[cfg(debug_assertions)]
    if let Some(file) = std::env::var_os("TERNARYCODE_E2E_OPENED") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
            let _ = writeln!(f, "browser {}", path.display());
        }
        return;
    }
    // The app that opens web addresses: the default browser, which need not be the app set
    // for .html files (often an editor).
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = command("/usr/bin/open");
        if let Some(app) = default_browser() {
            c.arg("-a").arg(app);
        }
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        let mut c = command("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut cmd = command("xdg-open");
    let _ = cmd
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// The app macOS opens web addresses with (the default browser).
#[cfg(target_os = "macos")]
fn default_browser() -> Option<std::path::PathBuf> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};
    let url = NSURL::URLWithString(&NSString::from_str("https://example.com"))?;
    let app = NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&url)?;
    app.path().map(|p| std::path::PathBuf::from(p.to_string()))
}

/// Shows a file or folder selected in the file manager (Finder, Explorer), or opens the folder
/// holding it.
pub fn reveal_in_folder(path: &std::path::Path) {
    if cfg!(target_os = "macos") {
        let _ = command("open").arg("-R").arg(path).spawn();
    } else if cfg!(windows) {
        #[cfg(windows)]
        {
            // Explorer wants exactly `/select,"<path>"`, not the argument quoted as a whole.
            use std::os::windows::process::CommandExt;
            let _ = command("explorer.exe")
                .raw_arg(format!("/select,\"{}\"", path.display()))
                .spawn();
        }
    } else if let Some(dir) = path.parent() {
        let _ = command("xdg-open").arg(dir).spawn();
    }
}

/// Moves a file or folder to the Trash (it can be put back from there).
#[cfg(target_os = "macos")]
pub fn trash(path: &std::path::Path) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|e| e.localizedDescription().to_string())
}

/// Moves a file or folder to the Recycle Bin (the shell's delete with undo).
#[cfg(windows)]
pub fn trash(path: &std::path::Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };
    // A list of paths, each ending in NUL, the list in another NUL.
    let from: Vec<u16> = path.as_os_str().encode_wide().chain([0, 0]).collect();
    // SAFETY: all-zero is a valid SHFILEOPSTRUCTW; `from` outlives the call.
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE;
    op.pFrom = from.as_ptr();
    op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16;
    // SAFETY: a filled-in operation on our own buffers.
    let r = unsafe { SHFileOperationW(&mut op) };
    if r == 0 && op.fAnyOperationsAborted == 0 {
        Ok(())
    } else {
        Err(crate::i18n::tf("The file could not be moved to the Recycle Bin (error {}).", &[&r.to_string()]))
    }
}

/// Moves a file or folder to the Trash through `gio trash` (GNOME, and most desktops).
#[cfg(not(any(target_os = "macos", windows)))]
pub fn trash(path: &std::path::Path) -> Result<(), String> {
    match command("gio").arg("trash").arg(path).status() {
        Ok(s) if s.success() => Ok(()),
        _ => Err(crate::i18n::t("This system has no Trash TernaryCode can use (gio trash).").to_string()),
    }
}

#[cfg(debug_assertions)]
static E2E_CLIPBOARD_FILES: std::sync::Mutex<Option<Vec<std::path::PathBuf>>> = std::sync::Mutex::new(None);

/// Test hook: overrides the OS file clipboard inside E2E runs without touching the user's real clipboard.
#[cfg(debug_assertions)]
pub fn set_e2e_clipboard_files(paths: Option<Vec<std::path::PathBuf>>) {
    if let Ok(mut guard) = E2E_CLIPBOARD_FILES.lock() {
        *guard = paths;
    }
}

/// Parses a `text/uri-list` payload (`file://...` lines, percent-encoded) into local paths.
pub fn parse_file_uri_list(text: &str) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let rest = if let Some(r) = line.strip_prefix("file://localhost") {
            r
        } else if let Some(r) = line.strip_prefix("file://") {
            r
        } else {
            continue;
        };
        let mut bytes = Vec::with_capacity(rest.len());
        let b = rest.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%'
                && i + 2 < b.len()
                && let (Some(hi), Some(lo)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16))
            {
                bytes.push(((hi << 4) | lo) as u8);
                i += 3;
            } else {
                bytes.push(b[i]);
                i += 1;
            }
        }
        if let Ok(decoded) = String::from_utf8(bytes)
            && !decoded.is_empty()
        {
            out.push(std::path::PathBuf::from(decoded));
        }
    }
    out
}

/// Reads copied files/folders from the OS clipboard (e.g. `⌘C` in Finder, `Ctrl+C` in Explorer/Files).
pub fn clipboard_files() -> Vec<std::path::PathBuf> {
    #[cfg(debug_assertions)]
    if let Ok(guard) = E2E_CLIPBOARD_FILES.lock()
        && let Some(paths) = guard.as_ref()
    {
        return paths.clone();
    }
    os_clipboard_files()
}

/// Writes files/folders to the OS clipboard so they can be pasted into folders in TernaryCode or Finder.
pub fn copy_files_to_clipboard(paths: &[std::path::PathBuf]) -> bool {
    if paths.is_empty() {
        return false;
    }
    #[cfg(debug_assertions)]
    if std::env::var_os("TERNARYCODE_DEV_SCRIPT").is_some()
        || E2E_CLIPBOARD_FILES.lock().ok().is_some_and(|g| g.is_some())
    {
        set_e2e_clipboard_files(Some(paths.to_vec()));
        return true;
    }
    os_copy_files_to_clipboard(paths)
}

#[cfg(target_os = "macos")]
fn os_clipboard_files() -> Vec<std::path::PathBuf> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL};
    use objc2_foundation::NSURL;

    let pb = NSPasteboard::generalPasteboard();
    let Some(items) = pb.pasteboardItems() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in 0..items.count() {
        let item = items.objectAtIndex(i);
        // SAFETY: NSPasteboardTypeFileURL is a static AppKit extern constant.
        let Some(raw) = item.stringForType(unsafe { NSPasteboardTypeFileURL }) else {
            continue;
        };
        let Some(url) = NSURL::URLWithString(&raw) else {
            continue;
        };
        // Finder writes file-reference URLs (`file:///.file/id=...`): resolve to a real POSIX path.
        let resolved = url.filePathURL().unwrap_or(url);
        if let Some(path) = resolved.path() {
            let s = path.to_string();
            if !s.is_empty() {
                out.push(std::path::PathBuf::from(s));
            }
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn os_copy_files_to_clipboard(paths: &[std::path::PathBuf]) -> bool {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSString, NSURL};

    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    let mut objs: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = Vec::with_capacity(paths.len());
    for path in paths {
        let abs = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.clone())
        };
        let url = NSURL::fileURLWithPath(&NSString::from_str(&abs.to_string_lossy()));
        let Some(url_str) = url.absoluteString() else { continue };
        let item = NSPasteboardItem::new();
        // SAFETY: NSPasteboardTypeFileURL is a static AppKit extern constant.
        if item.setString_forType(&url_str, unsafe { NSPasteboardTypeFileURL }) {
            objs.push(ProtocolObject::from_retained(item));
        }
    }
    if objs.is_empty() {
        return false;
    }
    let arr = NSArray::from_retained_slice(&objs);
    pb.writeObjects(&arr)
}

#[cfg(windows)]
fn os_clipboard_files() -> Vec<std::path::PathBuf> {
    use std::ffi::{OsString, c_void};
    use std::os::windows::ffi::OsStringExt;

    const CF_HDROP: u32 = 15;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(hwnd: *mut c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn GetClipboardData(format: u32) -> *mut c_void;
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn DragQueryFileW(hdrop: *mut c_void, i_file: u32, lpsz_file: *mut u16, cch: u32) -> u32;
    }
    let mut out = Vec::new();
    // SAFETY: OpenClipboard is paired with CloseClipboard on success; DragQueryFileW reads CF_HDROP.
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return out;
        }
        let hdrop = GetClipboardData(CF_HDROP);
        if !hdrop.is_null() {
            let count = DragQueryFileW(hdrop, u32::MAX, std::ptr::null_mut(), 0);
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, std::ptr::null_mut(), 0);
                if len > 0 {
                    let mut buf = vec![0u16; len as usize + 1];
                    let got = DragQueryFileW(hdrop, i, buf.as_mut_ptr(), buf.len() as u32);
                    if got > 0 {
                        buf.truncate(got as usize);
                        out.push(std::path::PathBuf::from(OsString::from_wide(&buf)));
                    }
                }
            }
        }
        CloseClipboard();
    }
    out
}

#[cfg(windows)]
fn os_copy_files_to_clipboard(_paths: &[std::path::PathBuf]) -> bool {
    false
}

#[cfg(not(any(target_os = "macos", windows)))]
fn os_clipboard_files() -> Vec<std::path::PathBuf> {
    let try_cmd = |program: &str, args: &[&str]| -> Option<Vec<std::path::PathBuf>> {
        let out = command(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let paths = parse_file_uri_list(&String::from_utf8_lossy(&out.stdout));
        (!paths.is_empty()).then_some(paths)
    };
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && let Some(paths) = try_cmd("wl-paste", &["--no-newline", "--type", "text/uri-list"])
    {
        return paths;
    }
    try_cmd("xclip", &["-selection", "clipboard", "-o", "-t", "text/uri-list"]).unwrap_or_default()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn os_copy_files_to_clipboard(paths: &[std::path::PathBuf]) -> bool {
    use std::io::Write as _;
    let payload: String = paths
        .iter()
        .map(|p| {
            let s = p.to_string_lossy();
            let mut enc = String::from("file://");
            for b in s.as_bytes() {
                match *b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => enc.push(*b as char),
                    other => {
                        let _ = std::fmt::Write::write_fmt(&mut enc, format_args!("%{other:02X}"));
                    }
                }
            }
            enc
        })
        .collect::<Vec<_>>()
        .join("\r\n");
    let try_write = |program: &str, args: &[&str]| -> bool {
        let Ok(mut child) = command(program)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return false;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(payload.as_bytes());
        }
        child.wait().is_ok_and(|s| s.success())
    };
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && try_write("wl-copy", &["--type", "text/uri-list"])
    {
        return true;
    }
    try_write("xclip", &["-selection", "clipboard", "-i", "-t", "text/uri-list"])
}

/// `path` quoted for PowerShell (the shell on Windows) when it needs it: single quotes, with
/// `'` doubled inside.
#[cfg(windows)]
pub fn shell_quote(path: &str) -> String {
    let plain = !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '\\' | '.' | '_' | '-' | '+' | ',' | ':' | '='));
    if plain {
        path.to_string()
    } else {
        format!("'{}'", path.replace('\'', "''"))
    }
}

/// `path` quoted for a POSIX shell when it needs it.
#[cfg(not(windows))]
pub fn shell_quote(path: &str) -> String {
    let plain = !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | ',' | '@' | ':' | '='));
    if plain {
        path.to_string()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

/// Lets the app open more files at once than the 256 macOS gives an app started from the Dock
/// (every pane holds a few: its terminal, the hook socket's connections, language servers).
pub fn raise_open_files_limit() {
    #[cfg(unix)]
    // SAFETY: plain syscalls on a struct owned here.
    unsafe {
        let mut r = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) == 0 {
            let want = r.rlim_max.min(10_240);
            if r.rlim_cur < want {
                r.rlim_cur = want;
                let _ = libc::setrlimit(libc::RLIMIT_NOFILE, &r);
            }
        }
    }
}

#[cfg(test)]
mod tests {

    /// The default browser is found (Open in Browser opens pages there, not in the app set
    /// for .html files).
    #[cfg(target_os = "macos")]
    #[test]
    fn finds_the_default_browser() {
        assert!(super::default_browser().is_some_and(|app| app.extension().is_some_and(|e| e == "app")));
    }

    #[test]
    fn browser_files() {
        use std::path::Path;
        assert!(super::browser_file(Path::new("site/index.HTML")));
        assert!(super::browser_file(Path::new("logo.svg")));
        assert!(!super::browser_file(Path::new("README.md")));
        assert!(!super::browser_file(Path::new("html")));
    }
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn applescript_strings_are_escaped() {
        assert_eq!(applescript_string(r#"say "hi" \ ok"#), r#""say \"hi\" \\ ok""#);
    }

    #[test]
    fn only_web_and_mail_addresses_are_opened() {
        for u in ["https://x.com/a", "http://localhost:3000/", "HTTPS://X.COM", "mailto:me@x.com"] {
            assert!(openable_url(u), "{u}");
        }
        for u in ["javascript:alert(1)", "smb://server/share", "file:///etc/passwd", "x-apple.systempreferences:", "ssh://h", "http:/x", ""] {
            assert!(!openable_url(u), "{u}");
        }
    }

    #[test]
    fn dropped_paths_are_typed_as_the_terminals_here_type_them() {
        if cfg!(windows) {
            assert_eq!(dropped_path(r"C:\a b\shot.png"), r#""C:\a b\shot.png""#);
            assert_eq!(dropped_path(r"C:\x.png"), r"C:\x.png");
        } else {
            assert_eq!(dropped_path("/Users/me/Screenshot 1.png"), r"/Users/me/Screenshot\ 1.png");
            assert_eq!(dropped_path("/tmp/it's (1)&$x.txt"), r"/tmp/it\'s\ \(1\)\&\$x.txt");
            assert_eq!(dropped_path("/tmp/plain-name_1.rs"), "/tmp/plain-name_1.rs");
        }
    }

    #[test]
    fn paths_are_quoted_only_when_needed() {
        assert_eq!(shell_quote("src/main.rs"), "src/main.rs");
        assert_eq!(shell_quote("my file.rs"), "'my file.rs'");
        #[cfg(not(windows))]
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        #[cfg(windows)]
        {
            assert_eq!(shell_quote("it's"), "'it''s'");
            assert_eq!(shell_quote(r"C:\src\main.rs"), r"C:\src\main.rs");
        }
    }

    #[test]
    fn parses_file_uri_lists_with_spaces_and_unicode() {
        let input = "# comment\r\nfile:///Users/me/My%20File%20%281%29.png\nfile://localhost/tmp/caf%C3%A9.txt\nhttps://ignored.example/x\n";
        let got = parse_file_uri_list(input);
        assert_eq!(
            got,
            vec![
                std::path::PathBuf::from("/Users/me/My File (1).png"),
                std::path::PathBuf::from("/tmp/café.txt"),
            ]
        );
    }
}

