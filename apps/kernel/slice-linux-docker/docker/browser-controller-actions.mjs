const DEFAULT_ACTION_TIMEOUT_MS = 5_000;
const MAX_ACTION_TIMEOUT_MS = 5_000;
const MIN_ACTION_TIMEOUT_MS = 100;
const ACTION_POLL_INTERVAL_MS = 50;
// Leave room for the Runtime.callFunctionOn envelope inside the 64 KiB CDP frame cap.
const MAX_FILL_TEXT_BYTES = 48 * 1024;

export const ACTION_ERROR_CODES = Object.freeze({
  INVALID_ARGUMENT: "ACTION_INVALID",
  TIMEOUT: "ACTION_TIMEOUT",
  FAILED: "ACTION_FAILED",
  STALE_DOCUMENT: "STALE_DOCUMENT",
  ELEMENT_REFERENCE_INVALIDATED: "ELEMENT_REFERENCE_INVALIDATED",
  FRAME_UNSUPPORTED: "FRAME_UNSUPPORTED",
});

export class BrowserActionError extends Error {
  constructor(code, details = {}) {
    super(code);
    this.name = "BrowserActionError";
    this.code = code;
    this.details = details;
  }
}

function fail(code, details) {
  throw new BrowserActionError(code, details);
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requirePositiveInteger(value) {
  if (!Number.isSafeInteger(value) || value < 1) {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT);
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
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT);
  }
  return value;
}

function normalizeElement(raw) {
  if (!isPlainObject(raw)) fail(ACTION_ERROR_CODES.INVALID_ARGUMENT);
  return {
    tabId: requireIdentifier(raw.tab_id),
    documentId: requireIdentifier(raw.document_id),
    frameId: requireIdentifier(raw.frame_id),
    mainFrameId: requireIdentifier(raw.main_frame_id),
    snapshotRevision: requirePositiveInteger(raw.snapshot_revision),
    backendNodeId: requirePositiveInteger(raw.backend_node_id),
  };
}

function utf8ByteLength(value) {
  return Buffer.byteLength(value, "utf8");
}

function normalizeAction(raw) {
  if (!isPlainObject(raw) || typeof raw.kind !== "string") {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT);
  }
  if (raw.kind === "click") {
    if (Object.keys(raw).some((key) => key !== "kind")) {
      fail(ACTION_ERROR_CODES.INVALID_ARGUMENT);
    }
    return { kind: "click" };
  }
  if (raw.kind === "submit") {
    if (Object.keys(raw).some((key) => key !== "kind")) {
      fail(ACTION_ERROR_CODES.INVALID_ARGUMENT);
    }
    return { kind: "submit" };
  }
  if (raw.kind === "fill") {
    if (
      Object.keys(raw).some((key) => !["kind", "text", "append"].includes(key)) ||
      typeof raw.text !== "string" ||
      (raw.append !== undefined && typeof raw.append !== "boolean") ||
      utf8ByteLength(raw.text) > MAX_FILL_TEXT_BYTES
    ) {
      fail(ACTION_ERROR_CODES.INVALID_ARGUMENT);
    }
    return { kind: "fill", text: raw.text, append: raw.append === true };
  }
  fail(ACTION_ERROR_CODES.INVALID_ARGUMENT);
}

function normalizeTimeout(value) {
  if (value === undefined) return DEFAULT_ACTION_TIMEOUT_MS;
  if (!Number.isSafeInteger(value) || value < 1) {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT);
  }
  return Math.max(MIN_ACTION_TIMEOUT_MS, Math.min(value, MAX_ACTION_TIMEOUT_MS));
}

function readNow(now) {
  const value = Number(now());
  if (!Number.isFinite(value)) fail(ACTION_ERROR_CODES.FAILED);
  return value;
}

function timeoutDetails(budget) {
  return {
    attempts: budget.attempts,
    timeout_ms: budget.timeoutMs,
    reason: budget.lastReason,
  };
}

function remainingBudget(budget) {
  const remaining = Math.ceil(budget.deadline - readNow(budget.now));
  if (remaining <= 0) {
    fail(ACTION_ERROR_CODES.TIMEOUT, timeoutDetails(budget));
  }
  return remaining;
}

async function sendWithinBudget(connection, method, params, budget) {
  const timeoutMs = remainingBudget(budget);
  try {
    const result = await connection.send(method, params, timeoutMs);
    remainingBudget(budget);
    return result;
  } catch (error) {
    if (error instanceof BrowserActionError) throw error;
    if (error?.code === "CDP_COMMAND_TIMEOUT" || readNow(budget.now) >= budget.deadline) {
      fail(ACTION_ERROR_CODES.TIMEOUT, timeoutDetails(budget));
    }
    throw error;
  }
}

