// LOCALFAST forged-client harness (test tool, row 353). A Node client that does what the Cloud web page does to open a
// same-machine direct carrier to a kernel — real Cloud sign-in, real 30 s relay lease token, real encrypted
// local_browser_connect over the real relay, then the loopback WebSocket or WebRTC data-channel handshake — and lets a cell
// forge exactly one field of it. Every forged cell runs next to an unforged control so a denial is the forged field's doing.
import { createRequire } from 'node:module'
import { pathToFileURL } from 'node:url'
import { createHash, randomBytes } from 'node:crypto'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const here = dirname(fileURLToPath(import.meta.url))
export const repo = resolve(here, '../..')
const kc = `${repo}/packages/kernel-client`
export const WebSocket = createRequire(`${kc}/package.json`)('ws')
const crypto = await import(pathToFileURL(`${kc}/dist/relay-crypto.js`).href)
export const { RelayClientIdentity, createRelayKeypair, relayPublicKeyThumbprint } = crypto
export const sleep = ms => new Promise(r => setTimeout(r, ms))
export const newIdentity = () => new RelayClientIdentity(createRelayKeypair().privateKey)

/** A signed-in Cloud session of the stack's fixture owner (the page's own cookies; the harness calls the same API routes). */
export async function cloudSession({ stack, playwrightCore = '/w/valenv/tools/node_modules/playwright-core/index.mjs', signIn = '/w/valenv/scripts/sign-in.mjs', chrome = '/opt/google/chrome/chrome' }) {
  const meta = JSON.parse((await import('node:fs')).readFileSync(`/w/valenv/prs/${stack}/metadata.json`, 'utf8'))
  const origin = `https://${meta.hostname}`
  const { chromium } = await import(playwrightCore)
  const { signInValidation } = await import(signIn)
  const browser = await chromium.launch({ executablePath: chrome, headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] })
  const ctx = await browser.newContext()
  const page = await ctx.newPage()
  await signInValidation(page, origin, '/terminal?ui=os')
  const api = ctx.request
  const csrf = async () => (await (await api.get(`${origin}/auth/csrf`)).json()).csrfToken
  const post = async (path, body, extra = {}) => {
    const r = await api.post(`${origin}${path}`, { data: body, headers: { 'csrf-token': await csrf(), ...extra } })
    const text = await r.text(); let json = null; try { json = JSON.parse(text) } catch {}
    return { status: r.status(), json, text: text.slice(0, 300) }
  }
  const cs = await post('/auth/cloud-session', {})
  const token = cs.json?.cloudSessionToken
  const headers = token ? { 'x-chariox-browser-session': token } : {}
  return {
    origin, browser, ctx, page, meta,
    bootstrap: (daemon, thumb) => post('/browser/relay-kernel/bootstrap', { targetDaemonId: daemon, publicKeyThumbprint: thumb }, headers),
    lease: (daemon, thumb) => post('/browser/relay-kernel/local-lease', { targetDaemonId: daemon, publicKeyThumbprint: thumb }, headers),
    /** A second signed-in browser context (own storage, own tab keys). */
    async freshPage() { const c = await browser.newContext({ viewport: { width: 1440, height: 900 } }); const pg = await c.newPage(); await signInValidation(pg, origin, '/terminal?ui=os'); return { ctx: c, page: pg } },
    close: () => browser.close().catch(() => {}),
  }
}

/** One encrypted request/response exchange through the real relay carrying `token` (a Cloud-issued client identity). */
export function relayCall({ relayUrl, token, target, identity, kernelKey, bodies, timeoutMs = 8000 }) {
  return new Promise((res) => {
    const out = { frames: [], responses: [], error: null }
    const ws = new WebSocket(relayUrl, { handshakeTimeout: timeoutMs })
    const done = (why) => { clearTimeout(timer); try { ws.terminate() } catch {} ; if (why) out.error ??= why; res(out) }
    const timer = setTimeout(() => done('timeout'), timeoutMs)
    ws.on('error', e => done('socket error ' + e.message))
    ws.on('close', () => done(null))
    ws.on('open', () => ws.send(JSON.stringify({ kind: 'client_connect', auth_token: token, target })))
    ws.on('message', data => {
      const f = JSON.parse(String(data)); out.frames.push(f.kind)
      if (f.kind === 'client_connected') {
        out.kernelKeyFromRelay = f.daemon_public_key
        const key = kernelKey ?? f.daemon_public_key
        bodies.forEach((b, i) => ws.send(JSON.stringify({ kind: 'client_request', request_id: String(i), target,
          encrypted_request: identity.encrypt(key, JSON.stringify(b)) })))
      } else if (f.kind === 'client_response') {
        let body = null
        if (f.encrypted_response) { try { body = JSON.parse(identity.decrypt(f.encrypted_response)) } catch { body = { undecryptable: true } } }
        out.responses[Number(f.request_id)] = { error: f.error ?? null, body }
        if (out.responses.filter(Boolean).length === bodies.length) done(null)
      } else if (f.kind === 'close' || f.kind === 'error') { out.error ??= `${f.kind}: ${f.reason ?? f.code ?? ''}`; done(null) }
    })
  })
}

