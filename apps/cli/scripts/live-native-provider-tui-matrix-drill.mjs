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
  providerProfileMetadata,
} from "./lib/drill-provider-profiles.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const localCommandDrill = path.join(scriptDir, "live-native-tui-provider-command-drill.mjs")
const localPermissionDrill = path.join(scriptDir, "live-native-tui-permission-drill.mjs")
const remoteNativeDrill = path.join(scriptDir, "live-remote-native-tui-drill.mjs")

const ALL_PROVIDERS = "opencode,codex,claude"
const LOCAL_COMMAND_PROVIDERS = "codex,opencode"

const MATRIX = [
  scenario({
    id: "local-native-tui",
    description: "local native provider TUI command path",
    script: localCommandDrill,
    args: ["--providers", LOCAL_COMMAND_PROVIDERS],
    classification: "provider-error",
    runtimeSignals: ["provider-run-lifecycle", "session-authority"],
    deployment: "local",
    providers: ["codex", "opencode"],
    exitCriteria: [
      "native provider TUI commands enter the kernel-owned prompt path",
      "provider output is visible through the normal session history path",
    ],
  }),
  scenario({
    id: "permission-visibility",
    description: "native provider permission prompts surface through kernel interactions",
    script: localPermissionDrill,
    args: ["--providers", ALL_PROVIDERS],
    classification: "kernel-authority",
    runtimeSignals: ["permission-interaction", "session-authority"],
    deployment: "local",
    providers: ["claude", "codex", "opencode"],
    exitCriteria: [
      "provider-native permission requests create kernel-owned runtime interactions",
      "Arroba observers can resolve the interaction without a provider-specific client path",
    ],
  }),
  scenario({
    id: "provider-auth-health",
    description: "native provider auth/profile readiness across supported providers",
    script: localPermissionDrill,
    args: ["--providers", ALL_PROVIDERS],
    classification: "provider-auth",
    runtimeSignals: ["provider-run-lifecycle"],
    deployment: "local",
    providers: ["claude", "codex", "opencode"],
    exitCriteria: [
      "provider profiles selected by native TUI drills are authenticated",
      "auth failures are classified before native runtime behavior is treated as failed",
    ],
  }),
  scenario({
    id: "remote-native-tui",
    description: "same-host remote native provider TUI via standard home/worker relay path",
    script: remoteNativeDrill,
    args: ["--standard-home-worker", "--providers", ALL_PROVIDERS],
    classification: "relay-runtime",
    runtimeSignals: ["provider-run-lifecycle", "session-authority"],
    deployment: "same-host-remote",
    providers: ["claude", "codex", "opencode"],
    exitCriteria: [
      "remote native TUIs attach to the home kernel session",
      "worker provider runs execute through the leased-agent relay protocol",
    ],
  }),
  scenario({
    id: "slice-native-tui",
    description: "home-managed slice-backed native provider TUI",
    script: remoteNativeDrill,
    args: ["--home-managed-slice-local-docker", "--providers", ALL_PROVIDERS],
    classification: "worker-execution",
    runtimeSignals: ["provider-run-lifecycle", "session-authority"],
    deployment: "local",
    providers: ["claude", "codex", "opencode"],
    exitCriteria: [
      "slice selection remains a home-managed worker execution environment",
      "native TUIs still attach through the normal home kernel session path",
    ],
  }),
  scenario({
    id: "transcript-parity",
    description: "native provider and Arroba observers share transcript projection",
    script: remoteNativeDrill,
    args: ["--standard-home-worker", "--providers", ALL_PROVIDERS, "--include-attachments"],
    classification: "ui-client-projection",
    runtimeSignals: ["client-projection-health", "runtime-projection-health", "session-authority"],
    deployment: "same-host-remote",
    providers: ["claude", "codex", "opencode"],
    exitCriteria: [
      "native-origin and Arroba-origin turns render in one session transcript",
      "attachment and output projection are observed through normal terminal events",
    ],
  }),
  scenario({
    id: "hetzner-native-tui",
    description: "Hetzner worker native provider TUI relay path",
    script: remoteNativeDrill,
    args: ["--hetzner-worker", "--providers", ALL_PROVIDERS],
    classification: "worker-execution",
    runtimeSignals: ["provider-run-lifecycle", "session-authority"],
    deployment: "hetzner",
    providers: ["claude", "codex", "opencode"],
    requires: ["hetzner"],
    exitCriteria: [
      "remote worker on Hetzner accepts native TUI leased-agent execution",
      "relay remains transport-only while home kernel owns session authority",
    ],
  }),
]

