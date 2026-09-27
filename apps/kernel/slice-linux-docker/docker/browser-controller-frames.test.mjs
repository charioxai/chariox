import assert from "node:assert/strict";
import test from "node:test";
import { BrowserFrameSessions, BrowserFrameTargets } from "./browser-controller-frames.mjs";

const tree = (id, parentId, childFrames = []) => ({ frame: { id, parentId }, childFrames });

test("isolated listeners observe late descendants and close their owned sessions", async () => {
  const registry = new BrowserFrameTargets();
  registry.replace("tab", [tree("root")]);
  const listener = new BrowserFrameSessions(registry, (session) => session === "root-session" ? "tab" : undefined);
  const connection = frameConnection();
  assert.equal(listener.observe(attached("child", "root-session"), connection), true);
  await settle();
  assert.equal(registry.get("child"), "tab");
  assert.equal(listener.observe(attached("grandchild", "child"), connection), true);
  await settle();
  listener.observe({ method: "Page.frameAttached", sessionId: "grandchild", params: { frameId: "late", parentFrameId: "grandchild" } }, connection);
  assert.equal(registry.get("late"), "tab");
  const start = connection.calls.length;
  await listener.close();
  for (const id of ["child", "grandchild"]) {
    const cleanup = connection.calls.slice(start);
    assert.ok(cleanup.some((call) => call.method === "Runtime.runIfWaitingForDebugger" && call.sessionId === id));
    assert.ok(cleanup.some((call) => call.method === "Target.detachFromTarget" && call.params.sessionId === id));
    assert.equal(listener.observe({ method: "Page.frameAttached", sessionId: id, params: {} }, connection), false);
  }
  assert.equal(registry.get("root"), "tab");
});

test("failed or removed frame initialization resumes the child without retaining it", async () => {
  for (const failure of ["enable", "removed"]) {
    const registry = new BrowserFrameTargets();
    const listener = new BrowserFrameSessions(registry, () => "tab");
    let unblock;
    const gate = new Promise((resolve) => { unblock = resolve; });
    const connection = frameConnection(async (method) => {
      if (failure === "enable" && method === "Page.enable") throw new Error("disconnected");
      if (failure === "removed" && method === "Page.getFrameTree") await gate;
    });
    listener.observe(attached("child", "root-session"), connection);
    await settle();
    if (failure === "removed") { listener.clear(); unblock(); await settle(); }
    assert.equal(registry.get("child"), undefined);
    assert.ok(connection.calls.some((call) => call.method === "Runtime.runIfWaitingForDebugger"));
    assert.ok(connection.calls.some((call) => call.method === "Target.detachFromTarget"));
    assert.equal(listener.observe({ method: "Page.frameAttached", sessionId: "child", params: {} }, connection), false);
  }
});

test("detaching a parent frame drops descendant sessions without dropping another tab", async () => {
  const listener = new BrowserFrameSessions(new BrowserFrameTargets(), (id) => ({ root: "a", other: "b" })[id]);
  const connection = frameConnection();
  listener.observe(attached("child", "root"), connection);
  listener.observe(attached("other-child", "other"), connection);
  await settle();
  listener.observe(attached("grandchild", "child"), connection);
  await settle();
  listener.observe({ method: "Target.detachedFromTarget", params: { sessionId: "child" } }, connection);
  await settle();
  assert.equal(listener.observe({ method: "Page.frameAttached", sessionId: "grandchild", params: {} }, connection), false);
  assert.equal(listener.observe({ method: "Page.frameAttached", sessionId: "other-child", params: {} }, connection), true);
  await listener.close();
});

test("isolated session admission resumes overflow without opening another listener", async () => {
  const listener = new BrowserFrameSessions(new BrowserFrameTargets(), () => "tab");
  const connection = frameConnection();
  for (let i = 0; i < 65; i++) listener.observe(attached(`child-${i}`, "root"), connection);
  await settle();
  assert.equal(connection.calls.filter((call) => call.method === "Page.enable").length, 64);
  assert.ok(connection.calls.some((call) => call.method === "Runtime.runIfWaitingForDebugger" && call.sessionId === "child-64"));
  assert.ok(connection.calls.some((call) => call.method === "Target.detachFromTarget" && call.params.sessionId === "child-64"));
  await listener.close();
});

