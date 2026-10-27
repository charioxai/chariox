// MP-08/MP-10: an OOM-killed renderer leaves the browser/CDP pipe alive.
// Reload only that page. Never replay input or replace healthy tabs.
export class BrowserRendererRecovery {
  constructor(browser) { this.browser = browser; this.crashed = new Map(); this.backoff = new Map(); }
  close() {
    for (const entry of this.crashed.values()) clearTimeout(entry.timer);
    this.crashed.clear(); this.backoff.clear();
  }
  forget(targetId) {
    clearTimeout(this.crashed.get(targetId)?.timer);
    this.crashed.delete(targetId); this.backoff.delete(targetId);
  }
  observe(message) {
    const { method, params = {}, sessionId } = message;
    const targetId = this.browser.targetsBySession.get(sessionId) ?? params.targetId;
    if (method === 'Page.frameNavigated' && !params.frame?.parentId) {
      clearTimeout(this.crashed.get(targetId)?.timer);
      this.crashed.delete(targetId);
      return;
    }
    if (!['Inspector.targetCrashed', 'Target.targetCrashed'].includes(method)
        || !this.browser.sessionsByTarget.has(targetId) || !this.browser.documentIdsByTarget.has(targetId)
        || (this.crashed.has(targetId) && !this.crashed.get(targetId).awaitingCommit)) return;
    clearTimeout(this.crashed.get(targetId)?.timer);
    const previous = this.backoff.get(targetId);
    const attempts = previous && performance.now() - previous.at < 60_000 ? previous.attempts + 1 : 0;
    this.backoff.set(targetId, { attempts, at: performance.now() });
    const entry = { connection: this.browser.connection, sessionId: this.browser.sessionsByTarget.get(targetId) };
    this.crashed.set(targetId, entry);
    this.browser.pageFramesByTarget.delete(targetId);
    this.browser.focusWorldsByTarget.delete(targetId);
    this.browser.snapshotStateByTarget.delete(targetId);
    this.browser.appliedViewport = null;
    this.schedule(targetId, entry, Math.min(30_000, 1000 * 2 ** Math.min(attempts, 5)));
  }
  schedule(targetId, entry, delay) {
    const current = () => this.crashed.get(targetId) === entry && this.browser.connection === entry.connection;
    clearTimeout(entry.timer);
    entry.timer = setTimeout(async () => {
      if (!current()) return;
      try {
        await this.browser.memory.requireHeadroom();
        if (!current()) return;
        entry.awaitingCommit = true;
        await entry.connection.send('Page.reload', {}, entry.sessionId);
        // CDP acknowledges dispatch, not a replacement document. Keep a
        // bounded retry alive until frameNavigated retires this exact attempt.
        if (current()) this.schedule(targetId, entry, 5000);
      } catch {
        // Keep a definitive crashed state while memory/CDP is unavailable.
        // A late retry cannot operate on another browser generation or tab.
        if (current()) this.schedule(targetId, entry, 5000);
      }
    }, delay);
    entry.timer.unref?.();
  }
}
