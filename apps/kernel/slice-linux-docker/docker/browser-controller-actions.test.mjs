import assert from "node:assert/strict";
import test from "node:test";

import {
  BrowserActionError,
  actionabilityFunction,
  fillFunction,
  fillRequestBytes,
  MAX_FILL_REQUEST_BYTES,
  performBrowserAction,
} from "./browser-controller-actions.mjs";

const ACTION_ERROR_CODES = Object.freeze({
  TIMEOUT: "browser_action_timeout",
  POST_ACTION_UNCERTAIN: "browser_action_post_action_uncertain",
  INVALID_ARGUMENT: "browser_action_invalid",
});

function fakeTime(start = 0) {
  let value = start;
  const sleeps = [];
  return {
    now: () => value,
    advance: (milliseconds) => { value += milliseconds; },
    sleep: async (milliseconds) => {
      sleeps.push(milliseconds);
      value += milliseconds;
    },
    sleeps,
  };
}

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

test("detached elements reject before polling or input and release their remote object", async () => {
  const connection = new FakeActionConnection([{ state: "detached" }]);
  await assert.rejects(performBrowserAction({
    connection, sessionId: "session-a", targetId: "target-a", documentId: "loader-a",
    nodeRef: "backend:103", action: { kind: "fill", text: "not inserted" },
    sleep: async () => { throw new Error("detached references must not poll"); },
  }), { code: "stale_element_reference" });
  assert.equal(connection.calls.some((call) => call.method.startsWith("Input.")), false);
  assert.equal(connection.calls.some((call) => call.method === "Runtime.releaseObject"), true);
});

test("actionability accepts a hit inside the target's own shadow tree", () => {
  const ownerWindow = {
    getComputedStyle: () => ({ display: "block", visibility: "visible", opacity: "1" }),
  };
  ownerWindow.top = ownerWindow;
  const shadowRoot = {};
  const inner = { getRootNode: () => shadowRoot };
  const target = {
    isConnected: true,
    scrollIntoView: () => {},
    getBoundingClientRect: () => ({ left: 10, top: 20, width: 100, height: 30 }),
    matches: () => false,
    closest: () => null,
    getAttribute: () => null,
    contains: () => false,
    isContentEditable: false,
  };
  shadowRoot.host = target;
  target.shadowRoot = { elementFromPoint: () => inner };
  target.ownerDocument = {
    defaultView: ownerWindow,
    elementFromPoint: () => target,
  };

  assert.equal(actionabilityFunction.call(target).state, "ready");
});

