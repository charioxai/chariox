import test from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";

import {
  BrowserController,
  ControllerProtocol,
  ERROR_CODES,
  redactDiagnostic,
} from "./browser-controller.mjs";

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

  constructor(url) {
    super();
    this.url = url;
    this.readyState = 0;
    this.sent = [];
    this.closed = false;
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
        : [{ type: "page", webSocketDebuggerUrl: "ws://127.0.0.1:9222/devtools/page/1" }],
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
