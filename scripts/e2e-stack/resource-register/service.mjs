// MP-08 / MP-09 / MP-10 / MP-11 AU-11: disposable local service for the development drill.
import { createServer } from 'node:http'
import { randomBytes, randomUUID } from 'node:crypto'

export async function startKeyService() {
  const keys = new Map(), resources = new Map(), events = []
  function issue() {
    const key = { id: randomUUID(), value: randomBytes(32).toString('hex'), active: true }
    keys.set(key.value, key)
    return key
  }
  const server = createServer((request, response) => {
    const key = keys.get((request.headers.authorization ?? '').replace(/^Bearer /, ''))
    const send = (status, body) => {
      events.push({ method: request.method, path: request.url, status, key_id: key?.id ?? null })
      response.writeHead(status, { 'content-type': 'application/json' }).end(JSON.stringify(body))
    }
    if (!key?.active) return send(401, { error: 'key revoked or unknown' })
    if (request.method === 'GET' && request.url === '/check') return send(200, { authenticated: true, key_id: key.id })
    if (request.method === 'POST' && request.url === '/resources') {
      const id = randomUUID()
      resources.set(id, key.id)
      return send(201, { id, kind: 'api_key', name: 'deploy key', revoke_path: `/keys/${key.id}` })
    }
    if (request.method === 'DELETE' && request.url === `/keys/${key.id}`) {
      key.active = false
      // Deliberately echo the revoked value: the kernel's register tool must discard the body.
      return send(200, { revoked: true, echo: key.value })
    }
    send(404, { error: 'resource not found' })
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const origin = `http://127.0.0.1:${server.address().port}`
  return { origin, issue, events, resources, close: () => new Promise(resolve => server.close(resolve)),
    async check(key) { return (await fetch(`${origin}/check`, { headers: { authorization: `Bearer ${key.value}` } })).status } }
}