function scenario(definition) {
  return {
    ...definition,
    requires: definition.requires ?? [],
  }
}

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs [options]",
    "",
    "Runs native provider TUI validation scenarios through the existing live drills.",
    "By default this runs local and same-host scenarios. Hetzner is opt-in.",
    "",
    "Options:",
    "  --include-hetzner        Include Hetzner worker scenarios",
    "  --only IDS               Comma-separated scenario ids",
    "  --dry-run                Print selected commands without running drills",
    "  --continue-on-failure    Run every selected scenario before exiting non-zero",
    "  --report PATH            Write a machine-readable matrix report; defaults under .artifacts/drill-matrices",
    "  --artifact-index PATH    Write a verifiable artifact index for the matrix report",
    "  --provider-account P=A   Label the provider account/profile used by this matrix without exposing credentials",
    "  --hetzner-host HOST      Forwarded to Hetzner drill scenarios",
    "  --hetzner-key PATH       Forwarded to Hetzner drill scenarios",
    "  --hetzner-repo PATH      Forwarded to Hetzner drill scenarios",
    "",
    "Scenario ids:",
    ...MATRIX.map((scenarioItem) => `  ${scenarioItem.id.padEnd(23)} ${scenarioItem.description}`),
  ].join("\n"))
}

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
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
    providerAccounts: {},
    passthrough: [],
    help: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") continue
    else if (arg === "--include-hetzner") options.includeHetzner = true
    else if (arg === "--dry-run") options.dryRun = true
    else if (arg === "--continue-on-failure") options.continueOnFailure = true
    else if (arg === "--report") options.reportPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--report=")) options.reportPath = arg.slice("--report=".length)
    else if (arg === "--artifact-index") options.artifactIndexPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--artifact-index=")) options.artifactIndexPath = arg.slice("--artifact-index=".length)
    else if (arg === "--provider-account") applyProviderAccountAlias(options.providerAccounts, readValue(argv, index++, arg))
    else if (arg.startsWith("--provider-account=")) applyProviderAccountAlias(options.providerAccounts, arg.slice("--provider-account=".length))
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
  return selectDrillMatrixScenarios({
    scenarios: MATRIX,
    requestedIds: options.only,
    enabledRequirements: new Set(options.includeHetzner ? ["hetzner"] : []),
    requirementLabels: { hetzner: "--include-hetzner" },
  })
}

function commandForScenario(scenarioItem, passthrough) {
  return {
    command: process.execPath,
    args: appendHetznerPassthrough([
      scenarioItem.script,
      ...scenarioItem.args,
      "--keep-artifacts-on-failure",
    ], scenarioItem, passthrough),
  }
}

function metadataFor(selected, options) {
  const providers = [...new Set(selected.flatMap((scenarioItem) => scenarioItem.providers ?? []))].sort()
  const deploymentPresets = [
    "local",
    "same-host-remote",
    "self-hosted-relay",
    ...(options.includeHetzner ? ["hetzner"] : []),
  ]
  return {
    includeHetzner: options.includeHetzner,
    ...providerProfileMetadata({ providers, defaultModel: "provider-default", providerAccounts: options.providerAccounts }),
    ...drillDeploymentPresetMetadata(deploymentPresets, { hetznerPassthrough: options.passthrough }),
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const selected = selectScenarios(options)
  const reportPath = options.reportPath ?? defaultDrillMatrixReportPath("native-provider-tui-matrix", { rootDir: repoRoot })
  const artifactIndexPath = options.artifactIndexPath ?? defaultDrillMatrixArtifactIndexPath(reportPath)
  const results = await runDrillMatrix({
    matrixName: "native-provider-tui-matrix",
    scenarios: selected,
    commandForScenario: (scenarioItem) => commandForScenario(scenarioItem, options.passthrough),
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
  console.error(`[native-provider-tui-matrix] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
