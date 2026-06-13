#!/usr/bin/env node
import { spawn } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..', '..')
const localDrill = path.join(scriptDir, 'live-workspace-live-sync-drill.mjs')
const remoteDrill = path.join(scriptDir, 'live-remote-workspace-live-sync-drill.mjs')
const localPermissionDrill = path.join(scriptDir, 'live-workspace-live-sync-permission-drill.mjs')
const remotePermissionDrill = path.join(scriptDir, 'live-remote-workspace-live-sync-permission-drill.mjs')

const codexModelId = process.env.ARROBA_WORKSPACE_LIVE_SYNC_CODEX_MODEL ?? process.env.ARROBA_CODEX_MODEL ?? 'gpt-5.5'
const CODEX_MODEL = ['--provider-model', `codex=${codexModelId}`]
const OPENCODE_MODEL = ['--provider-model', 'opencode=opencode/gpt-5.2']

const MATRIX = [
  scenario('local-off-codex', 'local off-mode Codex smoke', localDrill, ['--provider', 'codex', ...CODEX_MODEL, '--mode', 'off']),
  scenario('local-managed-codex', 'local managed Codex two-target fanout', localDrill, ['--provider', 'codex', ...CODEX_MODEL, '--mode', 'managed', '--managed-target-count', '2']),
  scenario('local-tracked-codex', 'local tracked Codex cross-branch bidirectional fanout', localDrill, ['--provider', 'codex', ...CODEX_MODEL, '--mode', 'tracked', '--tracked-target-count', '2', '--tracked-bidirectional', '--target-branch', 'live-sync-tracked-parity-target', '--timeout-ms', '700000']),
  scenario('local-permission-codex', 'local Codex permission gating', localPermissionDrill, ['--provider', 'codex', ...CODEX_MODEL]),
  scenario('remote-managed-codex', 'same-host remote managed Codex fanout', remoteDrill, ['--provider', 'codex', ...CODEX_MODEL, '--mode', 'managed', '--managed-target-count', '2', '--full'], { remote: true }),
  scenario('remote-tracked-codex', 'same-host remote tracked Codex fanout', remoteDrill, ['--provider', 'codex', ...CODEX_MODEL, '--mode', 'tracked', '--tracked-target-count', '2', '--tracked-bidirectional', '--target-branch', 'remote-live-sync-tracked-parity-target', '--full', '--timeout-ms', '700000'], { remote: true }),
  scenario('remote-tracked-restart-codex', 'same-host remote tracked Codex relay restart recovery', remoteDrill, ['--provider', 'codex', ...CODEX_MODEL, '--mode', 'tracked', '--tracked-target-count', '2', '--tracked-bidirectional', '--target-branch', 'remote-live-sync-restart-tracked-target', '--full', '--restart-relay-before-sync', '--timeout-ms', '700000'], { remote: true }),
  scenario('remote-permission-codex', 'same-host remote Codex permission gating', remotePermissionDrill, ['--provider', 'codex', ...CODEX_MODEL], { remote: true }),
  scenario('hetzner-managed-codex', 'Hetzner remote managed Codex unsupported-platform fast-fail', remoteDrill, ['--provider', 'codex', ...CODEX_MODEL, '--mode', 'managed', '--managed-target-count', '2', '--full', '--hetzner-worker', '--timeout-ms', '120000'], {
    remote: true,
    hetzner: true,
    expectedFailure: true,
    expectedOutputIncludes: 'managed mode needs selective write fencing',
  }),
  scenario('hetzner-tracked-codex', 'Hetzner remote tracked Codex fanout', remoteDrill, ['--provider', 'codex', ...CODEX_MODEL, '--mode', 'tracked', '--tracked-target-count', '2', '--tracked-bidirectional', '--target-branch', 'hetzner-live-sync-tracked-parity-target', '--full', '--hetzner-worker', '--timeout-ms', '1000000'], { remote: true, hetzner: true }),
  scenario('hetzner-permission-codex', 'Hetzner remote tracked Codex permission gating', remotePermissionDrill, ['--provider', 'codex', ...CODEX_MODEL, '--mode', 'tracked', '--hetzner-worker', '--timeout-ms', '360000'], { remote: true, hetzner: true }),
  scenario('local-managed-opencode', 'local managed OpenCode Zen two-target fanout', localDrill, ['--provider', 'opencode', ...OPENCODE_MODEL, '--mode', 'managed', '--managed-target-count', '2', '--timeout-ms', '700000'], { opencode: true }),
  scenario('local-tracked-opencode', 'local tracked OpenCode Zen cross-branch bidirectional fanout', localDrill, ['--provider', 'opencode', ...OPENCODE_MODEL, '--mode', 'tracked', '--tracked-target-count', '2', '--tracked-bidirectional', '--target-branch', 'live-sync-opencode-tracked-parity-target', '--timeout-ms', '700000'], { opencode: true }),
  scenario('local-permission-opencode', 'local OpenCode Zen permission gating', localPermissionDrill, ['--provider', 'opencode', ...OPENCODE_MODEL, '--timeout-ms', '700000'], { opencode: true }),
  scenario('remote-managed-opencode', 'same-host remote managed OpenCode Zen fanout', remoteDrill, ['--provider', 'opencode', ...OPENCODE_MODEL, '--mode', 'managed', '--managed-target-count', '2', '--full', '--timeout-ms', '700000'], { remote: true, opencode: true }),
  scenario('remote-tracked-opencode', 'same-host remote tracked OpenCode Zen fanout', remoteDrill, ['--provider', 'opencode', ...OPENCODE_MODEL, '--mode', 'tracked', '--tracked-target-count', '2', '--tracked-bidirectional', '--target-branch', 'remote-live-sync-opencode-tracked-parity-target', '--full', '--timeout-ms', '700000'], { remote: true, opencode: true }),
  scenario('remote-permission-opencode', 'same-host remote OpenCode Zen permission gating', remotePermissionDrill, ['--provider', 'opencode', ...OPENCODE_MODEL, '--timeout-ms', '700000'], { remote: true, opencode: true }),
  scenario('hetzner-managed-opencode', 'Hetzner remote managed OpenCode Zen unsupported-platform fast-fail', remoteDrill, ['--provider', 'opencode', ...OPENCODE_MODEL, '--mode', 'managed', '--managed-target-count', '2', '--full', '--hetzner-worker', '--timeout-ms', '120000'], {
    remote: true,
    hetzner: true,
    opencode: true,
    expectedFailure: true,
    expectedOutputIncludes: 'managed mode needs selective write fencing',
  }),
  scenario('hetzner-tracked-opencode', 'Hetzner remote tracked OpenCode Zen fanout', remoteDrill, ['--provider', 'opencode', ...OPENCODE_MODEL, '--mode', 'tracked', '--tracked-target-count', '2', '--tracked-bidirectional', '--target-branch', 'hetzner-live-sync-opencode-tracked-parity-target', '--full', '--hetzner-worker', '--timeout-ms', '1000000'], { remote: true, hetzner: true, opencode: true }),
  scenario('hetzner-permission-opencode', 'Hetzner remote tracked OpenCode Zen permission gating', remotePermissionDrill, ['--provider', 'opencode', ...OPENCODE_MODEL, '--mode', 'tracked', '--hetzner-worker', '--timeout-ms', '700000'], { remote: true, hetzner: true, opencode: true }),
]

