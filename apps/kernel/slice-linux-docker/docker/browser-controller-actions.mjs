const DEFAULT_ACTION_TIMEOUT_MS = 5_000;
const MAX_ACTION_TIMEOUT_MS = 5_000;
const MIN_ACTION_TIMEOUT_MS = 100;
const ACTION_POLL_INTERVAL_MS = 50;
const DIALOG_OPEN_WAIT_MS = 5_000;

// Keep this in sync with browser-controller-cdp.mjs.  The action module cannot
// import the connection module without creating a cycle, and the guard is
// deliberately conservative: an encoded request must be strictly smaller
// than the receive-side frame cap.
const MAX_CDP_FRAME_BYTES = 4 * 1024 * 1024;
// Keep action requests comfortably below the receive-side cap. This envelope
// budget also leaves room for JSON escaping and the CDP request metadata.
const MAX_FILL_REQUEST_BYTES = Math.min(64 * 1024, MAX_CDP_FRAME_BYTES - 1);
const MAX_FILL_TEXT_BYTES = 48 * 1024;

export const ACTION_ERROR_CODES = Object.freeze({
  INVALID_ARGUMENT: "browser_action_invalid",
  TIMEOUT: "browser_action_timeout",
  FAILED: "browser_action_failed",
  POST_ACTION_UNCERTAIN: "browser_action_post_action_uncertain",
  STALE_DOCUMENT: "stale_document_reference",
  ELEMENT_REFERENCE_INVALIDATED: "stale_element_reference",
  FRAME_UNSUPPORTED: "browser_action_frame_unsupported",
});

export { MAX_FILL_REQUEST_BYTES };

export class BrowserActionError extends Error {
  constructor(code, message, details = {}) {
    super(message ?? code);
    this.name = "BrowserActionError";
    this.code = code;
    this.details = details;
    Object.assign(this, details);
  }
}

function fail(code, message, details = {}) {
  throw new BrowserActionError(code, message, details);
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function readNow(now) {
  const value = Number(now());
  if (!Number.isFinite(value)) {
    fail(ACTION_ERROR_CODES.FAILED, "browser action clock returned a non-finite value");
  }
  return value;
}

function timeoutDetails(budget) {
  return {
    attempts: budget.attempts,
    timeout_ms: budget.timeoutMs,
    reason: budget.lastReason,
  };
}

function timeoutError(budget, message = "browser action deadline expired") {
  const error = new BrowserActionError(
    ACTION_ERROR_CODES.TIMEOUT,
    message,
    timeoutDetails(budget),
  );
  // Keep the mature aggregate's camel-case diagnostic available to direct
  // callers while the details object remains a stable protocol-shaped value.
  error.timeoutMs = budget.timeoutMs;
  return error;
}

function postActionDetails(budget, phase, error) {
  return {
    outcome: "post_action_uncertain",
    phase,
    attempts: budget.attempts,
    timeout_ms: budget.timeoutMs,
    cause_code: typeof error?.code === "string" ? error.code : "unknown",
  };
}

function postActionError(budget, phase, error) {
  const uncertain = new BrowserActionError(
    ACTION_ERROR_CODES.POST_ACTION_UNCERTAIN,
    "browser action outcome is uncertain; do not retry automatically",
    postActionDetails(budget, phase, error),
  );
  uncertain.timeoutMs = budget.timeoutMs;
  return uncertain;
}

function remainingBudget(budget) {
  const remaining = Math.ceil(budget.deadline - readNow(budget.now));
  if (remaining <= 0) {
    throw timeoutError(budget);
  }
  return remaining;
}

async function sendWithinBudget(connection, sessionId, method, params, budget) {
  const remaining = remainingBudget(budget);
  const command = Promise.resolve().then(() => connection.send(method, params, sessionId));
  let timer;
  let result;
  try {
    // The mature CdpConnection's third argument is the session ID.  Do not
    // pass a per-call timeout here: doing so would silently route a deadline
    // number as a CDP session and break child-frame actions.
    result = await Promise.race([
      command,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(timeoutError(budget)), remaining);
      }),
    ]);
  } catch (error) {
    clearTimeout(timer);
    if (error instanceof BrowserActionError) {
      void command.catch(() => {});
      throw error;
    }
    if (readNow(budget.now) >= budget.deadline) {
      void command.catch(() => {});
      throw timeoutError(budget);
    }
    throw error;
  } finally {
    clearTimeout(timer);
  }
  remainingBudget(budget);
  return result;
}

