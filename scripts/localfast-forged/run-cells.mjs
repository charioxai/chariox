// LOCALFAST-A1 denial cells F0..F9 against a private pr-stack and drill kernels.
// usage: node run-cells.mjs <evidence-dir> <state-k1.json> <state-k2.json> <state-k3.json> [cell ids, default all]
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import net from 'node:net'
import {
  cloudSession, relayCall, wsCarrier, rtcEngine, prove, connectedOk, refusedReason, directRequest,
  newIdentity, sleep,
} from './forged-client.mjs'

const [evDir, s1, s2, s3, ...only] = process.argv.slice(2)
mkdirSync(evDir, { recursive: true })
const K = { k1: JSON.parse(readFileSync(s1, 'utf8')), k2: JSON.parse(readFileSync(s2, 'utf8')), k3: JSON.parse(readFileSync(s3, 'utf8')) }
try { K.k4 = JSON.parse(readFileSync(s1.replace('state-k1', 'state-k4'), 'utf8')) } catch {}
const stack = K.k1.stack
const results = []
const log = []
const note = (...a) => { const l = a.map(x => typeof x === 'string' ? x : JSON.stringify(x)).join(' '); log.push(l); console.log(l) }
const cell = (id, status, detail) => { const r = { id, status, at: new Date().toISOString(), detail }; results.push(r); console.log('CELL', JSON.stringify(r)); writeFileSync(`${evDir}/cells.json`, JSON.stringify(results, null, 1)); writeFileSync(`${evDir}/cells.log`, log.join('\n')) }
const want = id => !only.length || only.includes(id)
const target = k => ({ daemon_id: K[k].daemon })

const sess = await cloudSession({ stack })
const rtc = await rtcEngine()
const LIST = { ListSessions: null }

/** Mints one grant for `identity` on kernel `k` the way the page does: 30 s relay lease, encrypted local_browser_connect. */
async function mint(k, identity, { offer, lease, token } = {}) {
  const tok = token ?? (lease ?? await sess.lease(K[k].daemon, identity.publicKeyThumbprint)).json?.relayToken
  if (!tok) return { error: 'no relay token', lease }
  const r = await relayCall({ relayUrl: relayUrlFor(), token: tok, target: target(k), identity,
    bodies: [{ local_browser_connect: offer ? { offer } : {} }] })
  const body = r.responses[0]?.body
  const issued = body?.LocalBrowserConnectIssued
  return { issued, kernelKey: r.kernelKeyFromRelay, relay: r, refusal: r.responses[0]?.error ?? r.error, body }
}
let relayWs = null
function relayUrlFor() { return relayWs }
{ // relay URL from a bootstrap (same call the page makes)
  const id = newIdentity()
  const b = await sess.bootstrap(K.k1.daemon, id.publicKeyThumbprint)
  relayWs = b.json?.relayUrl; note('relay', relayWs, 'bootstrap', b.status)
}

const wsUrl = issued => issued.endpoint
const goodHeaders = () => ({ Origin: sess.origin })

async function admitWs(k, identity, mutate, { headers = goodHeaders(), grantOverride } = {}) {
  const m = await mint(k, identity)
  if (!m.issued) return { m, mintFailed: true }
  const c = await wsCarrier(wsUrl(m.issued), { headers })
  if (!c.send) return { m, upgrade: c }
  const v = await prove(c, { grant: grantOverride ?? m.issued.grant, kernelKey: m.kernelKey, identity, origin: sess.origin, mutate })
  return { m, c, v }
}
async function admitRtc(k, identity, mutate) {
  const { sdp, carrier } = await rtc.offer()
  const m = await mint(k, identity, { offer: sdp })
  if (!m.issued) { carrier.close(); return { m, mintFailed: true } }
  const open = await carrier.accept(m.issued.answer)
  if (!open) { carrier.close(); return { m, notOpen: true } }
  const v = await prove(carrier, { grant: m.issued.grant, kernelKey: m.kernelKey, identity, origin: sess.origin, mutate })
  return { m, c: carrier, v }
}
const verdictOf = r => r.mintFailed ? `mint refused: ${JSON.stringify(r.m.refusal ?? r.m.body ?? r.m.error).slice(0, 160)}` : r.upgrade ? `upgrade HTTP ${r.upgrade.refused}` : r.notOpen ? 'channel did not open' : (connectedOk(r.v) ? 'ADMITTED' : `denied: ${refusedReason(r.v)}`)
const denied = r => !r.mintFailed && !r.notOpen && !connectedOk(r.v) && !(r.upgrade?.send)

