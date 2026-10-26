// MP-08 / MP-10 / MP-11 H1/H13. Adapted from round2 har at 63ec764151c90d6e1672befd93e19143a2390ac9; Passive metadata only, never cookies/auth/bodies.
import { isSensitiveDrillKey, redactDrillSecretText } from '../../lib/drill-secrets.mjs'
const REQUEST_HEADERS = new Set(['accept', 'accept-language', 'content-type', 'sec-fetch-dest', 'sec-fetch-mode', 'sec-fetch-site', 'sec-fetch-user'])
const RESPONSE_HEADERS = new Set(['content-type', 'content-length', 'location'])
const metadataBytes = value => Buffer.byteLength(JSON.stringify(value))
const headers = (values, allowlist) => Object.entries(values ?? {})
  .filter(([name]) => allowlist.has(name.toLowerCase()))
  .map(([name, value]) => ({ name: name.toLowerCase(), value: name.toLowerCase() === 'location' ? nonSecretUrl(String(value)) : redactDrillSecretText(String(value)) }))

function nonSecretUrl(value) {
  try {
    const url = new URL(value)
    url.username = ''; url.password = ''; url.hash = ''
    for (const key of [...url.searchParams.keys()]) if (isSensitiveDrillKey(key)) url.searchParams.delete(key)
    return redactDrillSecretText(url.href)
  } catch { return redactDrillSecretText(value.split(/[?#]/)[0]) }
}

export class PassiveHar {
  entries = []
  #requests = new Map()
  #errors = new Set()
  #failedRequests = 0
  #flushAcknowledged = false
  #closed = false
  #retainedBytes = 0
  #pendingExtras = 0

  constructor({ maxRequests = 10000, maxEntries = 100000, maxBytes = 64 * 1024 * 1024 } = {}) {
    if (![maxRequests, maxEntries, maxBytes].every(v => Number.isSafeInteger(v) && v > 0)) throw new Error('MP-10 invalid HAR capacity')
    this.maxRequests = maxRequests; this.maxEntries = maxEntries; this.maxBytes = maxBytes
  }

  #fail(message) { this.#errors.add(message); throw new Error(`MP-10 HAR ${message}`) }

  #charge(bytes) {
    if (this.#retainedBytes + bytes > this.maxBytes) this.#fail('metadata byte capacity exceeded')
    this.#retainedBytes += bytes
  }

  #retain(value) {
    this.#charge(metadataBytes(value))
    return value
  }

  #update(entry, fields, releasedBytes = 0) {
    const updated = { ...entry, ...fields }
    this.#charge(metadataBytes(updated) - metadataBytes(entry) - releasedBytes)
    Object.assign(entry, fields)
  }

  retireSession(sessionId) {
    if ([...this.#requests.keys()].some(key => JSON.parse(key)[0] === sessionId)) this.#errors.add('session retired with pending capture')
  }

  assertComplete({ allowFailedRequests = false } = {}) {
    if (this.#errors.size) throw new Error(`MP-10 HAR ${[...this.#errors].join('; ')}`)
    if (this.#requests.size) throw new Error('MP-10 HAR active requests or missing ExtraInfo/terminal events')
    if (!allowFailedRequests && this.#failedRequests) throw new Error('MP-10 HAR failed requests require declared policy')
  }

  acknowledgeFlush(policy) { this.assertComplete(policy); this.#flushAcknowledged = true; this.#closed = true }


  consume({ method, params: p = {}, sessionId = '' }) {
    if (this.#closed) throw new Error('MP-10 HAR closed')
    if (!method?.startsWith('Network.') || !p.requestId) return
    // Response ExtraInfo carries cookies/auth; this metadata-only policy omits it.
    if (!['Network.requestWillBeSent', 'Network.requestWillBeSentExtraInfo', 'Network.responseReceived', 'Network.loadingFinished', 'Network.loadingFailed'].includes(method)) return
    const key = JSON.stringify([sessionId, p.requestId])
    const state = this.#requests.get(key) ?? { hops: [], extras: [] }
    if (!this.#requests.has(key) && this.#requests.size >= this.maxRequests) this.#fail('request capacity exceeded')
    this.#requests.set(key, state)
    const current = state.hops[state.hops.length - 1]
    if (method === 'Network.requestWillBeSentExtraInfo') {
      // Keep only admitted values even while an early ExtraInfo awaits its hop.
      if (this.#pendingExtras >= this.maxEntries) this.#fail('ExtraInfo capacity exceeded')
      state.extras.push(this.#retain(headers(p.headers, REQUEST_HEADERS)))
      this.#pendingExtras++
    } else if (method === 'Network.requestWillBeSent') {
      if (current) {
        if (!p.redirectResponse) this.#fail('request ID reused without redirect')
        current.expectsExtra = p.redirectHasExtraInfo
        this.#response(current.entry, p.redirectResponse)
        this.#update(current.entry, { _redirect: true })
      }
      if (this.entries.length >= this.maxEntries) this.#fail('entry capacity exceeded')
      const entry = {
        startedDateTime: new Date(p.wallTime * 1000).toISOString(), time: -1,
        request: { method: p.request.method, url: nonSecretUrl(p.request.url), httpVersion: '',
          headers: headers(p.request.headers, REQUEST_HEADERS), cookies: [], queryString: [], headersSize: -1, bodySize: -1 },
        response: { status: 0, statusText: '', httpVersion: '', headers: [], cookies: [], content: { size: -1, mimeType: '' }, redirectURL: '', headersSize: -1, bodySize: -1 },
        cache: {}, timings: { blocked: -1, dns: -1, connect: -1, send: -1, wait: -1, receive: -1 },
        _resourceType: p.type?.toLowerCase(),
        _terminal: false,
        _requestMetadata: { sessionId, requestId: p.requestId, redirectIndex: state.hops.length,
          headersSource: 'Network.requestWillBeSent', extraInfo: 'pending' },
      }
      this.entries.push(this.#retain(entry))
      state.hops.push({ entry, expectsExtra: undefined, assigned: false, timestamp: p.timestamp })
    } else if (method === 'Network.responseReceived' && current) {
      this.#response(current.entry, p.response)
      current.expectsExtra = p.hasExtraInfo
    } else if (['Network.loadingFinished', 'Network.loadingFailed'].includes(method) && current) {
      const fields = { time: Math.max(0, (p.timestamp - current.timestamp) * 1000), _terminal: true }
      if (method === 'Network.loadingFinished') fields.response = { ...current.entry.response, bodySize: p.encodedDataLength }
      else fields._failure = 'Network.loadingFailed'
      this.#update(current.entry, fields)
      if (method === 'Network.loadingFailed') {
        this.#failedRequests++
        if (current.expectsExtra === undefined) current.expectsExtra = false
      }
    }
    this.#merge(state)
    if (current?.entry._terminal && state.hops.every(hop => hop.assigned) && !state.extras.length) this.#requests.delete(key)
  }

  #response(entry, response) {
    const responseHeaders = headers(response.headers, RESPONSE_HEADERS)
    this.#update(entry, { response: {
      ...entry.response,
      status: response.status,
      statusText: response.statusText ?? '',
      httpVersion: response.protocol ?? '',
      headers: responseHeaders,
      content: { ...entry.response.content, mimeType: response.mimeType ?? '' },
      redirectURL: responseHeaders.find(header => header.name === 'location')?.value ?? '',
    } })
  }

  #merge(state) {
    for (const hop of state.hops) {
      if (hop.assigned) continue
      // CDP guarantees ExtraInfo order for hops, not order relative to normal
      // events. Wait for hasExtraInfo/redirectHasExtraInfo to skip missing hops.
      if (hop.expectsExtra === undefined) break
      if (hop.expectsExtra === false) {
        this.#update(hop.entry, { _requestMetadata: { ...hop.entry._requestMetadata, extraInfo: 'not_emitted' } })
        hop.assigned = true; continue
      }
      if (!state.extras.length) break
      const merged = new Map(hop.entry.request.headers.map(header => [header.name, header.value]))
      const extra = state.extras[0]
      for (const header of extra) merged.set(header.name, header.value)
      this.#update(hop.entry, {
        request: { ...hop.entry.request, headers: [...merged].map(([name, value]) => ({ name, value })) },
        _requestMetadata: { ...hop.entry._requestMetadata, headersSource: 'Network.requestWillBeSentExtraInfo', extraInfo: 'merged' },
      }, metadataBytes(extra))
      state.extras.shift()
      this.#pendingExtras--
      hop.assigned = true
    }
  }

  snapshot() {
    return structuredClone({ log: { version: '1.2', creator: { name: 'Chariox passive metadata', version: 'round2' }, entries: this.entries,
      _capture: { mpItems: ['MP-08', 'MP-10', 'MP-11'], metadataOnly: true,
        activeRequests: this.#requests.size, failedRequests: this.#failedRequests,
        retainedMetadataBytes: this.#retainedBytes,
        errors: [...this.#errors], flushAcknowledged: this.#flushAcknowledged,
        missingExtraInfo: this.entries.filter(entry => entry._requestMetadata.extraInfo === 'pending').length,
        unmatchedExtraInfo: [...this.#requests.values()].reduce((count, state) => count + state.extras.length, 0) } } })
  }
}

// The caller supplies its observer's CDP send/event seams. No browser instance,
// interception, navigation, body retrieval or provider-facing CDP is created.
export async function observePassiveHar({ send, subscribe, sessionIds }) {
  const har = new PassiveHar()
  let eventError
  const unsubscribe = subscribe(message => { try { har.consume(message) } catch (error) { eventError = error } })
  try {
    for (const sessionId of sessionIds) await send('Network.enable', {}, sessionId)
  } catch (error) { unsubscribe(); throw error }
  let closed = false
  return { har, detach: unsubscribe, async close({ drain, policy, timeoutMs = 5000 } = {}) {
    if (closed) throw new Error('MP-10 HAR observer closed')
    closed = true
    if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1) { unsubscribe(); throw new Error('MP-10 invalid flush timeout') }
    let timer, expired = false
    const work = async () => {
      // Caller settles solver work and acknowledges its event queue; no mutation replay.
      if (!drain || (await drain())?.settled !== true) throw new Error('MP-10 HAR drain not acknowledged')
      if (expired) throw new Error('MP-10 HAR flush deadline')
      for (const sessionId of sessionIds) {
        await send('Runtime.getIsolateId', {}, sessionId)
        if (expired) throw new Error('MP-10 HAR flush deadline')
      }
      if (eventError) throw eventError
      har.acknowledgeFlush(policy)
      return har.snapshot()
    }
    try {
      return await Promise.race([work(), new Promise((_, reject) => {
        timer = setTimeout(() => { expired = true; reject(new Error('MP-10 HAR flush deadline')) }, timeoutMs)
      })])
    } finally { clearTimeout(timer); unsubscribe() }
  } }
}