function requirePositiveInteger(value) {
  if (!Number.isSafeInteger(value) || value < 1) {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "browser action requires a positive integer");
  }
  return value;
}

function requireIdentifier(value) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 512 ||
    /[\u0000-\u001f\u007f]/.test(value)
  ) {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "browser action requires a valid identifier");
  }
  return value;
}

function utf8ByteLength(value) {
  return Buffer.byteLength(value, "utf8");
}

function normalizeTimeout(timeoutMs) {
  if (timeoutMs === undefined) {
    return DEFAULT_ACTION_TIMEOUT_MS;
  }
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "browser action timeout must be a positive integer");
  }
  return Math.max(MIN_ACTION_TIMEOUT_MS, Math.min(timeoutMs, MAX_ACTION_TIMEOUT_MS));
}

function normalizeExpectedDocumentUrl(value) {
  if (value === undefined || value === null) return null;
  if (typeof value !== "string" || value.length === 0 || utf8ByteLength(value) > 2_048) {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "secret fill expected document URL is invalid");
  }
  try {
    return new URL(value).href;
  } catch {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "secret fill expected document URL is invalid");
  }
}

function normalizeAction(action) {
  if (!isPlainObject(action) || typeof action.kind !== "string") {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "browser action kind is required");
  }
  if (action.kind === "click") {
    if (Object.keys(action).some((key) => key !== "kind")) {
      fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "click action has unknown fields");
    }
    return { kind: "click" };
  }
  if (action.kind === "fill") {
    if (
      Object.keys(action).some((key) => ![
        "kind",
        "text",
        "append",
        "submit",
        "expected_document_url",
      ].includes(key)) ||
      typeof action.text !== "string" ||
      (action.append !== undefined && typeof action.append !== "boolean") ||
      (action.submit !== undefined && typeof action.submit !== "boolean") ||
      utf8ByteLength(action.text) > MAX_FILL_TEXT_BYTES
    ) {
      fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "fill action is invalid");
    }
    return {
      kind: "fill",
      text: action.text,
      append: action.append === true,
      submit: action.submit === true,
      expectedDocumentUrl: normalizeExpectedDocumentUrl(action.expected_document_url),
    };
  }
  if (action.kind === "submit") {
    if (Object.keys(action).some((key) => key !== "kind")) {
      fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "submit action has unknown fields");
    }
    return { kind: "submit" };
  }
  fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, `unsupported browser action ${JSON.stringify(action.kind)}`);
}

function parseBackendNodeReference(nodeRef) {
  if (typeof nodeRef !== "string" || !/^backend:[1-9][0-9]*$/.test(nodeRef)) {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "browser action requires a valid controller node reference");
  }
  const backendNodeId = Number(nodeRef.slice("backend:".length));
  if (!Number.isSafeInteger(backendNodeId)) {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "browser action node reference exceeds the safe integer range");
  }
  return backendNodeId;
}

export function assertNotCancelled(signal) {
  if (signal?.aborted) {
    throw new BrowserActionError("browser_action_cancelled", "browser action was cancelled");
  }
}

async function assertCurrentDocument(connection, sessionId, documentId, budget) {
  const frameTree = await sendWithinBudget(connection, sessionId, "Page.getFrameTree", {}, budget);
  if (frameTree?.frameTree?.frame?.loaderId !== documentId) {
    fail(ACTION_ERROR_CODES.STALE_DOCUMENT, "browser target moved away from the requested document");
  }
}

async function resolveBackendNode(connection, sessionId, backendNodeId, budget) {
  let resolved;
  try {
    resolved = await sendWithinBudget(
      connection,
      sessionId,
      "DOM.resolveNode",
      { backendNodeId },
      budget,
    );
  } catch (error) {
    if (error?.code === "browser_cdp_command_failed" || error?.code === "CDP_COMMAND_FAILED") {
      fail(
        ACTION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED,
        "browser element is no longer attached to the current document",
      );
    }
    throw error;
  }
  const objectId = resolved?.object?.objectId;
  if (typeof objectId !== "string" || !objectId) {
    fail(
      ACTION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED,
      "browser element is no longer attached to the current document",
    );
  }
  return objectId;
}

