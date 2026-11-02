// MP-08 / MP-09 / MP-10 / MP-11 AU-15: local WorkOS auth.md protocol service.
// Credentials live only in memory; diagnostics contain counts and HTTP status.
import http from 'node:http'
import { randomBytes, createHmac } from 'node:crypto'
import { pathToFileURL } from 'node:url'

const opaque = prefix => prefix + randomBytes(24).toString('base64url')
export async function createAuthService({ identityOnly = false, expiresIn = 120 } = {}) {
  const registrations = new Map(), assertions = new Map(), tokens = new Map(), attempts = new Map()
  const signingKey = randomBytes(32)
  const privateValues = []
  const counts = { registrations: 0, polls: 0, claims: 0, accepted: 0 }
  let origin
  function assertion(id) {
    const encoded = value => Buffer.from(JSON.stringify(value)).toString('base64url')
    const body = encoded({ alg: 'HS256', typ: 'JWT' }) + '.' + encoded({ iss: origin, sub: id, aud: `${origin}/`, iat: Math.floor(Date.now() / 1000), exp: Math.floor(Date.now() / 1000) + 3600, jti: opaque('jwt-') })
    return body + '.' + createHmac('sha256', signingKey).update(body).digest('base64url')
  }
  function issue(record, claimed) {
    const token = opaque('access-')
    privateValues.push(token)
    tokens.set(token, { record, claimed })
    return { access_token: token, token_type: 'Bearer', expires_in: 3600, scope: claimed ? 'api.read api.write' : 'api.read' }
  }
  function ceremony(record, email) {
    const attempt = opaque('attempt-'), code = String(100000 + randomBytes(4).readUInt32BE() % 900000)
    record.email = email
    record.expires = Date.now() + expiresIn * 1000
    attempts.set(attempt, { record, code })
    return { user_code: code, verification_uri: `${origin}/claim?attempt=${attempt}`, expires_in: expiresIn, interval: 1 }
  }
  const server = http.createServer(async (request, response) => {
    function reply(status, body, type = 'application/json') {
      response.writeHead(status, { 'content-type': type, 'cache-control': 'no-store' })
      response.end(type === 'application/json' ? JSON.stringify(body) : body)
    }
    try {
      const url = new URL(request.url, origin)
      let body = ''
      for await (const chunk of request) {
        body += chunk
        if (body.length > 65536) { reply(413, { error: 'invalid_request' }); return }
      }
      const input = body ? (request.headers['content-type']?.includes('application/json') ? JSON.parse(body) : Object.fromEntries(new URLSearchParams(body))) : {}
      if (request.method === 'GET' && url.pathname === '/.well-known/oauth-protected-resource') {
        reply(200, { resource: `${origin}/`, resource_name: 'Local auth.md drill', authorization_servers: [origin], scopes_supported: ['api.read', 'api.write'], bearer_methods_supported: ['header'] })
      } else if (request.method === 'GET' && url.pathname === '/.well-known/oauth-authorization-server') {
        reply(200, { issuer: origin, token_endpoint: `${origin}/oauth2/token`, grant_types_supported: ['urn:ietf:params:oauth:grant-type:jwt-bearer', 'urn:workos:agent-auth:grant-type:claim'], agent_auth: {
          skill: `${origin}/auth.md`, identity_endpoint: `${origin}/agent/identity`, claim_endpoint: `${origin}/agent/identity/claim`, identity_types_supported: identityOnly ? ['identity_assertion'] : ['anonymous', 'service_auth'],
        } })
      } else if (request.method === 'GET' && url.pathname === '/auth.md') {
        reply(200, '# auth.md\nDiscover /.well-known/oauth-protected-resource and the AS metadata. POST /agent/identity with type anonymous, or type service_auth and login_hint. Exchange identity_assertion at /oauth2/token using the JWT bearer grant. For claims show user_code and verification_uri, then poll the claim grant with claim_token. Use the resulting Bearer credential at /api/check.\n', 'text/markdown')
      } else if (request.method === 'POST' && url.pathname === '/agent/identity') {
        if (identityOnly || !['anonymous', 'service_auth'].includes(input.type)) { reply(400, { error: `${input.type}_not_enabled` }); return }
        if (input.type === 'service_auth' && !input.login_hint) { reply(400, { error: 'invalid_request' }); return }
        const id = opaque('reg-')
        const record = { id, assertion: assertion(id), claimToken: opaque('claim-'), claimed: false }
        privateValues.push(record.assertion, record.claimToken)
        registrations.set(record.claimToken, record)
        assertions.set(record.assertion, record)
        counts.registrations++
        const result = { registration_id: record.id, registration_type: input.type, claim_token: record.claimToken, claim_token_expires: new Date(Date.now() + 600000).toISOString(), post_claim_scopes: ['api.read', 'api.write'] }
        if (input.type === 'service_auth') result.claim = ceremony(record, input.login_hint)
        else Object.assign(result, { identity_assertion: record.assertion, assertion_expires: new Date(Date.now() + 3600000).toISOString(), pre_claim_scopes: ['api.read'] })
        reply(200, result)
      } else if (request.method === 'POST' && url.pathname === '/agent/identity/claim') {
        const record = registrations.get(input.claim_token)
        if (!record || !input.email) { reply(400, { error: 'invalid_claim_token' }); return }
        reply(200, { registration_id: record.id, status: 'initiated', claim_attempt: ceremony(record, input.email) })
      } else if (request.method === 'GET' && url.pathname === '/claim') {
        const attempt = attempts.get(url.searchParams.get('attempt'))
        if (!attempt) { reply(404, { error: 'not_found' }); return }
        reply(200, '<!doctype html><title>Local account claim</title><form method="post" action="/claim/complete"><label>Account email <input name="email" type="email" required></label><label>Code <input name="user_code" inputmode="numeric" required></label><input type="hidden" name="attempt" value="' + url.searchParams.get('attempt') + '"><button>Claim account</button></form>', 'text/html')
      } else if (request.method === 'POST' && url.pathname === '/claim/complete') {
        const attempt = attempts.get(input.attempt)
        if (!attempt || attempt.code !== input.user_code || attempt.record.email !== input.email || Date.now() >= attempt.record.expires) { reply(400, { error: 'invalid_request' }); return }
        attempt.record.claimed = true
        for (const [token, entry] of tokens) if (entry.record === attempt.record) tokens.delete(token)
        counts.claims++
        reply(200, '<!doctype html><p>Account claimed.</p>', 'text/html')
      } else if (request.method === 'POST' && url.pathname === '/oauth2/token') {
        if (input.grant_type === 'urn:workos:agent-auth:grant-type:claim') {
          counts.polls++
          const record = registrations.get(input.claim_token)
          if (!record) { reply(400, { error: 'invalid_claim_token' }); return }
          if (Date.now() >= record.expires) { reply(400, { error: 'expired_token' }); return }
          if (!record.claimed) { reply(400, { error: 'authorization_pending' }); return }
          reply(200, { ...issue(record, true), identity_assertion: record.assertion, assertion_expires: new Date(Date.now() + 3600000).toISOString() })
        } else if (input.grant_type === 'urn:ietf:params:oauth:grant-type:jwt-bearer') {
          const record = assertions.get(input.assertion)
          if (!record || input.resource !== `${origin}/`) { reply(400, { error: 'invalid_grant' }); return }
          reply(200, issue(record, false))
        } else reply(400, { error: 'unsupported_grant_type' })
      } else if (request.method === 'GET' && url.pathname === '/api/check') {
        if (!tokens.has(request.headers.authorization?.replace(/^Bearer /, ''))) { reply(401, { error: 'invalid_token' }); return }
        counts.accepted++
        reply(200, { accepted: true })
      } else reply(404, { error: 'not_found' })
    } catch { reply(400, { error: 'invalid_request' }) }
  })
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve) })
  origin = `http://127.0.0.1:${server.address().port}`
  return { origin, counts, exposureKind(text) {
    const value = privateValues.find(value => text.includes(value))
    return value ? value.startsWith('access-') ? 'service access credential' : value.startsWith('claim-') ? 'service claim token' : 'service identity assertion' : null
  }, assertAbsent(text) {
    if (privateValues.some(value => text.includes(value))) throw new Error('Service credential appeared in model-visible evidence')
  }, async close() { signingKey.fill(0); await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve())) } }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const service = await createAuthService({ identityOnly: process.argv.includes('--identity-only') })
  console.log(JSON.stringify({ MP: ['MP-08', 'MP-09', 'MP-10', 'MP-11'], origin: service.origin }))
  for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, async () => { await service.close(); process.exitCode = 0 })
}
