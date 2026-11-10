//! Local PTY sessions on Windows, through the pseudoconsole (ConPTY): the same interface as the
//! Unix `pty.rs`, so the app drives both alike.
//!
//! - The program runs attached to a pseudoconsole fed by two anonymous pipes. A thread reads its
//!   output into a channel that the session task parses, a thread writes keyboard input, and a
//!   thread waits for the program to end.
//! - The pseudoconsole keeps its output open after the program ended: the session then closes
//!   it (after taking what is still coming), and the reader sees the end of the output.
//! - Hanging up closes the pseudoconsole (the programs attached to it get `CTRL_CLOSE_EVENT`,
//!   the counterpart of SIGHUP), then terminates the program if it is still there after the
//!   grace time. Closing can wait until the output is drained, so it runs on its own thread
//!   while the reader keeps reading.
//! - There are no process groups: the "foreground" program is the newest live child of the
//!   shell (a Toolhelp snapshot), followed through `cmd.exe`, which runs `.cmd` launchers.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::{mpsc, watch};
use windows_sys::Win32::Foundation::{
    CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, S_OK, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
    TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, GetProcessTimes, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcess,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject,
};

use crate::core::{Fed, Feeder, GridSize};

/// How a program on a PTY ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitInfo {
    Code(i32),
    /// Unix only (a program killed by a signal); kept so the app matches on one type.
    Signal(i32),
    /// The status is unknown.
    Unknown,
}

impl ExitInfo {
    pub fn success(self) -> bool {
        self == ExitInfo::Code(0)
    }
}

pub struct SpawnOptions {
    /// Program and arguments; `None` runs the default shell ([`default_shell`]).
    pub program: Option<(String, Vec<String>)>,
    pub cwd: PathBuf,
    /// Extra environment variables (on top of the app's own environment).
    pub env: HashMap<String, String>,
    pub size: GridSize,
    /// Cell size in physical pixels (unused: the pseudoconsole has no pixel size).
    pub cell_px: (u16, u16),
}

/// The open pseudoconsole.
struct Console(HPCON);

impl Console {
    /// Closes it on a thread of its own: `ClosePseudoConsole` can wait until the output pipe
    /// was drained (by the session's reader).
    fn close(self) {
        let hpc = self.0;
        let _ = std::thread::Builder::new()
            .name("ternarycode-pty-close".into())
            .spawn(move || {
                // SAFETY: the handle came from CreatePseudoConsole and is closed once (taken).
                unsafe { ClosePseudoConsole(hpc) };
            });
    }
}

/// A program running on a local pseudoconsole.
pub struct LocalPty {
    pid: u32,
    process: OwnedHandle,
    /// When the program started (FILETIME ticks): a "child" that started earlier has the pid of
    /// a parent that is gone (Windows reuses ids).
    started: u64,
    console: Mutex<Option<Console>>,
    /// Our ends of the pipes (output to read, input to write) until `start` takes them.
    pipes: Mutex<Option<(File, File)>>,
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
}

/// The shell a pane runs when nothing else is asked for: PowerShell 7 (`pwsh.exe`) when it is
/// installed, otherwise the Windows PowerShell every Windows has, otherwise `cmd.exe`.
pub fn default_shell() -> &'static str {
    static SHELL: OnceLock<String> = OnceLock::new();
    SHELL.get_or_init(|| {
        if let Some(pwsh) = find_on_path("pwsh.exe") {
            return pwsh.to_string_lossy().into_owned();
        }
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let system = Path::new(&root).join("System32");
        let powershell = system.join("WindowsPowerShell\\v1.0\\powershell.exe");
        if powershell.is_file() {
            return powershell.to_string_lossy().into_owned();
        }
        std::env::var("COMSPEC")
            .ok()
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| system.join("cmd.exe").to_string_lossy().into_owned())
    })
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// `s` as a NUL-terminated UTF-16 string.
fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Appends `arg` to a command line, quoted the way programs built with the Microsoft C runtime
/// (and Rust's std) split it again.
fn push_arg(cmd: &mut String, arg: &str) {
    if !cmd.is_empty() {
        cmd.push(' ');
    }
    let quote = arg.is_empty() || arg.contains([' ', '\t', '"']);
    if !quote {
        cmd.push_str(arg);
        return;
    }
    cmd.push('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        if c == '\\' {
            backslashes += 1;
            continue;
        }
        // Backslashes before a quote are doubled, and the quote escaped; others stay as is.
        let n = if c == '"' { backslashes * 2 + 1 } else { backslashes };
        cmd.extend(std::iter::repeat_n('\\', n));
        backslashes = 0;
        cmd.push(c);
    }
    // Doubled before the closing quote.
    cmd.extend(std::iter::repeat_n('\\', backslashes * 2));
    cmd.push('"');
}

