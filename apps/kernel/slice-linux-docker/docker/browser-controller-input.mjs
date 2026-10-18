// MP-08/MP-10: hidden RenderWidgets cannot acknowledge CDP mouse input until
// Chromium produces visual state. A target-local capture keeps that renderer
// active for physical input without Page.bringToFront or OS-window activation.
const WAKE_VISIBILITY_MS = 2000;

export class BrowserInputCapture {
  constructor() { this.visibilityBySession = new Map(); this.displayLeases = new Map(); this.operations = new Map(); }

  ordered(sessionId, operation) {
    const run=(this.operations.get(sessionId)??Promise.resolve()).then(operation);
    const tail=run.catch(()=>{}).finally(()=>{if(this.operations.get(sessionId)===tail)this.operations.delete(sessionId)});
    this.operations.set(sessionId,tail);return run;
  }

  // MP-08/MP-10/MP-11: a display's logical wake-up makes document.hasFocus()
  // true even while Chrome's omnibox owns the keys. Observe the real focus
  // with emulation suspended, excluding same-session input and lease close.
  // Keep a physically blurred page blurred until normal page input resumes it.
  physicalFocus(connection,sessionId,read) {
    return this.ordered(sessionId,async()=>{
      let lease;
      for(;;){
        lease=this.displayLeases.get(sessionId);if(!lease)break;
        if(lease.closing){await lease.closed;continue}
        await lease.ready;if(lease.closing)continue;
        lease.active++;break;
      }
      const capture=this.visibilityBySession.get(sessionId),emulated=capture&&!capture.focusSuspended;
      try {
        if(emulated)await connection.send('Emulation.setFocusEmulationEnabled',{enabled:false},sessionId);
        const focused=await read();
        if(capture&&focused===false)capture.focusSuspended=true;
        return focused;
      } finally {
        try {
          if(emulated&&!capture.focusSuspended&&lease&&!lease.closing&&this.displayLeases.get(sessionId)===lease)
            await connection.send('Emulation.setFocusEmulationEnabled',{enabled:true},sessionId);
        } finally {if(lease&&!--lease.active)for(const wake of lease.waiters.splice(0))wake()}
      }
    });
  }

  // A display hold (`wake`) that gets no answer from a busy renderer assumes the page hidden: the wake-up is then
  // acquired, which is always safe, instead of failing the viewer's attach on a page that is still parsing.
  async visibility(connection, sessionId, wake = false) {
    if (this.visibilityBySession.has(sessionId)) return this.visibilityBySession.get(sessionId).visible;
    let result;
    try {
      result = await connection.send("Runtime.evaluate", {
        expression: "document.visibilityState === 'visible'",
        returnByValue: true,
        awaitPromise: false,
      }, sessionId, wake ? { timeoutMs: WAKE_VISIBILITY_MS } : undefined);
    } catch (error) {
      if (!wake || error?.code !== 'browser_cdp_timeout') throw error;
      return false;
    }
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
      }, true).then(settle,error=>{settle();ready.reject(error);throw error});
      lease.closed.catch(()=>{});
    }
    lease.refs++;await lease.ready;let released=false;
    return async()=>{if(released)return;released=true;if(--lease.refs)return;lease.closing=true;lease.release();await lease.closed};
  }
  run(connection,sessionId,operation,{resumeFocus=true}={}){return this.ordered(sessionId,()=>this.runHeld(connection,sessionId,operation,resumeFocus))}
  async runHeld(connection,sessionId,operation,resumeFocus=true){
    const lease=this.displayLeases.get(sessionId);
    if(!lease)return this.runTransient(connection,sessionId,operation);
    if(lease.closing){await lease.closed;return this.runHeld(connection,sessionId,operation,resumeFocus)}
    await lease.ready;
    if(lease.closing){await lease.closed;return this.runHeld(connection,sessionId,operation,resumeFocus)}
    lease.active++;
    try{
      const capture=this.visibilityBySession.get(sessionId);
      if(resumeFocus&&capture?.focusSuspended){
        await connection.send('Emulation.setFocusEmulationEnabled',{enabled:true},sessionId);
        capture.focusSuspended=false;
      }
      return await operation();
    }
    finally{if(!--lease.active)for(const wake of lease.waiters.splice(0))wake()}
  }
  async runTransient(connection, sessionId, operation, keepAlive = false) {
    const visible = await this.visibility(connection, sessionId, keepAlive);
    // A display can outlive physical foreground visibility. Acquire its
    // renderer wake-up at hold time, before any later background transition.
    if (visible && !keepAlive) return operation();
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
