import test from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";

import {
  BrowserController,
  ControllerProtocol,
  ERROR_CODES,
  redactDiagnostic,
} from "./browser-controller.mjs";
import { OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES } from "./browser-controller-observations.mjs";
import { BrowserPageFeatures } from "./browser-controller-page-features.mjs";
import { UPLOAD_ARTIFACT_KIND } from "./browser-controller-upload-staging.mjs";

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
    pageFeatures: options.pageFeatures,
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
      result = { frameTree: { frame: { id: "frame-b", loaderId: "document-b" } } };
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
          frameId: "frame-b",
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
    controller.resolveElementReference("owner-a", 1, {
      tab_id: tab.tab_id,
      target_generation: tab.target_generation,
      element_ref: elementRef,
    }).backend_node_id,
    41,
  );

  targets.splice(1, 1);
  await controller.refreshTabs("owner-a", 1);
  assert.throws(
    () => controller.resolveElementReference("owner-a", 1, {
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

test("runs a bounded locator action through an opaque observed element reference", async (t) => {
  const fixture = makeFixture();
  const { controller } = fixture;
  await controller.start("owner-a");
  const tab = controller.getTabRegistrySnapshot("owner-a", 1).tabs[0];

  FakeWebSocket.onSend = (socket, message) => {
    let result = {};
    if (message.method === "Page.getFrameTree") {
      result = { frameTree: { frame: { id: "frame-action", loaderId: "document-action" } } };
    } else if (message.method === "Accessibility.getFullAXTree") {
      result = {
        nodes: [{
          nodeId: "ax-action",
          backendDOMNodeId: 73,
          role: { value: "button" },
          name: { value: "Continue" },
        }],
      };
    } else if (message.method === "DOMSnapshot.captureSnapshot") {
      result = {
        strings: ["BUTTON"],
        documents: [{
          frameId: "frame-action",
          nodes: {
            backendNodeId: [73],
            parentIndex: [-1],
            nodeType: [1],
            nodeName: [0],
            nodeValue: [-1],
            attributes: [[]],
          },
          layout: { nodeIndex: [0], bounds: [[2, 3, 80, 24]] },
        }],
      };
    }
    queueMicrotask(() => socket.respond(message.id, result));
  };
  const snapshot = await controller.captureTabSnapshot("owner-a", 1, {
    tab_id: tab.tab_id,
    target_generation: tab.target_generation,
  });
  const elementRef = snapshot.accessibility_nodes[0].element_ref;

  const actionMethods = [];
  FakeWebSocket.onSend = (socket, message) => {
    actionMethods.push(message);
    let result = {};
    if (message.method === "Page.getFrameTree") {
      result = { frameTree: { frame: { id: "frame-action", loaderId: "document-action" } } };
    } else if (message.method === "DOM.resolveNode") {
      result = { object: { objectId: "object-action" } };
    } else if (message.method === "Runtime.callFunctionOn") {
      result = {
        result: {
          value: {
            state: "ready",
            x: 42,
            y: 19,
            width: 80,
            height: 24,
            editable: false,
          },
        },
      };
    }
    queueMicrotask(() => socket.respond(message.id, result));
  };
  const action = await controller.performElementAction("owner-a", 1, {
    tab_id: tab.tab_id,
    target_generation: tab.target_generation,
    element_ref: elementRef,
    action: { kind: "click" },
    timeout_ms: 250,
  });

  assert.deepEqual(action, {
    tab_id: tab.tab_id,
    document_id: "document-action",
    snapshot_revision: 1,
    action_kind: "click",
    attempts: 2,
    elapsed_ms: 50,
  });
  assert.deepEqual(
    actionMethods
      .filter((message) => message.method === "Input.dispatchMouseEvent")
      .map((message) => message.params.type),
    ["mouseMoved", "mousePressed", "mouseReleased"],
  );
  assert.deepEqual(
    actionMethods.find((message) => message.method === "DOM.resolveNode")?.params,
    { backendNodeId: 73 },
  );
  assert.doesNotMatch(JSON.stringify(action), /73|object-action/);
  t.after(async () => {
    await controller.shutdown("owner-a", 1);
  });
});

test("wires bounded page features through controller-owned tabs and connections", async (t) => {
  const targets = [{
    id: "page-1",
    type: "page",
    webSocketDebuggerUrl: "ws://127.0.0.1:9222/devtools/page/1",
  }];
  const pageFeatures = new BrowserPageFeatures({
    uploadRoots: ["/safe"],
    downloadRoot: "/safe/downloads",
    realpath: async (candidate) => candidate,
    stat: async (candidate) => ({
      isDirectory: () => candidate === "/safe" || candidate === "/safe/downloads",
      isFile: () => candidate === "/safe/report.txt",
      size: candidate === "/safe/report.txt" ? 1024 : 0,
      dev: 1,
      ino: candidate === "/safe" ? 10 : candidate === "/safe/downloads" ? 11 : 20,
    }),
    open: async () => ({
      stat: async () => ({ isFile: () => true, size: 1024, dev: 1, ino: 20 }),
      read: async (buffer, offset, length) => {
        buffer.fill(0x41, offset, offset + length);
        return { bytesRead: length };
      },
      close: async () => {},
    }),
    uploadArtifactBroker: {
      separate_uid: true,
      stage: async ({ expectedSize }) => ({
        kind: UPLOAD_ARTIFACT_KIND,
        artifact_id: "controller-upload",
        path: "/broker-owned/controller-upload",
        size: expectedSize,
        immutable: true,
        sealed: true,
        separate_uid: true,
        broker_owned: true,
        release: async () => {},
      }),
    },
  });
  const fixture = makeFixture({ targets, pageFeatures });
  const { controller } = fixture;
  await controller.start("owner-a");
  const tab = controller.getTabRegistrySnapshot("owner-a", 1).tabs[0];

  FakeWebSocket.onSend = (socket, message) => {
    let result = {};
    if (message.method === "Page.getFrameTree") {
      result = { frameTree: { frame: { id: "frame-main", loaderId: "document-feature" } } };
    } else if (message.method === "Accessibility.getFullAXTree") {
      result = {
        nodes: [{
          nodeId: "ax-upload",
          backendDOMNodeId: 91,
          frameId: "frame-main",
          role: { value: "textbox" },
          name: { value: "Upload" },
        }],
      };
    } else if (message.method === "DOMSnapshot.captureSnapshot") {
      result = {
        strings: ["INPUT"],
        documents: [{
          frameId: "frame-main",
          nodes: {
            backendNodeId: [91],
            parentIndex: [-1],
            nodeType: [1],
            nodeName: [0],
            nodeValue: [-1],
            attributes: [[]],
          },
          layout: { nodeIndex: [0], bounds: [[1, 2, 30, 20]] },
        }],
      };
    } else if (message.method === "DOM.describeNode") {
      result = {
        node: {
          frameId: "frame-main",
          shadowRootType: "open",
          nodeName: "INPUT",
          localName: "input",
          nodeValue: "private page value",
        },
      };
    }
    queueMicrotask(() => socket.respond(message.id, result));
  };

  const snapshot = await controller.captureTabSnapshot("owner-a", 1, {
    tab_id: tab.tab_id,
    target_generation: tab.target_generation,
  });
  const elementRef = snapshot.accessibility_nodes[0].element_ref;
  const elementRequest = {
    tab_id: tab.tab_id,
    target_generation: tab.target_generation,
    element_ref: elementRef,
  };
  const description = await controller.describeElementContext("owner-a", 1, elementRequest);
  assert.equal(description.frame_id, "frame-main");
  assert.doesNotMatch(JSON.stringify(description), /private page value/);
  assert.deepEqual(await controller.handleDialog("owner-a", 1, {
    tab_id: tab.tab_id,
    target_generation: tab.target_generation,
    dialog: { action: "accept", prompt_text: "private response" },
  }), { action: "accept" });
  assert.deepEqual(await controller.configureDownloads("owner-a", 1), { enabled: true });
  const upload = await controller.uploadFiles("owner-a", 1, {
    ...elementRequest,
    paths: ["/safe/report.txt"],
  });
  assert.equal(upload.file_count, 1);
  assert.doesNotMatch(JSON.stringify(upload), /report\.txt/);
  assert.deepEqual(await controller.grantPermissions("owner-a", 1, {
    origin: "https://example.test",
    permissions: ["notifications"],
  }), { origin: "https://example.test", permissions: ["notifications"] });
  assert.deepEqual(await controller.resetPermissions("owner-a", 1), { reset: true });

  targets.push({
    id: "popup-1",
    type: "page",
    webSocketDebuggerUrl: "ws://127.0.0.1:9222/devtools/page/popup-1",
  });
  const popup = await controller.waitForPopup("owner-a", 1, {
    known_tab_ids: [tab.tab_id],
    timeout_ms: 250,
  });
  assert.match(popup.tab_id, /^tab-/);
  assert.notEqual(popup.tab_id, tab.tab_id);

  const methods = FakeWebSocket.instances.flatMap((socket) => socket.sent.map((message) => message.method));
  for (const method of [
    "DOM.describeNode",
    "Page.handleJavaScriptDialog",
    "Browser.setDownloadBehavior",
    "DOM.setFileInputFiles",
    "Browser.grantPermissions",
    "Browser.resetPermissions",
  ]) {
    assert.equal(methods.includes(method), true, method);
  }
  t.after(async () => {
    await controller.shutdown("owner-a", 1);
  });
});

test("propagates post-action uncertainty through the public controller action API", async (t) => {
  const fixture = makeFixture();
  const { controller } = fixture;
  await controller.start("owner-a");
  const tab = controller.getTabRegistrySnapshot("owner-a", 1).tabs[0];
  controller.observationStore.resolve = () => ({
    tab_id: tab.tab_id,
    document_id: "document-action",
    frame_id: "frame-main",
    main_frame_id: "frame-main",
    snapshot_revision: 1,
    backend_node_id: 91,
  });
  FakeWebSocket.onSend = (socket, message) => {
    if (
      message.method === "Input.dispatchMouseEvent" &&
      message.params.type === "mousePressed"
    ) {
      queueMicrotask(() => socket.emit("message", {
        data: JSON.stringify({
          id: message.id,
          error: { code: -32000, message: "synthetic post-action failure" },
        }),
      }));
      return;
    }
    let result = {};
    if (message.method === "Page.getFrameTree") {
      result = { frameTree: { frame: { id: "frame-main", loaderId: "document-action" } } };
    } else if (message.method === "DOM.resolveNode") {
      result = { object: { objectId: "object-action" } };
    } else if (message.method === "Runtime.callFunctionOn") {
      result = {
        result: {
          value: {
            state: "ready",
            x: 42,
            y: 19,
            width: 80,
            height: 24,
            editable: false,
          },
        },
      };
    }
    queueMicrotask(() => socket.respond(message.id, result));
  };

  await assert.rejects(
    controller.performElementAction("owner-a", 1, {
      tab_id: tab.tab_id,
      target_generation: tab.target_generation,
      element_ref: "opaque-element",
      action: { kind: "click" },
      timeout_ms: 500,
    }),
    (error) => {
      assert.equal(error.code, ERROR_CODES.ACTION_POST_ACTION_UNCERTAIN);
      assert.equal(error.message, "browser action outcome is uncertain after a post-action failure");
      return true;
    },
  );
  assert.equal(ERROR_CODES.ACTION_POST_ACTION_UNCERTAIN, "ACTION_POST_ACTION_UNCERTAIN");
  t.after(async () => {
    await controller.shutdown("owner-a", 1);
  });
});

test("releases page-feature uploads when a tab detaches and on controller shutdown", async (t) => {
  const targets = [{
    id: "page-1",
    type: "page",
    webSocketDebuggerUrl: "ws://127.0.0.1:9222/devtools/page/1",
  }];
  const releasedTabs = [];
  let shutdowns = 0;
  const pageFeatures = {
    async prepare() {},
    async releaseUploadsForTab(tabId) {
      releasedTabs.push(tabId);
    },
    async shutdown() {
      shutdowns += 1;
    },
  };
  const fixture = makeFixture({ targets, pageFeatures });
  const { controller } = fixture;
  await controller.start("owner-a");
  const tab = controller.getTabRegistrySnapshot("owner-a", 1).tabs[0];
  targets.length = 0;
  const refreshed = await controller.refreshTabs("owner-a", 1);
  assert.deepEqual(refreshed.detached.map(({ tab_id }) => tab_id), [tab.tab_id]);
  assert.deepEqual(releasedTabs, [tab.tab_id]);
  await controller.shutdown("owner-a", 1);
  assert.equal(shutdowns, 1);
  t.after(async () => {
    await controller.shutdownForSignal();
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

test("expires a 100ms action while it waits behind longer queued work", async (t) => {
  const fixture = makeFixture();
  const { controller, clock } = fixture;
  await controller.start("owner-a");
  const tab = controller.getTabRegistrySnapshot("owner-a", 1).tabs[0];
  let releaseBlocker;
  const blocker = new Promise((resolve) => {
    releaseBlocker = resolve;
  });
  const held = controller._enqueue(1, async () => blocker);
  await flush();

  const queuedAction = controller.performElementAction("owner-a", 1, {
    tab_id: tab.tab_id,
    target_generation: tab.target_generation,
    element_ref: "never-resolved",
    action: { kind: "click" },
    timeout_ms: 100,
  });
  clock.advance(101);
  releaseBlocker();
  await held;
  await assert.rejects(
    queuedAction,
    (error) => error.code === ERROR_CODES.ACTION_TIMEOUT,
  );
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