fn command_line(program: &str, args: &[String]) -> String {
    let mut cmd = String::new();
    push_arg(&mut cmd, program);
    for a in args {
        push_arg(&mut cmd, a);
    }
    cmd
}

/// The environment block: `env` over this process's environment. Names are case-insensitive
/// on Windows, so the first spelling wins; the block is sorted as Windows expects.
fn env_block(env: &HashMap<String, String>) -> Vec<u16> {
    let mut vars: Vec<(std::ffi::OsString, std::ffi::OsString)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (k, v) in env {
        if seen.insert(k.to_uppercase()) {
            vars.push((k.into(), v.into()));
        }
    }
    for (k, v) in std::env::vars_os() {
        if seen.insert(k.to_string_lossy().to_uppercase()) {
            vars.push((k, v));
        }
    }
    vars.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (k, v) in vars {
        block.extend(k.encode_wide());
        block.push(u16::from(b'='));
        block.extend(v.encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

fn coord(size: GridSize) -> COORD {
    COORD {
        X: size.cols.clamp(1, i16::MAX as usize) as i16,
        Y: size.rows.clamp(1, i16::MAX as usize) as i16,
    }
}

fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
    // SAFETY: CreatePipe fills both handles on success; they are owned from here on.
    unsafe {
        if CreatePipe(&mut read, &mut write, std::ptr::null(), 0) == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        ))
    }
}

fn filetime(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}

/// When the process behind `handle` started.
fn start_time(handle: HANDLE) -> Option<u64> {
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: four valid out-pointers; a handle with query rights.
    let ok = unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) };
    (ok != 0).then(|| filetime(created))
}

/// Whether the process behind `handle` still runs.
fn running(handle: HANDLE) -> bool {
    let mut code = 0u32;
    // SAFETY: a handle with query rights and a valid out-pointer.
    unsafe { GetExitCodeProcess(handle, &mut code) != 0 && code == STILL_ACTIVE }
}

const STILL_ACTIVE: u32 = 259;

impl LocalPty {
    pub fn spawn(opts: &SpawnOptions) -> io::Result<Arc<Self>> {
        let mut env = opts.env.clone();
        base_env(&mut env);
        let cmdline = match &opts.program {
            Some((program, args)) => command_line(program, args),
            None => command_line(default_shell(), &[]),
        };

        let (in_read, in_write) = pipe()?;
        let (out_read, out_write) = pipe()?;
        let mut hpc: HPCON = 0;
        // SAFETY: valid pipe handles; the pseudoconsole duplicates them.
        let hr = unsafe {
            CreatePseudoConsole(
                coord(opts.size),
                in_read.as_raw_handle() as HANDLE,
                out_write.as_raw_handle() as HANDLE,
                0,
                &mut hpc,
            )
        };
        // The pseudoconsole has its own copies: with ours closed, the output ends when it closes.
        drop(in_read);
        drop(out_write);
        if hr != S_OK {
            return Err(io::Error::other(format!(
                "the pseudoconsole could not be created (0x{hr:08x})"
            )));
        }
        let console = Console(hpc);

        // The attribute list that attaches the program to the pseudoconsole. Its size is asked
        // first (that call fails by design); u64 storage keeps it aligned.
        let mut size = 0usize;
        // SAFETY: a null list with a size pointer only reports the size needed.
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
        let mut storage = vec![0u64; size.div_ceil(8).max(1)];
        let list = storage.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        // SAFETY: `list` has `size` bytes.
        if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
            let e = io::Error::last_os_error();
            console.close();
            return Err(e);
        }
        // SAFETY: the attribute's value is the pseudoconsole handle itself.
        let attached = unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                hpc as *const std::ffi::c_void,
                std::mem::size_of::<HPCON>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if attached == 0 {
            let e = io::Error::last_os_error();
            // SAFETY: initialized above.
            unsafe { DeleteProcThreadAttributeList(list) };
            console.close();
            return Err(e);
        }

