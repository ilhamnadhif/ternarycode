//! Outbound WebSocket client bridging the local Companion HTTP/SSE server to a
//! Cloudflare Workers + Durable Objects Relay (`relay/src/index.js`).
//!
//! When enabled, Desktop maintains one lightweight outbound TLS WebSocket to
//! `wss://<relay>/r/<host_id>/host`. When no mobile or web client is actively viewing,
//! zero application frames are sent and the Cloudflare Durable Object hibernates at
//! zero compute cost.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

use crate::app::AppMsg;
use crate::companion::{
    CompanionCmd, RelayConfig, RelayStatus, ServerState, dispatch_http_request, hash_token,
};

const RELAY_PING_INTERVAL: Duration = Duration::from_secs(25);
const RELAY_IDLE_READ_TIMEOUT: Duration = Duration::from_secs(75);
const RELAY_SSE_KA_INTERVAL: Duration = Duration::from_secs(20);
const MAX_BACKOFF_SECS: u64 = 15;

#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum InboundMsg {
    Req {
        id: String,
        m: String,
        p: String,
        #[serde(default)]
        q: String,
        #[serde(default)]
        auth: String,
        #[serde(default)]
        ip: String,
        #[serde(default)]
        b: String,
    },
    Sub {
        cid: String,
        token: String,
        #[serde(default)]
        ip: String,
        #[serde(default)]
        sse: bool,
    },
    Unsub {
        cid: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum OutboundMsg {
    Res {
        id: String,
        s: u16,
        ct: String,
        b: String,
    },
    Evt {
        cid: String,
        d: String,
    },
    Bcast {
        cids: Vec<String>,
        d: String,
    },
    Unsub {
        cid: String,
        code: u16,
        reason: String,
    },
    Ka,
}

struct RelaySub {
    token_hash: String,
    sse: bool,
}

/// Converts an `https://` or `http://` relay base URL into the host registration WebSocket URL.
pub(crate) fn build_host_ws_url(cfg: &RelayConfig) -> String {
    let base = cfg.relay_url.trim().trim_end_matches('/');
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("wss://{base}")
    };
    format!("{ws_base}/r/{}/host", cfg.host_id)
}

fn parse_client_ip(raw: &str) -> IpAddr {
    raw.trim()
        .parse::<IpAddr>()
        .unwrap_or(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)))
}

fn set_status(state: &ServerState, status: RelayStatus) {
    *state.relay_status.write() = status;
    state.ctx.request_repaint();
}

