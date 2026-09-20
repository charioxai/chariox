import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import { createServer } from "node:net";

import {
  BrowserController,
  ControllerProtocol,
  ERROR_CODES,
  redactDiagnostic,
} from "./browser-controller.mjs";
import { OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES } from "./browser-controller-observations.mjs";

class FakeClock {
  constructor() {
    this.value = 0;
  }

  now() {
    return this.value;
  }

  advance(milliseconds) {
    this.value += milliseconds;
  }
}

function makeTimers(clock) {
  let nextId = 1;
  const intervals = new Map();
  return {
    setTimeout() {
      return nextId++;
    },
    clearTimeout() {},
    setInterval(callback) {
      const id = nextId++;
      intervals.set(id, callback);
      return id;
    },
    clearInterval(id) {
      intervals.delete(id);
    },
    sleep(milliseconds) {
      clock.advance(milliseconds);
      return Promise.resolve();
    },
    tickIntervals() {
      for (const callback of intervals.values()) {
        callback();
      }
    },
  };
}

class FakeProcess extends EventEmitter {
  constructor() {
    super();
    this.stdout = { resume() {} };
    this.stderr = { resume() {} };
    this.kills = [];
    this.exitOnSignals = new Set();
    this.autoExitOnBrowserClose = false;
  }

  kill(signal) {
    this.kills.push(signal);
    if (this.exitOnSignals.has(signal)) {
      this.exit(0, signal);
    }
    return true;
  }

  exit(code = 0, signal = null) {
    this.emit("exit", code, signal);
  }
}

class FakeWebSocket extends EventEmitter {
  static instances = [];
  static onSend = null;

  constructor(url, protocols = [], options) {
    super();
    if (protocols && typeof protocols === "object" && !Array.isArray(protocols) && options === undefined) {
      options = protocols;
      protocols = [];
    }
    options = options ?? {};
    this.url = url;
    this.protocols = protocols;
    this.options = options;
    this.maxPayload = Number.isSafeInteger(options?.maxPayload)
      ? options.maxPayload
      : Number.MAX_SAFE_INTEGER;
    this.readyState = 0;
    this.sent = [];
    this.closed = false;
    this.incomingFragments = [];
    this.incomingBytes = 0;
    this.receivedMessages = 0;
    this.process = FakeWebSocket.currentProcess;
    FakeWebSocket.instances.push(this);
    queueMicrotask(() => this.open());
  }

  addEventListener(eventName, listener) {
    this.on(eventName, listener);
  }

  removeEventListener(eventName, listener) {
    this.off(eventName, listener);
  }

  open() {
    if (this.closed) {
      return;
    }
    this.readyState = 1;
    this.emit("open", {});
  }

  send(encoded) {
    const message = JSON.parse(encoded);
    this.sent.push(message);
    this.constructor.onSend?.(this, message);
  }

  respond(id, result = {}) {
    this.emit("message", { data: JSON.stringify({ id, result }) });
  }

  receiveFragment(data, { final = false } = {}) {
    const fragment = data instanceof Uint8Array ? data : new Uint8Array(data);
    if (this.incomingBytes + fragment.byteLength > this.maxPayload) {
      this.close();
      return false;
    }
    this.incomingFragments.push(fragment);
    this.incomingBytes += fragment.byteLength;
    if (final) {
      const payload = Buffer.concat(this.incomingFragments.map((part) => Buffer.from(part)));
      this.incomingFragments = [];
      this.incomingBytes = 0;
      this.receivedMessages += 1;
      this.emit("message", { data: payload });
    }
    return true;
  }

  close() {
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.readyState = 3;
    this.emit("close", {});
  }
}

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

function encodeServerFrame(opcode, payload, final = true) {
  const body = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);
  const headerLength = body.length < 126 ? 2 : body.length <= 0xffff ? 4 : 10;
  const header = Buffer.alloc(headerLength);
  header[0] = (final ? 0x80 : 0) | opcode;
  if (body.length < 126) {
    header[1] = body.length;
  } else if (body.length <= 0xffff) {
    header[1] = 126;
    header.writeUInt16BE(body.length, 2);
  } else {
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(body.length), 2);
  }
  return Buffer.concat([header, body]);
}

