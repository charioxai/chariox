import { randomUUID } from "node:crypto";

// Prompt defaults live only with the open dialog. The shared prompt shows its
// bounded initial reply; answer results and the event journal never echo it.
export class BrowserDialogDefaults {
  constructor() {
    this.byTarget = new Map();
    this.dialogs = new Map();
    this.answering = new Set();
  }

  observe(message, targetId, documentId) {
    if (!targetId) return;
    if (message.method === "Page.javascriptDialogOpening") {
      this.byTarget.delete(targetId);
      const { type, message: text, url } = message.params ?? {};
      if (!["alert", "confirm", "prompt", "beforeunload"].includes(type)) return;
      let origin = "null";
      try { origin = new URL(url).origin; } catch { /* CDP may omit the initiating URL. */ }
      this.dialogs.set(targetId, { dialog_id: randomUUID(), target_id: targetId,
        document_id: documentId, kind: type, message: typeof text === "string" ? text.slice(0, 8192) : "", origin });
      if (message.params?.type === "prompt") {
        const text = message.params.defaultPrompt ?? "";
        const bounded = typeof text === "string" && text.length <= 2048 && Buffer.byteLength(text, "utf8") <= 2048;
        this.byTarget.set(targetId, { documentId, text: bounded ? text : null });
        if (bounded) this.dialogs.get(targetId).default_prompt = text;
      }
    } else if (
      message.method === "Page.javascriptDialogClosed" ||
      message.method === "Target.targetDestroyed" ||
      message.method === "Target.targetCrashed" ||
      message.method === "Inspector.targetCrashed" ||
      (message.method === "Page.frameNavigated" && !message.params?.frame?.parentId)
    ) {
      this.byTarget.delete(targetId);
      this.dialogs.delete(targetId);
    }
  }

  get(targetId, documentId) {
    const entry = this.byTarget.get(targetId);
    return entry?.documentId === documentId ? entry.text : undefined;
  }

  pending() { return [...this.dialogs.values()].map(dialog => ({ ...dialog })); }
  current(targetId, dialogId, documentId) {
    const dialog = this.dialogs.get(targetId);
    return dialog?.dialog_id === dialogId && dialog.document_id === documentId;
  }
  claim(targetId, dialogId, documentId) {
    if (!this.current(targetId, dialogId, documentId) || this.answering.has(dialogId)) return false;
    this.answering.add(dialogId);
    return true;
  }
  release(dialogId) { this.answering.delete(dialogId); }
  delete(targetId) { this.byTarget.delete(targetId); this.dialogs.delete(targetId); }
  clear() { this.byTarget.clear(); this.dialogs.clear(); this.answering.clear(); }
}
