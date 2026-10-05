// Prompt defaults stay private to the controller and live only as long as the
// open dialog. They must never enter the event journal or a response payload.
export class BrowserDialogObservationError extends Error {
  constructor(code, message) { super(message); this.code = code; }
}

export class BrowserDialogDefaults {
  constructor() {
    this.byTarget = new Map();
  }

  observe(message, targetId, documentId) {
    if (!targetId) return;
    if (message.method === "Page.javascriptDialogOpening") {
      this.byTarget.delete(targetId);
      if (message.params?.type === "prompt") {
        const text = message.params.defaultPrompt ?? "";
        const bounded = typeof text === "string" && text.length <= 2048 && Buffer.byteLength(text, "utf8") <= 2048;
        this.byTarget.set(targetId, { documentId, text: bounded ? text : null });
      } else {
        this.byTarget.set(targetId, { documentId });
      }
    } else if (
      message.method === "Page.javascriptDialogClosed" ||
      message.method === "Target.targetDestroyed" ||
      message.method === "Target.targetCrashed" ||
      message.method === "Inspector.targetCrashed" ||
      (message.method === "Page.frameNavigated" && !message.params?.frame?.parentId)
    ) {
      this.byTarget.delete(targetId);
    }
  }

  get(targetId, documentId) {
    const entry = this.byTarget.get(targetId);
    return entry?.documentId === documentId ? entry.text : undefined;
  }

  isOpen(targetId) { return this.byTarget.has(targetId); }

  // MP-08/MP-10/MP-11: a modal pauses renderer reads. Keep only the observed
  // document and tab visibility; never answer it or guess a new binding.
  pageObservation(target, documentId, viewportMatches, focused) {
    const entry = this.byTarget.get(target.targetId);
    if (!entry) return null;
    if (typeof documentId !== "string" || !documentId || entry.documentId !== documentId) {
      throw new BrowserDialogObservationError("browser_dialog_open",
        "open browser dialog has no current observed document binding");
    }
    if (!viewportMatches) {
      throw new BrowserDialogObservationError("browser_dialog_open",
        "resolve the open browser dialog before changing its viewport");
    }
    return { target_id: target.targetId, document_id: documentId,
      url: typeof target.url === "string" ? target.url : "",
      title: typeof target.title === "string" ? target.title : "", focused: focused === true };
  }

  assertSnapshotAllowed(targetId, documentId) {
    const entry = this.byTarget.get(targetId);
    if (!entry) return;
    if (entry.documentId !== documentId) {
      throw new BrowserDialogObservationError("stale_document_reference",
        "browser dialog belongs to a different document; capture a fresh binding");
    }
    throw new BrowserDialogObservationError("browser_dialog_open",
      "browser page observation is paused by an open JavaScript dialog; resolve it through the browser dialog action");
  }

  delete(targetId) { this.byTarget.delete(targetId); }
  clear() { this.byTarget.clear(); }
}
