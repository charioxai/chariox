#!/usr/bin/env node
import { spawn } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..', '..')
const drillScript = path.join(scriptDir, 'live-remote-home-extension-drill.mjs')

const MATRIX = [
  {
    id: 'local-single',
    description: 'local self-hosted relay, single user',
    args: [],
  },
  {
    id: 'local-collab',
    description: 'local self-hosted relay, collab',
    args: ['--collab'],
  },
  {
    id: 'hetzner-single',
    description: 'Hetzner worker, single user',
    args: ['--hetzner-worker'],
    hetzner: true,
  },
  {
    id: 'hetzner-collab',
    description: 'Hetzner worker, collab',
    args: ['--hetzner-worker', '--collab'],
    hetzner: true,
  },
]

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-remote-home-extension-matrix-drill.mjs [options]',
    '',
    'Runs the remote home-owned extension drill across supported deployment modes.',
    'By default this runs only local single-user and local collab scenarios.',
    '',
    'Options:',
    '  --include-hetzner        Include Hetzner worker scenarios',
    '  --only IDS               Comma-separated scenario ids',
    '  --dry-run                Print selected commands without running drills',
    '  --continue-on-failure    Run every selected scenario before exiting non-zero',
    '  --hetzner-host HOST      Forwarded to Hetzner drill scenarios',
    '  --hetzner-key PATH       Forwarded to Hetzner drill scenarios',
    '  --hetzner-repo PATH      Forwarded to Hetzner drill scenarios',
    '',
    'Scenario ids:',
    ...MATRIX.map((scenario) => `  ${scenario.id.padEnd(15)} ${scenario.description}`),
  ].join('\n'))
}

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith('--')) throw new Error(`${flag} requires a value`)
  return value
}

function parseArgs(argv) {
  const options = {
    includeHetzner: false,
    only: null,
    dryRun: false,
    continueOnFailure: false,
    passthrough: [],
    help: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--') continue
    else if (arg === '--include-hetzner') options.includeHetzner = true
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
  const known = new Map(MATRIX.map((scenario) => [scenario.id, scenario]))
  let selected
  if (options.only) {
    selected = options.only.map((id) => {
      const scenario = known.get(id)
      if (!scenario) throw new Error(`unknown scenario id: ${id}`)
      return scenario
    })
  } else {
    selected = MATRIX.filter((scenario) => !scenario.hetzner || options.includeHetzner)
  }
  const hetznerSelected = selected.some((scenario) => scenario.hetzner)
  if (hetznerSelected && !options.includeHetzner) {
    throw new Error('Hetzner scenarios require --include-hetzner')
  }
  if (selected.length === 0) throw new Error('no scenarios selected')
  return selected
}

function commandForScenario(scenario, passthrough) {
  return {
    command: process.execPath,
    args: [drillScript, ...scenario.args, ...(scenario.hetzner ? passthrough : [])],
  }
}

function quoteCommand(command, args) {
  return [command, ...args].map((part) => (/[ "'\\]/.test(part) ? JSON.stringify(part) : part)).join(' ')
}

async function runScenario(scenario, passthrough) {
  const start = Date.now()
  const { command, args } = commandForScenario(scenario, passthrough)
  console.log(`[remote-home-extension-matrix] start ${scenario.id}: ${scenario.description}`)
  console.log(`[remote-home-extension-matrix] command ${quoteCommand(command, args)}`)
  const status = await new Promise((resolve) => {
    const child = spawn(command, args, { cwd: repoRoot, stdio: 'inherit' })
    child.on('exit', (code, signal) => resolve({ code, signal }))
    child.on('error', (error) => resolve({ code: 1, signal: null, error }))
  })
  const durationMs = Date.now() - start
  if (status.code === 0) {
    console.log(`[remote-home-extension-matrix] pass ${scenario.id} duration_ms=${durationMs}`)
    return { scenario, ok: true, durationMs }
  }
  const reason = status.error?.message ?? `code=${status.code} signal=${status.signal ?? 'none'}`
  console.error(`[remote-home-extension-matrix] fail ${scenario.id} duration_ms=${durationMs} ${reason}`)
  return { scenario, ok: false, durationMs, reason }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const selected = selectScenarios(options)
  console.log(`[remote-home-extension-matrix] selected ${selected.map((scenario) => scenario.id).join(', ')}`)
  if (options.dryRun) {
    for (const scenario of selected) {
      const { command, args } = commandForScenario(scenario, options.passthrough)
      console.log(`[remote-home-extension-matrix] dry-run ${scenario.id}: ${quoteCommand(command, args)}`)
    }
    return
  }
  const results = []
  for (const scenario of selected) {
    const result = await runScenario(scenario, options.passthrough)
    results.push(result)
    if (!result.ok && !options.continueOnFailure) break
  }
  const failed = results.filter((result) => !result.ok)
  console.log('[remote-home-extension-matrix] summary')
  for (const result of results) {
    console.log(`  ${result.ok ? 'pass' : 'fail'} ${result.scenario.id} duration_ms=${result.durationMs}${result.reason ? ` ${result.reason}` : ''}`)
  }
  if (failed.length > 0) process.exitCode = 1
}

main().catch((error) => {
  console.error(`[remote-home-extension-matrix] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
