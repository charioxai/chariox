const DEFAULT_ACTION_TIMEOUT_MS = 5_000;
const MAX_ACTION_TIMEOUT_MS = 5_000;
const MIN_ACTION_TIMEOUT_MS = 100;
const ACTION_POLL_INTERVAL_MS = 50;
const MAX_FILL_TEXT_BYTES = 65_536;
const DIALOG_OPEN_WAIT_MS = 5_000;

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
  signal,
  now = Date.now,
  sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
  assertContext = async () => {},
  withInput = operation => operation(),
}) {
  const backendNodeId = parseBackendNodeReference(nodeRef);
  const normalizedAction = normalizeAction(action);
  const boundedTimeoutMs = normalizeTimeout(timeoutMs);
  const startedAt = now();
  let attempts = 0;
  let previousGeometry = null;
  let lastReason = "not_ready";
  let releaseInBackground = false;

  while (true) {
    assertNotCancelled(signal);
    if (attempts > 0 && now() - startedAt >= boundedTimeoutMs) {
      throw new BrowserActionError(
        "browser_action_timeout",
        `browser ${normalizedAction.kind} did not become actionable within ${boundedTimeoutMs}ms`,
        { reason: lastReason, attempts, timeoutMs: boundedTimeoutMs },
      );
    }
    attempts += 1;
    await assertContext();
    await assertCurrentDocument(connection, sessionId, targetId, documentId);
    const objectId = await resolveBackendNode(connection, sessionId, backendNodeId, normalizedAction.expectedDocumentUrl != null);
    try {
      const actionability = await inspectActionability(connection, sessionId, objectId);
      if (actionability.state === "detached") {
        throw new BrowserActionError(
          "stale_element_reference",
          "browser element was detached; capture a fresh snapshot before retrying",
        );
      }
      const geometry = readyGeometry(actionability, normalizedAction);
      lastReason = actionability.state;
      if (geometry && sameGeometry(previousGeometry, geometry)) {
        assertNotCancelled(signal);
        const actionResult = await withInput(() => executeAction(
          connection,
          sessionId,
          objectId,
          geometry,
          normalizedAction,
          signal,
        ));
        releaseInBackground = actionResult.dialogOpened;
        return {
          target_id: targetId,
          document_id: documentId,
          action_kind: normalizedAction.kind,
          dialog_opened: actionResult.dialogOpened,
          attempts,
          elapsed_ms: Math.max(0, now() - startedAt),
        };
      }
      previousGeometry = geometry;
    } finally {
      if (releaseInBackground) {
        void releaseObject(connection, sessionId, objectId);
      } else {
        await releaseObject(connection, sessionId, objectId);
      }
    }
    await sleep(ACTION_POLL_INTERVAL_MS);
  }
}

export function assertNotCancelled(signal) {
  if (signal?.aborted) {
    throw new BrowserActionError("browser_action_cancelled", "browser action was cancelled");
  }
}

function normalizeAction(action) {
  if (action?.kind === "click") {
    return { kind: "click", expectedDocumentUrl: normalizeExpectedDocumentUrl(action.expected_document_url) };
  }
  if (action?.kind === "fill") {
    if (typeof action.text !== "string") {
      throw invalidAction("fill text must be a string");
    }
    if (utf8ByteLength(action.text) > MAX_FILL_TEXT_BYTES) {
      throw invalidAction(`fill text exceeds ${MAX_FILL_TEXT_BYTES} UTF-8 bytes`);
    }
    const expectedDocumentUrl = normalizeExpectedDocumentUrl(action.expected_document_url);
    if (action.mask_code_input === true && expectedDocumentUrl === null) {
      throw invalidAction("protected owner code requires a bound document URL");
    }
    return {
      kind: "fill",
      text: action.text,
      append: action.append === true,
      submit: action.submit === true,
      expectedDocumentUrl,
      maskCodeInput: action.mask_code_input === true,
    };
  }
  if (action?.kind === "submit") {
    return { kind: "submit" };
  }
  throw invalidAction(`unsupported browser action ${JSON.stringify(action?.kind ?? null)}`);
}

