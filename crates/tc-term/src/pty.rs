//! Local PTY sessions: spawn a shell (or a program) on a pseudo-terminal, feed its output into a
//! `Feeder` from the tokio runtime, and write keyboard input back.
//!
//! The PTY is opened and its program started here as `alacritty_terminal::tty::new` does it, but
//! without the SIGCHLD socket that gives each terminal (see [`spawn_on_pty`]). The program's exit
//! is looked at with `waitid` when SIGCHLD arrives; letting go of a terminal sends its program
//! SIGHUP and waits for it, on a thread of its own.
//!
//! A PTY can also be adopted: when the app restarts in place for an update, the new app takes
//! over the terminals (and programs) the old one started, see [`LocalPty::adopt`].

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;
use rustix_openpty::rustix;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::sync::{mpsc, watch};

use crate::core::{Fed, Feeder, GridSize};

/// How a program on a PTY ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitInfo {
    Code(i32),
    Signal(i32),
    /// Reaped elsewhere; the status is unknown.
    Unknown,
}

impl ExitInfo {
    pub fn success(self) -> bool {
        self == ExitInfo::Code(0)
    }
}

pub struct SpawnOptions {
    /// Program and arguments; `None` runs the user's login shell.
    pub program: Option<(String, Vec<String>)>,
    pub cwd: PathBuf,
    /// Extra environment variables (on top of the app's own environment).
    pub env: HashMap<String, String>,
    pub size: GridSize,
    /// Cell size in physical pixels.
    pub cell_px: (u16, u16),
}

/// A program running on a local pseudo-terminal.
pub struct LocalPty {
    /// The terminal's master side: a PTY this app opened, or one an earlier run of it opened and
    /// handed over at an update. `None` once let go of.
    pty: Mutex<Option<File>>,
    master: RawFd,
    pid: u32,
    exit: Mutex<Option<ExitInfo>>,
}

/// The environment every PTY gets unless the caller overrides it.
fn base_env(env: &mut HashMap<String, String>) {
    let mut set = |k: &str, v: &str| {
        env.entry(k.to_string()).or_insert_with(|| v.to_string());
    };
    set("TERM", "xterm-256color");
    set("COLORTERM", "truecolor");
    set("TERM_PROGRAM", "TernaryCode");
    // Apps opened from the Finder or the Dock get no locale; shells then run in the C locale and
    // mangle UTF-8.
    if std::env::var_os("LANG").is_none() && std::env::var_os("LC_ALL").is_none() {
        set("LANG", "en_US.UTF-8");
    }
}

