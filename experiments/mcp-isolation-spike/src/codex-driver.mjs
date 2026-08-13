import { spawn } from 'node:child_process'
import { appendFile, mkdir } from 'node:fs/promises'
import path from 'node:path'
import { renderCodexMcpArgs, reservePort } from './provider-launcher.mjs'

export function resolveCodexBinary() {
  return process.env.CHARIOX_CODEX_BIN || process.env.CODEX_BIN || 'codex'
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

class CodexJsonRpcSocket {
  constructor(endpoint) {
    this.endpoint = endpoint
    this.ws = new WebSocket(endpoint)
    this.nextId = 1
    this.pending = new Map()
    this.notifications = []
    this.notificationWaiters = []
    this.ws.addEventListener('message', (event) => {
      this.handleMessage(String(event.data))
    })
    this.ws.addEventListener('close', () => {
      for (const { reject } of this.pending.values()) reject(new Error(`Codex websocket closed: ${endpoint}`))
      this.pending.clear()
    })
  }

  async open() {
    if (this.ws.readyState === WebSocket.OPEN) return
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error(`timed out opening ${this.endpoint}`)), 10_000)
      this.ws.addEventListener('open', () => {
        clearTimeout(timeout)
        resolve()
      }, { once: true })
      this.ws.addEventListener('error', (event) => {
        clearTimeout(timeout)
        reject(new Error(`failed opening ${this.endpoint}: ${event.message ?? 'websocket error'}`))
      }, { once: true })
    })
  }

  async initialize() {
    await this.requestWithId(0, 'initialize', {
      protocolVersion: 2,
      clientInfo: { name: 'chariox-mcp-isolation-spike', version: '0.0.0-spike' },
      capabilities: {},
      notifications: [],
    })
    this.notify('initialized', {})
  }

  handleMessage(raw) {
    let message
    try {
      message = JSON.parse(raw)
    } catch {
      return
    }
    if (message.id != null && message.method) {
      this.respondToServerRequest(message)
      return
    }
    if (message.id != null && this.pending.has(message.id)) {
      const pending = this.pending.get(message.id)
      this.pending.delete(message.id)
      if (message.error) pending.reject(new Error(message.error.message ?? JSON.stringify(message.error)))
      else pending.resolve(message.result)
      return
    }
    if (message.method) {
      this.notifications.push(message)
      this.resolveNotificationWaiters(message)
    }
  }

  transcriptText() {
    return this.notifications
      .filter((message) => message.method === 'item/agentMessage/delta')
      .map((message) => message.params?.delta ?? '')
      .join('')
  }

  resolveNotificationWaiters(message) {
    const remaining = []
    for (const waiter of this.notificationWaiters) {
      if (waiter.predicate(message)) {
        clearTimeout(waiter.timeout)
        waiter.resolve(message)
      } else {
        remaining.push(waiter)
      }
    }
    this.notificationWaiters = remaining
  }

  respondToServerRequest(message) {
    let result = {}
    if (message.method === 'item/commandExecution/requestApproval') {
      result = { decision: 'decline' }
    } else if (message.method === 'item/fileChange/requestApproval') {
      result = { decision: 'decline' }
    } else if (message.method === 'item/permissions/requestApproval') {
      result = { permissions: message.params?.permissions ?? {}, scope: 'session' }
    } else if (message.method === 'mcpServer/elicitation/request') {
      result = { action: 'accept', content: {}, _meta: null }
    }
    this.ws.send(JSON.stringify({ jsonrpc: '2.0', id: message.id, result }))
  }

  request(method, params = {}) {
    return this.requestWithId(this.nextId++, method, params)
  }

  requestWithId(id, method, params = {}) {
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`Codex request timed out: ${method}`))
      }, 20_000)
      this.pending.set(id, {
        resolve: (value) => {
          clearTimeout(timeout)
          resolve(value)
        },
        reject: (error) => {
          clearTimeout(timeout)
          reject(error)
        },
      })
      this.ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })
  }

  notify(method, params = {}) {
    this.ws.send(JSON.stringify({ jsonrpc: '2.0', method, params }))
  }

  async threadStart({ cwd = process.cwd(), model = process.env.CODEX_SPIKE_MODEL || 'gpt-5.2', ephemeral = true } = {}) {
    return this.request('thread/start', {
      approvalPolicy: 'never',
      sandbox: 'danger-full-access',
      sandboxPolicy: { type: 'dangerFullAccess' },
      personality: 'pragmatic',
      ephemeral,
      serviceName: 'chariox-mcp-isolation-spike',
      cwd,
      model,
    })
  }

  async threadResume(threadId, { cwd = process.cwd(), model = process.env.CODEX_SPIKE_MODEL || 'gpt-5.2' } = {}) {
    return this.request('thread/resume', {
      threadId,
      approvalPolicy: 'never',
      sandbox: 'danger-full-access',
      sandboxPolicy: { type: 'dangerFullAccess' },
      personality: 'pragmatic',
      cwd,
      model,
    })
  }

  async turnStart(threadId, prompt, { cwd = process.cwd(), model = process.env.CODEX_SPIKE_MODEL || 'gpt-5.2', effort = 'low' } = {}) {
    const result = await this.request('turn/start', {
      threadId,
      input: [{ type: 'text', text: prompt }],
      approvalPolicy: 'never',
      personality: 'pragmatic',
      sandbox: 'danger-full-access',
      sandboxPolicy: { type: 'dangerFullAccess' },
      summary: 'detailed',
      cwd,
      model,
      effort,
    })
    return {
      result,
      turnId: result?.turn?.id ?? result?.id ?? null,
    }
  }

  waitForNotification(predicate, timeoutMs = 120_000) {
    const existing = this.notifications.find(predicate)
    if (existing) return Promise.resolve(existing)
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.notificationWaiters = this.notificationWaiters.filter((waiter) => waiter.resolve !== resolve)
        reject(new Error('timed out waiting for Codex notification'))
      }, timeoutMs)
      this.notificationWaiters.push({ predicate, resolve, reject, timeout })
    })
  }

  async waitForTurnCompleted(turnId, timeoutMs = 120_000) {
    return this.waitForNotification((message) => {
      if (message.method !== 'turn/completed') return false
      const completedId = message.params?.turn?.id
      return !turnId || completedId === turnId
    }, timeoutMs)
  }

  async close() {
    if (!this.ws || this.ws.readyState === WebSocket.CLOSED) return
    if (this.ws.readyState === WebSocket.CLOSING) return
    await new Promise((resolve) => {
      const timer = setTimeout(resolve, 500)
      this.ws.addEventListener('close', () => {
        clearTimeout(timer)
        resolve()
      }, { once: true })
      this.ws.close()
    })
  }
}

