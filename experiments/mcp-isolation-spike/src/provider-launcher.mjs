import { spawn } from 'node:child_process'
import net from 'node:net'
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'

export function renderCodexMcpArgs(mcpServers) {
  const args = []
  for (const server of mcpServers) {
    const prefix = `mcp_servers.${server.name}`
    args.push('-c', `${prefix}.command=${JSON.stringify(server.command)}`)
    if (server.args?.length) args.push('-c', `${prefix}.args=${JSON.stringify(server.args)}`)
    args.push('-c', `${prefix}.required=true`)
    args.push('-c', `${prefix}.tool_timeout_sec=15`)
  }
  return args
}

export function renderOpenCodeConfig(mcpServers) {
  const mcp = {}
  for (const server of mcpServers) {
    mcp[server.name] = {
      type: 'local',
      command: [server.command, ...(server.args ?? [])],
      enabled: true,
    }
  }
  return { mcp }
}

export async function reservePort() {
  const server = net.createServer()
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const port = server.address().port
  await new Promise((resolve) => server.close(resolve))
  return port
}

export async function writeProviderPlans({ artifactDir, agents }) {
  const plansDir = path.join(artifactDir, 'provider-plans')
  await mkdir(plansDir, { recursive: true })
  const plans = []
  for (const agent of agents) {
    const codex = {
      provider: 'codex',
      agent_id: agent.id,
      mcp_names: agent.mcps.map((mcp) => mcp.name),
      args: ['app-server', ...renderCodexMcpArgs(agent.mcps), '--listen', 'ws://127.0.0.1:<port>'],
    }
    const opencode = {
      provider: 'opencode',
      agent_id: agent.id,
      mcp_names: agent.mcps.map((mcp) => mcp.name),
      env: { OPENCODE_CONFIG_CONTENT: JSON.stringify(renderOpenCodeConfig(agent.mcps)) },
      args: ['serve', '--hostname', '127.0.0.1', '--port', '<port>'],
    }
    plans.push(codex, opencode)
    await writeFile(path.join(plansDir, `${agent.id}-codex.json`), JSON.stringify(codex, null, 2), 'utf8')
    await writeFile(path.join(plansDir, `${agent.id}-opencode.json`), JSON.stringify(opencode, null, 2), 'utf8')
  }
  return plans
}

export function spawnProvider(command, args, { env = {}, cwd = process.cwd(), logPrefix = 'provider' } = {}) {
  const child = spawn(command, args, {
    cwd,
    env: { ...process.env, ...env },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  child.stdout.on('data', (chunk) => process.stdout.write(`[${logPrefix}] ${chunk}`))
  child.stderr.on('data', (chunk) => process.stderr.write(`[${logPrefix}] ${chunk}`))
  return child
}
