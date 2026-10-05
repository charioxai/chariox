// MP-08/MP-11: authority-sensitive native dispatch, without page values.
import test from "node:test";
import assert from "node:assert/strict";
import { sensitiveHostInput, inputHostTab } from "./kernel-browser-input.mjs";

function fixture(value) {
  const sent = [];
  const connection = { send: async (method, params) => {
    sent.push({ method, params });
    if (method === "Page.getFrameTree") return { frameTree: { frame: { id: "f", loaderId: "doc" } } };
    if (method === "Page.createIsolatedWorld") return { executionContextId: 7 };
    if (method === "Runtime.evaluate") return { result: { value } };
    if (method === "DOM.getDocument") return { root: { nodeId: 1 } };
    if (method === "DOM.querySelectorAll") return { nodeIds: [] };
    return {};
  } };
  return { sent, browser: { resolvePageTarget: async () => ({ connection, sessionId: "s" }), inputCapture: { run: async (_c, _s, fn) => fn() } } };
}
const tab = { target_id: "t", document_id: "doc" };
test("MP-11: protected, payment and unknown classifications block retained physical input", async () => {
  for (const value of [true, undefined, null]) {
    const { browser, sent } = fixture(value);
    assert.equal(await sensitiveHostInput(browser, tab, { kind: "click", x: 1, y: 2 }), true);
    await assert.rejects(inputHostTab(browser, tab, { kind: "key", key: "Enter" }, { requireRoutine: true }), /requires focus/);
    assert(!sent.some(({ method }) => method.startsWith("Input.")));
  }
});
test("MP-08: routine retained click and scroll proceed; classification returns no values", async () => {
  const { browser, sent } = fixture(false);
  await inputHostTab(browser, tab, { kind: "click", x: 1, y: 2 }, { requireRoutine: true });
  assert.equal(sent.filter(({ method }) => method === "Input.dispatchMouseEvent").length, 2);
  const scan = sent.find(({ method }) => method === "Runtime.evaluate").params;
  assert.equal(scan.returnByValue, true);
  assert.match(scan.expression, /autocomplete/);
  assert.equal(await sensitiveHostInput(browser, tab, { kind: "scroll" }), false);
});
