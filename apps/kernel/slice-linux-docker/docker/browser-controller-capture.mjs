// VAUDIT S3: read a secret a page shows (a revealed API key, a webhook signing
// secret, a value a copy button puts on the clipboard) for the kernel's Vault.
// The value goes back to the kernel only; it never appears in an error.
import { randomUUID } from "node:crypto";
import { assertCurrentDocument, assertNotCancelled, BrowserActionError, resolveBackendNode } from "./browser-controller-actions.mjs";

const VALUE_LIMIT = 16 * 1024;
// A QR code is read from a picture of its element: the kernel decodes it and
// the picture of a page element is not a secret.
const QR_IMAGE_LIMIT = 4 * 1024 * 1024;
const QR_MIN_SIDE_PX = 400;
const QR_MAX_SCALE = 4;
const POLL_INTERVAL_MS = 100;

export async function captureBrowserValue({
  connection,
  sessionId,
  targetId,
  documentId,
  nodeRef,
  revealNodeRef = null,
  read = "node",
  expectedDocumentUrl,
  click,
  timeoutMs = 10_000,
  signal,
  now = Date.now,
  sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
}) {
  const backendNodeId = backendNode(nodeRef);
  if (revealNodeRef !== null) backendNode(revealNodeRef);
  if (read !== "node" && read !== "clipboard" && read !== "qr") throw invalid("capture read must be node, clipboard or qr");
  if (read === "qr" && revealNodeRef !== null) throw invalid("qr capture reads node_ref (the QR image) as shown; omit reveal_node_ref");
  if (read === "clipboard" && revealNodeRef !== null) {
    throw invalid("clipboard capture clicks node_ref (the copy button) itself; omit reveal_node_ref");
  }
  if (typeof expectedDocumentUrl !== "string" || !expectedDocumentUrl) throw invalid("capture requires the observed document URL");
  await assertCurrentDocument(connection, sessionId, targetId, documentId);
  if (read === "qr") return captureQrImage(connection, sessionId, backendNodeId);
  const poll = async (sample) => {
    const startedAt = now();
    while (true) {
      assertNotCancelled(signal);
      const value = await sample();
      if (value) {
        if (value.length > VALUE_LIMIT) throw new BrowserActionError("browser_capture_too_long", "the captured value is longer than 16 KiB");
        return value;
      }
      if (now() - startedAt >= timeoutMs) {
        throw new BrowserActionError(
          "browser_capture_empty",
          read === "clipboard"
            ? "the copy button put no text on the clipboard; point at the button that copies the secret"
            : "the element showed no new value; pass the reveal button as reveal_node_ref or point at the element that holds the value",
        );
      }
      await sleep(POLL_INTERVAL_MS);
    }
  };
  if (read === "clipboard") {
    const slot = `__charioxCapture${randomUUID().replaceAll("-", "")}`;
    await callOnNode(connection, sessionId, backendNodeId, false, INSTALL_CLIPBOARD_RECORDER, [slot, expectedDocumentUrl]);
    try {
      await click(nodeRef);
      return await poll(() => callOnNode(connection, sessionId, backendNodeId, false, `function(slot) { return this.ownerDocument?.defaultView?.[slot]?.text ?? null; }`, [slot]));
    } finally {
      await callOnNode(connection, sessionId, backendNodeId, false, `function(slot) { this.ownerDocument?.defaultView?.[slot]?.restore(); return true; }`, [slot]).catch(() => {});
    }
  }
  const shown = () => callOnNode(connection, sessionId, backendNodeId, true, READ_NODE_VALUE, [expectedDocumentUrl]);
  const before = revealNodeRef === null ? null : await shown();
  if (revealNodeRef !== null) await click(revealNodeRef);
  return poll(async () => {
    const value = await shown();
    return value && value !== before ? value : null;
  });
}

