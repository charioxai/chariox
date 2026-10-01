// MP-08/MP-10: hidden RenderWidgets cannot acknowledge CDP mouse input until
// Chromium produces visual state. A target-local capture keeps that renderer
// active for physical input without Page.bringToFront or OS-window activation.
export class BrowserInputCapture {
  constructor() { this.visibilityBySession = new Map(); }

  async visibility(connection, sessionId) {
    if (this.visibilityBySession.has(sessionId)) return this.visibilityBySession.get(sessionId).visible;
    const result = await connection.send("Runtime.evaluate", {
      expression: "document.visibilityState === 'visible'",
      returnByValue: true,
      awaitPromise: false,
    }, sessionId);
    // Reconciliation may have begun its evaluation just before input enabled
    // emulation. Preserve the physical visibility captured by that input.
    return this.visibilityBySession.get(sessionId)?.visible ?? result?.result?.value === true;
  }

  async run(connection, sessionId, operation) {
    const visible = await this.visibility(connection, sessionId);
    if (visible) return operation();
    const capture = { visible };
    this.visibilityBySession.set(sessionId, capture);
    let dialogOpened = false;
    const restore = async () => {
      try {
        await connection.send("Emulation.setFocusEmulationEnabled", { enabled: false }, sessionId);
      } finally {
        if (this.visibilityBySession.get(sessionId) === capture) this.visibilityBySession.delete(sessionId);
      }
    };
    try {
      await connection.send("Emulation.setFocusEmulationEnabled", { enabled: true }, sessionId);
      const result = await operation();
      dialogOpened = result?.dialogOpened === true;
      return result;
    } finally {
      // Chromium applies the browser-side capture release immediately, but its
      // renderer reply waits for an open dialog. Return the dialog result so
      // the next same-tab operation can answer it, as with releaseObject.
      if (dialogOpened) void restore().catch(() => {});
      else await restore();
    }
  }
}