/// Opens a PTY and starts the program on it as `alacritty_terminal::tty::new` does (the same
/// shell, variables and setup), less the SIGCHLD handler that registers for each terminal: it
/// writes a byte to a socket only alacritty's event loop reads, not used here. Unread, the socket
/// was full after 8192 SIGCHLDs (every git or language server that ends sends one), and macOS
/// then blocks that write in the handler in spite of `MSG_DONTWAIT`: each thread the next
/// SIGCHLDs went to hung, and the app with them.
fn spawn_on_pty(opts: &SpawnOptions, env: &HashMap<String, String>) -> io::Result<(File, u32)> {
    let os = |e: rustix::io::Errno| io::Error::from_raw_os_error(e.raw_os_error());
    let win = rustix::termios::Winsize {
        ws_row: opts.size.rows as u16,
        ws_col: opts.size.cols as u16,
        ws_xpixel: (opts.size.cols as u16).saturating_mul(opts.cell_px.0.max(1)),
        ws_ypixel: (opts.size.rows as u16).saturating_mul(opts.cell_px.1.max(1)),
    };
    let pty = rustix_openpty::openpty(None, Some(&win)).map_err(os)?;
    let (master, slave) = (pty.controller, pty.user);
    let (master_fd, slave_fd) = (master.as_raw_fd(), slave.as_raw_fd());
    // Input in UTF-8 (a backspace takes back a whole character).
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Ok(mut termios) = rustix::termios::tcgetattr(&master) {
        termios.input_modes.set(rustix::termios::InputModes::IUTF8, true);
        let _ = rustix::termios::tcsetattr(&master, rustix::termios::OptionalActions::Now, &termios);
    }
    let user = ShellUser::from_env()?;
    let mut cmd = match &opts.program {
        Some((program, args)) => {
            let mut cmd = Command::new(program);
            cmd.args(args);
            cmd
        }
        None => user.login_shell(),
    };
    cmd.stdin(slave.try_clone()?);
    cmd.stderr(slave.try_clone()?);
    cmd.stdout(slave);
    // As alacritty's `tty` set them, so programs see what they saw before.
    cmd.env("ALACRITTY_WINDOW_ID", "0");
    cmd.env("USER", &user.name);
    cmd.env("HOME", &user.home);
    cmd.env("WINDOWID", "0");
    cmd.envs(env);
    // Linux startup notification, for the app's own window only.
    cmd.env_remove("XDG_ACTIVATION_TOKEN");
    cmd.env_remove("DESKTOP_STARTUP_ID");
    let cwd = CString::new(opts.cwd.as_os_str().as_bytes()).ok();
    // SAFETY: between fork and exec, only async-signal-safe calls on values made before the fork.
    unsafe {
        cmd.pre_exec(move || {
            // A session of its own, with this terminal as its controlling one.
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            // A folder that is gone leaves the program where the app is.
            if let Some(cwd) = &cwd {
                libc::chdir(cwd.as_ptr());
            }
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            if slave_fd > 2 {
                libc::close(slave_fd);
            }
            if master_fd > 2 {
                libc::close(master_fd);
            }
            for sig in [libc::SIGCHLD, libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM, libc::SIGALRM] {
                libc::signal(sig, libc::SIG_DFL);
            }
            Ok(())
        });
    }
    // SAFETY: fcntl on the fd just opened; the reader needs it non-blocking.
    unsafe {
        let flags = libc::fcntl(master_fd, libc::F_GETFL);
        if flags == -1 || libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    let child = cmd.spawn().map_err(|e| {
        io::Error::new(e.kind(), format!("Failed to spawn command '{}': {e}", cmd.get_program().to_string_lossy()))
    })?;
    // The terminal's other side is the program's alone now: its end is how the reader learns
    // the program and everything it started are gone.
    drop(cmd);
    Ok((File::from(master), child.id()))
}

/// Who the program runs as: `$USER`, `$HOME` and `$SHELL`, else this user's passwd entry.
struct ShellUser {
    name: String,
    home: String,
    shell: String,
}

impl ShellUser {
    fn from_env() -> io::Result<Self> {
        let mut entry: Option<io::Result<[String; 3]>> = None;
        let mut get = |var: &str, field: usize| match std::env::var(var) {
            Ok(value) => Ok(value),
            Err(_) => match entry.get_or_insert_with(passwd_entry) {
                Ok(e) => Ok(e[field].clone()),
                Err(e) => Err(io::Error::new(e.kind(), e.to_string())),
            },
        };
        Ok(Self { name: get("USER", 0)?, home: get("HOME", 1)?, shell: get("SHELL", 2)? })
    }

    /// The user's shell as a login shell. On macOS through `login`, as Terminal does, so it is a
    /// terminal session of its own.
    #[cfg(target_os = "macos")]
    fn login_shell(&self) -> Command {
        let name = self.shell.rsplit('/').next().unwrap_or(&self.shell);
        let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        // `exec -a` puts the '-' of a login shell before its name (zsh's builtin, hence zsh).
        let exec = format!("exec -a {} {}", q(&format!("-{name}")), q(&self.shell));
        // -f: no password for the user already logged in; -l: no cd to $HOME (and no '-'); -p:
        // the environment kept; -q: quiet, as a ~/.hushlogin asks (login looks for one only in
        // the folder it starts in).
        let hush = std::path::Path::new(&self.home).join(".hushlogin").exists();
        let mut cmd = Command::new("/usr/bin/login");
        cmd.args([if hush { "-qflp" } else { "-flp" }, self.name.as_str(), "/bin/zsh", "-fc", exec.as_str()]);
        cmd
    }

    #[cfg(not(target_os = "macos"))]
    fn login_shell(&self) -> Command {
        Command::new(&self.shell)
    }
}

/// This user's name, home folder and shell from the user database.
fn passwd_entry() -> io::Result<[String; 3]> {
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: an all-zero passwd is a valid value for getpwuid_r to fill in; its strings point
    // into `buf`, which outlives their use here.
    unsafe {
        let mut pw: libc::passwd = std::mem::zeroed();
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        let r = libc::getpwuid_r(libc::getuid(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut found);
        if r != 0 || found.is_null() {
            return Err(io::Error::other("this user is not in the user database"));
        }
        let text = |p: *const libc::c_char| {
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
        };
        Ok([text(pw.pw_name), text(pw.pw_dir), text(pw.pw_shell)])
    }
}

impl LocalPty {
    pub fn spawn(opts: &SpawnOptions) -> io::Result<Arc<Self>> {
        let mut env = opts.env.clone();
        base_env(&mut env);
        let (file, pid) = spawn_on_pty(opts, &env)?;
        let master = file.as_raw_fd();
        Ok(Arc::new(Self {
            pty: Mutex::new(Some(file)),
            master,
            pid,
            exit: Mutex::new(None),
        }))
    }

    /// Takes over a terminal the app's previous run started: `master` is its fd, kept open
    /// through `exec` (see [`LocalPty::prepare_handoff`]), and `pid` its program, still this
    /// process's child (exec keeps the pid). Fails when either is not what it should be.
    pub fn adopt(master: RawFd, pid: u32) -> io::Result<Arc<Self>> {
        // SAFETY: fcntl and isatty on an fd number; nothing is assumed about it until checked.
        unsafe {
            if libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC) == -1 || libc::isatty(master) == 0 {
                return Err(io::Error::last_os_error());
            }
            let flags = libc::fcntl(master, libc::F_GETFL);
            if flags != -1 && flags & libc::O_NONBLOCK == 0 {
                libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
        // SAFETY: the fd is open and was handed over to this run alone.
        let file = unsafe { File::from_raw_fd(master) };
        let me = Arc::new(Self {
            pty: Mutex::new(Some(file)),
            master,
            pid,
            exit: Mutex::new(None),
        });
        // Not a child of this process: the program is not the one handed over. The fd is
        // closed, the pid left alone.
        if me.poll_exit() == Some(ExitInfo::Unknown) {
            drop(me.pty.lock().take());
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        Ok(me)
    }

    /// Lets the terminal's fd live through the `exec` that restarts the app, and returns it
    /// with the program's pid, for [`LocalPty::adopt`] in the new run. `None` once the program
    /// has ended.
    pub fn prepare_handoff(&self) -> Option<(RawFd, u32)> {
        if self.poll_exit().is_some() || self.pty.lock().is_none() {
            return None;
        }
        // SAFETY: the fd stays open while `self.pty` holds it.
        let ok = unsafe { libc::fcntl(self.master, libc::F_SETFD, 0) } != -1;
        ok.then_some((self.master, self.pid))
    }

    /// Process id of the program started on the PTY (the shell).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    fn dup_master(&self) -> io::Result<File> {
        match self.pty.lock().as_ref() {
            Some(file) => file.try_clone(),
            None => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
        }
    }

    /// Tells the kernel the new size; the foreground program gets SIGWINCH.
    pub fn resize(&self, size: GridSize, cell_w: u16, cell_h: u16) {
        let win = libc::winsize {
            ws_row: size.rows as u16,
            ws_col: size.cols as u16,
            ws_xpixel: (size.cols as u16).saturating_mul(cell_w),
            ws_ypixel: (size.rows as u16).saturating_mul(cell_h),
        };
        // SAFETY: the fd stays open while `self.pty` holds it; a failure is harmless.
        unsafe {
            libc::ioctl(self.master, libc::TIOCSWINSZ, &win as *const libc::winsize);
        }
    }

    /// Process group in the foreground of the terminal (the shell's own when it waits at the
    /// prompt, a job's while one runs).
    pub fn foreground_pgid(&self) -> Option<i32> {
        if self.exit.lock().is_some() {
            return None;
        }
        // SAFETY: plain syscall on an fd we own.
        let pgid = unsafe { libc::tcgetpgrp(self.master) };
        (pgid > 0).then_some(pgid)
    }

    /// The program's exit status once it has ended. The child is looked at, not reaped
    /// (`WNOWAIT`): it stays a zombie until this `LocalPty` is dropped, which reaps it after
    /// sending SIGHUP. Reaping it here would free its pid while that SIGHUP is still to come, and a new
    /// process given the same pid would get it.
    pub fn poll_exit(&self) -> Option<ExitInfo> {
        let mut cached = self.exit.lock();
        if cached.is_some() {
            return *cached;
        }
        // SAFETY: an all-zero siginfo_t is valid; waitid on our own child.
        let mut si: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let r = unsafe {
            libc::waitid(
                libc::P_PID,
                self.pid as libc::id_t,
                &mut si,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        let info = if r == 0 {
            // SAFETY: filled in by waitid (all zero when the child is still running).
            let (pid, status) = unsafe { (si.si_pid(), si.si_status()) };
            match si.si_code {
                _ if pid == 0 => None,
                libc::CLD_EXITED => Some(ExitInfo::Code(status)),
                libc::CLD_KILLED | libc::CLD_DUMPED => Some(ExitInfo::Signal(status)),
                _ => None,
            }
        } else if io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD) {
            Some(ExitInfo::Unknown)
        } else {
            None
        };
        *cached = info;
        info
    }

    pub fn is_alive(&self) -> bool {
        self.poll_exit().is_none()
    }

    /// Sends a signal to the program's process group and to the foreground job's.
    pub fn signal(&self, sig: i32) {
        if self.poll_exit().is_some() {
            return;
        }
        let fg = self.foreground_pgid();
        // SAFETY: plain kill(2) calls on our own child's groups.
        unsafe {
            libc::kill(-(self.pid as i32), sig);
            if let Some(fg) = fg
                && fg != self.pid as i32
            {
                libc::kill(-fg, sig);
            }
        }
    }

    /// Ends the program the way closing a terminal window does: SIGHUP, then SIGKILL if it is
    /// still there after `grace`. Runs on a background thread.
    pub fn hang_up(self: &Arc<Self>, grace: Duration) {
        self.signal(libc::SIGHUP);
        let me = self.clone();
        std::thread::spawn(move || {
            let step = Duration::from_millis(20);
            let mut waited = Duration::ZERO;
            while waited < grace {
                if me.poll_exit().is_some() {
                    return;
                }
                std::thread::sleep(step);
                waited += step;
            }
            me.signal(libc::SIGKILL);
        });
    }
}

/// Ends these programs as the app quits: SIGHUP to each, then SIGKILL to those still there
/// after `grace`, waited for here. `hang_up`'s thread would die with the app, and a shell still
/// starting ignores SIGHUP; once the app has closed its terminal it then waits for it forever,
/// holding one of the system's few hundred terminals. Returns at once when all have ended.
pub fn hang_up_all(ptys: &[Arc<LocalPty>], grace: Duration) {
    for p in ptys {
        p.signal(libc::SIGHUP);
    }
    let start = std::time::Instant::now();
    while start.elapsed() < grace && ptys.iter().any(|p| p.poll_exit().is_none()) {
        std::thread::sleep(Duration::from_millis(10));
    }
    for p in ptys {
        p.signal(libc::SIGKILL);
    }
}

impl Drop for LocalPty {
    fn drop(&mut self) {
        // SIGHUP the shell and its foreground job, close the master fd right away so the slave
        // side sees EOF/HUP without waiting 2 seconds, then reap (escalating to SIGKILL if it
        // ignores SIGHUP). Never blocks the UI thread.
        if let Some(file) = self.pty.lock().take() {
            let pid = self.pid as libc::pid_t;
            // SAFETY: tcgetpgrp on our still-open master fd.
            let fg = unsafe {
                let pgrp = libc::tcgetpgrp(file.as_raw_fd());
                (pgrp > 0 && pgrp != pid).then_some(pgrp)
            };
            drop(file);
            std::thread::spawn(move || {
                // SAFETY: kill and waitpid on this process's own child and session.
                unsafe {
                    libc::kill(-pid, libc::SIGHUP);
                    libc::kill(pid, libc::SIGHUP);
                    if let Some(fg) = fg {
                        libc::kill(-fg, libc::SIGHUP);
                    }
                    let mut status = 0;
                    let start = std::time::Instant::now();
                    let reaped = loop {
                        let r = libc::waitpid(pid, &mut status, libc::WNOHANG);
                        if r == pid || (r == -1 && io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)) {
                            break true;
                        }
                        if start.elapsed() >= Duration::from_secs(2) {
                            break false;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    };
                    if !reaped {
                        libc::kill(-pid, libc::SIGKILL);
                        libc::kill(pid, libc::SIGKILL);
                        if let Some(fg) = fg
                            && libc::getsid(fg) == pid
                        {
                            libc::kill(-fg, libc::SIGKILL);
                        }
                        while libc::waitpid(pid, &mut status, 0) == -1 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {}
                    }
                }
            });
        }
    }
}

/// Wakes every session when a child process changes state. A `watch` channel, so a SIGCHLD that
/// arrives while a session is busy is still seen on its next turn.
fn child_signals() -> watch::Receiver<u64> {
    static TX: OnceLock<watch::Sender<u64>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, _) = watch::channel(0u64);
        let sender = tx.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<()>(1);
        // On a thread of its own with its own runtime: a task on the caller's runtime would end
        // with that runtime (each test builds one), and later sessions would get no SIGCHLD.
        let _ = std::thread::Builder::new().name("sigchld".into()).spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_io().build() else {
                return;
            };
            rt.block_on(async move {
                let Ok(mut sig) =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::child())
                else {
                    return;
                };
                let _ = ready_tx.send(());
                while sig.recv().await.is_some() {
                    sender.send_modify(|n| *n = n.wrapping_add(1));
                }
            });
        });
        let _ = ready_rx.recv_timeout(Duration::from_secs(2));
        tx
    });
    tx.subscribe()
}

