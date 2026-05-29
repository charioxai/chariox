import { spawn } from 'node:child_process'
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { requestJson, waitForJsonHealth } from './http-json.mjs'
import { renderOpenCodeConfig, reservePort } from './provider-launcher.mjs'

export function resolveOpenCodeBinary() {
  return process.env.ARROBA_OPENCODE_BIN || process.env.OPENCODE_BIN || 'opencode'
}

export class OpenCodeServerRun {
  constructor({ agentId, port, baseUrl, child, logPath }) {
    this.agentId = agentId
    this.port = port
    this.baseUrl = baseUrl
    this.child = child
    this.logPath = logPath
    this.stdout = ''
    this.stderr = ''
  }

  async health() {
    return requestJson(this.baseUrl, 'GET', '/global/health')
  }

  async mcpStatus() {
    return requestJson(this.baseUrl, 'GET', '/mcp')
  }

  async createSession({ directory = process.cwd(), title = `arroba-spike-${this.agentId}` } = {}) {
    return requestJson(
      this.baseUrl,
      'POST',
      `/session?directory=${encodeURIComponent(directory)}`,
      { title },
      { timeoutMs: 10_000 },
    )
  }

  async getSession(sessionId, { directory = process.cwd() } = {}) {
    return requestJson(
      this.baseUrl,
      'GET',
      `/session/${encodeURIComponent(sessionId)}?directory=${encodeURIComponent(directory)}`,
      undefined,
      { timeoutMs: 10_000 },
    )
  }

  async prompt(sessionId, prompt, {
    directory = process.cwd(),
    model = process.env.OPENCODE_SPIKE_MODEL || 'opencode/gpt-5.2',
    variant = process.env.OPENCODE_SPIKE_VARIANT || 'low',
  } = {}) {
    const [providerID, modelID] = model.includes('/') ? model.split('/', 2) : ['openai', model]
    return requestJson(
      this.baseUrl,
      'POST',
      `/session/${encodeURIComponent(sessionId)}/message?directory=${encodeURIComponent(directory)}`,
      {
        model: { providerID, modelID },
        variant,
        parts: [{ type: 'text', text: prompt }],
      },
      { timeoutMs: 180_000 },
    )
  }

  async messages(sessionId, { directory = process.cwd() } = {}) {
    return requestJson(
      this.baseUrl,
      'GET',
      `/session/${encodeURIComponent(sessionId)}/message?directory=${encodeURIComponent(directory)}`,
      undefined,
      { timeoutMs: 10_000 },
    )
  }

  async transcriptText(sessionId, { directory = process.cwd() } = {}) {
    const messages = await this.messages(sessionId, { directory })
    return (messages ?? [])
      .filter((message) => message.info?.role === 'assistant')
      .flatMap((message) => message.parts ?? [])
      .filter((part) => part.type === 'text')
      .map((part) => part.text ?? '')
      .join('\n')
  }

  async stop() {
    if (!this.child || this.child.exitCode != null) return
    this.child.kill('SIGTERM')
    await Promise.race([
      new Promise((resolve) => this.child.once('exit', resolve)),
      new Promise((resolve) => setTimeout(resolve, 4000)),
    ])
    if (this.child.exitCode == null) {
      this.child.kill('SIGKILL')
      await Promise.race([
        new Promise((resolve) => this.child.once('exit', resolve)),
        new Promise((resolve) => setTimeout(resolve, 1000)),
      ])
    }
  }
}

export async function launchOpenCodeServer({ agentId, mcps, artifactDir, extraEnv = {} }) {
  const port = await reservePort()
  const baseUrl = `http://127.0.0.1:${port}`
  const executable = resolveOpenCodeBinary()
  const logDir = path.join(artifactDir, 'opencode-logs')
  await mkdir(logDir, { recursive: true })
  const logPath = path.join(logDir, `${agentId}.log`)
  const config = renderOpenCodeConfig(mcps)
  const env = {
    ...process.env,
    ...extraEnv,
    OPENCODE_CONFIG_CONTENT: JSON.stringify(config),
  }
  const args = ['serve', '--hostname', '127.0.0.1', '--port', String(port)]
  const child = spawn(executable, args, {
    cwd: process.cwd(),
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  const run = new OpenCodeServerRun({ agentId, port, baseUrl, child, logPath })
  child.stdout.on('data', (chunk) => { run.stdout += String(chunk) })
  child.stderr.on('data', (chunk) => { run.stderr += String(chunk) })
  child.once('exit', () => {
    void writeFile(logPath, [
      `command: ${executable} ${args.join(' ')}`,
      `baseUrl: ${baseUrl}`,
      `OPENCODE_CONFIG_CONTENT: ${JSON.stringify(config)}`,
      '',
      '--- stdout ---',
      run.stdout,
      '',
      '--- stderr ---',
      run.stderr,
    ].join('\n'), 'utf8').catch(() => {})
  })
  try {
    await waitForJsonHealth(baseUrl, '/global/health', (value) => value?.healthy === true)
    return run
  } catch (error) {
    await run.stop()
    await writeFile(logPath, [
      `failed to launch OpenCode for ${agentId}: ${error.message}`,
      `command: ${executable} ${args.join(' ')}`,
      `baseUrl: ${baseUrl}`,
      `OPENCODE_CONFIG_CONTENT: ${JSON.stringify(config)}`,
      '',
      '--- stdout ---',
      run.stdout,
      '',
      '--- stderr ---',
      run.stderr,
    ].join('\n'), 'utf8').catch(() => {})
    throw error
  }
}