function clientCloseCode(buffer) {
  let offset = 0;
  while (offset + 2 <= buffer.length) {
    const first = buffer[offset];
    const second = buffer[offset + 1];
    const masked = (second & 0x80) !== 0;
    let length = second & 0x7f;
    let headerLength = 2;
    if (length === 126) {
      if (offset + 4 > buffer.length) return null;
      length = buffer.readUInt16BE(offset + 2);
      headerLength = 4;
    } else if (length === 127) {
      if (offset + 10 > buffer.length) return null;
      const wideLength = buffer.readBigUInt64BE(offset + 2);
      if (wideLength > BigInt(Number.MAX_SAFE_INTEGER)) return null;
      length = Number(wideLength);
      headerLength = 10;
    }
    const maskLength = masked ? 4 : 0;
    const frameLength = headerLength + maskLength + length;
    if (offset + frameLength > buffer.length) return null;
    if ((first & 0x0f) === 0x8) {
      const payloadOffset = offset + headerLength + maskLength;
      if (length < 2) return 1005;
      if (!masked) return buffer.readUInt16BE(payloadOffset);
      const maskOffset = offset + headerLength;
      return ((buffer[payloadOffset] ^ buffer[maskOffset]) << 8) |
        (buffer[payloadOffset + 1] ^ buffer[maskOffset + 1]);
    }
    offset += frameLength;
  }
  return null;
}

