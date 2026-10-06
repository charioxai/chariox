// MP-11 #879: exercise the real private provider fixture using a credential-free DTO.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { createServer } from 'node:http'
import { createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { once } from 'node:events'
import { startHostedRemoteProviderRun } from '../apps/cli/scripts/lib/hosted-cloud-kernel-scenarios.mjs'
import { bindProviderMcpFixture } from '../apps/cli/scripts/lib/provider-mcp-fixture.mjs'
import { callRuntimeMcp } from '../apps/cli/scripts/lib/runtime-mcp-assertions.mjs'
import { callRuntimeMcp as hostedCall } from '../apps/cli/scripts/lib/hosted-cloud-runtime-helpers.mjs'

const fixtureSource = readFileSync(new URL('../apps/kernel/src/provider/registry/dev_stub_adapter/runtime_mcp_fixture.cjs', import.meta.url), 'utf8')
const fixtureSecret = 'mp11-synthetic-binding-only'

test('MP-11 protocol-435 consumer executes tools inside the private provider; no binding leaves it', async t => {
  const called = []
  const server = createServer((request, reply) => {
    assert.equal(request.headers.authorization, `Bearer ${fixtureSecret}`)
    let body = ''
    request.on('data', chunk => { body += chunk })
    request.on('end', () => {
      const requestBody = JSON.parse(body)
      called.push({ path: request.url, method: requestBody.method })
      let result = { tools: [{ name: 'private-fixture-tool' }] }
      if (requestBody.method === 'tools/call') result = { isError: false, structuredContent: { marker: 'provider-owned-tool-result' } }
      if (requestBody.params?.name === 'echo-binding') result = { marker: fixtureSecret, encoded: Buffer.from(fixtureSecret).toString('base64') }
      if (requestBody.params?.name === 'denied') {
        reply.end(JSON.stringify({ error: { message: 'not allowed for host', data: fixtureSecret } })); return
      }
      reply.setHeader('content-type', 'application/json')
      reply.end(JSON.stringify({ jsonrpc: '2.0', id: requestBody.id, result }))
    })
  })
  server.listen(0, '127.0.0.1'); await once(server, 'listening')
  t.after(() => new Promise(resolve => server.close(resolve)))
  const child = spawn(process.execPath, ['-e', fixtureSource], { stdio: ['pipe', 'pipe', 'pipe'], env: {
    PATH: process.env.PATH, CHARIOX_DEV_STUB_RUNTIME_MCP_URL: `http://127.0.0.1:${server.address().port}/mcp`,
    CHARIOX_DEV_STUB_RUNTIME_MCP_TOKEN: fixtureSecret,
  } })
  const exit = once(child, 'exit')
  t.after(async () => { if (child.exitCode === null && child.signalCode === null) { assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1); child.kill('SIGTERM') }; await exit })
  const records = []
  const listeners = new Set()
  let recordId = 0
  let stdout = ''
  child.stdout.on('data', chunk => {
    stdout += chunk.toString()
    const record = { provider_run_id: 'leased:lease:worker', kind: 'ProviderOutput', record_id: ++recordId, bytes: [...chunk] }
    records.push(record)
    for (const listener of listeners) listener({ event: 'terminal_output', records: [record] })
  })
  const sent = []
  const client = {
    onKernelEvent(fn) { listeners.add(fn); return () => listeners.delete(fn) },
    async subscribeToKernelEvents() {},
    async send(request) {
      sent.push(request)
      if (request.ListAgents) return { AgentsListed: { agents: [{ id: 'agent', remote_execution: { leased_agent_id: 'lease', active_worker_provider_run_id: 'worker' } }] } }
      if (request.GetProviderRun) return { ProviderRun: { provider_run: { id: 'leased:lease:worker', session_id: 'session', provider: 'dev-stub', model: 'runtime-mcp-fixture', state: 'Running' } } }
      if (request.SendTerminalInput) child.stdin.write(Buffer.from(request.SendTerminalInput.data_base64, 'base64'))
      if (request.PumpTerminalOutput) return { TerminalOutput: { records: records.splice(0) } }
      return {}
    },
  }
  const requests = {
    submitPromptRequest: () => ({ SubmitPrompt: {} }),
    listAgentsRequest: () => ({ ListAgents: {} }),
    getProviderRunRequest: () => ({ GetProviderRun: {} }),
    sendTerminalInputRequest: (sessionId, attachmentId, input, providerRunId) => ({ SendTerminalInput: {
      session_id: sessionId, attachment_id: attachmentId, provider_run_id: providerRunId, data_base64: Buffer.from(input).toString('base64'),
    } }),
    pumpTerminalOutputRequest: () => ({ PumpTerminalOutput: {} }),
  }
  const run = await startHostedRemoteProviderRun({ client, requests, sessionId: 'session', attachmentId: 'attachment',
    agentId: 'agent', prompt: 'initialize', timeoutMs: 1000, pollMs: 1 })
  assert.equal(run.runtime_mcp_auth_token, undefined)
  assert.deepEqual(await callRuntimeMcp(run.fixtureMcp, null, 'tools/list'), { tools: [{ name: 'private-fixture-tool' }] })
  const tool = await hostedCall(run.fixtureMcp.proxy('home_echo_mcp'), null, 'tools/call', { name: 'private-fixture-tool' })
  assert.equal(tool.structuredContent.marker, 'provider-owned-tool-result')
  const scrubbed = await run.fixtureMcp.call('tools/call', { name: 'echo-binding' })
  assert.deepEqual(scrubbed, { marker: '[REDACTED]', encoded: '[REDACTED]' })
  const absent = await run.fixtureMcp.call('fixture/env-absent', { names: ['CHARIOX_SECRET_DRILL_LOCAL_TOKEN'] })
  assert.equal(absent.absent.CHARIOX_SECRET_DRILL_LOCAL_TOKEN, true)
  child.stdin.write('synthetic-terminal-secret\n')
  const receipt = await run.fixtureMcp.call('fixture/input-digest')
  assert.equal(receipt.sha256, createHash('sha256').update('synthetic-terminal-secret').digest('hex'))
  assert.doesNotMatch(stdout, /synthetic-terminal-secret/)
  const denial = await run.fixtureMcp.call('tools/call', { name: 'denied' }, { rawEnvelope: true })
  assert.deepEqual(denial.error, { message: 'not allowed for host', data: '[REDACTED]' })
  assert.equal(listeners.size, 0)
  assert.deepEqual(called.map(entry => entry.path), ['/mcp', '/mcp/proxy/home_echo_mcp', '/mcp', '/mcp'])
  assert.doesNotMatch(stdout, new RegExp(fixtureSecret))
  assert.doesNotMatch(JSON.stringify(sent), new RegExp(fixtureSecret))
  child.stdin.end(); await exit
})

test('MP-11 private fixture refuses ordinary provider runs', () => {
  for (const provider of ['codex', 'claude', 'opencode', 'dev-stub']) {
    assert.throws(() => bindProviderMcpFixture({ id: 'run', provider, model: 'ordinary' }, {}), /explicit private/)
  }
})