/** A framed carrier: next() resolves the next JSON frame or {closed}. */
export function wsCarrier(url, { headers = {}, timeoutMs = 6000 } = {}) {
  return new Promise((res) => {
    const q = []; const waiters = []; let closed = null
    const push = f => { const w = waiters.shift(); if (w) w(f); else q.push(f) }
    const ws = new WebSocket(url, { headers, handshakeTimeout: timeoutMs, followRedirects: false })
    const carrier = {
      kind: 'ws', ws,
      send: o => ws.send(JSON.stringify(o)),
      sendRaw: s => ws.send(s),
      next: (ms = 6000) => new Promise(r => { if (q.length) return r(q.shift()); if (closed) return r(closed); const t = setTimeout(() => { const i = waiters.indexOf(w); if (i >= 0) waiters.splice(i, 1); r({ timeout: true }) }, ms); const w = f => { clearTimeout(t); r(f) }; waiters.push(w) }),
      close: () => { try { ws.terminate() } catch {} },
      get closed() { return closed },
    }
    ws.on('message', d => { try { push(JSON.parse(String(d))) } catch { push({ unparsable: String(d).slice(0, 80) }) } })
    ws.on('unexpected-response', (_req, r) => { closed = { refused: r.statusCode }; res({ refused: r.statusCode }); push(closed) })
    ws.on('error', e => { closed ??= { closed: e.message }; if (!carrier.opened) res({ refused: 0, error: e.message }) })
    ws.on('close', (code) => { closed ??= { closed: code }; while (waiters.length) waiters.shift()(closed) })
    ws.on('open', () => { carrier.opened = true; res(carrier) })
  })
}

/** Reads the kernel's challenge, answers with a (possibly forged) proof, returns the verdict frame. */
export async function prove(carrier, { grant, kernelKey, identity, origin, mutate = p => p, wait = 6000 }) {
  const ch = await carrier.next(wait)
  if (ch.kind !== 'local_challenge') return { stage: 'challenge', frame: ch }
  const payload = mutate({ grant, challenge: ch.challenge, origin, kernel_id: ch.kernel_id, endpoint_epoch: ch.endpoint_epoch }, ch)
  carrier.send({ kind: 'local_connect', proof: identity.encrypt(kernelKey, JSON.stringify(payload)) })
  const verdict = await carrier.next(wait)
  return { stage: 'verdict', challenge: ch, frame: verdict }
}

export const connectedOk = v => v?.frame?.kind === 'local_connected'
export const refusedReason = v => v?.frame?.reason ?? (v?.frame?.closed !== undefined ? `closed:${v.frame.closed}` : v?.frame?.timeout ? 'timeout' : v?.frame?.kind ?? JSON.stringify(v.frame))

/** After admission, one request over the direct carrier; returns the decrypted body or the refusal. */
export async function directRequest(carrier, { identity, kernelKey, target, body, id = 'r' + randomBytes(3).toString('hex'), wait = 8000 }) {
  carrier.send({ kind: 'client_request', request_id: id, target, encrypted_request: identity.encrypt(kernelKey, JSON.stringify(body)) })
  for (;;) {
    const f = await carrier.next(wait)
    if (f.kind === 'client_response' && f.request_id === id) return { error: f.error ?? null, body: f.encrypted_response ? JSON.parse(identity.decrypt(f.encrypted_response)) : null }
    if (f.kind === 'client_event' || f.kind === 'ping') continue
    return { frame: f }
  }
}

export const sha = s => createHash('sha256').update(s).digest('hex').slice(0, 12)

