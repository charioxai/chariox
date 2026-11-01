// MP-08/MP-10/MP-11 M2: metadata for responses this exact renderer loaded.
// Bodies stay in Chromium until requested. No new fetch, headers or credentials.
const MAX = 4096;
export const MAX_RESPONSE_BODY_BYTES = 4 * 1024 * 1024;
// Chromium's default response cache is much larger than our reader. Keep two
// readable bodies per session; eviction already uses the ordinary resource path.
export const responseBufferLimits = Object.freeze({
  maxResourceBufferSize: MAX_RESPONSE_BODY_BYTES,
  maxTotalBufferSize: 2 * MAX_RESPONSE_BODY_BYTES,
});
const key = (...parts) => JSON.stringify(parts);
const url = value => typeof value === 'string' && value.length <= 8192 && /^https?:\/\//.test(value);
export class BrowserResponseBodies {
  constructor() { this.requests = new Map(); this.loaded = new Map(); }
  clear() { this.requests.clear(); this.loaded.clear(); }
  drop(sessionId) {
    for (const [id, entry] of this.requests) if (entry.sessionId === sessionId) this.requests.delete(id);
    for (const [id, entry] of this.loaded) if (entry.sessionId === sessionId) this.loaded.delete(id);
  }
  observe(message) {
    const { sessionId, method, params: p } = message ?? {};
    if (typeof sessionId !== 'string' || typeof p?.requestId !== 'string') return;
    const id = key(sessionId, p.requestId);
    if (method === 'Network.requestWillBeSent' || method === 'Network.responseReceived') {
      const entry = this.requests.get(id) ?? { sessionId, requestId: p.requestId, urls: new Set() };
      const address = p.request?.url ?? p.response?.url;
      if (url(address) && entry.urls.size < 8) entry.urls.add(address);
      if (typeof p.frameId === 'string') entry.frameId = p.frameId;
      if (typeof p.loaderId === 'string') entry.loaderId = p.loaderId;
      this.requests.delete(id); this.requests.set(id, entry);
      if (this.requests.size > MAX) this.requests.delete(this.requests.keys().next().value);
    } else if (method === 'Network.loadingFinished') {
      const entry = this.requests.get(id); this.requests.delete(id);
      if (!entry || !entry.frameId || !entry.loaderId) return;
      for (const address of entry.urls) {
        const lookup = key(sessionId, entry.frameId, entry.loaderId, address);
        this.loaded.delete(lookup); this.loaded.set(lookup, entry);
        if (this.loaded.size > MAX) this.loaded.delete(this.loaded.keys().next().value);
      }
    } else if (method === 'Network.loadingFailed') this.requests.delete(id);
  }
  async read(world, address, maxBytes = MAX_RESPONSE_BODY_BYTES) {
    const documentId = world.documentId ?? world.frame?.loaderId;
    if (!documentId) return null;
    const lookup = key(world.sessionId, world.frame.id, documentId, address), entry = this.loaded.get(lookup);
    if (!entry) return null;
    try {
      const reply = await world.connection.send('Network.getResponseBody', { requestId: entry.requestId }, world.sessionId);
      if (this.loaded.get(lookup) !== entry || typeof reply.body !== 'string') return null;
      const encoding = reply.base64Encoded ? 'base64' : 'utf8';
      if (Buffer.byteLength(reply.body, encoding) > maxBytes) return null;
      return Buffer.from(reply.body, encoding);
    } catch { return null; } // evicted response: ordinary resource admission/read remains the fallback
  }
}
