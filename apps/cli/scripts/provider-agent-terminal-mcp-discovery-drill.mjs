#!/usr/bin/env node
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
const serverPath = path.join(repoRoot, 'apps/shell/dist/agent-terminal-main.js')

function run(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: { ...process.env, ...(options.env ?? {}) },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    const timer = setTimeout(() => {
      child.kill('SIGTERM')
      reject(new Error(`${command} timed out\n${stdout}\n${stderr}`))
    }, options.timeoutMs ?? 30_000)
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.once('error', (error) => {
      clearTimeout(timer)
      reject(error)
    })
    child.once('close', (code) => {
      clearTimeout(timer)
      resolve({ code: code ?? -1, stdout, stderr })
    })
  })
}

function assert(condition, message, details) {
  if (!condition) throw new Error(`${message}${details ? `\n${details}` : ''}`)
}

async function codex() {
  const home = await mkdtemp(path.join(tmpdir(), 'chariox-codex-mcp-'))
  try {
    const add = await run('codex', ['mcp', 'add', 'chariox-agent-terminal', '--', 'node', serverPath], { env: { CODEX_HOME: home } })
    assert(add.code === 0, 'Codex could not register the agent terminal MCP', `${add.stdout}\n${add.stderr}`)
    const listed = await run('codex', ['mcp', 'list'], { env: { CODEX_HOME: home } })
    assert(listed.code === 0 && listed.stdout.includes('chariox-agent-terminal'), 'Codex did not discover the registered agent terminal MCP', `${listed.stdout}\n${listed.stderr}`)
    assert(listed.stdout.includes('enabled'), 'Codex registered the MCP in a disabled state', listed.stdout)
    return { provider: 'codex', discovered: true, output: listed.stdout.trim() }
  } finally {
    await rm(home, { recursive: true, force: true })
  }
}

async function opencode() {
  const config = JSON.stringify({
    mcp: {
      'chariox-agent-terminal': {
        type: 'local',
        command: ['node', serverPath],
        enabled: true,
      },
    },
  })
  const listed = await run('opencode', ['mcp', 'list', '--pure', '--log-level', 'ERROR'], {
    env: { OPENCODE_CONFIG_CONTENT: config },
  })
  assert(listed.code === 0 && listed.stdout.includes('chariox-agent-terminal'), 'OpenCode did not discover the agent terminal MCP', `${listed.stdout}\n${listed.stderr}`)
  assert(/connected/i.test(listed.stdout), 'OpenCode did not connect to the agent terminal MCP', listed.stdout)
  return { provider: 'opencode', discovered: true, output: listed.stdout.trim() }
}

async function claude() {
  const cwd = await mkdtemp(path.join(tmpdir(), 'chariox-claude-mcp-'))
  try {
    const added = await run('claude', ['mcp', 'add', '--scope', 'project', 'chariox-agent-terminal', '--', 'node', serverPath], { cwd })
    assert(added.code === 0, 'Claude could not register the agent terminal MCP', `${added.stdout}\n${added.stderr}`)
    const configPath = path.join(cwd, '.mcp.json')
    const config = JSON.parse(await readFile(configPath, 'utf8'))
    assert(config.mcpServers?.['chariox-agent-terminal']?.command === 'node', 'Claude wrote an invalid agent terminal MCP config', JSON.stringify(config))
    const listed = await run('claude', ['mcp', 'list'], { cwd })
    assert(listed.code === 0 && listed.stdout.includes('chariox-agent-terminal'), 'Claude did not discover the project agent terminal MCP', `${listed.stdout}\n${listed.stderr}`)
    // Project-scoped servers intentionally require approval before Claude starts
    // them. Registration/discovery is the safe validation here; no model turn is run.
    assert(/pending approval/i.test(listed.stdout), 'Claude project MCP did not remain approval-gated', listed.stdout)
    return { provider: 'claude', discovered: true, approval_gated: true, output: listed.stdout.trim() }
  } finally {
    await rm(cwd, { recursive: true, force: true })
  }
}

async function main() {
  const results = await Promise.all([codex(), opencode(), claude()])
  console.log(JSON.stringify({ ok: true, providers: results.map(({ provider, discovered, approval_gated }) => ({ provider, discovered, ...(approval_gated ? { approval_gated } : {}) })) }))
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error)
  process.exitCode = 1
})
