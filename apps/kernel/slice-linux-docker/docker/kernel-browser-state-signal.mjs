// MP-08/MP-10 (protocol 556): a parked state read wakes when something a state read could report may have
// changed. Events only say "read again"; the kernel compares the read's content, so an extra wake costs one
// read and a missed one is bounded by the wait.
export class StateSignal {
  constructor() { this.count = 0; this.waiters = new Set(); }
  notify() { this.count++; for (const wake of [...this.waiters]) wake(); }
  /** Resolves with the count once it differs from `after`, `ms` elapse or `signal` aborts. */
  wait(after, ms, signal) {
    if (after === null || this.count !== after || !(ms > 0) || signal?.aborted) return Promise.resolve(this.count);
    return new Promise(resolve => {
      const done = () => { clearTimeout(timer); signal?.removeEventListener('abort', done); this.waiters.delete(done); resolve(this.count); };
      const timer = setTimeout(done, ms);
      this.waiters.add(done);
      signal?.addEventListener('abort', done, { once: true });
    });
  }
}
// CDP events after which a state read can differ: a page Tab opening, closing, navigating or retitling.
// Sub-frame navigations and other targets (workers, iframes) change nothing a state read reports.
// Row 655: the controller's own short attachments to a frame's renderer (a card following an element inside a
// challenge frame reads its geometry every 100 ms) end with a detach the controller asked for: that is no Tab change.
export function expectDetach(connection, sessionId) {
  if (!connection || typeof sessionId !== 'string') return;
  const expected = connection.expectedDetaches ??= new Set();
  // A detach whose event never came (the browser went away) is forgotten, never kept without bound.
  if (expected.size >= 1_024) expected.clear();
  expected.add(sessionId);
}
export function stateChangingEvent(message, connection) {
  const method = message?.method ?? '', params = message?.params;
  if (method === 'Target.detachedFromTarget' && typeof params?.sessionId === 'string' && connection?.expectedDetaches?.delete(params.sessionId)) return false;
  if (method === 'Target.targetCreated' || method === 'Target.targetInfoChanged') return params?.targetInfo?.type === 'page';
  if (method === 'Page.frameNavigated') return !params?.frame?.parentId;
  return ['Page.javascriptDialogOpening', 'Page.javascriptDialogClosed', 'Target.targetDestroyed', 'Target.detachedFromTarget', 'Target.targetCrashed', 'Inspector.targetCrashed', 'Inspector.detached', 'Chariox.browserDisconnected'].includes(method);
}