async function inspectActionability(connection, sessionId, objectId, budget) {
  const response = await sendWithinBudget(
    connection,
    sessionId,
    "Runtime.callFunctionOn",
    {
      objectId,
      functionDeclaration: actionabilityFunction.toString(),
      returnByValue: true,
      awaitPromise: false,
    },
    budget,
  );
  if (response?.exceptionDetails) {
    fail(ACTION_ERROR_CODES.FAILED, "browser actionability inspection failed");
  }
  const result = response?.result?.value;
  if (!isPlainObject(result) || typeof result.state !== "string") {
    fail(ACTION_ERROR_CODES.FAILED, "browser actionability inspection returned an invalid result");
  }
  if (result.state === "ready" && result.crossOriginFrame) {
    const { quads } = await sendWithinBudget(
      connection,
      sessionId,
      "DOM.getContentQuads",
      { objectId },
      budget,
    );
    const quad = quads?.[0];
    if (!Array.isArray(quad) || quad.length !== 8 || !quad.every(Number.isFinite)) {
      return { state: "frame_unavailable" };
    }
    result.x = (quad[0] + quad[2] + quad[4] + quad[6]) / 4;
    result.y = (quad[1] + quad[3] + quad[5] + quad[7]) / 4;
  }
  return result;
}

export function actionabilityFunction() {
  function composedContains(ancestor, candidate) {
    for (let current = candidate; current; ) {
      if (current === ancestor) return true;
      if (current.parentElement) {
        current = current.parentElement;
        continue;
      }
      const root = current.getRootNode?.();
      current = root?.host || null;
    }
    return false;
  }

  function isShadowHostOnTargetRootChain(target, candidate) {
    const visitedRoots = new Set();
    for (let root = target.getRootNode?.(); root?.host; root = root.host.getRootNode?.()) {
      if (visitedRoots.has(root)) return false;
      visitedRoots.add(root);
      if (root.host === candidate) return true;
    }
    return false;
  }

  function composedElementFromPoint(root, x, y) {
    let hit = root?.elementFromPoint?.(x, y) || null;
    while (hit?.shadowRoot) {
      const nested = hit.shadowRoot.elementFromPoint?.(x, y) || null;
      if (!nested || nested === hit) break;
      hit = nested;
    }
    return hit;
  }

  if (!this.isConnected) return { state: "detached" };
  this.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
  const ownerDocument = this.ownerDocument
    || (typeof document !== "undefined" ? document : null);
  const ownerWindow = ownerDocument?.defaultView
    || (typeof window !== "undefined" ? window : null);
  const style = ownerWindow?.getComputedStyle?.(this) || {};
  const rect = this.getBoundingClientRect();
  if (
    style.display === "none" ||
    style.visibility === "hidden" ||
    Number(style.opacity) <= 0 ||
    rect.width <= 0 ||
    rect.height <= 0
  ) {
    return { state: "not_visible" };
  }
  if (
    this.disabled ||
    this.matches?.(":disabled") ||
    this.closest?.("[inert]") ||
    this.getAttribute?.("aria-disabled") === "true"
  ) {
    return { state: "disabled" };
  }
  const localX = rect.left + rect.width / 2;
  const localY = rect.top + rect.height / 2;
  const hit = composedElementFromPoint(ownerDocument, localX, localY);
  if (
    !hit ||
    (
      hit !== this &&
      !this.contains?.(hit) &&
      !composedContains(this, hit) &&
      !isShadowHostOnTargetRootChain(this, hit)
    )
  ) {
    return { state: "obscured" };
  }

  let x = localX;
  let y = localY;
  let currentWindow = ownerWindow;
  let crossOriginFrame = false;
  while (currentWindow && currentWindow !== currentWindow.top) {
    let frameElement;
    try {
      frameElement = currentWindow.frameElement;
    } catch {
      frameElement = null;
    }
    if (!frameElement) {
      crossOriginFrame = true;
      break;
    }
    const frameRect = frameElement.getBoundingClientRect();
    const frameStyle = frameElement.ownerDocument?.defaultView?.getComputedStyle?.(frameElement) || {};
    const paddingLeft = Number.parseFloat(frameStyle.paddingLeft) || 0;
    const paddingTop = Number.parseFloat(frameStyle.paddingTop) || 0;
    x += frameRect.left + frameElement.clientLeft + paddingLeft;
    y += frameRect.top + frameElement.clientTop + paddingTop;
    currentWindow = frameElement.ownerDocument?.defaultView || null;
  }

  const tag = String(this.tagName || "").toLowerCase();
  const inputType = tag === "input" ? String(this.type || "text").toLowerCase() : null;
  const editableInput = inputType !== null && ![
    "button",
    "checkbox",
    "color",
    "file",
    "hidden",
    "image",
    "radio",
    "range",
    "reset",
    "submit",
  ].includes(inputType);
  const editable =
    tag === "select" ||
    this.isContentEditable ||
    ((editableInput || tag === "textarea") && !this.readOnly);
  return {
    state: "ready",
    x,
    y,
    width: rect.width,
    height: rect.height,
    editable,
    ...(tag === "select" ? { isSelect: true } : {}),
    ...(crossOriginFrame ? { crossOriginFrame: true } : {}),
  };
}

