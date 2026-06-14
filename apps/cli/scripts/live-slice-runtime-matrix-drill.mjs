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
import { drillDeploymentPresetMetadata } from "./lib/drill-environment-presets.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const sliceLifecycleDrill = path.join(scriptDir, "live-slice-lifecycle-drill.mjs")
const claudeHeadlessSliceDrill = path.join(scriptDir, "live-claude-headless-slice-drill.mjs")
const dockerBrowserStateDrill = path.join(scriptDir, "live-docker-slice-browser-state-drill.mjs")

const MATRIX = [
  scenario({
    id: "slice-lifecycle",
    description: "local Docker slice create, start, display, stop, delete, and cleanup",
    script: sliceLifecycleDrill,
    classification: "slice-runtime",
    deployment: "local",
    exitCriteria: [
      "kernel-owned slice lifecycle reaches running only after worker discovery",
      "headed display endpoint and cleanup remain driven by kernel state",
    ],
  }),
  scenario({
    id: "provider-auth",
    description: "slice provider auth summaries, login, aliases, and removal",
    script: sliceLifecycleDrill,
    classification: "slice-auth",
    deployment: "local",
    providers: ["claude", "codex", "opencode"],
    exitCriteria: [
      "Codex, OpenCode, and Claude account summaries are extracted for a slice",
      "provider auth alias, login, import, and removal flow through kernel requests",
    ],
  }),
  scenario({
    id: "session-start",
    description: "session creation accepts only slices scoped to the selected worktree",
    script: sliceLifecycleDrill,
    classification: "kernel-authority",
    deployment: "local",
    exitCriteria: [
      "session creation binds the initial agent to a same-worktree slice",
      "wrong-worktree slice references are rejected before launch",
    ],
  }),
  scenario({
    id: "agent-reuse",
    description: "multiple agents and sessions reuse a worktree-compatible slice",
    script: sliceLifecycleDrill,
    classification: "worker-execution",
    deployment: "local",
    exitCriteria: [
      "new agents can join an existing slice from the same worktree",
      "active-agent slice deletion is blocked by kernel lifecycle policy",
    ],
  }),
  scenario({
    id: "ui-projection",
    description: "TUI slash-command and waiting-room slice projection",
    script: sliceLifecycleDrill,
    classification: "ui-client-projection",
    deployment: "local",
    exitCriteria: [
      "slash-command status renders provider auth aliases from kernel state",
      "waiting-room delete confirms and refreshes slice inventory through shared controllers",
    ],
  }),
  scenario({
    id: "docker-browser-state",
    description: "local Docker slice browser and saved-state persistence",
    script: dockerBrowserStateDrill,
    classification: "docker-runtime",
    deployment: "local",
    requires: ["browser-state"],
    exitCriteria: [
      "Docker slice browser state survives save and restore",
      "slice filesystem and browser state are validated through runtime artifacts",
    ],
  }),
  scenario({
    id: "self-hosted-relay-claude-headless",
    description: "self-hosted relay slice worker with Claude headless provider execution",
    script: claudeHeadlessSliceDrill,
    classification: "worker-execution",
    deployment: "self-hosted-relay",
    requires: ["self-hosted-relay"],
    providers: ["claude"],
    exitCriteria: [
      "slice worker connects through a self-hosted relay while home owns session authority",
      "Claude headless provider output returns through the normal session history path",
    ],
  }),
]

function scenario(definition) {
  return {
    args: [],
    providers: [],
    requires: [],
    ...definition,
  }
}

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/live-slice-runtime-matrix-drill.mjs [options]",
    "",
    "Runs slice lifecycle, provider-auth, session/agent binding, UI projection, and relay-backed provider scenarios.",
    "By default this runs local slice lifecycle scenarios. Browser-state and self-hosted relay scenarios are opt-in.",
    "",
    "Options:",
    "  --include-browser-state        Include Docker browser saved-state persistence scenario",
    "  --include-self-hosted-relay    Include self-hosted relay slice provider scenario",
    "  --only IDS                     Comma-separated scenario ids",
    "  --dry-run                      Print selected commands without running drills",
    "  --continue-on-failure          Run every selected scenario before exiting non-zero",
    "  --report PATH                  Write a machine-readable matrix report; defaults under .artifacts/drill-matrices",
    "  --artifact-index PATH          Write a verifiable artifact index for the matrix report",
    "",
    "Scenario ids:",
    ...MATRIX.map((scenarioItem) => `  ${scenarioItem.id.padEnd(34)} ${scenarioItem.description}`),
  ].join("\n"))
}

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function parseArgs(argv) {
  const options = {
    includeBrowserState: false,
    includeSelfHostedRelay: false,
    only: null,
    dryRun: false,
    continueOnFailure: false,
    reportPath: null,
    artifactIndexPath: null,
    help: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") continue
    else if (arg === "--include-browser-state") options.includeBrowserState = true
    else if (arg === "--include-self-hosted-relay") options.includeSelfHostedRelay = true
    else if (arg === "--dry-run") options.dryRun = true
    else if (arg === "--continue-on-failure") options.continueOnFailure = true
    else if (arg === "--report") options.reportPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--report=")) options.reportPath = arg.slice("--report=".length)
    else if (arg === "--artifact-index") options.artifactIndexPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--artifact-index=")) options.artifactIndexPath = arg.slice("--artifact-index=".length)
    else if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--only") options.only = parseDrillScenarioIds(readValue(argv, index++, arg))
    else if (arg.startsWith("--only=")) options.only = parseDrillScenarioIds(arg.slice("--only=".length))
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function selectScenarios(options) {
  const enabledRequirements = new Set()
  if (options.includeBrowserState) enabledRequirements.add("browser-state")
  if (options.includeSelfHostedRelay) enabledRequirements.add("self-hosted-relay")
  return selectDrillMatrixScenarios({
    scenarios: MATRIX,
    requestedIds: options.only,
    enabledRequirements,
    requirementLabels: {
      "browser-state": "--include-browser-state",
      "self-hosted-relay": "--include-self-hosted-relay",
    },
  })
}

function commandForScenario(scenarioItem) {
  return {
    command: process.execPath,
    args: [scenarioItem.script, ...scenarioItem.args, "--keep-artifacts-on-failure"],
  }
}

function metadataFor(selected, options) {
  const providers = [...new Set(selected.flatMap((scenarioItem) => scenarioItem.providers ?? []))].sort()
  return {
    includeBrowserState: options.includeBrowserState,
    includeSelfHostedRelay: options.includeSelfHostedRelay,
    providers: providers.join(","),
    ...drillDeploymentPresetMetadata([
      "local",
      ...(options.includeSelfHostedRelay ? ["self-hosted-relay"] : []),
    ]),
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const selected = selectScenarios(options)
  const reportPath = options.reportPath ?? defaultDrillMatrixReportPath("slice-runtime-matrix", { rootDir: repoRoot })
  const artifactIndexPath = options.artifactIndexPath ?? defaultDrillMatrixArtifactIndexPath(reportPath)
  const results = await runDrillMatrix({
    matrixName: "slice-runtime-matrix",
    scenarios: selected,
    commandForScenario,
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
  console.error(`[slice-runtime-matrix] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
