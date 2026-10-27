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
  drillDeploymentPresetMetadata,
  parseHetznerPassthroughArg,
} from './lib/drill-environment-presets.mjs'
import {
  applyProviderAccountAlias,
  providerProfileMetadata,
} from './lib/drill-provider-profiles.mjs'
import {
  workspaceLiveSyncRequiredScenarioIds,
  workspaceLiveSyncScenarioClassification,
  workspaceLiveSyncScenarioDeployment,
  workspaceLiveSyncScenarioMode,
  workspaceLiveSyncScenarioProvider,
  workspaceLiveSyncScenarioRuntimeSignals,
} from './lib/workspace-live-sync-fixtures.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..', '..')
const localDrill = path.join(scriptDir, 'live-workspace-live-sync-drill.mjs')
const remoteDrill = path.join(scriptDir, 'live-remote-workspace-live-sync-drill.mjs')
const localPermissionDrill = path.join(scriptDir, 'live-workspace-live-sync-permission-drill.mjs')
const remotePermissionDrill = path.join(scriptDir, 'live-remote-workspace-live-sync-permission-drill.mjs')

const codexModelId = process.env.CHARIOX_WORKSPACE_LIVE_SYNC_CODEX_MODEL ?? process.env.CHARIOX_CODEX_MODEL ?? 'gpt-5.5'
const opencodeModelId = process.env.CHARIOX_WORKSPACE_LIVE_SYNC_OPENCODE_MODEL ?? process.env.CHARIOX_OPENCODE_MODEL ?? 'opencode/gpt-5.2'
const CODEX_MODEL = ['--provider-model', `codex=${codexModelId}`]
const OPENCODE_MODEL = ['--provider-model', `opencode=${opencodeModelId}`]
const REQUIRED_SCENARIOS = new Set(workspaceLiveSyncRequiredScenarioIds())

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
  const requires = []
  if (flags.remote) requires.push('remote')
  if (flags.hetzner) requires.push('hetzner')
  if (flags.opencode) requires.push('opencode')
  return {
    id,
    description,
    script,
    args,
    ...flags,
    classification: flags.classification ?? workspaceLiveSyncClassification({ id, args, flags }),
    deployment: workspaceLiveSyncScenarioDeployment(id),
    mode: workspaceLiveSyncScenarioMode(id),
    provider: workspaceLiveSyncScenarioProvider(id),
    runtimeSignals: workspaceLiveSyncRuntimeSignals({ id, args, flags }),
    requires,
    exitCriteria: workspaceLiveSyncExitCriteria({ id, args, flags }),
  }
}

function workspaceLiveSyncClassification({ id, args, flags }) {
  if (flags.expectedFailure) return null
  if (REQUIRED_SCENARIOS.has(id)) return workspaceLiveSyncScenarioClassification(id)
  if (id.includes('permission')) return 'kernel-authority'
  if (id.includes('restart')) return 'relay-target-freshness'
  const mode = valueAfter(args, '--mode') ?? null
  if (mode === 'managed' || mode === 'tracked') return 'workspace-live-sync-conflict'
  return null
}

function workspaceLiveSyncRuntimeSignals({ id, args, flags }) {
  if (flags.expectedFailure) return []
  if (REQUIRED_SCENARIOS.has(id)) return workspaceLiveSyncScenarioRuntimeSignals(id)
  const signals = ['session-authority']
  if (id.includes('restart')) signals.push('relay-target-freshness')
  const mode = valueAfter(args, '--mode') ?? null
  if (mode === 'managed' || mode === 'tracked' || id.includes('permission')) {
    signals.push('workspace-live-sync-state')
  }
  return [...new Set(signals)].sort()
}

function workspaceLiveSyncExitCriteria({ id, args, flags }) {
  if (flags.expectedFailure) {
    return [
      'scenario fails for the documented unsupported capability',
      `failure output includes ${JSON.stringify(flags.expectedOutputIncludes)}`,
    ]
  }
  const provider = valueAfter(args, '--provider') ?? (flags.opencode ? 'opencode' : 'codex')
  const mode = valueAfter(args, '--mode') ?? (id.includes('permission') ? 'permission' : 'managed')
  const placement = flags.hetzner ? 'Hetzner remote worker' : flags.remote ? 'same-host remote worker' : 'local worker'
  if (id.includes('permission')) {
    return [
      `${provider} permission checks use kernel-owned workspace live sync policy`,
      `${placement} leaves non-selected repositories unrestricted`,
    ]
  }
  if (mode === 'off') {
    return [
      `${provider} session runs with workspace live sync disabled`,
      'selected workspace sync is inactive and other repositories remain unrestricted',
    ]
  }
  if (mode === 'tracked') {
    return [
      `${provider} tracked mode fans out turn-end file changes across selected targets`,
      `${placement} preserves selected-workspace scope while leaving other repositories unrestricted`,
    ]
  }
  return [
    `${provider} managed mode fans out selected workspace writes across configured targets`,
    `${placement} uses kernel-owned live sync status, targets, and conflict reporting`,
  ]
}