function readyGeometry(actionability, action) {
  if (actionability.state !== "ready") return null;
  if (action.kind === "fill" && actionability.editable !== true) {
    actionability.state = "not_editable";
    return null;
  }
  const geometry = {
    x: actionability.x,
    y: actionability.y,
    width: actionability.width,
    height: actionability.height,
  };
  return Object.values(geometry).every(Number.isFinite) ? geometry : null;
}

function sameGeometry(left, right) {
  return left !== null &&
    right !== null &&
    left.x === right.x &&
    left.y === right.y &&
    left.width === right.width &&
    left.height === right.height;
}

async function dispatchClick(connection, sessionId, objectId, geometry, budget, signal, documentId) {
  const movedDialog = await dispatchMouseEvent(
    connection,
    sessionId,
    { type: "mouseMoved", x: geometry.x, y: geometry.y },
    budget,
  );
  if (movedDialog) return true;

  assertNotCancelled(signal);
  await budget.assertContext();
  remainingBudget(budget);
  await assertCurrentDocument(connection, sessionId, documentId, budget);
  const revalidated = await inspectActionability(connection, sessionId, objectId, budget);
  const revalidatedGeometry = readyGeometry(revalidated, { kind: "click" });
  budget.lastReason = revalidated.state;
  if (!revalidatedGeometry || !sameGeometry(geometry, revalidatedGeometry)) {
    return false;
  }

  let pressed = false;
  try {
    const pressedDialog = await dispatchMouseEvent(
      connection,
      sessionId,
      {
        type: "mousePressed",
        x: geometry.x,
        y: geometry.y,
        button: "left",
        clickCount: 1,
      },
      budget,
    );
    pressed = true;
    if (pressedDialog) return true;
    const releasedDialog = await dispatchMouseEvent(
      connection,
      sessionId,
      {
        type: "mouseReleased",
        x: geometry.x,
        y: geometry.y,
        button: "left",
        clickCount: 1,
      },
      budget,
    );
    // Once pressed, finish the release even if cancellation arrives. Reporting
    // cancellation before this point would hand over a stuck mouse button.
    assertNotCancelled(signal);
    return releasedDialog;
  } catch (error) {
    if (error?.code === "browser_action_cancelled") {
      throw error;
    }
    throw postActionError(budget, pressed ? "mouseReleased" : "mousePressed", error);
  }
}

async function dispatchMouseEvent(connection, sessionId, params, budget) {
  const remaining = remainingBudget(budget);
  const dialogWaiter = typeof connection.waitForEvent === "function"
    ? connection.waitForEvent("Page.javascriptDialogOpening", DIALOG_OPEN_WAIT_MS, sessionId)
    : null;
  const command = Promise.resolve().then(() => connection.send("Input.dispatchMouseEvent", params, sessionId));
  let timer;
  const deadline = new Promise((_, reject) => {
    timer = setTimeout(() => reject(timeoutError(budget)), remaining);
  });
  if (!dialogWaiter) {
    try {
      await Promise.race([command, deadline]);
      remainingBudget(budget);
      return false;
    } finally {
      clearTimeout(timer);
    }
  }
  try {
    const outcome = await Promise.race([
      command.then(() => ({ kind: "completed" })),
      dialogWaiter.promise.then((event) => ({
        kind: event ? "dialog" : "event_timeout",
      })),
      deadline,
    ]);
    if (outcome.kind === "completed") {
      dialogWaiter.cancel();
      remainingBudget(budget);
      return false;
    }
    if (outcome.kind === "dialog") {
      void command.catch(() => {});
      return true;
    }
    await command;
    remainingBudget(budget);
    return false;
  } finally {
    clearTimeout(timer);
  }
}

function fillRequestParams(objectId, text, append) {
  return {
    objectId,
    functionDeclaration: fillFunction.toString(),
    arguments: [{ value: text }, { value: append }],
    returnByValue: true,
    awaitPromise: false,
  };
}