function normalizeExpectedDocumentUrl(value) {
  if (value === undefined || value === null) return null;
  if (typeof value !== "string" || value.length === 0 || utf8ByteLength(value) > 2_048) {
    throw invalidAction("secret fill expected document URL is invalid");
  }
  try {
    return new URL(value).href;
  } catch {
    throw invalidAction("secret fill expected document URL is invalid");
  }
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

export async function assertCurrentDocument(connection, sessionId, targetId, documentId) {
  const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
  const currentDocumentId = frameTree?.frameTree?.frame?.loaderId;
  if (currentDocumentId !== documentId) {
    throw new BrowserActionError(
      "stale_document_reference",
      `browser target ${JSON.stringify(targetId)} moved away from the requested document`,
    );
  }
}

async function resolveBackendNode(connection, sessionId, backendNodeId, isolated = false) {
  let resolved;
  try {
    let executionContextId;
    if (isolated) {
      // CDP owns frame/node identities. Do not ask page JavaScript (including
      // overridden ownerDocument/defaultView getters) which realm owns a node.
      const snapshot = await connection.send("DOMSnapshot.captureSnapshot", { computedStyles: [] }, sessionId);
      const document = snapshot.documents?.find(document => document.nodes?.backendNodeId?.includes(backendNodeId));
      const frameId = snapshot.strings?.[document?.frameId];
      if (typeof frameId !== "string" || !frameId) {
        throw new BrowserActionError("stale_element_reference", "browser secret target frame is unavailable");
      }
      const world = await connection.send("Page.createIsolatedWorld", {
        frameId, worldName: "chariox-secret-fill", grantUniveralAccess: false,
      }, sessionId);
      executionContextId = world?.executionContextId;
      if (!Number.isSafeInteger(executionContextId) || executionContextId <= 0) {
        throw new BrowserActionError("browser_action_failed", "browser secret target isolated world is unavailable");
      }
    }
    resolved = await connection.send(
      "DOM.resolveNode",
      { backendNodeId, ...(isolated ? { executionContextId } : {}) },
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
  if (result.state === "ready" && result.crossOriginFrame) {
    const { quads } = await connection.send("DOM.getContentQuads", { objectId }, sessionId);
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
  if (!this.isConnected) return { state: "detached" };
  this.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
  const ownerDocument = this.ownerDocument;
  const ownerWindow = ownerDocument.defaultView;
  const style = ownerWindow.getComputedStyle(this);
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
  const localX = rect.left + rect.width / 2;
  const localY = rect.top + rect.height / 2;
  let hitTarget = ownerDocument.elementFromPoint(localX, localY);
  while (hitTarget?.shadowRoot?.elementFromPoint) {
    const nestedTarget = hitTarget.shadowRoot.elementFromPoint(localX, localY);
    if (!nestedTarget || nestedTarget === hitTarget) break;
    hitTarget = nestedTarget;
  }
  let hitInsideTarget = hitTarget === this || this.contains?.(hitTarget);
  let hitRoot = hitTarget?.getRootNode?.();
  while (!hitInsideTarget && hitRoot?.host) {
    hitInsideTarget = hitRoot.host === this || this.contains?.(hitRoot.host);
    hitRoot = hitRoot.host.getRootNode?.();
  }
  if (!hitTarget || !hitInsideTarget) {
    return { state: "obscured" };
  }
  let x = localX;
  let y = localY;
  let currentWindow = ownerWindow;
  let crossOriginFrame = false;
  while (currentWindow && currentWindow !== currentWindow.top) {
    const frameElement = currentWindow.frameElement;
    if (!frameElement) {
      crossOriginFrame = true;
      break;
    }
    const frameRect = frameElement.getBoundingClientRect();
    const frameStyle = frameElement.ownerDocument.defaultView.getComputedStyle(frameElement);
    const paddingLeft = Number.parseFloat(frameStyle.paddingLeft) || 0;
    const paddingTop = Number.parseFloat(frameStyle.paddingTop) || 0;
    x += frameRect.left + frameElement.clientLeft + paddingLeft;
    y += frameRect.top + frameElement.clientTop + paddingTop;
    currentWindow = frameElement.ownerDocument.defaultView;
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
    ...(crossOriginFrame ? { crossOriginFrame: true } : {}),
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
  signal,
) {
  assertNotCancelled(signal);
  if (action.kind === "click") {
    const assertTarget = action.expectedDocumentUrl === null ? async () => {} : async () => {
      const response = await connection.send("Runtime.callFunctionOn", {
        objectId,
        functionDeclaration: "function() { return this.ownerDocument.defaultView.location.href; }",
        returnByValue: true,
      }, sessionId);
      if (response.exceptionDetails || response.result?.value !== action.expectedDocumentUrl) {
        throw new BrowserActionError("stale_document_reference", "owner click target URL changed before input");
      }
    };
    return {
      dialogOpened: await dispatchClick(connection, sessionId, geometry.x, geometry.y, signal, assertTarget),
    };
  }
  if (action.kind === "submit") {
    await submitNearestForm(connection, sessionId, objectId);
    return { dialogOpened: false };
  }
  if (action.expectedDocumentUrl !== null) {
    await secureFillElement(connection, sessionId, objectId, action);
    return { dialogOpened: false };
  }
  await focusElement(connection, sessionId, objectId, !action.append);
  assertNotCancelled(signal);
  if (action.text) {
    await connection.send("Input.insertText", { text: action.text }, sessionId);
  } else if (!action.append) {
    await dispatchBackspace(connection, sessionId);
  }
  if (action.submit) {
    assertNotCancelled(signal);
    await submitNearestForm(connection, sessionId, objectId);
  }
  return { dialogOpened: false };
}

async function secureFillElement(connection, sessionId, objectId, action) {
  const response = await connection.send(
    "Runtime.callFunctionOn",
    {
      objectId,
      functionDeclaration: `function(text, append, expectedDocumentUrl, submit, maskCodeInput) {
        const ownerDocument = this.ownerDocument;
        const ownerWindow = ownerDocument?.defaultView;
        if (!ownerWindow || ownerWindow.location.href !== expectedDocumentUrl) {
          return { ok: false, reason: "target_url_changed" };
        }
        // MP-08 / MP-10 / MP-11: owner codes may use ordinary editable
        // fields. Mask before focus/value handlers; Vault fills remain password-only.
        const inputPrototype = ownerWindow.HTMLInputElement?.prototype;
        const textareaPrototype = ownerWindow.HTMLTextAreaElement?.prototype;
        const elementPrototype = ownerWindow.Element?.prototype;
        const getAttribute = elementPrototype?.getAttribute;
        const hasAttribute = elementPrototype?.hasAttribute;
        const isInput = !!inputPrototype && Object.prototype.isPrototypeOf.call(inputPrototype, this);
        const isTextarea = !!textareaPrototype && Object.prototype.isPrototypeOf.call(textareaPrototype, this);
        const editable = () => typeof getAttribute === "function" && typeof hasAttribute === "function" &&
          !hasAttribute.call(this, "disabled") && !hasAttribute.call(this, "readonly") &&
          String(getAttribute.call(this, "aria-disabled") || "false").toLowerCase() !== "true" &&
          String(getAttribute.call(this, "aria-readonly") || "false").toLowerCase() !== "true";
        const isMaskedEditableInput = () => {
          if (!editable()) return false;
          if (isInput) return String(getAttribute.call(this, "type") || "text").toLowerCase() === "password";
          if (!maskCodeInput || !isTextarea || typeof ownerWindow.getComputedStyle !== "function") return false;
          return ownerWindow.getComputedStyle(this).getPropertyValue("-webkit-text-security") === "disc";
        };
        const maskCodeTarget = () => {
          if (isInput) {
            const setter = Object.getOwnPropertyDescriptor(inputPrototype, "type")?.set;
            if (typeof setter !== "function") return false;
            setter.call(this, "password");
          } else if (isTextarea && maskCodeInput) {
            const setter = ownerWindow.CSSStyleDeclaration?.prototype?.setProperty;
            if (typeof setter !== "function") return false;
            setter.call(this.style, "-webkit-text-security", "disc", "important");
          } else return false;
          return isMaskedEditableInput();
        };
        if (maskCodeInput) {
          const codeType = editable() ? String(getAttribute.call(this, "type") || "text").toLowerCase() : "";
          if (!editable() || !(isTextarea || (isInput && ["text", "tel", "number", "password", "search"].includes(codeType)))) {
            return { ok: false, reason: "target_not_masked" };
          }
          if (!isMaskedEditableInput() && !maskCodeTarget()) return { ok: false, reason: "target_not_masked" };
        }
        const restoreMaskAfterHandler = () => {
          if (isMaskedEditableInput()) return true;
          maskCodeTarget();
          return false;
        };
        if (!isMaskedEditableInput()) return { ok: false, reason: "target_not_masked" };
        // MP-08/MP-10: submission eligibility is checked before secret mutation.
        // Some sign-in steps use a separate JavaScript button and no native form.
        const form = submit ? this.form || this.closest?.("form") : null;
        const formPrototype = ownerWindow.HTMLFormElement?.prototype;
        const submitForm = formPrototype?.requestSubmit ?? formPrototype?.submit;
        if (submit && !form) return { ok: false, reason: "form_not_found" };
        if (submit && typeof submitForm !== "function") {
          return { ok: false, reason: "form_submit_unavailable" };
        }
        this.focus();
        if (ownerWindow.location.href !== expectedDocumentUrl) {
          return { ok: false, reason: "target_url_changed" };
        }
        const activeElement = (this.getRootNode?.() ?? ownerDocument).activeElement;
        if (!(activeElement === this || this.contains?.(activeElement))) {
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
        if (!restoreMaskAfterHandler()) return { ok: false, reason: "target_not_masked" };
        this.dispatchEvent(new ownerWindow.Event("change", { bubbles: true }));
        if (!restoreMaskAfterHandler()) return { ok: false, reason: "target_not_masked" };
        if (submit) {
          // Form controls named submit/requestSubmit must not shadow DOM methods.
          try { submitForm.call(form); }
          catch { return { ok: false, reason: "form_submit_failed" }; }
        }
        return { ok: true };
      }`,
      arguments: [
        { value: action.text },
        { value: action.append },
        { value: action.expectedDocumentUrl },
        { value: action.submit },
        { value: action.maskCodeInput },
      ],
      returnByValue: true,
      awaitPromise: false,
    },
    sessionId,
  );
  const outcome = response?.result?.value;
  if (response?.exceptionDetails || outcome?.ok !== true) {
    if (outcome?.reason === "target_url_changed") {
      throw new BrowserActionError(
        "browser_secret_target_changed",
        "browser secret target changed before insertion",
      );
    }
    if (outcome?.reason === "target_not_focusable") {
      throw new BrowserActionError(
        "browser_secret_target_not_focusable",
        "browser secret target could not receive focus before insertion",
      );
    }
    if (outcome?.reason === "target_not_masked") {
      throw new BrowserActionError(
        "browser_secret_target_not_masked",
        "browser protected target must remain masked and editable during insertion",
      );
    }
    if (outcome?.reason === "form_not_found") {
      throw new BrowserActionError(
        "browser_submit_failed",
        "browser password field has no native form; use submit=false, then click the observed sign-in button",
      );
    }
    if (["form_submit_unavailable", "form_submit_failed"].includes(outcome?.reason)) {
      throw new BrowserActionError(
        "browser_submit_failed",
        "browser password form could not submit",
        { reason: outcome.reason },
      );
    }
    if (response?.exceptionDetails) {
      throw new BrowserActionError(
        "browser_secret_input_exception",
        "browser secret insertion script raised an exception; rediscover the password field before retrying",
      );
    }
    if (outcome?.reason === "value_setter_unavailable") {
      throw new BrowserActionError(
        "browser_action_failed",
        "browser password field has no native value setter",
      );
    }
    throw new BrowserActionError(
      "browser_action_failed",
      "browser secret target could not receive secure input",
      // Do not expose exception text: page code can include the inserted secret.
      { reason: "secure_fill_failed" },
    );
  }
}

async function submitNearestForm(connection, sessionId, objectId) {
  const response = await connection.send(
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
    sessionId,
  );
  if (response?.exceptionDetails || response?.result?.value?.ok !== true) {
    throw new BrowserActionError(
      "browser_submit_failed",
      "browser element does not belong to a submittable form",
    );
  }
}

async function dispatchClick(connection, sessionId, x, y, signal, assertTarget = async () => {}) {
  await assertTarget();
  if (await dispatchMouseEvent(connection, sessionId, { type: "mouseMoved", x, y })) {
    return true;
  }
  assertNotCancelled(signal);
  // Hover handlers can change a SPA URL without replacing the loader/node.
  // Revalidate after mouse movement, immediately before the button press.
  await assertTarget();
  assertNotCancelled(signal);
  if (await dispatchMouseEvent(connection, sessionId, {
    type: "mousePressed",
    x,
    y,
    button: "left",
    clickCount: 1,
  })) {
    return true;
  }
  const dialogOpened = await dispatchMouseEvent(connection, sessionId, {
    type: "mouseReleased",
    x,
    y,
    button: "left",
    clickCount: 1,
  });
  // Once pressed, finish the release even if cancellation arrives. Reporting
  // cancellation before this point would hand over a stuck mouse button.
  assertNotCancelled(signal);
  return dialogOpened;
}

async function dispatchMouseEvent(connection, sessionId, params) {
  const dialogWaiter = typeof connection.waitForEvent === "function"
    ? connection.waitForEvent("Page.javascriptDialogOpening", DIALOG_OPEN_WAIT_MS, sessionId)
    : null;
  const command = connection.send("Input.dispatchMouseEvent", params, sessionId);
  if (!dialogWaiter) {
    await command;
    return false;
  }
  const outcome = await Promise.race([
    command.then(() => ({ kind: "completed" })),
    dialogWaiter.promise.then((event) => ({
      kind: event ? "dialog" : "event_timeout",
    })),
  ]);
  if (outcome.kind === "completed") {
    dialogWaiter.cancel();
    return false;
  }
  if (outcome.kind === "dialog") {
    void command.catch(() => {});
    return true;
  }
  await command;
  return false;
}

async function focusElement(connection, sessionId, objectId, selectAll) {
  const response = await connection.send(
    "Runtime.callFunctionOn",
    {
      objectId,
      functionDeclaration: `function(selectAll) {
        this.focus();
        const activeElement = (this.getRootNode?.() ?? this.ownerDocument).activeElement;
        const focused = activeElement === this || this.contains?.(activeElement);
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
