// VAUDIT S3: read a secret a page shows (a revealed API key, a webhook signing
// secret, a value a copy button puts on the clipboard) for the kernel's Vault.
// The value goes back to the kernel only; it never appears in an error.
import { randomUUID } from "node:crypto";
import { assertCurrentDocument, assertNotCancelled, BrowserActionError, resolveBackendNode } from "./browser-controller-actions.mjs";

const VALUE_LIMIT = 16 * 1024;
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
  if (read !== "node" && read !== "clipboard") throw invalid("capture read must be node or clipboard");
  if (read === "clipboard" && revealNodeRef !== null) {
    throw invalid("clipboard capture clicks node_ref (the copy button) itself; omit reveal_node_ref");
  }
  if (typeof expectedDocumentUrl !== "string" || !expectedDocumentUrl) throw invalid("capture requires the observed document URL");
  await assertCurrentDocument(connection, sessionId, targetId, documentId);
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