async function startFragmentedWebSocketServer() {
  const server = createServer();
  let client = null;
  let handshakeComplete = false;
  let commandResponded = false;
  let incoming = Buffer.alloc(0);
  let resolveCommand;
  let resolveCloseCode;
  const commandReceived = new Promise((resolve) => {
    resolveCommand = resolve;
  });
  const closeCode = new Promise((resolve) => {
    resolveCloseCode = resolve;
  });

  server.on("connection", (socket) => {
    client = socket;
    socket.on("data", (chunk) => {
      incoming = Buffer.concat([incoming, chunk]);
      if (!handshakeComplete) {
        const boundary = incoming.indexOf(Buffer.from("\r\n\r\n"));
        if (boundary < 0) return;
        const request = incoming.subarray(0, boundary).toString("latin1");
        const key = request.match(/^Sec-WebSocket-Key:\s*(.+)$/im)?.[1]?.trim();
        if (!key) {
          socket.destroy();
          return;
        }
        const accept = createHash("sha1")
          .update(key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11")
          .digest("base64");
        socket.write(
          "HTTP/1.1 101 Switching Protocols\r\n" +
          "Upgrade: websocket\r\n" +
          "Connection: Upgrade\r\n" +
          `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
        );
        incoming = incoming.subarray(boundary + 4);
        handshakeComplete = true;
      }
      if (handshakeComplete && !commandResponded && incoming.length > 0) {
        commandResponded = true;
        socket.write(encodeServerFrame(0x1, JSON.stringify({ id: 1, result: {} })));
        resolveCommand();
      }
      if (handshakeComplete) {
        const code = clientCloseCode(incoming);
        if (code !== null) resolveCloseCode(code);
      }
    });
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject);
      resolve();
    });
  });
  const address = server.address();
  assert.ok(address && typeof address === "object");
  return {
    url: `ws://127.0.0.1:${address.port}`,
    commandReceived,
    closeCode,
    sendOversizedPayload() {
      assert.ok(client && handshakeComplete && commandResponded);
      client.write(
        encodeServerFrame(
          0x1,
          Buffer.alloc(OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES - 1, 0x78),
          false,
        ),
      );
      client.write(encodeServerFrame(0x0, Buffer.from("xx"), true));
    },
    async close() {
      client?.destroy();
      await new Promise((resolve) => server.close(() => resolve()));
    },
  };
}

async function waitFor(predicate, timeoutMs = 2000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  assert.ok(predicate(), "condition did not become true before timeout");
}

function makeFixture(options = {}) {
  const clock = new FakeClock();
  const timers = makeTimers(clock);
  const processes = [];
  let currentProcess = null;
  FakeWebSocket.instances = [];
  FakeWebSocket.currentProcess = null;
  FakeWebSocket.onSend = (socket, message) => {
    if (message.method === "Browser.close") {
      queueMicrotask(() => {
        socket.respond(message.id);
        if (socket.process?.autoExitOnBrowserClose) {
          socket.process.exit(0, null);
        }
      });
      return;
    }
    if (!socket.holdResponses) {
      queueMicrotask(() => socket.respond(message.id));
    }
  };

  const fetchImpl = options.fetchImpl || (async () => ({
    ok: true,
    text: async () => JSON.stringify(
      options.targetMode === "none"
        ? []
        : options.targets || [{
          id: "page-1",
          type: "page",
          webSocketDebuggerUrl: "ws://127.0.0.1:9222/devtools/page/1",
        }],
    ),
  }));
  const spawnBrowser = async () => {
    currentProcess = new FakeProcess();
    currentProcess.autoExitOnBrowserClose = Boolean(options.autoExitOnBrowserClose);
    processes.push(currentProcess);
    FakeWebSocket.currentProcess = currentProcess;
    return currentProcess;
  };
  const signals = [];
  const signalProcess = (child, signal) => {
    signals.push(signal);
    child.kill(signal);
  };
  const controller = new BrowserController({
    controllerId: options.controllerId || "controller-test",
    clock,
    timers,
    fetchImpl,
    WebSocketImpl: FakeWebSocket,
    spawnBrowser,
    signalProcess,
    startupTimeoutMs: options.startupTimeoutMs || 3,
    pollIntervalMs: options.pollIntervalMs || 1,
    cdpConnectTimeoutMs: 3,
    cdpCommandTimeoutMs: 3,
    shutdownGraceMs: 1,
    terminateGraceMs: 1,
    killGraceMs: 1,
    heartbeatIntervalMs: 1,
    queueLimit: options.queueLimit || 8,
  });
  return {
    controller,
    clock,
    timers,
    processes,
    signals,
    sockets: FakeWebSocket.instances,
    get currentProcess() {
      return currentProcess;
    },
  };
}

test("admits one owner and preserves one controller identity", async (t) => {
  const fixture = makeFixture();
  const { controller } = fixture;
  const ready = await controller.start("owner-a");
  assert.equal(ready.controller_id, "controller-test");
  assert.equal(ready.owner_id, "owner-a");
  assert.equal(ready.generation, 1);
  assert.throws(
    () => controller.health("owner-b"),
    (error) => error.code === ERROR_CODES.OWNER_MISMATCH,
  );
  t.after(async () => {
    await controller.shutdown("owner-a", 1);
  });
});

test("reports a fatal startup timeout and kills the owned browser", async (t) => {
  const fixture = makeFixture({ targetMode: "none" });
  const { controller, signals } = fixture;
  await assert.rejects(
    controller.start("owner-a"),
    (error) => error.code === ERROR_CODES.STARTUP_TIMEOUT,
  );
  assert.equal(controller.health("owner-a").state, "fatal");
  assert.equal(controller.health("owner-a").fatal_code, ERROR_CODES.STARTUP_TIMEOUT);
  assert.deepEqual(signals, ["SIGKILL"]);
  t.after(async () => {
    await controller.shutdownForSignal();
  });
});

test("emits ready state and keeps heartbeat monotonic", async (t) => {
  const fixture = makeFixture();
  const { controller, clock, timers } = fixture;
  const events = [];
  controller.onEvent((event) => events.push(event));
  const ready = await controller.start("owner-a");
  const first = controller.health("owner-a", ready.generation);
  clock.advance(5);
  timers.tickIntervals();
  const second = controller.health("owner-a", ready.generation);
  clock.value = 2;
  const third = controller.health("owner-a", ready.generation);
  assert.equal(events.some((event) => event.event === "ready"), true);
  assert.equal(second.heartbeat_seq > first.heartbeat_seq, true);
  assert.equal(third.last_heartbeat_ms >= second.last_heartbeat_ms, true);
  t.after(async () => {
    await controller.shutdown("owner-a", 1);
  });
});

test("integrates CDP refresh and reconnect reconciliation with stable tab identities", async (t) => {
  let targets = [
    { id: "page-a", type: "page", webSocketDebuggerUrl: "ws://127.0.0.1:9222/devtools/page/a" },
    { id: "page-b", type: "page", webSocketDebuggerUrl: "ws://127.0.0.1:9222/devtools/page/b" },
  ];
  const fixture = makeFixture({ targets });
  const { controller } = fixture;
  await controller.start("owner-a");
  const initial = controller.getTabRegistrySnapshot("owner-a", 1);
  const tabIds = new Map(initial.tabs.map((tab) => [tab.target_id, tab.tab_id]));
  assert.deepEqual([...tabIds.keys()], ["page-a", "page-b"]);

  targets.splice(0, targets.length, targets[1], targets[0]);
  const refreshed = await controller.refreshTabs("owner-a", 1);
  assert.deepEqual(refreshed.tabs.map((tab) => tab.tab_id), [tabIds.get("page-a"), tabIds.get("page-b")]);

  targets.push(targets[0]);
  const reconnected = await controller.refreshTabs("owner-a", 1, { reconnect: true });
  assert.equal(reconnected.duplicates_suppressed, 1);
  assert.deepEqual(reconnected.tabs.map((tab) => tab.tab_id), [tabIds.get("page-a"), tabIds.get("page-b")]);

  targets.splice(0, targets.length, targets[0]);
  const detached = await controller.refreshTabs("owner-a", 1);
  assert.deepEqual(detached.detached.map((tab) => tab.tab_id), [tabIds.get("page-a")]);
  assert.throws(
    () => controller.tabRegistry.getTab(tabIds.get("page-a"), { generation: 1 }),
    (error) => error.code === "TAB_INVALIDATED",
  );
  assert.equal(controller.getTabRegistrySnapshot("owner-a", 1).tabs[0].tab_id, tabIds.get("page-b"));

  const claimed = controller.claimViewport("owner-a", 1, { owner_id: "owner-a", version: 0 });
  assert.deepEqual(claimed, {
    width: 1280,
    height: 800,
    owner_id: "owner-a",
    version: 1,
    changed: true,
    idempotent: false,
  });
  const resized = controller.resizeViewport("owner-a", 1, {
    owner_id: "owner-a",
    version: claimed.version,
    width: 1440,
    height: 900,
  });
  assert.equal(resized.version, 2);
  assert.equal(controller.getTabRegistrySnapshot("owner-a", 1).viewport.owner_id, "owner-a");
  assert.equal(controller.getTabRegistrySnapshot("owner-a", 1).viewport.version, 2);
  t.after(async () => {
    await controller.shutdown("owner-a", 1);
  });
});

test("captures bounded observations through stable tabs and invalidates detached element references", async (t) => {
  const targets = [
    { id: "page-a", type: "page", webSocketDebuggerUrl: "ws://127.0.0.1:9222/devtools/page/a" },
    { id: "page-b", type: "page", webSocketDebuggerUrl: "ws://127.0.0.1:9222/devtools/page/b" },
  ];
  const fixture = makeFixture({ targets });
  const { controller } = fixture;
  await controller.start("owner-a");
  const tab = controller.getTabRegistrySnapshot("owner-a", 1).tabs
    .find((candidate) => candidate.target_id === "page-b");
  assert.ok(tab);

  FakeWebSocket.onSend = (socket, message) => {
    let result = {};
    if (message.method === "Page.getFrameTree") {
      result = { frameTree: { frame: { loaderId: "document-b" } } };
    } else if (message.method === "Accessibility.getFullAXTree") {
      result = {
        nodes: [{
          nodeId: "ax-button",
          backendDOMNodeId: 41,
          role: { value: "button" },
          name: { value: "Continue" },
        }],
      };
    } else if (message.method === "DOMSnapshot.captureSnapshot") {
      result = {
        strings: ["BUTTON"],
        documents: [{
          nodes: {
            backendNodeId: [41],
            parentIndex: [-1],
            nodeType: [1],
            nodeName: [0],
            nodeValue: [-1],
            attributes: [[]],
          },
          layout: { nodeIndex: [0], bounds: [[1, 2, 30, 20]] },
        }],
      };
    }
    queueMicrotask(() => socket.respond(message.id, result));
  };

  const snapshot = await controller.captureTabSnapshot("owner-a", 1, {
    tab_id: tab.tab_id,
    target_generation: tab.target_generation,
  });
  assert.equal(snapshot.tab_id, tab.tab_id);
  assert.equal(snapshot.document_id, "document-b");
  assert.equal(snapshot.accessibility_nodes[0].name, "Continue");
  const elementRef = snapshot.accessibility_nodes[0].element_ref;
  assert.doesNotMatch(elementRef, /41|page-b|document-b/);
  assert.equal(
    (await controller.resolveElementReference("owner-a", 1, {
      tab_id: tab.tab_id,
      target_generation: tab.target_generation,
      element_ref: elementRef,
    })).backend_node_id,
    41,
  );

  targets.splice(1, 1);
  await controller.refreshTabs("owner-a", 1);
  await assert.rejects(
    controller.resolveElementReference("owner-a", 1, {
      tab_id: tab.tab_id,
      target_generation: tab.target_generation,
      element_ref: elementRef,
    }),
    (error) => error.code === "TAB_INVALIDATED",
  );
  assert.equal(FakeWebSocket.instances[1].closed, true);

  t.after(async () => {
    await controller.shutdown("owner-a", 1);
  });
});

test("allows bounded raw AX and DOM snapshots above ordinary CDP frames and rejects overflow", async (t) => {
  const fixture = makeFixture();
  const { controller } = fixture;
  await controller.start("owner-a");
  const tab = controller.getTabRegistrySnapshot("owner-a", 1).tabs[0];
  let padding = "x".repeat(70 * 1024);
  FakeWebSocket.onSend = (socket, message) => {
    let result = {};
    if (message.method === "Page.getFrameTree") {
      result = { frameTree: { frame: { id: "root-frame", loaderId: "document-raw" } } };
    } else if (message.method === "Accessibility.getFullAXTree") {
      result = { nodes: [], padding };
    } else if (message.method === "DOMSnapshot.captureSnapshot") {
      result = {
        padding,
        strings: ["BUTTON"],
        documents: [{
          nodes: {
            backendNodeId: [50],
            parentIndex: [-1],
            nodeType: [1],
            nodeName: [0],
            nodeValue: [-1],
            attributes: [[]],
          },
          layout: { nodeIndex: [0], bounds: [[1, 2, 30, 20]] },
        }],
      };
    }
    queueMicrotask(() => socket.respond(message.id, result));
  };

  const snapshot = await controller.captureTabSnapshot("owner-a", 1, {
    tab_id: tab.tab_id,
    target_generation: tab.target_generation,
  });
  assert.equal(snapshot.document_id, "document-raw");
  assert.equal(snapshot.dom_nodes[0].node_name, "BUTTON");

  padding = "x".repeat(OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES + 1);
  await assert.rejects(
    controller.captureTabSnapshot("owner-a", 1, {
      tab_id: tab.tab_id,
      target_generation: tab.target_generation,
    }),
    (error) => error.code === ERROR_CODES.CDP_PROTOCOL_INVALID,
  );
  t.after(async () => {
    await controller.shutdown("owner-a", 1);
  });
});

test("rejects oversized binary CDP frames before UTF-8 decoding", async (t) => {
  const fixture = makeFixture();
  const { controller, sockets } = fixture;
  await controller.start("owner-a");
  const socket = sockets[0];
  socket.emit("message", {
    data: new Uint8Array(OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES + 1),
  });
  await flush();
  assert.equal(socket.closed, true);
  assert.equal(controller.health("owner-a").state, "fatal");
  assert.equal(controller.health("owner-a").fatal_code, ERROR_CODES.CONTROLLER_CRASHED);
  t.after(async () => {
    await controller.shutdownForSignal();
  });
});

test("caps fragmented WebSocket payloads before message assembly", async (t) => {
  const fixture = makeFixture();
  const { controller, sockets } = fixture;
  await controller.start("owner-a");
  const socket = sockets[0];
  assert.equal(socket.options.maxPayload, OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES);

  assert.equal(
    socket.receiveFragment(new Uint8Array(OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES - 1)),
    true,
  );
  assert.equal(socket.incomingBytes, OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES - 1);
  assert.equal(socket.receivedMessages, 0);

  assert.equal(socket.receiveFragment(new Uint8Array(2)), false);
  await flush();
  assert.equal(socket.closed, true);
  assert.equal(socket.incomingBytes, OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES - 1);
  assert.equal(socket.incomingFragments.length, 1);
  assert.equal(socket.receivedMessages, 0);
  assert.equal(controller.health("owner-a").state, "fatal");

  t.after(async () => {
    await controller.shutdownForSignal();
  });
});

test("production default WebSocket rejects an oversized fragmented payload in the receiver", async (t) => {
  const server = await startFragmentedWebSocketServer();
  t.after(async () => {
    await server.close();
  });
  let process;
  const controller = new BrowserController({
    controllerId: "controller-production-websocket",
    fetchImpl: async () => ({
      ok: true,
      text: async () => JSON.stringify([{
        id: "page-1",
        type: "page",
        webSocketDebuggerUrl: server.url,
      }]),
    }),
    spawnBrowser: async () => {
      process = new FakeProcess();
      return process;
    },
    startupTimeoutMs: 2_000,
    pollIntervalMs: 10,
    cdpConnectTimeoutMs: 1_000,
    cdpCommandTimeoutMs: 1_000,
    shutdownGraceMs: 1,
    terminateGraceMs: 1,
    killGraceMs: 1,
    heartbeatIntervalMs: 10,
  });

  const started = await controller.start("owner-a");
  assert.equal(started.state, "ready");
  await server.commandReceived;
  server.sendOversizedPayload();
  const receivedCloseCode = await Promise.race([
    server.closeCode,
    new Promise((_, reject) => setTimeout(() => reject(new Error("WebSocket close timed out")), 2_000)),
  ]);
  assert.equal(receivedCloseCode, 1009);
  await waitFor(() => controller.health("owner-a").state === "fatal");
  assert.equal(controller.health("owner-a").fatal_code, ERROR_CODES.CONTROLLER_CRASHED);
  assert.deepEqual(process.kills, ["SIGKILL"]);
  await controller.shutdownForSignal();
});

test("gracefully closes CDP and Chromium without a forced kill", async (t) => {
  const fixture = makeFixture({ autoExitOnBrowserClose: true });
  const { controller, processes, sockets, signals } = fixture;
  await controller.start("owner-a");
  const result = await controller.shutdown("owner-a", 1);
  assert.equal(result.state, "stopped");
  assert.equal(result.browser_close_sent, true);
  assert.equal(result.forced_kill, false);
  assert.equal(processes[0].kills.length, 0);
  assert.equal(sockets[0].closed, true);
  assert.deepEqual(signals, []);
  t.after(async () => {
    await controller.shutdownForSignal();
  });
});

test("falls back from graceful shutdown to SIGTERM and SIGKILL", async (t) => {
  const fixture = makeFixture();
  const { controller, processes, signals } = fixture;
  await controller.start("owner-a");
  const result = await controller.shutdown("owner-a", 1);
  assert.equal(result.forced_kill, true);
  assert.deepEqual(processes[0].kills, ["SIGTERM", "SIGKILL"]);
  assert.deepEqual(signals, ["SIGTERM", "SIGKILL"]);
  t.after(async () => {
    await controller.shutdownForSignal();
  });
});

test("changes generation on crash restart and rejects stale authority", async (t) => {
  const fixture = makeFixture({ autoExitOnBrowserClose: true });
  const { controller, processes } = fixture;
  await controller.start("owner-a");
  processes[0].exit(1, "SIGSEGV");
  assert.equal(controller.health("owner-a").state, "fatal");
  const restarted = await controller.restart("owner-a", 1);
  assert.equal(restarted.generation, 2);
  assert.equal(restarted.state, "ready");
  await assert.rejects(
    controller.execute("owner-a", 1, "health_probe"),
    (error) => error.code === ERROR_CODES.STALE_GENERATION,
  );
  processes[0].emit("exit", 1, "SIGSEGV");
  assert.equal(controller.health("owner-a", 2).state, "ready");
  t.after(async () => {
    await controller.shutdown("owner-a", 2);
  });
});

test("bounds the operation queue and keeps the probe data-only", async (t) => {
  const fixture = makeFixture({ queueLimit: 1 });
  const { controller, sockets } = fixture;
  await controller.start("owner-a");
  const socket = sockets[0];
  socket.holdResponses = true;
  const first = controller.execute("owner-a", 1, "health_probe");
  await flush();
  await assert.rejects(
    controller.execute("owner-a", 1, "health_probe"),
    (error) => error.code === ERROR_CODES.QUEUE_SATURATED,
  );
  const request = socket.sent[socket.sent.length - 1];
  socket.respond(request.id, { browser: "not-returned-to-callers" });
  const result = await first;
  assert.deepEqual(result, {
    operation: "health_probe",
    generation: 1,
    cdp_connected: true,
  });
  t.after(async () => {
    await controller.shutdown("owner-a", 1);
  });
});

test("cleans pending work and reports fatal state on CDP disconnect", async (t) => {
  const fixture = makeFixture();
  const { controller, sockets, signals } = fixture;
  await controller.start("owner-a");
  sockets[0].holdResponses = true;
  const pending = controller.execute("owner-a", 1, "health_probe");
  await flush();
  sockets[0].close();
  await assert.rejects(
    pending,
    (error) => error.code === ERROR_CODES.CDP_DISCONNECTED,
  );
  assert.equal(controller.health("owner-a").state, "fatal");
  assert.equal(controller.health("owner-a").fatal_code, ERROR_CODES.CONTROLLER_CRASHED);
  assert.deepEqual(signals, ["SIGKILL"]);
  t.after(async () => {
    await controller.shutdownForSignal();
  });
});

test("enforces structured request IDs, byte bounds, and secret redaction", async (t) => {
  const fixture = makeFixture({ autoExitOnBrowserClose: true });
  const { controller } = fixture;
  const protocol = new ControllerProtocol(controller);
  const started = await protocol.handle({
    type: "start",
    request_id: "start-1",
    owner_id: "owner-a",
  });
  assert.equal(started.ok, true);
  assert.equal(started.request_id, "start-1");
  const invalid = await protocol.handle({
    type: "health",
    request_id: "bad-1",
    owner_id: "owner-a",
    password: "super-secret",
  });
  assert.equal(invalid.error.code, ERROR_CODES.SCHEMA_INVALID);
  assert.doesNotMatch(JSON.stringify(invalid), /super-secret/);
  const duplicate = await protocol.handle({
    type: "health",
    request_id: "start-1",
    owner_id: "owner-a",
  });
  assert.equal(duplicate.error.code, ERROR_CODES.DUPLICATE_REQUEST_ID);
  const oversized = await protocol.handleLine("x".repeat(64 * 1024 + 1));
  assert.equal(oversized.error.code, ERROR_CODES.REQUEST_TOO_LARGE);
  assert.equal(redactDiagnostic("token=super-secret password=also-secret").includes("super-secret"), false);
  assert.equal(redactDiagnostic("token=super-secret password=also-secret").includes("also-secret"), false);
  const stopped = await protocol.handle({
    type: "shutdown",
    request_id: "stop-1",
    owner_id: "owner-a",
    generation: 1,
  });
  assert.equal(stopped.ok, true);
  t.after(async () => {
    await controller.shutdownForSignal();
  });
});
