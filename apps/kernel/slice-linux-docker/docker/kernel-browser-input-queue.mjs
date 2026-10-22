// MP-08/MP-10/MP-11: a person's input while the page's renderer is busy (a 16 MB parse) cannot pass the
// renderer-bound fences (document, focus, sensitive-field reads). A fence that timed out before anything was
// dispatched reached no page, so the input waits here in order and is admitted through the same fences once the
// renderer answers. Input that may have been dispatched is never parked, so nothing is typed twice.
const LIMIT = 32, TTL_MS = 60_000, STALE_MS = 3_000;

export const markDispatched = error => { if (error && typeof error === 'object') error.afterDispatch = true; return error; };
// Any CDP read the fences make; an Input.* call that timed out may have reached the page.
// A CDP input call whose acknowledgement timed out: the press and its release were sent in order, so nothing is stuck.
export const lateInputAck = error => error?.code === 'browser_cdp_timeout' && /^browser CDP method Input\./.test(error.message);
const busy = error => error?.code === 'browser_cdp_timeout' && error.afterDispatch !== true && !lateInputAck(error);

// A reference refused as stale while the person's tab and document are unchanged is a binding that moved under a
// busy renderer (a first paint that changed the page geometry): the same fences admit it again once the window rebinds.
const stale = error => error?.code === 'user_domain_stale_reference';

const typed = command => command.input?.kind === 'text' || command.input?.kind === 'key';
const sameScroll = (a, b) => a.input?.kind === 'scroll' && b.input?.kind === 'scroll' && a.tab_id === b.tab_id && a.document_id === b.document_id
  && a.binding_id === b.binding_id && a.input.x === b.input.x && a.input.y === b.input.y;

export class BusyInputQueue {
  // `lane` names the surface an input acts on (the page, or Chrome's own band): each observer has one queue per lane, so an
  // input that waits for the page's renderer never holds back the band's keys, which do not depend on it.
  constructor(run, reply, { retryMs = 500, limit = LIMIT, ttlMs = TTL_MS, staleMs = STALE_MS, sameDocument = () => false, event = () => {}, lane = () => '' } = {}) {
    Object.assign(this, { run, reply, retryMs, limit, ttlMs, staleMs, sameDocument, event, lane }); this.queues = new Map(); this.attempts = new Set(); this.stopped = false;
  }

  /** How many inputs of `key` (an observer) wait, across its lanes. */
  parked(key) { let count = 0; for (const queue of this.queues.values()) if (queue.key === key) count += queue.entries.length; return count; }

  async submit(key, command, options) {
    const slot = `${key}\0${this.lane(command)}`, queue = this.queues.get(slot);
    if (queue) return this.park(queue, command, options);
    // The first attempt is in no queue yet: a retirement that lands while its fence waits aborts it here and keeps it from parking.
    const attempt = { key, command, abort: new AbortController() };
    this.attempts.add(attempt);
    const signal = options?.signal ? AbortSignal.any([options.signal, attempt.abort.signal]) : attempt.abort.signal;
    try { return await this.run(command, { ...options, signal }); }
    catch (error) {
      if (this.stopped || signal.aborted || !busy(error)) throw error;
      const created = { key, slot, entries: [], draining: false };
      this.queues.set(slot, created);
      try { return this.park(created, command, options, error); } catch (refused) { this.queues.delete(slot); throw refused; }
    } finally { this.attempts.delete(attempt); }
  }

  park(queue, command, options, cause) {
    const last = queue.entries.at(-1);
    if (last && last !== queue.active && sameScroll(last.command, command)) { // a wheel burst replays as one scroll, not as a late jump of many
      last.command = { ...last.command, input: { ...last.command.input, delta_x: (last.command.input.delta_x ?? 0) + (command.input.delta_x ?? 0), delta_y: (last.command.input.delta_y ?? 0) + (command.input.delta_y ?? 0) } };
      return this.reply(command, options);
    }
    if (queue.entries.length >= this.limit) throw cause ?? Object.assign(Error('MP-08: busy page input queue full'), { code: 'browser_cdp_timeout' });
    const abort = new AbortController();
    queue.entries.push({ command, options: { ...options, signal: abort.signal }, abort, expires: Date.now() + this.ttlMs });
    this.event('input_parked', cause);
    if (!queue.draining) { queue.draining = true; void this.drain(queue); }
    return this.reply(command, options);
  }

  async drain(queue) {
    await new Promise(resolve => setTimeout(resolve, this.retryMs)); // the fence just failed: let a burst join before the first retry
    while (!this.stopped && queue.entries.length) {
      const entry = queue.entries[0];
      if (Date.now() > entry.expires) { queue.entries.shift(); this.event('input_expired'); continue; }
      queue.active = entry;
      try { await this.run(entry.command, entry.options); queue.active = null; if (queue.entries[0] === entry) queue.entries.shift(); this.event('input_admitted'); }
      catch (error) {
        queue.active = null;
        if (entry.abort.signal.aborted) continue;
        if (busy(error) || stale(error) && this.sameDocument(entry.command) && Date.now() < (entry.staleUntil ??= Date.now() + this.staleMs)) { await new Promise(resolve => setTimeout(resolve, this.retryMs)); continue; }
        queue.entries.shift(); this.event(`input_dropped ${/^[a-z_]{1,48}$/.test(error?.code) ? error.code : 'error'}`); // the cause, as a fixed code, for the display timing
        // Typing that lost a key is dropped whole: the rest would arrive as a mangled word (a URL typed without its first letter).
        if (typed(entry.command)) while (queue.entries.length && typed(queue.entries[0].command)) { queue.entries.shift(); this.event('input_dropped'); }
      }
    }
    this.queues.delete(queue.slot);
  }

  // The parking reply ended the kernel's action for this input, so its authority ends here: the terminal that parked it
  // disconnected, or another actor took over its tab. A request already being admitted is cancelled at its next fence.
  retire(drop) {
    for (const attempt of this.attempts) if (drop(attempt.key, attempt.command)) attempt.abort.abort();
    for (const queue of this.queues.values()) {
      const kept = [];
      for (const entry of queue.entries) if (drop(queue.key, entry.command)) { entry.abort.abort(); this.event('input_dropped'); } else kept.push(entry);
      queue.entries = kept;
    }
  }

  retireAll() { this.retire(() => true); }

  stop() { this.stopped = true; this.queues.clear(); }
}