function attached(sessionId, parent) {
  return { method: "Target.attachedToTarget", sessionId: parent, params: { sessionId, targetInfo: { type: "iframe" } } };
}

test("an orphaned paused attachment is resumed but manual frame attachments stay caller-owned", async () => {
  const listener = new BrowserFrameSessions(new BrowserFrameTargets(), () => undefined);
  const connection = frameConnection();
  const paused = attached("orphan", "removed-root");
  paused.params.waitingForDebugger = true;
  assert.equal(listener.observe(paused, connection), true);
  await settle();
  assert.deepEqual(connection.calls.map((call) => call.method), ["Runtime.runIfWaitingForDebugger", "Target.detachFromTarget"]);
  assert.equal(listener.observe(attached("manual", undefined), connection), false);
  assert.equal(connection.calls.length, 2);
});

function frameConnection(beforeSend = async () => {}) {
  return {
    calls: [],
    async send(method, params = {}, sessionId) {
      this.calls.push({ method, params, sessionId });
      await beforeSend(method);
      return method === "Page.getFrameTree" ? { frameTree: tree(sessionId, "root") } : {};
    },
  };
}

function settle() { return new Promise((resolve) => setImmediate(resolve)); }

test("frame ownership covers isolated trees and removes descendants without losing other tabs", () => {
  const registry = new BrowserFrameTargets();
  registry.replace("a", [tree("root-a", undefined, [tree("child-a", "root-a")]), tree("isolated-a", "child-a", [tree("grandchild-a", "isolated-a")])]);
  registry.replace("b", [tree("root-b")]);
  for (const id of ["root-a", "child-a", "isolated-a", "grandchild-a"]) assert.equal(registry.get(id), "a");
  registry.record({ method: "Page.frameDetached", params: { frameId: "child-a", reason: "swap" } }, "a");
  assert.equal(registry.get("grandchild-a"), "a", "renderer swaps preserve logical frame ownership");
  registry.record({ method: "Page.frameDetached", params: { frameId: "child-a", reason: "remove" } }, "a");
  for (const id of ["child-a", "isolated-a", "grandchild-a"]) assert.equal(registry.get(id), undefined);
  assert.equal(registry.get("root-a"), "a");
  assert.equal(registry.get("root-b"), "b");
  registry.removeTarget("a");
  assert.equal(registry.get("root-a"), undefined);
  assert.equal(registry.get("root-b"), "b");
  registry.clear();
  assert.equal(registry.get("root-b"), undefined);
});

test("late frame events update ownership while root navigation replaces the old tree", () => {
  const registry = new BrowserFrameTargets();
  registry.replace("a", [tree("root-a")]);
  registry.record({ method: "Page.frameAttached", params: { frameId: "late", parentFrameId: "root-a" } }, "a");
  registry.record({ method: "Page.frameNavigated", params: { frame: { id: "late", parentId: "root-a" } } }, "a");
  assert.equal(registry.get("late"), "a");
  registry.record({ method: "Page.frameNavigated", params: { frame: { id: "root-a" } } }, "a");
  assert.equal(registry.get("late"), undefined);
  assert.equal(registry.get("root-a"), "a");
});

test("frame retention has a per-tab bound and releases capacity on removal", () => {
  const registry = new BrowserFrameTargets();
  registry.replace("a", [tree("root-a")]);
  for (let i = 0; i < 1_000; i++) registry.record({ method: "Page.frameAttached", params: { frameId: `child-${i}`, parentFrameId: "root-a" } }, "a");
  assert.equal(registry.get("child-62"), "a");
  assert.equal(registry.get("child-63"), undefined);
  registry.record({ method: "Page.frameDetached", params: { frameId: "child-0", reason: "remove" } }, "a");
  registry.record({ method: "Page.frameAttached", params: { frameId: "new", parentFrameId: "root-a" } }, "a");
  assert.equal(registry.get("new"), "a");
  registry.replace("b", [tree("root-b")]);
  assert.equal(registry.get("root-b"), "b");
});