function scenario(id, description, script, args, flags = {}) {
  return { id, description, script, args, ...flags }
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-workspace-live-sync-matrix-drill.mjs [options]',
    '',
    'Runs Workspace Live Sync validation scenarios through the existing live drills.',
    'By default this runs local Codex scenarios only.',
    '',
    'Options:',
    '  --include-remote        Include same-host remote relay scenarios',
    '  --include-hetzner       Include Hetzner worker scenarios',
    '  --include-opencode      Include OpenCode Zen scenarios',
    '  --only IDS              Comma-separated scenario ids',
    '  --dry-run               Print selected commands without running drills',
    '  --continue-on-failure   Run every selected scenario before exiting non-zero',
    '  --hetzner-host HOST     Forwarded to Hetzner drill scenarios',
    '  --hetzner-key PATH      Forwarded to Hetzner drill scenarios',
    '  --hetzner-repo PATH     Forwarded to Hetzner drill scenarios',
    '',
    'Environment:',
    '  ARROBA_WORKSPACE_LIVE_SYNC_CODEX_MODEL  Codex model for Codex scenarios; defaults to gpt-5.5',
    '  ARROBA_CODEX_MODEL                      Fallback Codex model override',
    '',
    'Scenario ids:',
    ...MATRIX.map((item) => `  ${item.id.padEnd(31)} ${item.description}`),
  ].join('\n'))
}

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith('--')) throw new Error(`${flag} requires a value`)
  return value
}

