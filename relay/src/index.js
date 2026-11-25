// TernaryCode Companion Relay — Cloudflare Workers + SQLite Durable Objects
// with WebSocket Hibernation API (100% Free-Tier compatible, 0 GB-s idle cost).
//
// Topology:
//   Desktop  ──(outbound WSS)──▶  /r/:hostId/host   (authenticated via X-Relay-Secret)
//   Mobile/Web ──(HTTPS/WSS)───▶  /r/:hostId/*      (proxied over Host WS; auth verified on Desktop)
//
// Security:
//   - First connection for a fresh random 80-bit `hostId` persists SHA-256(secret) in DO SQLite.
//   - Every subsequent host reconnect must match that constant-time SHA-256 hash.
//   - All companion pairing codes & Bearer device tokens are verified exclusively on Desktop.
//   - Zero logging of tokens, pairing codes, or terminal payloads.

const HOST_ID_RE = /^[a-z0-9][a-z0-9_-]{7,47}$/;
const MAX_BODY_BYTES = 64 * 1024;
const MAX_CLIENT_SOCKETS = 32;
const HTTP_TIMEOUT_MS = 12_000;

const SECURITY_HEADERS = {
  "Cache-Control": "no-store",
  "X-Content-Type-Options": "nosniff",
  "X-Frame-Options": "DENY",
  "Referrer-Policy": "no-referrer",
};

function jsonResponse(status, obj, extraHeaders = {}) {
  return new Response(JSON.stringify(obj), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      ...SECURITY_HEADERS,
      ...extraHeaders,
    },
  });
}