export function fillRequestBytes(objectId, text, append, sessionId = "s".repeat(512)) {
  return utf8ByteLength(JSON.stringify({
    // Bound using a maximal safe request ID and the actual optional session
    // envelope field used by CdpConnection.
    id: Number.MAX_SAFE_INTEGER,
    method: "Runtime.callFunctionOn",
    params: fillRequestParams(objectId, text, append),
    ...(sessionId ? { sessionId } : {}),
  }));
}

async function fillSelect(connection, sessionId, objectId, action, budget) {
  const encodedBytes = fillRequestBytes(objectId, action.text, action.append);
  if (encodedBytes >= MAX_FILL_REQUEST_BYTES) {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "fill request exceeds the CDP frame budget", {
      reason: "fill_request_too_large",
      encoded_bytes: encodedBytes,
      max_bytes: MAX_FILL_REQUEST_BYTES,
    });
  }
  let response;
  try {
    response = await sendWithinBudget(
      connection,
      sessionId,
      "Runtime.callFunctionOn",
      fillRequestParams(objectId, action.text, action.append),
      budget,
    );
  } catch (error) {
    throw postActionError(budget, "fill", error);
  }
  if (response?.exceptionDetails) {
    fail(ACTION_ERROR_CODES.FAILED, "browser select fill failed");
  }
  if (response?.result?.value?.ok !== true) {
    fail(ACTION_ERROR_CODES.FAILED, "browser select fill could not be applied");
  }
}

export function fillFunction(text, append) {
  function composedContains(ancestor, candidate) {
    for (let current = candidate; current; ) {
      if (current === ancestor) return true;
      if (current.parentElement) {
        current = current.parentElement;
        continue;
      }
      const root = current.getRootNode?.();
      current = root?.host || null;
    }
    return false;
  }

  function composedActiveElement(root) {
    let active = root?.activeElement || null;
    while (active?.shadowRoot?.activeElement) {
      const nested = active.shadowRoot.activeElement;
      if (!nested || nested === active) break;
      active = nested;
    }
    return active;
  }

  const ownerDocument = typeof document !== "undefined" ? document : this.ownerDocument;
  if (!this.isConnected || this.disabled || this.readOnly) return { ok: false };
  this.focus();
  const active = composedActiveElement(ownerDocument);
  if (active !== this && !this.contains?.(active) && !composedContains(this, active)) {
    return { ok: false };
  }

  const tag = String(this.tagName || "").toLowerCase();
  if (tag === "select") {
    const options = Array.from(this.options || []);
    const optionIndex = options.findIndex((candidate) =>
      candidate.value === text || candidate.text === text || candidate.textContent === text,
    );
    const option = optionIndex < 0 ? null : options[optionIndex];
    if (!option) return { ok: false };
    // Assign the index, rather than only the value, so duplicate values cannot
    // silently select the first matching option.
    try {
      this.selectedIndex = optionIndex;
    } catch {
      return { ok: false };
    }
    const selectedOptions = Array.from(this.options || []);
    if (
      this.selectedIndex !== optionIndex ||
      selectedOptions[optionIndex] !== option ||
      option.selected !== true
    ) {
      return { ok: false };
    }
  } else if ("value" in this) {
    const previous = String(this.value ?? "");
    const next = append ? previous + text : text;
    const prototype = Object.getPrototypeOf(this);
    const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
    const setValue = (value) => {
      try {
        if (typeof setter === "function") setter.call(this, value);
        else this.value = value;
        return true;
      } catch {
        return false;
      }
    };
    if (!setValue(next) || String(this.value) !== next) {
      setValue(previous);
      return { ok: false };
    }
  } else if (this.isContentEditable) {
    const previous = String(this.textContent || "");
    this.textContent = append ? previous + text : text;
    if (String(this.textContent || "") !== (append ? previous + text : text)) {
      return { ok: false };
    }
  } else {
    return { ok: false };
  }

  const ownerWindow = ownerDocument?.defaultView
    || (typeof window !== "undefined" ? window : globalThis);
  const EventConstructor = ownerWindow?.Event || globalThis.Event;
  if (typeof EventConstructor !== "function") return { ok: false };
  this.dispatchEvent(new EventConstructor("input", { bubbles: true, composed: true }));
  this.dispatchEvent(new EventConstructor("change", { bubbles: true }));
  return { ok: true };
}

