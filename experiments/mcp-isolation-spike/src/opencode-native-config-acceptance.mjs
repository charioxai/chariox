#!/usr/bin/env node

// Native-only acceptance probe for PR364 discovery configuration.
//
// This deliberately launches the installed official OpenCode binary directly.
// It does not build or start Chariox, and it never prompts a model. The local
// fake MCP is only a marker: its start/initialize/tools/list/tools/call events
// are written to a disposable state file so global-config autostart is visible.

import { execFileSync, spawn } from 'node:child_process'
import { once } from 'node:events'
import { fileURLToPath, pathToFileURL } from 'node:url'
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
const emittedEnvironmentFile = process.argv.find((argument) => argument.startsWith('--emitted-child-env='))?.slice('--emitted-child-env='.length)
  || process.env.CHARIOX_EMITTED_CHILD_ENV_FILE?.trim()

const replayEnvironmentKeys = new Set([
  'CHARIOX_MANAGED_PROVIDER_PROCESS',
  'CHARIOX_PROVIDER_PROCESS_KEY',
  'CHARIOX_PROVIDER_RUN_ID',
  'GIT_SSH_COMMAND',
  'HOME',
  'LANG',
  'LC_ALL',
  'OPENCODE_CONFIG',
  'OPENCODE_CONFIG_CONTENT',
  'OPENCODE_CONFIG_DIR',
  'OPENCODE_DISABLE_PROJECT_CONFIG',
  'PATH',
  'SSH_AUTH_SOCK',
  'TERM',
  'XDG_CACHE_HOME',
  'XDG_CONFIG_HOME',
  'XDG_DATA_HOME',
  'XDG_STATE_HOME',
])

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

const realHome = process.env.HOME || os.homedir()
const realProviderDataHome = process.env.XDG_DATA_HOME || path.join(realHome, '.local', 'share')

function safeEnvironment(root, {
  configDir,
  configFile,
  inlineConfig,
  disableProjectConfig = false,
  pluginMarkerPath,
  xdgConfigHome,
} = {}) {
  const env = {
    PATH: process.env.PATH || '/usr/local/bin:/usr/bin:/bin',
    HOME: path.join(root, 'home'),
    TMPDIR: path.join(root, 'tmp'),
    XDG_CONFIG_HOME: xdgConfigHome || path.join(root, 'home', 'xdg-config'),
    // Keep the provider's real auth/data root. No auth file is opened by this
    // harness because it never creates a session or sends a model prompt.
    XDG_DATA_HOME: realProviderDataHome,
    XDG_STATE_HOME: path.join(root, 'xdg-state'),
    XDG_CACHE_HOME: path.join(root, 'xdg-cache'),
    LANG: 'C',
    LC_ALL: 'C',
  }
  if (configDir !== undefined) env.OPENCODE_CONFIG_DIR = configDir
  if (configFile !== undefined) env.OPENCODE_CONFIG = configFile
  if (inlineConfig !== undefined) env.OPENCODE_CONFIG_CONTENT = JSON.stringify(inlineConfig)
  if (disableProjectConfig) env.OPENCODE_DISABLE_PROJECT_CONFIG = 'true'
  if (pluginMarkerPath !== undefined) env.CHARIOX_NATIVE_PLUGIN_MARKER = pluginMarkerPath
  return env
}

function parseEnvironmentSnapshot(value) {
  if (!value || typeof value !== 'string') return null
  const text = value.trim()
  if (!text) return null
  try {
    const parsed = JSON.parse(text)
    if (Array.isArray(parsed)) return Object.fromEntries(parsed)
    if (parsed?.environment && typeof parsed.environment === 'object') return parsed.environment
    if (parsed?.child_environment && typeof parsed.child_environment === 'object') return parsed.child_environment
    if (parsed && typeof parsed === 'object') return parsed
  } catch {}
  const entries = value.split(/\0|\r?\n/).filter(Boolean)
  const environment = {}
  for (const entry of entries) {
    const separator = entry.indexOf('=')
    if (separator > 0) environment[entry.slice(0, separator)] = entry.slice(separator + 1)
  }
  return Object.keys(environment).length > 0 ? environment : null
}

