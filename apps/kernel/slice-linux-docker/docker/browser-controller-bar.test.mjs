import assert from "node:assert/strict";
import test from "node:test";
import { applyBrowserBar } from "./browser-controller-bar.mjs";

function fakeBrowser(windows, windowOfTarget) {
  const calls = [];
  return {
    calls,
    windows,
    connection: {
      async send(method, params) {
        calls.push([method, params]);
        if (method === "Browser.getWindowForTarget") {
          if (!(params.targetId in windowOfTarget)) throw new Error("No target with given id found");
          return { windowId: windowOfTarget[params.targetId] };
        }
        if (method === "Browser.getWindowBounds") return { bounds: { windowState: windows[params.windowId] } };
        if (method === "Browser.setWindowBounds") {
          windows[params.windowId] = params.bounds.windowState;
          return {};
        }
        throw new Error(`unexpected ${method}`);
      },
    },
  };
}

test("ordinary Tabs' windows are fullscreen while the bar is hidden; App view windows are left alone", async () => {
  const fake = fakeBrowser({ 1: "normal", 2: "maximized", 3: "fullscreen" }, { a: 1, b: 1, c: 2, app: 3 });
  const pages = ["a", "b", "c", "app", "gone"].map((targetId) => ({ targetId }));
  assert.equal(await applyBrowserBar(fake.connection, pages, new Set(["app"]), false), 2);
  assert.deepEqual(fake.windows, { 1: "fullscreen", 2: "fullscreen", 3: "fullscreen" });
  // Chromium leaves maximized through normal; one window holding two Tabs is set once.
  const sets = fake.calls.filter(([method]) => method === "Browser.setWindowBounds").map(([, p]) => [p.windowId, p.bounds.windowState]);
  assert.deepEqual(sets, [[1, "fullscreen"], [2, "normal"], [2, "fullscreen"]]);
});

test("showing the bar maximizes ordinary windows, and a window already right is not touched", async () => {
  const fake = fakeBrowser({ 1: "fullscreen", 2: "maximized", 3: "fullscreen" }, { a: 1, c: 2, app: 3 });
  await applyBrowserBar(fake.connection, [{ targetId: "a" }, { targetId: "c" }, { targetId: "app" }], new Set(["app"]), true);
  assert.deepEqual(fake.windows, { 1: "maximized", 2: "maximized", 3: "fullscreen" });
  assert.ok(!fake.calls.some(([method, p]) => method === "Browser.setWindowBounds" && p.windowId === 2));
});

test("a window shared with an App view keeps the App view's state", async () => {
  const fake = fakeBrowser({ 1: "fullscreen" }, { a: 1, app: 1 });
  assert.equal(await applyBrowserBar(fake.connection, [{ targetId: "a" }], new Set(["app"]), true), 0);
  assert.deepEqual(fake.windows, { 1: "fullscreen" });
});
