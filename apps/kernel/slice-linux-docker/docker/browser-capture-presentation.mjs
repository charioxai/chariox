// MP-08/MP-10/MP-11: a dense CDP screenshot temporarily enlarges the native
// window. Admit native pixels only outside capture and compositor restoration.
export class CapturePresentationFence {
  constructor(now = Date.now) { this.now = now; this.intervals = []; this.floor = -Infinity; this.waiters = new Set(); }
  bindOwner(owner, timing) { this.owner = owner; this.timing = timing; }
  stable(start, end = start) {
    return Number.isFinite(start) && Number.isFinite(end) && end >= start && start > this.floor &&
      this.intervals.every(interval => end < interval.start || start > interval.end);
  }
  // MP-08/MP-10/MP-11: wait for restoration, never force another capture
  // while this fence still excludes it. Each caller can cancel on source close.
  whenStable(listener) {
    let cancelled = false, timer;
    const check = () => {
      if (cancelled) return;
      clearTimeout(timer); timer = undefined;
      if (this.intervals.some(interval => !Number.isFinite(interval.end))) return;
      const remaining = Math.max(this.floor, ...this.intervals.map(interval => interval.end)) - this.now() + 1;
      if (remaining > 0) { timer = setTimeout(check, remaining); return; }
      cancelled = true; this.waiters.delete(check); listener();
    };
    this.waiters.add(check); queueMicrotask(check);
    return () => { cancelled = true; clearTimeout(timer); this.waiters.delete(check); };
  }
  restored(interval) {
    interval.end = this.now() + 1; delete interval.restore;
    while (this.intervals.length > 64 && Number.isFinite(this.intervals[0].end))
      this.floor = Math.max(this.floor, this.intervals.shift().end);
    for (const check of [...this.waiters]) check();
  }
  // MP-08/MP-10/MP-11: a later start may repeat the SAME restoration proof.
  // Failure never opens the interval or admits its old pixels.
  recoverFailed() {
    if (this.recovering) return this.recovering;
    const failed = this.intervals.filter(interval => interval.restore);
    if (!failed.length) return Promise.resolve();
    this.recovering = (async () => {
      for (const interval of failed) { await interval.restore(); this.restored(interval); }
    })().finally(() => { this.recovering = null; });
    return this.recovering;
  }
  async run(capture, restore) {
    const interval = { start: this.now(), end: Infinity }; this.intervals.push(interval);
    try { return await capture(); }
    finally {
      try { await restore(); }
      catch (error) { interval.restore = restore; throw error; }
      this.restored(interval);
    }
  }
}

// The existing CDP owner is the authority; no separate session/document cache.
async function presentationWorld(owner, connection, sessionId) {
  const target = owner?.targetsBySession?.get(sessionId), frame = target && owner.currentPageFrame?.(target);
  if (owner?.connection !== connection || !frame || frame.connection !== connection || frame.sessionId !== sessionId ||
      typeof frame.frame.loaderId !== 'string' || !frame.frame.loaderId) return null;
  let world = owner.focusWorldsByTarget?.get(target);
  const validWorld = () => world?.documentId === frame.frame.loaderId && world.connection === connection &&
    world.sessionId === sessionId && world.frameId === frame.frame.id && Number.isSafeInteger(world.contextId) && world.contextId > 0;
  const cached = validWorld();
  if (!cached && typeof owner.ensureFocusWorld === 'function') {
    world = await owner.ensureFocusWorld(connection, sessionId, target, frame.frame);
    if (owner.connection !== connection || owner.currentPageFrame(target) !== frame) throw Error('MP-11: screenshot presentation owner retired');
  }
  if (!validWorld()) return null;
  const contextId = world.contextId;
  return { contextId, cached, current: () => owner.connection === connection && owner.currentPageFrame(target) === frame &&
    owner.focusWorldsByTarget.get(target) === world && validWorld() && world.contextId === contextId };
}

export function fenceScreenshotPresentation(connection) {
  const send = connection.send.bind(connection), fence = new CapturePresentationFence();
  connection.presentationFence = fence;
  connection.send = (method, params, sessionId, options) => method !== 'Page.captureScreenshot' ? send(method, params, sessionId, options) :
    fence.run(() => send(method, params, sessionId, options), async () => {
      const at = performance.timeOrigin + performance.now();
      try {
        const proof = await presentationWorld(fence.owner, connection, sessionId);
        fence.timing?.event?.('capture_presentation_owner', { warm: Number(Boolean(proof)), world_cached: Number(Boolean(proof?.cached)) });
        let contextId = proof?.contextId;
        if (!proof) {
          const { frameTree } = await send('Page.getFrameTree', {}, sessionId);
          const { executionContextId } = await send('Page.createIsolatedWorld', {
            frameId: frameTree.frame.id, worldName: 'chariox-capture-presentation',
          }, sessionId);
          contextId = executionContextId;
        }
        const { result } = await send('Runtime.evaluate', { contextId, awaitPromise: true, returnByValue: true,
          expression: 'new Promise(resolve => { setTimeout(() => resolve(false), 1000); requestAnimationFrame(() => requestAnimationFrame(() => resolve(true))); })',
        }, sessionId);
        if (proof && !proof.current()) throw Error('MP-11: screenshot presentation owner retired');
        if (result?.value !== true) throw Error('MP-08/MP-10: screenshot presentation did not restore');
      } finally { fence.timing?.('capture_presentation_restore', at); }
    });
}

// One deferred retry per source, shared by Browser and Computer capture.
export function resumeAfterCapturePresentation(source, fence, wake) {
  if (source.presentationResume) return;
  source.presentationResume = fence.whenStable?.(() => {
    source.presentationResume = null;
    if (!source.closed) void Promise.resolve().then(wake).catch(() => source.close()).catch(() => {});
  });
}
