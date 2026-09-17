#!/usr/bin/env node

// Native-only acceptance probe for PR364 discovery configuration.
//
// This deliberately launches the installed official OpenCode binary directly.
// It does not build or start Chariox, and it never prompts a model. The local
// fake MCP is only a marker: its start/initialize/tools/list/tools/call events
// are written to a disposable state file so global-config autostart is visible.

import { execFileSync, spawn } from 'node:child_process'
import { once } from 'node:events'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs/promises'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'

const scriptPath = fileURLToPath(import.meta.url)
const scriptDir = path.dirname(scriptPath)
const repoRoot = path.resolve(scriptDir, '../../..')
const fakeMcp = path.join(scriptDir, 'fake-mcp-server.mjs')
const executable = process.env.CHARIOX_OPENCODE_BIN?.trim() || '/usr/local/bin/opencode'

function reservePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      const port = typeof address === 'object' && address ? address.port : null
      server.close((error) => error ? reject(error) : resolve(port))
    })
  })
}

function safeEnvironment(root, { inlineConfig, disableProjectConfig = false } = {}) {
  const env = {
    PATH: process.env.PATH || '/usr/local/bin:/usr/bin:/bin',
    HOME: path.join(root, 'home'),
    TMPDIR: path.join(root, 'tmp'),
    XDG_CONFIG_HOME: path.join(root, 'xdg-config'),
    XDG_DATA_HOME: path.join(root, 'xdg-data'),
    XDG_STATE_HOME: path.join(root, 'xdg-state'),
    XDG_CACHE_HOME: path.join(root, 'xdg-cache'),
    OPENCODE_CONFIG_DIR: path.join(root, 'xdg-config', 'opencode'),
    LANG: 'C',
    LC_ALL: 'C',
  }
  if (inlineConfig !== undefined) env.OPENCODE_CONFIG_CONTENT = JSON.stringify(inlineConfig)
  if (disableProjectConfig) env.OPENCODE_DISABLE_PROJECT_CONFIG = 'true'
  return env
}

async function requestJson(baseUrl, pathname, { method = 'GET', body, timeoutMs = 5000 } = {}) {
  const response = await fetch(`${baseUrl}${pathname}`, {
    method,
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(timeoutMs),
  })
  const text = await response.text()
  let value
  try {
    value = text ? JSON.parse(text) : null
  } catch {
    value = { raw: text.slice(0, 1000) }
  }
  if (!response.ok) throw new Error(`${method} ${pathname} returned ${response.status}: ${JSON.stringify(value)}`)
  return value
}

async function waitForHealth(baseUrl, child, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = 'health endpoint was not reachable'
  while (Date.now() < deadline) {
    if (child.exitCode !== null) throw new Error(`OpenCode exited before health: ${child.exitCode}; ${lastError}`)
    try {
      const health = await requestJson(baseUrl, '/global/health', { timeoutMs: 800 })
      if (health?.healthy === true) return health
      lastError = `unhealthy response ${JSON.stringify(health)}`
    } catch (error) {
      lastError = error.message
    }
    await sleep(100)
  }
  throw new Error(lastError)
}

function parseJsonOutput(stdout) {
  const text = stdout.trim()
  if (!text) return null
  try {
    return JSON.parse(text)
  } catch {}
  for (let start = 0; start < text.length; start += 1) {
    if (text[start] !== '{' && text[start] !== '[') continue
    for (let end = text.length; end > start; end -= 1) {
      try {
        return JSON.parse(text.slice(start, end))
      } catch {}
    }
  }
  return null
}

function commandMode(command) {
  if (!Array.isArray(command)) return null
  const index = command.indexOf('--mode')
  return index >= 0 ? command[index + 1] ?? null : null
}

function summarizeResolvedConfig(config) {
  const mcp = config?.mcp && typeof config.mcp === 'object' ? config.mcp : {}
  return {
    names: Object.keys(mcp).sort(),
    entries: Object.fromEntries(Object.entries(mcp).map(([name, value]) => [name, {
      type: value?.type ?? null,
      enabled: value?.enabled ?? null,
      marker_mode: commandMode(value?.command),
    }])),
    permission_star: config?.permission?.['*'] ?? null,
    plan_permission_star: config?.agent?.plan?.permission?.['*'] ?? null,
  }
}

