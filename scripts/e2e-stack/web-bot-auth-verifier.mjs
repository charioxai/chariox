// MP-08 / MP-09 / MP-10 / MP-11: local RFC 9421 + Web Bot Auth verifier.
import { createHash, createPublicKey, verify as verifySignature } from 'node:crypto'
import { appendFile } from 'node:fs/promises'
import { createServer } from 'node:http'
import { pathToFileURL } from 'node:url'

const DIRECTORY_PATH = '/.well-known/http-message-signatures-directory'
const DIRECTORY_TYPE = 'application/http-message-signatures-directory+json'
const MAX_BODY_BYTES = 2 * 1024 * 1024
const MAX_SIGNATURE_SECONDS = 24 * 60 * 60
const ED25519_SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex')
const PIXEL = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/n3sAAAAASUVORK5CYII=', 'base64')

export function createWebBotAuthVerifier({ allowedDirectoryUrls = [], allowedDirectoryOrigins = [] } = {}) {
  const pinnedUrls = new Set(allowedDirectoryUrls.map(directoryUrl))
  const pinnedOrigins = new Set(allowedDirectoryOrigins.map(value => new URL(value).origin))
  const usedNonces = new Map()

  return {
    async verifyRequest({ method, url, headers, body = Buffer.alloc(0), scheme = 'http' }) {
      const normalizedHeaders = normalizeHeaders(headers)
      const inputHeader = normalizedHeaders['signature-input']
      const signatureHeader = normalizedHeaders.signature
      const agentHeader = normalizedHeaders['signature-agent']
      if (!inputHeader || !signatureHeader || !agentHeader) return denied('missing_signature')

      try {
        const { label, components, parameters, serializedParameters } = parseSignatureInput(inputHeader)
        if (label !== 'sig1' || parameters.alg !== 'ed25519' || parameters.tag !== 'web-bot-auth') return denied('unsupported_signature')
        const agentUrl = parseSignatureAgent(agentHeader, label)
        const parsedDirectoryUrl = directoryUrl(agentUrl)
        if (!pinnedUrls.has(parsedDirectoryUrl) && !pinnedOrigins.has(new URL(parsedDirectoryUrl).origin)) return denied('untrusted_directory')

        const requestUrl = new URL(url, `${scheme}://${normalizedHeaders.host ?? ''}`)
        if (!['http:', 'https:'].includes(requestUrl.protocol) || requestUrl.username || requestUrl.password || requestUrl.hash) return denied('invalid_target')
        const now = Math.floor(Date.now() / 1000)
        const { created, expires, keyid, nonce } = parameters
        if (!Number.isSafeInteger(created) || !Number.isSafeInteger(expires) || expires <= created || created > now + 60 || expires < now || expires - created > MAX_SIGNATURE_SECONDS) return denied('invalid_signature_time')
        if (typeof nonce !== 'string' || nonce.length < 16 || nonce.length > 256) return denied('invalid_nonce')
        const nonceKey = `${parsedDirectoryUrl}\n${keyid}\n${nonce}`
        pruneNonces(usedNonces, now)
        if (usedNonces.has(nonceKey)) return denied('replayed_nonce')

        const directory = await fetchDirectory(parsedDirectoryUrl)
        const jwk = directory.keys.find(candidate => candidate.kid === keyid)
        if (!validJwk(jwk, now, created, expires)) return denied('unknown_or_inactive_key')

        const required = ['@method', '@target-uri', '@authority', 'signature-agent']
        if (!required.every(name => components.some(component => component.name === name))) return denied('missing_covered_component')
        if (!components.some(component => component.name === '@authority' || component.name === '@target-uri')) return denied('missing_web_bot_auth_component')
        if (components.some(component => !['@method', '@target-uri', '@authority', 'signature-agent', 'content-digest'].includes(component.name))) return denied('unsupported_component')
        if (components.filter(component => component.name === 'signature-agent').some(component => component.key !== label)) return denied('wrong_agent_key')
        if (components.filter(component => component.name === 'content-digest').length > 1) return denied('duplicate_component')

        const lines = []
        for (const component of components) {
          const identifier = component.name === 'signature-agent' ? `"signature-agent";key="${component.key}"` : `"${component.name}"`
          let value
          switch (component.name) {
            case '@method': value = method; break
            case '@target-uri': value = requestUrl.href; break
            case '@authority': value = requestUrl.host; break
            case 'signature-agent': value = sfString(parsedDirectoryUrl); break
            case 'content-digest':
              value = normalizedHeaders['content-digest']
              if (!value || !validContentDigest(value, body)) return denied('content_digest_mismatch')
              break
          }
          if (value === undefined) return denied('missing_component_value')
          lines.push(`${identifier}: ${value}`)
        }
        lines.push(`"@signature-params": ${serializedParameters}`)
        const signatureBytes = parseSignature(signatureHeader, label)
        const publicBytes = Buffer.from(jwk.x, 'base64url')
        const publicKey = createPublicKey({ key: Buffer.concat([ED25519_SPKI_PREFIX, publicBytes]), format: 'der', type: 'spki' })
        if (!verifySignature(null, Buffer.from(lines.join('\n')), publicKey, signatureBytes)) return denied('bad_signature')

        usedNonces.set(nonceKey, expires)
        return { ok: true, reason: 'verified', directoryUrl: parsedDirectoryUrl, kid: keyid }
      } catch (error) {
        return denied(error?.code === 'FETCH_FAILED' ? 'directory_fetch_failed' : 'invalid_signature')
      }
    },
  }
}

