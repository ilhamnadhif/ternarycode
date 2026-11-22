//! Embedded local HTTP/1.1 + Server-Sent Events server for Phone Pairing and Mobile Companion
//! remote monitoring & control (zero extra web-framework dependencies).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::mpsc::Sender;

use eframe::egui;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tc_core::agent::AgentKind;
use tc_core::model::PairedDevice;
use tc_core::{PaneId, ProjectId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

use crate::app::AppMsg;

const WEB_HTML: &str = include_str!("companion_web.html");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickRemoteAction {
    Approve,
    Reject,
    Interrupt,
}

/// Commands sent from paired mobile companions back to the desktop UI thread.
#[derive(Clone, Debug)]
pub enum CompanionCmd {
    SendInput { pane: PaneId, text: String, enter: bool },
    QuickAction { pane: PaneId, kind: QuickRemoteAction },
    SwitchProfile { pane: PaneId, profile_id: String },
    ResumeSession { project: ProjectId, agent: AgentKind, session_id: String, profile_id: Option<String> },
    DevicePaired(PairedDevice),
    DeviceSeen { id: String, ip: String, at: i64 },
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct CompanionSnapshot {
    pub failover_mode: String,
    pub projects: Vec<CompanionProject>,
    pub panes: Vec<CompanionPane>,
    pub profiles: Vec<CompanionProfile>,
    pub alerts: Vec<CompanionAlert>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompanionProject {
    pub id: ProjectId,
    pub name: String,
    pub branch: Option<String>,
    pub sessions: Vec<CompanionSession>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompanionSession {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub profile: Option<String>,
    pub status: String,
    pub age: String,
    pub live_pane: Option<PaneId>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompanionPane {
    pub id: PaneId,
    pub project_id: ProjectId,
    pub project_name: String,
    pub label: String,
    pub branch: Option<String>,
    pub profile: Option<String>,
    pub status: String,
    pub status_label: String,
    pub message: Option<String>,
    pub output: String,
    pub rate_limit: Option<CompanionRateLimit>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompanionRateLimit {
    pub from_profile: String,
    pub reason: String,
    pub reset_secs: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompanionProfile {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub state: String,
    pub enabled: bool,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub quota_pct: Option<f32>,
    pub reset_secs: u64,
    pub active_session: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompanionAlert {
    pub time: String,
    pub level: String,
    pub profile: Option<String>,
    pub message: String,
}

#[derive(Default)]
struct AuthState {
    pair_token: String,
    /// `token_hash` -> `device_id`.
    devices: HashMap<String, String>,
}

struct ServerState {
    auth: RwLock<AuthState>,
    snapshot: RwLock<String>,
    events: broadcast::Sender<String>,
    tx: Sender<AppMsg>,
    ctx: egui::Context,
}

/// Handle to a running embedded Mobile Companion server.
pub struct CompanionHandle {
    pub port: u16,
    pub local_ip: String,
    shared: Arc<ServerState>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl CompanionHandle {
    pub fn start(
        rt: &tokio::runtime::Handle,
        port: u16,
        devices: &[PairedDevice],
        tx: Sender<AppMsg>,
        ctx: egui::Context,
    ) -> Result<Self, String> {
        let pair_token = random_token(12);
        let mut dev_map = HashMap::new();
        for d in devices {
            dev_map.insert(d.token_hash.clone(), d.id.clone());
        }
        let (events, _) = broadcast::channel(32);
        let shared = Arc::new(ServerState {
            auth: RwLock::new(AuthState { pair_token, devices: dev_map }),
            snapshot: RwLock::new("{}".into()),
            events,
            tx,
            ctx,
        });
        let std_listener = std::net::TcpListener::bind(("0.0.0.0", port))
            .or_else(|_| std::net::TcpListener::bind(("0.0.0.0", 0)))
            .map_err(|e| format!("Could not bind companion port: {e}"))?;
        let bound_port = std_listener.local_addr().map(|a| a.port()).unwrap_or(port);
        std_listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let (sd_tx, mut sd_rx) = tokio::sync::oneshot::channel::<()>();
        let state = shared.clone();
        rt.spawn(async move {
            let Ok(listener) = TcpListener::from_std(std_listener) else { return };
            loop {
                tokio::select! {
                    _ = &mut sd_rx => break,
                    res = listener.accept() => {
                        let Ok((stream, addr)) = res else { continue };
                        let st = state.clone();
                        tokio::spawn(async move {
                            let _ = handle_conn(stream, addr, st).await;
                        });
                    }
                }
            }
        });
        Ok(Self {
            port: bound_port,
            local_ip: detect_lan_ip(),
            shared,
            shutdown: Some(sd_tx),
        })
    }

    pub fn pair_token(&self) -> String {
        self.shared.auth.read().pair_token.clone()
    }

    pub fn regenerate_token(&self) -> String {
        let fresh = random_token(12);
        self.shared.auth.write().pair_token = fresh.clone();
        fresh
    }

    pub fn sync_devices(&self, devices: &[PairedDevice]) {
        let mut auth = self.shared.auth.write();
        auth.devices.clear();
        for d in devices {
            auth.devices.insert(d.token_hash.clone(), d.id.clone());
        }
    }

    pub fn pairing_url(&self, tunnel_url: &str) -> String {
        let base = tunnel_url.trim().trim_end_matches('/');
        let base = if base.is_empty() {
            format!("http://{}:{}", self.local_ip, self.port)
        } else {
            base.to_string()
        };
        format!("{base}/?pair={}", self.pair_token())
    }

    pub fn publish(&self, snap: &CompanionSnapshot) {
        if let Ok(json) = serde_json::to_string(snap) {
            let changed = {
                let mut cur = self.shared.snapshot.write();
                if *cur == json {
                    false
                } else {
                    *cur = json.clone();
                    true
                }
            };
            if changed {
                let _ = self.shared.events.send(json);
            }
        }
    }
}

impl Drop for CompanionHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// SHA-256 hex digest of a raw bearer token (stored on disk instead of plaintext tokens).
pub fn hash_token(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Cryptographically strong random uppercase alphanumeric token (e.g. `K7M9-P2X4-Q8R2`).
pub fn random_token(bytes: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let raw = random_bytes(bytes);
    let mut out = String::with_capacity(bytes + 3);
    for (i, b) in raw.into_iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        out.push(ALPHABET[(b as usize) % ALPHABET.len()] as char);
    }
    out
}

fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    #[cfg(unix)]
    {
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom")
            && f.read_exact(&mut buf).is_ok()
        {
            return buf;
        }
    }
    #[cfg(windows)]
    {
        if getrandom::fill(&mut buf).is_ok() {
            return buf;
        }
    }
    // Fallback mixing high-resolution time & stack entropy.
    let mut h = Sha256::new();
    h.update(format!("{:?}-{}", std::time::SystemTime::now(), std::process::id()).as_bytes());
    let d = h.finalize();
    for (i, b) in buf.iter_mut().enumerate() {
        *b = d[i % d.len()];
    }
    buf
}

/// Best-effort LAN IPv4 address using a non-transmitting UDP route lookup.
pub fn detect_lan_ip() -> String {
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0")
        && sock.connect("8.8.8.8:80").is_ok()
        && let Ok(addr) = sock.local_addr()
        && let IpAddr::V4(ip) = addr.ip()
        && !ip.is_loopback()
    {
        return ip.to_string();
    }
    "127.0.0.1".into()
}

#[derive(Deserialize)]
struct PairReq {
    token: String,
    #[serde(default)]
    device_name: String,
}

#[derive(Deserialize)]
struct InputReq {
    #[serde(default)]
    text: String,
    #[serde(default)]
    enter: bool,
    #[serde(default)]
    action: String,
}

#[derive(Deserialize)]
struct SwitchProfileReq {
    profile_id: String,
}

#[derive(Deserialize)]
struct ResumeReq {
    project_id: ProjectId,
    agent: String,
    session_id: String,
    #[serde(default)]
    profile_id: Option<String>,
}

async fn handle_conn(mut stream: TcpStream, addr: SocketAddr, state: Arc<ServerState>) -> std::io::Result<()> {
    let mut buf = vec![0u8; 32 * 1024];
    let mut filled = 0usize;
    let header_end = loop {
        let n = stream.read(&mut buf[filled..]).await?;
        if n == 0 {
            return Ok(());
        }
        filled += n;
        if let Some(pos) = find_bytes(&buf[..filled], b"\r\n\r\n") {
            break pos + 4;
        }
        if filled >= buf.len() {
            return write_response(&mut stream, 413, "application/json", r#"{"error":"request too large"}"#).await;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = head.lines();
    let req_line = lines.next().unwrap_or("");
    let mut parts = req_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let raw_target = parts.next().unwrap_or("/");
    let (path, query) = raw_target.split_once('?').unwrap_or((raw_target, ""));

    let mut content_len = 0usize;
    let mut auth_header = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_len = v.trim().parse().unwrap_or(0).min(64 * 1024);
            } else if k.eq_ignore_ascii_case("authorization") {
                auth_header = Some(v.trim().to_string());
            }
        }
    }
    let mut body = buf[header_end..filled].to_vec();
    while body.len() < content_len {
        let mut chunk = vec![0u8; (content_len - body.len()).min(8192)];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }

    // Public routes: Web UI shell + Pairing token exchange.
    if method == "GET" && (path == "/" || path == "/index.html") {
        return write_response(&mut stream, 200, "text/html; charset=utf-8", WEB_HTML).await;
    }
    if method == "POST" && path == "/api/pair" {
        let Ok(req) = serde_json::from_slice::<PairReq>(&body) else {
            return write_response(&mut stream, 400, "application/json", r#"{"error":"invalid JSON"}"#).await;
        };
        let expected = state.auth.read().pair_token.clone();
        let norm = |s: &str| s.trim().to_ascii_uppercase().replace('-', "");
        if expected.is_empty() || norm(&req.token) != norm(&expected) {
            return write_response(&mut stream, 403, "application/json", r#"{"error":"Invalid or expired pairing code"}"#).await;
        }
        let raw_device_token = random_token(24);
        let token_hash = hash_token(&raw_device_token);
        let now = chrono::Utc::now().timestamp();
        let dev_id = format!("dev-{}", random_token(6).replace('-', "").to_ascii_lowercase());
        let name = if req.device_name.trim().is_empty() { "Mobile Companion".into() } else { req.device_name.trim().chars().take(48).collect() };
        let device = PairedDevice {
            id: dev_id.clone(),
            name,
            token_hash: token_hash.clone(),
            ip: addr.ip().to_string(),
            paired_at: now,
            last_seen: now,
        };
        state.auth.write().devices.insert(token_hash, dev_id.clone());
        let _ = state.tx.send(AppMsg::Companion(CompanionCmd::DevicePaired(device)));
        state.ctx.request_repaint();
        let resp = serde_json::json!({ "device_id": dev_id, "device_token": raw_device_token }).to_string();
        return write_response(&mut stream, 200, "application/json", &resp).await;
    }

    // Authenticated routes require Bearer token (or ?token= for SSE EventSource).
    let bearer = auth_header
        .as_deref()
        .and_then(|h| h.strip_prefix("Bearer ").or_else(|| h.strip_prefix("bearer ")))
        .map(str::trim)
        .map(str::to_string)
        .or_else(|| query_param(query, "token"));
    let dev_id = bearer.as_deref().and_then(|raw| {
        let h = hash_token(raw);
        state.auth.read().devices.get(&h).cloned()
    });
    let Some(dev_id) = dev_id else {
        return write_response(&mut stream, 401, "application/json", r#"{"error":"unauthorized"}"#).await;
    };
    let _ = state.tx.send(AppMsg::Companion(CompanionCmd::DeviceSeen {
        id: dev_id,
        ip: addr.ip().to_string(),
        at: chrono::Utc::now().timestamp(),
    }));

    match (method, path) {
        ("GET", "/api/state") => {
            let snap = state.snapshot.read().clone();
            write_response(&mut stream, 200, "application/json", &snap).await
        }
        ("GET", "/api/events") => {
            let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
            stream.write_all(headers.as_bytes()).await?;
            let initial = state.snapshot.read().clone();
            stream.write_all(format!("data: {initial}\n\n").as_bytes()).await?;
            stream.flush().await?;
            let mut rx = state.events.subscribe();
            let token_hash = bearer.as_deref().map(hash_token).unwrap_or_default();
            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        // Drop SSE stream immediately if device was revoked.
                        if !state.auth.read().devices.contains_key(&token_hash) {
                            break;
                        }
                        match msg {
                            Ok(payload) => {
                                if stream.write_all(format!("data: {payload}\n\n").as_bytes()).await.is_err() {
                                    break;
                                }
                                let _ = stream.flush().await;
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {
                                let latest = state.snapshot.read().clone();
                                if stream.write_all(format!("data: {latest}\n\n").as_bytes()).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {
                        if !state.auth.read().devices.contains_key(&token_hash) {
                            break;
                        }
                        if stream.write_all(b": keepalive\n\n").await.is_err() {
                            break;
                        }
                    }
                }
            }
            Ok(())
        }
        ("POST", "/api/session/resume") => {
            let Ok(req) = serde_json::from_slice::<ResumeReq>(&body) else {
                return write_response(&mut stream, 400, "application/json", r#"{"error":"invalid JSON"}"#).await;
            };
            let agent = AgentKind::from_slug(&req.agent).unwrap_or(AgentKind::Claude);
            let _ = state.tx.send(AppMsg::Companion(CompanionCmd::ResumeSession {
                project: req.project_id,
                agent,
                session_id: req.session_id,
                profile_id: req.profile_id,
            }));
            state.ctx.request_repaint();
            write_response(&mut stream, 200, "application/json", r#"{"ok":true}"#).await
        }
        ("POST", p) if p.starts_with("/api/pane/") && p.ends_with("/input") => {
            let id_str = &p["/api/pane/".len()..p.len() - "/input".len()];
            let Ok(pane) = id_str.parse::<PaneId>() else {
                return write_response(&mut stream, 400, "application/json", r#"{"error":"bad pane id"}"#).await;
            };
            let Ok(req) = serde_json::from_slice::<InputReq>(&body) else {
                return write_response(&mut stream, 400, "application/json", r#"{"error":"invalid JSON"}"#).await;
            };
            let cmd = match req.action.as_str() {
                "approve" => CompanionCmd::QuickAction { pane, kind: QuickRemoteAction::Approve },
                "reject" => CompanionCmd::QuickAction { pane, kind: QuickRemoteAction::Reject },
                "interrupt" => CompanionCmd::QuickAction { pane, kind: QuickRemoteAction::Interrupt },
                _ => CompanionCmd::SendInput { pane, text: req.text, enter: req.enter },
            };
            let _ = state.tx.send(AppMsg::Companion(cmd));
            state.ctx.request_repaint();
            write_response(&mut stream, 200, "application/json", r#"{"ok":true}"#).await
        }
        ("POST", p) if p.starts_with("/api/pane/") && p.ends_with("/switch-profile") => {
            let id_str = &p["/api/pane/".len()..p.len() - "/switch-profile".len()];
            let Ok(pane) = id_str.parse::<PaneId>() else {
                return write_response(&mut stream, 400, "application/json", r#"{"error":"bad pane id"}"#).await;
            };
            let Ok(req) = serde_json::from_slice::<SwitchProfileReq>(&body) else {
                return write_response(&mut stream, 400, "application/json", r#"{"error":"invalid JSON"}"#).await;
            };
            let _ = state.tx.send(AppMsg::Companion(CompanionCmd::SwitchProfile {
                pane,
                profile_id: req.profile_id,
            }));
            state.ctx.request_repaint();
            write_response(&mut stream, 200, "application/json", r#"{"ok":true}"#).await
        }
        _ => write_response(&mut stream, 404, "application/json", r#"{"error":"not found"}"#).await,
    }
}

async fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.flush().await
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=')
            && k == key
        {
            return Some(percent_decode(v));
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(val) = u8::from_str_radix(hex, 16)
        {
            out.push(val);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn http_req(port: u16, req: &str) -> String {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    #[test]
    fn pairing_auth_and_revoke_flow() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = CompanionHandle::start(rt.handle(), 0, &[], tx, egui::Context::default()).unwrap();
        let port = handle.port;
        handle.publish(&CompanionSnapshot { failover_mode: "auto".into(), ..Default::default() });

        // Unauthenticated state request is rejected with 401.
        let unauth = http_req(port, "GET /api/state HTTP/1.1\r\nHost: localhost\r\n\r\n");
        assert!(unauth.starts_with("HTTP/1.1 401"));

        // Pair with the active pairing token.
        let body = format!(r#"{{"token":"{}","device_name":"Test iPhone"}}"#, handle.pair_token());
        let pair_resp = http_req(
            port,
            &format!("POST /api/pair HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
        );
        assert!(pair_resp.starts_with("HTTP/1.1 200"), "{pair_resp}");
        let json_body = pair_resp.split("\r\n\r\n").nth(1).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(json_body).unwrap();
        let dev_token = parsed["device_token"].as_str().unwrap().to_string();

        // Paired device can read /api/state and send remote commands.
        let ok = http_req(
            port,
            &format!("GET /api/state HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {dev_token}\r\n\r\n"),
        );
        assert!(ok.starts_with("HTTP/1.1 200") && ok.contains("\"failover_mode\":\"auto\""));

        let sw = r#"{"profile_id":"claude2"}"#;
        let sw_resp = http_req(
            port,
            &format!(
                "POST /api/pane/7/switch-profile HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {dev_token}\r\nContent-Length: {}\r\n\r\n{sw}",
                sw.len()
            ),
        );
        assert!(sw_resp.starts_with("HTTP/1.1 200"));
        let msgs: Vec<_> = rx.try_iter().collect();
        assert!(msgs.iter().any(|m| matches!(m, AppMsg::Companion(CompanionCmd::SwitchProfile { pane: 7, profile_id }) if profile_id == "claude2")));

        // Revoking the device immediately invalidates its token.
        handle.sync_devices(&[]);
        let revoked = http_req(
            port,
            &format!("GET /api/state HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {dev_token}\r\n\r\n"),
        );
        assert!(revoked.starts_with("HTTP/1.1 401"));
    }
}