        // SAFETY: all-zero is a valid STARTUPINFOEXW.
        let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        // Standard handles set, to nothing: the program uses the pseudoconsole's, never ours
        // (redirected ones, such as a test run's pipes, would otherwise be inherited).
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
        si.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
        si.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
        si.lpAttributeList = list;
        let mut cmd: Vec<u16> = wide(OsStr::new(&cmdline));
        let block = env_block(&env);
        let cwd = wide(opts.cwd.as_os_str());
        // SAFETY: all-zero is a valid PROCESS_INFORMATION; CreateProcessW fills it.
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: NUL-terminated strings and blocks that outlive the call.
        let created = unsafe {
            CreateProcessW(
                std::ptr::null(),
                cmd.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                block.as_ptr() as *const std::ffi::c_void,
                cwd.as_ptr(),
                &si.StartupInfo,
                &mut pi,
            )
        };
        let error = io::Error::last_os_error();
        // SAFETY: initialized above; no longer needed once the process exists (or failed to).
        unsafe { DeleteProcThreadAttributeList(list) };
        drop(storage);
        if created == 0 {
            console.close();
            return Err(error);
        }
        // SAFETY: CreateProcessW returned both handles; the thread's is not needed.
        let process = unsafe {
            CloseHandle(pi.hThread);
            OwnedHandle::from_raw_handle(pi.hProcess)
        };
        let started = start_time(process.as_raw_handle() as HANDLE).unwrap_or(0);
        Ok(Arc::new(Self {
            pid: pi.dwProcessId,
            process,
            started,
            console: Mutex::new(Some(console)),
            pipes: Mutex::new(Some((File::from(out_read), File::from(in_write)))),
            exit: Mutex::new(None),
        }))
    }

    /// Process id of the program started on the PTY (the shell).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    fn handle(&self) -> HANDLE {
        self.process.as_raw_handle() as HANDLE
    }

    /// Tells the pseudoconsole the new size (the program sees a window-size event).
    pub fn resize(&self, size: GridSize, _cell_w: u16, _cell_h: u16) {
        if let Some(console) = self.console.lock().as_ref() {
            // SAFETY: an open pseudoconsole; a failure is harmless.
            unsafe { ResizePseudoConsole(console.0, coord(size)) };
        }
    }

    /// The program in the foreground: the shell's newest running child (followed through
    /// `cmd.exe`), or the shell's own pid when it runs none. A pid, not a process group: the
    /// app only compares it with [`pid`](Self::pid) and reads the process it names.
    pub fn foreground_pgid(&self) -> Option<i32> {
        if self.poll_exit().is_some() {
            return None;
        }
        let mut current = (self.pid, self.started);
        let mut found = self.pid;
        for _ in 0..3 {
            let Some((child, started, name)) = newest_child(current.0, current.1) else {
                break;
            };
            found = child;
            if !name.eq_ignore_ascii_case("cmd.exe") {
                break;
            }
            current = (child, started);
        }
        Some(found as i32)
    }

    /// The program's exit status once it has ended. The handle stays open, so its pid is not
    /// given to another process while the pane may still act on it.
    pub fn poll_exit(&self) -> Option<ExitInfo> {
        let mut cached = self.exit.lock();
        if cached.is_some() {
            return *cached;
        }
        // SAFETY: our own process handle.
        if unsafe { WaitForSingleObject(self.handle(), 0) } != WAIT_OBJECT_0 {
            return None;
        }
        let mut code = 0u32;
        // SAFETY: as above.
        let info = if unsafe { GetExitCodeProcess(self.handle(), &mut code) } != 0 {
            ExitInfo::Code(code as i32)
        } else {
            ExitInfo::Unknown
        };
        *cached = Some(info);
        Some(info)
    }

    pub fn is_alive(&self) -> bool {
        self.poll_exit().is_none()
    }

    /// Closes the pseudoconsole (once).
    fn close_console(&self) {
        if let Some(console) = self.console.lock().take() {
            console.close();
        }
    }

    /// Ends the program the way closing a console window does: the pseudoconsole closes (its
    /// programs get `CTRL_CLOSE_EVENT`), then the program is terminated if it is still there
    /// after `grace`. Waits on a background thread.
    pub fn hang_up(self: &Arc<Self>, grace: Duration) {
        self.close_console();
        let me = self.clone();
        let _ = std::thread::Builder::new()
            .name("ternarycode-pty-hangup".into())
            .spawn(move || {
                let ms = grace.as_millis().min(u128::from(u32::MAX - 1)) as u32;
                // SAFETY: our own process handle, kept open by `me`.
                unsafe {
                    if WaitForSingleObject(me.handle(), ms) != WAIT_OBJECT_0 {
                        TerminateProcess(me.handle(), 1);
                    }
                }
            });
    }
}