export async function startVerifierServer({
  host = '127.0.0.1',
  port = 0,
  allowedDirectoryUrls = [],
  allowedDirectoryOrigins = [],
  eventLog,
  expectedCredential,
} = {}) {
  if (expectedCredential !== undefined && (typeof expectedCredential !== 'string' || !expectedCredential.length)) throw new Error('invalid verifier credential fixture')
  const verifier = createWebBotAuthVerifier({ allowedDirectoryUrls, allowedDirectoryOrigins })
  const counts = { verified: 0, denied: 0, routes: {} }
  let navigationProof
  const server = createServer(async (request, response) => {
    let body
    try {
      body = await readBody(request)
    } catch {
      await record({ event: 'web_bot_auth_verification', outcome: 'denied', route: routeBucket(request.url), method: safeMethod(request.method), status: 413, reason: 'request_too_large' })
      response.writeHead(413, { 'content-type': 'application/json' }).end('{"accepted":false}')
      return
    }
    const result = await verifier.verifyRequest({ method: request.method ?? '', url: request.url ?? '/', headers: request.headers, body, scheme: request.socket.encrypted ? 'https' : 'http' })
    const route = routeBucket(request.url)
    const query = new URL(request.url ?? '/', 'http://verifier.invalid').searchParams
    const credentialAccepted = expectedCredential === undefined || query.get('kind') !== 'credential' || query.get('probe') === expectedCredential
    if (!result.ok || !credentialAccepted) {
      await record({ event: 'web_bot_auth_verification', outcome: 'denied', route, method: safeMethod(request.method), status: 403, reason: result.ok ? 'credential_rejected' : result.reason })
      response.writeHead(403, { 'content-type': 'application/json', 'cache-control': 'no-store' }).end('{"accepted":false}')
      return
    }
    await record({ event: 'web_bot_auth_verification', outcome: 'verified', route, method: safeMethod(request.method), status: 200 })
    if (request.method === 'GET' && request.url === '/') {
      // Only a public, body-free navigation proof is available to the drill.
      // Never retain cookies, authorization, query parameters or tool requests.
      const headers = Object.fromEntries(['host', 'signature-agent', 'signature-input', 'signature']
        .map(name => [name, request.headers[name]]).filter(([, value]) => typeof value === 'string'))
      navigationProof = { method: 'GET', url: '/', headers, body: new Uint8Array(), scheme: request.socket.encrypted ? 'https' : 'http' }
    }
    if (request.url === '/' || request.url?.startsWith('/?')) {
      response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' }).end('<!doctype html><html><head><title>Web Bot Auth verifier</title></head><body><img alt="" src="/assets/pixel.png"><script src="/assets/agent.js"></script><main id="result">ready</main></body></html>')
      return
    }
    if (request.url === '/assets/pixel.png') {
      response.writeHead(200, { 'content-type': 'image/png', 'cache-control': 'no-store' }).end(PIXEL)
      return
    }
    if (request.url === '/assets/agent.js') {
      response.writeHead(200, { 'content-type': 'text/javascript; charset=utf-8', 'cache-control': 'no-store' }).end('fetch("/api", {method:"POST",headers:{"content-type":"application/json"},body:JSON.stringify({probe:"browser-subresource"})}).then(r=>r.json()).then(v=>document.querySelector("#result").textContent=v.accepted?"verified":"denied");')
      return
    }
    response.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' }).end('{"accepted":true}')
  })
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, host, () => { server.removeListener('error', reject); resolve() })
  })
  const address = server.address()
  const actualHost = typeof address === 'object' && address ? address.address : host
  const displayHost = actualHost.includes(':') ? `[${actualHost}]` : actualHost
  const url = `http://${displayHost}:${address.port}`
  let closing
  const record = async event => {
    counts[event.outcome] += 1
    const routeCounts = counts.routes[event.route] ??= { verified: 0, denied: 0 }
    routeCounts[event.outcome] += 1
    if (eventLog) await appendFile(eventLog, `${JSON.stringify(event)}\n`, { mode: 0o600 })
  }
  return {
    url,
    address,
    events: () => ({ ...counts, routes: Object.fromEntries(Object.entries(counts.routes).map(([route, values]) => [route, { ...values }])) }),
    navigationProof: () => navigationProof ? structuredClone(navigationProof) : undefined,
    close: () => {
      if (!closing) closing = new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve({ ...counts, routes: Object.fromEntries(Object.entries(counts.routes).map(([route, values]) => [route, { ...values }])) })))
      return closing
    },
  }
}

