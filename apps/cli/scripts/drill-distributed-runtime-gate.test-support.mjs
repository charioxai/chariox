import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex, writeDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
import { DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES_BY_REPO } from "./lib/drill-distributed-runtime-evidence.mjs"
import { drillFailureTaxonomyManifest } from "./lib/drill-failure-taxonomy.mjs"
import {
  DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS,
  drillRuntimeAuthorityManifest,
} from "./lib/drill-runtime-authority-invariants.mjs"
import { drillRuntimeSignalOwnersFor, drillRuntimeSignalsManifest } from "./lib/drill-runtime-signals.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-distributed-runtime-gate.mjs", import.meta.url))
const summaryScriptPath = fileURLToPath(new URL("./drill-validation-gate-summary.mjs", import.meta.url))


export async function writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud }) {
  const ossMatrixRoot = path.join(ossRoot, ".artifacts", "drill-matrices")
  await writeMatrixReport(path.join(ossMatrixRoot, "native-provider-tui.json"), {
    matrix: "native-provider-tui-matrix",
    metadata: {
      deploymentPresets: "hetzner,local,same-host-remote,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("local-native-tui", "kernel-authority", ["provider-run-lifecycle", "session-authority"]),
      scenario("permission-visibility", "ui-client-projection", ["permission-interaction"]),
      scenario("remote-native-tui", "relay-runtime", ["provider-run-lifecycle", "session-authority"]),
      scenario("slice-native-tui", "worker-execution", ["provider-run-lifecycle", "session-authority"]),
      scenario("transcript-parity", "provider-error", ["client-projection-health", "runtime-projection-health"]),
      scenario("provider-auth-health", "provider-auth", ["provider-run-lifecycle"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "remote-agent-runtime.json"), {
    matrix: "remote-agent-runtime-matrix",
    metadata: {
      deploymentPresets: includeCloud
        ? "hetzner,hosted-cloud,same-host-remote,self-hosted-relay"
        : "hetzner,same-host-remote,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
      scenario("hetzner-collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
      scenario("hetzner-single-user-remote-agent", "worker-execution", ["agent-lifecycle", "lease-health", "session-authority"]),
      ...(includeCloud
        ? [
          scenario("hosted-collab-remote-agent", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
          scenario("hosted-single-user-remote-agent", "relay-runtime", ["agent-lifecycle", "home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "relay-target-freshness"]),
        ]
        : []),
      scenario("lease-reconnect", "relay-target-freshness", ["lease-health", "relay-target-freshness"]),
      scenario("provider-run-binding", "worker-execution", ["lease-health", "provider-run-lifecycle"]),
      scenario("remote-prompt-dispatch", "relay-runtime", ["agent-lifecycle", "provider-run-lifecycle"]),
      scenario("single-user-remote-agent", "ui-client-projection", ["agent-lifecycle", "client-projection-health", "runtime-projection-health", "session-authority"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "remote-home-extension.json"), {
    matrix: "remote-home-extension-matrix",
    metadata: {
      deploymentPresets: "hetzner,local,self-hosted-relay",
    },
    scenarios: [
      scenario("local-single", "remote-extension-sync", ["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"]),
      scenario("local-collab", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
      scenario("hetzner-single", "worker-execution", ["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"]),
      scenario("hetzner-collab", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "runtime-resilience-chaos.json"), {
    matrix: "runtime-resilience-chaos-matrix",
    metadata: {
      deploymentPresets: includeCloud
        ? "hetzner,hosted-cloud,local,same-host-remote,self-hosted-relay"
        : "hetzner,local,same-host-remote,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("local-kernel-websocket-drop", "relay-runtime", ["client-projection-health", "relay-target-freshness"]),
      scenario("local-kernel-restart-durable-state", "kernel-authority", ["provider-run-lifecycle", "runtime-transition-audit", "session-authority"]),
      scenario("local-relay-restart-reconnect", "relay-target-freshness", ["provider-run-lifecycle", "relay-target-freshness", "session-authority"]),
      scenario("local-tui-web-terminal-parity", "ui-client-projection", ["client-projection-health", "runtime-projection-health", "session-authority"]),
      scenario("same-host-remote-worker-restart", "relay-target-freshness", ["lease-health", "relay-target-freshness", "session-authority"]),
      scenario("worker-provider-resume-codex", "provider-error", ["lease-health", "provider-run-lifecycle", "session-authority"]),
      scenario("worker-provider-resume-opencode", "provider-error", ["lease-health", "provider-run-lifecycle", "session-authority"]),
      ...(includeCloud
        ? [
          scenario("hetzner-collaborator-reconnect-authority", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
          scenario("hosted-cloud-relay-second-kernel-reconnect", "relay-runtime", ["agent-lifecycle", "lease-health", "provider-run-lifecycle", "relay-target-freshness"]),
        ]
        : []),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "slice-runtime.json"), {
    matrix: "slice-runtime-matrix",
    metadata: {
      deploymentPresets: "local,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("agent-reuse", "worker-execution", ["agent-lifecycle", "slice-runtime-state"]),
      scenario("docker-browser-state", "docker-runtime", ["slice-runtime-state"]),
      scenario("provider-auth", "slice-auth", ["provider-run-lifecycle", "slice-auth-state"]),
      scenario("session-start", "kernel-authority", ["session-authority", "slice-runtime-state"]),
      scenario("slice-lifecycle", "slice-runtime", ["slice-runtime-state"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "workspace-live-sync.json"), {
    matrix: "workspace-live-sync-matrix",
    metadata: {
      deploymentPresets: "hetzner,local,same-host-remote,self-hosted-relay",
      providers: "codex,opencode",
    },
    scenarios: [
      scenario("local-managed-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("local-tracked-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("local-permission-codex", "kernel-authority", ["session-authority", "workspace-live-sync-state"]),
      scenario("remote-managed-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("remote-tracked-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("remote-tracked-restart-codex", "relay-target-freshness", ["relay-target-freshness", "session-authority", "workspace-live-sync-state"]),
    ],
  })

  if (includeCloud) {
    await writeMatrixReport(path.join(cloudRoot, ".artifacts", "drill-matrices", "browser-terminal-resilience.json"), {
      matrix: "browser-terminal-resilience-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud,local",
        providerCount: 3,
        providers: "claude,codex,opencode",
        defaultModel: "provider-default",
        providerModelOverrides: "",
      },
      scenarios: [
        scenario("local-browser-relay-kernel-reconnect", "relay-runtime", ["client-projection-health", "provider-run-lifecycle", "relay-target-freshness", "session-authority"], { providers: ["claude", "codex", "opencode"] }),
        scenario("hosted-browser-relay-kernel-reconnect", "cloud-runtime", ["client-projection-health", "provider-run-lifecycle", "relay-target-freshness", "session-authority"], { providers: ["claude", "codex", "opencode"] }),
      ],
    })
    await writeMatrixReport(path.join(cloudRoot, ".artifacts", "drill-matrices", "cloud-slice-runtime.json"), {
      matrix: "cloud-slice-runtime-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud",
        providerCount: 3,
        providers: "claude,codex,opencode",
        defaultModel: "provider-default",
        providerModelOverrides: "",
      },
      scenarios: [
        scenario("ui-projection", "ui-client-projection", ["client-projection-health", "runtime-projection-health"], { providers: ["claude", "codex", "opencode"] }),
      ],
    })
  }
}

export async function writeMatrixReport(file, { matrix, metadata, scenarios }) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.matrix.v1",
    matrix,
    status: "passed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata,
    scenarios,
  }, null, 2)}\n`, "utf8")
}

export async function writeValidationSuiteArtifact(rootDir, {
  coverageAreas = ["distributed-observability", "suite-contract"],
  evidenceRepo = "cloud",
  providerAccountAliases = "",
  plannedOwners = "",
  plannedClassifications = "",
  validationPresets = evidenceRepo === "oss" ? "distributed-runtime,distributed-state-health,runtime-authority" : "cloud-distributed-runtime",
} = {}) {
  const artifactPath = path.join(rootDir, "cloud-validation-suite.json")
  await mkdir(rootDir, { recursive: true })
  await writeFile(artifactPath, `${JSON.stringify({
    schema: "arroba.drill.validation_suite_run.v1",
    status: "passed",
    ok: true,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    exitCode: 0,
    signal: null,
    error: null,
    testCount: 1,
    command: "node --test scripts/cloud-validation-suite.test.mjs",
    testPaths: ["scripts/cloud-validation-suite.test.mjs"],
    manifest: {
      schema: "arroba.drill.validation_suite.v1",
      testCount: 1,
      command: "node --test scripts/cloud-validation-suite.test.mjs",
      coverage: [{
        id: "suite-contract",
        description: "Cloud validation-suite contract",
        testCount: 1,
        testPaths: ["scripts/cloud-validation-suite.test.mjs"],
      }],
      failureTaxonomyManifest: drillFailureTaxonomyManifest(),
      runtimeAuthorityManifest: drillRuntimeAuthorityManifest(),
      runtimeSignalsManifest: drillRuntimeSignalsManifest(),
      testPaths: ["scripts/cloud-validation-suite.test.mjs"],
    },
  }, null, 2)}\n`, "utf8")
  await writeDrillArtifactIndex({
    rootDir,
    artifacts: ["cloud-validation-suite.json"],
    metadata: {
      drill: "cloud-validation-suite",
      tests: 1,
      coverageAreas: coverageAreas.join(","),
      runtimeAuthorityInvariants: DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS.join(","),
      runtimeSignals: DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS.join(","),
      runtimeSignalOwners: drillRuntimeSignalOwnersFor(DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS).join(","),
      owners: "validation-platform",
      classifications: evidenceRepo === "cloud" ? "cloud-validation-suite" : "validation-suite",
      requiredFailureClassifications: DISTRIBUTED_RUNTIME_REQUIRED_FAILURE_CLASSIFICATIONS.join(","),
      artifactKinds: "validation-suite-run",
      evidenceRepos: evidenceRepo,
      generatedMatrixNames: generatedMatrixNamesForEvidenceRepo(evidenceRepo).join(","),
      generatedMatrixRepos: evidenceRepo,
      generatedEvidenceRepos: evidenceRepo,
      validationPresets,
      ...(providerAccountAliases ? { providerAccountAliases } : {}),
      ...(plannedOwners ? { plannedOwners } : {}),
      ...(plannedClassifications ? { plannedClassifications } : {}),
      exitCriterionStatuses: "satisfied",
    },
  })
  return path.join(rootDir, "arroba-drill-artifacts.json")
}

export function generatedMatrixNamesForEvidenceRepo(evidenceRepo) {
  return DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES_BY_REPO[evidenceRepo] ?? []
}

export async function writeCloudGeneratedMatrixRegistry(cloudRoot, {
  matrices = [
    { name: "browser-terminal-resilience-matrix", repo: "cloud" },
    { name: "cloud-slice-runtime-matrix", repo: "cloud" },
    { name: "native-provider-tui-matrix", repo: "oss" },
    { name: "remote-agent-runtime-matrix", repo: "oss" },
    { name: "remote-home-extension-matrix", repo: "oss" },
    { name: "runtime-resilience-chaos-matrix", repo: "oss" },
    { name: "slice-runtime-matrix", repo: "oss" },
    { name: "workspace-live-sync-matrix", repo: "oss" },
  ],
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-drill-generated-matrix-names.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudDrillGeneratedMatrixNamesManifest() {",
    `  return { schema: "arroba.cloud.drill.generated_matrix_names.v1", matrices: ${JSON.stringify(matrices)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

export async function writeCloudRuntimeSignalsRegistry(cloudRoot, {
  signals = drillRuntimeSignalsManifest().signals,
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-runtime-signals.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudRuntimeSignalsManifest() {",
    `  return { schema: "arroba.drill.runtime_signals.v1", signals: ${JSON.stringify(signals)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

export async function writeCloudRuntimeAuthorityRegistry(cloudRoot, {
  invariants = drillRuntimeAuthorityManifest().invariants.map((invariant) => invariant.id === "relay-cloud-transport-only"
    ? { ...invariant, id: "cloud-control-plane-only", owner: "cloud-deployment" }
    : invariant),
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-runtime-authority-invariants.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudRuntimeAuthorityManifest() {",
    `  return { schema: "arroba.drill.runtime_authority_invariants.v1", invariants: ${JSON.stringify(invariants)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

export async function writeCloudFailureTaxonomyRegistry(cloudRoot, {
  classifications = drillFailureTaxonomyManifest().classifications
    .map((classification) => classification.kind === "docker-runtime"
      ? { ...classification, owner: "worker-kernel" }
      : classification),
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-failure-taxonomy.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudFailureTaxonomyManifest() {",
    `  return { schema: "arroba.drill.failure_taxonomy.v1", target: "scenario", classifications: ${JSON.stringify(classifications)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

export const DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS = Object.freeze([
  "agent-lifecycle",
  "client-projection-health",
  "home-extension-manifest-sync",
  "lease-health",
  "permission-interaction",
  "provider-run-lifecycle",
  "relay-target-freshness",
  "runtime-projection-health",
  "runtime-transition-audit",
  "session-authority",
  "slice-auth-state",
  "slice-runtime-state",
  "workspace-live-sync-state",
])

export const DISTRIBUTED_RUNTIME_REQUIRED_FAILURE_CLASSIFICATIONS = Object.freeze([
  "cloud-runtime",
  "docker-runtime",
  "kernel-authority",
  "provider-auth",
  "provider-error",
  "projection-staleness",
  "relay-runtime",
  "relay-target-freshness",
  "remote-extension-sync",
  "remote-host-capacity",
  "remote-worker-version",
  "runtime-projection-health",
  "slice-auth",
  "slice-runtime",
  "ui-client-projection",
  "worker-execution",
  "workspace-live-sync-conflict",
])

export async function writeValidationSuiteManifestArtifact(rootDir) {
  const artifactPath = path.join(rootDir, "cloud-validation-suite.json")
  await mkdir(rootDir, { recursive: true })
  await writeFile(artifactPath, `${JSON.stringify({
    schema: "arroba.drill.validation_suite.v1",
    testCount: 1,
    command: "node --test scripts/cloud-validation-suite.test.mjs",
    coverage: [{
      id: "suite-contract",
      description: "Cloud validation-suite contract",
      testCount: 1,
      testPaths: ["scripts/cloud-validation-suite.test.mjs"],
    }],
    testPaths: ["scripts/cloud-validation-suite.test.mjs"],
  }, null, 2)}\n`, "utf8")
  await writeDrillArtifactIndex({
    rootDir,
    artifacts: ["cloud-validation-suite.json"],
    metadata: {
      drill: "cloud-validation-suite",
      tests: 1,
      artifactKinds: "validation-suite",
      evidenceRepos: "cloud",
    },
  })
}

export async function writeFailureManifest(file, {
  drill = "failed-drill",
  message = "Token refresh failed: 401",
} = {}) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.failure.v1",
    rootDir: path.dirname(file),
    failedAt: "2026-06-13T00:00:00.000Z",
    metadata: { drill },
    error: { name: "Error", message, stack: null },
  }, null, 2)}\n`, "utf8")
  return file
}

export async function writeFakeDistributedRuntimeMatrixScripts({ cloudRoot, ossRoot }) {
  await writeFakeMatrixScript({
    file: path.join(ossRoot, "apps", "cli", "scripts", "live-native-provider-tui-matrix-drill.mjs"),
    report: matrixReport({
      matrix: "native-provider-tui-matrix",
      metadata: {
        deploymentPresets: "hetzner,local,same-host-remote,self-hosted-relay",
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("local-native-tui", "kernel-authority", ["provider-run-lifecycle", "session-authority"]),
        scenario("permission-visibility", "ui-client-projection", ["permission-interaction"]),
        scenario("remote-native-tui", "relay-runtime", ["provider-run-lifecycle", "session-authority"]),
        scenario("slice-native-tui", "worker-execution", ["provider-run-lifecycle", "session-authority"]),
        scenario("transcript-parity", "provider-error", ["client-projection-health", "runtime-projection-health"]),
        scenario("provider-auth-health", "provider-auth", ["provider-run-lifecycle"]),
      ],
    }),
  })
  await writeFakeMatrixScript({
    file: path.join(ossRoot, "apps", "cli", "scripts", "live-remote-agent-runtime-matrix-drill.mjs"),
    report: matrixReport({
      matrix: "remote-agent-runtime-matrix",
      metadata: {
        deploymentPresets: "hetzner,hosted-cloud,same-host-remote,self-hosted-relay",
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
        scenario("hetzner-collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
        scenario("hetzner-single-user-remote-agent", "worker-execution", ["agent-lifecycle", "lease-health", "session-authority"]),
        scenario("hosted-collab-remote-agent", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
        scenario("hosted-single-user-remote-agent", "relay-runtime", ["agent-lifecycle", "home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "relay-target-freshness"]),
        scenario("lease-reconnect", "relay-target-freshness", ["lease-health", "relay-target-freshness"]),
        scenario("provider-run-binding", "worker-execution", ["lease-health", "provider-run-lifecycle"]),
        scenario("remote-prompt-dispatch", "relay-runtime", ["agent-lifecycle", "provider-run-lifecycle"]),
        scenario("single-user-remote-agent", "ui-client-projection", ["agent-lifecycle", "client-projection-health", "runtime-projection-health", "session-authority"]),
      ],
    }),
  })
  await writeFakeMatrixScript({
    file: path.join(ossRoot, "apps", "cli", "scripts", "live-remote-home-extension-matrix-drill.mjs"),
    report: matrixReport({
      matrix: "remote-home-extension-matrix",
      metadata: {
        deploymentPresets: "hetzner,local,self-hosted-relay",
      },
      scenarios: [
        scenario("local-single", "remote-extension-sync", ["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"]),
        scenario("local-collab", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
        scenario("hetzner-single", "worker-execution", ["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"]),
        scenario("hetzner-collab", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
      ],
    }),
  })
  await writeFakeMatrixScript({
    file: path.join(ossRoot, "apps", "cli", "scripts", "live-runtime-resilience-chaos-matrix-drill.mjs"),
    report: matrixReport({
      matrix: "runtime-resilience-chaos-matrix",
      metadata: {
        deploymentPresets: "hetzner,hosted-cloud,local,same-host-remote,self-hosted-relay",
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("local-kernel-websocket-drop", "relay-runtime", ["client-projection-health", "relay-target-freshness"]),
        scenario("local-kernel-restart-durable-state", "kernel-authority", ["provider-run-lifecycle", "runtime-transition-audit", "session-authority"]),
        scenario("local-relay-restart-reconnect", "relay-target-freshness", ["provider-run-lifecycle", "relay-target-freshness", "session-authority"]),
        scenario("local-tui-web-terminal-parity", "ui-client-projection", ["client-projection-health", "runtime-projection-health", "session-authority"]),
        scenario("same-host-remote-worker-restart", "relay-target-freshness", ["lease-health", "relay-target-freshness", "session-authority"]),
        scenario("worker-provider-resume-codex", "provider-error", ["lease-health", "provider-run-lifecycle", "session-authority"]),
        scenario("worker-provider-resume-opencode", "provider-error", ["lease-health", "provider-run-lifecycle", "session-authority"]),
        scenario("hetzner-collaborator-reconnect-authority", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
        scenario("hosted-cloud-relay-second-kernel-reconnect", "relay-runtime", ["agent-lifecycle", "lease-health", "provider-run-lifecycle", "relay-target-freshness"]),
      ],
    }),
  })
  await writeFakeMatrixScript({
    file: path.join(ossRoot, "apps", "cli", "scripts", "live-slice-runtime-matrix-drill.mjs"),
    report: matrixReport({
      matrix: "slice-runtime-matrix",
      metadata: {
        deploymentPresets: "local,self-hosted-relay",
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("agent-reuse", "worker-execution", ["agent-lifecycle", "slice-runtime-state"]),
        scenario("docker-browser-state", "docker-runtime", ["slice-runtime-state"]),
        scenario("provider-auth", "slice-auth", ["provider-run-lifecycle", "slice-auth-state"]),
        scenario("session-start", "kernel-authority", ["session-authority", "slice-runtime-state"]),
        scenario("slice-lifecycle", "slice-runtime", ["slice-runtime-state"]),
      ],
    }),
  })
  await writeFakeMatrixScript({
    file: path.join(ossRoot, "apps", "cli", "scripts", "live-workspace-live-sync-matrix-drill.mjs"),
    report: matrixReport({
      matrix: "workspace-live-sync-matrix",
      metadata: {
        deploymentPresets: "hetzner,local,same-host-remote,self-hosted-relay",
        providers: "codex,opencode",
      },
      scenarios: [
        scenario("local-managed-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
        scenario("local-tracked-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
        scenario("local-permission-codex", "kernel-authority", ["session-authority", "workspace-live-sync-state"]),
        scenario("remote-managed-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
        scenario("remote-tracked-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
        scenario("remote-tracked-restart-codex", "relay-target-freshness", ["relay-target-freshness", "session-authority", "workspace-live-sync-state"]),
      ],
    }),
  })
  await writeFakeMatrixScript({
    file: path.join(cloudRoot, "scripts", "browser-terminal-resilience-matrix.mjs"),
    report: matrixReport({
      matrix: "browser-terminal-resilience-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud,local",
        providerCount: 3,
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("local-browser-relay-kernel-reconnect", "relay-runtime", ["client-projection-health", "provider-run-lifecycle", "relay-target-freshness", "session-authority"], { providers: ["claude", "codex", "opencode"] }),
        scenario("hosted-browser-relay-kernel-reconnect", "cloud-runtime", ["client-projection-health", "provider-run-lifecycle", "relay-target-freshness", "session-authority"], { providers: ["claude", "codex", "opencode"] }),
      ],
    }),
  })
  await writeFakeMatrixScript({
    file: path.join(cloudRoot, "scripts", "staging-slice-runtime-matrix.mjs"),
    report: matrixReport({
      matrix: "cloud-slice-runtime-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud",
        providerCount: 3,
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("ui-projection", "ui-client-projection", ["client-projection-health", "runtime-projection-health"], { providers: ["claude", "codex", "opencode"] }),
      ],
    }),
  })
}

export function matrixReport({ matrix, metadata, scenarios }) {
  return {
    schema: "arroba.drill.matrix.v1",
    matrix,
    status: "passed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata,
    scenarios,
  }
}

export async function writeFakeMatrixScript({ file, report }) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `#!/usr/bin/env node
import { createHash } from "node:crypto"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import path from "node:path"

const args = process.argv.slice(2)
const reportPath = valueFor("--report")
const artifactIndexPath = valueFor("--artifact-index") ?? valueFor("--output-artifact-index")
await mkdir(path.dirname(reportPath), { recursive: true })
const baseReport = ${JSON.stringify(report, null, 2)}
const providerAccountAliases = providerAccountAliasesFor(args)
const reportWithMetadata = providerAccountAliases.length > 0
  ? {
    ...baseReport,
    metadata: {
      ...baseReport.metadata,
      providerAccountAliases: providerAccountAliases.join(","),
    },
  }
  : baseReport
const report = args.includes("--dry-run") ? dryRunReportFor(reportWithMetadata) : reportWithMetadata
await writeFile(reportPath, \`\${JSON.stringify(report, null, 2)}\\n\`, "utf8")
if (artifactIndexPath) {
  const bytes = await readFile(reportPath)
  const repo = repoForMatrix(report.matrix)
  const index = {
    schema: "arroba.drill.artifact_index.v1",
    rootDir: path.dirname(reportPath),
    createdAt: "2026-06-13T00:00:02.000Z",
    metadata: {
      drill: report.matrix,
      matrix: report.matrix,
      artifactKinds: "matrix-report",
      generatedMatrixNames: report.matrix,
      generatedMatrixRepos: repo,
      generatedEvidenceRepos: repo,
      ...(providerAccountAliases.length > 0 ? { providerAccountAliases: providerAccountAliases.join(",") } : {}),
    },
    artifacts: [{
      path: path.basename(reportPath),
      schema: report.schema,
      sha256: createHash("sha256").update(bytes).digest("hex"),
      sizeBytes: bytes.byteLength,
    }],
  }
  await writeFile(artifactIndexPath, \`\${JSON.stringify(index, null, 2)}\\n\`, "utf8")
}

function repoForMatrix(matrix) {
  return matrix === "browser-terminal-resilience-matrix" || matrix === "cloud-slice-runtime-matrix"
    ? "cloud"
    : "oss"
}

function valueFor(flag) {
  const index = args.indexOf(flag)
  if (index < 0 || !args[index + 1]) return null
  return args[index + 1]
}

function providerAccountAliasesFor(args) {
  const aliases = []
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === "--provider-account" && args[index + 1]) {
      aliases.push(args[index + 1])
      index += 1
    } else if (args[index].startsWith("--provider-account=")) {
      aliases.push(args[index].slice("--provider-account=".length))
    }
  }
  return aliases.sort()
}

function dryRunReportFor(report) {
  return {
    ...report,
    status: "dry-run",
    dryRun: true,
    completedAt: report.startedAt,
    durationMs: 0,
    scenarios: report.scenarios.map((scenario) => ({
      ...scenario,
      status: "dry-run",
      ok: true,
      classification: null,
      durationMs: 0,
      reason: null,
    })),
  }
}
`, "utf8")
}

export async function writeFakeValidationSuiteScript({
  classification,
  evidenceRepo,
  file,
}) {
  const validationPresets = evidenceRepo === "oss" ? "distributed-runtime,distributed-state-health,runtime-authority" : "cloud-distributed-runtime"
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `#!/usr/bin/env node
import { createHash } from "node:crypto"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import path from "node:path"

const args = process.argv.slice(2)
const outputPath = valueFor("--output")
const artifactIndexPath = valueFor("--output-artifact-index")
const outputDir = path.dirname(outputPath)
await mkdir(outputDir, { recursive: true })
const report = {
  schema: "arroba.drill.validation_suite_run.v1",
  status: "passed",
  ok: true,
  startedAt: "2026-06-13T00:00:00.000Z",
  completedAt: "2026-06-13T00:00:01.000Z",
  durationMs: 1000,
  exitCode: 0,
  signal: null,
  error: null,
  command: "node --test fake-validation-suite.test.mjs",
  testCount: 1,
  testPaths: ["fake-validation-suite.test.mjs"],
  manifest: {
    schema: "arroba.drill.validation_suite.v1",
    testCount: 1,
    command: "node --test fake-validation-suite.test.mjs",
    coverage: [
      {
        id: "distributed-observability",
        description: "Distributed observability evidence.",
        testCount: 1,
        testPaths: ["fake-validation-suite.test.mjs"],
      },
      {
        id: "suite-contract",
        description: "Suite contract evidence.",
        testCount: 1,
        testPaths: ["fake-validation-suite.test.mjs"],
      },
    ],
    failureTaxonomyManifest: ${JSON.stringify(drillFailureTaxonomyManifest())},
    runtimeAuthorityManifest: ${JSON.stringify(drillRuntimeAuthorityManifest())},
    runtimeSignalsManifest: ${JSON.stringify(drillRuntimeSignalsManifest())},
    testPaths: ["fake-validation-suite.test.mjs"],
  },
}
await writeFile(outputPath, \`\${JSON.stringify(report, null, 2)}\\n\`, "utf8")
const bytes = await readFile(outputPath)
const index = {
  schema: "arroba.drill.artifact_index.v1",
  rootDir: outputDir,
  createdAt: "2026-06-13T00:00:02.000Z",
  metadata: {
    drill: "validation-suite",
    tests: 1,
    coverageAreas: "distributed-observability,suite-contract",
    runtimeAuthorityInvariants: ${JSON.stringify(DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS.join(","))},
    runtimeSignals: ${JSON.stringify(DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS.join(","))},
    runtimeSignalOwners: "kernel-authority,provider-account,provider-runtime,runtime-network,runtime-state,ui-client,worker-kernel",
    owners: "validation-platform",
    classifications: ${JSON.stringify(classification)},
    requiredFailureClassifications: ${JSON.stringify(DISTRIBUTED_RUNTIME_REQUIRED_FAILURE_CLASSIFICATIONS.join(","))},
    artifactKinds: "validation-suite-run",
    evidenceRepos: ${JSON.stringify(evidenceRepo)},
    generatedMatrixNames: ${JSON.stringify(generatedMatrixNamesForEvidenceRepo(evidenceRepo).join(","))},
    generatedMatrixRepos: ${JSON.stringify(evidenceRepo)},
    generatedEvidenceRepos: ${JSON.stringify(evidenceRepo)},
    validationPresets: ${JSON.stringify(validationPresets)},
    exitCriterionStatuses: "satisfied",
  },
  artifacts: [{
    path: path.basename(outputPath),
    schema: report.schema,
    sha256: createHash("sha256").update(bytes).digest("hex"),
    sizeBytes: bytes.byteLength,
  }],
}
await writeFile(artifactIndexPath, \`\${JSON.stringify(index, null, 2)}\\n\`, "utf8")

function valueFor(flag) {
  const index = args.indexOf(flag)
  if (index < 0 || !args[index + 1]) throw new Error(\`\${flag} requires a value\`)
  return args[index + 1]
}
`, "utf8")
}

export function scenario(id, classification, runtimeSignals = [], overrides = {}) {
  return {
    id,
    description: `${id} scenario`,
    status: "passed",
    ok: true,
    expectedFailure: false,
    classification,
    durationMs: 10,
    reason: null,
    requires: [],
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
    exitCriteria: [`${id} exit criteria`],
    runtimeSignals,
    ...overrides,
  }
}

export {
  assert,
  execFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
  os,
  path,
  test,
  verifyDrillArtifactIndex,
  writeDrillArtifactIndex,
  DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES_BY_REPO,
  drillFailureTaxonomyManifest,
  DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS,
  drillRuntimeAuthorityManifest,
  drillRuntimeSignalOwnersFor,
  drillRuntimeSignalsManifest,
  scriptPath,
  summaryScriptPath,
}
