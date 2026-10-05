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
    if (method === "Runtime.evaluate") {
      const sensitive = typeof value === "function" ? value() : value;
      return { result: sensitive === false ? { objectId:"targets", subtype:"array" } : { value:sensitive } };
    }
    if (method === "Runtime.getProperties") return { result:[] };
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
  assert.equal(scan.returnByValue, false);
  assert.match(scan.expression, /autocomplete/);
  await sensitiveHostInput(browser, tab, {kind:'text',text:'MP-11-fixture-input-content'});
  assert(!sent.filter(({method}) => method === 'Runtime.evaluate').some(({params}) => params.expression.includes('MP-11-fixture-input-content')));
  assert.equal(await sensitiveHostInput(browser, tab, { kind: "scroll" }), false);
});
test("MP-08/MP-11: retained Tab releases after focus moves onto a sensitive button", async () => {
  let sensitive = false;
  const { browser, sent } = fixture(() => sensitive);
  const target = await browser.resolvePageTarget();
  const send = target.connection.send;
  target.connection.send = async (method, params) => {
    const result = await send(method, params);
    if (method === "Input.dispatchKeyEvent" && params.type === "keyDown") sensitive = true;
    return result;
  };
  await inputHostTab(browser, tab, { kind: "key", key: "Tab" }, { requireRoutine: true });
  assert.deepEqual(sent.filter(call => call.method === "Input.dispatchKeyEvent").map(call => call.params.type), ["keyDown", "keyUp"]);
});
test("MP-11: Tab release still refuses revoked authority", async () => {
  const { browser, sent } = fixture(false);
  const cancellation = new AbortController();
  const target = await browser.resolvePageTarget();
  const send = target.connection.send;
  target.connection.send = async (method, params) => {
    const result = await send(method, params);
    if (method === "Input.dispatchKeyEvent" && params.type === "keyDown") cancellation.abort();
    return result;
  };
  await assert.rejects(inputHostTab(browser, tab, { kind: "key", key: "Tab" }, { requireRoutine: true, signal: cancellation.signal }), { code: "browser_action_cancelled" });
  assert.equal(sent.filter(call => call.method === "Input.dispatchKeyEvent").length, 1);
});

test("MP-11: unavailable native listener metadata refuses input and releases remote objects", async () => {
  for (const failing of ['DOM.describeNode', 'DOM.resolveNode', 'DOMDebugger.getEventListeners', 'Runtime.callFunctionOn']) {
    const { browser, sent } = fixture(false);
    const { connection } = await browser.resolvePageTarget();
    const send = connection.send;
    connection.send = async (method, params) => {
      const result = await send(method, params);
      if (method === failing) throw new Error('MP-11: unavailable fixture metadata');
      if (method === 'Runtime.getProperties') return { result:[{name:'0',value:{objectId:'isolated-node',subtype:'node'}}] };
      if (method === 'DOM.describeNode') return {node:{backendNodeId:7}};
      if (method === 'DOM.resolveNode') return {object:{objectId:'main-node'}};
      if (method === 'DOMDebugger.getEventListeners') {
        assert.equal(params.objectId, 'main-node');
        return {listeners:[{type:'click',backendNodeId:7}]};
      }
      return result;
    };
    await assert.rejects(inputHostTab(browser, tab, {kind:'click',x:1,y:2}, {requireRoutine:true}), {code:'sensitive_requires_focus'});
    assert(!sent.some(({method}) => method.startsWith('Input.')));
    assert(sent.some(({method}) => method === 'Runtime.releaseObjectGroup'));
  }
});
