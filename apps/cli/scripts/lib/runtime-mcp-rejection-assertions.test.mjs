import assert from 'node:assert/strict'
import http from 'node:http'
import test from 'node:test'

import { expectRuntimeMcpReject as expectHostedReject } from './hosted-cloud-runtime-helpers.mjs'
import { expectRuntimeMcpReject as expectSharedReject } from './runtime-mcp-assertions.mjs'

const helpers = [
  ['shared', expectSharedReject],
  ['hosted', expectHostedReject],
]

async function withRuntimeMcpServer(run) {
  const server = http.createServer((request, response) => {
    response.setHeader('Content-Type', 'application/json')
    if (request.url === '/rpc-error') {
      response.end(JSON.stringify({ jsonrpc: '2.0', id: 'test', error: { code: -32000, message: 'denied' } }))
      return
    }
    const isError = request.url === '/tool-error'
    response.end(JSON.stringify({ jsonrpc: '2.0', id: 'test', result: { isError, content: [] } }))
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  try {
    const address = server.address()
    await run(`http://127.0.0.1:${address.port}`)
  } finally {
    await new Promise((resolve) => server.close(resolve))
  }
}

for (const [name, expectReject] of helpers) {
  test(`${name} runtime MCP rejection assertion fails on successful calls`, async () => {
    await withRuntimeMcpServer(async (baseUrl) => {
      await assert.rejects(
        expectReject(`${baseUrl}/success`, 'token', 'tools/call'),
        /runtime MCP tools\/call unexpectedly succeeded/,
      )
    })
  })

  test(`${name} runtime MCP rejection assertion accepts tool and protocol errors`, async () => {
    await withRuntimeMcpServer(async (baseUrl) => {
      const toolError = await expectReject(`${baseUrl}/tool-error`, 'token', 'tools/call')
      assert.equal(toolError.isError, true)
      const protocolError = await expectReject(`${baseUrl}/rpc-error`, 'token', 'tools/call')
      assert.match(protocolError.error, /denied/)
    })
  })
}