export class CodexAppServerRun {
  constructor({ agentId, endpoint, port, child, logPath }) {
    this.agentId = agentId
    this.endpoint = endpoint
    this.port = port
    this.child = child
    this.logPath = logPath
  }

  async connectInitialized() {
    const socket = new CodexJsonRpcSocket(this.endpoint)
    await socket.open()
    await socket.initialize()
    return socket
  }

  async stop() {
    if (this.child.exitCode != null) return
    try {
      process.kill(-this.child.pid, 'SIGTERM')
    } catch {
      this.child.kill('SIGTERM')
    }
    await Promise.race([
      new Promise((resolve) => this.child.once('exit', resolve)),
      sleep(2000),
    ])
    if (this.child.exitCode == null) {
      try {
        process.kill(-this.child.pid, 'SIGKILL')
      } catch {
        this.child.kill('SIGKILL')
      }
    }
  }
}

export async function launchCodexServer({ agentId, mcps, artifactDir, extraEnv = {} }) {
  await mkdir(artifactDir, { recursive: true })
  const port = await reservePort()
  const endpoint = `ws://127.0.0.1:${port}`
  const logPath = path.join(artifactDir, `${agentId}-codex.log`)
  const args = ['app-server', ...renderCodexMcpArgs(mcps), '--listen', endpoint]
  const child = spawn(resolveCodexBinary(), args, {
    cwd: process.cwd(),
    env: { ...process.env, ...extraEnv },
    stdio: ['ignore', 'pipe', 'pipe'],
    detached: true,
  })
  child.stdout.on('data', (chunk) => { void appendFile(logPath, `[stdout] ${chunk}`) })
  child.stderr.on('data', (chunk) => { void appendFile(logPath, `[stderr] ${chunk}`) })

  const run = new CodexAppServerRun({ agentId, endpoint, port, child, logPath })
  const deadline = Date.now() + 15_000
  let lastError = null
  while (Date.now() < deadline) {
    if (child.exitCode != null) throw new Error(`Codex app-server exited early for ${agentId}; see ${logPath}`)
    try {
      const socket = await run.connectInitialized()
      await socket.close()
      return run
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error(`timed out waiting for Codex app-server ${agentId}: ${lastError?.message ?? 'unknown error'}`)
}