function offlineHtmlPage() {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta name="theme-color" content="#09090B">
<title>TernaryCode · Computer Offline</title>
<style>
  *{box-sizing:border-box}
  html,body{margin:0;height:100%;background:#09090B;color:#EDEDF0;font-family:-apple-system,BlinkMacSystemFont,"Inter","Segoe UI",system-ui,sans-serif;display:flex;align-items:center;justify-content:center;padding:24px}
  .box{max-width:340px;text-align:center;display:flex;flex-direction:column;align-items:center;gap:12px}
  .tile{width:52px;height:52px;border-radius:14px;background:#1A1A1D;border:1px solid rgba(255,255,255,.1);display:flex;align-items:center;justify-content:center;color:#F2B14B}
  h1{margin:0;font-size:17px;font-weight:600;letter-spacing:-.2px}
  p{margin:0;font-size:13px;line-height:19px;color:#A0A0A9}
  button{margin-top:6px;height:40px;padding:0 18px;border-radius:10px;border:1px solid rgba(255,255,255,.14);background:#2E6BE6;color:#fff;font-size:13px;font-weight:500;cursor:pointer}
  button:active{opacity:.85}
  .meta{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:11px;color:#6C6C75}
</style>
</head>
<body>
<div class="box">
  <div class="tile" aria-hidden="true">
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <line x1="2" y1="2" x2="22" y2="22"/><path d="M8.5 16.5a5 5 0 0 1 7 0"/><path d="M2 8.82a15 15 0 0 1 4.17-2.65"/><path d="M10.66 5c4.01-.36 8.14.9 11.34 3.76"/><path d="M16.85 11.25a10 10 0 0 1 2.22 1.68"/><path d="M5 13a10 10 0 0 1 5.24-2.76"/><line x1="12" y1="20" x2="12.01" y2="20"/>
    </svg>
  </div>
  <h1>Computer is offline</h1>
  <p>Make sure TernaryCode is open on your computer with Remote Access enabled in Settings &rarr; Phone.</p>
  <button onclick="location.reload()">Retry now</button>
  <div class="meta" id="cd">Auto-retrying in 5s&hellip;</div>
</div>
<script>
  let n = 5;
  const el = document.getElementById("cd");
  setInterval(() => {
    n -= 1;
    if (n <= 0) location.reload();
    else if (el) el.textContent = "Auto-retrying in " + n + "s…";
  }, 1000);
</script>
</body>
</html>`;
}

async function sha256Hex(text) {
  const bytes = new TextEncoder().encode(text);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

function constantTimeEq(a, b) {
  if (typeof a !== "string" || typeof b !== "string" || a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i += 1) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
}

export function parseRelayRoute(pathname) {
  if (pathname === "/" || pathname === "/health") {
    return { kind: "health" };
  }
  const m = pathname.match(/^\/r\/([^/]+)(\/.*)?$/);
  if (!m) {
    return { kind: "not_found" };
  }
  const hostId = m[1];
  if (!HOST_ID_RE.test(hostId)) {
    return { kind: "invalid_host" };
  }
  const subPath = m[2] && m[2].length > 0 ? m[2] : "/";
  return { kind: "room", hostId, subPath };
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const route = parseRelayRoute(url.pathname);

    if (route.kind === "health") {
      return jsonResponse(200, {
        ok: true,
        service: "ternarycode-relay",
        protocol: 1,
      });
    }
    if (route.kind === "invalid_host") {
      return jsonResponse(400, { error: "Invalid relay host identifier." });
    }
    if (route.kind !== "room") {
      return jsonResponse(404, { error: "Not found." });
    }

    // Reject oversized request bodies at the edge before waking the Durable Object.
    const rawLen = request.headers.get("content-length");
    if (rawLen !== null) {
      const len = Number(rawLen);
      if (!Number.isFinite(len) || len < 0) {
        return jsonResponse(400, { error: "Invalid Content-Length." });
      }
      if (len > MAX_BODY_BYTES) {
        return jsonResponse(413, { error: "Payload too large." });
      }
    }

    const id = env.RELAY_ROOM.idFromName(route.hostId);
    const stub = env.RELAY_ROOM.get(id);

    const headers = new Headers(request.headers);
    headers.set("X-Relay-Subpath", route.subPath);
    const cfIp = request.headers.get("cf-connecting-ip");
    if (cfIp) {
      headers.set("X-Relay-Client-IP", cfIp);
    }

    const forwardedUrl = new URL(request.url);
    forwardedUrl.pathname = route.subPath;
    const forwarded = new Request(forwardedUrl.toString(), {
      method: request.method,
      headers,
      body: request.method === "GET" || request.method === "HEAD" ? null : request.body,
    });
    return stub.fetch(forwarded);
  },
};

export class RelayRoom {
  constructor(ctx, env) {
    this.ctx = ctx;
    this.env = env;
    this.pendingHttp = new Map(); // reqId -> { resolve, reject, timer }
    this.sseStreams = new Map(); // cid -> { writer, encoder, closed }

    // Ensure SQLite metadata table exists for constant-time host secret verification.
    this.ctx.blockConcurrencyWhile(async () => {
      this.ctx.storage.sql.exec(
        "CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL)"
      );
    });

    // Zero-cost heartbeat while hibernated: "ping" -> "pong" never wakes the DO.
    if (
      typeof this.ctx.setWebSocketAutoResponse === "function" &&
      typeof WebSocketRequestResponsePair === "function"
    ) {
      this.ctx.setWebSocketAutoResponse(
        new WebSocketRequestResponsePair("ping", "pong")
      );
    }
  }

  _getMeta(key) {
    const cur = this.ctx.storage.sql.exec("SELECT v FROM meta WHERE k = ?", key);
    for (const row of cur) {
      return row.v;
    }
    return null;
  }

  _setMeta(key, value) {
    this.ctx.storage.sql.exec(
      "INSERT OR REPLACE INTO meta (k, v) VALUES (?, ?)",
      key,
      value
    );
  }

  _activeHostSocket() {
    const hosts = this.ctx.getWebSockets("host");
    return hosts.length > 0 ? hosts[hosts.length - 1] : null;
  }

  _sendToHost(payload) {
    const host = this._activeHostSocket();
    if (!host) return false;
    try {
      host.send(typeof payload === "string" ? payload : JSON.stringify(payload));
      return true;
    } catch {
      return false;
    }
  }

  _closeSse(cid) {
    const entry = this.sseStreams.get(cid);
    if (!entry) return;
    this.sseStreams.delete(cid);
    if (!entry.closed) {
      entry.closed = true;
      entry.writer.close().catch(() => {});
    }
  }

  _writeSse(cid, chunk) {
    const entry = this.sseStreams.get(cid);
    if (!entry || entry.closed) return false;
    entry.writer.write(entry.encoder.encode(chunk)).catch(() => {
      this._closeSse(cid);
      this._sendToHost({ t: "unsub", cid });
    });
    return true;
  }

  async fetch(request) {
    const url = new URL(request.url);
    const subPath = request.headers.get("X-Relay-Subpath") || url.pathname || "/";
    const clientIp =
      request.headers.get("X-Relay-Client-IP") ||
      request.headers.get("cf-connecting-ip") ||
      "relay";

    // 1. Desktop host WebSocket registration.
    if (subPath === "/host") {
      return this._handleHostConnect(request, url);
    }

    // 2. Mobile / Web companion real-time WebSocket stream (hibernatable).
    if (subPath === "/api/ws") {
      return this._handleClientWs(request, url, clientIp);
    }

    // 3. Backwards-compatible SSE stream over relay.
    if (request.method === "GET" && subPath === "/api/events") {
      return this._handleClientSse(request, url, clientIp);
    }

    // 4. Status probe for monitoring/tests.
    if (request.method === "GET" && subPath === "/_relay/status") {
      const hostOnline = this._activeHostSocket() !== null;
      const clientCount =
        this.ctx.getWebSockets("client").length + this.sseStreams.size;
      return jsonResponse(200, { online: hostOnline, clients: clientCount });
    }

    // 5. All regular Companion HTTP endpoints (GET /, POST /api/pair, GET /api/state, POST /api/...).
    return this._handleHttpProxy(request, url, subPath, clientIp);
  }

  async _handleHostConnect(request, url) {
    if ((request.headers.get("upgrade") || "").toLowerCase() !== "websocket") {
      return jsonResponse(426, { error: "WebSocket upgrade required." });
    }
    const auth = request.headers.get("authorization") || "";
    const bearer = auth.toLowerCase().startsWith("bearer ")
      ? auth.slice(7).trim()
      : "";
    const secret = (
      request.headers.get("x-relay-secret") ||
      bearer ||
      url.searchParams.get("secret") ||
      ""
    ).trim();

    if (secret.length < 16) {
      return jsonResponse(401, { error: "Missing or weak relay secret." });
    }

    const incomingHash = await sha256Hex(secret);
    const storedHash = this._getMeta("secret_hash");
    if (storedHash === null) {
      this._setMeta("secret_hash", incomingHash);
    } else if (!constantTimeEq(storedHash, incomingHash)) {
      return jsonResponse(403, { error: "Invalid relay secret for host." });
    }

    // Replace any stale host socket cleanly.
    for (const oldHost of this.ctx.getWebSockets("host")) {
      try {
        oldHost.close(1000, "replaced");
      } catch {}
    }

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    this.ctx.acceptWebSocket(server, ["host"]);
    server.serializeAttachment({ role: "host", connectedAt: Date.now() });

    // Re-subscribe any already-waiting client WebSockets after a host reconnect.
    for (const cws of this.ctx.getWebSockets("client")) {
      const att = cws.deserializeAttachment();
      if (att && att.cid && att.token) {
        try {
          server.send(
            JSON.stringify({
              t: "sub",
              cid: att.cid,
              token: att.token,
              ip: att.ip || "relay",
            })
          );
        } catch {}
      }
    }

    return new Response(null, { status: 101, webSocket: client });
  }

  _extractBearer(request, url) {
    const auth = request.headers.get("authorization") || "";
    if (auth.toLowerCase().startsWith("bearer ")) {
      const tok = auth.slice(7).trim();
      if (tok) return tok;
    }
    const q = (url.searchParams.get("token") || "").trim();
    return q || null;
  }

  async _handleClientWs(request, url, clientIp) {
    if ((request.headers.get("upgrade") || "").toLowerCase() !== "websocket") {
      return jsonResponse(426, { error: "WebSocket upgrade required." });
    }
    const token = this._extractBearer(request, url);
    if (!token) {
      return jsonResponse(401, { error: "unauthorized" });
    }
    if (!this._activeHostSocket()) {
      return jsonResponse(503, {
        error: "Computer is offline. Make sure TernaryCode is running.",
      });
    }
    if (this.ctx.getWebSockets("client").length >= MAX_CLIENT_SOCKETS) {
      return jsonResponse(429, { error: "Too many active companion connections." });
    }

    const cid = crypto.randomUUID();
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    this.ctx.acceptWebSocket(server, ["client", `cid:${cid}`]);
    server.serializeAttachment({
      role: "client",
      cid,
      token,
      ip: clientIp,
    });

    if (!this._sendToHost({ t: "sub", cid, token, ip: clientIp })) {
      try {
        server.close(1013, "host offline");
      } catch {}
      return jsonResponse(503, { error: "Computer is offline." });
    }

    return new Response(null, { status: 101, webSocket: client });
  }

  async _handleClientSse(request, url, clientIp) {
    const token = this._extractBearer(request, url);
    if (!token) {
      return jsonResponse(401, { error: "unauthorized" });
    }
    if (!this._activeHostSocket()) {
      return jsonResponse(503, {
        error: "Computer is offline. Make sure TernaryCode is running.",
      });
    }
    if (this.sseStreams.size >= MAX_CLIENT_SOCKETS) {
      return jsonResponse(429, { error: "Too many active streams." });
    }

    const cid = `sse-${crypto.randomUUID()}`;
    const { readable, writable } = new TransformStream();
    const writer = writable.getWriter();
    const encoder = new TextEncoder();
    this.sseStreams.set(cid, { writer, encoder, closed: false });

    if (request.signal) {
      request.signal.addEventListener("abort", () => {
        this._closeSse(cid);
        this._sendToHost({ t: "unsub", cid });
      });
    }

    if (!this._sendToHost({ t: "sub", cid, sse: true, token, ip: clientIp })) {
      this._closeSse(cid);
      return jsonResponse(503, { error: "Computer is offline." });
    }

    // Flush an initial comment immediately so HTTP clients observe headers right away.
    this._writeSse(cid, ": connected\n\n");

    return new Response(readable, {
      status: 200,
      headers: {
        "Content-Type": "text/event-stream; charset=utf-8",
        "Cache-Control": "no-cache, no-store",
        Connection: "keep-alive",
        "X-Accel-Buffering": "no",
      },
    });
  }

  async _handleHttpProxy(request, url, subPath, clientIp) {
    const host = this._activeHostSocket();
    if (!host) {
      if (
        request.method === "GET" &&
        (subPath === "/" || subPath === "/index.html")
      ) {
        return new Response(offlineHtmlPage(), {
          status: 503,
          headers: {
            "Content-Type": "text/html; charset=utf-8",
            ...SECURITY_HEADERS,
          },
        });
      }
      return jsonResponse(503, {
        error: "Computer is offline. Make sure TernaryCode is running.",
      });
    }

    let bodyText = "";
    if (request.method !== "GET" && request.method !== "HEAD") {
      const buf = await request.arrayBuffer();
      if (buf.byteLength > MAX_BODY_BYTES) {
        return jsonResponse(413, { error: "Payload too large." });
      }
      bodyText = new TextDecoder().decode(buf);
    }

    const reqId = crypto.randomUUID();
    const authHeader = request.headers.get("authorization") || "";
    const query = url.search.startsWith("?") ? url.search.slice(1) : "";

    const responsePromise = new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pendingHttp.delete(reqId);
        reject(new Error("timeout"));
      }, HTTP_TIMEOUT_MS);
      this.pendingHttp.set(reqId, { resolve, reject, timer });
    });

    const sent = this._sendToHost({
      t: "req",
      id: reqId,
      m: request.method,
      p: subPath,
      q: query,
      auth: authHeader,
      ip: clientIp,
      b: bodyText,
    });

    if (!sent) {
      const p = this.pendingHttp.get(reqId);
      if (p) {
        clearTimeout(p.timer);
        this.pendingHttp.delete(reqId);
      }
      return jsonResponse(503, { error: "Computer disconnected." });
    }

    try {
      const res = await responsePromise;
      const status = Number.isInteger(res.s) ? res.s : 200;
      const contentType =
        typeof res.ct === "string" && res.ct
          ? res.ct
          : "application/json; charset=utf-8";
      const body = typeof res.b === "string" ? res.b : "";
      return new Response(body, {
        status,
        headers: {
          "Content-Type": contentType,
          ...SECURITY_HEADERS,
        },
      });
    } catch (err) {
      if (err && err.message === "timeout") {
        return jsonResponse(504, {
          error: "Computer took too long to respond.",
        });
      }
      return jsonResponse(503, { error: "Computer disconnected." });
    }
  }

  _deliverToClient(cid, snapshotJson) {
    if (this.sseStreams.has(cid)) {
      this._writeSse(cid, `data: ${snapshotJson}\n\n`);
      return;
    }
    const sockets = this.ctx.getWebSockets(`cid:${cid}`);
    const frame = JSON.stringify({ t: "state", d: snapshotJson });
    for (const sock of sockets) {
      try {
        sock.send(frame);
      } catch {
        try {
          sock.close(1011, "send failed");
        } catch {}
      }
    }
  }

  async webSocketMessage(ws, message) {
    if (typeof message !== "string") return;
    if (message === "ping") {
      try {
        ws.send("pong");
      } catch {}
      return;
    }

    const att = ws.deserializeAttachment() || {};
    let msg;
    try {
      msg = JSON.parse(message);
    } catch {
      return;
    }
    if (!msg || typeof msg !== "object") return;

    if (att.role === "host") {
      if (msg.t === "res" && typeof msg.id === "string") {
        const entry = this.pendingHttp.get(msg.id);
        if (entry) {
          clearTimeout(entry.timer);
          this.pendingHttp.delete(msg.id);
          entry.resolve(msg);
        }
        return;
      }

      if (msg.t === "evt" && typeof msg.cid === "string" && typeof msg.d === "string") {
        this._deliverToClient(msg.cid, msg.d);
        return;
      }

      if (msg.t === "bcast" && typeof msg.d === "string") {
        if (Array.isArray(msg.cids)) {
          for (const cid of msg.cids) {
            if (typeof cid === "string") {
              this._deliverToClient(cid, msg.d);
            }
          }
        } else {
          const frame = JSON.stringify({ t: "state", d: msg.d });
          for (const cws of this.ctx.getWebSockets("client")) {
            try {
              cws.send(frame);
            } catch {}
          }
          for (const cid of Array.from(this.sseStreams.keys())) {
            this._writeSse(cid, `data: ${msg.d}\n\n`);
          }
        }
        return;
      }

      if (msg.t === "unsub" && typeof msg.cid === "string") {
        const code = Number.isInteger(msg.code) ? msg.code : 4401;
        const reason = typeof msg.reason === "string" ? msg.reason : "unauthorized";
        if (this.sseStreams.has(msg.cid)) {
          this._closeSse(msg.cid);
        }
        for (const cws of this.ctx.getWebSockets(`cid:${msg.cid}`)) {
          try {
            cws.close(code, reason);
          } catch {}
        }
        return;
      }

      if (msg.t === "ka") {
        for (const cid of Array.from(this.sseStreams.keys())) {
          this._writeSse(cid, ": keepalive\n\n");
        }
      }
      return;
    }

    if (att.role === "client") {
      if (msg.t === "ping") {
        try {
          ws.send(JSON.stringify({ t: "pong" }));
        } catch {}
      }
    }
  }

  async webSocketClose(ws) {
    this._handleSocketClosed(ws);
  }

  async webSocketError(ws) {
    this._handleSocketClosed(ws);
  }

  _handleSocketClosed(ws) {
    const att = ws.deserializeAttachment() || {};
    if (att.role === "host") {
      // Only treat host as offline if no replacement host socket is active.
      const remaining = this.ctx.getWebSockets("host").filter((s) => s !== ws);
      if (remaining.length > 0) return;

      for (const [reqId, entry] of this.pendingHttp.entries()) {
        clearTimeout(entry.timer);
        this.pendingHttp.delete(reqId);
        entry.reject(new Error("host disconnected"));
      }
      for (const cid of Array.from(this.sseStreams.keys())) {
        this._closeSse(cid);
      }
      for (const cws of this.ctx.getWebSockets("client")) {
        try {
          cws.send(JSON.stringify({ t: "offline" }));
        } catch {}
      }
    } else if (att.role === "client" && att.cid) {
      this._sendToHost({ t: "unsub", cid: att.cid });
    }
  }
}
