import { timestamp } from './kernel-browser-timing.mjs';
// MP-08/MP-10: supervise only the browser this host started. A lost CDP pipe
// rejects in-flight actions; recovery restores state and never retries input.
const FIRST_WAIT_MS = 250, MAX_WAIT_MS = 30_000;
// A restart that dies again sooner than this is the same failure, not a new one.
const STABLE_MS = 60_000;

export class BrowserRecovery {
  constructor(host) {
    this.host = host;
    this.enabled = false;
    this.attempts = 0;
    this.notBefore = 0;
  }
  disarm() {
    clearTimeout(this.timer);
    clearTimeout(this.stable);
    this.timer = this.stable = null;
    this.off?.(); this.off = null;
  }
  stop() { this.enabled = false; this.disarm(); }
  watch(connection) {
    this.disarm();
    this.enabled = true;
    this.stable = setTimeout(() => { this.attempts = 0; }, STABLE_MS);
    this.stable.unref?.();
    this.off = connection.subscribe(event => {
      if (event.method === 'Chariox.browserDisconnected') this.schedule();
    });
    if (connection.isOpen?.() === false) this.schedule();
  }
  // Observation (a viewer reading state or mirror) waits out the backoff; an agent action is its own request and starts the browser at once.
  waiting() { return Date.now() < this.notBefore; }
  schedule() {
    if (!this.enabled || this.timer) return;
    clearTimeout(this.stable); this.stable = null;
    this.host.stateSignal.notify();
    this.host.timing('browser_lost', timestamp());
    const wait = Math.min(MAX_WAIT_MS, FIRST_WAIT_MS * 2 ** Math.min(this.attempts++, 7));
    this.notBefore = Date.now() + wait;
    // The browser comes back only when something asks for it. The wake tells watchers (a Room view's state read) that asking is allowed again;
    // an agent action asks at once. With nobody asking, closing a window or a crash never brings the browser back.
    this.timer = setTimeout(() => {
      this.timer = null;
      if (this.enabled) this.host.stateSignal.notify();
    }, wait);
    this.timer.unref?.();
  }
}
