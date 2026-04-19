#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { mkdir, rm, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-shell-scriptability-drill.mjs [--keep-artifacts-on-failure]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const kernelPort = 47000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[shell-drill] ${name}`)
  else console.log(`[shell-drill] ${name}`, JSON.stringify(details))
}

async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', reject)
    child.on('close', (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function buildKernel() {
  const existingBinary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const existing = await stat(existingBinary).then((info) => info.isFile()).catch(() => false)
  if (existing) return existingBinary
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  }
  return existingBinary
}

async function waitForDaemon(shellBin, kernelUrl, workspace, env) {
  const scriptPath = path.join(workspace, 'wait.arroba')
  await writeFile(scriptPath, 'session list\n', 'utf8')
  const deadline = Date.now() + 20_000
  let last = null
  while (Date.now() < deadline) {
    last = await run(process.execPath, [shellBin, 'run', scriptPath, '--kernel-url', kernelUrl, '--workspace', workspace, '--worktree', workspace], { env })
    if (last.code === 0) return
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`daemon did not become ready\nstdout:\n${last?.stdout ?? ''}\nstderr:\n${last?.stderr ?? ''}`)
}

function requireOutput(output, pattern, label) {
  if (!pattern.test(output)) {
    throw new Error(`missing ${label}: ${pattern}\n--- output ---\n${output}`)
  }
}

async function cleanupSession(kernelUrl, sessionId) {
  if (!sessionId) return
  const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
  const { endSessionRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const client = new LocalIpcClient(kernelUrl)
  try {
    await client.send(endSessionRequest(sessionId)).catch(() => {})
  } finally {
    await client.close().catch(() => {})
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-shell-scriptability-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const scriptsDir = path.join(rootDir, 'scripts')
  const skillDir = path.join(rootDir, 'shell-drill-skill')
  const mcpPath = path.join(rootDir, 'echo-mcp.mjs')
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const shellBin = path.join(repoRoot, 'apps/shell/dist/shell.js')
  const env = {
    ...process.env,
    HOME: home,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `shell-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
  }

  let daemon = null
  let sessionId = null
  let succeeded = false
  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })
    await mkdir(scriptsDir, { recursive: true })
    await mkdir(skillDir, { recursive: true })
    await writeFile(path.join(skillDir, 'SKILL.md'), '---\nname: shell-drill-skill\ndescription: Skill used by the arroba-shell live drill.\n---\nUse this only for shell live drills.\n', 'utf8')
    await writeFile(mcpPath, [
      "process.stdin.resume()",
      "process.stdin.on('data', () => {})",
    ].join('\n'), 'utf8')
    await writeFile(path.join(workspace, 'sourced.arroba'), [
      'set effort low',
      'vars',
    ].join('\n'), 'utf8')

    const kernelBinary = await buildKernel()
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
    await waitForDaemon(shellBin, kernelUrl, workspace, env)
    log('daemon-ready', { kernelUrl })

    const successScript = path.join(scriptsDir, 'success.arroba')
    await writeFile(successScript, [
      'set provider dev-stub',
      'set model shell-drill-model',
      'set effort low',
      'vars',
      'source sourced.arroba',
      'session list',
      'session new $workspace as session',
      'session use $session',
      'agent list',
      'agent spawn alpha shell-drill-model as alpha',
      'agent spawn beta shell-drill-model as beta',
      'agent list',
      'agent focus $alpha',
      'agent cycle',
      'config path',
      'config show',
      'config set providers.model shell-drill-model',
      'config managed-io dev-stub off',
      'config unset providers.model',
      'mcp list',
      'mcp install shell_echo --command node --arg $mcp',
      'mcp show shell_echo',
      'mcp grant $alpha shell_echo',
      'mcp grants $alpha',
      'mcp revoke $alpha shell_echo',
      'mcp uninstall shell_echo',
      'skill list',
      'skill install $skill',
      'skill show shell-drill-skill',
      'skill grant $alpha shell-drill-skill',
      'skill grants $alpha',
      'skill revoke $alpha shell-drill-skill',
      'skill uninstall shell-drill-skill',
      'workflow list',
      'workflow new shell-flow as workflow',
      'workflow show $workflow',
      'workflow alias $workflow shell-flow-alias',
      'workflow node add $workflow $alpha as node1',
      'workflow node add $workflow $beta as node2',
      'workflow node can-complete-run $workflow $node1 false',
      'workflow node can-emit-intermediate-output $workflow $node1 true',
      'workflow node intermediate-output-schema $workflow $node1 none',
      'workflow node max-turns $workflow $node1 2',
      'workflow edge add $workflow $node1 $node2',
      'workflow endpoint new $workflow $node1 shell-entry',
      'workflow endpoint alias $workflow shell-entry shell-entry-2',
      'workflow endpoint bind $workflow shell-entry-2 $node1',
      'workflow flush-context $workflow true',
      'workflow run-output-schema $workflow none',
      'workflow intermediate-output-schema $workflow none',
      'workflow launch-policy queue',
      'workflow max-turns off',
      'workflow watchdog list',
      'workflow watchdog add $workflow shell-entry-2 every 30s skip shell-drill-watchdog',
      'workflow watchdog list',
      'workflow queue list',
      'workflow queue flush',
      'workflow runs',
      'provider processes',
      'provider processes teardown',
    ].join('\n'), 'utf8')

    const success = await run(process.execPath, [
      shellBin,
      'run',
      successScript,
      '--kernel-url',
      kernelUrl,
      '--workspace',
      workspace,
      '--worktree',
      workspace,
      '--var',
      `workspace=${workspace}`,
      '--var',
      `mcp=${mcpPath}`,
      '--var',
      `skill=${skillDir}`,
    ], { env })
    if (success.code !== 0) {
      throw new Error(`success script failed\nstdout:\n${success.stdout}\nstderr:\n${success.stderr}`)
    }
    requireOutput(success.stdout, /bound \$session = /, 'session binding')
    requireOutput(success.stdout, /bound \$alpha = /, 'agent binding')
    requireOutput(success.stdout, /installed MCP shell_echo/, 'MCP install')
    requireOutput(success.stdout, /installed skill shell-drill-skill/, 'skill install')
    requireOutput(success.stdout, /created workflow/, 'workflow create')
    requireOutput(success.stdout, /created workflow watchdog/, 'watchdog create')
    sessionId = success.stdout.match(/bound \$session = (\S+)/)?.[1] ?? null
    if (!sessionId) {
      throw new Error(`success script did not expose a session id\n${success.stdout}`)
    }
    log('success-script-passed', { sessionId })

    const auditScript = path.join(scriptsDir, 'continue-on-error.arroba')
    await writeFile(auditScript, [
      'vars',
      'session use $session',
      'stop',
      'waiting',
      'machine list',
      'relay status',
      'session list',
    ].join('\n'), 'utf8')
    const audit = await run(process.execPath, [
      shellBin,
      'run',
      auditScript,
      '--kernel-url',
      kernelUrl,
      '--workspace',
      workspace,
      '--worktree',
      workspace,
      '--var',
      'seeded=yes',
      '--var',
      `session=${sessionId}`,
      '--continue-on-error',
    ], { env })
    if (audit.code === 0) {
      throw new Error(`continue-on-error script unexpectedly exited 0\nstdout:\n${audit.stdout}\nstderr:\n${audit.stderr}`)
    }
    requireOutput(audit.stdout, /\$seeded = yes/, 'seeded variable')
    requireOutput(audit.stdout, /line 3 failed; continuing/, 'line 3 continue diagnostic')
    requireOutput(audit.stdout, /line 4 failed; continuing/, 'line 4 continue diagnostic')
    requireOutput(audit.stdout, /line 5 failed; continuing/, 'line 5 continue diagnostic')
    requireOutput(audit.stdout, /relay not configured/, 'relay status output')
    requireOutput(audit.stdout, /session/, 'continued command after failures')
    log('continue-on-error-script-passed')

    succeeded = true
  } finally {
    await cleanupSession(kernelUrl, sessionId)
    if (daemon) {
      daemon.kill('SIGTERM')
      await new Promise((resolve) => setTimeout(resolve, 250))
      if (!daemon.killed) daemon.kill('SIGKILL')
    }
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true })
    } else {
      console.error(`shell drill artifacts kept at ${rootDir}`)
    }
  }
  log('passed')
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error)
  process.exit(1)
})
