#!/usr/bin/env node
import { mkdir, open, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { JsonRpcFramer, writeJsonRpc } from './mcp-framing.mjs'

function argValue(name, fallback = null) {
  const index = process.argv.indexOf(name)
  if (index < 0) return fallback
  return process.argv[index + 1] ?? fallback
}

const name = argValue('--name', process.env.PROXY_MCP_NAME ?? 'fake-alpha')
const statePath = argValue('--state', process.env.PROXY_MCP_STATE ?? null)
const agentId = argValue('--agent-id', process.env.PROXY_MCP_AGENT_ID ?? null)
const backingMode = argValue('--backing-mode', process.env.PROXY_MCP_BACKING_MODE ?? 'backing-shared')

async function loadState() {
  if (!statePath) return { servers: {}, calls: [] }
  try {
    return JSON.parse(await readFile(statePath, 'utf8'))
  } catch {
    return { servers: {}, calls: [] }
  }
}

async function saveState(state) {
  if (!statePath) return
  await mkdir(path.dirname(statePath), { recursive: true })
  await writeFile(statePath, JSON.stringify(state, null, 2), 'utf8')
}

async function withStateLock(fn) {
  if (!statePath) return fn()
  const lockPath = `${statePath}.lock`
  for (let attempt = 0; attempt < 400; attempt += 1) {
    let handle
    try {
      handle = await open(lockPath, 'wx')
      try {
        return await fn()
      } finally {
        await handle.close().catch(() => {})
        await rm(lockPath, { force: true }).catch(() => {})
      }
    } catch (error) {
      await handle?.close().catch(() => {})
      if (error?.code !== 'EEXIST') throw error
      await new Promise((resolve) => setTimeout(resolve, 5))
    }
  }
  throw new Error(`timed out acquiring fake MCP state lock ${lockPath}`)
}

async function record(kind, extra = {}) {
  await withStateLock(async () => {
    const state = await loadState()
    const key = `proxy:${name}:${agentId ?? 'shared'}`
    const server = state.servers[key] ?? {
      name,
      mode: 'proxy',
      agent_id: agentId,
      starts: 0,
      initializes: 0,
      tool_lists: 0,
      tool_calls: 0,
    }
    if (kind === 'start') server.starts += 1
    if (kind === 'initialize') server.initializes += 1
    if (kind === 'tools/list') server.tool_lists += 1
    if (kind === 'tools/call') server.tool_calls += 1
    server.last_event_at = new Date().toISOString()
    state.servers[key] = server
    state.calls.push({ at: new Date().toISOString(), kind, name, mode: 'proxy', backing_mode: backingMode, agent_id: agentId, ...extra })
    await saveState(state)
  })
}

async function recordBacking(kind, extra = {}) {
  await withStateLock(async () => {
    const state = await loadState()
    const backingAgentId = backingMode === 'backing-per-agent' ? agentId : null
    const key = `${backingMode}:${name}:${backingAgentId ?? 'shared'}`
    const server = state.servers[key] ?? {
      name,
      mode: backingMode,
      agent_id: backingAgentId,
      starts: 0,
      initializes: 0,
      tool_lists: 0,
      tool_calls: 0,
    }
    if (kind === 'ensure') server.starts = Math.max(server.starts, 1)
    if (kind === 'initialize') server.initializes = Math.max(server.initializes, 1)
    if (kind === 'tools/list') server.tool_lists += 1
    if (kind === 'tools/call') server.tool_calls += 1
    server.last_event_at = new Date().toISOString()
    state.servers[key] = server
    state.calls.push({ at: new Date().toISOString(), kind, name, mode: backingMode, backing_id: key, agent_id: backingAgentId, ...extra })
    await saveState(state)
  })
}

await record('start', { pid: process.pid, argv: process.argv.slice(2) })
await recordBacking('ensure', { via_proxy: true })
await recordBacking('initialize', { via_proxy: true })

let responseFrameFormat = 'line'
function reply(message) {
  writeJsonRpc(process.stdout, message, responseFrameFormat)
}

async function handle(message) {
  if (!message || typeof message !== 'object') return
  const { id, method, params } = message
  try {
    if (method === 'initialize') {
      await record('initialize', { params })
      reply({
        jsonrpc: '2.0',
        id,
        result: {
          protocolVersion: '2024-11-05',
          capabilities: { tools: {} },
          serverInfo: { name: `chariox-spike-proxy-${name}`, version: '0.0.0-spike' },
        },
      })
      return
    }
    if (method === 'notifications/initialized') return
    if (method === 'tools/list') {
      await record('tools/list')
      await recordBacking('tools/list', { via_proxy: true })
      reply({
        jsonrpc: '2.0',
        id,
        result: {
          tools: [{
            name: `${name}_echo`,
            description: `Spike proxied fake MCP tool for ${name}.`,
            inputSchema: {
              type: 'object',
              properties: { message: { type: 'string' } },
              required: ['message'],
            },
          }],
        },
      })
      return
    }
    if (method === 'tools/call') {
      await record('tools/call', { tool: params?.name ?? null, arguments: params?.arguments ?? null })
      await recordBacking('tools/call', { via_proxy: true, tool: params?.name ?? null, arguments: params?.arguments ?? null })
      reply({
        jsonrpc: '2.0',
        id,
        result: {
          content: [{
            type: 'text',
            text: JSON.stringify({ ok: true, name, mode: backingMode, proxied: true, agent_id: agentId, input: params?.arguments ?? {} }),
          }],
        },
      })
      return
    }
    reply({ jsonrpc: '2.0', id, error: { code: -32601, message: `unknown method ${method}` } })
  } catch (error) {
    reply({ jsonrpc: '2.0', id, error: { code: -32000, message: error.message } })
  }
}

new JsonRpcFramer(
  process.stdin,
  (message) => { void handle(message) },
  { onFrame: (format) => { responseFrameFormat = format } },
)
process.stdin.resume()
