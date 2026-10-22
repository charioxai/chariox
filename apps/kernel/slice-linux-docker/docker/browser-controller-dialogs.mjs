// Prompt defaults stay private to the controller and live only as long as the
// open dialog. They must never enter the event journal or a response payload.
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

  delete(targetId) { this.byTarget.delete(targetId); }
  clear() { this.byTarget.clear(); }
}

const DIALOG_TYPES = new Set(["alert", "confirm", "prompt", "beforeunload"]);
const DIALOG_MESSAGE_BYTES = 2048;

// The page dialog a Tab has open right now, for the person looking at it (protocol 568). A page that opened one
// answers nothing until it is handled, so the viewer shows it and answers through the `dialog` command. Prompt
// defaults stay private (see BrowserDialogDefaults): the viewer answers `accept` and the controller applies it.
export class BrowserOpenDialogs {
  constructor(onChange = () => {}) {
    this.byTarget = new Map();
    this.onChange = onChange;
  }

  observe(message, targetId) {
    if (!targetId) return;
    if (message.method === "Page.javascriptDialogOpening") {
      const type = message.params?.type;
      if (!DIALOG_TYPES.has(type)) return;
      const text = typeof message.params.message === "string" ? message.params.message : "";
      this.byTarget.set(targetId, { type, message: boundedUtf8(text, DIALOG_MESSAGE_BYTES) });
      this.onChange();
    } else if (
      message.method === "Page.javascriptDialogClosed" ||
      message.method === "Target.targetDestroyed" ||
      message.method === "Target.targetCrashed" ||
      message.method === "Inspector.targetCrashed" ||
      (message.method === "Page.frameNavigated" && !message.params?.frame?.parentId)
    ) {
      this.delete(targetId);
    }
  }

  get(targetId) { return this.byTarget.get(targetId) ?? null; }

  get size() { return this.byTarget.size; }

  delete(targetId) { if (this.byTarget.delete(targetId)) this.onChange(); }

  clear() { if (this.byTarget.size) { this.byTarget.clear(); this.onChange(); } }
}

function boundedUtf8(text, maxBytes) {
  let bytes = 0, end = 0;
  for (const char of text) {
    bytes += Buffer.byteLength(char, "utf8");
    if (bytes > maxBytes) break;
    end += char.length;
  }
  return text.slice(0, end);
}
