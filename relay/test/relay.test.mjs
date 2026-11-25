import test from "node:test";
import assert from "node:assert/strict";
import worker, { RelayRoom, parseRelayRoute } from "../src/index.js";

// Lightweight in-memory Cloudflare Durable Object + WebSocketPair harness for deep unit tests.
class MockSocket {
  constructor() {
    this.peer = null;
    this.sent = [];
    this.closed = null;
    this.attachment = null;
  }
  send(data) {
    if (this.closed) throw new Error("socket closed");
    this.sent.push(data);
    if (this.peer && this.peer.onPeerMessage) {
      this.peer.onPeerMessage(data);
    }
  }
  close(code = 1000, reason = "") {
    this.closed = { code, reason };
    if (this.peer && !this.peer.closed) {
      this.peer.closed = { code, reason };
      if (this.peer.onPeerClose) this.peer.onPeerClose(code, reason);
    }
  }
  serializeAttachment(obj) {
    this.attachment = structuredClone(obj);
  }
  deserializeAttachment() {
    return this.attachment;
  }
}

class MockWebSocketPair {
  constructor() {
    const a = new MockSocket();
    const b = new MockSocket();
    a.peer = b;
    b.peer = a;
    this[0] = a;
    this[1] = b;
  }
}

class MockSql {
  constructor() {
    this.meta = new Map();
  }
  exec(query, ...params) {
    const q = query.trim().toUpperCase();
    if (q.startsWith("CREATE TABLE")) return [];
    if (q.startsWith("SELECT V FROM META")) {
      const val = this.meta.get(params[0]);
      return val !== undefined ? [{ v: val }] : [];
    }
    if (q.startsWith("INSERT OR REPLACE INTO META")) {
      this.meta.set(params[0], params[1]);
      return [];
    }
    throw new Error("Unsupported SQL in mock: " + query);
  }
}

class MockCtx {
  constructor() {
    this.storage = { sql: new MockSql() };
    this.sockets = []; // { ws, tags }
    this.autoResponse = null;
  }
  blockConcurrencyWhile(fn) {
    return fn();
  }
  setWebSocketAutoResponse(pair) {
    this.autoResponse = pair;
  }
  acceptWebSocket(ws, tags = []) {
    this.sockets.push({ ws, tags });
  }
  getWebSockets(tag) {
    return this.sockets
      .filter((e) => !e.ws.closed && (!tag || e.tags.includes(tag)))
      .map((e) => e.ws);
  }
}

// Inject Cloudflare runtime globals for tests.
globalThis.WebSocketPair = MockWebSocketPair;
globalThis.WebSocketRequestResponsePair = class {
  constructor(req, res) {
    this.request = req;
    this.response = res;
  }
};
const OrigResponse = globalThis.Response;
globalThis.Response = class PatchedResponse extends OrigResponse {
  constructor(body, init) {
    if (init && init.status === 101 && init.webSocket) {
      super(null, { status: 200 });
      this._wsStatus = 101;
      this.webSocket = init.webSocket;
      return;
    }
    super(body, init);
  }
  get status() {
    return this._wsStatus ?? super.status;
  }
};

function makeRoom() {
  const ctx = new MockCtx();
  const room = new RelayRoom(ctx, {});
  return { ctx, room };
}

test("parseRelayRoute validates host IDs and subpaths", () => {
  assert.deepEqual(parseRelayRoute("/"), { kind: "health" });
  assert.deepEqual(parseRelayRoute("/health"), { kind: "health" });
  assert.deepEqual(parseRelayRoute("/r/tc9a8b7c6d5e"), {
    kind: "room",
    hostId: "tc9a8b7c6d5e",
    subPath: "/",
  });
  assert.deepEqual(parseRelayRoute("/r/tc9a8b7c6d5e/api/state"), {
    kind: "room",
    hostId: "tc9a8b7c6d5e",
    subPath: "/api/state",
  });
  assert.deepEqual(parseRelayRoute("/r/bad!host/api/state"), {
    kind: "invalid_host",
  });
  assert.deepEqual(parseRelayRoute("/r/short"), { kind: "invalid_host" });
  assert.deepEqual(parseRelayRoute("/other"), { kind: "not_found" });
});

test("edge worker handles health and oversized payload rejection", async () => {
  const health = await worker.fetch(new Request("https://relay.test/health"), {});
  assert.equal(health.status, 200);
  const body = await health.json();
  assert.equal(body.ok, true);

  const big = await worker.fetch(
    new Request("https://relay.test/r/tc9a8b7c6d5e/api/pair", {
      method: "POST",
      headers: { "Content-Length": "999999" },
      body: "x",
    }),
    {}
  );
  assert.equal(big.status, 413);
});