test("actionability converts nested-frame coordinates through border and padding insets", () => {
  const topWindow = {
    getComputedStyle: () => ({ paddingLeft: "4px", paddingTop: "5px" }),
  };
  topWindow.top = topWindow;
  const frameElement = {
    clientLeft: 2,
    clientTop: 3,
    getBoundingClientRect: () => ({ left: 100, top: 50 }),
    ownerDocument: { defaultView: topWindow },
  };
  const ownerWindow = {
    top: topWindow,
    frameElement,
    getComputedStyle: () => ({ display: "block", visibility: "visible", opacity: "1" }),
  };
  const target = {
    isConnected: true,
    scrollIntoView: () => {},
    getBoundingClientRect: () => ({ left: 10, top: 20, width: 20, height: 10 }),
    matches: () => false,
    closest: () => null,
    getAttribute: () => null,
    contains: () => false,
    isContentEditable: false,
  };
  target.ownerDocument = {
    defaultView: ownerWindow,
    elementFromPoint: () => target,
  };

  const result = actionabilityFunction.call(target);

  assert.equal(result.state, "ready");
  assert.equal(result.x, 126);
  assert.equal(result.y, 83);
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

test("secret fill rejects a same-document URL change before inserting the secret", async () => {
  const connection = new FakeActionConnection([
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
  ]);
  connection.documentUrl = "https://example.test/recovery";

  await assert.rejects(
    performBrowserAction({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:104",
      action: {
        kind: "fill",
        text: "secret-canary",
        append: false,
        submit: false,
        expected_document_url: "https://example.test/login",
      },
      timeoutMs: 500,
      sleep: async () => {},
    }),
    (error) =>
      error instanceof BrowserActionError &&
      error.code === "browser_secret_target_changed",
  );

  assert.equal(
    connection.calls.some((call) => call.method === "Input.insertText"),
    false,
    "the secret must not reach native input after the target URL changes",
  );
});

test("secret fill writes only through the document-bound operation", async () => {
  const connection = new FakeActionConnection([
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
  ]);
  connection.documentUrl = "https://example.test/login";

  const result = await performBrowserAction({
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    nodeRef: "backend:104",
    action: {
      kind: "fill",
      text: "secret-canary",
      append: false,
      submit: true,
      expected_document_url: "https://example.test/login",
    },
    timeoutMs: 500,
    sleep: async () => {},
  });

  assert.equal(result.action_kind, "fill");
  assert.equal(JSON.stringify(result).includes("secret-canary"), false);
  assert.equal(connection.calls.some((call) => call.method === "Input.insertText"), false);
  const secureFill = connection.calls.find(
    (call) =>
      call.method === "Runtime.callFunctionOn" &&
      call.params.functionDeclaration.includes("expectedDocumentUrl"),
  );
  assert.equal(secureFill.params.arguments[0].value, "secret-canary");
  assert.equal(secureFill.params.arguments[2].value, "https://example.test/login");
  assert.equal(secureFill.params.arguments[3].value, true);
});

test("secret fill distinguishes a focus failure from a document URL change", async () => {
  const connection = new FakeActionConnection([
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
  ]);
  connection.documentUrl = "https://example.test/login";
  connection.secureFillFocusSucceeded = false;

  await assert.rejects(
    performBrowserAction({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:104",
      action: {
        kind: "fill",
        text: "secret-canary",
        append: false,
        submit: false,
        expected_document_url: "https://example.test/login",
      },
      timeoutMs: 500,
      sleep: async () => {},
    }),
    (error) =>
      error instanceof BrowserActionError &&
      error.code === "browser_secret_target_not_focusable",
  );

  const secureFill = connection.calls.find(
    (call) =>
      call.method === "Runtime.callFunctionOn" &&
      call.params.functionDeclaration.includes("expectedDocumentUrl"),
  );
  assert.match(secureFill.params.functionDeclaration, /reason: "target_not_focusable"/);
  assert.equal(connection.calls.some((call) => call.method === "Input.insertText"), false);
});

test("secret fill rejects a password field that becomes unmasked while focusing", async () => {
  const connection = new FakeActionConnection([
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
  ]);
  connection.documentUrl = "https://example.test/login";
  connection.secureFillMaskedAfterFocus = false;

  await assert.rejects(
    performBrowserAction({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:104",
      action: {
        kind: "fill",
        text: "secret-canary",
        append: false,
        submit: false,
        expected_document_url: "https://example.test/login",
      },
      timeoutMs: 500,
      sleep: async () => {},
    }),
    (error) =>
      error instanceof BrowserActionError &&
      error.code === "browser_secret_target_not_masked",
  );

  const secureFill = connection.calls.find(
    (call) =>
      call.method === "Runtime.callFunctionOn" &&
      call.params.functionDeclaration.includes("expectedDocumentUrl"),
  );
  assert.equal(
    secureFill.params.functionDeclaration.match(/reason: "target_not_masked"/g)?.length,
    2,
    "the operation must verify masking both before and after focus handlers run",
  );
  assert.equal(connection.calls.some((call) => call.method === "Input.insertText"), false);
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
    assert.ok(connection.dialogWaitTimeoutMs >= 5_000);
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

test("expires a queued action at its advertised absolute deadline", async () => {
  const time = fakeTime(90);
  const connection = new FakeActionConnection([{ state: "not_visible" }]);

  await assert.rejects(
    performBrowserAction({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:103",
      action: { kind: "click" },
      timeoutMs: 500,
      deadline: 100,
      ...time,
    }),
    (error) => error.code === ACTION_ERROR_CODES.TIMEOUT,
  );
  assert.deepEqual(time.sleeps, [10]);
  assert.equal(time.now(), 100);
  assert.equal(connection.calls.some((call) => call.method.startsWith("Input.")), false);
});

test("caps every actionability poll sleep at the remaining deadline", async () => {
  const time = fakeTime();
  const connection = new FakeActionConnection([{ state: "disabled" }]);

  await assert.rejects(
    performBrowserAction({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:103",
      action: { kind: "click" },
      timeoutMs: 125,
      ...time,
    }),
    (error) => error.code === ACTION_ERROR_CODES.TIMEOUT,
  );
  assert.deepEqual(time.sleeps, [50, 50, 25]);
  assert.equal(time.now(), 125);
});

test("does not retry when release crosses the deadline after a click", async () => {
  const time = fakeTime();
  const connection = new FakeActionConnection({
    actionability: [
      { state: "ready", x: 50, y: 75, width: 100, height: 30 },
      { state: "ready", x: 50, y: 75, width: 100, height: 30 },
    ],
    afterSend: (method) => {
      if (
        method === "Runtime.releaseObject" &&
        connection.calls.filter((call) => call.method === "Runtime.releaseObject").length === 2
      ) {
        time.advance(51);
      }
    },
  });

  await assert.rejects(
    performBrowserAction({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:103",
      action: { kind: "click" },
      timeoutMs: 100,
      ...time,
    }),
    (error) => {
      assert.equal(error.code, ACTION_ERROR_CODES.POST_ACTION_UNCERTAIN);
      assert.equal(error.outcome, "post_action_uncertain");
      assert.equal(error.phase, "cleanup");
      return true;
    },
  );
  assert.deepEqual(
    connection.calls
      .filter((call) => call.method === "Input.dispatchMouseEvent")
      .map((call) => call.params.type),
    ["mouseMoved", "mousePressed", "mouseReleased"],
  );
});

test("reports an uncertain fill when its mutation response crosses the deadline", async () => {
  const time = fakeTime();
  const connection = new FakeActionConnection({
    actionability: [
      { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
      { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
    ],
    afterSend: (method) => {
      if (method === "Input.insertText") time.advance(51);
    },
  });

  await assert.rejects(
    performBrowserAction({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:104",
      action: { kind: "fill", text: "one-shot", append: false },
      timeoutMs: 100,
      ...time,
    }),
    (error) => {
      assert.equal(error.code, ACTION_ERROR_CODES.POST_ACTION_UNCERTAIN);
      assert.equal(error.outcome, "post_action_uncertain");
      assert.equal(error.phase, "fill");
      return true;
    },
  );
  assert.equal(
    connection.calls.filter((call) => call.method === "Input.insertText").length,
    1,
  );
});

test("selects a duplicate-valued option by exact visible index and selected state", () => {
  const previousDocument = globalThis.document;
  const previousEvent = globalThis.Event;
  const options = [
    { value: "shared", text: "first", selected: true },
    { value: "shared", text: "second", selected: false },
  ];
  let selectedIndex = 0;
  const document = { activeElement: null };
  const control = {
    isConnected: true,
    disabled: false,
    readOnly: false,
    tagName: "select",
    options,
    value: options[0].value,
    get selectedIndex() { return selectedIndex; },
    set selectedIndex(index) {
      selectedIndex = index;
      options.forEach((option, optionIndex) => {
        option.selected = optionIndex === index;
      });
      this.value = options[index]?.value ?? "";
    },
    focus() { document.activeElement = this; },
    dispatchEvent() {},
  };
  globalThis.document = document;
  globalThis.Event = class Event {
    constructor(type, init) {
      this.type = type;
      this.init = init;
    }
  };
  try {
    assert.deepEqual(fillFunction.call(control, "second", false), { ok: true });
    assert.equal(control.selectedIndex, 1);
    assert.equal(control.value, "shared");
    assert.equal(options[0].selected, false);
    assert.equal(options[1].selected, true);
  } finally {
    if (previousDocument === undefined) delete globalThis.document;
    else globalThis.document = previousDocument;
    if (previousEvent === undefined) delete globalThis.Event;
    else globalThis.Event = previousEvent;
  }
});

test("follows an open shadow root for both actionability and fill focus", () => {
  const previousDocument = globalThis.document;
  const previousWindow = globalThis.window;
  const previousEvent = globalThis.Event;
  const shadowRoot = { activeElement: null, host: null };
  const host = { shadowRoot, parentElement: null, getRootNode: () => document };
  shadowRoot.host = host;
  const document = {
    activeElement: host,
    elementFromPoint: () => host,
  };
  const control = {
    isConnected: true,
    disabled: false,
    readOnly: false,
    tagName: "input",
    type: "text",
    value: "",
    parentElement: null,
    getRootNode: () => shadowRoot,
    scrollIntoView() {},
    getBoundingClientRect: () => ({ left: 10, top: 20, width: 100, height: 20 }),
    matches: () => false,
    closest: () => null,
    getAttribute: () => null,
    contains: () => false,
    focus() {
      shadowRoot.activeElement = control;
      document.activeElement = host;
    },
    dispatchEvent() {},
  };
  globalThis.document = document;
  globalThis.window = {
    getComputedStyle: () => ({ display: "block", visibility: "visible", opacity: "1" }),
  };
  globalThis.window.top = globalThis.window;
  globalThis.Event = class Event {};
  try {
    assert.deepEqual(actionabilityFunction.call(control), {
      state: "ready",
      x: 60,
      y: 30,
      width: 100,
      height: 20,
      editable: true,
    });
    assert.deepEqual(fillFunction.call(control, "shadow-value", false), { ok: true });
    assert.equal(control.value, "shadow-value");
  } finally {
    if (previousDocument === undefined) delete globalThis.document;
    else globalThis.document = previousDocument;
    if (previousWindow === undefined) delete globalThis.window;
    else globalThis.window = previousWindow;
    if (previousEvent === undefined) delete globalThis.Event;
    else globalThis.Event = previousEvent;
  }
});

test("bounds escape-heavy fill JSON before it reaches the CDP frame", async () => {
  const escapedUnit = "\\\"";
  let acceptedText = "";
  while (fillRequestBytes("object-1", acceptedText + escapedUnit, false) < MAX_FILL_REQUEST_BYTES) {
    acceptedText += escapedUnit;
  }
  const acceptedBytes = fillRequestBytes("object-1", acceptedText, false);
  const rejectedBytes = fillRequestBytes("object-1", acceptedText + escapedUnit, false);
  assert.ok(acceptedText.length > 0);
  assert.ok(acceptedBytes < MAX_FILL_REQUEST_BYTES);
  assert.ok(rejectedBytes >= MAX_FILL_REQUEST_BYTES);

  const acceptedConnection = new FakeActionConnection({
    actionability: [
      { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
      { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
    ],
  });
  await performBrowserAction({
    connection: acceptedConnection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    nodeRef: "backend:104",
    action: { kind: "fill", text: acceptedText, append: false },
    ...fakeTime(),
  });
  assert.equal(
    acceptedConnection.calls.filter((call) => call.method === "Input.insertText").length,
    1,
  );

  const rejectedConnection = new FakeActionConnection({
    actionability: [
      { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
      { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
    ],
  });
  await assert.rejects(
    performBrowserAction({
      connection: rejectedConnection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:104",
      action: { kind: "fill", text: acceptedText + escapedUnit, append: false },
      ...fakeTime(),
    }),
    (error) => error.code === ACTION_ERROR_CODES.INVALID_ARGUMENT,
  );
  assert.equal(
    rejectedConnection.calls.filter((call) => call.method === "Input.insertText").length,
    0,
  );
});

class FakeActionConnection {
  constructor(actionabilityOrOptions) {
    const options = Array.isArray(actionabilityOrOptions)
      ? { actionability: actionabilityOrOptions }
      : actionabilityOrOptions;
    this.actionability = options.actionability;
    this.calls = [];
    this.loaderId = "loader-a";
    this.afterSend = options.afterSend;
  }

  async send(method, params = {}, sessionId) {
    this.calls.push({ method, params, sessionId });
    const finish = (result) => {
      this.afterSend?.(method, params, sessionId);
      return result;
    };
    if (method === "Page.getFrameTree") {
      return finish({ frameTree: { frame: { loaderId: this.loaderId } } });
    }
    if (method === "DOM.resolveNode") {
      if (this.resolveError) throw this.resolveError;
      return finish({ object: { objectId: "object-1" } });
    }
    if (method === "Runtime.callFunctionOn") {
      if (params.functionDeclaration.includes("scrollIntoView")) {
        return finish({
          result: {
            value: {
              ...(this.actionability.length > 1
                ? this.actionability.shift()
                : this.actionability[0]),
            },
          },
        });
      }
      if (params.functionDeclaration.includes("expectedDocumentUrl")) {
        const expectedDocumentUrl = params.arguments[2].value;
        return finish({
          result: {
            value: this.documentUrl !== expectedDocumentUrl
              ? { ok: false, reason: "target_url_changed" }
              : this.secureFillFocusSucceeded === false
                ? { ok: false, reason: "target_not_focusable" }
                : this.secureFillMaskedAfterFocus === false
                  ? { ok: false, reason: "target_not_masked" }
                : { ok: true },
          },
        });
      }
      return finish({ result: { value: { ok: true } } });
    }
    if (method === "Runtime.releaseObject") {
      return finish({});
    }
    if (method.startsWith("Input.")) {
      if (method === "Input.dispatchMouseEvent" && params.type === this.dialogEventType) {
        this.dialogWaiter?.({ method: "Page.javascriptDialogOpening" });
        this.afterSend?.(method, params, sessionId);
        return new Promise(() => {});
      }
      return finish({});
    }
    throw new Error(`unexpected CDP method ${method}`);
  }

  waitForEvent(_method, timeoutMs) {
    this.dialogWaitTimeoutMs = timeoutMs;
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
