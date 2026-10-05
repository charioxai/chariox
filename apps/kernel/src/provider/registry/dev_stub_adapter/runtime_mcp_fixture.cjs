// MP-11: explicit private dev-stub fixture. Bindings stay inside the provider process.
const runtimeUrl = process.env.CHARIOX_DEV_STUB_RUNTIME_MCP_URL
const runtimeToken = process.env.CHARIOX_DEV_STUB_RUNTIME_MCP_TOKEN
delete process.env.CHARIOX_DEV_STUB_RUNTIME_MCP_URL
delete process.env.CHARIOX_DEV_STUB_RUNTIME_MCP_TOKEN
const secretForms = runtimeToken ? [runtimeToken, encodeURIComponent(runtimeToken),
  Buffer.from(runtimeToken).toString('base64'), Buffer.from(runtimeToken).toString('base64url')] : []
const { createHash } = require('node:crypto')
let lastInputDigest = null
let buffered = ''
let pending = Promise.resolve()
function safe(value) {
  let text = JSON.stringify(value)
  for (const secret of secretForms) text = text.split(secret).join('[REDACTED]')
  return text
}
async function call(request) {
  if (request.method === 'fixture/input-digest') return { result: { sha256: lastInputDigest } }
  if (request.method === 'fixture/env-absent') {
    const names = request.params?.names
    if (!Array.isArray(names) || names.length > 16 || names.some(name => !/^CHARIOX_[A-Z0-9_]+$/.test(name))) throw Error('invalid fixture environment check')
    return { result: { absent: Object.fromEntries(names.map(name => [name, !Object.hasOwn(process.env, name)])) } }
  }
  if (!runtimeUrl || !runtimeToken) throw Error('private fixture runtime unavailable')
  if (!['tools/list', 'tools/call'].includes(request.method)) throw Error('unsupported fixture method')
  const url = new URL(runtimeUrl)
  if (request.proxy) {
    if (!/^[A-Za-z0-9_.-]+$/.test(request.proxy)) throw Error('invalid fixture proxy')
    url.pathname = url.pathname.replace(/\/mcp\/?$/, `/mcp/proxy/${request.proxy}`)
  }
  const response = await fetch(url, { method: 'POST', headers: {
    authorization: `Bearer ${runtimeToken}`, 'content-type': 'application/json',
  }, body: JSON.stringify({ jsonrpc: '2.0', id: request.id, method: request.method, params: request.params ?? {} }),
    signal: AbortSignal.timeout(90000) })
  let size = 0
  const parts = []
  for await (const chunk of response.body) {
    size += chunk.length
    if (size > 8 * 1024 * 1024) throw Error('fixture response exceeded limit')
    parts.push(chunk)
  }
  const value = JSON.parse(Buffer.concat(parts).toString('utf8'))
  if (!response.ok && !value.error) throw Error('fixture runtime rejected request')
  return value
}
async function handle(line) {
  const marker = line.indexOf('MP11_MCP_REQUEST ')
  if (marker < 0) {
    // MP-11: prove terminal handoff receipt without echoing the input.
    if (line) lastInputDigest = createHash('sha256').update(line).digest('hex')
    return
  }
  const parts = line.slice(marker).trim().split(' ')
  const id = parts[1]
  if (!/^[a-f0-9-]{36}$/.test(id ?? '')) return
  let value
  try {
    const request = JSON.parse(Buffer.from(parts[2], 'base64').toString('utf8'))
    value = await call({ ...request, id })
  } catch {
    value = { error: { message: 'private provider MCP fixture request failed' } }
  }
  process.stdout.write(`MP11_MCP_RESULT ${id} ${Buffer.from(safe(value)).toString('base64')}\n`)
}
process.stdin.setEncoding('utf8')
process.stdin.on('data', chunk => {
  buffered += chunk.replace(/\r/g, '\n')
  if (buffered.length > 2 * 1024 * 1024) buffered = ''
  let end
  while ((end = buffered.indexOf('\n')) >= 0) {
    const line = buffered.slice(0, end)
    buffered = buffered.slice(end + 1)
    pending = pending.then(() => handle(line)).catch(() => {})
  }
})
process.stdout.write('MP11_MCP_FIXTURE_READY\n')
process.stdin.resume()
