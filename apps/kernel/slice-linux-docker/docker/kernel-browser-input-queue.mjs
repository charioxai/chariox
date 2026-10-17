// MP-08/MP-10/MP-11: a person's input while the page's renderer is busy (a 16 MB parse) cannot pass the
// renderer-bound fences (document, focus, sensitive-field reads). A fence that timed out before anything was
// dispatched reached no page, so the input waits here in order and is admitted through the same fences once the
// renderer answers. Input that may have been dispatched is never parked, so nothing is typed twice.
const LIMIT = 32, TTL_MS = 60_000;

export const markDispatched = error => { if (error && typeof error === 'object') error.afterDispatch = true; return error; };
// Only the fences' own read calls: an Input.* timeout may have reached the page.
const READS = /^browser CDP method (Runtime\.evaluate|Page\.getFrameTree|Page\.createIsolatedWorld) timed out/;
const busy = error => error?.code === 'browser_cdp_timeout' && error.afterDispatch !== true && READS.test(error.message);

export class BusyInputQueue {
  constructor(run, reply, { retryMs = 500, limit = LIMIT, ttlMs = TTL_MS, event = () => {} } = {}) {
    Object.assign(this, { run, reply, retryMs, limit, ttlMs, event }); this.queues = new Map(); this.stopped = false;
  }

  async submit(key, command, options) {
    const queue = this.queues.get(key);
    if (queue) return this.park(queue, command, options);
    try { return await this.run(command, options); }
    catch (error) {
      if (this.stopped || !busy(error)) throw error;
      const created = { key, entries: [], draining: false };
      this.queues.set(key, created);
      try { return this.park(created, command, options, error); } catch (refused) { this.queues.delete(key); throw refused; }
    }
  }

  park(queue, command, options, cause) {
    if (queue.entries.length >= this.limit) throw cause ?? Object.assign(Error('MP-08: busy page input queue full'), { code: 'browser_cdp_timeout' });
    queue.entries.push({ command, options: { ...options, signal: undefined }, expires: Date.now() + this.ttlMs });
    this.event('input_parked');
    if (!queue.draining) { queue.draining = true; void this.drain(queue); }
    return this.reply(command, options);
  }

  async drain(queue) {
    while (!this.stopped && queue.entries.length) {
      const entry = queue.entries[0];
      if (Date.now() > entry.expires) { queue.entries.shift(); this.event('input_expired'); continue; }
      try { await this.run(entry.command, entry.options); queue.entries.shift(); this.event('input_admitted'); }
      catch (error) {
        if (busy(error)) { await new Promise(resolve => setTimeout(resolve, this.retryMs)); continue; }
        queue.entries.shift(); this.event('input_dropped');
      }
    }
    this.queues.delete(queue.key);
  }

  stop() { this.stopped = true; this.queues.clear(); }
}
