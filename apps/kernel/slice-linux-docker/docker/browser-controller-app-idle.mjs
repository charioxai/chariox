// MP-08/MP-09/MP-10/MP-11: the kernel controller retires idle Room App Tabs.
// Its drain keeps running without viewers. A deadline does not settle a handler.
export const APP_VIEW_IDLE_MS = 10 * 60 * 1000;

export function appViewIdleMs() {
  const raw = process.env.CHARIOX_APP_VIEW_IDLE_TIMEOUT_MS;
  if (raw === undefined) return APP_VIEW_IDLE_MS;
  const ms = Number(raw);
  if (!/^\d+$/.test(raw) || !Number.isSafeInteger(ms) || ms < 1) {
    throw new Error('CHARIOX_APP_VIEW_IDLE_TIMEOUT_MS must be a positive integer');
  }
  return ms;
}

export async function retireIdleAppTabs(tabs) {
  const now = tabs.now();
  const viewing = [...(tabs.browser.roomDisplay?.displays.values() ?? [])]
    .some(stream => !stream.suspended && stream.expires > Date.now());
  for (const [session, app] of tabs.apps) {
    // User-domain instances have their own lifetime. Room admission and display
    // leases never confer authority over those instances or another Room.
    if (app.instanceId !== null || app.retiring) continue;
    if (app.agentCalls || app.agentRunning || app.pending.size || (!app.background && viewing)) {
      app.lastActivity = now;
      continue;
    }
    if (now - app.lastActivity < tabs.idleMs) continue;
    // Reserve before awaiting CDP: an agent refuses this retiring target, and
    // a concurrent Open waits then creates a fresh one. A failed close retries.
    app.retiring = Promise.resolve().then(async () => {
      await tabs.browser.closePageTarget(tabs.connection, app.targetId);
      if (tabs.apps.get(session) === app) tabs.apps.delete(session);
      tabs.calls = tabs.calls.filter(call => call.target_id !== app.targetId);
      tabs.wakeCalls();
    });
    try {
      await app.retiring;
    } catch {
      // One failing close must not prevent another Tab's admitted calls from
      // reaching the kernel. Report once and retry on the next idle drain.
      if (!app.idleCloseFailureReported) {
        app.idleCloseFailureReported = true;
        console.error("App idle cleanup could not close a Tab; retrying on the next drain");
      }
    } finally { app.retiring = null; }
  }
}