impl Drop for LocalPty {
    fn drop(&mut self) {
        // Our pipe ends first (when no session took them), so closing never waits on output
        // nobody reads.
        drop(self.pipes.lock().take());
        self.close_console();
    }
}

/// The newest running child of `parent` (which started at `parent_started`): its pid, start
/// time and executable name. Prompt helpers that run between commands are not programs the
/// user started.
fn newest_child(parent: u32, parent_started: u64) -> Option<(u32, u64, String)> {
    const NOT_PROGRAMS: &[&str] = &["conhost.exe", "OpenConsole.exe", "starship.exe", "oh-my-posh.exe"];
    let mut best: Option<(u32, u64, String)> = None;
    for_each_process(|entry| {
        let pid = entry.th32ProcessID;
        if entry.th32ParentProcessID != parent || pid == parent || pid == 0 {
            return;
        }
        let name = exe_name(&entry.szExeFile);
        if NOT_PROGRAMS.iter().any(|n| name.eq_ignore_ascii_case(n)) {
            return;
        }
        // SAFETY: OpenProcess returns null on failure; the handle is closed below.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return;
        }
        // SAFETY: an owned handle from OpenProcess.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        let raw = handle.as_raw_handle() as HANDLE;
        let Some(started) = start_time(raw) else { return };
        if started < parent_started || !running(raw) {
            return;
        }
        if best.as_ref().is_none_or(|(_, t, _)| started > *t) {
            best = Some((pid, started, name));
        }
    });
    best
}

fn exe_name(raw: &[u16]) -> String {
    let end = raw.iter().position(|c| *c == 0).unwrap_or(raw.len());
    String::from_utf16_lossy(&raw[..end])
}

/// Runs `f` for every process in a snapshot of the system.
fn for_each_process(mut f: impl FnMut(&PROCESSENTRY32W)) {
    // SAFETY: a snapshot handle, closed when `snapshot` drops; entries are plain data.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return;
        }
        let snapshot = OwnedHandle::from_raw_handle(snap);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let raw = snapshot.as_raw_handle() as HANDLE;
        if Process32FirstW(raw, &mut entry) == 0 {
            return;
        }
        loop {
            f(&entry);
            if Process32NextW(raw, &mut entry) == 0 {
                break;
            }
        }
    }
}

