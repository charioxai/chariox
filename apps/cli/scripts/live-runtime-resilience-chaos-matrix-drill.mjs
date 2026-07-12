#!/usr/bin/env node
import path from "node:path"
import { fileURLToPath } from "node:url"

import {
  defaultDrillMatrixArtifactIndexPath,
  defaultDrillMatrixReportPath,
  parseDrillScenarioIds,
  runDrillMatrix,
  selectDrillMatrixScenarios,
} from "./lib/drill-matrix-runner.mjs"
import {
  appendHetznerPassthrough,
  drillDeploymentPresetMetadata,
  parseHetznerPassthroughArg,
} from "./lib/drill-environment-presets.mjs"
import {
  applyProviderAccountAlias,
  applyProviderModelOverride,
  providerProfileMetadata,
  resolveProviderModel,
} from "./lib/drill-provider-profiles.mjs"
import {
  DRILL_CHAOS_FAULT_KINDS,
  DRILL_CHAOS_INVARIANT_IDS,
  DRILL_CHAOS_REPLAY_SCHEMA,
} from "./lib/drill-chaos-contract.mjs"
import { DEFAULT_DETERMINISTIC_RUNTIME_CHAOS_SEED } from "./lib/drill-deterministic-runtime-model.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")

const kernelReconnectDrill = path.join(scriptDir, "live-kernel-reconnect-drill.mjs")
const localRestartDrill = path.join(scriptDir, "live-local-restart-drill.mjs")
const relayRuntimeDrill = path.join(scriptDir, "live-relay-runtime-drill.mjs")
const remoteRestartDrill = path.join(scriptDir, "live-remote-restart-drill.mjs")
const remoteHomeExtensionDrill = path.join(scriptDir, "live-remote-home-extension-drill.mjs")
const hostedCloudRelayDrill = path.join(scriptDir, "live-hosted-cloud-relay-drill.mjs")
const tuiWebParityDrill = path.join(scriptDir, "tui-web-terminal-parity-drill.mjs")
const providerThreadTransferDrill = path.join(scriptDir, "live-provider-thread-transfer-drill.mjs")
const deterministicRuntimeChaosDrill = path.join(scriptDir, "deterministic-runtime-chaos-drill.mjs")

const DEFAULT_CODEX_MODEL = process.env.ARROBA_RUNTIME_RESILIENCE_CODEX_MODEL
  ?? process.env.ARROBA_CODEX_MODEL
  ?? "gpt-5.4-mini"
const DEFAULT_OPENCODE_MODEL = process.env.ARROBA_RUNTIME_RESILIENCE_OPENCODE_MODEL
  ?? process.env.ARROBA_OPENCODE_MODEL
  ?? "opencode/gpt-5.2"
const DEFAULT_CHAOS_SEED = process.env.ARROBA_RUNTIME_RESILIENCE_CHAOS_SEED
  ?? DEFAULT_DETERMINISTIC_RUNTIME_CHAOS_SEED

