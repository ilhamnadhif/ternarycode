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
}

