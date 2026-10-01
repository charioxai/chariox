// Only packaged JavaScript runs here. Backend content is always plain text.
export function mountCampaignView({ document, bridge, storage, setTimer = setTimeout, clearTimer = clearTimeout }) {
  const title = document.getElementById('campaign-title');
  const body = document.getElementById('campaign-body');
  const status = document.getElementById('status');
  const draft = document.getElementById('draft');
  let timer;
  let closed = false;
  let pending;
  try { draft.value = storage.getItem('campaign-draft') ?? ''; } catch {}
  draft.addEventListener('input', () => {
    try { storage.setItem('campaign-draft', draft.value); } catch {}
  });

  async function refresh() {
    if (closed) return;
    if (pending) return pending;
    pending = (async () => {
      try {
        const result = await bridge.call('read_campaign', {});
        if (closed) return;
        title.textContent = result.campaign?.title ?? 'No active campaign';
        body.textContent = result.campaign?.body ?? '';
        const labels = { fresh: 'Current campaign', stale: 'Offline cache, content may be out of date',
          expired: 'Cached campaign expired', scheduled: 'Campaign has not started', unavailable: 'Campaign unavailable' };
        status.textContent = labels[result.status] ?? 'Campaign unavailable';
        if (result.refreshFailed) status.textContent += '. Latest refresh failed.';
        // Never replace the draft node, value, selection or focus on refresh.
      } catch {
        if (closed) return;
        // Without the backend's expiry decision, fail closed on campaign data.
        title.textContent = 'Campaign unavailable';
        body.textContent = '';
        status.textContent = 'Could not contact the App backend';
      }
    })().finally(() => { pending = undefined; });
    return pending;
  }
  async function poll() {
    if (!document.hidden) await refresh();
    if (!closed) timer = setTimer(poll, 5000);
  }
  void poll();
  return { refresh, close() { closed = true; clearTimer(timer); } };
}

if (typeof window !== 'undefined') {
  const view = mountCampaignView({ document, bridge: window.chariox, storage: window.sessionStorage });
  window.addEventListener('pagehide', () => view.close(), { once: true });
}
