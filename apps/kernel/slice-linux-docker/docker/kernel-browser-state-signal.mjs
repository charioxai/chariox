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
export function stateChangingEvent(message) {
  const method = message?.method ?? '', params = message?.params;
  if (method === 'Target.targetCreated' || method === 'Target.targetInfoChanged') return params?.targetInfo?.type === 'page';
  if (method === 'Page.frameNavigated') return !params?.frame?.parentId;
  return ['Page.javascriptDialogOpening', 'Page.javascriptDialogClosed', 'Target.targetDestroyed', 'Target.detachedFromTarget', 'Inspector.detached', 'Chariox.browserDisconnected'].includes(method);
}
