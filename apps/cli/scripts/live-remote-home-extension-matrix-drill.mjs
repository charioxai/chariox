#!/usr/bin/env node
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  defaultDrillMatrixArtifactIndexPath,
  defaultDrillMatrixReportPath,
  parseDrillScenarioIds,
  runDrillMatrix,
  selectDrillMatrixScenarios,
} from './lib/drill-matrix-runner.mjs'
import {
  appendHetznerPassthrough,
  hetznerPassthroughMetadata,
  parseHetznerPassthroughArg,
} from './lib/drill-environment-presets.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..', '..')
const drillScript = path.join(scriptDir, 'live-remote-home-extension-drill.mjs')

const MATRIX = [
  {
    id: 'local-single',
    description: 'local self-hosted relay, single user',
    args: [],
    exitCriteria: [
      'home-owned script, MCP, and connector tools execute on home for a remote worker agent',
      'worker lacks local definitions/credentials and stale calls are blocked after revoke',
    ],
  },
  {
    id: 'local-collab',
    description: 'local self-hosted relay, collab',
    args: ['--collab'],
    exitCriteria: [
      'collaborator remote agent can invoke only home-granted tools',
      'collaborator cannot grant, revoke, widen scope, or inspect home credentials',
    ],
  },
  {
    id: 'hetzner-single',
    description: 'Hetzner worker, single user',
    args: ['--hetzner-worker'],
    requires: ['hetzner'],
    exitCriteria: [
      'single-user home-owned extensions execute on home while worker runs on Hetzner',
      'self-hosted relay carries projection and invocation without owning runtime authority',
    ],
  },
  {
    id: 'hetzner-collab',
    description: 'Hetzner worker, collab',
    args: ['--hetzner-worker', '--collab'],
    requires: ['hetzner'],
    exitCriteria: [
      'collab remote agent on Hetzner can invoke only home-authorized tools',
      'home revoke and authorization checks remain authoritative across machines',
    ],
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
    '  --report PATH            Write a machine-readable matrix report; defaults under .artifacts/drill-matrices',
    '  --artifact-index PATH    Write a verifiable artifact index for the matrix report',
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
    reportPath: null,
    artifactIndexPath: null,
    passthrough: [],
    help: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--') continue
    else if (arg === '--include-hetzner') options.includeHetzner = true
    else if (arg === '--dry-run') options.dryRun = true
    else if (arg === '--continue-on-failure') options.continueOnFailure = true
    else if (arg === '--report') options.reportPath = readValue(argv, i++, arg)
    else if (arg.startsWith('--report=')) options.reportPath = arg.slice('--report='.length)
    else if (arg === '--artifact-index') options.artifactIndexPath = readValue(argv, i++, arg)
    else if (arg.startsWith('--artifact-index=')) options.artifactIndexPath = arg.slice('--artifact-index='.length)
    else if (arg === '--help' || arg === '-h') options.help = true
    else if (arg === '--only') options.only = parseDrillScenarioIds(readValue(argv, i++, arg))
    else if (arg.startsWith('--only=')) options.only = parseDrillScenarioIds(arg.slice('--only='.length))
    else {
      const hetznerArg = parseHetznerPassthroughArg(argv, i)
      if (hetznerArg) {
        options.passthrough.push(...hetznerArg.args)
        i = hetznerArg.nextIndex
        continue
      }
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  return options
}

function selectScenarios(options) {
  return selectDrillMatrixScenarios({
    scenarios: MATRIX,
    requestedIds: options.only,
    enabledRequirements: new Set(options.includeHetzner ? ['hetzner'] : []),
    requirementLabels: { hetzner: '--include-hetzner' },
  })
}

function commandForScenario(scenario, passthrough) {
  return {
    command: process.execPath,
    args: appendHetznerPassthrough([drillScript, ...scenario.args], scenario, passthrough),
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const selected = selectScenarios(options)
  const reportPath = options.reportPath ?? defaultDrillMatrixReportPath('remote-home-extension-matrix', { rootDir: repoRoot })
  const artifactIndexPath = options.artifactIndexPath ?? defaultDrillMatrixArtifactIndexPath(reportPath)
  const results = await runDrillMatrix({
    matrixName: 'remote-home-extension-matrix',
    scenarios: selected,
    commandForScenario: (scenario) => commandForScenario(scenario, options.passthrough),
    cwd: repoRoot,
    continueOnFailure: options.continueOnFailure,
    dryRun: options.dryRun,
    reportPath,
    artifactIndexPath,
    metadata: {
      includeHetzner: options.includeHetzner,
      ...hetznerPassthroughMetadata(options.passthrough),
    },
  })
  const failed = results.filter((result) => !result.ok)
  if (failed.length > 0) process.exitCode = 1
}

main().catch((error) => {
  console.error(`[remote-home-extension-matrix] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
