import assert from "node:assert/strict";
import test from "node:test";

import {
  assertPrivateDebuggerUrl,
  BrowserCdpClient,
  CdpConnection,
} from "./browser-controller-cdp.mjs";

const viewport = {
  css_width: 1280,
  css_height: 720,
  device_scale_factor: 2,
  desktop_pixel_width: 2560,
  desktop_pixel_height: 1440,
};

test("persistent browser connection returns page identities, focus, and applied viewport", async () => {
  const connection = new FakeConnection();
  let connectionCount = 0;
  const browser = new BrowserCdpClient({
    connectionFactory: async () => {
      connectionCount += 1;
      return connection;
    },
  });

  const first = await browser.reconcile(viewport);
  const second = await browser.reconcile(viewport);

  assert.equal(connectionCount, 1);
  assert.equal(first.browser_generation, 1);
  assert.deepEqual(first, second);
  assert.deepEqual(first.tabs, [
    {
      target_id: "target-a",
      document_id: "loader-a",
      url: "https://a.test/",
      title: "A",
    },
    {
      target_id: "target-b",
      document_id: "loader-b",
      url: "https://b.test/",
      title: "B",
    },
  ]);
  assert.equal(first.focused_target_id, "target-b");
  assert.deepEqual(first.viewport, viewport);
  assert.equal(
    connection.calls.filter((call) => call.method === "Target.attachToTarget").length,
    2,
    "persistent target sessions should be reused",
  );
  assert.equal(
    connection.calls.filter((call) => call.method === "Page.enable").length,
    2,
    "each persistent target session should subscribe to Page lifecycle events once",
  );
  assert.deepEqual(
    connection.calls.find(
      (call) => call.method === "Emulation.setDeviceMetricsOverride" && call.sessionId === "session-a",
    ).params,
    {
      width: 1280,
      height: 720,
      deviceScaleFactor: 2,
      mobile: false,
      screenWidth: 1280,
      screenHeight: 720,
    },
  );
  assert.equal(
    connection.calls.find(
      (call) => call.method === "Runtime.evaluate" && call.sessionId === "session-a",
    ).params.expression,
    "document.visibilityState === 'visible'",
    "active-tab focus must not disappear merely because the browser window loses OS focus",
  );
});

test("viewport validation happens before opening a browser connection", async () => {
  let connected = false;
  const browser = new BrowserCdpClient({
    connectionFactory: async () => {
      connected = true;
      return new FakeConnection();
    },
  });

  await assert.rejects(
    browser.reconcile({ ...viewport, css_width: 0 }),
    (error) => error.code === "browser_viewport_invalid",
  );
  assert.equal(connected, false);
});

test("structured snapshots bind compact accessibility and DOM nodes to one document", async () => {
  const connection = new SnapshotConnection();
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
  });
  await browser.reconcile(viewport);

  const first = await browser.snapshot({
    target_id: "target-a",
    document_id: "loader-a",
  });
  const second = await browser.snapshot({
    target_id: "target-a",
    document_id: "loader-a",
  });

  assert.equal(first.browser_generation, 1);
  assert.equal(first.target_id, "target-a");
  assert.equal(first.document_id, "loader-a");
  assert.equal(first.snapshot_revision, 1);
  assert.equal(second.snapshot_revision, 2);
  assert.deepEqual(first.accessibility_nodes, [
    {
      node_ref: "backend:103",
      parent_ref: null,
      child_refs: ["backend:104"],
      role: "button",
      name: "Save",
      description: "Save this form",
      value: "",
      ignored: false,
      disabled: false,
      focused: true,
    },
    {
      node_ref: "backend:104",
      parent_ref: "backend:103",
      child_refs: [],
      role: "StaticText",
      name: "Save",
      description: "",
      value: "",
      ignored: false,
      disabled: false,
      focused: false,
    },
  ]);
  assert.deepEqual(
    first.dom_nodes.find((node) => node.node_ref === "backend:103"),
    {
      node_ref: "backend:103",
      parent_ref: "backend:102",
      node_type: 1,
      node_name: "BUTTON",
      text: "",
      attributes: { id: "save", type: "submit" },
      bounds: { x: 10, y: 20, width: 100, height: 30 },
    },
  );
  assert.deepEqual(
    first.dom_nodes.find((node) => node.node_ref === "backend:105").attributes,
    { type: "password", value: "[redacted]" },
    "password values must never leave the controller snapshot",
  );
  assert.deepEqual(first.dom_documents, [
    { document_index: 0, url: "https://a.test/", owner_node_ref: null },
    { document_index: 1, url: "https://frame.test/", owner_node_ref: "backend:103" },
  ]);
  assert.deepEqual(first.shadow_roots, [
    { node_ref: "backend:102", shadow_root_type: "open" },
  ]);

  connection.loaderId = "loader-a2";
  await assert.rejects(
    browser.snapshot({ target_id: "target-a", document_id: "loader-a" }),
    (error) => error.code === "stale_document_reference",
  );
  connection.loaderId = "loader-a";
  const afterRejectedCapture = await browser.snapshot({
    target_id: "target-a",
    document_id: "loader-a",
  });
  assert.equal(
    afterRejectedCapture.snapshot_revision,
    3,
    "a rejected capture must not consume a snapshot revision",
  );
});