function normalizeHeaders(headers) {
  return Object.fromEntries(Object.entries(headers ?? {}).map(([name, value]) => [name.toLowerCase(), Array.isArray(value) ? value.join(', ') : String(value ?? '')]))
}

function parseSignatureInput(value) {
  const start = value.indexOf('=')
  if (start < 1) throw new Error('invalid signature input')
  const label = value.slice(0, start).trim()
  const rest = value.slice(start + 1).trim()
  if (!/^[A-Za-z*][A-Za-z0-9_.*-]*$/.test(label) || rest[0] !== '(') throw new Error('invalid signature input')
  const end = matchingParen(rest)
  const serializedParameters = rest.slice(0, end + 1) + rest.slice(end + 1)
  const components = parseComponents(rest.slice(1, end))
  const parameterParts = splitOutsideQuotes(rest.slice(end + 1).replace(/^;/, ''), ';')
  const parameters = Object.create(null)
  for (const part of parameterParts) {
    const separator = part.indexOf('=')
    if (separator < 1) throw new Error('invalid signature parameter')
    const name = part.slice(0, separator).trim()
    const raw = part.slice(separator + 1).trim()
    if (!name || !raw || Object.hasOwn(parameters, name)) throw new Error('invalid signature parameter')
    if (['created', 'expires'].includes(name)) {
      if (!/^\d+$/.test(raw)) throw new Error('invalid timestamp parameter')
      parameters[name] = Number(raw)
    } else if (['keyid', 'alg', 'nonce', 'tag'].includes(name)) {
      parameters[name] = parseSfString(raw)
    }
  }
  if (!['created', 'expires', 'keyid', 'alg', 'nonce', 'tag'].every(name => parameters[name] !== undefined)) throw new Error('missing required signature parameter')
  return { label, components, parameters, serializedParameters }
}

function parseComponents(value) {
  const components = []
  let rest = value.trim()
  while (rest) {
    const match = /^"([^"\\]+)"(?:;key="([^"\\]+)")?/.exec(rest)
    if (!match) throw new Error('invalid covered component')
    components.push({ name: match[1].toLowerCase(), key: match[2] })
    rest = rest.slice(match[0].length).trimStart()
  }
  if (!components.length || components.some(component => !component.name)) throw new Error('empty component list')
  return components
}

function parseSignatureAgent(value, label) {
  const match = new RegExp(`(?:^|,\\s*)${escapeRegExp(label)}=("(?:[^"\\\\]|\\\\["\\\\])*")`).exec(value)
  if (!match) throw new Error('missing Signature-Agent member')
  return parseSfString(match[1])
}

function parseSignature(value, label) {
  const match = new RegExp(`(?:^|,\\s*)${escapeRegExp(label)}=:([A-Za-z0-9+/]*={0,2}):`).exec(value)
  if (!match) throw new Error('missing Signature member')
  const bytes = Buffer.from(match[1], 'base64')
  if (bytes.length !== 64 || bytes.toString('base64') !== match[1]) throw new Error('invalid Signature bytes')
  return bytes
}

function validJwk(jwk, now, created, expires) {
  if (!jwk || jwk.kty !== 'OKP' || jwk.crv !== 'Ed25519' || jwk.use !== 'sig' || typeof jwk.x !== 'string' || !Number.isSafeInteger(jwk.nbf) || !Number.isSafeInteger(jwk.exp)) return false
  const publicBytes = Buffer.from(jwk.x, 'base64url')
  if (publicBytes.length !== 32 || publicBytes.toString('base64url') !== jwk.x) return false
  const thumbprint = createHash('sha256').update(JSON.stringify({ crv: 'Ed25519', kty: 'OKP', x: jwk.x })).digest('base64url')
  return jwk.kid === thumbprint && jwk.nbf <= now && now < jwk.exp && created >= jwk.nbf && created < jwk.exp && expires <= jwk.exp
}