const MATRIX = [
  scenario({
    id: "deterministic-runtime-convergence",
    description: "seeded virtual-clock fault injection proves idempotent execution and eventual TUI/web convergence",
    script: deterministicRuntimeChaosDrill,
    args: [],
    classification: "ui-client-projection",
    runtimeSignals: ["client-projection-health", "runtime-projection-health", "runtime-transition-audit", "session-authority"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "every accepted action executes exactly once despite drop, delay, duplication, and reorder faults",
      "TUI and web projections converge through cursor replay or snapshot fallback with monotonic cursors",
      "process death suppresses stale callbacks and leaves bounded empty queues with no leaked resources",
    ],
  }),
  scenario({
    id: "local-kernel-websocket-drop",
    description: "local kernel websocket close, reconnect, resubscribe, and request replay",
    script: kernelReconnectDrill,
    args: ["--keep-artifacts-on-failure"],
    classification: "relay-runtime",
    runtimeSignals: ["client-projection-health", "relay-target-freshness"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "client observes transport_closed and transport_resumed without losing the control request",
      "second subscription resumes from the last retained event id instead of resetting transcript state",
    ],
  }),
  scenario({
    id: "local-kernel-restart-durable-state",
    description: "local kernel restart restores durable session, grants, history, and active state cleanup",
    script: localRestartDrill,
    args: ["--keep-artifacts-on-failure"],
    classification: "kernel-authority",
    runtimeSignals: ["provider-run-lifecycle", "runtime-transition-audit", "session-authority"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "session, agent, MCP grant, skill grant, and history outline survive a kernel restart",
      "stale active provider run, prompt, and workflow activity are reconciled after restart",
    ],
  }),
  scenario({
    id: "local-relay-restart-reconnect",
    description: "self-hosted relay restart reconnects kernel/client transport and accepts a post-reconnect prompt",
    script: relayRuntimeDrill,
    args: ["--provider", "dev-stub", "--model", "runtime-resilience-dev-stub"],
    classification: "relay-target-freshness",
    runtimeSignals: ["provider-run-lifecycle", "relay-target-freshness", "session-authority"],
    deployment: "self-hosted-relay",
    provider: "dev-stub",
    exitCriteria: [
      "relay restart produces transport_closed then transport_resumed for the active session",
      "post-reconnect prompt completes exactly once through the kernel-owned session path",
    ],
  }),
  scenario({
    id: "local-tui-web-terminal-parity",
    description: "non-interactive TUI and web terminal projection parity checks",
    script: tuiWebParityDrill,
    args: [],
    classification: "ui-client-projection",
    runtimeSignals: ["client-projection-health", "runtime-projection-health", "session-authority"],
    deployment: "local",
    provider: "dev-stub",
    exitCriteria: [
      "TUI transcript, queue, footer, prompt, and waiting-room projections match web terminal contracts",
      "visual-session and visual-control scripts remain syntactically valid for screenshot-backed evidence runs",
    ],
  }),
  scenario({
    id: "same-host-remote-worker-restart",
    description: "same-host home and worker kernel restart repairs leased-agent binding",
    script: remoteRestartDrill,
    args: ["--keep-artifacts-on-failure"],
    classification: "relay-target-freshness",
    runtimeSignals: ["lease-health", "relay-target-freshness", "session-authority"],
    deployment: "same-host-remote",
    provider: "dev-stub",
    exitCriteria: [
      "home restart preserves the remote binding",
      "worker restart refreshes stale leased-agent state before the next prompt",
      "both-restart path leaves exactly one repaired remote execution binding",
    ],
  }),
  providerThreadScenario({
    id: "worker-provider-resume-codex",
    provider: "codex",
    description: "Codex provider thread resumes on a same-host worker after worker transfer",
    drill: "worker-resume",
    classification: "provider-error",
    runtimeSignals: ["lease-health", "provider-run-lifecycle", "session-authority"],
    deployment: "same-host-remote",
    extraArgs: ["--cleanup-on-success"],
    exitCriteria: [
      "provider resume state is captured before transfer",
      "worker-side provider run resumes without duplicating the prompt",
    ],
  }),
  providerThreadScenario({
    id: "worker-provider-resume-opencode",
    provider: "opencode",
    description: "OpenCode provider thread resumes on a same-host worker after worker transfer",
    drill: "worker-resume",
    classification: "provider-error",
    runtimeSignals: ["lease-health", "provider-run-lifecycle", "session-authority"],
    deployment: "same-host-remote",
    extraArgs: ["--cleanup-on-success"],
    exitCriteria: [
      "provider resume state is captured before transfer",
      "worker-side provider run resumes without duplicating the prompt",
    ],
  }),
  providerThreadScenario({
    id: "slice-restart-codex",
    provider: "codex",
    description: "Codex slice worker restart preserves recoverable provider thread state",
    drill: "slice-restart",
    classification: "slice-runtime",
    runtimeSignals: ["provider-run-lifecycle", "session-authority", "slice-runtime-state"],
    deployment: "local",
    requires: ["slice"],
    extraArgs: ["--slice-build-image", "always", "--cleanup-on-success"],
    exitCriteria: [
      "slice restart leaves kernel-owned session authority intact",
      "provider state is either resumed once or surfaced as a structured retry state",
    ],
  }),
  providerThreadScenario({
    id: "slice-restart-opencode",
    provider: "opencode",
    description: "OpenCode slice worker restart preserves recoverable provider thread state",
    drill: "slice-restart",
    classification: "slice-runtime",
    runtimeSignals: ["provider-run-lifecycle", "session-authority", "slice-runtime-state"],
    deployment: "local",
    requires: ["slice"],
    extraArgs: ["--slice-build-image", "always", "--cleanup-on-success"],
    exitCriteria: [
      "slice restart leaves kernel-owned session authority intact",
      "provider state is either resumed once or surfaced as a structured retry state",
    ],
  }),
  scenario({
    id: "hetzner-collaborator-reconnect-authority",
    description: "Hetzner collaborator remote agent survives relay loss and repairs after worker loss while home authority remains enforced",
    script: remoteHomeExtensionDrill,
    args: ["--hetzner-worker", "--collab", "--restart-relay", "--restart-worker"],
    requires: ["hetzner"],
    classification: "kernel-authority",
    runtimeSignals: ["home-extension-manifest-sync", "lease-health", "session-authority"],
    deployment: "hetzner",
    provider: "dev-stub",
    exitCriteria: [
      "the active collaborator runtime resumes through a restarted relay without losing its home extension grants",
      "worker restart repairs the stale lease and grant/revoke plus stale invocation checks remain home-kernel-owned",
    ],
  }),
  scenario({
    id: "hosted-cloud-relay-second-kernel-reconnect",
    description: "hosted Cloud relay second-kernel runtime reconnect smoke",
    script: hostedCloudRelayDrill,
    args: [],
    env: {
      ARROBA_CLOUD_HOSTED_SECOND_KERNEL: "1",
      ARROBA_CLOUD_HOSTED_MULTI_USER: "0",
    },
    requires: ["hosted-cloud"],
    classification: "relay-runtime",
    runtimeSignals: ["agent-lifecycle", "lease-health", "provider-run-lifecycle", "relay-target-freshness"],
    deployment: "hosted-cloud",
    provider: "dev-stub",
    exitCriteria: [
      "Cloud-issued relay credentials reconnect home and worker kernels",
      "hosted second-kernel provider turn completes through relay transport without Cloud proxying runtime traffic",
    ],
  }),
]

