// Only packaged JavaScript runs here. Backend content is always plain text.
export function mountCampaignView({ document, bridge, storage, setTimer = setTimeout, clearTimer = clearTimeout, now = Date.now }) {
  const title = document.getElementById('campaign-title');
  const body = document.getElementById('campaign-body');
  const status = document.getElementById('status');
  const draft = document.getElementById('draft');
  let timer;
  let expiryTimer;
  let displayedUntilMs = null;
  let closed = false;
  let pending;
  let cancelWait;
  // The bridge has no cancellation API. Retain at most two unanswered calls,
  // allowing recovery after one lost reply without accumulating a call queue.
  const unanswered = new Set();
  try { draft.value = storage.getItem('campaign-draft') ?? ''; } catch {}
  draft.addEventListener('input', () => {
    try { storage.setItem('campaign-draft', draft.value); } catch {}
  });

  function checkExpiry() {
    clearTimer(expiryTimer);
    if (closed || displayedUntilMs === null) return;
    if (now() >= displayedUntilMs) {
      displayedUntilMs = null;
      title.textContent = 'No active campaign';
      body.textContent = '';
      status.textContent = 'Cached campaign expired';
    } else expiryTimer = setTimer(checkExpiry, Math.min(2_147_483_647, displayedUntilMs - now()));
  }

  async function read() {
    if (unanswered.size >= 2) throw new Error('Bridge is unavailable');
    const call = Promise.resolve().then(() => bridge.call('read_campaign', {}));
    unanswered.add(call);
    call.then(() => unanswered.delete(call), () => unanswered.delete(call));
    let deadline;
    const timeout = new Promise((_resolve, reject) => {
      cancelWait = () => reject(new Error('Bridge wait ended'));
      deadline = setTimer(cancelWait, 10_000);
    });
    try { return await Promise.race([call, timeout]); }
    finally { clearTimer(deadline); cancelWait = undefined; }
  }

  async function refresh() {
    if (closed) return;
    if (pending) return pending;
    pending = (async () => {
      try {
        const result = await read();
        if (closed) return;
        clearTimer(expiryTimer);
        displayedUntilMs = result.campaign && Number.isSafeInteger(result.offlineUntilMs)
          && Number.isSafeInteger(result.campaign.endsAtMs)
          ? Math.min(result.offlineUntilMs, result.campaign.endsAtMs) : null;
        title.textContent = displayedUntilMs === null ? 'No active campaign' : result.campaign.title;
        body.textContent = displayedUntilMs === null ? '' : result.campaign.body;
        const labels = { fresh: 'Current campaign', stale: 'Offline cache, content may be out of date',
          expired: 'Cached campaign expired', scheduled: 'Campaign has not started', unavailable: 'Campaign unavailable' };
        status.textContent = labels[result.status] ?? 'Campaign unavailable';
        if (result.refreshFailed) status.textContent += '. Latest refresh failed.';
        checkExpiry();
        // Never replace the draft node, value, selection or focus on refresh.
      } catch {
        if (closed) return;
        // Without the backend's expiry decision, fail closed on campaign data.
        title.textContent = 'Campaign unavailable';
        body.textContent = '';
        displayedUntilMs = null;
        clearTimer(expiryTimer);
        status.textContent = 'Could not contact the App backend. Reopen the App if it does not recover.';
      }
    })().finally(() => { pending = undefined; });
    return pending;
  }
  async function poll() {
    if (!document.hidden) await refresh();
    if (!closed) timer = setTimer(poll, 5000);
  }
  function onVisibility() {
    checkExpiry();
    if (!document.hidden && !closed) void refresh();
  }
  document.addEventListener('visibilitychange', onVisibility);
  void poll();
  return { refresh, close() {
    closed = true;
    clearTimer(timer);
    clearTimer(expiryTimer);
    cancelWait?.();
    document.removeEventListener('visibilitychange', onVisibility);
  } };
}

if (typeof window !== 'undefined') {
  const view = mountCampaignView({ document, bridge: window.chariox, storage: window.sessionStorage });
  window.addEventListener('pagehide', () => view.close(), { once: true });
}
