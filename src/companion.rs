//! Embedded local HTTP/1.1 + Server-Sent Events server for Phone Pairing and Mobile Companion
//! remote monitoring & control (zero extra web-framework dependencies).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use eframe::egui;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tc_core::agent::AgentKind;
use tc_core::model::PairedDevice;
use tc_core::{PaneId, ProjectId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, broadcast, watch};

use crate::app::AppMsg;

const WEB_HTML: &str = include_str!("companion_web.html");
const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_CONCURRENT_CONNS: usize = 64;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const PAIR_RATE_WINDOW: Duration = Duration::from_secs(60);
const PAIR_MAX_FAILURES: usize = 5;

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
    pub host_os: String,
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
    pub provider: Option<String>,
    pub profile: Option<String>,
    pub supports_profiles: bool,
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
    pub resets_at_unix: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompanionWindowQuota {
    pub label: String,
    pub used_pct: Option<f32>,
    pub tokens: u64,
    pub resets_at_unix: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompanionProfile {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub account_hint: Option<String>,
    pub plan_type: Option<String>,
    pub signed_in: Option<bool>,
    pub state: String,
    pub enabled: bool,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub quota_pct: Option<f32>,
    pub resets_at_unix: Option<i64>,
    pub window_5h: Option<CompanionWindowQuota>,
    pub window_weekly: Option<CompanionWindowQuota>,
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
    pair_failures: RwLock<HashMap<IpAddr, Vec<Instant>>>,
    snapshot: RwLock<String>,
    events: broadcast::Sender<String>,
    shutdown: watch::Sender<bool>,
    auth_ver: watch::Sender<u64>,
    tx: Sender<AppMsg>,
    ctx: egui::Context,
}

impl ServerState {
    fn bump_auth(&self) {
        let next = self.auth_ver.borrow().wrapping_add(1);
        let _ = self.auth_ver.send(next);
    }
}

/// Handle to a running embedded Mobile Companion server.
pub struct CompanionHandle {
    pub port: u16,
    pub local_ip: String,
    shared: Arc<ServerState>,
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
        let (shutdown, mut sd_rx) = watch::channel(false);
        let (auth_ver, _) = watch::channel(0u64);
        let shared = Arc::new(ServerState {
            auth: RwLock::new(AuthState { pair_token, devices: dev_map }),
            pair_failures: RwLock::new(HashMap::new()),
            snapshot: RwLock::new("{}".into()),
            events,
            shutdown,
            auth_ver,
            tx,
            ctx,
        });
        let mut attempts = 0u8;
        let std_listener = loop {
            match std::net::TcpListener::bind(("0.0.0.0", port)) {
                Ok(l) => break l,
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && attempts < 5 => {
                    // Previous server task may still be releasing its listener after a rapid toggle.
                    attempts += 1;
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => return Err(format!("Could not bind companion port {port}: {e}")),
            }
        };
        let bound_port = std_listener.local_addr().map(|a| a.port()).unwrap_or(port);
        std_listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let state = shared.clone();
        let sem = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNS));
        rt.spawn(async move {
            let Ok(listener) = TcpListener::from_std(std_listener) else { return };
            loop {
                tokio::select! {
                    _ = sd_rx.changed() => {
                        if *sd_rx.borrow() {
                            break;
                        }
                    }
                    res = listener.accept() => {
                        let Ok((stream, addr)) = res else { continue };
                        let Ok(permit) = sem.clone().try_acquire_owned() else {
                            drop(stream);
                            continue;
                        };
                        let st = state.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
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
        {
            let mut auth = self.shared.auth.write();
            auth.devices.clear();
            for d in devices {
                auth.devices.insert(d.token_hash.clone(), d.id.clone());
            }
        }
        self.shared.bump_auth();
    }

    /// Revokes a single device by ID without disturbing concurrently paired devices,
    /// immediately terminating any open SSE streams using its token.
    pub fn revoke_device(&self, device_id: &str) {
        {
            let mut auth = self.shared.auth.write();
            auth.devices.retain(|_, id| id != device_id);
        }
        self.shared.bump_auth();
    }

    pub fn pairing_url(&self, tunnel_url: &str) -> String {
        let base = tunnel_url.trim().trim_end_matches('/');
        let base = if base.is_empty() {
            format!("http://{}:{}", self.local_ip, self.port)
        } else {
            base.to_string()
        };
        let sep = if base.contains('?') { "&pair=" } else { "/?pair=" };
        format!("{base}{sep}{}", self.pair_token())
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
        let _ = self.shared.shutdown.send(true);
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

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
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

/// Best-effort LAN IPv4 address using non-transmitting UDP route lookups (including private RFC1918
/// probes so offline / air-gapped Wi-Fi networks still resolve the local interface).
pub fn detect_lan_ip() -> String {
    for target in ["8.8.8.8:80", "192.168.0.1:80", "10.0.0.1:80", "172.16.0.1:80"] {
        if let Ok(sock) = UdpSocket::bind("0.0.0.0:0")
            && sock.connect(target).is_ok()
            && let Ok(addr) = sock.local_addr()
            && let IpAddr::V4(ip) = addr.ip()
            && !ip.is_loopback()
            && !ip.is_unspecified()
        {
            return ip.to_string();
        }
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

fn check_pair_rate_limit(state: &ServerState, ip: IpAddr) -> bool {
    let now = Instant::now();
    let mut map = state.pair_failures.write();
    let list = map.entry(ip).or_default();
    list.retain(|t| now.duration_since(*t) < PAIR_RATE_WINDOW);
    list.len() < PAIR_MAX_FAILURES
}

fn record_pair_failure(state: &ServerState, ip: IpAddr) {
    let now = Instant::now();
    let mut map = state.pair_failures.write();
    // Bound map size against spoofed/many IPs.
    if map.len() > 256 {
        map.retain(|_, v| {
            v.retain(|t| now.duration_since(*t) < PAIR_RATE_WINDOW);
            !v.is_empty()
        });
    }
    map.entry(ip).or_default().push(now);
}

fn clear_pair_failures(state: &ServerState, ip: IpAddr) {
    state.pair_failures.write().remove(&ip);
}

async fn read_chunked_body(stream: &mut TcpStream, initial: &[u8]) -> Result<Vec<u8>, u16> {
    let mut raw = initial.to_vec();
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        while find_bytes(&raw[pos..], b"\r\n").is_none() {
            let mut tmp = [0u8; 4096];
            let n = match tokio::time::timeout(IO_TIMEOUT, stream.read(&mut tmp)).await {
                Ok(Ok(0)) => return Err(400),
                Ok(Ok(n)) => n,
                _ => return Err(408),
            };
            raw.extend_from_slice(&tmp[..n]);
            if raw.len() > MAX_BODY_BYTES + 4096 {
                return Err(413);
            }
        }
        let rel = find_bytes(&raw[pos..], b"\r\n").ok_or(400u16)?;
        let size_line = std::str::from_utf8(&raw[pos..pos + rel]).map_err(|_| 400u16)?;
        let hex = size_line.split(';').next().unwrap_or("").trim();
        let chunk_len = usize::from_str_radix(hex, 16).map_err(|_| 400u16)?;
        pos += rel + 2;
        if chunk_len == 0 {
            return Ok(out);
        }
        if out.len().saturating_add(chunk_len) > MAX_BODY_BYTES {
            return Err(413);
        }
        let need = pos.saturating_add(chunk_len).saturating_add(2);
        while raw.len() < need {
            let mut tmp = [0u8; 4096];
            let n = match tokio::time::timeout(IO_TIMEOUT, stream.read(&mut tmp)).await {
                Ok(Ok(0)) => return Err(400),
                Ok(Ok(n)) => n,
                _ => return Err(408),
            };
            raw.extend_from_slice(&tmp[..n]);
            if raw.len() > MAX_BODY_BYTES + 4096 {
                return Err(413);
            }
        }
        if &raw[pos + chunk_len..pos + chunk_len + 2] != b"\r\n" {
            return Err(400);
        }
        out.extend_from_slice(&raw[pos..pos + chunk_len]);
        pos += chunk_len + 2;
    }
}

async fn handle_conn(mut stream: TcpStream, addr: SocketAddr, state: Arc<ServerState>) -> std::io::Result<()> {
    let mut buf = vec![0u8; 32 * 1024];
    let mut filled = 0usize;
    let header_end = loop {
        let n = match tokio::time::timeout(IO_TIMEOUT, stream.read(&mut buf[filled..])).await {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Ok(()),
        };
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
    let Some(method) = parts.next() else {
        return write_response(&mut stream, 400, "application/json", r#"{"error":"malformed request"}"#).await;
    };
    let Some(raw_target) = parts.next() else {
        return write_response(&mut stream, 400, "application/json", r#"{"error":"malformed request"}"#).await;
    };
    let (path, query) = raw_target.split_once('?').unwrap_or((raw_target, ""));

    let mut content_len: Option<usize> = None;
    let mut chunked = false;
    let mut expect_continue = false;
    let mut auth_header = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim();
            if k.eq_ignore_ascii_case("content-length") {
                let Ok(len) = v.parse::<usize>() else {
                    return write_response(&mut stream, 400, "application/json", r#"{"error":"invalid content-length"}"#).await;
                };
                if len > MAX_BODY_BYTES {
                    return write_response(&mut stream, 413, "application/json", r#"{"error":"payload too large"}"#).await;
                }
                content_len = Some(len);
            } else if k.eq_ignore_ascii_case("transfer-encoding") {
                if v.to_ascii_lowercase().contains("chunked") {
                    chunked = true;
                }
            } else if k.eq_ignore_ascii_case("expect") {
                if v.eq_ignore_ascii_case("100-continue") {
                    expect_continue = true;
                }
            } else if k.eq_ignore_ascii_case("authorization") {
                auth_header = Some(v.to_string());
            }
        }
    }

    if expect_continue {
        let _ = tokio::time::timeout(IO_TIMEOUT, stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")).await;
    }

    let body = if chunked {
        match read_chunked_body(&mut stream, &buf[header_end..filled]).await {
            Ok(b) => b,
            Err(code) => return write_response(&mut stream, code, "application/json", r#"{"error":"invalid chunked body"}"#).await,
        }
    } else if let Some(expected_len) = content_len {
        let mut body = buf[header_end..filled].to_vec();
        while body.len() < expected_len {
            let mut chunk = vec![0u8; (expected_len - body.len()).min(8192)];
            let n = match tokio::time::timeout(IO_TIMEOUT, stream.read(&mut chunk)).await {
                Ok(Ok(n)) => n,
                Ok(Err(e)) => return Err(e),
                Err(_) => return write_response(&mut stream, 408, "application/json", r#"{"error":"request timeout"}"#).await,
            };
            if n == 0 {
                return write_response(&mut stream, 400, "application/json", r#"{"error":"incomplete request body"}"#).await;
            }
            body.extend_from_slice(&chunk[..n]);
        }
        body.truncate(expected_len);
        body
    } else {
        Vec::new()
    };

    // Public routes: Web UI shell + Pairing token exchange.
    if method == "GET" && (path == "/" || path == "/index.html") {
        return write_response(&mut stream, 200, "text/html; charset=utf-8", WEB_HTML).await;
    }
    if method == "POST" && path == "/api/pair" {
        if !check_pair_rate_limit(&state, addr.ip()) {
            return write_response(
                &mut stream,
                429,
                "application/json",
                r#"{"error":"Too many pairing attempts. Wait a minute and try again."}"#,
            )
            .await;
        }
        let Ok(req) = serde_json::from_slice::<PairReq>(&body) else {
            return write_response(&mut stream, 400, "application/json", r#"{"error":"invalid JSON"}"#).await;
        };
        let expected = state.auth.read().pair_token.clone();
        let norm = |s: &str| s.trim().to_ascii_uppercase().replace('-', "");
        let got_hash = Sha256::digest(norm(&req.token).as_bytes());
        let exp_hash = Sha256::digest(norm(&expected).as_bytes());
        if expected.is_empty() || !ct_eq(&got_hash, &exp_hash) {
            record_pair_failure(&state, addr.ip());
            return write_response(&mut stream, 403, "application/json", r#"{"error":"Invalid or expired pairing code"}"#).await;
        }
        clear_pair_failures(&state, addr.ip());
        let raw_device_token = random_token(24);
        let token_hash = hash_token(&raw_device_token);
        let now = chrono::Utc::now().timestamp();
        let dev_id = format!("dev-{}", random_token(6).replace('-', "").to_ascii_lowercase());
        let name = if req.device_name.trim().is_empty() {
            "Mobile Companion".into()
        } else {
            req.device_name.trim().chars().take(48).collect()
        };
        let device = PairedDevice {
            id: dev_id.clone(),
            name,
            token_hash: token_hash.clone(),
            ip: addr.ip().to_string(),
            paired_at: now,
            last_seen: now,
        };
        state.auth.write().devices.insert(token_hash, dev_id.clone());
        state.bump_auth();
        let _ = state.tx.send(AppMsg::Companion(CompanionCmd::DevicePaired(device)));
        state.ctx.request_repaint();
        let resp = serde_json::json!({ "device_id": dev_id, "device_token": raw_device_token }).to_string();
        return write_response(&mut stream, 200, "application/json", &resp).await;
    }

    // Authenticated routes require Bearer token; `?token=` query auth is restricted exclusively to SSE EventSource.
    let header_bearer = auth_header.as_deref().and_then(|h| {
        let (scheme, tok) = h.split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then(|| tok.trim().to_string()).filter(|t| !t.is_empty())
    });
    let bearer = header_bearer.or_else(|| {
        if method == "GET" && path == "/api/events" {
            query_param(query, "token").filter(|t| !t.is_empty())
        } else {
            None
        }
    });
    let token_hash = bearer.as_deref().map(hash_token);
    let dev_id = token_hash.as_deref().and_then(|h| state.auth.read().devices.get(h).cloned());
    let (Some(dev_id), Some(token_hash)) = (dev_id, token_hash) else {
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
            if tokio::time::timeout(IO_TIMEOUT, stream.write_all(headers.as_bytes())).await.is_err() {
                return Ok(());
            }
            let initial = state.snapshot.read().clone();
            if tokio::time::timeout(IO_TIMEOUT, stream.write_all(format!("data: {initial}\n\n").as_bytes())).await.is_err() {
                return Ok(());
            }
            let _ = tokio::time::timeout(IO_TIMEOUT, stream.flush()).await;
            let mut rx = state.events.subscribe();
            let mut sd_rx = state.shutdown.subscribe();
            let mut auth_rx = state.auth_ver.subscribe();
            loop {
                if *sd_rx.borrow() || !state.auth.read().devices.contains_key(&token_hash) {
                    break;
                }
                tokio::select! {
                    _ = sd_rx.changed() => {
                        if *sd_rx.borrow() {
                            break;
                        }
                    }
                    _ = auth_rx.changed() => {
                        if !state.auth.read().devices.contains_key(&token_hash) {
                            break;
                        }
                    }
                    msg = rx.recv() => {
                        if !state.auth.read().devices.contains_key(&token_hash) {
                            break;
                        }
                        match msg {
                            Ok(payload) => {
                                if tokio::time::timeout(IO_TIMEOUT, stream.write_all(format!("data: {payload}\n\n").as_bytes())).await.is_err() {
                                    break;
                                }
                                let _ = tokio::time::timeout(IO_TIMEOUT, stream.flush()).await;
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {
                                let latest = state.snapshot.read().clone();
                                if tokio::time::timeout(IO_TIMEOUT, stream.write_all(format!("data: {latest}\n\n").as_bytes())).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(15)) => {
                        if !state.auth.read().devices.contains_key(&token_hash) {
                            break;
                        }
                        if tokio::time::timeout(IO_TIMEOUT, stream.write_all(b": keepalive\n\n")).await.is_err() {
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
            let Some(agent) = AgentKind::from_slug(&req.agent) else {
                return write_response(&mut stream, 400, "application/json", r#"{"error":"unknown agent"}"#).await;
            };
            if !tc_agent::sessions::valid_session_id(&req.session_id) || req.session_id.starts_with("pane-") {
                return write_response(&mut stream, 400, "application/json", r#"{"error":"invalid session id"}"#).await;
            }
            let _ = state.tx.send(AppMsg::Companion(CompanionCmd::ResumeSession {
                project: req.project_id,
                agent,
                session_id: req.session_id,
                profile_id: req.profile_id,
            }));
            state.ctx.request_repaint();
            write_response(&mut stream, 200, "application/json", r#"{"ok":true}"#).await
        }
        ("POST", p) if p.strip_prefix("/api/pane/").and_then(|s| s.strip_suffix("/input")).is_some() => {
            let id_str = p.strip_prefix("/api/pane/").and_then(|s| s.strip_suffix("/input")).unwrap_or("");
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
                "" => CompanionCmd::SendInput { pane, text: req.text, enter: req.enter },
                _ => return write_response(&mut stream, 400, "application/json", r#"{"error":"unknown action"}"#).await,
            };
            let _ = state.tx.send(AppMsg::Companion(cmd));
            state.ctx.request_repaint();
            write_response(&mut stream, 200, "application/json", r#"{"ok":true}"#).await
        }
        ("POST", p) if p.strip_prefix("/api/pane/").and_then(|s| s.strip_suffix("/switch-profile")).is_some() => {
            let id_str = p.strip_prefix("/api/pane/").and_then(|s| s.strip_suffix("/switch-profile")).unwrap_or("");
            let Ok(pane) = id_str.parse::<PaneId>() else {
                return write_response(&mut stream, 400, "application/json", r#"{"error":"bad pane id"}"#).await;
            };
            let Ok(req) = serde_json::from_slice::<SwitchProfileReq>(&body) else {
                return write_response(&mut stream, 400, "application/json", r#"{"error":"invalid JSON"}"#).await;
            };
            if req.profile_id.trim().is_empty() {
                return write_response(&mut stream, 400, "application/json", r#"{"error":"empty profile id"}"#).await;
            }
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
        408 => "Request Timeout",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = tokio::time::timeout(IO_TIMEOUT, async {
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(body.as_bytes()).await?;
        stream.flush().await
    })
    .await;
    Ok(())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
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
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
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
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
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
        let dev_id = parsed["device_id"].as_str().unwrap().to_string();
        let dev_token = parsed["device_token"].as_str().unwrap().to_string();

        // Query token is rejected on non-SSE endpoints.
        let q_reject = http_req(port, &format!("GET /api/state?token={dev_token} HTTP/1.1\r\nHost: localhost\r\n\r\n"));
        assert!(q_reject.starts_with("HTTP/1.1 401"), "{q_reject}");

        // Paired device can read /api/state with Bearer header (case-insensitive) and send commands.
        let ok = http_req(
            port,
            &format!("GET /api/state HTTP/1.1\r\nHost: localhost\r\nAuthorization: BEARER {dev_token}\r\n\r\n"),
        );
        assert!(ok.starts_with("HTTP/1.1 200") && ok.contains("\"failover_mode\":\"auto\""));

        // Overlapping path `/api/pane/input` must return 404 (NEVER panic on slice bounds!).
        let no_panic = http_req(
            port,
            &format!("POST /api/pane/input HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {dev_token}\r\nContent-Length: 2\r\n\r\n{{}}"),
        );
        assert!(no_panic.starts_with("HTTP/1.1 404"), "{no_panic}");

        // Unknown quick action returns 400.
        let bad_act = r#"{"action":"rm_rf"}"#;
        let bad_resp = http_req(
            port,
            &format!(
                "POST /api/pane/7/input HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {dev_token}\r\nContent-Length: {}\r\n\r\n{bad_act}",
                bad_act.len()
            ),
        );
        assert!(bad_resp.starts_with("HTTP/1.1 400"), "{bad_resp}");

        // Chunked transfer encoding works cleanly.
        let sw = r#"{"profile_id":"claude2"}"#;
        let chunked_body = format!("{:x}\r\n{sw}\r\n0\r\n\r\n", sw.len());
        let sw_resp = http_req(
            port,
            &format!(
                "POST /api/pane/7/switch-profile HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {dev_token}\r\nTransfer-Encoding: chunked\r\n\r\n{chunked_body}",
            ),
        );
        assert!(sw_resp.starts_with("HTTP/1.1 200"), "{sw_resp}");
        let msgs: Vec<_> = rx.try_iter().collect();
        assert!(msgs.iter().any(|m| matches!(m, AppMsg::Companion(CompanionCmd::SwitchProfile { pane: 7, profile_id }) if profile_id == "claude2")));

        // Revoking the device immediately closes active SSE streams and invalidates its token.
        let mut sse = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        sse.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        sse.write_all(format!("GET /api/events?token={dev_token} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes()).unwrap();
        let mut first = [0u8; 256];
        let n = sse.read(&mut first).unwrap();
        assert!(String::from_utf8_lossy(&first[..n]).contains("text/event-stream"));

        handle.revoke_device(&dev_id);
        let mut rest = Vec::new();
        let _ = sse.read_to_end(&mut rest); // Must return promptly on revoke, not wait 15s!

        let revoked = http_req(
            port,
            &format!("GET /api/state HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {dev_token}\r\n\r\n"),
        );
        assert!(revoked.starts_with("HTTP/1.1 401"));
    }

    #[test]
    fn pair_rate_limit_blocks_brute_force() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let handle = CompanionHandle::start(rt.handle(), 0, &[], tx, egui::Context::default()).unwrap();
        let port = handle.port;
        let bad = r#"{"token":"WRONG-CODE","device_name":"Bot"}"#;
        for _ in 0..PAIR_MAX_FAILURES {
            let r = http_req(
                port,
                &format!("POST /api/pair HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{bad}", bad.len()),
            );
            assert!(r.starts_with("HTTP/1.1 403"), "{r}");
        }
        let blocked = http_req(
            port,
            &format!("POST /api/pair HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{bad}", bad.len()),
        );
        assert!(blocked.starts_with("HTTP/1.1 429"), "{blocked}");
    }
}
