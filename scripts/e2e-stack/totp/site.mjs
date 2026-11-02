import { createHmac, randomBytes, timingSafeEqual } from 'node:crypto'
import http from 'node:http'

const BASE32 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'
const COOKIE = 'totp_session'

function base32(bytes) {
  let bits = 0, value = 0, out = ''
  for (const byte of bytes) {
    value = (value << 8) | byte
    bits += 8
    while (bits >= 5) { out += BASE32[(value >>> (bits - 5)) & 31]; bits -= 5 }
  }
  if (bits) out += BASE32[(value << (5 - bits)) & 31]
  return out
}

function hotp(seed, counter) {
  const digest = createHmac('sha1', seed).update(Buffer.from(BigInt(counter).toString(16).padStart(16, '0'), 'hex')).digest()
  const offset = digest[digest.length - 1] & 15
  return String((digest.readUInt32BE(offset) & 0x7fffffff) % 1_000_000).padStart(6, '0')
}

export function totpAt(seed, timestampMs) { return hotp(seed, Math.floor(timestampMs / 30_000)) }

function matches(seed, code, timestampMs, lastCounter) {
  if (!/^\d{6}$/.test(code)) return null
  const counter = Math.floor(timestampMs / 30_000)
  for (const candidate of [counter - 1, counter, counter + 1]) {
    if (candidate <= lastCounter) continue
    const expected = Buffer.from(hotp(seed, candidate))
    const supplied = Buffer.from(code)
    if (timingSafeEqual(expected, supplied)) return candidate
  }
  return null
}

function page(body) { return `<!doctype html><meta charset="utf-8"><title>TOTP test site</title>${body}` }
function form(action, fields) { return `<form method="post" action="/${action}">${fields}<button>${action}</button></form>` }

export async function startTotpSite({ now = () => Date.now() } = {}) {
  const seed = randomBytes(20)
  const seedText = base32(seed)
  const sessions = new Map()
  let lastAcceptedCounter = -1
  let enabled = false
  const injectedValues = new Set([seedText])
  const stats = { enrollmentSuccesses: 0, loginSuccesses: 0, rejectedCodes: 0, replayRejections: 0, backupSuccesses: 0 }
  const backupCodes = new Set(Array.from({ length: 8 }, () => randomBytes(6).toString('hex')))
  const parseCookies = (header = '') => Object.fromEntries(header.split(';').map(part => part.trim().split('=').map(decodeURIComponent)))
  const send = (res, status, body, cookie) => {
    res.writeHead(status, { 'content-type': 'text/html; charset=utf-8', ...(cookie ? { 'set-cookie': cookie } : {}) })
    res.end(body)
  }
  const readBody = req => new Promise((resolve, reject) => {
    let body = ''
    req.setEncoding('utf8')
    req.on('data', chunk => { body += chunk; if (body.length > 4096) req.destroy(new Error('request too large')) })
    req.on('end', () => resolve(new URLSearchParams(body)))
    req.on('error', reject)
  })
  const server = http.createServer(async (req, res) => {
    try {
      const sessionId = parseCookies(req.headers.cookie)[COOKIE]
      let session = sessions.get(sessionId)
      if (req.method === 'GET' && req.url === '/') return send(res, 200, session?.authenticated
        ? page('<h1>Signed in</h1><a href="/logout">Log out</a>')
        : page('<h1>Test account</h1><a href="/enable">Enable 2FA</a> <a href="/login">Sign in</a>'))
      if (req.method === 'GET' && req.url === '/enable') {
        const id = randomBytes(24).toString('hex')
        sessions.set(id, { authenticated: true, enrolled: false, lastCounter: -1 })
        return send(res, 200, page(`<h1>Enable two-factor authentication</h1><p>Ask the account owner to enter the authenticator setup key in your authenticator app.</p><label>Authentication code <input name="code" form="enable-form" autocomplete="one-time-code"></label><form id="enable-form" method="post" action="/enable"><button>enable</button></form>`), `${COOKIE}=${id}; HttpOnly; SameSite=Strict; Path=/`)
      }
      if (req.method === 'POST' && req.url === '/enable') {
        const body = await readBody(req)
        const counter = matches(seed, body.get('code') ?? '', now(), lastAcceptedCounter)
        if (counter === null) { stats.rejectedCodes++; return send(res, 401, page('<p>Invalid or replayed code</p><a href="/enable">Try again</a>')) }
        if (!session) return send(res, 401, page('<p>Enrollment session expired</p>'))
        enabled = true; injectedValues.add(body.get('code')); session.enrolled = true; session.lastCounter = counter; lastAcceptedCounter = counter; stats.enrollmentSuccesses++
        return send(res, 200, page('<h1>Two-factor authentication enabled</h1><a href="/logout">Log out</a>'))
      }
      if (req.method === 'GET' && req.url === '/logout') {
        sessions.delete(sessionId)
        return send(res, 200, page('<h1>Signed out</h1><a href="/login">Sign in</a>'), `${COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0`)
      }
      if (req.method === 'GET' && req.url === '/login') return send(res, 200, page(`<h1>Sign in</h1>${form('login', '<label>Authentication code <input name="code" autocomplete="one-time-code"></label><label>Backup code <input name="backup"></label>')}`))
      if (req.method === 'POST' && req.url === '/login') {
        const body = await readBody(req)
        if (!enabled) return send(res, 403, page('<p>Enable 2FA before signing in</p>'))
        let usedBackup = false
        let counter = null
        const backup = body.get('backup') ?? ''
        if (backup && backupCodes.delete(backup)) usedBackup = true
        else counter = matches(seed, body.get('code') ?? '', now(), lastAcceptedCounter)
        if (!usedBackup && counter === null) {
          stats.rejectedCodes++
          if ([now() - 60_000, now() - 30_000, now(), now() + 30_000].some(time => (body.get('code') ?? '') === totpAt(seed, time))) stats.replayRejections++
          return send(res, 401, page('<p>Invalid or replayed code</p><a href="/login">Try again</a>'))
        }
        const id = randomBytes(24).toString('hex')
        injectedValues.add(usedBackup ? backup : body.get('code'))
        if (!usedBackup) lastAcceptedCounter = counter
        sessions.set(id, { authenticated: true, enrolled: true, lastCounter: counter ?? -1 })
        if (usedBackup) stats.backupSuccesses++; else stats.loginSuccesses++
        return send(res, 200, page('<h1>Signed in</h1><a href="/logout">Log out</a>'), `${COOKIE}=${id}; HttpOnly; SameSite=Strict; Path=/`)
      }
      send(res, 404, page('<h1>Not found</h1>'))
    } catch { send(res, 400, page('<p>Invalid request</p>')) }
  })
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve) })
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    close: () => new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve())),
    stats: () => ({ ...stats }),
    // The caller may present this value in an owner-controlled setup interaction; never log or persist it.
    supplySeedToOwner: () => seedText,
    backupCodeForOwner: () => [...backupCodes][0],
    checkAbsence: text => [...injectedValues].every(value => !text.includes(value)),
  }
}
