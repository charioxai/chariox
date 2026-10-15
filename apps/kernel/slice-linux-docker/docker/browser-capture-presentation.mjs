// MP-08/MP-10/MP-11: a dense CDP screenshot temporarily enlarges the native
// window. Admit native pixels only outside capture and compositor restoration.
export class CapturePresentationFence {
  constructor(now = Date.now) { this.now = now; this.intervals = []; this.floor = -Infinity; }
  stable(start, end = start) {
    return Number.isFinite(start) && Number.isFinite(end) && end >= start && start > this.floor &&
      this.intervals.every(interval => end < interval.start || start > interval.end);
  }
  async run(capture, restore) {
    const interval = { start: this.now(), end: Infinity }; this.intervals.push(interval);
    try { return await capture(); }
    finally {
      // A failed restoration remains fenced until this connection is replaced.
      await restore(); interval.end = this.now() + 1;
      while (this.intervals.length > 64 && Number.isFinite(this.intervals[0].end))
        this.floor = Math.max(this.floor, this.intervals.shift().end);
    }
  }
}

export function fenceScreenshotPresentation(connection) {
  const send = connection.send.bind(connection), fence = new CapturePresentationFence();
  connection.presentationFence = fence;
  connection.send = (method, params, sessionId) => method !== 'Page.captureScreenshot' ? send(method, params, sessionId) :
    fence.run(() => send(method, params, sessionId), async () => {
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
