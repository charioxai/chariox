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
