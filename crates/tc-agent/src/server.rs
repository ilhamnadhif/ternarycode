//! The socket `ternarycode-hook` writes to. It lives in a private per-user folder (on Windows: a
//! named pipe with a random name, open to this user only); every pane gets a random token, and
//! a message is accepted only with its pane's token (PRD section 8).

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};

use crate::protocol::HookMsg;

/// Largest amount read from one connection.
const MAX_CONNECTION: u64 = 64 * 1024;
/// Longest time one connection is read.
const CONNECTION_TIME: std::time::Duration = std::time::Duration::from_secs(5);

fn hash_of(data_dir: &Path) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data_dir.as_os_str().as_encoded_bytes() {
        hash = (hash ^ *b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Where the socket goes. Socket paths are limited to about 100 bytes, so it is not put in the
/// data folder: on macOS the per-user temporary folder (private), elsewhere `$XDG_RUNTIME_DIR`
/// or a private folder in `/tmp`. The name is derived from the data folder, so separate data
/// folders (tests) don't meet.
#[cfg(unix)]
pub fn socket_path(data_dir: &Path) -> PathBuf {
    let hash = hash_of(data_dir);
    let base = if cfg!(target_os = "macos") {
        std::env::temp_dir()
    } else {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                // SAFETY: getuid never fails.
                PathBuf::from(format!("/tmp/ternarycode-{}", unsafe { libc::getuid() }))
            })
    };
    base.join(format!("ternarycode-{:08x}", hash as u32)).join("hook.sock")
}

/// Windows: a named pipe. Pipe names are shared by every user of the machine, so the name is
/// random (nobody can make it first), and panes learn it from their environment. Without random
/// numbers there is no name (an empty path), and `start` fails.
#[cfg(windows)]
pub fn socket_path(data_dir: &Path) -> PathBuf {
    match new_token() {
        Ok(random) => PathBuf::from(format!(
            r"\\.\pipe\ternarycode-{:08x}-{random}",
            hash_of(data_dir) as u32
        )),
        Err(_) => PathBuf::new(),
    }
}

/// A folder owned by this user (not a link someone put in its place); `closed`: and closed to
/// others.
#[cfg(unix)]
fn check_private(dir: &Path, closed: bool) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(dir)?;
    // SAFETY: getuid never fails.
    let me = unsafe { libc::getuid() };
    if !m.is_dir() || m.uid() != me || (closed && m.mode() & 0o077 != 0) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a private folder of this user", dir.display()),
        ));
    }
    Ok(())
}

/// Makes `dir` a private folder of this user. chmod follows links, so what is there already is
/// checked first: a link or someone else's folder is refused before anything is changed.
#[cfg(unix)]
fn make_private(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    match check_private(dir, false) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = dir.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Not create_dir_all: that takes a link put here meanwhile for the folder.
            std::fs::DirBuilder::new().mode(0o700).create(dir)?;
        }
        Err(e) => return Err(e),
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    check_private(dir, true)
}

/// 16 random bytes in hex. Without random numbers there is no token: an all-zero one would be
/// known to everyone.
fn new_token() -> io::Result<String> {
    let mut raw = [0u8; 16];
    getrandom::fill(&mut raw).map_err(io::Error::other)?;
    Ok(raw.iter().map(|b| format!("{b:02x}")).collect())
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub struct HookServer {
    path: PathBuf,
    tokens: Arc<Mutex<HashMap<u64, String>>>,
    /// The socket's (device, inode): the path may be another instance's socket by the end.
    #[cfg(unix)]
    socket: (u64, u64),
    /// The listening socket's fd, and the task accepting on it.
    #[cfg(unix)]
    listener: (std::os::fd::RawFd, tokio::task::AbortHandle),
}

/// The hook server as the app's next run takes it over (the app restarting in place for an
/// update, its agents still running): the listening socket's fd, kept open through `exec`, and
/// the panes' tokens, which the agents' hooks still send.
#[cfg(unix)]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Handoff {
    pub fd: i32,
    pub tokens: HashMap<u64, String>,
}

