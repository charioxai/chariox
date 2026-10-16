// MP-08/MP-10 (protocol 541): a parked state read wakes when something a state read could report may have
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
// CDP events after which a state read can differ: Tabs opening, closing, navigating or retitling, loads.
const STATE_EVENTS = /^(Target\.(targetCreated|targetDestroyed|targetInfoChanged|detachedFromTarget)|Page\.(frameNavigated|navigatedWithinDocument|frameStartedLoading|frameStoppedLoading|loadEventFired)|Inspector\.detached|Chariox\.browserDisconnected)$/;
export const stateChangingEvent = message => STATE_EVENTS.test(message?.method ?? '');