test("host secret claim & constant-time hijack protection", async () => {
  const { room } = makeRoom();

  // Weak secret rejected.
  const weak = await room.fetch(
    new Request("https://relay.test/host", {
      headers: { Upgrade: "websocket", "X-Relay-Secret": "short" },
    })
  );
  assert.equal(weak.status, 401);

  // First valid connect claims the host room.
  const first = await room.fetch(
    new Request("https://relay.test/host", {
      headers: {
        Upgrade: "websocket",
        "X-Relay-Secret": "super-secret-host-token-123456",
      },
    })
  );
  assert.equal(first.status, 101);

  // Wrong secret is rejected with 403.
  const hijack = await room.fetch(
    new Request("https://relay.test/host", {
      headers: {
        Upgrade: "websocket",
        "X-Relay-Secret": "attacker-wrong-secret-12345678",
      },
    })
  );
  assert.equal(hijack.status, 403);

  // Same secret replaces old host cleanly.
  const second = await room.fetch(
    new Request("https://relay.test/host", {
      headers: {
        Upgrade: "websocket",
        "X-Relay-Secret": "super-secret-host-token-123456",
      },
    })
  );
  assert.equal(second.status, 101);
  assert.equal(first.webSocket.closed.reason, "replaced");
});

test("offline host returns Slate HTML for GET / and 503 JSON for API", async () => {
  const { room } = makeRoom();
  const html = await room.fetch(
    new Request("https://relay.test/", {
      headers: { "X-Relay-Subpath": "/" },
    })
  );
  assert.equal(html.status, 503);
  assert.match(await html.text(), /Computer is offline/);

  const api = await room.fetch(
    new Request("https://relay.test/api/state", {
      headers: { "X-Relay-Subpath": "/api/state" },
    })
  );
  assert.equal(api.status, 503);
  const j = await api.json();
  assert.match(j.error, /offline/i);
});

test("HTTP proxy over host WebSocket & client WS broadcast + revoke", async () => {
  const { ctx, room } = makeRoom();

  // Connect host.
  const hostRes = await room.fetch(
    new Request("https://relay.test/host", {
      headers: {
        Upgrade: "websocket",
        "X-Relay-Secret": "super-secret-host-token-abcdef",
      },
    })
  );
  assert.equal(hostRes.status, 101);
  const hostServerWs = ctx.getWebSockets("host")[0];

  // Auto-reply to HTTP proxy requests coming to the host socket.
  hostRes.webSocket.onPeerMessage = (raw) => {
    const msg = JSON.parse(raw);
    if (msg.t === "req") {
      room.webSocketMessage(
        hostServerWs,
        JSON.stringify({
          t: "res",
          id: msg.id,
          s: 200,
          ct: "application/json",
          b: JSON.stringify({ echoedPath: msg.p, auth: msg.auth, ip: msg.ip }),
        })
      );
    }
  };

  // Execute proxied HTTP GET /api/state.
  const apiRes = await room.fetch(
    new Request("https://relay.test/api/state", {
      headers: {
        "X-Relay-Subpath": "/api/state",
        "X-Relay-Client-IP": "203.0.113.9",
        Authorization: "Bearer dev-tok-1",
      },
    })
  );
  assert.equal(apiRes.status, 200);
  const apiJson = await apiRes.json();
  assert.deepEqual(apiJson, {
    echoedPath: "/api/state",
    auth: "Bearer dev-tok-1",
    ip: "203.0.113.9",
  });

  // Connect a companion client WebSocket.
  const clientRes = await room.fetch(
    new Request("https://relay.test/api/ws?token=dev-tok-1", {
      headers: {
        Upgrade: "websocket",
        "X-Relay-Subpath": "/api/ws",
        "X-Relay-Client-IP": "203.0.113.9",
      },
    })
  );
  assert.equal(clientRes.status, 101);
  const subMsg = JSON.parse(hostServerWs.sent[hostServerWs.sent.length - 1]);
  assert.equal(subMsg.t, "sub");
  assert.equal(subMsg.token, "dev-tok-1");

  // Broadcast snapshot from host reaches client WS.
  await room.webSocketMessage(
    hostServerWs,
    JSON.stringify({
      t: "bcast",
      cids: [subMsg.cid],
      d: JSON.stringify({ host_os: "Mac", panes: [] }),
    })
  );
  const clientServerWs = ctx.getWebSockets(`cid:${subMsg.cid}`)[0];
  const delivered = JSON.parse(clientServerWs.sent[0]);
  assert.equal(delivered.t, "state");
  assert.match(delivered.d, /"host_os":"Mac"/);

  // Host revoking device immediately closes client WS with 4401.
  await room.webSocketMessage(
    hostServerWs,
    JSON.stringify({ t: "unsub", cid: subMsg.cid, code: 4401, reason: "revoked" })
  );
  assert.deepEqual(clientRes.webSocket.closed, { code: 4401, reason: "revoked" });
});