async function focusElement(connection, sessionId, objectId, selectAll, budget) {
  let response;
  try {
    response = await sendWithinBudget(
      connection,
      sessionId,
      "Runtime.callFunctionOn",
      {
        objectId,
        functionDeclaration: `function(selectAll) {
          function composedContains(ancestor, candidate) {
            for (let current = candidate; current; ) {
              if (current === ancestor) return true;
              if (current.parentElement) {
                current = current.parentElement;
                continue;
              }
              const root = current.getRootNode?.();
              current = root?.host || null;
            }
            return false;
          }
          function composedActiveElement(root) {
            let active = root?.activeElement || null;
            while (active?.shadowRoot?.activeElement) {
              const nested = active.shadowRoot.activeElement;
              if (!nested || nested === active) break;
              active = nested;
            }
            return active;
          }
          this.focus();
          const activeElement = composedActiveElement(this.ownerDocument);
          const focused = activeElement === this || this.contains?.(activeElement) || composedContains(this, activeElement);
          if (focused && selectAll) {
            if (typeof this.select === "function") {
              this.select();
            } else {
              const ownerDocument = this.ownerDocument;
              const selection = ownerDocument?.defaultView?.getSelection?.() || window.getSelection();
              const range = ownerDocument.createRange();
              range.selectNodeContents(this);
              selection.removeAllRanges();
              selection.addRange(range);
            }
          }
          return { ok: Boolean(focused) };
        }`,
        arguments: [{ value: selectAll }],
        returnByValue: true,
      },
      budget,
    );
  } catch (error) {
    throw error;
  }
  if (response?.exceptionDetails || response?.result?.value?.ok !== true) {
    fail(ACTION_ERROR_CODES.FAILED, "browser fill target could not receive focus");
  }
}

async function secureFillElement(connection, sessionId, objectId, action, budget) {
  let response;
  try {
    response = await sendWithinBudget(
      connection,
      sessionId,
      "Runtime.callFunctionOn",
      {
        objectId,
        functionDeclaration: `function(text, append, expectedDocumentUrl, submit) {
          function composedContains(ancestor, candidate) {
            for (let current = candidate; current; ) {
              if (current === ancestor) return true;
              if (current.parentElement) {
                current = current.parentElement;
                continue;
              }
              const root = current.getRootNode?.();
              current = root?.host || null;
            }
            return false;
          }
          function composedActiveElement(root) {
            let active = root?.activeElement || null;
            while (active?.shadowRoot?.activeElement) {
              const nested = active.shadowRoot.activeElement;
              if (!nested || nested === active) break;
              active = nested;
            }
            return active;
          }
          const ownerDocument = this.ownerDocument;
          const ownerWindow = ownerDocument?.defaultView;
          if (!ownerWindow || ownerWindow.location.href !== expectedDocumentUrl) {
            return { ok: false, reason: "target_url_changed" };
          }
          const isMaskedEditableInput = () => {
            const inputPrototype = ownerWindow.HTMLInputElement?.prototype;
            const elementPrototype = ownerWindow.Element?.prototype;
            const getAttribute = elementPrototype?.getAttribute;
            const hasAttribute = elementPrototype?.hasAttribute;
            if (
              !inputPrototype ||
              !Object.prototype.isPrototypeOf.call(inputPrototype, this) ||
              typeof getAttribute !== "function" ||
              typeof hasAttribute !== "function"
            ) {
              return false;
            }
            return String(getAttribute.call(this, "type") || "text").toLowerCase() === "password" &&
              !hasAttribute.call(this, "disabled") &&
              !hasAttribute.call(this, "readonly") &&
              String(getAttribute.call(this, "aria-disabled") || "false").toLowerCase() !== "true" &&
              String(getAttribute.call(this, "aria-readonly") || "false").toLowerCase() !== "true";
          };
          if (!isMaskedEditableInput()) {
            return { ok: false, reason: "target_not_masked" };
          }
          this.focus();
          if (ownerWindow.location.href !== expectedDocumentUrl) {
            return { ok: false, reason: "target_url_changed" };
          }
          const activeElement = composedActiveElement(ownerDocument);
          if (!(activeElement === this || this.contains?.(activeElement) || composedContains(this, activeElement))) {
            return { ok: false, reason: "target_not_focusable" };
          }
          if (!isMaskedEditableInput()) {
            return { ok: false, reason: "target_not_masked" };
          }
          const currentValue = this.isContentEditable
            ? String(this.textContent || "")
            : String(this.value || "");
          const nextValue = append ? currentValue + text : text;
          if (this.isContentEditable) {
            this.textContent = nextValue;
          } else {
            const prototype = this.matches?.("textarea")
              ? ownerWindow.HTMLTextAreaElement?.prototype
              : ownerWindow.HTMLInputElement?.prototype;
            const setter = prototype && Object.getOwnPropertyDescriptor(prototype, "value")?.set;
            if (typeof setter !== "function") {
              return { ok: false, reason: "value_setter_unavailable" };
            }
            setter.call(this, nextValue);
          }
          this.dispatchEvent(new ownerWindow.Event("input", { bubbles: true, composed: true }));
          this.dispatchEvent(new ownerWindow.Event("change", { bubbles: true }));
          if (submit) {
            const form = this.form || this.closest?.("form");
            if (!form) return { ok: false, reason: "form_not_found" };
            if (typeof form.requestSubmit === "function") form.requestSubmit();
            else form.submit();
          }
          return { ok: true };
        }`,
        arguments: [
          { value: action.text },
          { value: action.append },
          { value: action.expectedDocumentUrl },
          { value: action.submit },
        ],
        returnByValue: true,
        awaitPromise: false,
      },
      budget,
    );
  } catch (error) {
    throw postActionError(budget, "secure_fill", error);
  }
  const outcome = response?.result?.value;
  if (response?.exceptionDetails) {
    throw postActionError(budget, "secure_fill", new BrowserActionError(ACTION_ERROR_CODES.FAILED));
  }
  if (outcome?.ok === true) return;
  if (outcome?.reason === "target_url_changed") {
    fail("browser_secret_target_changed", "browser secret target changed before insertion");
  }
  if (outcome?.reason === "target_not_focusable") {
    fail("browser_secret_target_not_focusable", "browser secret target could not receive focus before insertion");
  }
  if (outcome?.reason === "target_not_masked") {
    fail("browser_secret_target_not_masked", "browser secret target must remain an editable password field during insertion");
  }
  fail(ACTION_ERROR_CODES.FAILED, "browser secret target could not receive secure input", {
    reason: outcome?.reason ?? "secure_fill_failed",
  });
}

