// Data refresh uses the same approved HTTP and installation state as any App.
// There is no worker timer: an open view or an incoming integration does work.
export const SOURCE = 'https://campaigns.example.com/current.json';
export const REFRESH_MS = 60_000;
export const FRESH_MS = 5 * 60_000;
export const OFFLINE_MS = 60 * 60_000;
const KEY = 'campaign-cache';

function campaignData(response) {
  if (response.status !== 200) throw new Error('Campaign source refused the request');
  const bytes = Buffer.from(response.bodyBase64, 'base64');
  if (bytes.length > 16 * 1024) throw new Error('Campaign data exceeds 16 KiB');
  const data = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
  const fields = ['id', 'title', 'body', 'startsAtMs', 'endsAtMs'];
  if (!data || Array.isArray(data) || typeof data !== 'object'
    || Object.keys(data).length !== fields.length || fields.some(key => !Object.hasOwn(data, key))
    || typeof data.id !== 'string' || !data.id || data.id.length > 64
    || typeof data.title !== 'string' || !data.title || data.title.length > 200
    || typeof data.body !== 'string' || data.body.length > 4000
    || !Number.isSafeInteger(data.startsAtMs) || data.startsAtMs < 0
    || !Number.isSafeInteger(data.endsAtMs) || data.endsAtMs <= data.startsAtMs) {
    throw new Error('Invalid campaign data');
  }
  // Closed data shape: no mutable code, templates, image URLs or HTML.
  return data;
}

export default function register(chariox, { now = Date.now } = {}) {
  let pending;
  const load = () => chariox.state.get(KEY);
  const write = (record, value) => chariox.state.transaction({
    schemaVersion: 0,
    checks: [{ key: KEY, version: record?.version ?? null }],
    writes: [{ key: KEY, value }],
  });

  async function refresh() {
    // Persist the attempt before networking. Reopening/restarting and failed
    // requests cannot bypass the interval. CAS also protects concurrent callers.
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const record = await load();
      const at = now();
      if (at < (record?.value?.nextRefreshAtMs ?? 0)) return;
      const claim = { ...record?.value, nextRefreshAtMs: at + REFRESH_MS };
      try { await write(record, claim); }
      catch (error) {
        if (error?.code === 'CONFLICT') continue;
        throw error;
      }
      // Read the committed claim's version; never overwrite a newer claim.
      const claimed = await load();
      if (claimed.value.nextRefreshAtMs !== claim.nextRefreshAtMs) return;
      let next;
      try {
        const response = await chariox.http.request({ url: SOURCE, method: 'GET',
          headers: [['accept', 'application/json']] }, { timeoutMs: 5000 });
        const campaign = campaignData(response);
        const fetchedAtMs = now();
        next = { ...claim, campaign, fetchedAtMs, refreshFailed: false,
          freshUntilMs: Math.min(fetchedAtMs + FRESH_MS, campaign.endsAtMs),
          offlineUntilMs: Math.min(fetchedAtMs + OFFLINE_MS, campaign.endsAtMs) };
      } catch {
        // A failed or invalid response leaves the last good content intact.
        // Do not expose network diagnostics or untrusted source text as errors.
        next = { ...claim, refreshFailed: true };
      }
      try { await write(claimed, next); }
      catch (error) { if (error?.code !== 'CONFLICT') throw error; }
      return;
    }
    throw new chariox.AppError('CONFLICT', 'Campaign refresh is busy; try again');
  }

  async function read() {
    // Human calls, agent calls and integration deliveries share one request.
    if (!pending) pending = refresh().finally(() => { pending = undefined; });
    await pending;
    const value = (await load())?.value ?? {};
    const at = now();
    let status = 'unavailable';
    let campaign = null;
    if (value.campaign) {
      if (at >= value.offlineUntilMs) status = 'expired';
      else if (at < value.campaign.startsAtMs) status = 'scheduled';
      else {
        campaign = value.campaign;
        status = at < value.freshUntilMs ? 'fresh' : 'stale';
      }
    }
    return { campaign, status, refreshFailed: value.refreshFailed ?? false,
      fetchedAtMs: value.fetchedAtMs ?? null, freshUntilMs: value.freshUntilMs ?? null,
      offlineUntilMs: value.offlineUntilMs ?? null, nextRefreshAtMs: value.nextRefreshAtMs ?? at };
  }

  chariox.tools.register('read_campaign', read);
  // This event is delivered only by a configured installation inbox route.
  // The payload cannot replace data or change the signed source destination.
  chariox.events.register('campaign_changed', async () => { await read(); return null; });
}