/** WebRTC engine: a blank Chromium page (no Cloud origin) builds the offer and runs the data channel; the harness forges the
 * handshake bytes. Channel framing: one kind byte (0 last fragment, 1 more, 2 ping, 3 pong) then up to 16 KiB of UTF-8 text. */
export async function rtcEngine({ playwrightCore = '/w/valenv/tools/node_modules/playwright-core/index.mjs', chrome = '/opt/google/chrome/chrome' } = {}) {
  const { chromium } = await import(playwrightCore)
  const browser = await chromium.launch({ executablePath: chrome, headless: true,
    args: ['--no-sandbox', '--disable-dev-shm-usage', '--disable-features=WebRtcHideLocalIpsWithMdns'] })
  return {
    browser,
    close: () => browser.close().catch(() => {}),
    /** Step 1: an offer SDP. Step 2 (after the kernel's answer): returns the open carrier. */
    async offer() {
      const ctx = await browser.newContext(); const page = await ctx.newPage(); await page.goto('about:blank')
      const q = []; const waiters = []; let closed = null; let partial = ''
      const push = f => { const w = waiters.shift(); if (w) w(f); else q.push(f) }
      await page.exposeBinding('fcRx', (_s, b64) => {
        const buf = Buffer.from(b64, 'base64'); const kind = buf[0]
        if (kind === 2 || kind === 3) return
        partial += buf.subarray(1).toString('utf8')
        if (kind === 0) { const t = partial; partial = ''; try { push(JSON.parse(t)) } catch { push({ unparsable: t.slice(0, 80) }) } }
      })
      await page.exposeBinding('fcClosed', () => { closed ??= { closed: 'channel' }; while (waiters.length) waiters.shift()(closed) })
      const sdp = await page.evaluate(async () => {
        const pc = new RTCPeerConnection({ iceServers: [] }); const dc = pc.createDataChannel('chariox', { ordered: true }); dc.binaryType = 'arraybuffer'
        window.__fc = { pc, dc }
        dc.onmessage = e => { const u = new Uint8Array(e.data); if (u[0] === 2) { dc.send(new Uint8Array([3, ...u.subarray(1)])); return }
          let s = ''; for (let i = 0; i < u.length; i += 8192) s += String.fromCharCode(...u.subarray(i, i + 8192)); window.fcRx(btoa(s)) }
        dc.onclose = () => window.fcClosed()
        await pc.setLocalDescription(await pc.createOffer())
        await new Promise(r => { if (pc.iceGatheringState === 'complete') return r(); const t = setTimeout(r, 3000); pc.addEventListener('icegatheringstatechange', () => { if (pc.iceGatheringState === 'complete') { clearTimeout(t); r() } }) })
        return pc.localDescription.sdp
      })
      const carrier = {
        kind: 'rtc', page,
        send: o => carrier.sendRaw(JSON.stringify(o)),
        sendRaw: async s => { const b = Buffer.from(s, 'utf8'); const parts = []; for (let i = 0; i < Math.max(1, b.length); i += 16384) parts.push(b.subarray(i, i + 16384))
          for (const [i, p] of parts.entries()) { const msg = Buffer.concat([Buffer.from([i === parts.length - 1 ? 0 : 1]), p]); await page.evaluate(h => window.__fc.dc.send(Uint8Array.from(atob(h), c => c.charCodeAt(0))), msg.toString('base64')) } },
        next: (ms = 6000) => new Promise(r => { if (q.length) return r(q.shift()); if (closed) return r(closed); const t = setTimeout(() => { const i = waiters.indexOf(w); if (i >= 0) waiters.splice(i, 1); r({ timeout: true }) }, ms); const w = f => { clearTimeout(t); r(f) }; waiters.push(w) }),
        close: () => ctx.close().catch(() => {}),
        get closed() { return closed },
        /** Applies the kernel's answer; resolves true when the channel opens. */
        async accept(answer, ms = 8000) {
          return page.evaluate(async ({ answer, ms }) => { const { pc, dc } = window.__fc
            await pc.setRemoteDescription({ type: 'answer', sdp: answer })
            return await new Promise(r => { if (dc.readyState === 'open') return r(true); const t = setTimeout(() => r(false), ms); dc.onopen = () => { clearTimeout(t); r(true) } }) }, { answer, ms })
        },
      }
      return { sdp, carrier }
    },
  }
}