function parseArgs(argv) {
  const options = {
    includeRemote: false,
    includeHetzner: false,
    includeOpencode: false,
    only: null,
    dryRun: false,
    continueOnFailure: false,
    passthrough: [],
    help: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--') continue
    else if (arg === '--include-remote') options.includeRemote = true
    else if (arg === '--include-hetzner') options.includeHetzner = true
    else if (arg === '--include-opencode') options.includeOpencode = true
    else if (arg === '--dry-run') options.dryRun = true
    else if (arg === '--continue-on-failure') options.continueOnFailure = true
    else if (arg === '--help' || arg === '-h') options.help = true
    else if (arg === '--only') options.only = readValue(argv, i++, arg).split(',').map((id) => id.trim()).filter(Boolean)
    else if (arg.startsWith('--only=')) options.only = arg.slice('--only='.length).split(',').map((id) => id.trim()).filter(Boolean)
    else if (arg === '--hetzner-host' || arg === '--hetzner-key' || arg === '--hetzner-repo') {
      const value = readValue(argv, i, arg)
      options.passthrough.push(arg, value)
      i += 1
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  return options
}

function selectScenarios(options) {
  const known = new Map(MATRIX.map((item) => [item.id, item]))
  let selected
  if (options.only) {
    selected = options.only.map((id) => {
      const item = known.get(id)
      if (!item) throw new Error(`unknown scenario id: ${id}`)
      return item
    })
  } else {
    selected = MATRIX.filter((item) => {
      if (item.hetzner && !options.includeHetzner) return false
      if (item.remote && !item.hetzner && !options.includeRemote) return false
      if (item.opencode && !options.includeOpencode) return false
      return true
    })
  }
  if (selected.some((item) => item.hetzner) && !options.includeHetzner) {
    throw new Error('Hetzner scenarios require --include-hetzner')
  }
  if (selected.some((item) => item.remote && !item.hetzner) && !options.includeRemote) {
    throw new Error('same-host remote scenarios require --include-remote')
  }
  if (selected.some((item) => item.opencode) && !options.includeOpencode) {
    throw new Error('OpenCode scenarios require --include-opencode')
  }
  if (selected.length === 0) throw new Error('no scenarios selected')
  return selected
}

function commandForScenario(item, passthrough) {
  return {
    command: process.execPath,
    args: [item.script, ...item.args, '--keep-artifacts-on-failure', ...(item.hetzner ? passthrough : [])],
  }
}

function quoteCommand(command, args) {
  return [command, ...args].map((part) => (/[ "'\\]/.test(part) ? JSON.stringify(part) : part)).join(' ')
}

async function runScenario(item, passthrough) {
  const start = Date.now()
  const { command, args } = commandForScenario(item, passthrough)
  console.log(`[workspace-live-sync-matrix] start ${item.id}: ${item.description}`)
  console.log(`[workspace-live-sync-matrix] command ${quoteCommand(command, args)}`)
  let output = ''
  const appendOutput = (chunk, stream) => {
    const text = chunk.toString()
    stream.write(text)
    output += text
    if (output.length > 2_000_000) output = output.slice(-1_000_000)
  }
  const status = await new Promise((resolve) => {
    const child = spawn(command, args, { cwd: repoRoot, stdio: ['ignore', 'pipe', 'pipe'] })
    child.stdout.on('data', (chunk) => appendOutput(chunk, process.stdout))
    child.stderr.on('data', (chunk) => appendOutput(chunk, process.stderr))
    child.on('exit', (code, signal) => resolve({ code, signal }))
    child.on('error', (error) => resolve({ code: 1, signal: null, error }))
  })
  const durationMs = Date.now() - start
  if (status.code === 0) {
    if (item.expectedFailure) {
      const reason = 'expected unsupported failure but scenario exited successfully'
      console.error(`[workspace-live-sync-matrix] fail ${item.id} duration_ms=${durationMs} ${reason}`)
      return { item, ok: false, durationMs, reason }
    }
    console.log(`[workspace-live-sync-matrix] pass ${item.id} duration_ms=${durationMs}`)
    return { item, ok: true, durationMs }
  }
  if (item.expectedFailure) {
    const expected = item.expectedOutputIncludes
    if (!expected || output.includes(expected)) {
      console.log(`[workspace-live-sync-matrix] pass ${item.id} expected_failure duration_ms=${durationMs}`)
      return { item, ok: true, durationMs, expectedFailure: true }
    }
    const reason = `expected failure output to include ${JSON.stringify(expected)}`
    console.error(`[workspace-live-sync-matrix] fail ${item.id} duration_ms=${durationMs} ${reason}`)
    return { item, ok: false, durationMs, reason }
  }
  const reason = status.error?.message ?? `code=${status.code} signal=${status.signal ?? 'none'}`
  console.error(`[workspace-live-sync-matrix] fail ${item.id} duration_ms=${durationMs} ${reason}`)
  return { item, ok: false, durationMs, reason }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const selected = selectScenarios(options)
  console.log(`[workspace-live-sync-matrix] selected ${selected.map((item) => item.id).join(', ')}`)
  if (options.dryRun) {
    for (const item of selected) {
      const { command, args } = commandForScenario(item, options.passthrough)
      console.log(`[workspace-live-sync-matrix] dry-run ${item.id}: ${quoteCommand(command, args)}`)
    }
    return
  }
  const results = []
  for (const item of selected) {
    const result = await runScenario(item, options.passthrough)
    results.push(result)
    if (!result.ok && !options.continueOnFailure) break
  }
  const failed = results.filter((result) => !result.ok)
  console.log('[workspace-live-sync-matrix] summary')
  for (const result of results) {
    const expected = result.expectedFailure ? ' expected_failure' : ''
    console.log(`  ${result.ok ? 'pass' : 'fail'} ${result.item.id}${expected} duration_ms=${result.durationMs}${result.reason ? ` ${result.reason}` : ''}`)
  }
  if (failed.length > 0) process.exitCode = 1
}

main().catch((error) => {
  console.error(`[workspace-live-sync-matrix] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
