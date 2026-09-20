import test from "node:test";
import assert from "node:assert/strict";

import {
  ACTION_ERROR_CODES,
  actionabilityFunction,
  fillFunction,
  fillRequestBytes,
  MAX_FILL_REQUEST_BYTES,
  performBrowserAction,
} from "./browser-controller-actions.mjs";

const ELEMENT = Object.freeze({
  tab_id: "tab-7",
  document_id: "document-3",
  frame_id: "frame-main",
  main_frame_id: "frame-main",
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
  constructor({
    documentId = "document-3",
    actionability = [READY],
    resolveError,
    fillOk = true,
    afterSend,
  } = {}) {
    this.documentId = documentId;
    this.actionability = [...actionability];
    this.resolveError = resolveError;
    this.fillOk = fillOk;
    this.afterSend = afterSend;
    this.calls = [];
  }

  async send(method, params, timeoutMs) {
    this.calls.push({ method, params, timeoutMs });
    let result;
    if (method === "Page.getFrameTree") {
      result = { frameTree: { frame: { id: "frame-main", loaderId: this.documentId } } };
    } else if (method === "DOM.resolveNode") {
      if (this.resolveError) throw this.resolveError;
      result = { object: { objectId: "object-91" } };
    } else if (method === "Runtime.callFunctionOn") {
      if (Array.isArray(params.arguments)) {
        result = { result: { value: { ok: this.fillOk } } };
      } else {
        result = {
          result: {
            value: this.actionability.length > 1
              ? this.actionability.shift()
              : this.actionability[0],
          },
        };
      }
    } else if (method === "Input.dispatchMouseEvent" || method === "Runtime.releaseObject") {
      result = {};
    } else {
      throw new Error(`unexpected method ${method}`);
    }
    this.afterSend?.(method, timeoutMs, params);
    return result;
  }
}

function fakeTime() {
  let value = 0;
  const sleeps = [];
  return {
    now: () => value,
    advance: (milliseconds) => {
      value += milliseconds;
    },
    sleep: async (milliseconds) => {
      sleeps.push(milliseconds);
      value += milliseconds;
    },
    sleeps,
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

test("rechecks the hit target after hover and retries a replaced menu without pressing", async () => {
  const time = fakeTime();
  const connection = new FakeConnection({
    actionability: [READY, READY, { state: "obscured" }, READY, READY],
  });
  const result = await performBrowserAction({
    connection,
    element: ELEMENT,
    action: { kind: "click" },
    ...time,
  });

  assert.equal(result.attempts, 3);
  assert.deepEqual(
    connection.calls
      .filter((call) => call.method === "Input.dispatchMouseEvent")
      .map((call) => call.params.type),
    ["mouseMoved", "mouseMoved", "mousePressed", "mouseReleased"],
  );
  assert.equal(
    connection.calls.filter(
      (call) => call.method === "Input.dispatchMouseEvent" && call.params.type === "mousePressed",
    ).length,
    1,
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

  await assert.rejects(
    performBrowserAction({
      connection: new FakeConnection({ actionability: [{ state: "detached" }] }),
      element: ELEMENT,
      action: { kind: "click" },
    }),
    (error) => error.code === ACTION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED,
  );
});

test("rejects child-frame references before dispatching root-frame input", async () => {
  const connection = new FakeConnection();
  await assert.rejects(
    performBrowserAction({
      connection,
      element: { ...ELEMENT, frame_id: "frame-child" },
      action: { kind: "click" },
    }),
    (error) => error.code === ACTION_ERROR_CODES.FRAME_UNSUPPORTED,
  );
  assert.equal(connection.calls.length, 0);
});

test("enforces one wall-clock deadline across CDP calls and cleanup", async () => {
  const time = fakeTime();
  const connection = new FakeConnection({
    actionability: [READY, READY],
    afterSend: () => time.advance(20),
  });
  await assert.rejects(
    performBrowserAction({
      connection,
      element: ELEMENT,
      action: { kind: "click" },
      timeoutMs: 100,
      now: time.now,
      sleep: time.sleep,
    }),
    (error) => {
      assert.equal(error.code, ACTION_ERROR_CODES.TIMEOUT);
      assert.equal(error.details.timeout_ms, 100);
      return true;
    },
  );
  assert.equal(
    connection.calls.some((call) => call.method === "Input.dispatchMouseEvent"),
    false,
  );
  assert.equal(
    connection.calls.every((call) => Number.isSafeInteger(call.timeoutMs) && call.timeoutMs > 0),
    true,
  );
});

test("caps queued-near-expiry polling at the advertised absolute deadline", async () => {
  const time = fakeTime();
  time.advance(90);
  const advertisedDeadline = 100;
  await assert.rejects(
    performBrowserAction({
      connection: new FakeConnection({ actionability: [{ state: "not_visible" }] }),
      element: ELEMENT,
      action: { kind: "click" },
      timeoutMs: 500,
      deadline: advertisedDeadline,
      ...time,
    }),
    (error) => error.code === ACTION_ERROR_CODES.TIMEOUT,
  );
  assert.deepEqual(time.sleeps, [10]);
  assert.equal(time.now(), advertisedDeadline);
  assert.ok(time.now() <= advertisedDeadline);
});

test("does not retry after a pressed/released pair crosses the deadline", async () => {
  const time = fakeTime();
  const connection = new FakeConnection({
    actionability: [READY, READY],
    afterSend: (method, _timeoutMs, params) => {
      if (method === "Input.dispatchMouseEvent" && params.type === "mouseReleased") {
        time.advance(51);
      }
    },
  });
  await assert.rejects(
    performBrowserAction({
      connection,
      element: ELEMENT,
      action: { kind: "click" },
      timeoutMs: 100,
      ...time,
    }),
    (error) => {
      assert.equal(error.code, ACTION_ERROR_CODES.POST_ACTION_UNCERTAIN);
      assert.deepEqual(error.details, {
        outcome: "post_action_uncertain",
        phase: "mouseReleased",
        attempts: 2,
        timeout_ms: 100,
        cause_code: ACTION_ERROR_CODES.TIMEOUT,
      });
      return true;
    },
  );
  const mouseEvents = connection.calls
    .filter((call) => call.method === "Input.dispatchMouseEvent")
    .map((call) => call.params.type);
  assert.deepEqual(mouseEvents, ["mouseMoved", "mousePressed", "mouseReleased"]);
  assert.equal(mouseEvents.filter((type) => type === "mousePressed").length, 1);
  assert.equal(mouseEvents.filter((type) => type === "mouseReleased").length, 1);
  assert.equal(connection.calls.filter((call) => call.method === "Runtime.releaseObject").length, 1);
});

test("reports an indeterminate fill when its mutation response crosses the deadline", async () => {
  const time = fakeTime();
  const connection = new FakeConnection({
    actionability: [READY, READY],
    afterSend: (method, _timeoutMs, params) => {
      if (method === "Runtime.callFunctionOn" && Array.isArray(params.arguments)) {
        time.advance(51);
      }
    },
  });
  await assert.rejects(
    performBrowserAction({
      connection,
      element: ELEMENT,
      action: { kind: "fill", text: "one-shot", append: false },
      timeoutMs: 100,
      ...time,
    }),
    (error) => {
      assert.equal(error.code, ACTION_ERROR_CODES.POST_ACTION_UNCERTAIN);
      assert.deepEqual(error.details, {
        outcome: "post_action_uncertain",
        phase: "fill",
        attempts: 2,
        timeout_ms: 100,
        cause_code: ACTION_ERROR_CODES.TIMEOUT,
      });
      return true;
    },
  );
  assert.equal(
    connection.calls.filter(
      (call) => call.method === "Runtime.callFunctionOn" && Array.isArray(call.params.arguments),
    ).length,
    1,
  );
});

test("restores the prior value when a native number setter sanitizes fill text", () => {
  const previousDocument = globalThis.document;
  const events = [];
  const document = { activeElement: null };
  class NumberInput {
    constructor() {
      this.isConnected = true;
      this.disabled = false;
      this.readOnly = false;
      this.tagName = "input";
      this.type = "number";
      this._value = "7";
    }

    get value() {
      return this._value;
    }

    set value(value) {
      this._value = /^-?\d+(?:\.\d+)?$/.test(String(value)) ? String(value) : "";
    }

    focus() {
      document.activeElement = this;
    }

    dispatchEvent(event) {
      events.push(event.type);
    }
  }
  const control = new NumberInput();
  globalThis.document = document;
  try {
    const result = fillFunction.call(control, "opaque-not-a-number", false);
    assert.deepEqual(result, { ok: false });
    assert.equal(control.value, "7");
    assert.deepEqual(events, []);
    assert.doesNotMatch(JSON.stringify(result), /opaque-not-a-number/);
  } finally {
    if (previousDocument === undefined) delete globalThis.document;
    else globalThis.document = previousDocument;
  }
});

test("selects the exact visible-text option when duplicate values exist", () => {
  const previousDocument = globalThis.document;
  const previousEvent = globalThis.Event;
  const events = [];
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
    get selectedIndex() {
      return selectedIndex;
    },
    set selectedIndex(index) {
      selectedIndex = index;
      options.forEach((option, optionIndex) => {
        option.selected = optionIndex === index;
      });
      this.value = options[index]?.value ?? "";
    },
    focus() {
      document.activeElement = this;
    },
    dispatchEvent(event) {
      events.push(event.type);
    },
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
    assert.deepEqual(events, ["input", "change"]);
  } finally {
    if (previousDocument === undefined) delete globalThis.document;
    else globalThis.document = previousDocument;
    if (previousEvent === undefined) delete globalThis.Event;
    else globalThis.Event = previousEvent;
  }
});

test("follows open shadow roots for hit testing and focus", () => {
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
  globalThis.Event = class Event {
    constructor(type, init) {
      this.type = type;
      this.init = init;
    }
  };
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

test("does not treat an ordinary ancestor hit as actionable for a pointer-events-none target", () => {
  const previousDocument = globalThis.document;
  const previousWindow = globalThis.window;
  const ancestor = { parentElement: null, style: { pointerEvents: "none" } };
  const document = {
    activeElement: null,
    elementFromPoint: () => ancestor,
  };
  const control = {
    isConnected: true,
    disabled: false,
    readOnly: false,
    tagName: "button",
    parentElement: ancestor,
    getRootNode: () => document,
    scrollIntoView() {},
    getBoundingClientRect: () => ({ left: 10, top: 20, width: 100, height: 20 }),
    matches: () => false,
    closest: () => null,
    getAttribute: () => null,
    contains: () => false,
  };
  globalThis.document = document;
  globalThis.window = {
    getComputedStyle: () => ({ display: "block", visibility: "visible", opacity: "1" }),
  };
  try {
    assert.deepEqual(actionabilityFunction.call(control), { state: "obscured" });
  } finally {
    if (previousDocument === undefined) delete globalThis.document;
    else globalThis.document = previousDocument;
    if (previousWindow === undefined) delete globalThis.window;
    else globalThis.window = previousWindow;
  }
});

test("bounds the JSON-encoded fill frame near the 64 KiB CDP limit", async () => {
  const objectId = "object-91";
  const escapedUnit = "\\\"";
  let acceptedText = "";
  while (fillRequestBytes(objectId, acceptedText + escapedUnit, false) <= MAX_FILL_REQUEST_BYTES) {
    acceptedText += escapedUnit;
  }
  const acceptedBytes = fillRequestBytes(objectId, acceptedText, false);
  const rejectedBytes = fillRequestBytes(objectId, acceptedText + escapedUnit, false);
  assert.ok(acceptedText.length > 0);
  assert.ok(acceptedBytes <= MAX_FILL_REQUEST_BYTES);
  assert.ok(rejectedBytes > MAX_FILL_REQUEST_BYTES);

  const acceptedConnection = new FakeConnection({ actionability: [READY, READY] });
  await performBrowserAction({
    connection: acceptedConnection,
    element: ELEMENT,
    action: { kind: "fill", text: acceptedText, append: false },
    ...fakeTime(),
  });
  const acceptedFillCall = acceptedConnection.calls.find(
    (call) => call.method === "Runtime.callFunctionOn" && Array.isArray(call.params.arguments),
  );
  assert.equal(acceptedFillCall.params.arguments[0].value, acceptedText);

  const rejectedConnection = new FakeConnection({ actionability: [READY, READY] });
  await assert.rejects(
    performBrowserAction({
      connection: rejectedConnection,
      element: ELEMENT,
      action: { kind: "fill", text: acceptedText + escapedUnit, append: false },
      ...fakeTime(),
    }),
    (error) => {
      assert.equal(error.code, ACTION_ERROR_CODES.INVALID_ARGUMENT);
      assert.equal(error.details.reason, "fill_request_too_large");
      return true;
    },
  );
  assert.equal(
    rejectedConnection.calls.filter(
      (call) => call.method === "Runtime.callFunctionOn" && Array.isArray(call.params.arguments),
    ).length,
    0,
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