test("structured snapshot strings are bounded by UTF-8 bytes", async () => {
  const connection = new SnapshotConnection();
  connection.accessibilityName = "😀".repeat(2_000);
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
  });
  await browser.reconcile(viewport);

  const snapshot = await browser.snapshot({
    target_id: "target-a",
    document_id: "loader-a",
  });

  assert.equal(
    new TextEncoder().encode(snapshot.accessibility_nodes[0].name).length,
    2_048,
  );
});

test("dialog handling is document-bound and validates prompt input", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
  });
  await browser.reconcile(viewport);
  const frameTreeCallsBeforeDialog = connection.calls.filter(
    (call) => call.method === "Page.getFrameTree",
  ).length;

  const accepted = await browser.handleDialog({
    target_id: "target-a",
    document_id: "loader-a",
    action: "accept",
    prompt_text: "answer",
  });

  assert.deepEqual(accepted, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    action: "accept",
  });
  assert.deepEqual(
    connection.calls.find((call) => call.method === "Page.handleJavaScriptDialog").params,
    { accept: true, promptText: "answer" },
  );
  await browser.handleDialog({
    target_id: "target-a",
    document_id: "loader-a",
    action: "accept",
    prompt_text: "",
  });
  assert.deepEqual(
    connection.calls.filter((call) => call.method === "Page.handleJavaScriptDialog").at(-1).params,
    { accept: true, promptText: "" },
  );
  assert.equal(
    connection.calls.filter((call) => call.method === "Page.getFrameTree").length,
    frameTreeCallsBeforeDialog,
    "dialog handling must use the last controller-observed document because page commands block while a dialog is open",
  );
  await assert.rejects(
    browser.handleDialog({
      target_id: "target-a",
      document_id: "loader-b",
      action: "dismiss",
    }),
    (error) => error.code === "stale_document_reference",
  );
  await assert.rejects(
    browser.handleDialog({
      target_id: "target-a",
      document_id: "loader-a",
      action: "ignore",
    }),
    (error) => error.code === "browser_dialog_invalid",
  );
});

test("download and upload requests stay target-bound and return no file paths", async () => {
  const connection = new FakeConnection();
  const entries = new Map([
    ["/safe/downloads", { type: "directory", size: 0 }],
    ["/safe/uploads", { type: "directory", size: 0 }],
    ["/safe/uploads/report.txt", { type: "file", size: 12 }],
  ]);
  const fileSystem = {
    mkdir: async () => {},
    realpath: async (candidate) => {
      if (!entries.has(candidate)) throw Object.assign(new Error("missing"), { code: "ENOENT" });
      return candidate;
    },
    stat: async (candidate) => {
      const entry = entries.get(candidate);
      if (!entry) throw Object.assign(new Error("missing"), { code: "ENOENT" });
      return {
        size: entry.size,
        isDirectory: () => entry.type === "directory",
        isFile: () => entry.type === "file",
      };
    },
  };
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    downloadDirectory: "/safe/downloads",
    uploadRoots: ["/safe/uploads"],
    fileSystem,
  });
  await browser.reconcile(viewport);

  const downloads = await browser.configureDownloads({
    target_id: "target-a",
    document_id: "loader-a",
  });
  const upload = await browser.uploadFiles({
    target_id: "target-a",
    document_id: "loader-a",
    node_ref: "backend:103",
    file_paths: ["/safe/uploads/report.txt"],
  });

  assert.deepEqual(downloads, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    enabled: true,
  });
  assert.deepEqual(upload, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    file_count: 1,
    total_bytes: 12,
  });
  assert.equal(JSON.stringify({ downloads, upload }).includes("/safe"), false);
});

test("permission decisions derive the current target origin", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);

  const result = await browser.setPermission({
    target_id: "target-a",
    document_id: "loader-a",
    permission: "geolocation",
    setting: "prompt",
  });

  assert.deepEqual(result, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    permission: "geolocation",
    setting: "prompt",
  });
  assert.deepEqual(
    connection.calls.find((call) => call.method === "Browser.setPermission").params,
    {
      permission: { name: "geolocation" },
      setting: "prompt",
      origin: "https://a.test",
    },
  );
});

test("debugger discovery cannot redirect the controller away from loopback", () => {
  assert.equal(
    assertPrivateDebuggerUrl(
      "ws://localhost:9222/devtools/browser/one",
      "http://127.0.0.1:9222",
    ),
    "ws://localhost:9222/devtools/browser/one",
  );
  assert.throws(
    () =>
      assertPrivateDebuggerUrl(
        "ws://example.test:9222/devtools/browser/one",
        "http://127.0.0.1:9222",
      ),
    (error) => error.code === "browser_debugger_endpoint_unsafe",
  );
  assert.throws(
    () =>
      assertPrivateDebuggerUrl(
        "ws://127.0.0.1:9333/devtools/browser/one",
        "http://127.0.0.1:9222",
      ),
    (error) => error.code === "browser_debugger_endpoint_unsafe",
  );
});