/// What a running session reports.
#[derive(Debug)]
pub enum SessionEvent {
    /// Output was parsed into the terminal.
    Output(Fed),
    /// The terminal's foreground process group changed (a program started or ended). Checked
    /// here, as output arrives (at most every [`FG_INTERVAL`], and once more after the last
    /// output), so the app is not woken just to find nothing changed.
    Foreground,
    /// The program ended and its output is drained.
    Exited(ExitInfo),
}

/// The running I/O of one PTY: keyboard input goes in through `write`.
#[derive(Clone)]
pub struct Session {
    pub pty: Arc<LocalPty>,
    input: mpsc::UnboundedSender<Vec<u8>>,
    stop: Arc<Stop>,
}

/// Asks the reader to stop, and tells when it has.
#[derive(Default)]
struct Stop {
    asked: tokio::sync::Notify,
    done: std::sync::Mutex<bool>,
    ended: std::sync::Condvar,
}

impl Session {
    pub fn write(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            let _ = self.input.send(bytes);
        }
    }

    /// Stops reading the terminal, leaving its program running and what it prints next in the
    /// terminal for whoever reads it after (the app restarting in place). Waits up to `wait`
    /// for the reader to finish what it was doing; true once it has. No `Exited` event follows.
    pub fn stop_reading(&self, wait: Duration) -> bool {
        self.stop.asked.notify_one();
        let done = self.stop.done.lock().unwrap_or_else(|e| e.into_inner());
        let (done, _) = self
            .stop
            .ended
            .wait_timeout_while(done, wait, |d| !*d)
            .unwrap_or_else(|e| e.into_inner());
        *done
    }
}

