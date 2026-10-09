import assert from "node:assert/strict";
import test from "node:test";

import {
  BrowserActionError,
  actionabilityFunction,
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

  assert.equal(connection.calls.some((call) => call.method === "Input.insertText"), false);
});

for (const transitionEvent of ["input", "change"]) {
test(`secret fill rejects and remasks a password field unmasked by ${transitionEvent} handlers`, async () => {
  const connection = new FakeActionConnection([
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
    { state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true },
  ]);
  const attributes = new Map([["type", "password"]]);
  class Element {
    getAttribute(name) { return attributes.get(name) ?? null; }
    hasAttribute(name) { return attributes.has(name); }
  }
  class HTMLInputElement extends Element {
    get type() { return attributes.get("type") ?? "text"; }
    set type(value) { attributes.set("type", value); }
    get value() { return this.valueContents ?? ""; }
    set value(value) { this.valueContents = value; }
    matches(selector) { return selector === "textarea" ? false : selector === "input"; }
    focus() { this.ownerDocument.activeElement = this; }
    getRootNode() { return this.ownerDocument; }
    contains() { return false; }
    dispatchEvent(event) {
      if (event.type === transitionEvent) this.type = "text";
      return true;
    }
  }
  const ownerWindow = {
    Element,
    HTMLInputElement,
    Event: class { constructor(type) { this.type = type; } },
    location: { href: "https://example.test/login" },
  };
  const field = new HTMLInputElement();
  field.ownerDocument = { defaultView: ownerWindow, activeElement: null };
  const send = connection.send.bind(connection);
  connection.send = async (method, params = {}, sessionId) => {
    if (method === "Runtime.callFunctionOn" && params.functionDeclaration.includes("expectedDocumentUrl")) {
      connection.calls.push({ method, params, sessionId });
      const operation = new Function(`return (${params.functionDeclaration})`)();
      return { result: { value: operation.apply(field, params.arguments.map((arg) => arg.value)) } };
    }
    return send(method, params, sessionId);
  };

  await assert.rejects(performBrowserAction({
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    nodeRef: "backend:104",
    action: {
      kind: "fill",
      text: "event-handler-canary",
      append: false,
      submit: false,
      expected_document_url: "https://example.test/login",
    },
    timeoutMs: 500,
    sleep: async () => {},
  }), (error) => {
    assert.equal(`${error.name}:${error.code}:${error.message}`.includes("event-handler-canary"), false);
    return error instanceof BrowserActionError && error.code === "browser_secret_target_not_masked";
  });

  assert.equal(field.type, "password", "the field must be remasked before the CDP operation returns");
  assert.equal(connection.calls.some((call) => call.method === "Input.insertText"), false);
});
}

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
    if (method === "DOMSnapshot.captureSnapshot") {
      return { strings: ["frame-a"], documents: [{ frameId: 0, nodes: { backendNodeId: [103, 104] } }] };
    }
    if (method === "Page.createIsolatedWorld") {
      assert.equal(params.frameId, "frame-a");
      assert.equal(params.grantUniveralAccess, false);
      return { executionContextId: 7 };
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
      if (params.functionDeclaration.includes("expectedDocumentUrl")) {
        const expectedDocumentUrl = params.arguments[2].value;
        return {
          result: {
            value: this.documentUrl !== expectedDocumentUrl
              ? { ok: false, reason: "target_url_changed" }
              : this.secureFillFocusSucceeded === false
                ? { ok: false, reason: "target_not_focusable" }
                : this.secureFillMaskedAfterFocus === false
                  ? { ok: false, reason: "target_not_masked" }
                : { ok: true },
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

// MP-08 / MP-10 / MP-11 A07: execute the actual isolated fill function against
// native-shaped DOM objects. Synthetic values remain regression-only.
function protectedCodeFixture(type, hook = () => {}) {
  class Style {
    constructor() { this.values = new Map(); }
    setProperty(key, value) { this.values.set(key, value); }
    getPropertyValue(key) { return this.values.get(key) ?? ""; }
  }
  class Element {
    constructor() { this.attributes = new Map(); this.style = new Style(); this.events = []; }
    getAttribute(key) { return this.attributes.get(key) ?? null; }
    hasAttribute(key) { return this.attributes.has(key); }
    focus() { this.ownerDocument.activeElement = this; hook("focus", this); }
    getRootNode() { return this.ownerDocument; }
    contains() { return false; }
    matches(name) { return name === "textarea" && this instanceof TextArea; }
    dispatchEvent(event) { this.events.push(event.type); hook(event.type, this); return true; }
  }
  class Input extends Element {
    get type() { return this.getAttribute("type") ?? "text"; }
    set type(value) { this.attributes.set("type", value); }
    get value() { return this.storedValue ?? ""; }
    set value(value) { this.storedValue = value; }
  }
  class TextArea extends Element {
    get value() { return this.storedValue ?? ""; }
    set value(value) { this.storedValue = value; }
  }
  const field = type === "textarea" ? new TextArea() : new Input();
  if (type !== "textarea") field.type = type;
  field.ownerDocument = { activeElement: null, defaultView: {
    location: { href: "https://example.test/verification" },
    Element, HTMLInputElement: Input, HTMLTextAreaElement: TextArea,
    CSSStyleDeclaration: Style,
    getComputedStyle: element => element.style,
    Event: class { constructor(eventType) { this.type = eventType; } },
  } };
  class Connection extends FakeActionConnection {
    constructor() {
      super([{ state: "ready", x: 40, y: 20, width: 200, height: 24, editable: true }]);
    }
    async send(method, params = {}, sessionId) {
      if (method === "Runtime.callFunctionOn" && params.functionDeclaration.includes("expectedDocumentUrl")) {
        this.calls.push({ method, params, sessionId });
        const fill = Function(`return (${params.functionDeclaration})`)();
        return { result: { value: fill.apply(field, params.arguments.map(arg => arg.value)) } };
      }
      return super.send(method, params, sessionId);
    }
  }
  const connection = new Connection();
  const fill = (maskCode = true, expectedUrl = "https://example.test/verification") => performBrowserAction({
    connection, sessionId: "session-a", targetId: "target-a", documentId: "loader-a", nodeRef: "backend:104",
    action: { kind: "fill", text: "am7-code-canary", expected_document_url: expectedUrl, mask_code_input: maskCode },
    timeoutMs: 500, sleep: async () => {},
  });
  return { field, connection, fill };
}

for (const type of ["text", "tel", "number", "search", "password", "textarea"]) {
  test(`MP-08/MP-10/MP-11 protected owner code masks ${type} before input handlers`, async () => {
    const observed = [];
    const f = protectedCodeFixture(type, (event, field) => {
      observed.push({ event, masked: type === "textarea"
        ? field.style.getPropertyValue("-webkit-text-security") === "disc" : field.type === "password" });
    });
    const result = await f.fill();
    assert.equal(result.action_kind, "fill");
    assert.equal(f.field.value, "am7-code-canary");
    assert(observed.every(event => event.masked));
    assert.deepEqual(f.field.events, ["input", "change"]);
    assert.equal(f.connection.calls.some(call => call.method.startsWith("Input.")), false);
    assert.equal(JSON.stringify(result).includes("am7-code-canary"), false);
  });
}

test("MP-08/MP-10/MP-11 code masking does not loosen the ordinary Vault password-only fill", async () => {
  const f = protectedCodeFixture("text");
  await assert.rejects(f.fill(false), { code: "browser_secret_target_not_masked" });
  assert.equal(f.field.value, "");
  assert.equal(f.field.type, "text");
});

for (const condition of ["readonly", "disabled", "url-change", "checkbox"]) {
  test(`MP-08/MP-10/MP-11 owner code refuses ${condition} before mutation`, async () => {
    const f = protectedCodeFixture(condition === "checkbox" ? "checkbox" : "text");
    if (["readonly", "disabled"].includes(condition)) f.field.attributes.set(condition, "");
    await assert.rejects(f.fill(true, condition === "url-change" ? "https://example.test/other" : undefined));
    assert.equal(f.field.value, "");
    assert.deepEqual(f.field.events, []);
    assert.equal(f.field.type, condition === "checkbox" ? "checkbox" : "text");
  });
}

for (const event of ["focus", "input", "change"]) {
  test(`MP-08/MP-10/MP-11 owner code fails closed when ${event} handlers unmask its field`, async () => {
    const f = protectedCodeFixture("text", (kind, field) => { if (kind === event) field.type = "text"; });
    await assert.rejects(f.fill(), { code: "browser_secret_target_not_masked" });
    if (event === "focus") assert.equal(f.field.value, "");
    else {
      assert.equal(f.field.type, "password");
      assert.deepEqual(f.field.events, event === "input" ? ["input"] : ["input", "change"]);
    }
  });
}

for (const event of ["focus", "input", "change"]) {
  test(`MP-08/MP-10/MP-11 ordinary Vault fill still refuses ${event} unmasking`, async () => {
    const f = protectedCodeFixture("password", (kind, field) => { if (kind === event) field.type = "text"; });
    await assert.rejects(f.fill(false), { code: "browser_secret_target_not_masked" });
    if (event === "focus") assert.equal(f.field.value, "");
    else assert.equal(f.field.type, "password");
  });
}

test("MP-08/MP-10/MP-11 owner code without a document URL cannot fall back to native input", async () => {
  const f = protectedCodeFixture("text");
  await assert.rejects(performBrowserAction({
    connection: f.connection, sessionId: "session-a", targetId: "target-a", documentId: "loader-a", nodeRef: "backend:104",
    action: { kind: "fill", text: "fixture-only-code", mask_code_input: true },
    timeoutMs: 500, sleep: async () => {},
  }));
  assert.equal(f.connection.calls.some(call => call.method.startsWith("Input.")), false);
});
