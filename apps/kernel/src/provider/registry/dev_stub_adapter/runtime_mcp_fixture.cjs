// MP-11: explicit private dev-stub fixture. Bindings stay inside the provider process.
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
  return requestRuntime(request)
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
