// MP-08/MP-10/MP-11: a dense CDP screenshot temporarily enlarges the native
// window. Admit native pixels only outside capture and compositor restoration.
export class CapturePresentationFence {
  constructor(now = Date.now) { this.now = now; this.intervals = []; this.floor = -Infinity; this.waiters = new Set(); }
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
  async run(capture, restore) {
    const interval = { start: this.now(), end: Infinity }; this.intervals.push(interval);
    try { return await capture(); }
    finally {
      // A failed restoration remains fenced until this connection is replaced.
      await restore(); interval.end = this.now() + 1;
      while (this.intervals.length > 64 && Number.isFinite(this.intervals[0].end))
        this.floor = Math.max(this.floor, this.intervals.shift().end);
      for (const check of [...this.waiters]) check();
    }
  }
}

export function fenceScreenshotPresentation(connection) {
  const send = connection.send.bind(connection), fence = new CapturePresentationFence();
  connection.presentationFence = fence;
  connection.send = (method, params, sessionId, options) => method !== 'Page.captureScreenshot' ? send(method, params, sessionId, options) :
    fence.run(() => send(method, params, sessionId, options), async () => {
      const { frameTree } = await send('Page.getFrameTree', {}, sessionId);
      const { executionContextId } = await send('Page.createIsolatedWorld', {
        frameId: frameTree.frame.id, worldName: 'chariox-capture-presentation',
      }, sessionId);
      const { result } = await send('Runtime.evaluate', { contextId: executionContextId, awaitPromise: true, returnByValue: true,
        expression: 'new Promise(resolve => { setTimeout(() => resolve(false), 1000); requestAnimationFrame(() => requestAnimationFrame(() => resolve(true))); })',
      }, sessionId);
      if (result?.value !== true) throw Error('MP-08/MP-10: screenshot presentation did not restore');
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