function valueAfter(args, flag) {
  const index = args.indexOf(flag)
  if (index === -1) return null
  const value = args[index + 1]
  return value && !value.startsWith('--') ? value : null
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
    '  --report PATH           Write a machine-readable matrix report; defaults under .artifacts/drill-matrices',
    '  --artifact-index PATH   Write a verifiable artifact index for the matrix report',
    '  --provider-account P=A  Label the provider account/profile used by this matrix without exposing credentials',
    '  --hetzner-host HOST     Forwarded to Hetzner drill scenarios',
    '  --hetzner-key PATH      Forwarded to Hetzner drill scenarios',
    '  --hetzner-repo PATH     Forwarded to Hetzner drill scenarios',
    '',
    'Environment:',
    '  CHARIOX_WORKSPACE_LIVE_SYNC_CODEX_MODEL  Codex model for Codex scenarios; defaults to gpt-5.5',
    '  CHARIOX_CODEX_MODEL                      Fallback Codex model override',
    '  CHARIOX_WORKSPACE_LIVE_SYNC_OPENCODE_MODEL  OpenCode model for OpenCode scenarios; defaults to opencode/gpt-5.2',
    '  CHARIOX_OPENCODE_MODEL                      Fallback OpenCode model override',
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
    reportPath: null,
    artifactIndexPath: null,
    providerAccounts: {},
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
    else if (arg === '--report') options.reportPath = readValue(argv, i++, arg)
    else if (arg.startsWith('--report=')) options.reportPath = arg.slice('--report='.length)
    else if (arg === '--artifact-index') options.artifactIndexPath = readValue(argv, i++, arg)
    else if (arg.startsWith('--artifact-index=')) options.artifactIndexPath = arg.slice('--artifact-index='.length)
    else if (arg === '--provider-account') applyProviderAccountAlias(options.providerAccounts, readValue(argv, i++, arg))
    else if (arg.startsWith('--provider-account=')) applyProviderAccountAlias(options.providerAccounts, arg.slice('--provider-account='.length))
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
  const enabledRequirements = new Set()
  if (options.includeRemote) enabledRequirements.add('remote')
  if (options.includeHetzner) {
    enabledRequirements.add('remote')
    enabledRequirements.add('hetzner')
  }
  if (options.includeOpencode) enabledRequirements.add('opencode')
  return selectDrillMatrixScenarios({
    scenarios: MATRIX,
    requestedIds: options.only,
    enabledRequirements,
    requirementLabels: {
      remote: '--include-remote',
      hetzner: '--include-hetzner',
      opencode: '--include-opencode',
    },
  })
}

function commandForScenario(item, passthrough) {
  return {
    command: process.execPath,
    args: appendHetznerPassthrough([item.script, ...item.args, '--keep-artifacts-on-failure'], item, passthrough),
  }
}

function providerMetadataFor(selected, options) {
  const providers = [...new Set(selected
    .map((item) => valueAfter(item.args, '--provider'))
    .filter(Boolean))]
    .sort()
  const providerModels = {}
  if (providers.includes('codex')) providerModels.codex = codexModelId
  if (providers.includes('opencode')) providerModels.opencode = opencodeModelId
  return providerProfileMetadata({
    providers,
    defaultModel: 'per-provider',
    providerAccounts: options.providerAccounts,
    providerModels,
  })
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const selected = selectScenarios(options)
  const reportPath = options.reportPath ?? defaultDrillMatrixReportPath('workspace-live-sync-matrix', { rootDir: repoRoot })
  const artifactIndexPath = options.artifactIndexPath ?? defaultDrillMatrixArtifactIndexPath(reportPath)
  const results = await runDrillMatrix({
    matrixName: 'workspace-live-sync-matrix',
    scenarios: selected,
    commandForScenario: (item) => commandForScenario(item, options.passthrough),
    cwd: repoRoot,
    continueOnFailure: options.continueOnFailure,
    dryRun: options.dryRun,
    reportPath,
    artifactIndexPath,
    metadata: {
      includeRemote: options.includeRemote,
      includeHetzner: options.includeHetzner,
      includeOpencode: options.includeOpencode,
      generatedMatrixNames: 'workspace-live-sync-matrix',
      generatedMatrixRepos: 'oss',
      ...providerMetadataFor(selected, options),
      ...drillDeploymentPresetMetadata([
        'local',
        ...(options.includeRemote ? ['same-host-remote', 'self-hosted-relay'] : []),
        ...(options.includeHetzner ? ['hetzner', 'self-hosted-relay'] : []),
      ], { hetznerPassthrough: options.passthrough }),
      codexModelId,
      opencodeModelId,
    },
  })
  const failed = results.filter((result) => !result.ok)
  if (failed.length > 0) process.exitCode = 1
}

main().catch((error) => {
  console.error(`[workspace-live-sync-matrix] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
