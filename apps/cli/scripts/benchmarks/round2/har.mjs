// MP-08 / MP-10 / MP-11 H1. Passive metadata only, never cookies/auth/bodies.
import { isSensitiveDrillKey, redactDrillSecretText } from '../../lib/drill-secrets.mjs'
const REQUEST_HEADERS = new Set(['accept', 'accept-language', 'content-type', 'sec-fetch-dest', 'sec-fetch-mode', 'sec-fetch-site', 'sec-fetch-user'])
const RESPONSE_HEADERS = new Set(['content-type', 'content-length', 'location'])
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

  consume({ method, params: p = {}, sessionId = '' }) {
    if (!method.startsWith('Network.') || !p.requestId) return
    const key = JSON.stringify([sessionId, p.requestId])
    const state = this.#requests.get(key) ?? { hops: [], extras: [] }
    this.#requests.set(key, state)
    const current = state.hops[state.hops.length - 1]
    if (method === 'Network.requestWillBeSentExtraInfo') {
      // Keep only admitted values even while an early ExtraInfo awaits its hop.
      state.extras.push(headers(p.headers, REQUEST_HEADERS))
    } else if (method === 'Network.requestWillBeSent') {
      if (current) {
        if (!p.redirectResponse) throw new Error('MP-10 HAR request ID reused without redirect')
        current.expectsExtra = p.redirectHasExtraInfo
        this.#response(current.entry, p.redirectResponse)
        current.entry._redirect = true
      }
      const entry = {
        startedDateTime: new Date(p.wallTime * 1000).toISOString(), time: -1,
        request: { method: p.request.method, url: nonSecretUrl(p.request.url), httpVersion: '',
          headers: headers(p.request.headers, REQUEST_HEADERS), cookies: [], queryString: [], headersSize: -1, bodySize: -1 },
        response: { status: 0, statusText: '', httpVersion: '', headers: [], cookies: [], content: { size: -1, mimeType: '' }, redirectURL: '', headersSize: -1, bodySize: -1 },
        cache: {}, timings: { blocked: -1, dns: -1, connect: -1, send: -1, wait: -1, receive: -1 },
        _resourceType: p.type?.toLowerCase(),
        _requestMetadata: { sessionId, requestId: p.requestId, redirectIndex: state.hops.length,
          headersSource: 'Network.requestWillBeSent', extraInfo: 'pending' },
      }
      state.hops.push({ entry, expectsExtra: undefined, assigned: false, timestamp: p.timestamp })
      this.entries.push(entry)
    } else if (method === 'Network.responseReceived' && current) {
      this.#response(current.entry, p.response)
      current.expectsExtra = p.hasExtraInfo
    } else if (['Network.loadingFinished', 'Network.loadingFailed'].includes(method) && current) {
      current.entry.time = Math.max(0, (p.timestamp - current.timestamp) * 1000)
      if (method === 'Network.loadingFinished') current.entry.response.bodySize = p.encodedDataLength
      else current.entry._failure = p.errorText
    }
    this.#merge(state)
  }

  #response(entry, response) {
    entry.response.status = response.status
    entry.response.statusText = response.statusText ?? ''
    entry.response.httpVersion = response.protocol ?? ''
    entry.response.headers = headers(response.headers, RESPONSE_HEADERS)
    entry.response.content.mimeType = response.mimeType ?? ''
    entry.response.redirectURL = entry.response.headers.find(header => header.name === 'location')?.value ?? ''
  }

  #merge(state) {
    for (const hop of state.hops) {
      if (hop.assigned) continue
      // CDP guarantees ExtraInfo order for hops, not order relative to normal
      // events. Wait for hasExtraInfo/redirectHasExtraInfo to skip missing hops.
      if (hop.expectsExtra === undefined) break
      if (hop.expectsExtra === false) {
        hop.assigned = true; hop.entry._requestMetadata.extraInfo = 'not_emitted'; continue
      }
      if (!state.extras.length) break
      const merged = new Map(hop.entry.request.headers.map(header => [header.name, header.value]))
      for (const header of state.extras.shift()) merged.set(header.name, header.value)
      hop.entry.request.headers = [...merged].map(([name, value]) => ({ name, value }))
      hop.assigned = true
      hop.entry._requestMetadata.headersSource = 'Network.requestWillBeSentExtraInfo'
      hop.entry._requestMetadata.extraInfo = 'merged'
    }
  }

  snapshot() {
    return { log: { version: '1.2', creator: { name: 'Chariox passive metadata', version: 'round2' }, entries: this.entries,
      _capture: { mpItems: ['MP-08', 'MP-10', 'MP-11'], metadataOnly: true,
        missingExtraInfo: this.entries.filter(entry => entry._requestMetadata.extraInfo === 'pending').length,
        unmatchedExtraInfo: [...this.#requests.values()].reduce((count, state) => count + state.extras.length, 0) } } }
  }
}

// The caller supplies its observer's CDP send/event seams. No browser instance,
// interception, navigation, body retrieval or provider-facing CDP is created.
export async function observePassiveHar({ send, subscribe, sessionIds }) {
  const har = new PassiveHar()
  const unsubscribe = subscribe(message => har.consume(message))
  try {
    for (const sessionId of sessionIds) await send('Network.enable', {}, sessionId)
  } catch (error) { unsubscribe(); throw error }
  return { har, detach: unsubscribe }
}