async function submitNearestForm(connection, sessionId, objectId, budget) {
  let response;
  try {
    response = await sendWithinBudget(
      connection,
      sessionId,
      "Runtime.callFunctionOn",
      {
        objectId,
        functionDeclaration: `function() {
          const form = this.form || this.closest?.("form");
          if (!form) return { ok: false, reason: "form_not_found" };
          if (typeof form.requestSubmit === "function") form.requestSubmit();
          else form.submit();
          return { ok: true };
        }`,
        returnByValue: true,
        awaitPromise: false,
      },
      budget,
    );
  } catch (error) {
    throw postActionError(budget, "submit", error);
  }
  if (response?.exceptionDetails || response?.result?.value?.ok !== true) {
    fail("browser_submit_failed", "browser element does not belong to a submittable form");
  }
}

async function dispatchBackspace(connection, sessionId, budget) {
  try {
    await sendWithinBudget(
      connection,
      sessionId,
      "Input.dispatchKeyEvent",
      { type: "keyDown", key: "Backspace", code: "Backspace" },
      budget,
    );
    await sendWithinBudget(
      connection,
      sessionId,
      "Input.dispatchKeyEvent",
      { type: "keyUp", key: "Backspace", code: "Backspace" },
      budget,
    );
  } catch (error) {
    throw postActionError(budget, "backspace", error);
  }
}

async function executeAction(
  connection,
  sessionId,
  objectId,
  geometry,
  actionability,
  action,
  signal,
  budget,
  documentId,
) {
  assertNotCancelled(signal);
  remainingBudget(budget);
  if (action.kind === "click") {
    return {
      dialogOpened: await dispatchClick(
        connection,
        sessionId,
        objectId,
        geometry,
        budget,
        signal,
        documentId,
      ),
    };
  }
  if (action.kind === "submit") {
    await submitNearestForm(connection, sessionId, objectId, budget);
    return { dialogOpened: false };
  }
  if (action.expectedDocumentUrl !== null) {
    await secureFillElement(connection, sessionId, objectId, action, budget);
    return { dialogOpened: false };
  }
  const encodedBytes = fillRequestBytes(objectId, action.text, action.append);
  if (encodedBytes >= MAX_FILL_REQUEST_BYTES) {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "fill request exceeds the CDP frame budget", {
      reason: "fill_request_too_large",
      encoded_bytes: encodedBytes,
      max_bytes: MAX_FILL_REQUEST_BYTES,
    });
  }
  if (actionability?.isSelect === true) {
    await fillSelect(connection, sessionId, objectId, action, budget);
    if (action.submit) await submitNearestForm(connection, sessionId, objectId, budget);
    return { dialogOpened: false };
  }
  await focusElement(connection, sessionId, objectId, !action.append, budget);
  assertNotCancelled(signal);
  remainingBudget(budget);
  if (action.text) {
    try {
      await sendWithinBudget(
        connection,
        sessionId,
        "Input.insertText",
        { text: action.text },
        budget,
      );
    } catch (error) {
      throw postActionError(budget, "fill", error);
    }
  } else if (!action.append) {
    await dispatchBackspace(connection, sessionId, budget);
  }
  if (action.submit) {
    assertNotCancelled(signal);
    await submitNearestForm(connection, sessionId, objectId, budget);
  }
  return { dialogOpened: false };
}