async function assertCurrentDocument(connection, documentId, budget) {
  const frameTree = await sendWithinBudget(connection, "Page.getFrameTree", {}, budget);
  if (frameTree?.frameTree?.frame?.loaderId !== documentId) {
    fail(ACTION_ERROR_CODES.STALE_DOCUMENT);
  }
}

async function resolveBackendNode(connection, backendNodeId, budget) {
  let response;
  try {
    response = await sendWithinBudget(connection, "DOM.resolveNode", { backendNodeId }, budget);
  } catch (error) {
    if (error?.code === "CDP_COMMAND_FAILED") {
      fail(ACTION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED);
    }
    throw error;
  }
  const objectId = response?.object?.objectId;
  if (typeof objectId !== "string" || objectId.length === 0) {
    fail(ACTION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED);
  }
  return objectId;
}

async function inspectActionability(connection, objectId, budget) {
  const response = await sendWithinBudget(connection, "Runtime.callFunctionOn", {
    objectId,
    functionDeclaration: actionabilityFunction.toString(),
    returnByValue: true,
    awaitPromise: false,
  }, budget);
  if (response?.exceptionDetails) fail(ACTION_ERROR_CODES.FAILED);
  const result = response?.result?.value;
  if (!isPlainObject(result) || typeof result.state !== "string") {
    fail(ACTION_ERROR_CODES.FAILED);
  }
  return result;
}

function actionabilityFunction() {
  if (!this.isConnected) return { state: "detached" };
  this.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
  const style = window.getComputedStyle(this);
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
  const x = rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  const hit = document.elementFromPoint(x, y);
  if (!hit || (hit !== this && !this.contains?.(hit))) {
    return { state: "obscured" };
  }
  const tag = String(this.tagName || "").toLowerCase();
  const type = tag === "input" ? String(this.type || "text").toLowerCase() : "";
  const fillableInput = tag === "input" && ![
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
  ].includes(type);
  const editable =
    tag === "select" ||
    this.isContentEditable ||
    ((fillableInput || tag === "textarea") && !this.readOnly);
  return {
    state: "ready",
    x,
    y,
    width: rect.width,
    height: rect.height,
    editable,
  };
}

function actionableGeometry(result, action) {
  if (result.state !== "ready") return null;
  if (action.kind === "fill" && result.editable !== true) {
    result.state = "not_editable";
    return null;
  }
  const geometry = {
    x: result.x,
    y: result.y,
    width: result.width,
    height: result.height,
  };
  return Object.values(geometry).every(Number.isFinite) ? geometry : null;
}

function sameGeometry(left, right) {
  return left !== null &&
    left.x === right.x &&
    left.y === right.y &&
    left.width === right.width &&
    left.height === right.height;
}

async function click(connection, geometry, budget) {
  await sendWithinBudget(connection, "Input.dispatchMouseEvent", {
    type: "mouseMoved",
    x: geometry.x,
    y: geometry.y,
  }, budget);
  await sendWithinBudget(connection, "Input.dispatchMouseEvent", {
    type: "mousePressed",
    x: geometry.x,
    y: geometry.y,
    button: "left",
    clickCount: 1,
  }, budget);
  await sendWithinBudget(connection, "Input.dispatchMouseEvent", {
    type: "mouseReleased",
    x: geometry.x,
    y: geometry.y,
    button: "left",
    clickCount: 1,
  }, budget);
}

async function fill(connection, objectId, action, budget) {
  const response = await sendWithinBudget(connection, "Runtime.callFunctionOn", {
    objectId,
    functionDeclaration: fillFunction.toString(),
    arguments: [{ value: action.text }, { value: action.append }],
    returnByValue: true,
    awaitPromise: false,
  }, budget);
  if (response?.exceptionDetails || response?.result?.value?.ok !== true) {
    fail(ACTION_ERROR_CODES.FAILED);
  }
}

async function submit(connection, objectId, budget) {
  const response = await sendWithinBudget(connection, "Runtime.callFunctionOn", {
    objectId,
    functionDeclaration: submitFunction.toString(),
    returnByValue: true,
    awaitPromise: false,
  }, budget);
  if (response?.exceptionDetails || response?.result?.value?.ok !== true) {
    fail(ACTION_ERROR_CODES.FAILED);
  }
}