/// What a running session reports.
#[derive(Debug)]
pub enum SessionEvent {
    /// Output was parsed into the terminal.
    Output(Fed),
    /// The foreground program may have changed (checked as output arrives, at most every
    /// [`FG_INTERVAL`], and once more after the last output).
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
/// How long output is still taken after the program ended, before the pseudoconsole closes.
const DRAIN: Duration = Duration::from_millis(100);

/// Starts reading `pty` into `feeder` and writing input to it, on `rt`. `on_event` runs on a
/// runtime thread after each read and once at the end.
pub fn start(
    rt: &tokio::runtime::Handle,
    pty: Arc<LocalPty>,
    feeder: Arc<Mutex<Feeder>>,
    on_event: Arc<dyn Fn(SessionEvent) + Send + Sync>,
) -> io::Result<Session> {
    let (mut output, mut input) = pty
        .pipes
        .lock()
        .take()
        .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?;
    let waiter = pty.process.try_clone()?;
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let session = Session {
        pty: pty.clone(),
        input: tx.clone(),
    };

    // Writer: blocking writes into the input pipe.
    std::thread::Builder::new()
        .name("ternarycode-pty-in".into())
        .spawn(move || {
            while let Some(data) = rx.blocking_recv() {
                if input.write_all(&data).is_err() {
                    break;
                }
            }
        })?;

    // Reader: blocking reads, handed to the session task. Once that is gone it keeps reading
    // (and drops what it reads) until the end, so closing the pseudoconsole never waits on it.
    let (chunk_tx, mut chunks) = mpsc::channel::<Vec<u8>>(4);
    std::thread::Builder::new()
        .name("ternarycode-pty-out".into())
        .spawn(move || {
            let mut buf = vec![0u8; READ_BUF];
            loop {
                match output.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = chunk_tx.blocking_send(buf[..n].to_vec());
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        })?;

    // Waiter: the program's end.
    let (exit_tx, mut exit_rx) = watch::channel(false);
    std::thread::Builder::new()
        .name("ternarycode-pty-wait".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            // SAFETY: a duplicate of the process handle, owned by this thread.
            unsafe { WaitForSingleObject(waiter.as_raw_handle() as HANDLE, INFINITE) };
            let _ = exit_tx.send(true);
        })?;

    rt.spawn(async move {
        let deliver = |fed: Fed| {
            if !fed.reply.is_empty() {
                let _ = tx.send(fed.reply.clone());
            }
            on_event(SessionEvent::Output(fed));
        };
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
        let mut exited = false;
        let mut close_at: Option<tokio::time::Instant> = None;
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
            let close_timer = async {
                match close_at {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                chunk = chunks.recv() => {
                    // None: the output ended (the pseudoconsole closed).
                    let Some(bytes) = chunk else { break };
                    let fed = feeder.lock().feed(&bytes);
                    deliver(fed);
                    if fg_due.is_none() && !exited {
                        let at = fg_checked + FG_INTERVAL;
                        if at <= tokio::time::Instant::now() {
                            check_fg(&mut fg_seen, &mut fg_checked);
                        }
                        fg_due = Some((fg_checked + FG_INTERVAL).max(at));
                    }
                }
                _ = sync => {
                    let reply = feeder.lock().flush_sync();
                    deliver(Fed { reply, repaint: true, ..Default::default() });
                }
                _ = fg_timer => {
                    fg_due = None;
                    if !exited {
                        check_fg(&mut fg_seen, &mut fg_checked);
                    }
                }
                _ = exit_rx.changed(), if !exited => {
                    // The program is gone; the pseudoconsole keeps the output open. Take what
                    // is still coming, then close it so the output ends.
                    exited = true;
                    close_at = Some(tokio::time::Instant::now() + DRAIN);
                }
                _ = close_timer => {
                    close_at = None;
                    pty.close_console();
                }
            }
        }
        // The output ended; the status usually follows within a moment.
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
    use crate::scheme::SCHEMES;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line};
    use std::sync::mpsc as std_mpsc;

    #[test]
    fn command_lines_are_quoted_for_the_c_runtime() {
        assert_eq!(command_line("cmd.exe", &["/c".into(), "echo hi".into()]), r#"cmd.exe /c "echo hi""#);
        assert_eq!(
            command_line(r"C:\Program Files\PowerShell\7\pwsh.exe", &["-NoLogo".into()]),
            r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo"#
        );
        let mut s = String::new();
        push_arg(&mut s, r#"say "hi""#);
        assert_eq!(s, r#""say \"hi\"""#);
        let mut s = String::new();
        push_arg(&mut s, r"C:\dir with space\");
        assert_eq!(s, r#""C:\dir with space\\""#);
        let mut s = String::new();
        push_arg(&mut s, "");
        assert_eq!(s, r#""""#);
    }

    #[test]
    fn the_environment_block_is_sorted_and_case_insensitive() {
        let mut env = HashMap::new();
        env.insert("Path".to_string(), "ours".to_string());
        let block = env_block(&env);
        let text = String::from_utf16_lossy(&block);
        let vars: Vec<&str> = text.split('\0').filter(|v| !v.is_empty()).collect();
        let paths: Vec<&&str> = vars.iter().filter(|v| v.to_uppercase().starts_with("PATH=")).collect();
        assert_eq!(paths, [&"Path=ours"]);
        let mut sorted = vars.clone();
        sorted.sort_by_key(|v| v.split('=').next().unwrap_or("").to_uppercase());
        assert_eq!(vars, sorted);
        assert!(block.ends_with(&[0, 0]));
    }

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
            program: Some(("cmd.exe".into(), vec!["/d".into(), "/v:on".into(), "/c".into(), script.into()])),
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

    fn wait_exit(run: &Run) -> ExitInfo {
        loop {
            match run.events.recv_timeout(Duration::from_secs(20)).expect("event") {
                SessionEvent::Exited(info) => return info,
                SessionEvent::Output(_) | SessionEvent::Foreground => {}
            }
        }
    }

    #[test]
    fn runs_a_program_and_reports_output_and_exit() {
        let rt = rt();
        let run = run(&rt, "echo hello & exit 3", GridSize { cols: 40, rows: 5 });
        assert_eq!(wait_exit(&run), ExitInfo::Code(3));
        assert!(screen(&run.feeder).contains("hello"), "{}", screen(&run.feeder));
    }

    #[test]
    fn input_reaches_the_program_and_resize_is_seen() {
        let rt = rt();
        let run = run(
            &rt,
            "set /p line=& mode con & echo got:!line!",
            GridSize { cols: 40, rows: 5 },
        );
        run.session
            .pty
            .resize(GridSize { cols: 100, rows: 30 }, 8, 16);
        std::thread::sleep(Duration::from_millis(300));
        run.session.write(b"ping\r".to_vec());
        assert!(wait_exit(&run).success());
        let text = screen(&run.feeder);
        assert!(text.contains("got:ping"), "{text}");
        assert!(
            text.lines().any(|l| l.contains("Columns") && l.contains("100")),
            "{text}"
        );
    }

    #[test]
    fn foreground_program_and_hang_up() {
        let rt = rt();
        let run = run(&rt, "ping -n 30 127.0.0.1 >nul", GridSize { cols: 40, rows: 5 });
        let pty = run.session.pty.clone();
        let mut fg = None;
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            fg = pty.foreground_pgid();
            if fg.is_some_and(|p| p != pty.pid() as i32) {
                break;
            }
        }
        // cmd.exe runs ping.exe: that is the program in the foreground.
        assert!(fg.is_some_and(|p| p != pty.pid() as i32), "{fg:?}");
        assert!(pty.is_alive());
        pty.hang_up(Duration::from_millis(500));
        assert!(!wait_exit(&run).success());
    }
}