async function readEnvironmentSnapshot(filePath) {
  if (!filePath) return null
  const contents = filePath === '-' ? await new Promise((resolve, reject) => {
    let value = ''
    process.stdin.setEncoding('utf8')
    process.stdin.on('data', (chunk) => { value += chunk })
    process.stdin.once('end', () => resolve(value))
    process.stdin.once('error', reject)
  }) : await fs.readFile(filePath, 'utf8')
  return parseEnvironmentSnapshot(contents)
}

function redactEnvironmentValue(name, value) {
  if (value === undefined) return null
  if (name === 'OPENCODE_CONFIG_CONTENT') {
    try {
      const config = JSON.parse(value)
      return {
        kind: 'json',
        keys: Object.keys(config).sort(),
        mcp_names: Object.keys(config?.mcp ?? {}).sort(),
        plugin_count: Array.isArray(config?.plugin) ? config.plugin.length : 0,
        sha256_like: value.length,
      }
    } catch {
      return { kind: 'invalid-json', length: value.length }
    }
  }
  if (/AUTH|TOKEN|SECRET|PASSWORD|CREDENTIAL|COOKIE|KEY/i.test(name)) return '[redacted]'
  return value
}

function summarizeEnvironment(environment) {
  return Object.fromEntries(Object.keys(environment || {}).sort().map((name) => [
    name,
    redactEnvironmentValue(name, environment[name]),
  ]))
}

function selectedEnvironment(environment) {
  return Object.fromEntries(Object.entries(environment || {}).filter(([name, value]) => replayEnvironmentKeys.has(name) && typeof value === 'string'))
}

function rewriteFixturePath(value, replacements) {
  if (typeof value !== 'string') return value
  return replacements.reduce((result, [from, to]) => from ? result.split(from).join(to) : result, value)
}

