import assert from "node:assert/strict";
import test from "node:test";

import {
  BrowserActionError,
  performBrowserAction,
} from "./browser-controller-actions.mjs";

test("click auto-waits for a stable actionable element and uses native input", async () => {
  const connection = new FakeActionConnection([
    { state: "not_visible" },
    { state: "ready", x: 50, y: 75, width: 100, height: 30 },
    { state: "ready", x: 50, y: 75, width: 100, height: 30 },
  ]);
  let now = 0;

  const result = await performBrowserAction({
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    nodeRef: "backend:103",
    action: { kind: "click" },
    timeoutMs: 500,
    now: () => now,
    sleep: async (milliseconds) => { now += milliseconds; },
  });

  assert.equal(result.action_kind, "click");
  assert.equal(result.attempts, 3);
  assert.deepEqual(
    connection.calls
      .filter((call) => call.method === "Input.dispatchMouseEvent")
      .map((call) => call.params.type),
    ["mouseMoved", "mousePressed", "mouseReleased"],
  );
  assert.ok(
    connection.calls
      .filter((call) => call.method === "Page.getFrameTree")
      .every((call) => call.sessionId === "session-a"),
  );
});

test("fill auto-waits, replaces existing text, and inserts through native input", async () => {
  const connection = new FakeActionConnection([
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
  ]);

  const result = await performBrowserAction({
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    nodeRef: "backend:104",
    action: { kind: "fill", text: "hello", append: false },
    timeoutMs: 500,
    sleep: async () => {},
  });

  assert.equal(result.action_kind, "fill");
  assert.equal(
    connection.calls.find((call) => call.method === "Input.insertText").params.text,
    "hello",
  );
  const focusCall = connection.calls.find(
    (call) =>
      call.method === "Runtime.callFunctionOn" &&
      !call.params.functionDeclaration.includes("scrollIntoView"),
  );
  assert.equal(
    focusCall.params.arguments[0].value,
    true,
    "replacement fill selects the existing value before insertion",
  );
});

test("submit auto-waits and submits the nearest form through the resolved element", async () => {
  const connection = new FakeActionConnection([
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
  ]);

  const result = await performBrowserAction({
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    nodeRef: "backend:104",
    action: { kind: "submit" },
    timeoutMs: 500,
    sleep: async () => {},
  });

  assert.equal(result.action_kind, "submit");
  assert.ok(connection.calls.some(
    (call) =>
      call.method === "Runtime.callFunctionOn" &&
      call.params.functionDeclaration.includes("requestSubmit"),
  ));
});

test("fill can atomically submit the same resolved form", async () => {
  const connection = new FakeActionConnection([
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
  ]);

  const result = await performBrowserAction({
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    nodeRef: "backend:104",
    action: { kind: "fill", text: "secret", append: false, submit: true },
    timeoutMs: 500,
    sleep: async () => {},
  });

  assert.equal(result.action_kind, "fill");
  assert.ok(connection.calls.some(
    (call) =>
      call.method === "Runtime.callFunctionOn" &&
      call.params.functionDeclaration.includes("requestSubmit"),
  ));
});

for (const dialogEventType of ["mousePressed", "mouseReleased"]) {
  test(`click returns when ${dialogEventType} opens a dialog without waiting for its blocked response`, async () => {
    const connection = new FakeActionConnection([
      { state: "ready", x: 50, y: 75, width: 100, height: 30 },
      { state: "ready", x: 50, y: 75, width: 100, height: 30 },
    ]);
    connection.dialogEventType = dialogEventType;

    const result = await performBrowserAction({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:103",
      action: { kind: "click" },
      timeoutMs: 500,
      sleep: async () => {},
    });

    assert.equal(result.action_kind, "click");
    assert.equal(result.dialog_opened, true);
  });
}

test("locator actions reject stale documents and time out with stable codes", async () => {
  const stale = new FakeActionConnection([]);
  stale.loaderId = "loader-b";
  await assert.rejects(
    performBrowserAction({
      connection: stale,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:103",
      action: { kind: "click" },
    }),
    (error) => error instanceof BrowserActionError && error.code === "stale_document_reference",
  );

  const blocked = new FakeActionConnection([{ state: "disabled" }]);
  let now = 0;
  await assert.rejects(
    performBrowserAction({
      connection: blocked,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:103",
      action: { kind: "click" },
      timeoutMs: 100,
      now: () => now,
      sleep: async (milliseconds) => { now += milliseconds; },
    }),
    (error) =>
      error instanceof BrowserActionError &&
      error.code === "browser_action_timeout" &&
      error.reason === "disabled",
  );

  const readOnly = new FakeActionConnection([
    { state: "ready", x: 20, y: 20, width: 100, height: 20, editable: false },
  ]);
  now = 0;
  await assert.rejects(
    performBrowserAction({
      connection: readOnly,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:103",
      action: { kind: "fill", text: "no", append: false },
      timeoutMs: 100,
      now: () => now,
      sleep: async (milliseconds) => { now += milliseconds; },
    }),
    (error) =>
      error.code === "browser_action_timeout" && error.reason === "not_editable",
  );

  const disconnected = new FakeActionConnection([]);
  disconnected.resolveError = Object.assign(new Error("CDP timed out"), {
    code: "browser_cdp_timeout",
  });
  await assert.rejects(
    performBrowserAction({
      connection: disconnected,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:103",
      action: { kind: "click" },
    }),
    (error) => error.code === "browser_cdp_timeout",
  );
});

class FakeActionConnection {
  constructor(actionability) {
    this.actionability = actionability;
    this.calls = [];
    this.loaderId = "loader-a";
  }

  async send(method, params = {}, sessionId) {
    this.calls.push({ method, params, sessionId });
    if (method === "Page.getFrameTree") {
      return { frameTree: { frame: { loaderId: this.loaderId } } };
    }
    if (method === "DOM.resolveNode") {
      if (this.resolveError) throw this.resolveError;
      return { object: { objectId: "object-1" } };
    }
    if (method === "Runtime.callFunctionOn") {
      if (params.functionDeclaration.includes("scrollIntoView")) {
        return {
          result: {
            value: {
              ...(this.actionability.length > 1
                ? this.actionability.shift()
                : this.actionability[0]),
            },
          },
        };
      }
      return { result: { value: { ok: true } } };
    }
    if (method === "Runtime.releaseObject") {
      return {};
    }
    if (method.startsWith("Input.")) {
      if (method === "Input.dispatchMouseEvent" && params.type === this.dialogEventType) {
        this.dialogWaiter?.({ method: "Page.javascriptDialogOpening" });
        return new Promise(() => {});
      }
      return {};
    }
    throw new Error(`unexpected CDP method ${method}`);
  }

  waitForEvent() {
    let resolve;
    const promise = new Promise((eventResolve) => {
      resolve = eventResolve;
      this.dialogWaiter = eventResolve;
    });
    return {
      promise,
      cancel: () => {
        if (this.dialogWaiter === resolve) this.dialogWaiter = null;
        resolve(null);
      },
    };
  }
}
