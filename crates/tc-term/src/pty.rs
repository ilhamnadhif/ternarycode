//! Local PTY sessions: spawn a shell (or a program) on a pseudo-terminal, feed its output into a
//! `Feeder` from the tokio runtime, and write keyboard input back.
//!
//! The PTY itself comes from `alacritty_terminal::tty`. Three things are done here instead of
//! through its helpers:
//! - resizing uses `ioctl` directly, because `OnResize::on_resize` exits the process when the
//!   ioctl fails (it can, once the program is gone);
//! - the child's exit is read with `waitpid(WNOHANG)` when SIGCHLD arrives, since `Pty` only
//!   lends out `&Child`;
//! - dropping a `Pty` waits for the child, so it is dropped on a separate thread.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::tty::{self, Options, Shell};
use parking_lot::Mutex;
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
    pty: Mutex<Option<tty::Pty>>,
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

impl LocalPty {
    pub fn spawn(opts: &SpawnOptions) -> io::Result<Arc<Self>> {
        let mut env = opts.env.clone();
        base_env(&mut env);
        let options = Options {
            shell: opts.program.clone().map(|(p, a)| Shell::new(p, a)),
            working_directory: Some(opts.cwd.clone()),
            drain_on_exit: true,
            env,
        };
        let size = WindowSize {
            num_lines: opts.size.rows as u16,
            num_cols: opts.size.cols as u16,
            cell_width: opts.cell_px.0.max(1),
            cell_height: opts.cell_px.1.max(1),
        };
        let pty = tty::new(&options, size, 0)?;
        let master = pty.file().as_raw_fd();
        let pid = pty.child().id();
        Ok(Arc::new(Self {
            pty: Mutex::new(Some(pty)),
            master,
            pid,
            exit: Mutex::new(None),
        }))
    }

    /// Process id of the program started on the PTY (the shell).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn master_fd(&self) -> RawFd {
        self.master
    }

    fn dup_master(&self) -> io::Result<File> {
        match self.pty.lock().as_ref() {
            Some(pty) => pty.file().try_clone(),
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
        // SAFETY: the fd stays open while `self.pty` holds the Pty; a failure is harmless.
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
    /// (`WNOWAIT`): it stays a zombie until the `Pty` is dropped, which reaps it after sending
    /// SIGHUP. Reaping it here would free its pid while that SIGHUP is still to come, and a new
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

impl Drop for LocalPty {
    fn drop(&mut self) {
        // `Pty::drop` sends SIGHUP and then waits for the child: never on the UI thread.
        if let Some(pty) = self.pty.lock().take() {
            std::thread::spawn(move || drop(pty));
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
                while sig.recv().await.is_some() {
                    sender.send_modify(|n| *n = n.wrapping_add(1));
                }
            });
        });
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
}

impl Session {
    pub fn write(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            let _ = self.input.send(bytes);
        }
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
    let session = Session {
        pty: pty.clone(),
        input: tx.clone(),
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
                ready = reader.readable() => {
                    let Ok(mut guard) = ready else { break };
                    match guard.try_io(|inner| {
                        let mut f: &File = inner.get_ref();
                        f.read(&mut buf)
                    }) {
                        Ok(Ok(0)) => break,
                        Ok(Ok(n)) => {
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
                        Ok(Err(e)) if e.kind() == io::ErrorKind::Interrupted => {}
                        // EIO: the program and everything holding the terminal are gone.
                        Ok(Err(_)) => break,
                        Err(_would_block) => {}
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
