import assert from "node:assert/strict";
import test from "node:test";

import { acquireBrowserCookieWriterFence } from "./browser-controller-cookie-fence.mjs";

test("cookie writer fence freezes pages and pauses new browser writers until release", async () => {
  const connection = new FenceConnection();
  let closedTargets;
  const fence = await acquireBrowserCookieWriterFence({
    connection,
    pageSessions: ["page-a", "page-b"],
    knownWriterTargets: [{ targetId: "worker-known", sessionId: "worker-known-session" }],
    waitForNetworkIdle: async (sessions) => {
      assert.deepEqual(sessions, ["page-a", "page-b", "worker-known-session"]);
    },
    waitForWriterTargetsGone: async (targetIds) => { closedTargets = targetIds; },
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
  assert.deepEqual(
    connection.calls.filter((call) => call.sessionId === "worker-known-session")
      .map(({ method }) => method),
    ["Debugger.pause", "Network.emulateNetworkConditions"],
  );
  assert.deepEqual(closedTargets, ["worker-untracked"]);
  assert.ok(connection.calls.some(({ method, params }) =>
    method === "Target.closeTarget" && params.targetId === "worker-untracked"));

  const releaseStart = connection.calls.length;
  await fence.release();
  const releaseMethods = connection.calls.slice(releaseStart).map(({ method }) => method);
  assert.deepEqual(releaseMethods, [
    "Target.setAutoAttach",
    "Runtime.runIfWaitingForDebugger",
    "Target.detachFromTarget",
    "Network.emulateNetworkConditions",
    "Debugger.resume",
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
      knownWriterTargets: [{ targetId: "worker-known", sessionId: "worker-known-session" }],
      waitForNetworkIdle: async () => {},
    }),
    { code: "browser_cookie_writer_fence_failed", recoveryRequired: true },
  );
  assert.ok(connection.calls.some(({ method, params }) =>
    method === "Network.emulateNetworkConditions" && params.offline === false));
  assert.ok(connection.calls.some(({ method, sessionId }) =>
    method === "Debugger.resume" && sessionId === "worker-known-session"));
});

test("cookie writer fence closes an untracked dedicated worker through its own session", async () => {
  const connection = new FenceConnection();
  connection.targetInfos = [{ targetId: "worker-untracked", type: "worker" }];
  const fence = await acquireBrowserCookieWriterFence({
    connection,
    pageSessions: [],
    waitForNetworkIdle: async (sessions) => assert.deepEqual(sessions, []),
    waitForWriterTargetsGone: async (targetIds) => {
      assert.deepEqual(targetIds, ["worker-untracked"]);
    },
  });
  assert.ok(connection.calls.some(({ method, params }) =>
    method === "Target.attachToTarget"
      && params.targetId === "worker-untracked"
      && params.flatten === true));
  assert.ok(connection.calls.some(({ method, params, sessionId }) =>
    method === "Runtime.evaluate"
      && params.expression === "self.close()"
      && sessionId === "worker-untracked-session"));
  assert.ok(!connection.calls.some(({ method, params }) =>
    method === "Target.closeTarget" && params.targetId === "worker-untracked"));
  await fence.release();
});

class FenceConnection {
  constructor() {
    this.calls = [];
    this.listeners = new Set();
    this.failOn = null;
    this.targetInfos = [
      { targetId: "worker-known", type: "worker" },
      { targetId: "worker-untracked", type: "shared_worker" },
    ];
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
    if (method === "Target.getTargets") {
      return { targetInfos: this.targetInfos };
    }
    if (method === "Target.attachToTarget") return { sessionId: `${params.targetId}-session` };
    if (method === "Target.closeTarget") return { success: true };
    return {};
  }
}