/// Reads one connection: a message per line, each taken only with its pane's token. A hook
/// writes its line and closes at once; a connection kept open longer than `CONNECTION_TIME` is
/// closed, so idle clients can't pile up.
async fn serve_connection(
    stream: impl AsyncRead + Unpin,
    known: Arc<Mutex<HashMap<u64, String>>>,
    on_msg: Arc<dyn Fn(HookMsg) + Send + Sync>,
) {
    let mut lines = BufReader::new(stream.take(MAX_CONNECTION)).lines();
    let _ = tokio::time::timeout(CONNECTION_TIME, async {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(msg) = serde_json::from_str::<HookMsg>(&line) else {
                continue;
            };
            let valid = known
                .lock()
                .get(&msg.pane)
                .is_some_and(|t| same(t, &msg.token));
            if valid {
                on_msg(msg);
            }
        }
    })
    .await;
}

impl HookServer {
    /// Binds the socket and serves it on `rt`. `on_msg` runs for every accepted message.
    #[cfg(unix)]
    pub fn start(
        rt: &tokio::runtime::Handle,
        path: PathBuf,
        on_msg: Arc<dyn Fn(HookMsg) + Send + Sync>,
    ) -> io::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        // No tokens without random numbers, so no server either.
        new_token()?;
        if let Some(dir) = path.parent() {
            // Under /tmp (Linux without XDG_RUNTIME_DIR) another user could have made the
            // folders first: they must be ours and closed to others.
            if let Some(parent) = dir.parent()
                && parent.file_name().is_some_and(|n| n.to_string_lossy().starts_with("ternarycode-"))
            {
                make_private(parent)?;
            }
            make_private(dir)?;
        }
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        let bound = std::fs::symlink_metadata(&path)?;
        let socket = (bound.dev(), bound.ino());
        Ok(Self::serve(rt, path, socket, listener, HashMap::new(), on_msg))
    }

    #[cfg(unix)]
    fn serve(
        rt: &tokio::runtime::Handle,
        path: PathBuf,
        socket: (u64, u64),
        listener: std::os::unix::net::UnixListener,
        tokens: HashMap<u64, String>,
        on_msg: Arc<dyn Fn(HookMsg) + Send + Sync>,
    ) -> Self {
        use std::os::fd::AsRawFd;
        let fd = listener.as_raw_fd();
        let tokens = Arc::new(Mutex::new(tokens));
        let known = tokens.clone();
        let accept = rt.spawn(async move {
            let Ok(listener) = tokio::net::UnixListener::from_std(listener) else {
                return;
            };
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    // Out of file descriptors, say: wait instead of spinning on the error.
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    continue;
                };
                tokio::spawn(serve_connection(stream, known.clone(), on_msg.clone()));
            }
        });
        Self { path, tokens, socket, listener: (fd, accept.abort_handle()) }
    }

    /// Stops serving and hands the socket over to the app's next run (see [`Handoff`]): a hook
    /// sent from now on waits in the socket for the new run to read it. The socket file stays.
    #[cfg(unix)]
    pub fn handoff(self) -> io::Result<Handoff> {
        // A copy that lives through exec (`dup` does not set close-on-exec); the server's own
        // fd closes with its task.
        // SAFETY: dup of the fd the accepting task owns, which is open until it ends.
        let fd = unsafe { libc::dup(self.listener.0) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        self.listener.1.abort();
        let tokens = self.tokens.lock().clone();
        // Not dropped: that removes the socket file.
        std::mem::forget(self);
        Ok(Handoff { fd, tokens })
    }

    /// Serves the socket the app's previous run handed over, at `path`, with its tokens. Fails
    /// when the fd is not the socket at `path`; the caller then starts a new server.
    #[cfg(unix)]
    pub fn adopt(
        rt: &tokio::runtime::Handle,
        path: PathBuf,
        handoff: Handoff,
        on_msg: Arc<dyn Fn(HookMsg) + Send + Sync>,
    ) -> io::Result<Self> {
        use std::os::fd::FromRawFd;
        use std::os::unix::fs::MetadataExt;
        let fd = handoff.fd;
        // SAFETY: fstat on an fd number into a zeroed struct; nothing is assumed until checked.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut st) } == -1 || st.st_mode & libc::S_IFMT != libc::S_IFSOCK {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        // SAFETY: as above.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        // SAFETY: a socket fd handed over to this run alone.
        let listener = unsafe { std::os::unix::net::UnixListener::from_raw_fd(fd) };
        listener.set_nonblocking(true)?;
        // The fd is a socket; the file at `path` must be the one it is bound to.
        let local = listener.local_addr()?;
        let bound = std::fs::symlink_metadata(&path)?;
        if local.as_pathname() != Some(path.as_path()) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let socket = (bound.dev(), bound.ino());
        Ok(Self::serve(rt, path, socket, listener, handoff.tokens, on_msg))
    }

    /// Creates the named pipe and serves it on `rt`. `on_msg` runs for every accepted message.
    /// One instance always waits for the next client; each connection gets its own task.
    #[cfg(windows)]
    pub fn start(
        rt: &tokio::runtime::Handle,
        path: PathBuf,
        on_msg: Arc<dyn Fn(HookMsg) + Send + Sync>,
    ) -> io::Result<Self> {
        use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
        // No tokens without random numbers (and then `socket_path` made no name either).
        new_token()?;
        if path.as_os_str().is_empty() {
            return Err(io::Error::other("no pipe name"));
        }
        // Inbound only (clients write), local clients only. The default security of a pipe lets
        // only its creator (and administrators) write to it.
        fn instance(name: &std::ffi::OsStr, first: bool) -> io::Result<NamedPipeServer> {
            ServerOptions::new()
                .first_pipe_instance(first)
                .access_outbound(false)
                .reject_remote_clients(true)
                .create(name)
        }
        let name = path.as_os_str().to_owned();
        let first = {
            let _runtime = rt.enter();
            instance(&name, true)?
        };
        let tokens: Arc<Mutex<HashMap<u64, String>>> = Arc::default();
        let known = tokens.clone();
        rt.spawn(async move {
            let mut server = first;
            loop {
                let connected = server.connect().await;
                // The next instance first, so a client never finds no pipe to open.
                let next = loop {
                    match instance(&name, false) {
                        Ok(s) => break s,
                        Err(_) => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
                    }
                };
                let done = std::mem::replace(&mut server, next);
                if connected.is_ok() {
                    tokio::spawn(serve_connection(done, known.clone(), on_msg.clone()));
                }
            }
        });
        Ok(Self { path, tokens })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A new token for `pane` (replacing any earlier one). Should random numbers fail now, the
    /// pane gets an empty token, which `ternarycode-hook` takes for "not in TernaryCode", and
    /// nothing from it is accepted.
    pub fn register(&self, pane: u64) -> String {
        let mut tokens = self.tokens.lock();
        match new_token() {
            Ok(token) => {
                tokens.insert(pane, token.clone());
                token
            }
            Err(_) => {
                tokens.remove(&pane);
                String::new()
            }
        }
    }

    /// Accepts `token` for `pane` again: the token its shell was given by the app's previous
    /// run, when that run's socket could not be taken over.
    pub fn keep_token(&self, pane: u64, token: String) {
        self.tokens.lock().insert(pane, token);
    }

    pub fn unregister(&self, pane: u64) {
        self.tokens.lock().remove(&pane);
    }
}

impl Drop for HookServer {
    fn drop(&mut self) {
        // Only while the path is still our socket: another instance may have bound it since.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if std::fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.socket) {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn accepts_only_messages_with_the_pane_token() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let path = dir.path().join("run/hook.sock");
        #[cfg(windows)]
        let path = socket_path(dir.path());
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        let server = HookServer::start(
            rt.handle(),
            path.clone(),
            Arc::new(move |m| {
                let _ = tx.lock().send(m);
            }),
        )
        .unwrap();
        let token = server.register(7);
        let send = |pane: u64, token: &str, event: &str| {
            let env = |k: &str| -> Option<String> {
                match k {
                    crate::ENV_SOCK => Some(path.to_string_lossy().into_owned()),
                    crate::ENV_PANE => Some(pane.to_string()),
                    crate::ENV_TOKEN => Some(token.to_string()),
                    _ => None,
                }
            };
            let args = vec!["claude".to_string(), event.to_string(), "{}".to_string()];
            let (sock, msg) = crate::client::message(&args, env, || None).unwrap();
            crate::client::send(&sock, &msg).unwrap();
        };
        send(7, "wrong", "Stop");
        send(8, &token, "Stop");
        let t = std::time::Instant::now();
        send(7, &token, "PermissionRequest");
        let got = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        // PRD: hook to status in well under 50 ms; the socket part is a tiny share of that
        // (shared CI machines are given more room on Windows).
        let limit = Duration::from_millis(if cfg!(windows) { 250 } else { 50 });
        assert!(t.elapsed() < limit, "{:?}", t.elapsed());
        assert_eq!((got.pane, got.event.as_str()), (7, "PermissionRequest"));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err(), "only one accepted");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        server.unregister(7);
        send(7, &token, "Stop");
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        assert_eq!(server.register(9).len(), 32);
    }

    #[cfg(unix)]
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    #[cfg(unix)]
    fn a_linked_folder_is_refused_before_it_is_changed() {
        use std::os::unix::fs::PermissionsExt;
        let rt = runtime();
        let dir = tempfile::tempdir().unwrap();
        let theirs = dir.path().join("theirs");
        std::fs::create_dir(&theirs).unwrap();
        std::fs::set_permissions(&theirs, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(&theirs, dir.path().join("run")).unwrap();
        let started = HookServer::start(rt.handle(), dir.path().join("run/hook.sock"), Arc::new(|_| {}));
        assert_eq!(started.err().map(|e| e.kind()), Some(io::ErrorKind::PermissionDenied));
        let mode = std::fs::metadata(&theirs).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "chmod went through the link");
        assert!(!theirs.join("hook.sock").exists());
    }

    #[test]
    #[cfg(unix)]
    fn a_server_leaves_the_socket_of_the_next_one_alone() {
        let rt = runtime();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run/hook.sock");
        let first = HookServer::start(rt.handle(), path.clone(), Arc::new(|_| {})).unwrap();
        let second = HookServer::start(rt.handle(), path.clone(), Arc::new(|_| {})).unwrap();
        drop(first);
        assert!(path.exists(), "the second server's socket was removed");
        drop(second);
        assert!(!path.exists());
    }

    #[test]
    #[cfg(unix)]
    fn an_idle_connection_is_closed() {
        use std::io::Read;
        let rt = runtime();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run/hook.sock");
        let _server = HookServer::start(rt.handle(), path.clone(), Arc::new(|_| {})).unwrap();
        let mut idle = std::os::unix::net::UnixStream::connect(&path).unwrap();
        idle.set_read_timeout(Some(CONNECTION_TIME * 3)).unwrap();
        let t = std::time::Instant::now();
        // The server closes it (end of file), rather than the read timing out.
        assert_eq!(idle.read(&mut [0u8; 8]).unwrap(), 0);
        assert!(t.elapsed() < CONNECTION_TIME * 2, "{:?}", t.elapsed());
    }

    /// A server handed over (as the app restarts in place) keeps its socket file and tokens: a
    /// hook sent meanwhile, or after, reaches the new one.
    #[cfg(unix)]
    #[test]
    fn a_handed_over_server_keeps_its_socket_and_tokens() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run/hook.sock");
        let server = HookServer::start(rt.handle(), path.clone(), Arc::new(|_| panic!("old server"))).unwrap();
        let token = server.register(7);
        let handoff = server.handoff().unwrap();
        assert!(path.exists());
        let send = || {
            let env = |k: &str| -> Option<String> {
                match k {
                    crate::ENV_SOCK => Some(path.to_string_lossy().into_owned()),
                    crate::ENV_PANE => Some("7".into()),
                    crate::ENV_TOKEN => Some(token.clone()),
                    _ => None,
                }
            };
            let args = vec!["claude".to_string(), "Stop".to_string(), "{}".to_string()];
            let (sock, msg) = crate::client::message(&args, env, || None).unwrap();
            crate::client::send(&sock, &msg).unwrap();
        };
        // Sent between the two runs: it waits in the socket.
        std::thread::sleep(Duration::from_millis(50));
        send();
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        let _new = HookServer::adopt(rt.handle(), path.clone(), handoff, Arc::new(move |m| {
            let _ = tx.lock().send(m);
        }))
        .unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().pane, 7);
        send();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap().pane, 7);
    }
}
