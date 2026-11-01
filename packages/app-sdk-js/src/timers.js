// The pinned QuickJS cancel() returns true only for the originating export's
// live import token. A concurrent invocation cannot cancel another one's timer.
export function createTimers(clock, reportError) {
  const timers = new Map(); let next = 0;
  return {
    setTimeout(callback, delay = 0, ...args) {
      if (typeof callback !== 'function') throw new TypeError('timer callback must be a function');
      const id = ++next;
      const pending = clock.waitFor(BigInt(Math.trunc(Math.min(2147483647, Math.max(0, Number(delay) || 0)))) * 1000000n);
      const timer = { pending, active: true };
      timer.done = pending.then(() => {
        timers.delete(id);
        if (timer.active) callback(...args);
      }, () => { timers.delete(id); });
      timer.done.catch(reportError);
      timers.set(id, timer);
      return id;
    },
    clearTimeout(id) {
      const timer = timers.get(id);
      if (!timer) return;
      timer.active = false;
      timer.pending.cancel?.();
    },
    async finishInvocation() {
      const closed = [];
      for (const timer of timers.values()) {
        if (timer.pending.cancel?.() === true) {
          timer.active = false;
          closed.push(timer.done);
        }
      }
      await Promise.all(closed);
    },
  };
}