/** A hand-written HTTP upgrade so a cell controls every header line (duplicate Origin, odd Host, proxy headers, query). */
function rawUpgrade(endpoint, { path, host, origins = [], extra = [] }) {
  const u = new URL(endpoint)
  return new Promise(res => {
    const s = net.connect(Number(u.port), '127.0.0.1')
    let buf = ''
    const t = setTimeout(() => { s.destroy(); res('timeout') }, 5000)
    s.on('data', d => { buf += d.toString('latin1'); if (buf.includes('\r\n')) { clearTimeout(t); res(buf.split('\r\n')[0]); s.destroy() } })
    s.on('error', e => { clearTimeout(t); res('error ' + e.message) })
    s.on('connect', () => s.write([`GET ${path ?? u.pathname} HTTP/1.1`, `Host: ${host ?? u.host}`, 'Upgrade: websocket', 'Connection: Upgrade',
      'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==', 'Sec-WebSocket-Version: 13', ...origins.map(o => `Origin: ${o}`), ...extra, '', ''].join('\r\n')))
  })
}

const cells = {
  async F0() {
    const id = newIdentity(); const out = {}
    const w = await admitWs('k1', id)
    out.ws = verdictOf(w)
    if (w.c && connectedOk(w.v)) { const q = await directRequest(w.c, { identity: id, kernelKey: w.m.kernelKey, target: target('k1'), body: LIST }); out.wsRequest = q.body ? Object.keys(q.body)[0] : q; w.c.close() }
    const r = await admitRtc('k1', id)
    out.rtc = verdictOf(r)
    if (r.c && connectedOk(r.v)) { const q = await directRequest(r.c, { identity: id, kernelKey: r.m.kernelKey, target: target('k1'), body: LIST }); out.rtcRequest = q.body ? Object.keys(q.body)[0] : q; r.c.close() }
    cell('F0 control: unforged client is admitted on WebSocket and WebRTC and gets a reply', out.ws === 'ADMITTED' && out.rtc === 'ADMITTED' && out.wsRequest === 'SessionsListed' && out.rtcRequest === 'SessionsListed' ? 'PASS' : 'FAIL', out)
  },

  async F1() {
    const id = newIdentity(); const m = await mint('k1', id); const ep = m.issued.endpoint; const o = sess.origin; const out = {}
    const variants = {
      noOrigin: { origins: [] }, nullOrigin: { origins: ['null'] }, otherSite: { origins: ['https://evil.example'] },
      lookalikeSuffix: { origins: [o + '.evil.example'] }, lookalikePrefix: { origins: [o.replace('https://', 'https://evil.')] },
      httpScheme: { origins: [o.replace('https:', 'http:')] }, trailingSlash: { origins: [o + '/'] }, upperCase: { origins: [o.toUpperCase()] },
      duplicateSameOrigin: { origins: [o, o] }, duplicateEvilSecond: { origins: [o, 'https://evil.example'] },
      localhostHost: { origins: [o], host: ep.replace('ws://127.0.0.1', 'localhost').replace('/v1/browser', '') },
      forwardedFor: { origins: [o], extra: ['X-Forwarded-For: 203.0.113.9'] }, forwardedHost: { origins: [o], extra: ['X-Forwarded-Host: evil.example'] },
      query: { origins: [o], path: '/v1/browser?x=1' }, otherPath: { origins: [o], path: '/v1/other' },
    }
    for (const [name, v] of Object.entries(variants)) out[name] = await rawUpgrade(ep, v)
    // the same grant was never presented, so the honest upgrade still works afterwards (refusals leave nothing half-open)
    const c = await wsCarrier(ep, { headers: goodHeaders() })
    out.honestAfterRefusals = c.send ? 'upgraded' : `HTTP ${c.refused}`
    if (c.send) { const v = await prove(c, { grant: m.issued.grant, kernelKey: m.kernelKey, identity: id, origin: o }); out.honestProof = connectedOk(v) ? 'ADMITTED' : refusedReason(v); c.close() }
    // a real Chromium page on a foreign origin: the browser itself sets that Origin, the kernel must refuse the upgrade
    const ctx = await rtc.browser.newContext(); const page = await ctx.newPage()
    await page.route('http://evil.test/**', r => r.fulfill({ status: 200, contentType: 'text/html', body: '<html></html>' })); await page.goto('http://evil.test/')
    out.chromiumForeignPage = await page.evaluate(url => new Promise(res => { const w = new WebSocket(url); w.onopen = () => res('OPEN'); w.onerror = () => res('refused'); w.onmessage = () => res('GOT-FRAME'); setTimeout(() => res('timeout'), 5000) }), ep)
    await ctx.close()
    // WebRTC: the proof's origin field is the Origin check on a data channel
    for (const [name, origin] of Object.entries({ evilOrigin: 'https://evil.example', nullOrigin: 'null', lookalike: o + '.evil.example' })) {
      const r = await admitRtc('k1', id, p => ({ ...p, origin })); out['rtc_' + name] = verdictOf(r); r.c?.close()
    }
    const bad = Object.entries(out).filter(([k, v]) => !['honestAfterRefusals', 'honestProof'].includes(k) && !/^(HTTP\/1\.1 403|refused|denied)/.test(String(v)))
    cell('F1 invalid Origin (and Host/proxy/path) denied on WebSocket and WebRTC', !bad.length && out.honestProof === 'ADMITTED' ? 'PASS' : 'FAIL', { ...out, notDenied: bad })
  },
}