function replayEnvironment(root, snapshot, configFixture, pluginEventsPath, statePath) {
  const freshXdgConfigHome = path.join(root, 'home', 'opencode-discovery-xdg')
  const source = selectedEnvironment(snapshot)
  const env = safeEnvironment(root, { pluginMarkerPath: pluginEventsPath, xdgConfigHome: freshXdgConfigHome })
  for (const name of replayEnvironmentKeys) {
    if (source[name] !== undefined) env[name] = source[name]
  }
  // Re-home only disposable fixture paths. Do not copy ambient credentials or
  // data/auth files from the production child snapshot.
  const replacements = [
    [path.dirname(configFixture.globalConfigPath), configFixture.inheritedGlobalDir],
  ]
  for (const name of ['HOME', 'XDG_CONFIG_HOME', 'XDG_STATE_HOME', 'XDG_CACHE_HOME']) {
    if (name === 'HOME' || name === 'XDG_CONFIG_HOME') env[name] = name === 'HOME'
      ? path.join(root, 'home')
      : freshXdgConfigHome
    else if (name === 'XDG_STATE_HOME') env[name] = path.join(root, 'xdg-state')
    else env[name] = path.join(root, 'xdg-cache')
  }
  for (const name of ['OPENCODE_CONFIG', 'OPENCODE_CONFIG_DIR']) {
    if (env[name] !== undefined) {
      env[name] = name === 'OPENCODE_CONFIG'
        ? configFixture.globalConfigPath
        : configFixture.inheritedGlobalDir
    }
  }
  let substitutedMcpNames = []
  if (typeof env.OPENCODE_CONFIG_CONTENT === 'string') {
    try {
      const inlineConfig = JSON.parse(env.OPENCODE_CONFIG_CONTENT)
      if (inlineConfig?.mcp && typeof inlineConfig.mcp === 'object' && !Array.isArray(inlineConfig.mcp)) {
        substitutedMcpNames = Object.keys(inlineConfig.mcp).sort()
        inlineConfig.mcp = Object.fromEntries(substitutedMcpNames.map((name) => [
          name,
          marker(name, 'emitted', statePath),
        ]))
        env.OPENCODE_CONFIG_CONTENT = JSON.stringify(inlineConfig)
      }
    } catch {
      // The replay still reports the malformed inline content; OpenCode will
      // reject it without any network-capable MCP definition being invented.
    }
  }
  for (const name of Object.keys(env)) env[name] = rewriteFixturePath(env[name], replacements)
  env.XDG_DATA_HOME = realProviderDataHome
  env.CHARIOX_NATIVE_PLUGIN_MARKER = pluginEventsPath
  return { env, freshXdgConfigHome, source, substitutedMcpNames }
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
  const agents = config?.agent && typeof config.agent === 'object' ? config.agent : {}
  const plugins = Array.isArray(config?.plugin) ? config.plugin : []
  return {
    names: Object.keys(mcp).sort(),
    entries: Object.fromEntries(Object.entries(mcp).map(([name, value]) => [name, {
      type: value?.type ?? null,
      enabled: value?.enabled ?? null,
      marker_mode: commandMode(value?.command),
    }])),
    plugin_entries: plugins.map((plugin) => typeof plugin === 'string' && plugin.endsWith('native-marker-plugin.mjs')
      ? 'native-marker-plugin'
      : typeof plugin === 'string' ? plugin : '[tuple]'),
    agent_names: Object.keys(agents).sort(),
    agent_permission_stars: Object.fromEntries(Object.entries(agents).map(([name, value]) => [
      name,
      value?.permission?.['*'] ?? null,
    ])),
    permission_star: config?.permission?.['*'] ?? null,
    plan_permission_star: agents?.plan?.permission?.['*'] ?? null,
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

async function readPluginEvents(pluginEventsPath) {
  try {
    const lines = (await fs.readFile(pluginEventsPath, 'utf8')).trim().split('\n').filter(Boolean)
    return lines.map((line) => JSON.parse(line))
  } catch {
    return []
  }
}

async function resetEvidence(statePath, pluginEventsPath) {
  await fs.writeFile(statePath, JSON.stringify({ servers: {}, calls: [] }), { mode: 0o600 })
  await fs.writeFile(pluginEventsPath, '', { mode: 0o600 })
}

async function fileMetadata(filePath) {
  try {
    const stat = await fs.stat(filePath)
    return { exists: true, size: stat.size, mtime_ms: stat.mtimeMs, mode: stat.mode }
  } catch (error) {
    if (error.code === 'ENOENT') return { exists: false }
    throw error
  }
}

function summarizePluginEvents(events) {
  return {
    loads: events.filter((event) => event.kind === 'load').length,
    events: events.map((event) => ({ kind: event.kind })),
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

async function writeFixture(root, statePath, pluginEventsPath) {
  const inheritedGlobalDir = path.join(root, 'inherited-global')
  const isolatedGlobalDir = path.join(root, 'home', 'xdg-config', 'opencode')
  const projectDir = path.join(root, 'project')
  const disabledProjectDir = path.join(root, 'project-disabled')
  const pluginPath = path.join(root, 'native-marker-plugin.mjs')
  for (const directory of [
    path.join(root, 'home'),
    path.join(root, 'tmp'),
    path.join(root, 'xdg-data'),
    path.join(root, 'xdg-state'),
    path.join(root, 'xdg-cache'),
    inheritedGlobalDir,
    isolatedGlobalDir,
    projectDir,
    disabledProjectDir,
  ]) await fs.mkdir(directory, { recursive: true, mode: 0o700 })

  const pluginSource = [
    "import { appendFileSync } from 'node:fs'",
    "const marker = process.env.CHARIOX_NATIVE_PLUGIN_MARKER",
    "if (marker) appendFileSync(marker, JSON.stringify({ kind: 'load', pid: process.pid }) + '\\n')",
    'export default async function NativeMarkerPlugin() { return {} }',
    '',
  ].join('\n')
  const globalConfig = {
    $schema: 'https://opencode.ai/config.json',
    mcp: {
      'global-marker': marker('global-marker', 'global', statePath),
      'shared-marker': marker('shared-marker', 'global', statePath),
    },
    plugin: [pathToFileURL(pluginPath).href],
    agent: {
      plan: { permission: { '*': 'allow', read: 'allow' } },
    },
  }
  const projectConfig = {
    $schema: 'https://opencode.ai/config.json',
    mcp: {
      'project-marker': marker('project-marker', 'project', statePath),
      'shared-marker': marker('shared-marker', 'project', statePath),
    },
  }
  const disabledMcp = Object.fromEntries(Object.entries(globalConfig.mcp).map(([name, value]) => [
    name,
    { ...value, enabled: false },
  ]))
  const disabledProjectConfig = {
    $schema: 'https://opencode.ai/config.json',
    mcp: {
      ...disabledMcp,
      'project-marker': marker('project-marker', 'project', statePath),
    },
  }
  const isolatedConfig = { $schema: 'https://opencode.ai/config.json' }
  await fs.writeFile(pluginPath, pluginSource, { mode: 0o600 })
  await fs.writeFile(path.join(inheritedGlobalDir, 'opencode.json'), JSON.stringify(globalConfig, null, 2), { mode: 0o600 })
  await fs.writeFile(path.join(isolatedGlobalDir, 'opencode.json'), JSON.stringify(isolatedConfig, null, 2), { mode: 0o600 })
  await fs.writeFile(path.join(projectDir, 'opencode.json'), JSON.stringify(projectConfig, null, 2), { mode: 0o600 })
  await fs.writeFile(path.join(disabledProjectDir, 'opencode.json'), JSON.stringify(disabledProjectConfig, null, 2), { mode: 0o600 })
  await fs.writeFile(statePath, JSON.stringify({ servers: {}, calls: [] }), { mode: 0o600 })
  await fs.writeFile(pluginEventsPath, '', { mode: 0o600 })
  return {
    globalConfig,
    projectConfig,
    disabledProjectConfig,
    inheritedGlobalDir,
    isolatedGlobalDir,
    globalConfigPath: path.join(inheritedGlobalDir, 'opencode.json'),
    isolatedConfigPath: path.join(isolatedGlobalDir, 'opencode.json'),
    projectConfigPath: path.join(projectDir, 'opencode.json'),
    disabledProjectConfigPath: path.join(disabledProjectDir, 'opencode.json'),
    pluginPath,
    pluginUrl: pathToFileURL(pluginPath).href,
  }
}

function debugConfig(env, cwd, { pure = false } = {}) {
  const args = ['debug', 'config']
  if (pure) args.push('--pure')
  const result = { command: `${executable} ${args.join(' ')}`, cwd }
  try {
    const child = execFileSync(executable, args, {
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

async function launchServer(label, env, cwd, { pure = false } = {}) {
  const port = await reservePort()
  const baseUrl = `http://127.0.0.1:${port}`
  const args = ['serve']
  if (pure) args.push('--pure')
  args.push('--hostname', '127.0.0.1', '--port', String(port))
  const child = spawn(executable, args, {
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
  return pids
}

function livePids(pids) {
  return pids.filter((pid) => {
    try {
      process.kill(pid, 0)
      return true
    } catch (error) {
      if (error.code === 'ESRCH') return false
      if (error.code === 'EPERM') return true
      throw error
    }
  })
}

async function cleanupRecordedMarkers(state) {
  const pids = await killRecordedMarkers(state)
  const deadline = Date.now() + 1500
  let live = livePids(pids)
  while (live.length > 0 && Date.now() < deadline) {
    await sleep(50)
    live = livePids(pids)
  }
  if (live.length > 0) {
    for (const pid of live) {
      try { process.kill(pid, 'SIGKILL') } catch (error) { if (error.code !== 'ESRCH') throw error }
    }
    await sleep(50)
    live = livePids(live)
  }
  return { recorded_pids: pids, live_pids_after_exit: live }
}

async function runCase({ label, cwd, statePath, pluginEventsPath, env, pure = false }) {
  await resetEvidence(statePath, pluginEventsPath)
  const result = { label, pure, debug_config: debugConfig(env, cwd, { pure }) }
  let run
  try {
    run = await launchServer(label, env, cwd, { pure })
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
    result.plugin = summarizePluginEvents(await readPluginEvents(pluginEventsPath))
    result.cleanup = await cleanupRecordedMarkers(state)
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
    command: `node ${path.relative(repoRoot, scriptPath)}${emittedEnvironmentFile ? ` --emitted-child-env=${emittedEnvironmentFile}` : ''}`,
    executable,
    version: null,
    fixture_root: root,
    fixture_cleaned: false,
    no_cargo: true,
    no_model_prompt: true,
    emitted_child_environment_input: emittedEnvironmentFile || null,
    cases: {},
  }
  let configFixture
  let authPath
  let authMetadataBefore
  try {
    report.version = execFileSync(executable, ['--version'], { encoding: 'utf8', timeout: 5000 }).trim()
    const pluginEventsPath = path.join(root, 'plugin-events.jsonl')
    authPath = path.join(realProviderDataHome, 'opencode', 'auth.json')
    authMetadataBefore = await fileMetadata(authPath)
    configFixture = await writeFixture(root, statePath, pluginEventsPath)
    report.config_paths = {
      global: configFixture.globalConfigPath,
      isolated_xdg: configFixture.isolatedConfigPath,
      project: configFixture.projectConfigPath,
      project_with_enabled_false: configFixture.disabledProjectConfigPath,
      plugin: configFixture.pluginPath,
      global_precedence_source: 'disposable inherited-global/opencode.json via OPENCODE_CONFIG_DIR',
      project_precedence_source: 'project/opencode.json in server cwd',
    }
    const sourceEnv = safeEnvironment(root, {
      configDir: configFixture.inheritedGlobalDir,
      configFile: configFixture.globalConfigPath,
      pluginMarkerPath: pluginEventsPath,
    })
    const isolatedXdgConfigHome = path.join(root, 'home', 'opencode-discovery-xdg')
    const isolatedEnv = safeEnvironment(root, {
      pluginMarkerPath: pluginEventsPath,
      xdgConfigHome: isolatedXdgConfigHome,
    })
    report.provider_auth_boundary = {
      parent_home: realHome,
      preserved_xdg_data_home: realProviderDataHome,
      child_xdg_data_home: realProviderDataHome,
      auth_location_preserved: authPath,
      auth_file_present_before: authMetadataBefore.exists,
      credentials_read_by_harness: false,
      credentials_copied_or_deleted: false,
      isolated_config_env_removes: ['OPENCODE_CONFIG_DIR', 'OPENCODE_CONFIG'],
    }
    report.parent_config_environment = {
      OPENCODE_CONFIG: sourceEnv.OPENCODE_CONFIG,
      OPENCODE_CONFIG_DIR: sourceEnv.OPENCODE_CONFIG_DIR,
      XDG_CONFIG_HOME: sourceEnv.XDG_CONFIG_HOME,
      XDG_DATA_HOME: sourceEnv.XDG_DATA_HOME,
    }
    report.fresh_discovery_xdg_config_home = isolatedXdgConfigHome
    report.debug_paths = execFileSync(executable, ['debug', 'paths'], {
      cwd: path.join(root, 'project'),
      env: sourceEnv,
      encoding: 'utf8',
      timeout: 10_000,
    }).trim().split('\n')

    report.cases.ordinary = await runCase({
      label: 'ordinary-global-plus-project',
      cwd: path.join(root, 'project'),
      statePath,
      pluginEventsPath,
      env: sourceEnv,
    })

    report.cases.inline_empty = await runCase({
      label: 'inline-empty-mcp-project-enabled',
      cwd: path.join(root, 'project'),
      statePath,
      pluginEventsPath,
      env: safeEnvironment(root, {
        configDir: configFixture.inheritedGlobalDir,
        configFile: configFixture.globalConfigPath,
        inlineConfig: { mcp: {} },
        pluginMarkerPath: pluginEventsPath,
      }),
    })

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
      cwd: path.join(root, 'project'),
      statePath,
      pluginEventsPath,
      env: safeEnvironment(root, {
        configDir: configFixture.inheritedGlobalDir,
        configFile: configFixture.globalConfigPath,
        inlineConfig: {
          mcp: {},
          permission: discoveryPermissions,
          agent: { plan: { permission: discoveryPermissions } },
        },
        disableProjectConfig: true,
        pluginMarkerPath: pluginEventsPath,
      }),
    })

    report.cases.isolated_xdg = await runCase({
      label: 'isolated-xdg-no-config-overrides-project-disabled',
      cwd: path.join(root, 'project'),
      statePath,
      pluginEventsPath,
      env: safeEnvironment(root, {
        inlineConfig: {
          mcp: {},
          permission: discoveryPermissions,
          agent: { plan: { permission: discoveryPermissions } },
        },
        disableProjectConfig: true,
        pluginMarkerPath: pluginEventsPath,
      }),
    })

    report.cases.explicit_project_disabled = await runCase({
      label: 'explicit-project-enabled-false-global-mcp',
      cwd: path.join(root, 'project-disabled'),
      statePath,
      pluginEventsPath,
      env: safeEnvironment(root, {
        configDir: configFixture.inheritedGlobalDir,
        configFile: configFixture.globalConfigPath,
        pluginMarkerPath: pluginEventsPath,
      }),
    })

    const explicitDisabledMcp = Object.fromEntries(Object.entries(configFixture.globalConfig.mcp).map(([name, value]) => [
      name,
      { ...value, enabled: false },
    ]))
    report.explicit_disabled_mcp_names = Object.keys(explicitDisabledMcp).sort()
    report.cases.explicit_inline_disabled = await runCase({
      label: 'explicit-inline-enabled-false-global-mcp',
      cwd: path.join(root, 'project'),
      statePath,
      pluginEventsPath,
      env: safeEnvironment(root, {
        configDir: configFixture.inheritedGlobalDir,
        configFile: configFixture.globalConfigPath,
        inlineConfig: {
          mcp: explicitDisabledMcp,
          permission: discoveryPermissions,
          agent: { plan: { permission: discoveryPermissions } },
        },
        disableProjectConfig: true,
        pluginMarkerPath: pluginEventsPath,
      }),
    })

    report.cases.inline_plugin_inheritance = await runCase({
      label: 'red-inline-plugin-inheritance-with-fresh-xdg',
      cwd: path.join(root, 'project'),
      statePath,
      pluginEventsPath,
      env: safeEnvironment(root, {
        inlineConfig: {
          mcp: {},
          plugin: [configFixture.pluginUrl],
          permission: discoveryPermissions,
          agent: { plan: { permission: discoveryPermissions } },
        },
        disableProjectConfig: true,
        pluginMarkerPath: pluginEventsPath,
        xdgConfigHome: isolatedXdgConfigHome,
      }),
    })

    if (emittedEnvironmentFile) {
      const snapshot = await readEnvironmentSnapshot(emittedEnvironmentFile)
      if (!snapshot) throw new Error(`could not parse emitted child environment snapshot ${emittedEnvironmentFile}`)
      const replay = replayEnvironment(root, snapshot, configFixture, pluginEventsPath, statePath)
      const actualXdg = snapshot.XDG_CONFIG_HOME
      const actualHome = snapshot.HOME
      const actualData = snapshot.XDG_DATA_HOME
      const actualConfigKeys = ['OPENCODE_CONFIG', 'OPENCODE_CONFIG_DIR', 'OPENCODE_CONFIG_CONTENT', 'OPENCODE_DISABLE_PROJECT_CONFIG']
        .filter((name) => snapshot[name] !== undefined)
      report.actual_emitted_child_environment = {
        source_file: emittedEnvironmentFile,
        observed: summarizeEnvironment(snapshot),
        config_keys_present: actualConfigKeys,
        config_overrides_absent: snapshot.OPENCODE_CONFIG === undefined && snapshot.OPENCODE_CONFIG_DIR === undefined,
        xdg_config_under_home: typeof actualXdg === 'string' && typeof actualHome === 'string'
          ? path.relative(actualHome, actualXdg) !== '' && !path.relative(actualHome, actualXdg).startsWith('..')
          : false,
        xdg_data_unchanged: actualData === realProviderDataHome,
        replay_projection: {
          home: replay.env.HOME,
          xdg_config_home: replay.freshXdgConfigHome,
          xdg_data_home: replay.env.XDG_DATA_HOME,
          inherited_config_vars_replayed: ['OPENCODE_CONFIG', 'OPENCODE_CONFIG_DIR'].filter((name) => replay.env[name] !== undefined),
          inline_mcp_names_substituted_to_local_markers: replay.substitutedMcpNames,
          credentials_copied: false,
        },
      }
      report.cases.actual_emitted_child_environment = await runCase({
        label: 'actual-emitted-child-environment-replay',
        cwd: path.join(root, 'project'),
        statePath,
        pluginEventsPath,
        env: replay.env,
      })
    }

    const ordinary = report.cases.ordinary
    const inlineEmpty = report.cases.inline_empty
    const discovery = report.cases.discovery_policy
    const isolated = report.cases.isolated_xdg
    const explicitProjectDisabled = report.cases.explicit_project_disabled
    const explicitInlineDisabled = report.cases.explicit_inline_disabled
    const inlinePluginInheritance = report.cases.inline_plugin_inheritance
    const actualEmitted = report.cases.actual_emitted_child_environment
    const ordinaryNames = ordinary.debug_config?.resolved?.names ?? []
    const ordinarySharedMode = ordinary.debug_config?.resolved?.entries?.['shared-marker']?.marker_mode
    const discoveryGlobalStarts = markerCount(discovery, 'global', 'global-marker') + markerCount(discovery, 'global', 'shared-marker')
    const discoveryProjectStarts = markerCount(discovery, 'project', 'project-marker') + markerCount(discovery, 'project', 'shared-marker')
    const discoveryToolCalls = markerToolCalls(discovery)
    const globalStarts = (caseResult) => markerCount(caseResult, 'global', 'global-marker') + markerCount(caseResult, 'global', 'shared-marker')
    const projectStarts = (caseResult) => markerCount(caseResult, 'project', 'project-marker') + markerCount(caseResult, 'project', 'shared-marker')
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
      inherited_plugin_loads: discovery.plugin?.loads ?? 0,
      inherited_global_plan_permission: discovery.debug_config?.resolved?.plan_permission_star ?? null,
      isolated_xdg_global_mcp_starts: globalStarts(isolated),
      isolated_xdg_project_mcp_starts: projectStarts(isolated),
      isolated_xdg_plugin_loads: isolated.plugin?.loads ?? 0,
      isolated_xdg_resolved_mcp_names: isolated.debug_config?.resolved?.names ?? [],
      isolated_xdg_plan_permission: isolated.debug_config?.resolved?.plan_permission_star ?? null,
      explicit_project_disabled_global_mcp_starts: globalStarts(explicitProjectDisabled),
      explicit_project_disabled_project_mcp_starts: projectStarts(explicitProjectDisabled),
      explicit_project_disabled_plugin_loads: explicitProjectDisabled.plugin?.loads ?? 0,
      explicit_inline_disabled_global_mcp_starts: globalStarts(explicitInlineDisabled),
      explicit_inline_disabled_project_mcp_starts: projectStarts(explicitInlineDisabled),
      explicit_inline_disabled_plugin_loads: explicitInlineDisabled.plugin?.loads ?? 0,
      explicit_inline_disabled_plan_permission: explicitInlineDisabled.debug_config?.resolved?.plan_permission_star ?? null,
      inline_plugin_inheritance_loads: inlinePluginInheritance.plugin?.loads ?? 0,
      inline_plugin_inheritance_is_red: (inlinePluginInheritance.plugin?.loads ?? 0) > 0,
      inline_plugin_inheritance_mcp_starts: globalStarts(inlinePluginInheritance) + projectStarts(inlinePluginInheritance),
      actual_emitted_global_mcp_starts: actualEmitted ? globalStarts(actualEmitted) : null,
      actual_emitted_project_mcp_starts: actualEmitted ? projectStarts(actualEmitted) : null,
      actual_emitted_plugin_loads: actualEmitted?.plugin?.loads ?? null,
      actual_emitted_marker_cleanup: actualEmitted ? actualEmitted.cleanup?.live_pids_after_exit?.length === 0 : null,
    }
    const isolatedBoundaryPasses = globalStarts(isolated) === 0
      && isolated.plugin?.loads === 0
      && isolated.debug_config?.resolved?.plan_permission_star !== 'allow'
    const inlineDisableBoundaryPasses = globalStarts(explicitInlineDisabled) === 0
      && explicitInlineDisabled.plugin?.loads > 0
      && explicitInlineDisabled.debug_config?.resolved?.plan_permission_star === 'deny'
    report.passed = report.version.length > 0
      && ordinaryNames.includes('global-marker')
      && ordinaryNames.includes('project-marker')
      && ordinarySharedMode === 'global'
      && isolatedBoundaryPasses
      && inlineDisableBoundaryPasses
      && discoveryProjectStarts === 0
      && discoveryToolCalls === 0
      && discoveryGlobalStarts === 0
      && (report.actual_emitted_child_environment === undefined
        || (report.actual_emitted_child_environment.config_overrides_absent
          && report.actual_emitted_child_environment.xdg_config_under_home
          && report.actual_emitted_child_environment.xdg_data_unchanged
          && globalStarts(actualEmitted) === 0
          && projectStarts(actualEmitted) === 0
          && actualEmitted.plugin?.loads === 0
          && actualEmitted.cleanup?.live_pids_after_exit?.length === 0))
    if (discoveryGlobalStarts > 0) {
      report.residual_gap = 'The exact discovery-style inline mcp={} plus OPENCODE_DISABLE_PROJECT_CONFIG=true still autostarted a marker from the inherited global config; inline mcp removal does not prove native global MCP isolation.'
    }
    if (inlinePluginInheritance.plugin?.loads > 0) {
      report.inline_plugin_residual_gap = 'An inline plugin in OPENCODE_CONFIG_CONTENT still loaded with fresh XDG config and no OPENCODE_CONFIG/OPENCODE_CONFIG_DIR; the native boundary covers global config sources, not inherited inline plugins.'
    }
  } catch (error) {
    report.passed = false
    report.error = error.stack ?? error.message
  } finally {
    if (configFixture) {
      const authMetadataAfter = await fileMetadata(authPath)
      report.provider_auth_boundary ??= {}
      report.provider_auth_boundary.auth_file_unchanged = JSON.stringify(authMetadataBefore) === JSON.stringify(authMetadataAfter)
      report.provider_auth_boundary.auth_file_present_after = authMetadataAfter.exists
    }
    report.fixture_cleaned = true
    await fs.rm(root, { recursive: true, force: true })
  }
  console.log(JSON.stringify(report, null, 2))
  if (!report.passed) process.exitCode = 1
}

await main()