async function fetchDirectory(url) {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), 3_000)
  try {
    const response = await fetch(url, { redirect: 'manual', cache: 'no-store', signal: controller.signal, headers: { accept: DIRECTORY_TYPE } })
    if (response.status !== 200 || response.headers.get('content-type')?.split(';', 1)[0].trim().toLowerCase() !== DIRECTORY_TYPE) throw Object.assign(new Error('directory fetch rejected'), { code: 'FETCH_FAILED' })
    const value = await response.json()
    if (!value || !Array.isArray(value.keys)) throw Object.assign(new Error('invalid key directory'), { code: 'FETCH_FAILED' })
    return value
  } catch (error) {
    throw Object.assign(new Error('directory fetch failed'), { code: 'FETCH_FAILED', cause: error })
  } finally {
    clearTimeout(timeout)
  }
}

function directoryUrl(value) {
  const url = new URL(value)
  if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password || url.pathname !== DIRECTORY_PATH || url.search || url.hash) throw new Error('invalid directory URL')
  return url.href
}

function validContentDigest(value, body) {
  const match = /^sha-256=:([A-Za-z0-9+/]{43}=):$/.exec(value)
  if (!match) return false
  const expected = createHash('sha256').update(body).digest('base64')
  return match[1] === expected
}

function parseSfString(value) {
  if (!/^"(?:[^"\\]|\\["\\])*"$/.test(value)) throw new Error('invalid structured string')
  return value.slice(1, -1).replace(/\\(["\\])/g, '$1')
}

function sfString(value) { return `"${value.replace(/["\\]/g, '\\$&')}"` }
function matchingParen(value) { let quoted = false; let escaped = false; for (let i = 0; i < value.length; i++) { const ch = value[i]; if (escaped) { escaped = false; continue } if (quoted && ch === '\\') { escaped = true; continue } if (ch === '"') quoted = !quoted; else if (!quoted && ch === ')') return i } throw new Error('unclosed component list') }
function splitOutsideQuotes(value, separator) { const parts = []; let quoted = false; let escaped = false; let start = 0; for (let i = 0; i < value.length; i++) { const ch = value[i]; if (escaped) { escaped = false; continue } if (quoted && ch === '\\') { escaped = true; continue } if (ch === '"') quoted = !quoted; else if (!quoted && ch === separator) { parts.push(value.slice(start, i).trim()); start = i + 1 } } parts.push(value.slice(start).trim()); return parts.filter(Boolean) }
function pruneNonces(nonces, now) { for (const [key, expires] of nonces) if (expires < now) nonces.delete(key) }
function denied(reason) { return { ok: false, reason } }
function safeMethod(method) { return typeof method === 'string' && /^[A-Z]{1,16}$/.test(method) ? method : 'OTHER' }
function routeBucket(url = '') { if (url === '/' || url.startsWith('/?')) return '/'; if (url.startsWith('/assets/')) return '/assets/*'; if (url.startsWith('/api')) return '/api'; return '/other' }
function readBody(request) { return new Promise((resolve, reject) => { const chunks = []; let size = 0; request.on('data', chunk => { size += chunk.length; if (size > MAX_BODY_BYTES) { reject(new Error('request body too large')); request.destroy(); return } chunks.push(chunk) }); request.on('end', () => resolve(Buffer.concat(chunks))); request.on('error', reject) }) }
function escapeRegExp(value) { return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') }

function cliOptions(argv) {
  const result = { allowedDirectoryUrls: [] }
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i]
    const value = argv[++i]
    if (!value) throw new Error(`missing value for ${flag}`)
    if (flag === '--host') result.host = value
    else if (flag === '--port') result.port = Number(value)
    else if (flag === '--directory-url') result.allowedDirectoryUrls.push(value)
    else if (flag === '--event-log') result.eventLog = value
    else throw new Error(`unknown option ${flag}`)
  }
  if (!Number.isInteger(result.port ?? 0) || (result.port ?? 0) < 0 || (result.port ?? 0) > 65535) throw new Error('port must be between 0 and 65535')
  if (!result.allowedDirectoryUrls.length) throw new Error('at least one --directory-url is required')
  return result
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const running = await startVerifierServer(cliOptions(process.argv.slice(2)))
  process.stdout.write(`web-bot-auth-verifier listening at ${running.url}\n`)
  const stop = async () => { const counts = await running.close(); process.stdout.write(`${JSON.stringify({ event: 'web_bot_auth_counts', ...counts })}\n`); process.exit(0) }
  process.once('SIGINT', stop)
  process.once('SIGTERM', stop)
}