function summarizeMcpStatus(status) {
  if (!status || typeof status !== 'object' || Array.isArray(status)) return { raw: status }
  return Object.fromEntries(Object.entries(status).sort(([a], [b]) => a.localeCompare(b)).map(([name, value]) => [name, {
    status: value?.status ?? null,
    error: value?.error ?? null,
    tools: Array.isArray(value?.tools) ? value.tools.map((tool) => tool?.name ?? null) : null,
  }]))
}

async function readState(statePath) {
  try {
    return JSON.parse(await fs.readFile(statePath, 'utf8'))
  } catch {
    return { servers: {}, calls: [] }
  }
}

function summarizeMarkers(state) {
  const servers = Object.values(state?.servers ?? {})
  const markers = Object.fromEntries(servers.map((server) => [
    `${server.mode}:${server.name}`,
    {
      starts: server.starts ?? 0,
      initializes: server.initializes ?? 0,
      tool_lists: server.tool_lists ?? 0,
      tool_calls: server.tool_calls ?? 0,
    },
  ]).sort(([a], [b]) => a.localeCompare(b)))
  return {
    markers,
    events: (state?.calls ?? []).map((event) => ({
      kind: event.kind,
      name: event.name,
      mode: event.mode,
    })),
  }
}

function allMarkerEntries(caseResult) {
  return Object.values(caseResult.markers?.markers ?? {})
}

function marker(name, mode, statePath) {
  return {
    type: 'local',
    command: [process.execPath, fakeMcp, '--name', name, '--mode', mode, '--state', statePath],
    enabled: true,
  }
}

async function writeFixture(root, statePath) {
  const globalDir = path.join(root, 'xdg-config', 'opencode')
  const projectDir = path.join(root, 'project')
  for (const directory of [
    path.join(root, 'home'),
    path.join(root, 'tmp'),
    path.join(root, 'xdg-config'),
    path.join(root, 'xdg-data'),
    path.join(root, 'xdg-state'),
    path.join(root, 'xdg-cache'),
    globalDir,
    projectDir,
  ]) await fs.mkdir(directory, { recursive: true, mode: 0o700 })

  const globalConfig = {
    $schema: 'https://opencode.ai/config.json',
    mcp: {
      'global-marker': marker('global-marker', 'global', statePath),
      'shared-marker': marker('shared-marker', 'global', statePath),
    },
  }
  const projectConfig = {
    $schema: 'https://opencode.ai/config.json',
    mcp: {
      'project-marker': marker('project-marker', 'project', statePath),
      'shared-marker': marker('shared-marker', 'project', statePath),
    },
  }
  await fs.writeFile(path.join(globalDir, 'opencode.json'), JSON.stringify(globalConfig, null, 2), { mode: 0o600 })
  await fs.writeFile(path.join(projectDir, 'opencode.json'), JSON.stringify(projectConfig, null, 2), { mode: 0o600 })
  await fs.writeFile(statePath, JSON.stringify({ servers: {}, calls: [] }), { mode: 0o600 })
  return { globalConfig, projectConfig, globalConfigPath: path.join(globalDir, 'opencode.json'), projectConfigPath: path.join(projectDir, 'opencode.json') }
}

function debugConfig(env, cwd) {
  const result = { command: `${executable} debug config --pure`, cwd }
  try {
    const child = execFileSync(executable, ['debug', 'config', '--pure'], {
      cwd,
      env,
      encoding: 'utf8',
      timeout: 10_000,
      maxBuffer: 4 * 1024 * 1024,
    })
    result.exit_code = 0
    result.resolved = summarizeResolvedConfig(parseJsonOutput(child))
    if (!result.resolved) result.parse_error = true
  } catch (error) {
    result.exit_code = error.status ?? null
    result.error = error.message
    result.stdout_tail = String(error.stdout ?? '').slice(-2000)
    result.stderr_tail = String(error.stderr ?? '').slice(-2000)
  }
  return result
}