/** Returns the element as a base64 PNG, scaled up so a small code keeps its modules legible. */
async function captureQrImage(connection, sessionId, backendNodeId) {
  await connection.send("DOM.scrollIntoViewIfNeeded", { backendNodeId }, sessionId).catch(() => {});
  const box = await connection.send("DOM.getBoxModel", { backendNodeId }, sessionId).catch(() => null);
  const quad = box?.model?.border;
  if (!Array.isArray(quad) || quad.length !== 8) throw invalid("the element has no visible box; point node_ref at the QR image");
  const xs = [0, 2, 4, 6].map((i) => quad[i]), ys = [1, 3, 5, 7].map((i) => quad[i]);
  const width = Math.max(...xs) - Math.min(...xs), height = Math.max(...ys) - Math.min(...ys);
  if (!(width >= 1 && height >= 1)) throw invalid("the element has no visible box; point node_ref at the QR image");
  const { cssVisualViewport } = await connection.send("Page.getLayoutMetrics", {}, sessionId);
  const scale = Math.min(QR_MAX_SCALE, Math.max(1, Math.ceil(QR_MIN_SIDE_PX / Math.min(width, height))));
  const shot = await connection.send("Page.captureScreenshot", {
    format: "png", fromSurface: true, captureBeyondViewport: false,
    clip: { x: Math.min(...xs) + cssVisualViewport.pageX, y: Math.min(...ys) + cssVisualViewport.pageY, width, height, scale },
  }, sessionId);
  if (typeof shot?.data !== "string" || shot.data.length > QR_IMAGE_LIMIT * 1.4) {
    throw new BrowserActionError("browser_capture_failed", "the QR image could not be captured");
  }
  return shot.data;
}

// Isolated world: page script cannot observe or change what the kernel reads.
const READ_NODE_VALUE = `function(expectedDocumentUrl) {
  const view = this.ownerDocument?.defaultView;
  if (!view || view.location.href !== expectedDocumentUrl) return { changed: true };
  const field = this.localName === "input" || this.localName === "textarea";
  return { value: field ? String(this.value ?? "") : String(this.innerText ?? this.textContent ?? "").trim() };
}`;

// Main world: the page's own copy handler calls navigator.clipboard.writeText
// or fills the copy event. Record the first text; the page still sees success.
const INSTALL_CLIPBOARD_RECORDER = `function(slot, expectedDocumentUrl) {
  const view = this.ownerDocument?.defaultView;
  if (!view || view.location.href !== expectedDocumentUrl) return { changed: true };
  const clipboard = view.navigator.clipboard;
  const original = clipboard?.writeText;
  const own = !!clipboard && Object.prototype.hasOwnProperty.call(clipboard, "writeText");
  const state = { text: null };
  const record = (text) => { if (state.text === null && typeof text === "string" && text.trim()) state.text = text.trim(); };
  if (typeof original === "function") {
    // The system clipboard may refuse an unfocused page; the copy itself succeeded for the kernel.
    clipboard.writeText = function(text) { record(String(text)); original.call(clipboard, text).catch(() => undefined); return Promise.resolve(); };
  }
  const onCopy = (event) => {
    const active = view.document.activeElement;
    const field = active && typeof active.selectionStart === "number" ? active.value.slice(active.selectionStart, active.selectionEnd) : "";
    record(event.clipboardData?.getData("text/plain") || field || String(view.getSelection?.() ?? ""));
  };
  view.addEventListener("copy", onCopy);
  Object.defineProperty(view, slot, { configurable: true, value: {
    get text() { return state.text; },
    restore() {
      if (typeof original === "function") { if (own) clipboard.writeText = original; else delete clipboard.writeText; }
      view.removeEventListener("copy", onCopy);
      delete view[slot];
    },
  } });
  return { value: true };
}`;

async function callOnNode(connection, sessionId, backendNodeId, isolated, functionDeclaration, args) {
  const objectId = await resolveBackendNode(connection, sessionId, backendNodeId, isolated);
  try {
    const response = await connection.send("Runtime.callFunctionOn", {
      objectId,
      functionDeclaration,
      arguments: args.map((value) => ({ value })),
      returnByValue: true,
      awaitPromise: false,
    }, sessionId);
    // Page exception text can quote the secret: report a fixed reason only.
    if (response?.exceptionDetails) throw new BrowserActionError("browser_capture_failed", "the element could not be read");
    const result = response?.result?.value;
    if (result?.changed === true) throw new BrowserActionError("browser_secret_target_changed", "the page changed before capture; observe it again");
    return result && typeof result === "object" && "value" in result ? result.value : result;
  } finally {
    await connection.send("Runtime.releaseObject", { objectId }, sessionId).catch(() => {});
  }
}

function backendNode(nodeRef) {
  if (typeof nodeRef !== "string" || !/^backend:[1-9][0-9]*$/.test(nodeRef)) {
    throw invalid("capture requires a top-level page node_ref from a fresh snapshot");
  }
  return Number(nodeRef.slice("backend:".length));
}

function invalid(message) {
  return new BrowserActionError("browser_action_invalid", message);
}
