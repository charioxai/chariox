const runtimeUrl = process.env.CHARIOX_DEV_STUB_RUNTIME_MCP_URL
const runtimeToken = process.env.CHARIOX_DEV_STUB_RUNTIME_MCP_TOKEN
delete process.env.CHARIOX_DEV_STUB_RUNTIME_MCP_URL
delete process.env.CHARIOX_DEV_STUB_RUNTIME_MCP_TOKEN

async function requestRuntime(request, signal = new AbortController().signal) {
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
    signal: AbortSignal.any([signal, AbortSignal.timeout(90_000)]) })
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
