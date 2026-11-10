//! The `ternarycode` command in terminals: `ternarycode .`, `ternarycode ~/code/app` or
//! `ternarycode my-app` opens that project in the running app, starting it when needed.
//!
//! Where the command comes from:
//! - TernaryCode's own panes: always (the app's folder is on their `PATH`).
//! - macOS: a link to the app's program in a folder of the user's `PATH` they can write
//!   (Homebrew's, `/usr/local/bin`, `~/.local/bin`, `~/bin`), made at launch; else Settings →
//!   General installs it in `/usr/local/bin` with an administrator password.
//! - Linux: the `.deb`/`.rpm` put it in `/usr/bin`, the `.tar.gz` in `~/.local/bin`; an AppImage
//!   links itself into `~/.local/bin` at launch.
//! - Windows: the setup adds the app's folder to `PATH`.
//!
//! Started from a terminal while the app is not running, the app is launched apart from the
//! terminal (closing the terminal must not quit it).

#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Stdio;
use std::sync::Mutex;

/// Where the command was found or installed (none: not on the user's PATH yet).
static INSTALLED: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn installed() -> Option<PathBuf> {
    INSTALLED.lock().ok()?.clone()
}

/// The program the command runs: the installed app's on macOS, the AppImage on Linux. None for
/// development builds and copies that can't be linked to (a disk image, a package's).
#[cfg(target_os = "macos")]
fn program() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return None;
    }
    crate::update::installed_bundle().map(|b| b.join("Contents/MacOS/ternarycode"))
}

#[cfg(target_os = "linux")]
fn program() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|p| p.is_file())
}

#[cfg(windows)]
fn program() -> Option<PathBuf> {
    None
}

/// The folders the command may be linked into, best first.
#[cfg(unix)]
fn candidates() -> Vec<PathBuf> {
    let home = crate::paths::home_dir();
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        out.push(PathBuf::from("/opt/homebrew/bin"));
        out.push(PathBuf::from("/usr/local/bin"));
    }
    out.push(home.join(".local/bin"));
    out.push(home.join("bin"));
    out
}

#[cfg(unix)]
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".ternarycode-write-test-{}", std::process::id()));
    let ok = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// A `ternarycode` there that is ours to replace: a link to some copy of TernaryCode.
#[cfg(unix)]
fn ours(link: &Path) -> bool {
    std::fs::read_link(link).is_ok_and(|to| {
        let to = to.to_string_lossy();
        to.contains("TernaryCode") || to.contains("ternarycode")
    })
}

/// Makes sure the command is on the user's PATH (at launch, off the UI thread). Nothing is
/// replaced that is not a link to TernaryCode.
pub fn ensure_installed() {
    let path_dirs: Vec<PathBuf> = {
        let mut dirs = Vec::new();
        if let Some(p) = tc_lsp::servers::login_shell_path() {
            dirs.extend(std::env::split_paths(&p));
        }
        if let Some(p) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&p));
        }
        dirs
    };
    let exe = if cfg!(windows) { "ternarycode.exe" } else { "ternarycode" };
    let me = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok());
    let program = program();
    // Already there, pointing at this copy (or, without a copy to link, any TernaryCode).
    let found = path_dirs.iter().map(|d| d.join(exe)).find(|c| {
        let real = c.canonicalize().ok();
        real.is_some() && (real == program.as_ref().and_then(|p| p.canonicalize().ok()) || real == me || program.is_none())
    });
    if let Some(f) = found {
        set_installed(Some(f));
        return;
    }
    #[cfg(unix)]
    if let Some(program) = program {
        for dir in candidates() {
            if !dir.is_dir() || !path_dirs.contains(&dir) || !writable(&dir) {
                continue;
            }
            let link = dir.join(exe);
            if (link.exists() || link.symlink_metadata().is_ok()) && !ours(&link) {
                continue;
            }
            let tmp = dir.join(format!(".ternarycode-link-{}", std::process::id()));
            let _ = std::fs::remove_file(&tmp);
            if std::os::unix::fs::symlink(&program, &tmp).is_ok() && std::fs::rename(&tmp, &link).is_ok() {
                set_installed(Some(link));
                return;
            }
            let _ = std::fs::remove_file(&tmp);
        }
    }
    set_installed(None);
}

fn set_installed(p: Option<PathBuf>) {
    if let Ok(mut g) = INSTALLED.lock() {
        *g = p;
    }
}

/// macOS: links the command into `/usr/local/bin`, asking for an administrator password.
#[cfg(target_os = "macos")]
pub fn install_with_admin() -> Result<PathBuf, String> {
    let program = program().ok_or_else(|| {
        crate::i18n::t("Move TernaryCode to the Applications folder first.").to_string()
    })?;
    const SCRIPT: &str = r#"on run argv
    set cmd to "/bin/mkdir -p /usr/local/bin && /bin/ln -sf " & quoted form of (item 2 of argv) & " /usr/local/bin/ternarycode"
    do shell script cmd with prompt (item 1 of argv) with administrator privileges
end run"#;
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", SCRIPT, crate::i18n::t("TernaryCode wants to install the ternarycode command.")])
        .arg(&program)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(if err.contains("-128") { crate::i18n::t("Cancelled.").to_string() } else { err.trim().to_string() });
    }
    let link = PathBuf::from("/usr/local/bin/ternarycode");
    set_installed(Some(link.clone()));
    Ok(link)
}

/// The app's own folder on a pane's `PATH`, so `ternarycode` works in every pane.
pub fn pane_path() -> Option<std::ffi::OsString> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if !dirs.contains(&dir) {
        dirs.push(dir);
    }
    std::env::join_paths(dirs).ok()
}

/// Started from a terminal, with no TernaryCode running: starts the app apart from the
/// terminal with the same targets and returns true (this process then exits). macOS goes
/// through Launch Services for an app bundle, so it opens as if from the Finder. `detached`:
/// this process is such a copy (`TERNARYCODE_DETACHED` was set; read before the environment
/// is cleaned for the panes).
pub fn launch_detached(targets: &[String], detached: bool) -> bool {
    #[cfg(unix)]
    {
        use std::io::IsTerminal;
        use std::os::unix::process::CommandExt;
        if detached || !(std::io::stdin().is_terminal() || std::io::stdout().is_terminal())
        {
            return false;
        }
        let Some(exe) = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()) else {
            return false;
        };
        #[cfg(target_os = "macos")]
        if let Some(bundle) = exe.parent().and_then(Path::parent).and_then(Path::parent).filter(|b| b.extension().is_some_and(|e| e == "app")) {
            return std::process::Command::new("/usr/bin/open")
                .arg("-a")
                .arg(bundle)
                .arg("--args")
                .args(targets)
                .stdin(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
        }
        // An AppImage is started again as itself, not the program inside its mount.
        let program = std::env::var_os("APPIMAGE").map(PathBuf::from).unwrap_or(exe);
        let mut cmd = std::process::Command::new(program);
        cmd.args(targets)
            .env("TERNARYCODE_DETACHED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: setsid only, between fork and exec: a session of its own, without the
        // terminal, so closing the terminal does not hang it up.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.spawn().is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = (targets, detached);
        false
    }
}
