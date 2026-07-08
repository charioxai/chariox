export const DEFAULT_KERNEL = 'ws://127.0.0.1:43284'
export const DEFAULT_MODEL = 'gpt-5.2'
export const DEFAULT_PROVIDERS = ['opencode', 'codex']
export const DEFAULT_TIMEOUT_MS = 360_000
export const DEFAULT_POLL_MS = 1_000

export function parseArgs(argv) {
  const options = {
    kernel: DEFAULT_KERNEL,
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    spawnDaemon: true,
    machineRef: null,
    historyDir: null,
    providerHistoryDirs: [],
    keepArtifactsOnFailure: false,
    positiveOnly: false,
    mode: 'managed',
    managedTargetCount: 0,
    targetBranch: 'main',
    trackedTargetCount: 1,
    trackedBidirectional: false,
    remoteSourceSideEffects: false,
    rootDir: null,
    afterFixtureCommand: null,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--') continue
    else if (arg === '--kernel') options.kernel = argv[++i]
    else if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--no-spawn-daemon') options.spawnDaemon = false
    else if (arg === '--machine-ref') options.machineRef = argv[++i]
    else if (arg === '--history-dir') options.historyDir = argv[++i]
    else if (arg === '--provider-history-dir') options.providerHistoryDirs.push(argv[++i])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--positive-only') options.positiveOnly = true
    else if (arg === '--mode') {
      options.mode = argv[++i]
      if (!['off', 'managed', 'tracked'].includes(options.mode)) throw new Error('--mode must be off, managed, or tracked')
    }
    else if (arg === '--managed-target-count') options.managedTargetCount = Number(argv[++i])
    else if (arg === '--target-branch') options.targetBranch = argv[++i]
    else if (arg === '--tracked-target-count') options.trackedTargetCount = Number(argv[++i])
    else if (arg === '--tracked-bidirectional') options.trackedBidirectional = true
    else if (arg === '--remote-source-side-effects') options.remoteSourceSideEffects = true
    else if (arg === '--root-dir') options.rootDir = argv[++i]
    else if (arg === '--after-fixture-command') options.afterFixtureCommand = argv[++i]
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (!Number.isInteger(options.trackedTargetCount) || options.trackedTargetCount < 1) {
    throw new Error('--tracked-target-count must be a positive integer')
  }
  if (!Number.isInteger(options.managedTargetCount) || options.managedTargetCount < 0) {
    throw new Error('--managed-target-count must be a non-negative integer')
  }
  return options
}

export function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-workspace-live-sync-drill.mjs [options]',
    '',
    'Runs a live workspace live sync provider drill with isolated daemon/session/workspace lifecycle:',
    '- off: agents write directly in the selected repo and a sibling repo; both writes must land',
    '- positive: agents read seed.txt and exercise Arroba write/edit/patch/move/delete tools',
    '- negative: agents are asked to write directly without Arroba; direct output files must not appear',
    '- collision: two agents attempt the same text edit area; exactly one write may land',
    '- external changes: non-overlap stale edits rebase, overlapping stale edits are rejected',
    '- tracked mode: direct writes inside the selected worktree sync at turn end, while direct writes to the sibling repo must remain allowed and unsynced',
    '',
    'Options:',
    `  --kernel ${DEFAULT_KERNEL}`,
    '  --provider PROVIDER',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --provider-model PROVIDER=MODEL (for example codex=gpt-5.2 or opencode=opencode/gpt-5.2)',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --no-spawn-daemon',
    '  --machine-ref MACHINE_ID_OR_ALIAS (spawn agents on a remote worker machine)',
    '  --history-dir PATH (session history dir when using --no-spawn-daemon)',
    '  --provider-history-dir PATH (additional provider-event history dir; repeatable for remote workers)',
    '  --keep-artifacts-on-failure',
    '  --positive-only (stop after the managed read/write/edit/patch/move/delete smoke)',
    '  --mode off|managed|tracked',
    '  --managed-target-count COUNT (managed mode only; attach and validate target workspaces)',
    '  --target-branch BRANCH (tracked mode target branch; use a non-main value to drill explicit cross-branch links)',
    '  --tracked-target-count COUNT (tracked mode only; attach and validate multiple target workspaces)',
    '  --tracked-bidirectional (tracked mode only; validate target-origin fanout back to source/sibling targets)',
    '  --root-dir PATH (override isolated drill root)',
    '  --after-fixture-command CMD (run after local fixtures are initialized, before agents spawn)',
  ].join('\n'))
}
