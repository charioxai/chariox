const DEFAULT_ACTION_TIMEOUT_MS = 5_000;
const MAX_ACTION_TIMEOUT_MS = 5_000;
const MIN_ACTION_TIMEOUT_MS = 100;
const ACTION_POLL_INTERVAL_MS = 50;
const MAX_FILL_TEXT_BYTES = 65_536;

export class BrowserActionError extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.name = "BrowserActionError";
    this.code = code;
    Object.assign(this, details);
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
  now = Date.now,
  sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
}) {
  const backendNodeId = parseBackendNodeReference(nodeRef);
  const normalizedAction = normalizeAction(action);
  const boundedTimeoutMs = normalizeTimeout(timeoutMs);
  const startedAt = now();
  let attempts = 0;
  let previousGeometry = null;
  let lastReason = "not_ready";

  while (true) {
    if (attempts > 0 && now() - startedAt >= boundedTimeoutMs) {
      throw new BrowserActionError(
        "browser_action_timeout",
        `browser ${normalizedAction.kind} did not become actionable within ${boundedTimeoutMs}ms`,
        { reason: lastReason, attempts, timeoutMs: boundedTimeoutMs },
      );
    }
    attempts += 1;
    await assertCurrentDocument(connection, sessionId, targetId, documentId);
    const objectId = await resolveBackendNode(connection, sessionId, backendNodeId);
    try {
      const actionability = await inspectActionability(connection, sessionId, objectId);
      const geometry = readyGeometry(actionability, normalizedAction);
      lastReason = actionability.state;
      if (geometry && sameGeometry(previousGeometry, geometry)) {
        await executeAction(
          connection,
          sessionId,
          objectId,
          geometry,
          normalizedAction,
        );
        return {
          target_id: targetId,
          document_id: documentId,
          action_kind: normalizedAction.kind,
          attempts,
          elapsed_ms: Math.max(0, now() - startedAt),
        };
      }
      previousGeometry = geometry;
    } finally {
      await releaseObject(connection, sessionId, objectId);
    }
    await sleep(ACTION_POLL_INTERVAL_MS);
  }
}

function normalizeAction(action) {
  if (action?.kind === "click") {
    return { kind: "click" };
  }
  if (action?.kind === "fill") {
    if (typeof action.text !== "string") {
      throw invalidAction("fill text must be a string");
    }
    if (utf8ByteLength(action.text) > MAX_FILL_TEXT_BYTES) {
      throw invalidAction(`fill text exceeds ${MAX_FILL_TEXT_BYTES} UTF-8 bytes`);
    }
    return {
      kind: "fill",
      text: action.text,
      append: action.append === true,
    };
  }
  throw invalidAction(`unsupported browser action ${JSON.stringify(action?.kind ?? null)}`);
}

function normalizeTimeout(timeoutMs) {
  if (timeoutMs === undefined) {
    return DEFAULT_ACTION_TIMEOUT_MS;
  }
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) {
    throw invalidAction("browser action timeout must be a positive integer");
  }
  return Math.max(MIN_ACTION_TIMEOUT_MS, Math.min(timeoutMs, MAX_ACTION_TIMEOUT_MS));
}

function parseBackendNodeReference(nodeRef) {
  if (typeof nodeRef !== "string" || !/^backend:[1-9][0-9]*$/.test(nodeRef)) {
    throw invalidAction("browser action requires a valid controller node reference");
  }
  const backendNodeId = Number(nodeRef.slice("backend:".length));
  if (!Number.isSafeInteger(backendNodeId)) {
    throw invalidAction("browser action node reference exceeds the safe integer range");
  }
  return backendNodeId;
}

function invalidAction(message) {
  return new BrowserActionError("browser_action_invalid", message);
}

async function assertCurrentDocument(connection, sessionId, targetId, documentId) {
  const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
  const currentDocumentId = frameTree?.frameTree?.frame?.loaderId;
  if (currentDocumentId !== documentId) {
    throw new BrowserActionError(
      "stale_document_reference",
      `browser target ${JSON.stringify(targetId)} moved away from the requested document`,
    );
  }
}

