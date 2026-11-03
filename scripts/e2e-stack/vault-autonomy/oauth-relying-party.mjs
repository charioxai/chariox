// MP-08 / MP-09 / MP-10 / MP-11 AU-07: the "deployed app" of the OAuth drill. It reads its client id and secret
// from the workspace .env the agent wrote, on every request, and does the authorization-code exchange server side.
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { randomBytes } from 'node:crypto'

export async function startRelyingParty(providerOrigin, envFile) {
  const state = randomBytes(12).toString('hex'), log = []
  let redirectUri = ''
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1')
    const env = await readFile(envFile, 'utf8').catch(() => '')
    const clientId = env.match(/^CLIENT_ID=(.+)$/m)?.[1], clientSecret = env.match(/^CLIENT_SECRET=(.+)$/m)?.[1]
    const reply = (status, body) => { response.writeHead(status, { 'content-type': 'text/html' }); response.end(`<!doctype html><title>relying party</title><h1>${body}</h1>`) }
    if (url.pathname === '/login') {
      if (!clientId) return reply(503, 'no client configured yet')
      response.writeHead(302, { location: `${providerOrigin}/login/oauth/authorize?client_id=${encodeURIComponent(clientId)}&redirect_uri=${encodeURIComponent(redirectUri)}&response_type=code&state=${state}` })
      return response.end()
    }
    if (url.pathname === '/callback') {
      if (url.searchParams.get('state') !== state || !clientId || !clientSecret) { log.push({ exchange: 'refused' }); return reply(400, 'OAuth login failed: bad state or no client') }
      const exchange = await fetch(`${providerOrigin}/login/oauth/access_token`, { method: 'POST', headers: { accept: 'application/json' },
        body: new URLSearchParams({ client_id: clientId, client_secret: clientSecret, code: url.searchParams.get('code') ?? '', grant_type: 'authorization_code', redirect_uri: redirectUri }) })
      const token = await exchange.json().catch(() => ({}))
      if (!token.access_token) { log.push({ exchange: exchange.status, error: String(token.error ?? 'none') }); return reply(401, 'OAuth login failed: code exchange refused') }
      const user = await (await fetch(`${providerOrigin}/api/v1/user`, { headers: { authorization: `Bearer ${token.access_token}` } })).json()
      log.push({ exchange: 200, login: user.login })
      return reply(200, `OAuth login OK: ${user.login}`)
    }
    reply(404, 'not found')
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const origin = `http://127.0.0.1:${server.address().port}`
  redirectUri = `${origin}/callback`
  return { origin, redirectUri, log, stop: () => new Promise(resolve => server.close(resolve)) }
}