async function launchServer(label, env, cwd) {
  const port = await reservePort()
  const baseUrl = `http://127.0.0.1:${port}`
  const child = spawn(executable, ['serve', '--pure', '--hostname', '127.0.0.1', '--port', String(port)], {
    cwd,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  let stdout = ''
  let stderr = ''
  child.stdout.on('data', (chunk) => { stdout = (stdout + String(chunk)).slice(-8000) })
  child.stderr.on('data', (chunk) => { stderr = (stderr + String(chunk)).slice(-8000) })
  try {
    const health = await waitForHealth(baseUrl, child)
    return { label, port, baseUrl, child, health, stdout, stderr }
  } catch (error) {
    await stopServer({ child })
    throw new Error(`${label}: ${error.message}; stderr=${stderr.slice(-1000)}`)
  }
}

async function stopServer(run) {
  if (!run?.child || run.child.exitCode !== null) return
  run.child.kill('SIGTERM')
  await Promise.race([once(run.child, 'exit'), sleep(4000)])
  if (run.child.exitCode === null) {
    run.child.kill('SIGKILL')
    await Promise.race([once(run.child, 'exit'), sleep(1000)])
  }
}

async function killRecordedMarkers(state) {
  const pids = [...new Set((state?.calls ?? []).map((event) => event.pid).filter((pid) => Number.isInteger(pid) && pid > 1))]
  for (const pid of pids) {
    try { process.kill(pid, 'SIGTERM') } catch (error) { if (error.code !== 'ESRCH') throw error }
  }
}

async function runCase({ label, cwd, statePath, env }) {
  const result = { label, debug_config: debugConfig(env, cwd) }
  let run
  try {
    run = await launchServer(label, env, cwd)
    result.health = run.health
    await sleep(350)
    result.mcp_status = summarizeMcpStatus(await requestJson(run.baseUrl, '/mcp'))
    await sleep(350)
    result.server = { port: run.port, base_url: run.baseUrl }
  } catch (error) {
    result.error = error.message
  } finally {
    await stopServer(run)
    await sleep(150)
    const state = await readState(statePath)
    result.markers = summarizeMarkers(state)
    await killRecordedMarkers(state)
  }
  return result
}

function markerCount(caseResult, mode, name) {
  return caseResult.markers?.markers?.[`${mode}:${name}`]?.starts ?? 0
}

function markerToolCalls(caseResult) {
  return allMarkerEntries(caseResult).reduce((total, markerResult) => total + (markerResult.tool_calls ?? 0), 0)
}

async function main() {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'chariox-opencode-native-config-'))
  const statePath = path.join(root, 'marker-state.json')
  const report = {
    schema: 'chariox.opencode_native_config_acceptance.v1',
    command: `node ${path.relative(repoRoot, scriptPath)}`,
    executable,
    version: null,
    fixture_root: root,
    fixture_cleaned: false,
    no_cargo: true,
    no_model_prompt: true,
    cases: {},
  }
  let configFixture
  try {
    report.version = execFileSync(executable, ['--version'], { encoding: 'utf8', timeout: 5000 }).trim()
    configFixture = await writeFixture(root, statePath)
    report.config_paths = {
      global: configFixture.globalConfigPath,
      project: configFixture.projectConfigPath,
      global_precedence_source: 'XDG_CONFIG_HOME/opencode/opencode.json via OPENCODE_CONFIG_DIR',
      project_precedence_source: 'project/opencode.json in server cwd',
    }
    const baseEnv = safeEnvironment(root)
    report.debug_paths = execFileSync(executable, ['debug', 'paths', '--pure'], {
      cwd: path.join(root, 'project'),
      env: baseEnv,
      encoding: 'utf8',
      timeout: 10_000,
    }).trim().split('\n')

    report.cases.ordinary = await runCase({
      label: 'ordinary-global-plus-project',
      root,
      cwd: path.join(root, 'project'),
      statePath,
      env: baseEnv,
    })

    await fs.writeFile(statePath, JSON.stringify({ servers: {}, calls: [] }), { mode: 0o600 })
    report.cases.inline_empty = await runCase({
      label: 'inline-empty-mcp-project-enabled',
      root,
      cwd: path.join(root, 'project'),
      statePath,
      env: safeEnvironment(root, { inlineConfig: { mcp: {} } }),
    })

    await fs.writeFile(statePath, JSON.stringify({ servers: {}, calls: [] }), { mode: 0o600 })
    const discoveryPermissions = {
      '*': 'deny',
      read: 'allow',
      glob: 'allow',
      grep: 'allow',
      list: 'allow',
    }
    report.discovery_input_policy = {
      mcp: {},
      permission: discoveryPermissions,
      agent_plan_permission: discoveryPermissions,
      OPENCODE_DISABLE_PROJECT_CONFIG: 'true',
    }
    report.cases.discovery_policy = await runCase({
      label: 'discovery-policy-inline-empty-project-disabled',
      root,
      cwd: path.join(root, 'project'),
      statePath,
      env: safeEnvironment(root, {
        inlineConfig: {
          mcp: {},
          permission: discoveryPermissions,
          agent: { plan: { permission: discoveryPermissions } },
        },
        disableProjectConfig: true,
      }),
    })

    const ordinary = report.cases.ordinary
    const inlineEmpty = report.cases.inline_empty
    const discovery = report.cases.discovery_policy
    const ordinaryNames = ordinary.debug_config?.resolved?.names ?? []
    const ordinarySharedMode = ordinary.debug_config?.resolved?.entries?.['shared-marker']?.marker_mode
    const discoveryGlobalStarts = markerCount(discovery, 'global', 'global-marker') + markerCount(discovery, 'global', 'shared-marker')
    const discoveryProjectStarts = markerCount(discovery, 'project', 'project-marker') + markerCount(discovery, 'project', 'shared-marker')
    const discoveryToolCalls = markerToolCalls(discovery)
    report.observations = {
      ordinary_resolved_mcp_names: ordinaryNames,
      ordinary_project_only_marker_is_additive: ordinaryNames.includes('project-marker'),
      ordinary_project_overrides_shared_marker: ordinarySharedMode === 'project',
      ordinary_shared_marker_mode: ordinarySharedMode,
      config_precedence_observed: 'With OPENCODE_CONFIG_DIR set to the provider global directory, project MCP entries are additive but the same-name shared-marker resolves to the global/custom-directory entry.',
      ordinary_marker_starts: ordinary.markers,
      inline_empty_global_marker_autostarted: markerCount(inlineEmpty, 'global', 'global-marker') > 0,
      inline_empty_project_marker_autostarted: markerCount(inlineEmpty, 'project', 'project-marker') > 0,
      inline_empty_resolved_mcp_names: inlineEmpty.debug_config?.resolved?.names ?? [],
      discovery_global_marker_autostarted: markerCount(discovery, 'global', 'global-marker') > 0,
      discovery_global_marker_process_starts: discoveryGlobalStarts,
      discovery_project_marker_process_starts: discoveryProjectStarts,
      discovery_resolved_mcp_names: discovery.debug_config?.resolved?.names ?? [],
      discovery_mcp_status_names: Object.keys(discovery.mcp_status ?? {}).sort(),
      discovery_marker_tool_calls: discoveryToolCalls,
      discovery_spawn_policy_prevented_marker_execution: discoveryGlobalStarts === 0 && discoveryProjectStarts === 0,
      discovery_had_no_marker_tool_calls: discoveryToolCalls === 0,
    }
    report.passed = report.version.length > 0
      && ordinaryNames.includes('global-marker')
      && ordinaryNames.includes('project-marker')
      && ordinarySharedMode === 'project'
      && discoveryProjectStarts === 0
      && discoveryToolCalls === 0
      && discoveryGlobalStarts === 0
    if (discoveryGlobalStarts > 0) {
      report.residual_gap = 'The exact discovery-style inline mcp={} plus OPENCODE_DISABLE_PROJECT_CONFIG=true still autostarted a marker from global config; inline mcp removal does not prove global MCP isolation for native OpenCode.'
    }
  } catch (error) {
    report.passed = false
    report.error = error.stack ?? error.message
  } finally {
    report.fixture_cleaned = true
    await fs.rm(root, { recursive: true, force: true })
  }
  console.log(JSON.stringify(report, null, 2))
  if (!report.passed) process.exitCode = 1
}

await main()
