// MP-08/MP-10/MP-11: the same pressure policy for a Room and a native browser.
// Node's availableMemory includes the process's cgroup limit, unlike host RAM.
const MiB = 1024 * 1024;
const HEADROOM = 512 * MiB;
const RECLAIM_HEADROOM = 1024 * MiB;

export class BrowserMemoryPressureError extends Error {
  constructor() {
    super('Browser needs 512 MiB available; close a tab or stop another process, then retry.');
    this.code = 'browser_memory_pressure';
  }
}

export function requireBrowserMemory(available = process.availableMemory()) {
  if (!Number.isSafeInteger(available) || available < HEADROOM) throw new BrowserMemoryPressureError();
}

export class BrowserMemory {
  constructor({ availableMemory = () => process.availableMemory(), interval = 1000 } = {}) {
    this.availableMemory = availableMemory;
    this.interval = interval;
  }
  close() {
    clearTimeout(this.timer);
    this.connection = null;
    this.watchState = null;
    this.lastNotification = null;
  }
  watch(connection) {
    this.close();
    this.connection = connection;
    const watch = this.watchState = {};
    const tick = async () => {
      if (this.watchState !== watch || !connection.isOpen()) return;
      try { await this.reclaim(); }
      catch { /* Allocation is still refused below; a lost CDP connection owns recovery. */ }
      if (this.watchState === watch && connection.isOpen()) {
        this.timer = setTimeout(tick, this.interval);
        this.timer.unref?.();
      }
    };
    this.timer = setTimeout(tick, this.interval);
    this.timer.unref?.();
  }
  async reclaim() {
    const free = this.availableMemory();
    if (!Number.isSafeInteger(free) || free < 0) throw new BrowserMemoryPressureError();
    const level = free < HEADROOM ? 'critical' : free < RECLAIM_HEADROOM ? 'moderate' : null;
    if (!level) { this.lastNotification = null; return; }
    const connection = this.connection;
    if (!connection?.isOpen()) return;
    const now = performance.now();
    if (this.lastNotification?.level === level && now - this.lastNotification.at < 5000) return;
    // Chromium releases image/resource caches and spare renderers using its own
    // pressure handlers. Keep site isolation and the active document intact.
    this.lastNotification = { level, at: now };
    await connection.send('Memory.simulatePressureNotification', { level });
  }
  async requireHeadroom() {
    if (this.availableMemory() < HEADROOM) await this.reclaim().catch(() => {});
    requireBrowserMemory(this.availableMemory());
  }
}
