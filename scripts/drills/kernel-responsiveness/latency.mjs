import { join } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

export async function measureLatency({ rpc, create, cycle, root, mcpUrl, samples, spacing, report, save, load, timings, distribution }) {
  const room = await create(false), sessionId = room.session.id
  const terminal = async operation => (await rpc({ RoomTerminal: { session_id: sessionId, operation } }, true)).RoomTerminal.result
  const background = []
  for (let i = 0; i < 4; i++) background.push((await terminal({ operation: 'open', cols: 80, rows: 24, command: ['/bin/cat'] })).terminal.terminal_id)
  const path = join(root, 'work', 'latency.bin')
  await rpc({ WriteKernelFile: { path, offset: 0, total_size: 1, data_base64: 'eA==', overwrite: true } })
  timings.clear()
  const activity = []
  for (let i = 0; i < samples; i++) {
    const next = performance.now()+spacing
    await rpc({ ListSessions: {} }, true)
    await rpc({ GetDaemonHealth: null }, true)
    await rpc({ ReadKernelFile: { path, offset: 0, length: 1 } }, true)
    await rpc({ WriteKernelFile: { path, offset: 0, total_size: 1, data_base64: 'eA==', overwrite: true } }, true)
    await cycle(true)
    const opened = await terminal({ operation: 'open', cols: 80, rows: 24, command: ['/bin/sh', '-c', 'exit 0'] })
    await terminal({ operation: 'close', terminal_id: opened.terminal.terminal_id })
    const mcpStart = performance.now()
    const mcp = await fetch(mcpUrl, { method: 'POST', headers: { authorization: 'Bearer responsiveness-probe', 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: i+1, method: 'initialize', params: { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'responsiveness-probe', version: '1' } } }) })
    const initialized = await mcp.json()
    if (initialized.result?.serverInfo?.name !== 'chariox-runtime') throw Error('MCP initialize failed')
    if (!timings.has('McpInitialize')) timings.set('McpInitialize', [])
    timings.get('McpInitialize').push(performance.now()-mcpStart)
    if (load && i % 30 === 0) { activity.push({ sample: i, agents: await load.status() }); report.load_activity = activity }
    if (i % 30 === 0) {
      report.latency = Object.fromEntries([...timings].map(([kind, values]) => [kind, distribution([...values])]))
      report.latency_samples_completed = i+1; save()
    }
    await delay(Math.max(0,next-performance.now()))
  }
  report.latency = Object.fromEntries([...timings].map(([kind, values]) => [kind, distribution(values)]))
  report.latency_samples_completed = samples; save()
  for (const terminalId of background) await terminal({ operation: 'close', terminal_id: terminalId })
  await rpc({ EndSession: { session_id: sessionId } })
}
