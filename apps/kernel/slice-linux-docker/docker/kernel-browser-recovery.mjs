// MP-08/MP-10: supervise only the browser this host started. A lost CDP pipe
// rejects in-flight actions; recovery restores state and never retries input.
export class BrowserRecovery {
  constructor(host) {
    this.host = host;
    this.enabled = false;
    this.attempts = 0;
  }
  disarm() {
    clearTimeout(this.timer);
    this.timer = null;
    this.off?.(); this.off = null;
  }
  stop() { this.enabled = false; this.disarm(); }
  watch(connection) {
    this.disarm();
    this.enabled = true;
    this.attempts = 0;
    this.off = connection.subscribe(event => {
      if (event.method === 'Chariox.browserDisconnected') this.schedule();
    });
    if (connection.isOpen?.() === false) this.schedule();
  }
  schedule() {
    if (!this.enabled || this.timer) return;
    this.host.stateSignal.notify();
    const wait = Math.min(30_000, 250 * 2 ** Math.min(this.attempts++, 7));
    this.timer = setTimeout(async () => {
      this.timer = null;
      if (!this.enabled) return;
      try { await this.host.start({ allowStart: false }); }
      catch { this.schedule(); }
    }, wait);
    this.timer.unref?.();
  }
}