function scenario(definition) {
  return {
    args: [],
    env: undefined,
    requires: [],
    ...definition,
  }
}

function providerThreadScenario({
  id,
  provider,
  description,
  drill,
  classification,
  runtimeSignals,
  deployment,
  requires = [],
  extraArgs = [],
  exitCriteria,
}) {
  return scenario({
    id,
    provider,
    providerFamily: provider,
    description,
    script: providerThreadTransferDrill,
    args: ["--drill", drill, "--provider", provider, ...extraArgs],
    classification,
    runtimeSignals,
    deployment,
    requires,
    exitCriteria,
  })
}

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/live-runtime-resilience-chaos-matrix-drill.mjs [options]",
    "",
    "Runs runtime resilience chaos coverage by composing existing reconnect, restart, relay, remote, TUI/web, provider-resume, slice, Hetzner, and hosted Cloud drills.",
    "Local and same-host scenarios are selected by default. Slice, Hetzner, and hosted Cloud scenarios are opt-in.",
    "",
    "Options:",
    "  --include-slices         Include local Docker slice restart provider scenarios",
    "  --include-hetzner        Include Hetzner worker/collaborator scenarios",
    "  --include-hosted-cloud   Include hosted Cloud relay scenarios",
    "  --only IDS               Comma-separated scenario ids",
    "  --dry-run                Print selected commands without running drills",
    "  --continue-on-failure    Run every selected scenario before exiting non-zero",
    "  --report PATH            Write a machine-readable matrix report; defaults under .artifacts/drill-matrices",
    "  --artifact-index PATH     Write a verifiable artifact index for the matrix report",
    "  --chaos-seed VALUE        Replay seed for deterministic fault injection",
    "  --chaos-replay PATH       Deterministic replay artifact path",
    "  --provider-model P=M      Override model for provider-resume scenarios",
    "  --provider-account P=A    Label provider account/profile metadata without exposing credentials",
    "  --hetzner-host HOST       Forwarded to Hetzner drill scenarios",
    "  --hetzner-key PATH        Forwarded to Hetzner drill scenarios",
    "  --hetzner-repo PATH       Forwarded to Hetzner drill scenarios",
    "",
    "Environment defaults:",
    `  ARROBA_RUNTIME_RESILIENCE_CODEX_MODEL=${DEFAULT_CODEX_MODEL}`,
    `  ARROBA_RUNTIME_RESILIENCE_OPENCODE_MODEL=${DEFAULT_OPENCODE_MODEL}`,
    `  ARROBA_RUNTIME_RESILIENCE_CHAOS_SEED=${DEFAULT_CHAOS_SEED}`,
    "",
    "Scenario ids:",
    ...MATRIX.map((scenarioItem) => `  ${scenarioItem.id.padEnd(42)} ${scenarioItem.description}`),
  ].join("\n"))
}

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function parseArgs(argv) {
  const options = {
    includeSlices: false,
    includeHetzner: false,
    includeHostedCloud: false,
    only: null,
    dryRun: false,
    continueOnFailure: false,
    reportPath: null,
    artifactIndexPath: null,
    chaosSeed: DEFAULT_CHAOS_SEED,
    chaosReplayPath: null,
    providerAccounts: {},
    providerModels: {},
    passthrough: [],
    help: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") continue
    else if (arg === "--include-slices") options.includeSlices = true
    else if (arg === "--include-hetzner") options.includeHetzner = true
    else if (arg === "--include-hosted-cloud") options.includeHostedCloud = true
    else if (arg === "--dry-run") options.dryRun = true
    else if (arg === "--continue-on-failure") options.continueOnFailure = true
    else if (arg === "--report") options.reportPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--report=")) options.reportPath = arg.slice("--report=".length)
    else if (arg === "--artifact-index") options.artifactIndexPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--artifact-index=")) options.artifactIndexPath = arg.slice("--artifact-index=".length)
    else if (arg === "--chaos-seed") options.chaosSeed = readValue(argv, index++, arg)
    else if (arg.startsWith("--chaos-seed=")) options.chaosSeed = arg.slice("--chaos-seed=".length)
    else if (arg === "--chaos-replay") options.chaosReplayPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--chaos-replay=")) options.chaosReplayPath = arg.slice("--chaos-replay=".length)
    else if (arg === "--provider-account") applyProviderAccountAlias(options.providerAccounts, readValue(argv, index++, arg))
    else if (arg.startsWith("--provider-account=")) applyProviderAccountAlias(options.providerAccounts, arg.slice("--provider-account=".length))
    else if (arg === "--provider-model") applyProviderModelOverride(options.providerModels, readValue(argv, index++, arg))
    else if (arg.startsWith("--provider-model=")) applyProviderModelOverride(options.providerModels, arg.slice("--provider-model=".length))
    else if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--only") options.only = parseDrillScenarioIds(readValue(argv, index++, arg))
    else if (arg.startsWith("--only=")) options.only = parseDrillScenarioIds(arg.slice("--only=".length))
    else {
      const hetznerArg = parseHetznerPassthroughArg(argv, index)
      if (hetznerArg) {
        options.passthrough.push(...hetznerArg.args)
        index = hetznerArg.nextIndex
        continue
      }
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  return options
}

function selectScenarios(options) {
  const enabledRequirements = new Set()
  if (options.includeSlices) enabledRequirements.add("slice")
  if (options.includeHetzner) enabledRequirements.add("hetzner")
  if (options.includeHostedCloud) enabledRequirements.add("hosted-cloud")
  return selectDrillMatrixScenarios({
    scenarios: MATRIX,
    requestedIds: options.only,
    enabledRequirements,
    requirementLabels: {
      slice: "--include-slices",
      hetzner: "--include-hetzner",
      "hosted-cloud": "--include-hosted-cloud",
    },
  })
}

function modelForProvider(provider, options) {
  const defaultModel = provider === "codex"
    ? DEFAULT_CODEX_MODEL
    : DEFAULT_OPENCODE_MODEL
  return resolveProviderModel(provider, {
    defaultModel,
    providerModels: options.providerModels,
  })
}

function commandForScenario(scenarioItem, options) {
  let args = [scenarioItem.script, ...scenarioItem.args]
  if (scenarioItem.script === deterministicRuntimeChaosDrill) {
    args = [...args, "--seed", options.chaosSeed]
    if (options.chaosReplayPath) args = [...args, "--output", path.resolve(options.chaosReplayPath)]
  }
  if (scenarioItem.script === providerThreadTransferDrill && scenarioItem.provider && scenarioItem.provider !== "dev-stub") {
    args = [...args, "--provider-model", `${scenarioItem.provider}=${modelForProvider(scenarioItem.provider, options)}`]
  }
  return {
    command: process.execPath,
    args: appendHetznerPassthrough(args, scenarioItem, options.passthrough),
    env: scenarioItem.env,
  }
}

function metadataFor(selected, options) {
  const includesDeterministicChaos = selected.some((scenarioItem) => scenarioItem.script === deterministicRuntimeChaosDrill)
  const providers = [...new Set([
    ...selected
      .map((scenarioItem) => scenarioItem.providerFamily ?? scenarioItem.provider)
      .filter((provider) => provider && provider !== "dev-stub"),
    ...Object.keys(options.providerAccounts),
  ])].sort()
  return {
    includeSlices: options.includeSlices,
    includeHetzner: options.includeHetzner,
    includeHostedCloud: options.includeHostedCloud,
    generatedMatrixNames: "runtime-resilience-chaos-matrix",
    generatedMatrixRepos: "oss",
    ...(includesDeterministicChaos
      ? {
        deterministicChaosSeed: options.chaosSeed,
        deterministicChaosReplaySchema: DRILL_CHAOS_REPLAY_SCHEMA,
        deterministicChaosFaultKinds: DRILL_CHAOS_FAULT_KINDS.join(","),
        deterministicChaosInvariantIds: DRILL_CHAOS_INVARIANT_IDS.join(","),
      }
      : {}),
    resourceEvidence: "child drills use isolated ports/roots and preserve cleanup artifacts on failure",
    ...drillDeploymentPresetMetadata([
      "local",
      "same-host-remote",
      "self-hosted-relay",
      ...(options.includeHetzner ? ["hetzner"] : []),
      ...(options.includeHostedCloud ? ["hosted-cloud"] : []),
    ], { hetznerPassthrough: options.passthrough }),
    ...providerProfileMetadata({
      providers,
      defaultModel: providers.length > 0 ? "per-provider" : "dev-stub",
      providerAccounts: options.providerAccounts,
      providerModels: options.providerModels,
    }),
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const selected = selectScenarios(options)
  const reportPath = options.reportPath ?? defaultDrillMatrixReportPath("runtime-resilience-chaos-matrix", { rootDir: repoRoot })
  const artifactIndexPath = options.artifactIndexPath ?? defaultDrillMatrixArtifactIndexPath(reportPath)
  const results = await runDrillMatrix({
    matrixName: "runtime-resilience-chaos-matrix",
    scenarios: selected,
    commandForScenario: (scenarioItem) => commandForScenario(scenarioItem, options),
    cwd: repoRoot,
    continueOnFailure: options.continueOnFailure,
    dryRun: options.dryRun,
    reportPath,
    artifactIndexPath,
    metadata: metadataFor(selected, options),
  })
  if (results.some((result) => !result.ok)) process.exitCode = 1
}

main().catch((error) => {
  console.error(`[runtime-resilience-chaos-matrix] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
