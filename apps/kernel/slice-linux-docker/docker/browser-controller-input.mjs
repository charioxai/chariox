// MP-08/MP-10: hidden RenderWidgets cannot acknowledge CDP mouse input until
// Chromium produces visual state. A target-local capture keeps that renderer
// active for physical input without Page.bringToFront or OS-window activation.
export class BrowserInputCapture {
  constructor() { this.visibilityBySession = new Map(); this.displayLeases = new Map(); }

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

  // MD-DISPLAY-02: keep a hidden RenderWidget alive while a display is attached.
  // Physical visibility remains observational; this never activates an OS tab.
  async hold(connection, sessionId) {
    let lease=this.displayLeases.get(sessionId);
    if(lease?.closing){await lease.closed;return this.hold(connection,sessionId)}
    if(!lease){
      const ready=Promise.withResolvers(),release=Promise.withResolvers();
      lease={refs:0,active:0,ready:ready.promise,release:release.resolve,closing:false,waiters:[]};
      this.displayLeases.set(sessionId,lease);
      const settle=()=>{if(this.displayLeases.get(sessionId)===lease)this.displayLeases.delete(sessionId)};
      lease.closed=this.runTransient(connection,sessionId,async()=>{
        ready.resolve();await release.promise;
        if(lease.active)await new Promise(resolve=>lease.waiters.push(resolve));
      }).then(settle,error=>{settle();ready.reject(error);throw error});
      lease.closed.catch(()=>{});
    }
    lease.refs++;await lease.ready;let released=false;
    return async()=>{if(released)return;released=true;if(--lease.refs)return;lease.closing=true;lease.release();await lease.closed};
  }
  async run(connection,sessionId,operation){
    const lease=this.displayLeases.get(sessionId);
    if(!lease)return this.runTransient(connection,sessionId,operation);
    if(lease.closing){await lease.closed;return this.run(connection,sessionId,operation)}
    await lease.ready;
    if(lease.closing){await lease.closed;return this.run(connection,sessionId,operation)}
    lease.active++;
    try{return await operation()}
    finally{if(!--lease.active)for(const wake of lease.waiters.splice(0))wake()}
  }
  async runTransient(connection, sessionId, operation) {
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