async function resolveBackendNode(connection, sessionId, backendNodeId) {
  let resolved;
  try {
    resolved = await connection.send(
      "DOM.resolveNode",
      { backendNodeId },
      sessionId,
    );
  } catch (error) {
    if (error?.code !== "browser_cdp_command_failed") {
      throw error;
    }
    throw new BrowserActionError(
      "stale_element_reference",
      "browser element is no longer attached to the current document",
    );
  }
  const objectId = resolved?.object?.objectId;
  if (typeof objectId !== "string" || !objectId) {
    throw new BrowserActionError(
      "stale_element_reference",
      "browser element is no longer attached to the current document",
    );
  }
  return objectId;
}

async function inspectActionability(connection, sessionId, objectId) {
  const response = await connection.send(
    "Runtime.callFunctionOn",
    {
      objectId,
      functionDeclaration: actionabilityFunction.toString(),
      returnByValue: true,
      awaitPromise: false,
    },
    sessionId,
  );
  if (response?.exceptionDetails) {
    throw new BrowserActionError(
      "browser_action_failed",
      "browser actionability inspection failed",
    );
  }
  const result = response?.result?.value;
  if (!result || typeof result.state !== "string") {
    throw new BrowserActionError(
      "browser_action_failed",
      "browser actionability inspection returned an invalid result",
    );
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
    Number(style.opacity) === 0 ||
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
  const hitTarget = document.elementFromPoint(x, y);
  if (!hitTarget || (hitTarget !== this && !this.contains?.(hitTarget))) {
    return { state: "obscured" };
  }
  const inputType = this.matches?.("input")
    ? String(this.type || "text").toLowerCase()
    : null;
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
    this.isContentEditable ||
    ((editableInput || (this.matches?.("textarea") ?? false)) && !this.readOnly);
  return {
    state: "ready",
    x,
    y,
    width: rect.width,
    height: rect.height,
    editable,
  };
}

function readyGeometry(actionability, action) {
  if (actionability.state !== "ready") {
    return null;
  }
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
    left.x === right.x &&
    left.y === right.y &&
    left.width === right.width &&
    left.height === right.height;
}

async function executeAction(
  connection,
  sessionId,
  objectId,
  geometry,
  action,
) {
  if (action.kind === "click") {
    await dispatchClick(connection, sessionId, geometry.x, geometry.y);
    return;
  }
  await focusElement(connection, sessionId, objectId, !action.append);
  if (action.text) {
    await connection.send("Input.insertText", { text: action.text }, sessionId);
  } else if (!action.append) {
    await dispatchBackspace(connection, sessionId);
  }
}

async function dispatchClick(connection, sessionId, x, y) {
  await connection.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y }, sessionId);
  await connection.send(
    "Input.dispatchMouseEvent",
    { type: "mousePressed", x, y, button: "left", clickCount: 1 },
    sessionId,
  );
  await connection.send(
    "Input.dispatchMouseEvent",
    { type: "mouseReleased", x, y, button: "left", clickCount: 1 },
    sessionId,
  );
}

async function focusElement(connection, sessionId, objectId, selectAll) {
  const response = await connection.send(
    "Runtime.callFunctionOn",
    {
      objectId,
      functionDeclaration: `function(selectAll) {
        this.focus();
        const focused = document.activeElement === this || this.contains?.(document.activeElement);
        if (focused && selectAll) {
          if (typeof this.select === "function") {
            this.select();
          } else {
            const selection = window.getSelection();
            const range = document.createRange();
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
    sessionId,
  );
  if (response?.exceptionDetails || response?.result?.value?.ok !== true) {
    throw new BrowserActionError(
      "browser_action_failed",
      "browser fill target could not receive focus",
    );
  }
}

async function dispatchBackspace(connection, sessionId) {
  await connection.send(
    "Input.dispatchKeyEvent",
    { type: "keyDown", key: "Backspace", code: "Backspace" },
    sessionId,
  );
  await connection.send(
    "Input.dispatchKeyEvent",
    { type: "keyUp", key: "Backspace", code: "Backspace" },
    sessionId,
  );
}

async function releaseObject(connection, sessionId, objectId) {
  try {
    await connection.send("Runtime.releaseObject", { objectId }, sessionId);
  } catch {
    // The page may have navigated after a successful click.
  }
}

function utf8ByteLength(value) {
  let length = 0;
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    length += codePoint <= 0x7f
      ? 1
      : codePoint <= 0x7ff
        ? 2
        : codePoint <= 0xffff
          ? 3
          : 4;
    if (length > MAX_FILL_TEXT_BYTES) {
      break;
    }
  }
  return length;
}
