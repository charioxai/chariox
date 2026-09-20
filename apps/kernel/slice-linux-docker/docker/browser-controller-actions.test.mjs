import test from "node:test";
import assert from "node:assert/strict";

import {
  ACTION_ERROR_CODES,
  performBrowserAction,
} from "./browser-controller-actions.mjs";

const ELEMENT = Object.freeze({
  tab_id: "tab-7",
  document_id: "document-3",
  snapshot_revision: 4,
  backend_node_id: 91,
});

const READY = Object.freeze({
  state: "ready",
  x: 20,
  y: 30,
  width: 100,
  height: 40,
  editable: true,
});

class FakeConnection {
  constructor({ documentId = "document-3", actionability = [READY], resolveError, fillOk = true } = {}) {
    this.documentId = documentId;
    this.actionability = [...actionability];
    this.resolveError = resolveError;
    this.fillOk = fillOk;
    this.calls = [];
  }

  async send(method, params) {
    this.calls.push({ method, params });
    if (method === "Page.getFrameTree") {
      return { frameTree: { frame: { loaderId: this.documentId } } };
    }
    if (method === "DOM.resolveNode") {
      if (this.resolveError) throw this.resolveError;
      return { object: { objectId: "object-91" } };
    }
    if (method === "Runtime.callFunctionOn") {
      if (Array.isArray(params.arguments)) {
        return { result: { value: { ok: this.fillOk } } };
      }
      return {
        result: {
          value: this.actionability.length > 1
            ? this.actionability.shift()
            : this.actionability[0],
        },
      };
    }
    if (method === "Input.dispatchMouseEvent" || method === "Runtime.releaseObject") {
      return {};
    }
    throw new Error(`unexpected method ${method}`);
  }
}

function fakeTime() {
  let value = 0;
  return {
    now: () => value,
    sleep: async (milliseconds) => {
      value += milliseconds;
    },
  };
}

test("auto-waits for visible stable geometry before one trusted click", async () => {
  const time = fakeTime();
  const connection = new FakeConnection({
    actionability: [
      { state: "not_visible" },
      READY,
      READY,
    ],
  });
  const result = await performBrowserAction({
    connection,
    element: ELEMENT,
    action: { kind: "click" },
    timeoutMs: 500,
    ...time,
  });

  assert.deepEqual(result, {
    tab_id: "tab-7",
    document_id: "document-3",
    snapshot_revision: 4,
    action_kind: "click",
    attempts: 3,
    elapsed_ms: 100,
  });
  assert.deepEqual(
    connection.calls
      .filter((call) => call.method === "Input.dispatchMouseEvent")
      .map((call) => call.params.type),
    ["mouseMoved", "mousePressed", "mouseReleased"],
  );
  assert.equal(
    connection.calls.filter((call) => call.method === "Runtime.releaseObject").length,
    3,
  );
});

test("requires two equal actionable geometries before mutation", async () => {
  const time = fakeTime();
  const connection = new FakeConnection({
    actionability: [
      { ...READY, x: 10 },
      { ...READY, x: 11 },
      { ...READY, x: 11 },
    ],
  });
  const result = await performBrowserAction({
    connection,
    element: ELEMENT,
    action: { kind: "click" },
    ...time,
  });
  assert.equal(result.attempts, 3);
  assert.equal(
    connection.calls.filter((call) => call.method === "Input.dispatchMouseEvent").length,
    3,
  );
});

test("fills without returning or reporting the supplied text", async () => {
  const time = fakeTime();
  const connection = new FakeConnection({ actionability: [READY, READY] });
  const result = await performBrowserAction({
    connection,
    element: ELEMENT,
    action: { kind: "fill", text: "private-input", append: false },
    ...time,
  });
  assert.equal(result.action_kind, "fill");
  assert.doesNotMatch(JSON.stringify(result), /private-input/);
  const fillCall = connection.calls.find(
    (call) => call.method === "Runtime.callFunctionOn" && Array.isArray(call.params.arguments),
  );
  assert.deepEqual(fillCall.params.arguments, [
    { value: "private-input" },
    { value: false },
  ]);
});

test("bounds auto-wait and reports only the final actionability reason", async () => {
  const time = fakeTime();
  const connection = new FakeConnection({ actionability: [{ state: "disabled" }] });
  await assert.rejects(
    performBrowserAction({
      connection,
      element: ELEMENT,
      action: { kind: "click" },
      timeoutMs: 100,
      ...time,
    }),
    (error) => {
      assert.equal(error.code, ACTION_ERROR_CODES.TIMEOUT);
      assert.deepEqual(error.details, {
        attempts: 2,
        timeout_ms: 100,
        reason: "disabled",
      });
      return true;
    },
  );
  assert.equal(
    connection.calls.filter((call) => call.method === "Input.dispatchMouseEvent").length,
    0,
  );
});

test("rejects navigation and detached opaque references before mutation", async () => {
  await assert.rejects(
    performBrowserAction({
      connection: new FakeConnection({ documentId: "document-4" }),
      element: ELEMENT,
      action: { kind: "click" },
    }),
    (error) => error.code === ACTION_ERROR_CODES.STALE_DOCUMENT,
  );

  await assert.rejects(
    performBrowserAction({
      connection: new FakeConnection({
        resolveError: Object.assign(new Error("detached"), { code: "CDP_COMMAND_FAILED" }),
      }),
      element: ELEMENT,
      action: { kind: "click" },
    }),
    (error) => error.code === ACTION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED,
  );
});

test("rejects malformed actions and oversized fill text without CDP calls", async () => {
  const cases = [
    { kind: "click", unexpected: true },
    { kind: "fill", text: 42 },
    { kind: "fill", text: "x".repeat(48 * 1024 + 1) },
    { kind: "press", key: "Enter" },
  ];
  for (const action of cases) {
    const connection = new FakeConnection();
    await assert.rejects(
      performBrowserAction({ connection, element: ELEMENT, action }),
      (error) => error.code === ACTION_ERROR_CODES.INVALID_ARGUMENT,
    );
    assert.equal(connection.calls.length, 0);
  }
});

test("fails closed when a fill cannot be applied", async () => {
  const time = fakeTime();
  await assert.rejects(
    performBrowserAction({
      connection: new FakeConnection({ actionability: [READY, READY], fillOk: false }),
      element: ELEMENT,
      action: { kind: "fill", text: "value" },
      ...time,
    }),
    (error) => error.code === ACTION_ERROR_CODES.FAILED,
  );
});
