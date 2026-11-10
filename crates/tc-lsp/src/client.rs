//! One language server process and the JSON-RPC conversation with it. Requests return their id
//! at once; the answer comes later through the callback, with everything else the server sends
//! (notifications such as diagnostics, and its own requests, which the app answers with
//! `respond`). Nothing here blocks the caller: the reader, writer and process waiter are tasks
//! on the given runtime.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use crate::transport::{Framer, encode};

/// An error answer.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// What the server sent.
#[derive(Clone, Debug)]
pub enum Incoming {
    /// The answer to one of our requests.
    Response { id: i64, result: Result<Value, RpcError> },
    Notification { method: String, params: Value },
    /// A request from the server (answer with `Client::respond`).
    Request { id: Value, method: String, params: Value },
    /// The process ended (its last stderr lines, to show why).
    Exited { code: Option<i32>, stderr: String },
}

pub struct Client {
    out: mpsc::UnboundedSender<Vec<u8>>,
    next_id: AtomicI64,
    pid: Option<u32>,
    kill: Mutex<Option<oneshot::Sender<()>>>,
    rt: Handle,
}

/// The last bytes of the server's stderr kept, for the error shown when it dies.
const STDERR_KEPT: usize = 4096;

impl Client {
    /// Starts `program args` in `cwd` with `path` as its `PATH`. Messages from it go to
    /// `on_message`, called on a runtime thread.
    pub fn spawn(
        rt: &Handle,
        program: &Path,
        args: &[&str],
        cwd: &Path,
        path: Option<&OsString>,
        on_message: impl Fn(Incoming) + Send + Sync + 'static,
    ) -> std::io::Result<Client> {
        let _guard = rt.enter();
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // No console window for the server (the app has none of its own on Windows).
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        if let Some(p) = path {
            cmd.env("PATH", p);
        }
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let mut stdin = child.stdin.take().expect("piped stdin");
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let on_message = Arc::new(on_message);
        let tail = Arc::new(Mutex::new(Vec::<u8>::new()));

        // Writer: frames in order.
        let (out, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        rt.spawn(async move {
            while let Some(frame) = rx.recv().await {
                if stdin.write_all(&frame).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });
        // Reader: frames → messages.
        let reader_cb = on_message.clone();
        rt.spawn(async move {
            let mut framer = Framer::new();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match stdout.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        for msg in framer.push(&buf[..n]).into_iter().flatten() {
                            if let Some(m) = classify(msg) {
                                reader_cb(m);
                            }
                        }
                    }
                }
            }
        });
        // Stderr: the last bytes, for the exit message.
        let err_tail = tail.clone();
        rt.spawn(async move {
            let mut buf = vec![0u8; 8192];
            while let Ok(n) = stderr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let mut t = err_tail.lock();
                t.extend_from_slice(&buf[..n]);
                let over = t.len().saturating_sub(STDERR_KEPT);
                t.drain(..over);
            }
        });
        // The process: its exit, or a kill.
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        rt.spawn(async move {
            let status = tokio::select! {
                s = child.wait() => s.ok(),
                _ = kill_rx => {
                    let _ = child.start_kill();
                    child.wait().await.ok()
                }
            };
            // Let the stderr reader catch up.
            tokio::time::sleep(Duration::from_millis(50)).await;
            let stderr = String::from_utf8_lossy(&tail.lock()).trim().to_string();
            on_message(Incoming::Exited {
                code: status.and_then(|s| s.code()),
                stderr,
            });
        });
        Ok(Client {
            out,
            next_id: AtomicI64::new(1),
            pid,
            kill: Mutex::new(Some(kill_tx)),
            rt: rt.clone(),
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Sends a request; its answer arrives as `Incoming::Response` with this id.
    pub fn request(&self, method: &str, params: Value) -> i64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let _ = self.out.send(encode(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})));
        id
    }

    pub fn notify(&self, method: &str, params: Value) {
        let _ = self.out.send(encode(&json!({"jsonrpc": "2.0", "method": method, "params": params})));
    }

    /// Answers a request from the server.
    pub fn respond(&self, id: Value, result: Result<Value, RpcError>) {
        let msg = match result {
            Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": e.code, "message": e.message}}),
        };
        let _ = self.out.send(encode(&msg));
    }

    /// Asks the server to stop (`shutdown`, then `exit`) and kills it if it is still there a
    /// little later.
    pub fn shutdown(&self) {
        self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        if let Some(kill) = self.kill.lock().take() {
            self.rt.spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let _ = kill.send(());
            });
        }
    }

    /// Ends the process now.
    pub fn kill(&self) {
        if let Some(kill) = self.kill.lock().take() {
            let _ = kill.send(());
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A response, a notification or a request, by its fields.
fn classify(msg: Value) -> Option<Incoming> {
    let method = msg.get("method").and_then(Value::as_str).map(str::to_string);
    let id = msg.get("id").cloned().filter(|v| !v.is_null());
    match (method, id) {
        (Some(method), Some(id)) => Some(Incoming::Request {
            id,
            method,
            params: msg.get("params").cloned().unwrap_or(Value::Null),
        }),
        (Some(method), None) => Some(Incoming::Notification {
            method,
            params: msg.get("params").cloned().unwrap_or(Value::Null),
        }),
        (None, Some(id)) => {
            let id = id.as_i64()?;
            let result = match msg.get("error") {
                Some(e) if !e.is_null() => Err(RpcError {
                    code: e.get("code").and_then(Value::as_i64).unwrap_or(0),
                    message: e.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
                }),
                _ => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
            };
            Some(Incoming::Response { id, result })
        }
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_told_apart() {
        let r = classify(json!({"id": 3, "result": [1]})).unwrap();
        assert!(matches!(r, Incoming::Response { id: 3, result: Ok(_) }));
        let e = classify(json!({"id": 4, "error": {"code": -32601, "message": "no"}})).unwrap();
        assert!(matches!(e, Incoming::Response { id: 4, result: Err(RpcError { code: -32601, .. }) }));
        let n = classify(json!({"method": "textDocument/publishDiagnostics", "params": {}})).unwrap();
        assert!(matches!(n, Incoming::Notification { .. }));
        let q = classify(json!({"id": "x", "method": "workspace/applyEdit", "params": {}})).unwrap();
        assert!(matches!(q, Incoming::Request { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn a_server_answers_and_its_exit_is_reported() {
        // `cat` echoes our request back: it reads as a request from the server with our id.
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let client = Client::spawn(rt.handle(), Path::new("/bin/cat"), &[], Path::new("/"), None, move |m| {
            let _ = tx.send(m);
        })
        .unwrap();
        let id = client.request("initialize", json!({"x": 1}));
        let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        match got {
            Incoming::Request { id: rid, method, params } => {
                assert_eq!(rid, json!(id));
                assert_eq!(method, "initialize");
                assert_eq!(params["x"], 1);
            }
            other => panic!("{other:?}"),
        }
        client.kill();
        let exited = (0..10).find_map(|_| match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Incoming::Exited { .. }) => Some(()),
            _ => None,
        });
        assert!(exited.is_some());
    }
}