/// Bytes read per `read` call.
const READ_BUF: usize = 64 * 1024;
/// Least time between two foreground checks of one terminal.
pub const FG_INTERVAL: Duration = Duration::from_millis(400);

/// Starts reading `pty` into `feeder` and writing input to it, on `rt`. `on_event` runs on a
/// runtime thread after each read and once at the end.
pub fn start(
    rt: &tokio::runtime::Handle,
    pty: Arc<LocalPty>,
    feeder: Arc<Mutex<Feeder>>,
    on_event: Arc<dyn Fn(SessionEvent) + Send + Sync>,
) -> io::Result<Session> {
    let read_file = pty.dup_master()?;
    let write_file = pty.dup_master()?;
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let stop = Arc::new(Stop::default());
    let session = Session {
        pty: pty.clone(),
        input: tx.clone(),
        stop: stop.clone(),
    };

    // Writer.
    rt.spawn(async move {
        // SAFETY: the `File` owns its (duplicated) fd for as long as the AsyncFd lives.
        let Ok(writer) = (unsafe { AsyncFd::register_with_interest(write_file, Interest::WRITABLE) })
        else {
            return;
        };
        while let Some(data) = rx.recv().await {
            let mut off = 0;
            while off < data.len() {
                let Ok(mut guard) = writer.writable().await else {
                    return;
                };
                match guard.try_io(|inner| {
                    let mut f: &File = inner.get_ref();
                    f.write(&data[off..])
                }) {
                    Ok(Ok(n)) => off += n,
                    Ok(Err(e)) if e.kind() == io::ErrorKind::Interrupted => {}
                    Ok(Err(_)) => return,
                    Err(_would_block) => {}
                }
            }
        }
    });

    // Reader.
    rt.spawn(async move {
        struct StopGuard(Arc<Stop>);
        impl Drop for StopGuard {
            fn drop(&mut self) {
                *self.0.done.lock().unwrap_or_else(|e| e.into_inner()) = true;
                self.0.ended.notify_all();
            }
        }
        // Always mark `stop.done` on any exit path so `stop_reading` never hangs after the
        // reader task has already finished.
        let _stop_guard = StopGuard(stop.clone());
        let mut children = child_signals();
        // SAFETY: as above.
        let reader = match unsafe { AsyncFd::register_with_interest(read_file, Interest::READABLE) } {
            Ok(r) => r,
            Err(_) => {
                on_event(SessionEvent::Exited(
                    pty.poll_exit().unwrap_or(ExitInfo::Unknown),
                ));
                return;
            }
        };
        let mut buf = vec![0u8; READ_BUF];
        let deliver = |fed: Fed| {
            if !fed.reply.is_empty() {
                let _ = tx.send(fed.reply.clone());
            }
            on_event(SessionEvent::Output(fed));
        };
        // Foreground checks: output is when it changes. Throttled, with one more check after
        // the last output (a program can end, or start, and print nothing more for a while).
        let mut fg_seen = pty.foreground_pgid();
        let mut fg_checked = tokio::time::Instant::now();
        let mut fg_due: Option<tokio::time::Instant> = None;
        let check_fg = |seen: &mut Option<i32>, checked: &mut tokio::time::Instant| {
            *checked = tokio::time::Instant::now();
            let now = pty.foreground_pgid();
            if now != *seen {
                *seen = now;
                on_event(SessionEvent::Foreground);
            }
        };
        // Check once after subscribing to `child_signals()` in case the child exited before
        // subscription (fast-exiting commands or already-exited adopted children).
        if let Some(status) = pty.poll_exit() {
            loop {
                match reader.get_ref().read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let fed = feeder.lock().feed(&buf[..n]);
                        deliver(fed);
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    _ => break,
                }
            }
            on_event(SessionEvent::Exited(status));
            return;
        }
        loop {
            let deadline = feeder.lock().sync_deadline();
            let sync = async {
                match deadline {
                    Some(d) => tokio::time::sleep_until(tokio::time::Instant::from_std(d)).await,
                    None => std::future::pending::<()>().await,
                }
            };
            let fg_timer = async {
                match fg_due {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                // Whatever is read before it stops is in the terminal, which is drawn for the
                // next run only once it has stopped.
                _ = stop.asked.notified() => {
                    return;
                }
                ready = reader.readable() => {
                    let Ok(mut guard) = ready else { break };
                    // All the terminal holds now (a macOS read returns about 1 KB), parsed at
                    // once: one lock, one pass of the scanners and one event for the lot.
                    let mut n = 0;
                    let mut closed = false;
                    while n < buf.len() {
                        match guard.try_io(|inner| {
                            let mut f: &File = inner.get_ref();
                            f.read(&mut buf[n..])
                        }) {
                            Ok(Ok(0)) => {
                                closed = true;
                                break;
                            }
                            Ok(Ok(k)) => n += k,
                            Ok(Err(e)) if e.kind() == io::ErrorKind::Interrupted => {}
                            // EIO: the program and everything holding the terminal are gone.
                            Ok(Err(_)) => {
                                closed = true;
                                break;
                            }
                            Err(_would_block) => break,
                        }
                    }
                    if n > 0 {
                        let fed = feeder.lock().feed(&buf[..n]);
                        deliver(fed);
                        if fg_due.is_none() {
                            let at = fg_checked + FG_INTERVAL;
                            if at <= tokio::time::Instant::now() {
                                check_fg(&mut fg_seen, &mut fg_checked);
                            }
                            // The check after the last output.
                            fg_due = Some((fg_checked + FG_INTERVAL).max(at));
                        }
                    }
                    if closed {
                        break;
                    }
                }
                _ = sync => {
                    let reply = feeder.lock().flush_sync();
                    deliver(Fed { reply, repaint: true, ..Default::default() });
                }
                _ = fg_timer => {
                    fg_due = None;
                    check_fg(&mut fg_seen, &mut fg_checked);
                }
                changed = children.changed() => {
                    if changed.is_err() {
                        continue;
                    }
                    if pty.poll_exit().is_some() {
                        // Take what is still buffered, then stop (a background job may keep the
                        // terminal open after the shell left).
                        loop {
                            match reader.get_ref().read(&mut buf) {
                                Ok(n) if n > 0 => {
                                    let fed = feeder.lock().feed(&buf[..n]);
                                    deliver(fed);
                                }
                                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                                _ => break,
                            }
                        }
                        break;
                    }
                }
            }
        }
        // The terminal closed; the status usually follows within a moment.
        let mut status = pty.poll_exit();
        for _ in 0..100 {
            if status.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            status = pty.poll_exit();
        }
        on_event(SessionEvent::Exited(status.unwrap_or(ExitInfo::Unknown)));
    });
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{new_terminal, term_config};
    use crate::osc::OscSignal;
    use crate::scheme::SCHEMES;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line};
    use std::sync::mpsc as std_mpsc;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    fn screen(feeder: &Arc<Mutex<Feeder>>) -> String {
        let f = feeder.lock();
        let term = f.shared.term.lock();
        (0..term.screen_lines() as i32)
            .map(|l| {
                let row = &term.grid()[Line(l)];
                (0..term.columns())
                    .map(|c| row[Column(c)].c)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    struct Run {
        feeder: Arc<Mutex<Feeder>>,
        events: std_mpsc::Receiver<SessionEvent>,
        session: Session,
    }

    fn run(rt: &tokio::runtime::Runtime, script: &str, size: GridSize) -> Run {
        let (shared, rx) = new_terminal(
            term_config(1000, Default::default(), false),
            size,
            (&SCHEMES[0]).into(),
        );
        let feeder = Arc::new(Mutex::new(Feeder::new(shared, rx)));
        let pty = LocalPty::spawn(&SpawnOptions {
            program: Some(("/bin/sh".into(), vec!["-c".into(), script.into()])),
            cwd: std::env::temp_dir(),
            env: HashMap::new(),
            size,
            cell_px: (8, 16),
        })
        .unwrap();
        let (etx, erx) = std_mpsc::channel();
        let etx = Mutex::new(etx);
        let session = start(
            rt.handle(),
            pty,
            feeder.clone(),
            Arc::new(move |e| {
                let _ = etx.lock().send(e);
            }),
        )
        .unwrap();
        Run {
            feeder,
            events: erx,
            session,
        }
    }

    fn wait_exit(run: &Run) -> (ExitInfo, Vec<OscSignal>) {
        let mut signals = Vec::new();
        loop {
            match run.events.recv_timeout(Duration::from_secs(10)).expect("event") {
                SessionEvent::Output(fed) => signals.extend(fed.signals),
                SessionEvent::Foreground => {}
                SessionEvent::Exited(info) => return (info, signals),
            }
        }
    }

    #[test]
    fn runs_a_program_and_reports_output_signals_and_exit() {
        let rt = rt();
        let run = run(
            &rt,
            r"printf 'hello\033]9;agent done\007\n'; exit 3",
            GridSize { cols: 40, rows: 5 },
        );
        let (info, signals) = wait_exit(&run);
        assert_eq!(info, ExitInfo::Code(3));
        assert!(screen(&run.feeder).contains("hello"), "{}", screen(&run.feeder));
        assert_eq!(
            signals,
            vec![OscSignal::Notify {
                title: None,
                body: "agent done".into()
            }]
        );
    }

    #[test]
    fn input_reaches_the_program_and_resize_is_seen() {
        let rt = rt();
        let run = run(
            &rt,
            "read line; stty size; echo \"got:$line\"",
            GridSize { cols: 40, rows: 5 },
        );
        run.session
            .pty
            .resize(GridSize { cols: 100, rows: 30 }, 8, 16);
        run.session.write(b"ping\r".to_vec());
        let (info, _) = wait_exit(&run);
        assert!(info.success());
        let text = screen(&run.feeder);
        assert!(text.contains("30 100"), "{text}");
        assert!(text.contains("got:ping"), "{text}");
    }

    #[test]
    fn foreground_group_and_hang_up() {
        let rt = rt();
        let run = run(&rt, "sleep 30", GridSize { cols: 40, rows: 5 });
        let pty = run.session.pty.clone();
        std::thread::sleep(Duration::from_millis(200));
        // `sh -c` without job control: the shell's own group is in the foreground.
        assert_eq!(pty.foreground_pgid(), Some(pty.pid() as i32));
        assert!(pty.is_alive());
        pty.hang_up(Duration::from_millis(500));
        let (info, _) = wait_exit(&run);
        assert!(!info.success(), "{info:?}");
    }

    #[test]
    fn an_ended_program_keeps_its_pid_until_the_pane_lets_go() {
        let rt = rt();
        let run = run(&rt, "kill -TERM $$", GridSize { cols: 40, rows: 5 });
        let (info, _) = wait_exit(&run);
        assert_eq!(info, ExitInfo::Signal(libc::SIGTERM));
        let pid = run.session.pty.pid() as i32;
        // Still a zombie: the pid cannot go to another process while the pane may signal it.
        // SAFETY: signal 0 only checks that the process exists.
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
        assert_eq!(run.session.pty.poll_exit(), Some(ExitInfo::Signal(libc::SIGTERM)));
        drop(run);
        // Reaped once the pane let go of it (on a background thread).
        let gone = (0..250).any(|_| {
            std::thread::sleep(Duration::from_millis(20));
            (unsafe { libc::kill(pid, 0) }) == -1
        });
        assert!(gone, "pid {pid} was not reaped");
    }

    /// SIGCHLD still reaches a session on a runtime started after another one ended (each test
    /// builds its own): a background job holding the terminal open (Linux keeps it open after
    /// the shell left; macOS revokes it) does not delay the exit.
    #[test]
    fn the_exit_is_seen_after_another_runtime_ended() {
        let first = rt();
        let ended = run(&first, "exit 0", GridSize { cols: 40, rows: 5 });
        wait_exit(&ended);
        drop(ended);
        drop(first);
        let rt = rt();
        let start = std::time::Instant::now();
        // The shell waits a moment, so the session listens before it exits.
        let run = run(&rt, "trap '' HUP; sleep 3 & sleep 0.3; exit 4", GridSize { cols: 40, rows: 5 });
        assert_eq!(wait_exit(&run).0, ExitInfo::Code(4));
        assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
    }

    /// SIGCHLD keeps coming for as long as the app runs (each git or language server that ends):
    /// an open terminal must not fill up with it. alacritty's `tty::new` gave each terminal a
    /// socket the signal handler wrote a byte to and nothing read; on macOS the 8193rd SIGCHLD
    /// then blocked in the handler for good, and so did each thread the next ones went to.
    #[test]
    fn child_signals_do_not_pile_up_in_an_open_terminal() {
        let rt = rt();
        let run = run(&rt, "sleep 30", GridSize { cols: 40, rows: 5 });
        let (done_tx, done_rx) = std_mpsc::channel();
        std::thread::spawn(move || {
            for _ in 0..20_000 {
                // SAFETY: raise runs this process's SIGCHLD handlers on this thread; they only
                // wake whoever waits for a child.
                unsafe { libc::raise(libc::SIGCHLD) };
            }
            let _ = done_tx.send(());
        });
        assert!(done_rx.recv_timeout(Duration::from_secs(10)).is_ok(), "a SIGCHLD handler blocked");
        run.session.pty.hang_up(Duration::from_millis(500));
        wait_exit(&run);
    }

    /// A terminal handed over (as at an update) keeps its program: the new reader gets its
    /// output and the program its input. A pid that is not this process's child is refused.
    #[test]
    fn an_adopted_terminal_keeps_its_program() {
        let rt = rt();
        let run = run(&rt, "read line; echo \"got:$line\"", GridSize { cols: 40, rows: 5 });
        std::thread::sleep(Duration::from_millis(100));
        assert!(run.session.stop_reading(Duration::from_secs(2)));
        let (master, pid) = run.session.pty.prepare_handoff().unwrap();
        // SAFETY: a second fd for the same terminal, for the adopted side to own.
        let fd = unsafe { libc::dup(master) };
        assert!(LocalPty::adopt(unsafe { libc::dup(master) }, 1).is_err());
        let pty = LocalPty::adopt(fd, pid).unwrap();
        assert!(pty.is_alive());
        let (shared, rx) = new_terminal(term_config(1000, Default::default(), false), GridSize { cols: 40, rows: 5 }, (&SCHEMES[0]).into());
        let feeder = Arc::new(Mutex::new(Feeder::new(shared, rx)));
        let (etx, erx) = std_mpsc::channel();
        let etx = Mutex::new(etx);
        let session = start(rt.handle(), pty, feeder.clone(), Arc::new(move |e| {
            let _ = etx.lock().send(e);
        }))
        .unwrap();
        session.write(b"ping\r".to_vec());
        let adopted = Run { feeder, events: erx, session };
        assert_eq!(wait_exit(&adopted).0, ExitInfo::Code(0));
        assert!(screen(&adopted.feeder).contains("got:ping"), "{}", screen(&adopted.feeder));
        // The first side would reap the child too.
        std::mem::forget(run);
    }

    /// Throughput through a real PTY: `cat` of a 100 MB file (release build:
    /// `cargo test -p tc-term --release -- --ignored --nocapture pty_throughput`).
    #[test]
    #[ignore]
    fn pty_throughput() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        let mut f = std::io::BufWriter::new(File::create(&path).unwrap());
        let line = b"The quick brown fox jumps over the lazy dog 0123456789 \x1b[32mgreen\x1b[0m\n";
        let mut written = 0;
        while written < 100 * 1024 * 1024 {
            f.write_all(line).unwrap();
            written += line.len();
        }
        drop(f);
        let rt = rt();
        let start = std::time::Instant::now();
        let run = run(
            &rt,
            &format!("cat '{}'", path.display()),
            GridSize { cols: 120, rows: 40 },
        );
        let mut bytes_events = 0usize;
        loop {
            match run.events.recv_timeout(Duration::from_secs(60)).unwrap() {
                SessionEvent::Output(_) => bytes_events += 1,
                // Foreground checks come along too: not the end.
                SessionEvent::Foreground => {}
                SessionEvent::Exited(_) => break,
            }
        }
        let secs = start.elapsed().as_secs_f64();
        println!(
            "PTY throughput: {} MB in {secs:.2} s = {:.0} MB/s ({bytes_events} reads)",
            written / (1024 * 1024),
            written as f64 / (1024.0 * 1024.0) / secs
        );
    }
}
