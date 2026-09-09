import assert from "node:assert/strict";
import test from "node:test";

import { acquireBrowserCookieWriterFence } from "./browser-controller-cookie-fence.mjs";

test("cookie writer fence freezes pages and pauses new browser writers until release", async () => {
  const connection = new FenceConnection();
  const fence = await acquireBrowserCookieWriterFence({
    connection,
    pageSessions: ["page-a", "page-b"],
    waitForNetworkIdle: async (sessions) => assert.deepEqual(sessions, ["page-a", "page-b"]),
  });

  assert.equal(connection.calls[0].method, "Target.setAutoAttach");
  for (const sessionId of ["page-a", "page-b"]) {
    assert.deepEqual(
      connection.calls.filter((call) => call.sessionId === sessionId).map(({ method }) => method),
      ["ServiceWorker.enable", "ServiceWorker.stopAllWorkers", "Emulation.setScriptExecutionDisabled",
        "Network.emulateNetworkConditions", "Page.stopLoading",
        "Page.setWebLifecycleState"],
    );
  }

  const releaseStart = connection.calls.length;
  await fence.release();
  const releaseMethods = connection.calls.slice(releaseStart).map(({ method }) => method);
  assert.deepEqual(releaseMethods, [
    "Target.setAutoAttach",
    "Runtime.runIfWaitingForDebugger",
    "Target.detachFromTarget",
    "Page.setWebLifecycleState",
    "Network.emulateNetworkConditions",
    "Emulation.setScriptExecutionDisabled",
    "ServiceWorker.disable",
    "Page.setWebLifecycleState",
    "Network.emulateNetworkConditions",
    "Emulation.setScriptExecutionDisabled",
    "ServiceWorker.disable",
  ]);
  assert.equal(connection.calls.filter(({ method }) => method === "Network.emulateNetworkConditions")
    .at(-1).params.offline, false);
});

test("cookie writer fence fails closed and restores pages if acquisition is incomplete", async () => {
  const connection = new FenceConnection();
  connection.failOn = "Page.stopLoading";
  await assert.rejects(
    acquireBrowserCookieWriterFence({
      connection,
      pageSessions: ["page-a"],
      waitForNetworkIdle: async () => {},
    }),
    { code: "browser_cookie_writer_fence_failed", recoveryRequired: true },
  );
  assert.ok(connection.calls.some(({ method, params }) =>
    method === "Network.emulateNetworkConditions" && params.offline === false));
});

class FenceConnection {
  constructor() {
    this.calls = [];
    this.listeners = new Set();
    this.failOn = null;
  }

  subscribe(listener) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  async send(method, params = {}, sessionId) {
    this.calls.push({ method, params, sessionId });
    if (method === "Target.setAutoAttach" && params.autoAttach) {
      for (const listener of this.listeners) {
        listener({
          method: "Target.attachedToTarget",
          params: {
            sessionId: "new-worker",
            waitingForDebugger: true,
            targetInfo: { type: "service_worker" },
          },
        });
      }
    }
    if (method === this.failOn) throw new Error("fixture failure");
    return {};
  }
}