async function releaseObject(connection, sessionId, objectId, budget) {
  try {
    await sendWithinBudget(
      connection,
      sessionId,
      "Runtime.releaseObject",
      { objectId },
      budget,
    );
    return null;
  } catch (error) {
    // A page may navigate after a successful click. The caller distinguishes
    // that tolerated cleanup failure from a deadline crossing after mutation.
    return error;
  }
}

export async function performBrowserAction({
  connection,
  sessionId,
  targetId,
  documentId,
  nodeRef,
  action,
  timeoutMs,
  deadline,
  signal,
  now = Date.now,
  sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
  assertContext = async () => {},
} = {}) {
  if (
    typeof connection?.send !== "function" ||
    typeof now !== "function" ||
    typeof sleep !== "function" ||
    typeof assertContext !== "function"
  ) {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "browser action dependencies are invalid");
  }
  const backendNodeId = parseBackendNodeReference(nodeRef);
  const normalizedAction = normalizeAction(action);
  const boundedTimeoutMs = normalizeTimeout(timeoutMs);
  const startedAt = readNow(now);
  const requestedDeadline = deadline === undefined ? startedAt + boundedTimeoutMs : Number(deadline);
  if (!Number.isFinite(requestedDeadline)) {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT, "browser action deadline must be finite");
  }
  const budget = {
    deadline: Math.min(startedAt + boundedTimeoutMs, requestedDeadline),
    lastReason: "not_ready",
    attempts: 0,
    now,
    timeoutMs: boundedTimeoutMs,
    assertContext,
  };
  let previousGeometry = null;

  while (true) {
    assertNotCancelled(signal);
    if (budget.attempts > 0 && readNow(now) >= budget.deadline) {
      throw timeoutError(budget, `browser ${normalizedAction.kind} exceeded its action deadline`);
    }
    budget.attempts += 1;
    await assertContext();
    remainingBudget(budget);
    await assertCurrentDocument(connection, sessionId, documentId, budget);
    const objectId = await resolveBackendNode(connection, sessionId, backendNodeId, budget);
    let completed = false;
    let actionResult = { dialogOpened: false };
    let cleanupError = null;
    let releaseInBackground = false;
    try {
      const actionability = await inspectActionability(connection, sessionId, objectId, budget);
      if (actionability.state === "detached") {
        fail(
          ACTION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED,
          "browser element was detached; capture a fresh snapshot before retrying",
        );
      }
      const geometry = readyGeometry(actionability, normalizedAction);
      budget.lastReason = actionability.state;
      if (geometry && sameGeometry(previousGeometry, geometry)) {
        await assertContext();
        await assertCurrentDocument(connection, sessionId, documentId, budget);
        remainingBudget(budget);
        actionResult = await executeAction(
          connection,
          sessionId,
          objectId,
          geometry,
          actionability,
          normalizedAction,
          signal,
          budget,
          documentId,
        );
        releaseInBackground = actionResult.dialogOpened;
        completed = true;
      }
      previousGeometry = geometry;
    } finally {
      if (releaseInBackground) {
        void releaseObject(connection, sessionId, objectId, budget);
      } else {
        cleanupError = await releaseObject(connection, sessionId, objectId, budget);
      }
    }

    if (completed) {
      if (cleanupError && readNow(now) >= budget.deadline) {
        throw postActionError(budget, "cleanup", cleanupError);
      }
      try {
        remainingBudget(budget);
      } catch (error) {
        throw postActionError(budget, "post_action_deadline", error);
      }
      return {
        target_id: targetId,
        document_id: documentId,
        action_kind: normalizedAction.kind,
        dialog_opened: actionResult.dialogOpened,
        attempts: budget.attempts,
        elapsed_ms: Math.max(0, readNow(now) - startedAt),
      };
    }

    const remaining = remainingBudget(budget);
    await sleep(Math.min(ACTION_POLL_INTERVAL_MS, remaining));
  }
}