test("CDP responses are correlated and command failures stay bounded", async () => {
  const socket = new FakeSocket();
  const connection = new CdpConnection(socket, 20);
  const response = connection.send("Target.getTargets");
  const request = JSON.parse(socket.sent[0]);
  socket.message({ id: request.id, result: { targetInfos: [] } });
  assert.deepEqual(await response, { targetInfos: [] });

  const dialog = connection.waitForEvent("Page.javascriptDialogOpening", 20);
  socket.message({
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    params: { type: "prompt", message: "Name?" },
  });
  assert.deepEqual(await dialog.promise, {
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    params: { type: "prompt", message: "Name?" },
  });

  const timedOut = connection.send("Page.getFrameTree", {}, "session-a");
  await assert.rejects(timedOut, (error) => error.code === "browser_cdp_timeout");
  await connection.close();
});

class FakeConnection {
  constructor() {
    this.calls = [];
    this.open = true;
  }

  isOpen() {
    return this.open;
  }

  async close() {
    this.open = false;
  }

  async send(method, params = {}, sessionId) {
    this.calls.push({ method, params, sessionId });
    if (method === "Target.getTargets") {
      return {
        targetInfos: [
          { targetId: "target-a", type: "page", url: "https://a.test/", title: "A" },
          { targetId: "worker-a", type: "service_worker", url: "https://a.test/sw.js" },
          { targetId: "target-b", type: "page", url: "https://b.test/", title: "B" },
        ],
      };
    }
    if (method === "Target.attachToTarget") {
      return { sessionId: params.targetId === "target-a" ? "session-a" : "session-b" };
    }
    if (method === "Page.getFrameTree") {
      return {
        frameTree: {
          frame: { loaderId: sessionId === "session-a" ? "loader-a" : "loader-b" },
        },
      };
    }
    if (method === "Runtime.evaluate") {
      return { result: { value: sessionId === "session-b" } };
    }
    return {};
  }
}

class SnapshotConnection extends FakeConnection {
  constructor() {
    super();
    this.loaderId = "loader-a";
    this.accessibilityName = "Save";
  }

  async send(method, params = {}, sessionId) {
    if (method === "Page.getFrameTree") {
      this.calls.push({ method, params, sessionId });
      return { frameTree: { frame: { loaderId: this.loaderId } } };
    }
    if (method === "Accessibility.getFullAXTree") {
      this.calls.push({ method, params, sessionId });
      return {
        nodes: [
          {
            nodeId: "ax-button",
            backendDOMNodeId: 103,
            ignored: false,
            role: { value: "button" },
            name: { value: this.accessibilityName },
            description: { value: "Save this form" },
            childIds: ["ax-text"],
            properties: [
              { name: "focused", value: { value: true } },
              { name: "disabled", value: { value: false } },
            ],
          },
          {
            nodeId: "ax-text",
            backendDOMNodeId: 104,
            parentId: "ax-button",
            ignored: false,
            role: { value: "StaticText" },
            name: { value: "Save" },
          },
        ],
      };
    }
    if (method === "DOMSnapshot.captureSnapshot") {
      this.calls.push({ method, params, sessionId });
      return {
        strings: [
          "#document",
          "HTML",
          "BODY",
          "BUTTON",
          "#text",
          "Save",
          "id",
          "save",
          "type",
          "submit",
          "INPUT",
          "password",
          "value",
          "top-secret",
          "https://a.test/",
          "https://frame.test/",
          "open",
        ],
        documents: [
          {
            nodes: {
              parentIndex: [-1, 0, 1, 2, 3, 2],
              nodeType: [9, 1, 1, 1, 3, 1],
              nodeName: [0, 1, 2, 3, 4, 10],
              nodeValue: [0, 0, 0, 0, 5, 0],
              backendNodeId: [100, 101, 102, 103, 104, 105],
              attributes: [[], [], [], [6, 7, 8, 9], [], [8, 11, 12, 13]],
              contentDocumentIndex: { index: [3], value: [1] },
              shadowRootType: { index: [2], value: [16] },
            },
            documentURL: 14,
            layout: {
              nodeIndex: [3],
              bounds: [[10, 20, 100, 30]],
            },
          },
          {
            nodes: {
              parentIndex: [-1],
              nodeType: [9],
              nodeName: [0],
              nodeValue: [0],
              backendNodeId: [200],
              attributes: [[]],
            },
            documentURL: 15,
            layout: { nodeIndex: [], bounds: [] },
          },
        ],
      };
    }
    return super.send(method, params, sessionId);
  }
}

class FakeSocket extends EventTarget {
  constructor() {
    super();
    this.readyState = 1;
    this.sent = [];
  }

  send(payload) {
    this.sent.push(payload);
  }

  close() {
    this.readyState = 3;
    this.dispatchEvent(new Event("close"));
  }

  message(payload) {
    this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) }));
  }
}
