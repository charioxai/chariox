import { spawn } from 'node:child_process'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { JsonRpcFramer, waitForExit, writeJsonRpc } from './mcp-framing.mjs'

const spikeRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const fakeServerPath = path.join(spikeRoot, 'src', 'fake-mcp-server.mjs')
const proxyServerPath = path.join(spikeRoot, 'src', 'mcp-proxy-server.mjs')

export class McpClientProcess {
  constructor({ name, statePath, mode = 'backing', agentId = null, backingId = null }) {
    this.name = name
    this.statePath = statePath
    this.mode = mode
    this.agentId = agentId
    this.backingId = backingId ?? `${name}-${mode}`
    this.nextId = 1
    this.pending = new Map()
    this.child = spawn(process.execPath, [
      fakeServerPath,
      '--name', name,
      '--state', statePath,
      '--mode', mode,
      '--backing-id', this.backingId,
      ...(agentId ? ['--agent-id', agentId] : []),
    ], { stdio: ['pipe', 'pipe', 'pipe'] })
    this.stderr = ''
    this.child.stderr.on('data', (chunk) => { this.stderr += String(chunk) })
    this.child.on('exit', (code) => {
      for (const { reject } of this.pending.values()) reject(new Error(`MCP ${name} exited with ${code}: ${this.stderr}`))
      this.pending.clear()
    })
    new JsonRpcFramer(this.child.stdout, (message) => this.handleMessage(message))
  }

  handleMessage(message) {
    const pending = this.pending.get(message.id)
    if (!pending) return
    this.pending.delete(message.id)
    if (message.error) pending.reject(new Error(message.error.message ?? JSON.stringify(message.error)))
    else pending.resolve(message.result)
  }

  request(method, params = {}) {
    const id = this.nextId++
    writeJsonRpc(this.child.stdin, { jsonrpc: '2.0', id, method, params })
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject })
    })
  }

  notify(method, params = {}) {
    writeJsonRpc(this.child.stdin, { jsonrpc: '2.0', method, params })
  }

  async initialize() {
    const result = await this.request('initialize', {
      protocolVersion: '2024-11-05',
      capabilities: {},
      clientInfo: { name: 'chariox-mcp-isolation-spike', version: '0.0.0' },
    })
    this.notify('notifications/initialized')
    return result
  }

  toolsList() {
    return this.request('tools/list')
  }

  toolsCall(toolName, args) {
    return this.request('tools/call', { name: toolName, arguments: args })
  }

  async stop() {
    if (this.child.exitCode != null) return
    this.child.kill('SIGTERM')
    await Promise.race([waitForExit(this.child), new Promise((resolve) => setTimeout(resolve, 2000))])
    if (this.child.exitCode == null) this.child.kill('SIGKILL')
  }
}

export class McpSupervisor {
  constructor({ artifactDir }) {
    this.artifactDir = artifactDir
    this.statePath = path.join(artifactDir, 'mcp-state.json')
    this.processes = new Map()
  }

  async prepare() {
    await mkdir(this.artifactDir, { recursive: true })
    await writeFile(this.statePath, JSON.stringify({ servers: {}, calls: [] }, null, 2), 'utf8')
  }

  key({ name, scope = 'shared', agentId = null }) {
    return `${scope}:${name}:${agentId ?? 'shared'}`
  }

  async ensure({ name, scope = 'shared', agentId = null }) {
    const key = this.key({ name, scope, agentId })
    const existing = this.processes.get(key)
    if (existing) return existing
    const process = new McpClientProcess({
      name,
      statePath: this.statePath,
      mode: scope === 'per-agent' ? 'backing-per-agent' : 'backing-shared',
      agentId: scope === 'per-agent' ? agentId : null,
      backingId: key,
    })
    await process.initialize()
    this.processes.set(key, process)
    return process
  }

  providerStdioConfig({ name, agentId = null, mode = 'provider-owned' }) {
    return {
      name,
      transport: 'stdio',
      command: process.execPath,
      args: [
        fakeServerPath,
        '--name', name,
        '--state', this.statePath,
        '--mode', mode,
        '--backing-id', `${mode}:${name}:${agentId ?? 'shared'}`,
        ...(agentId ? ['--agent-id', agentId] : []),
      ],
    }
  }

  proxiedProviderStdioConfig({ name, agentId = null, backingMode = 'backing-shared' }) {
    return {
      name,
      transport: 'stdio',
      command: process.execPath,
      args: [
        proxyServerPath,
        '--name', name,
        '--state', this.statePath,
        '--backing-mode', backingMode,
        ...(agentId ? ['--agent-id', agentId] : []),
      ],
    }
  }

  async snapshot() {
    return JSON.parse(await readFile(this.statePath, 'utf8'))
  }

  async stopAll() {
    const processes = [...this.processes.values()]
    this.processes.clear()
    await Promise.all(processes.map((process) => process.stop()))
  }
}
