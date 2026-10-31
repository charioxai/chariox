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
  appendRemoteKernelPassthrough,
  drillDeploymentPresetMetadata,
  parseRemoteKernelPassthroughArg,
} from "./lib/drill-environment-presets.mjs"
import {
  applyProviderAccountAlias,
  applyProviderModelOverride,
  providerProfileMetadata,
  resolveProviderModel,
} from "./lib/drill-provider-profiles.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const remoteMachineDrill = path.join(scriptDir, "live-remote-machine-runtime-drill.mjs")
const remoteRestartDrill = path.join(scriptDir, "live-remote-restart-drill.mjs")
const remoteHomeExtensionDrill = path.join(scriptDir, "live-remote-home-extension-drill.mjs")
const hostedCloudDrill = path.join(scriptDir, "live-hosted-cloud-relay-drill.mjs")

const DEFAULT_CODEX_MODEL = process.env.CHARIOX_REMOTE_AGENT_RUNTIME_CODEX_MODEL
  ?? process.env.CHARIOX_CODEX_MODEL
  ?? "gpt-5.2-codex"
const DEFAULT_OPENCODE_MODEL = process.env.CHARIOX_REMOTE_AGENT_RUNTIME_OPENCODE_MODEL
  ?? process.env.CHARIOX_OPENCODE_MODEL
  ?? "opencode/gpt-5.2"
const DEFAULT_CLAUDE_MODEL = process.env.CHARIOX_REMOTE_AGENT_RUNTIME_CLAUDE_MODEL
  ?? process.env.CHARIOX_CLAUDE_MODEL
  ?? "sonnet"