/// Long-running task maintaining the outbound Cloudflare Relay WebSocket until shutdown.
pub(crate) async fn run_relay_loop(state: Arc<ServerState>, cfg: RelayConfig) {
    let mut sd_rx = state.shutdown.subscribe();
    let mut backoff_secs = 1u64;

    loop {
        if *sd_rx.borrow() {
            set_status(&state, RelayStatus::Off);
            break;
        }
        set_status(&state, RelayStatus::Connecting);
        let started = Instant::now();

        match run_single_session(&state, &cfg).await {
            Ok(()) => {
                if *sd_rx.borrow() {
                    set_status(&state, RelayStatus::Off);
                    break;
                }
                set_status(&state, RelayStatus::Error("Disconnected — reconnecting…".into()));
            }
            Err(err) => {
                if *sd_rx.borrow() {
                    set_status(&state, RelayStatus::Off);
                    break;
                }
                set_status(&state, RelayStatus::Error(err));
            }
        }

        // Reset backoff quickly if the previous connection stayed healthy for >= 30s.
        if started.elapsed() >= Duration::from_secs(30) {
            backoff_secs = 1;
        } else {
            backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
        }

        tokio::select! {
            _ = sd_rx.changed() => {
                if *sd_rx.borrow() {
                    set_status(&state, RelayStatus::Off);
                    break;
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(backoff_secs)) => {}
        }
    }
}

async fn run_single_session(state: &Arc<ServerState>, cfg: &RelayConfig) -> Result<(), String> {
    let url = build_host_ws_url(cfg);
    let mut req = url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("Invalid relay URL: {e}"))?;
    let secret_val = HeaderValue::from_str(&cfg.secret)
        .map_err(|e| format!("Invalid relay secret header: {e}"))?;
    req.headers_mut().insert("X-Relay-Secret", secret_val);

    let (ws_stream, _) = tokio::time::timeout(
        Duration::from_secs(12),
        tokio_tungstenite::connect_async(req),
    )
    .await
    .map_err(|_| "Relay connection timed out".to_string())?
    .map_err(|e| format!("Relay connect failed: {e}"))?;

    set_status(state, RelayStatus::Connected);

    let (mut ws_write, mut ws_read) = ws_stream.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Message>(128);

    // Writer task serializes all outbound WebSocket frames cleanly.
    let writer_handle = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if tokio::time::timeout(Duration::from_secs(10), ws_write.send(msg))
                .await
                .is_err()
            {
                break;
            }
        }
        let _ = ws_write.close().await;
    });

    let mut subs: HashMap<String, RelaySub> = HashMap::new();
    let mut events_rx = state.events.subscribe();
    let mut sd_rx = state.shutdown.subscribe();
    let mut auth_rx = state.auth_ver.subscribe();
    let mut ping_tick = tokio::time::interval(RELAY_PING_INTERVAL);
    ping_tick.tick().await;
    let mut sse_ka_tick = tokio::time::interval(RELAY_SSE_KA_INTERVAL);
    sse_ka_tick.tick().await;
    let mut last_activity = Instant::now();

    let session_result: Result<(), String> = loop {
        if *sd_rx.borrow() {
            break Ok(());
        }
        tokio::select! {
            _ = sd_rx.changed() => {
                if *sd_rx.borrow() {
                    break Ok(());
                }
            }
            _ = auth_rx.changed() => {
                let mut revoked = Vec::new();
                {
                    let auth = state.auth.read();
                    for (cid, sub) in &subs {
                        if !auth.devices.contains_key(&sub.token_hash) {
                            revoked.push(cid.clone());
                        }
                    }
                }
                for cid in revoked {
                    if subs.remove(&cid).is_some() {
                        state.sse_clients.fetch_sub(1, Ordering::AcqRel);
                        send_json(&out_tx, &OutboundMsg::Unsub {
                            cid,
                            code: 4401,
                            reason: "revoked".into(),
                        });
                    }
                }
            }
            evt = events_rx.recv() => {
                if subs.is_empty() {
                    continue;
                }
                let payload = match evt {
                    Ok(p) => p,
                    Err(broadcast::error::RecvError::Lagged(_)) => state.snapshot.read().clone(),
                    Err(_) => break Ok(()),
                };
                let cids: Vec<String> = subs.keys().cloned().collect();
                send_json(&out_tx, &OutboundMsg::Bcast { cids, d: payload });
            }
            _ = ping_tick.tick() => {
                if last_activity.elapsed() > RELAY_IDLE_READ_TIMEOUT {
                    break Err("Relay heartbeat timed out".into());
                }
                // Text "ping" is answered by Cloudflare's zero-cost Hibernation auto-response "pong".
                if out_tx.try_send(Message::Text("ping".into())).is_err() {
                    break Err("Relay send queue congested".into());
                }
            }
            _ = sse_ka_tick.tick() => {
                if subs.values().any(|s| s.sse) {
                    send_json(&out_tx, &OutboundMsg::Ka);
                }
            }
            frame = ws_read.next() => {
                match frame {
                    Some(Ok(Message::Text(txt))) => {
                        last_activity = Instant::now();
                        let s = txt.as_str();
                        if s == "pong" || s == "ping" {
                            continue;
                        }
                        let Ok(msg) = serde_json::from_str::<InboundMsg>(s) else {
                            continue;
                        };
                        match msg {
                            InboundMsg::Req { id, m, p, q, auth, ip, b } => {
                                let st = state.clone();
                                let tx = out_tx.clone();
                                tokio::spawn(async move {
                                    let auth_opt = if auth.trim().is_empty() { None } else { Some(auth.as_str()) };
                                    let resp = dispatch_http_request(
                                        &st,
                                        &m,
                                        &p,
                                        &q,
                                        auth_opt,
                                        parse_client_ip(&ip),
                                        b.as_bytes(),
                                    ).await;
                                    send_json(&tx, &OutboundMsg::Res {
                                        id,
                                        s: resp.status,
                                        ct: resp.content_type.to_string(),
                                        b: resp.body,
                                    });
                                });
                            }
                            InboundMsg::Sub { cid, token, ip, sse } => {
                                let tok_hash = hash_token(&token);
                                let dev_id = state.auth.read().devices.get(&tok_hash).cloned();
                                let Some(dev_id) = dev_id else {
                                    send_json(&out_tx, &OutboundMsg::Unsub {
                                        cid,
                                        code: 4401,
                                        reason: "unauthorized".into(),
                                    });
                                    continue;
                                };
                                if subs.insert(cid.clone(), RelaySub { token_hash: tok_hash, sse }).is_none() {
                                    state.sse_clients.fetch_add(1, Ordering::AcqRel);
                                }
                                state.touch_client();
                                let _ = state.tx.send(AppMsg::Companion(CompanionCmd::DeviceSeen {
                                    id: dev_id,
                                    ip: if ip.trim().is_empty() { "relay".into() } else { ip },
                                    at: chrono::Utc::now().timestamp(),
                                }));
                                state.ensure_fresh_snapshot().await;
                                let snap = state.snapshot.read().clone();
                                send_json(&out_tx, &OutboundMsg::Evt { cid, d: snap });
                            }
                            InboundMsg::Unsub { cid } => {
                                if subs.remove(&cid).is_some() {
                                    state.sse_clients.fetch_sub(1, Ordering::AcqRel);
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        last_activity = Instant::now();
                        let _ = out_tx.try_send(Message::Pong(payload));
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_activity = Instant::now();
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break Ok(());
                    }
                    Some(Err(e)) => {
                        break Err(format!("Relay read error: {e}"));
                    }
                    Some(Ok(_)) => {
                        last_activity = Instant::now();
                    }
                }
            }
        }
    };

    // Always release subscriber counts accurately when the relay socket drops.
    if !subs.is_empty() {
        state.sse_clients.fetch_sub(subs.len(), Ordering::AcqRel);
        subs.clear();
    }
    drop(out_tx);
    let _ = writer_handle.await;
    session_result
}

fn send_json<T: Serialize>(tx: &mpsc::Sender<Message>, val: &T) {
    if let Ok(json) = serde_json::to_string(val) {
        let _ = tx.try_send(Message::Text(json.into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::companion::{CompanionHandle, CompanionSnapshot};
    use tokio::net::TcpListener;

    #[test]
    fn host_ws_url_converts_schemes_cleanly() {
        let https_cfg = RelayConfig {
            relay_url: "https://ternarycode-relay.example.workers.dev/".into(),
            host_id: "tc1234567890abcd".into(),
            secret: "secret-token-12345678".into(),
        };
        assert_eq!(
            build_host_ws_url(&https_cfg),
            "wss://ternarycode-relay.example.workers.dev/r/tc1234567890abcd/host"
        );

        let http_cfg = RelayConfig {
            relay_url: "http://127.0.0.1:8787".into(),
            host_id: "tc1234567890abcd".into(),
            secret: "secret-token-12345678".into(),
        };
        assert_eq!(
            build_host_ws_url(&http_cfg),
            "ws://127.0.0.1:8787/r/tc1234567890abcd/host"
        );
    }

    #[test]
    #[allow(clippy::result_large_err)]
    fn relay_client_proxies_pair_state_subscribe_and_revoke() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();

        let (server_port, server_task) = rt.block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let handle = tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.unwrap();
                let mut got_secret = String::new();
                let ws = tokio_tungstenite::accept_hdr_async(
                    tcp,
                    |req: &tokio_tungstenite::tungstenite::handshake::server::Request, resp| {
                        if let Some(v) = req.headers().get("x-relay-secret") {
                            got_secret = v.to_str().unwrap_or("").to_string();
                        }
                        Ok(resp)
                    },
                )
                .await
                .unwrap();
                (ws, got_secret)
            });
            (port, handle)
        });

        let (tx, _rx) = std::sync::mpsc::channel();
        let cfg = RelayConfig {
            relay_url: format!("http://127.0.0.1:{server_port}"),
            host_id: "tcrelaytest12345".into(),
            secret: "test-relay-secret-abcdef123456".into(),
        };
        let companion = CompanionHandle::start(
            rt.handle(),
            0,
            &[],
            Some(cfg),
            tx,
            eframe::egui::Context::default(),
        )
        .unwrap();
        companion.publish(&CompanionSnapshot {
            host_os: "Mac".into(),
            failover_mode: "auto".into(),
            ..Default::default()
        });

        let pair_token = companion.pair_token();

        rt.block_on(async {
            let (mut ws, got_secret) = tokio::time::timeout(Duration::from_secs(3), server_task)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(got_secret, "test-relay-secret-abcdef123456");

            // 1. Pair via relayed HTTP request.
            let pair_req = serde_json::json!({
                "t": "req",
                "id": "r-pair",
                "m": "POST",
                "p": "/api/pair",
                "q": "",
                "auth": "",
                "ip": "203.0.113.5",
                "b": serde_json::json!({"token": pair_token, "device_name": "Relay Phone"}).to_string(),
            });
            ws.send(Message::Text(pair_req.to_string().into())).await.unwrap();
            let raw_res = tokio::time::timeout(Duration::from_secs(2), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let res_v: serde_json::Value = serde_json::from_str(raw_res.to_text().unwrap()).unwrap();
            assert_eq!(res_v["t"], "res");
            assert_eq!(res_v["id"], "r-pair");
            assert_eq!(res_v["s"], 200);
            let pair_body: serde_json::Value = serde_json::from_str(res_v["b"].as_str().unwrap()).unwrap();
            let dev_id = pair_body["device_id"].as_str().unwrap().to_string();
            let dev_token = pair_body["device_token"].as_str().unwrap().to_string();

            // 2. Subscribe client over relay -> receives immediate initial snapshot & active flag flips true.
            ws.send(Message::Text(
                serde_json::json!({
                    "t": "sub",
                    "cid": "client-1",
                    "token": dev_token,
                    "ip": "203.0.113.5",
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
            let evt_raw = tokio::time::timeout(Duration::from_secs(2), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let evt_v: serde_json::Value = serde_json::from_str(evt_raw.to_text().unwrap()).unwrap();
            assert_eq!(evt_v["t"], "evt");
            assert_eq!(evt_v["cid"], "client-1");
            assert!(evt_v["d"].as_str().unwrap().contains("\"host_os\":\"Mac\""));
            assert!(companion.has_active_clients());

            // 3. Publishing an updated snapshot broadcasts once over relay.
            companion.publish(&CompanionSnapshot {
                host_os: "Mac".into(),
                failover_mode: "manual".into(),
                ..Default::default()
            });
            let bc_raw = tokio::time::timeout(Duration::from_secs(2), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let bc_v: serde_json::Value = serde_json::from_str(bc_raw.to_text().unwrap()).unwrap();
            assert_eq!(bc_v["t"], "bcast");
            assert!(bc_v["d"].as_str().unwrap().contains("\"failover_mode\":\"manual\""));

            // 4. Revoking the device immediately emits an unsub(4401) frame and clears client count.
            companion.revoke_device(&dev_id);
            let unsub_raw = tokio::time::timeout(Duration::from_secs(2), ws.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let unsub_v: serde_json::Value = serde_json::from_str(unsub_raw.to_text().unwrap()).unwrap();
            assert_eq!(unsub_v["t"], "unsub");
            assert_eq!(unsub_v["cid"], "client-1");
            assert_eq!(unsub_v["code"], 4401);
        });
    }
}