function submitFunction() {
  if (!this.isConnected) return { ok: false };
  const form = this.matches?.("form") ? this : this.closest?.("form");
  if (!form) return { ok: false };
  if (typeof form.requestSubmit === "function") {
    const isSubmitter = this !== form && this.matches?.("button[type=submit], input[type=submit], button:not([type])");
    if (isSubmitter) form.requestSubmit(this);
    else form.requestSubmit();
  } else if (typeof form.submit === "function") {
    form.submit();
  } else {
    return { ok: false };
  }
  return { ok: true };
}

function fillFunction(text, append) {
  if (!this.isConnected || this.disabled || this.readOnly) return { ok: false };
  this.focus();
  if (document.activeElement !== this && !this.contains?.(document.activeElement)) {
    return { ok: false };
  }
  const tag = String(this.tagName || "").toLowerCase();
  if (tag === "select") {
    const option = Array.from(this.options || []).find(
      (candidate) => candidate.value === text || candidate.text === text,
    );
    if (!option) return { ok: false };
    this.value = option.value;
  } else if ("value" in this) {
    const previous = String(this.value || "");
    const next = append ? previous + text : text;
    const prototype = Object.getPrototypeOf(this);
    const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
    if (typeof setter === "function") setter.call(this, next);
    else this.value = next;
    if (String(this.value) !== next) return { ok: false };
  } else if (this.isContentEditable) {
    const previous = String(this.textContent || "");
    this.textContent = append ? previous + text : text;
  } else {
    return { ok: false };
  }
  this.dispatchEvent(new Event("input", { bubbles: true }));
  this.dispatchEvent(new Event("change", { bubbles: true }));
  return { ok: true };
}

async function releaseObject(connection, objectId, budget) {
  try {
    await sendWithinBudget(connection, "Runtime.releaseObject", { objectId }, budget);
  } catch {
    // A click may navigate and invalidate the object before release.
  }
}

export async function performBrowserAction({
  connection,
  element: rawElement,
  action: rawAction,
  timeoutMs,
  now = Date.now,
  sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
} = {}) {
  if (typeof connection?.send !== "function" || typeof now !== "function" || typeof sleep !== "function") {
    fail(ACTION_ERROR_CODES.INVALID_ARGUMENT);
  }
  const element = normalizeElement(rawElement);
  const action = normalizeAction(rawAction);
  const boundedTimeoutMs = normalizeTimeout(timeoutMs);
  const startedAt = readNow(now);
  const budget = {
    deadline: startedAt + boundedTimeoutMs,
    lastReason: "not_ready",
    attempts: 0,
    now,
    timeoutMs: boundedTimeoutMs,
  };
  if (element.frameId !== element.mainFrameId) {
    fail(ACTION_ERROR_CODES.FRAME_UNSUPPORTED);
  }
  let previousGeometry = null;

  while (true) {
    const elapsed = Math.max(0, readNow(now) - startedAt);
    if (budget.attempts > 0 && elapsed >= boundedTimeoutMs) {
      fail(ACTION_ERROR_CODES.TIMEOUT, timeoutDetails(budget));
    }
    budget.attempts += 1;
    await assertCurrentDocument(connection, element.documentId, budget);
    const objectId = await resolveBackendNode(connection, element.backendNodeId, budget);
    let completed = false;
    try {
      const result = await inspectActionability(connection, objectId, budget);
      if (result.state === "detached") {
        fail(ACTION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED);
      }
      const geometry = actionableGeometry(result, action);
      budget.lastReason = result.state;
      if (geometry && sameGeometry(previousGeometry, geometry)) {
        await assertCurrentDocument(connection, element.documentId, budget);
        remainingBudget(budget);
        if (action.kind === "click") await click(connection, geometry, budget);
        else if (action.kind === "fill") await fill(connection, objectId, action, budget);
        else await submit(connection, objectId, budget);
        completed = true;
      }
      previousGeometry = geometry;
    } finally {
      await releaseObject(connection, objectId, budget);
    }
    if (completed) {
      remainingBudget(budget);
      return {
        tab_id: element.tabId,
        document_id: element.documentId,
        snapshot_revision: element.snapshotRevision,
        action_kind: action.kind,
        attempts: budget.attempts,
        elapsed_ms: Math.max(0, readNow(now) - startedAt),
      };
    }
    const remaining = boundedTimeoutMs - Math.max(0, readNow(now) - startedAt);
    if (remaining <= 0) {
      fail(ACTION_ERROR_CODES.TIMEOUT, timeoutDetails(budget));
    }
    await sleep(Math.max(1, Math.min(ACTION_POLL_INTERVAL_MS, remaining)));
  }
}