import { defineF2toF6 } from './cells-f2-f6.mjs'
import { createRelayKeypair, RelayClientIdentity, WebSocket } from './forged-client.mjs'
Object.assign(cells, defineF2toF6({ admitWs, admitRtc, mint, sess, rtc, K, target, denied, verdictOf, sleep, LIST, goodHeaders, relayUrlFor, cell, newIdentity, wsCarrier, prove, connectedOk, refusedReason, relayCall, createRelayKeypair, RelayClientIdentity, directRequest, WebSocket }))

import { defineF8 } from './cells-f8.mjs'
Object.assign(cells, defineF8({ statePath: s3, note, mint, admitRtc, sess, K, target, cell, newIdentity, wsCarrier, prove, connectedOk, goodHeaders, sleep, directRequest }))

import { defineF7 } from './cells-f7.mjs'
Object.assign(cells, defineF7({ rtc, note, mint, sess, K, target, cell, newIdentity, wsCarrier, prove, connectedOk, goodHeaders, sleep, refusedReason, createRelayKeypair, RelayClientIdentity }))

import { defineF9 } from './cells-f9.mjs'
Object.assign(cells, defineF9({ note, mint, sess, K, target, cell, newIdentity, wsCarrier, prove, connectedOk, goodHeaders, sleep, refusedReason, relayCall, relayUrlFor, directRequest, admitRtc, WebSocket }))

import { defineF4r } from './cells-f4r.mjs'
Object.assign(cells, defineF4r({ mint, sess, cell, newIdentity, wsCarrier, prove, connectedOk, refusedReason, goodHeaders, sleep, admitRtc, rtc, K }))

export { cells, admitWs, admitRtc, mint, cell, note, sess, rtc, K, target, want, denied, verdictOf, sleep, LIST, relayUrlFor, goodHeaders, wsUrl, net }

if (import.meta.url === `file://${process.argv[1]}`) {
  try { for (const id of ['F0', 'F1', 'F2', 'F3', 'F4', 'F4r', 'F5', 'F6', 'F7', 'F8', 'F8w', 'F9']) { const fn = cells[id]; if (fn && want(id)) { try { await Promise.race([fn(), sleep(240000).then(() => { throw new Error('cell watchdog 240 s') })]) } catch (e) { cell(id, 'FAIL', 'harness error: ' + String(e.stack ?? e).slice(0, 500)) } } } }
  finally { await rtc.close(); await sess.close(); setTimeout(() => process.exit(0), 500).unref() }
}