const MATRIX = [
  providerScenario({
    id: "single-user-remote-agent",
    provider: "codex",
    description: "same-host remote Codex agent lease, prompt, cancellation, and placement",
    classification: "relay-runtime",
    runtimeSignals: ["agent-lifecycle", "lease-health", "session-authority"],
    exitCriteria: [
      "home kernel discovers a same-host remote worker through the relay",
      "remote Codex agent receives a worker lease and completes a prompt from home",
    ],
  }),
  providerScenario({
    id: "remote-prompt-dispatch",
    provider: "opencode",
    description: "same-host remote OpenCode prompt dispatch and worker execution",
    classification: "worker-execution",
    runtimeSignals: ["agent-lifecycle", "provider-run-lifecycle"],
    exitCriteria: [
      "home prompt dispatch reaches the worker provider run",
      "prompt completion and cancellation return through the kernel-owned session path",
    ],
  }),
  providerScenario({
    id: "provider-run-binding",
    provider: "claude-headless",
    providerFamily: "claude",
    description: "same-host remote provider-run binding and placement",
    classification: "provider-error",
    runtimeSignals: ["lease-health", "provider-run-lifecycle"],
    exitCriteria: [
      "remote agent provider run is bound to the selected worker kernel",
      "provider output is observed by the home session without a parallel client path",
    ],
  }),
  providerScenario({
    id: "provider-auth-health",
    provider: "codex",
    description: "remote Codex provider profile/auth health on worker",
    classification: "provider-auth",
    runtimeSignals: ["provider-run-lifecycle"],
    exitCriteria: [
      "worker advertises the provider selected by the home kernel",
      "provider authentication is sufficient for a remote prompt turn",
    ],
  }),
  providerScenario({
    id: "ui-client-projection",
    provider: "opencode",
    description: "remote worker projection appears in waiting-room/client rows",
    classification: "ui-client-projection",
    runtimeSignals: ["client-projection-health", "lease-health", "runtime-projection-health"],
    exitCriteria: [
      "terminal client projection includes the selected remote worker kernel",
      "client-visible state matches the worker selected for the remote agent",
    ],
  }),
  {
    id: "lease-reconnect",
    description: "same-host remote lease restore and worker rebind after kernel restarts",
    script: remoteRestartDrill,
    args: ["--keep-artifacts-on-failure"],
    classification: "relay-target-freshness",
    runtimeSignals: ["lease-health", "relay-target-freshness"],
    deployment: "same-host-remote",
    provider: "dev-stub",
    exitCriteria: [
      "home restart preserves the remote binding",
      "worker restart refreshes stale leased-agent state before the next prompt",
    ],
  },
  {
    id: "collab-remote-agent",
    description: "same-host collab remote agent authority through home-owned extension drill",
    script: remoteHomeExtensionDrill,
    args: ["--collab"],
    classification: "kernel-authority",
    runtimeSignals: ["lease-health", "session-authority"],
    deployment: "same-host-remote",
    provider: "dev-stub",
    exitCriteria: [
      "collaborator-owned remote agent runs through the home session authority path",
      "collaborator cannot grant, revoke, or widen home-owned capabilities",
    ],
  },
  {
    id: "remote-kernel-single-user-remote-agent",
    description: "remote kernel worker remote agent lease and home execution authority",
    script: remoteHomeExtensionDrill,
    args: ["--remote-kernel-worker"],
    requires: ["remote-kernel"],
    classification: "worker-execution",
    runtimeSignals: ["agent-lifecycle", "lease-health", "session-authority"],
    deployment: "remote-kernel",
    provider: "dev-stub",
    exitCriteria: [
      "remote worker on remote kernel receives the leased agent",
      "home remains authority for projected runtime capabilities",
    ],
  },
  {
    id: "remote-kernel-collab-remote-agent",
    description: "remote kernel worker collab remote agent authority",
    script: remoteHomeExtensionDrill,
    args: ["--remote-kernel-worker", "--collab"],
    requires: ["remote-kernel"],
    classification: "kernel-authority",
    runtimeSignals: ["lease-health", "session-authority"],
    deployment: "remote-kernel",
    provider: "dev-stub",
    exitCriteria: [
      "collab remote agent can run on the remote kernel worker",
      "home-side grant/revoke authority remains enforced across machines",
    ],
  },
  {
    id: "hosted-single-user-remote-agent",
    description: "hosted Cloud relay second-kernel remote agent",
    script: hostedCloudDrill,
    args: [],
    env: {
      CHARIOX_CLOUD_HOSTED_SECOND_KERNEL: "1",
      CHARIOX_CLOUD_HOSTED_MULTI_USER: "0",
    },
    requires: ["hosted-cloud"],
    classification: "relay-runtime",
    runtimeSignals: ["agent-lifecycle", "home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "relay-target-freshness"],
    deployment: "hosted-cloud",
    provider: "dev-stub",
    exitCriteria: [
      "Cloud-issued relay credentials connect home and worker kernels",
      "hosted second-kernel remote agent completes a provider turn",
      "home-owned script, MCP, and connector tools execute on home and stale calls are blocked after revoke",
    ],
  },
  {
    id: "hosted-collab-remote-agent",
    description: "hosted Cloud relay collab remote agent",
    script: hostedCloudDrill,
    args: [],
    env: {
      CHARIOX_CLOUD_HOSTED_SECOND_KERNEL: "1",
      CHARIOX_CLOUD_HOSTED_MULTI_USER: "1",
    },
    requires: ["hosted-cloud"],
    classification: "kernel-authority",
    runtimeSignals: ["home-extension-manifest-sync", "lease-health", "session-authority"],
    deployment: "hosted-cloud",
    provider: "dev-stub",
    exitCriteria: [
      "second Cloud user joins through session-scoped relay credentials",
      "collab remote agent authority stays owned by the home kernel",
      "collaborator cannot grant, revoke, request, or inspect home-owned extension credentials",
    ],
  },
]

function providerScenario({ id, provider, providerFamily = provider, description, classification, runtimeSignals, exitCriteria }) {
  return {
    id,
    provider,
    providerFamily,
    description,
    script: remoteMachineDrill,
    args: ["--provider", provider],
    classification,
    runtimeSignals,
    deployment: "same-host-remote",
    exitCriteria,
  }
}

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/live-remote-agent-runtime-matrix-drill.mjs [options]",
    "",
    "Runs leased remote-agent lifecycle, prompt dispatch, provider-run binding, collab, and deployment evidence.",
    "By default this runs same-host/self-hosted relay scenarios. remote kernel and hosted Cloud are opt-in.",
    "",
    "Options:",
    "  --include-remote-kernel        Include remote kernel worker scenarios",
    "  --include-hosted-cloud   Include hosted Cloud relay scenarios",
    "  --only IDS               Comma-separated scenario ids",
    "  --dry-run                Print selected commands without running drills",
    "  --continue-on-failure    Run every selected scenario before exiting non-zero",
    "  --scenario-provider I=P  Override a provider-backed scenario provider",
    "  --report PATH            Write a machine-readable matrix report; defaults under .artifacts/drill-matrices",
    "  --artifact-index PATH    Write a verifiable artifact index for the matrix report",
    "  --provider-model P=M     Override model for provider-backed scenarios",
    "  --provider-account P=A   Label the provider account/profile used by this matrix without exposing credentials",
    "  --remote-kernel-host HOST      Forwarded to remote kernel drill scenarios",
    "  --remote-kernel-key PATH       Forwarded to remote kernel drill scenarios",
    "  --remote-kernel-repo PATH      Forwarded to remote kernel drill scenarios",
    "",
    "Environment defaults:",
    `  CHARIOX_REMOTE_AGENT_RUNTIME_CODEX_MODEL=${DEFAULT_CODEX_MODEL}`,
    `  CHARIOX_REMOTE_AGENT_RUNTIME_OPENCODE_MODEL=${DEFAULT_OPENCODE_MODEL}`,
    `  CHARIOX_REMOTE_AGENT_RUNTIME_CLAUDE_MODEL=${DEFAULT_CLAUDE_MODEL}`,
    "",
    "Scenario ids:",
    ...MATRIX.map((scenario) => `  ${scenario.id.padEnd(34)} ${scenario.description}`),
  ].join("\n"))
}

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function parseArgs(argv) {
  const options = {
    includeRemoteKernel: false,
    includeHostedCloud: false,
    only: null,
    dryRun: false,
    continueOnFailure: false,
    reportPath: null,
    artifactIndexPath: null,
    providerAccounts: {},
    providerModels: {},
    scenarioProviders: {},
    passthrough: [],
    help: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") continue
    else if (arg === "--include-remote-kernel") options.includeRemoteKernel = true
    else if (arg === "--include-hosted-cloud") options.includeHostedCloud = true
    else if (arg === "--dry-run") options.dryRun = true
    else if (arg === "--continue-on-failure") options.continueOnFailure = true
    else if (arg === "--scenario-provider") applyScenarioProviderOverride(options.scenarioProviders, readValue(argv, index++, arg))
    else if (arg.startsWith("--scenario-provider=")) applyScenarioProviderOverride(options.scenarioProviders, arg.slice("--scenario-provider=".length))
    else if (arg === "--report") options.reportPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--report=")) options.reportPath = arg.slice("--report=".length)
    else if (arg === "--artifact-index") options.artifactIndexPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--artifact-index=")) options.artifactIndexPath = arg.slice("--artifact-index=".length)
    else if (arg === "--provider-account") applyProviderAccountAlias(options.providerAccounts, readValue(argv, index++, arg))
    else if (arg.startsWith("--provider-account=")) applyProviderAccountAlias(options.providerAccounts, arg.slice("--provider-account=".length))
    else if (arg === "--provider-model") applyProviderModelOverride(options.providerModels, readValue(argv, index++, arg))
    else if (arg.startsWith("--provider-model=")) applyProviderModelOverride(options.providerModels, arg.slice("--provider-model=".length))
    else if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--only") options.only = parseDrillScenarioIds(readValue(argv, index++, arg))
    else if (arg.startsWith("--only=")) options.only = parseDrillScenarioIds(arg.slice("--only=".length))
    else {
      const remoteKernelArg = parseRemoteKernelPassthroughArg(argv, index)
      if (remoteKernelArg) {
        options.passthrough.push(...remoteKernelArg.args)
        index = remoteKernelArg.nextIndex
        continue
      }
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  return options
}

function selectScenarios(options) {
  const enabledRequirements = new Set()
  if (options.includeRemoteKernel) enabledRequirements.add("remote-kernel")
  if (options.includeHostedCloud) enabledRequirements.add("hosted-cloud")
  const selected = selectDrillMatrixScenarios({
    scenarios: MATRIX,
    requestedIds: options.only,
    enabledRequirements,
    requirementLabels: {
      "remote-kernel": "--include-remote-kernel",
      "hosted-cloud": "--include-hosted-cloud",
    },
  })
  return selected.map((scenario) => scenarioWithProviderOverride(scenario, options.scenarioProviders[scenario.id]))
}

function applyScenarioProviderOverride(target, value) {
  const separator = value.indexOf("=")
  if (separator <= 0 || separator === value.length - 1) {
    throw new Error("--scenario-provider must use SCENARIO=PROVIDER")
  }
  const scenarioId = value.slice(0, separator).trim()
  const provider = value.slice(separator + 1).trim()
  const scenario = MATRIX.find((item) => item.id === scenarioId)
  if (!scenario || scenario.script !== remoteMachineDrill) {
    throw new Error(`scenario provider override is not supported for ${scenarioId}`)
  }
  if (!["codex", "opencode", "claude-headless"].includes(provider)) {
    throw new Error(`unsupported remote scenario provider: ${provider}`)
  }
  target[scenarioId] = provider
}

function scenarioWithProviderOverride(scenario, provider) {
  if (!provider) return scenario
  return {
    ...scenario,
    provider,
    providerFamily: provider === "claude-headless" ? "claude" : provider,
    args: ["--provider", provider],
  }
}

function modelForScenario(scenario, options) {
  const providerFamily = scenario.providerFamily ?? scenario.provider
  const explicit = options.providerModels[scenario.provider] ?? options.providerModels[providerFamily]
  if (explicit) return explicit
  const defaultModel = providerFamily === "codex"
    ? DEFAULT_CODEX_MODEL
    : providerFamily === "opencode"
      ? DEFAULT_OPENCODE_MODEL
      : DEFAULT_CLAUDE_MODEL
  return resolveProviderModel(scenario.provider, {
    defaultModel,
    providerModels: options.providerModels,
  })
}

function commandForScenario(scenario, options) {
  let args = [scenario.script, ...scenario.args]
  if (scenario.script === remoteMachineDrill) {
    args = [...args, "--provider-model", `${scenario.provider}=${modelForScenario(scenario, options)}`]
  }
  return {
    command: process.execPath,
    args: appendRemoteKernelPassthrough(args, scenario, options.passthrough),
    env: scenario.env,
  }
}

function metadataFor(selected, options) {
  const providers = [...new Set(selected
    .map((scenario) => scenario.providerFamily ?? scenario.provider)
    .filter((provider) => provider && provider !== "dev-stub"))].sort()
  return {
    includeRemoteKernel: options.includeRemoteKernel,
    includeHostedCloud: options.includeHostedCloud,
    generatedMatrixNames: "remote-agent-runtime-matrix",
    generatedMatrixRepos: "oss",
    ...drillDeploymentPresetMetadata([
      "same-host-remote",
      "self-hosted-relay",
      ...(options.includeRemoteKernel ? ["remote-kernel"] : []),
      ...(options.includeHostedCloud ? ["hosted-cloud"] : []),
    ], { remoteKernelPassthrough: options.passthrough }),
    ...providerProfileMetadata({
      providers,
      defaultModel: "per-provider",
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
  const reportPath = options.reportPath ?? defaultDrillMatrixReportPath("remote-agent-runtime-matrix", { rootDir: repoRoot })
  const artifactIndexPath = options.artifactIndexPath ?? defaultDrillMatrixArtifactIndexPath(reportPath)
  const results = await runDrillMatrix({
    matrixName: "remote-agent-runtime-matrix",
    scenarios: selected,
    commandForScenario: (scenario) => commandForScenario(scenario, options),
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
  console.error(`[remote-agent-runtime-matrix] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
