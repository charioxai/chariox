let buffer = Buffer.alloc(0)
function write(message) {
  const body = JSON.stringify(message)
  process.stdout.write(`${body}\n`)
}
function handle(message) {
  const { id, method, params } = message
  if (method === 'notifications/initialized') return
  if (method === 'initialize') {
    write({ jsonrpc: '2.0', id, result: { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'arroba-workflow-echo', version: '1.0.0' } } })
    return
  }
  if (method === 'tools/list') {
    write({ jsonrpc: '2.0', id, result: { tools: [{ name: 'echo_marker', description: 'Echoes a marker for Arroba workflow MCP drills.', inputSchema: { type: 'object', properties: { marker: { type: 'string' } }, required: ['marker'] } }] } })
    return
  }
  if (method === 'tools/call' && params?.name === 'echo_marker') {
    write({ jsonrpc: '2.0', id, result: { content: [{ type: 'text', text: `ECHO:${params?.arguments?.marker ?? ''}` }] } })
    return
  }
  write({ jsonrpc: '2.0', id, error: { code: -32601, message: `unknown method ${method}` } })
}
process.stdin.on('data', (chunk) => {
  buffer = Buffer.concat([buffer, chunk])
  while (true) {
    const newline = buffer.indexOf('\n')
    if (newline >= 0) {
      const line = buffer.subarray(0, newline).toString('utf8').trim()
      buffer = buffer.subarray(newline + 1)
      if (line) handle(JSON.parse(line))
      continue
    }
    const headerEnd = buffer.indexOf('\r\n\r\n')
    if (headerEnd < 0) return
    const header = buffer.subarray(0, headerEnd).toString('utf8')
    const match = /^content-length:\s*(\d+)$/im.exec(header)
    if (!match) throw new Error(`missing Content-Length: ${header}`)
    const length = Number(match[1])
    const bodyStart = headerEnd + 4
    const frameEnd = bodyStart + length
    if (buffer.length < frameEnd) return
    const message = JSON.parse(buffer.subarray(bodyStart, frameEnd).toString('utf8'))
    buffer = buffer.subarray(frameEnd)
    handle(message)
  }
})