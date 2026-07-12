const DEFAULT_STARTED_AT = "2026-06-13T00:00:00.000Z"
const DEFAULT_COMPLETED_AT = "2026-06-13T00:00:01.000Z"

export function runtimeAuthorityMatrixReportFixtures() {
  return [
    {
      fileName: "native-provider-tui.json",
      report: focusedRuntimeMatrixReport({
        matrix: "native-provider-tui-matrix",
        deploymentPresets: ["hetzner", "local", "same-host-remote", "self-hosted-relay"],
        providers: ["claude", "codex", "opencode"],
        scenarios: [
          focusedRuntimeScenario("local-native-tui", "kernel-authority", ["provider-run-lifecycle", "session-authority"]),
          focusedRuntimeScenario("permission-visibility", "kernel-authority", ["permission-interaction", "session-authority"]),
          focusedRuntimeScenario("remote-native-tui", "relay-runtime", ["provider-run-lifecycle", "runtime-projection-health", "session-authority"]),
          focusedRuntimeScenario("slice-native-tui", "worker-execution", ["agent-lifecycle", "lease-health", "provider-run-lifecycle"]),
          focusedRuntimeScenario("transcript-parity", "ui-client-projection", ["client-projection-health", "runtime-projection-health"]),
        ],
      }),
    },
    {
      fileName: "remote-agent-runtime.json",
      report: focusedRuntimeMatrixReport({
        matrix: "remote-agent-runtime-matrix",
        deploymentPresets: ["hetzner", "hosted-cloud", "same-host-remote", "self-hosted-relay"],
        providers: ["claude", "codex", "opencode"],
        scenarios: [
          focusedRuntimeScenario("collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
          focusedRuntimeScenario("hetzner-collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
          focusedRuntimeScenario("hetzner-single-user-remote-agent", "worker-execution", ["agent-lifecycle", "lease-health", "provider-run-lifecycle"]),
          focusedRuntimeScenario("hosted-collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
          focusedRuntimeScenario("hosted-single-user-remote-agent", "relay-runtime", ["lease-health", "runtime-projection-health"]),
          focusedRuntimeScenario("lease-reconnect", "kernel-authority", ["lease-health", "session-authority"]),
          focusedRuntimeScenario("provider-run-binding", "provider-error", ["provider-run-lifecycle", "session-authority"]),
          focusedRuntimeScenario("remote-prompt-dispatch", "provider-auth", ["provider-run-lifecycle", "session-authority"]),
          focusedRuntimeScenario("single-user-remote-agent", "ui-client-projection", ["agent-lifecycle", "client-projection-health", "runtime-projection-health", "session-authority"]),
        ],
      }),
    },
    {
      fileName: "slice-runtime.json",
      report: focusedRuntimeMatrixReport({
        matrix: "slice-runtime-matrix",
        deploymentPresets: ["local", "self-hosted-relay"],
        providers: ["claude", "codex", "opencode"],
        scenarios: [
          focusedRuntimeScenario("agent-reuse", "kernel-authority", ["agent-lifecycle", "session-authority"]),
          focusedRuntimeScenario("provider-auth", "slice-auth", ["provider-run-lifecycle", "slice-auth-state"]),
          focusedRuntimeScenario("session-start", "kernel-authority", ["agent-lifecycle", "session-authority"]),
          focusedRuntimeScenario("slice-lifecycle", "slice-runtime", ["provider-run-lifecycle", "slice-runtime-state"]),
          focusedRuntimeScenario("ui-projection", "ui-client-projection", ["client-projection-health", "runtime-projection-health"]),
        ],
      }),
    },
    {
      fileName: "browser-terminal-resilience.json",
      report: focusedRuntimeMatrixReport({
        matrix: "browser-terminal-resilience-matrix",
        deploymentPresets: ["hosted-cloud", "local", "self-hosted-relay"],
        providers: ["claude", "codex", "opencode"],
        scenarios: [
          focusedRuntimeScenario("local-browser-relay-kernel-reconnect", "relay-target-freshness", ["relay-target-freshness", "runtime-projection-health"]),
        ],
      }),
    },
    {
      fileName: "cloud-slice-runtime.json",
      report: focusedRuntimeMatrixReport({
        matrix: "cloud-slice-runtime-matrix",
        deploymentPresets: ["hosted-cloud", "local", "self-hosted-relay"],
        providers: ["claude", "codex", "opencode"],
        scenarios: [
          focusedRuntimeScenario("provider-auth", "slice-auth", ["provider-run-lifecycle", "slice-auth-state"]),
          focusedRuntimeScenario("slice-lifecycle", "slice-runtime", ["runtime-projection-health", "slice-runtime-state"]),
        ],
      }),
    },
    {
      fileName: "remote-home-extension.json",
      report: focusedRuntimeMatrixReport({
        matrix: "remote-home-extension-matrix",
        deploymentPresets: ["hetzner", "local", "self-hosted-relay"],
        providers: ["codex"],
        scenarios: [
          focusedRuntimeScenario("hetzner-collab", "kernel-authority", ["home-extension-manifest-sync", "lease-health"]),
          focusedRuntimeScenario("hetzner-single", "worker-execution", ["home-extension-manifest-sync", "provider-run-lifecycle"]),
          focusedRuntimeScenario("local-collab", "remote-extension-sync", ["home-extension-manifest-sync", "lease-health"]),
          focusedRuntimeScenario("local-single", "kernel-authority", ["home-extension-manifest-sync", "provider-run-lifecycle"]),
        ],
      }),
    },
    {
      fileName: "runtime-resilience-chaos.json",
      report: focusedRuntimeMatrixReport({
        matrix: "runtime-resilience-chaos-matrix",
        deploymentPresets: ["hetzner", "hosted-cloud", "local", "same-host-remote", "self-hosted-relay"],
        providers: ["codex", "opencode"],
        scenarios: [
          focusedRuntimeScenario("local-kernel-restart-durable-state", "kernel-authority", ["runtime-projection-health", "session-authority"]),
        ],
      }),
    },
    {
      fileName: "workspace-live-sync.json",
      report: focusedRuntimeMatrixReport({
        matrix: "workspace-live-sync-matrix",
        deploymentPresets: ["hetzner", "local", "same-host-remote", "self-hosted-relay"],
        providers: ["codex", "opencode"],
        scenarios: [
          focusedRuntimeScenario("local-managed-codex", "workspace-live-sync-conflict", ["workspace-live-sync-state"]),
          focusedRuntimeScenario("local-tracked-codex", "kernel-authority", ["workspace-live-sync-state"]),
          focusedRuntimeScenario("remote-managed-codex", "workspace-live-sync-conflict", ["relay-target-freshness", "workspace-live-sync-state"]),
          focusedRuntimeScenario("remote-tracked-codex", "relay-target-freshness", ["relay-target-freshness", "workspace-live-sync-state"]),
          focusedRuntimeScenario("remote-tracked-restart-codex", "kernel-authority", ["relay-target-freshness", "workspace-live-sync-state"]),
        ],
      }),
    },
  ]
}

export function distributedStateHealthPartialMatrixReport() {
  return focusedRuntimeMatrixReport({
    matrix: "remote-agent-runtime-matrix",
    deploymentPresets: ["local"],
    providers: ["codex"],
    scenarios: [
      focusedRuntimeScenario("lease-reconnect", "kernel-authority", ["lease-health", "provider-run-lifecycle"]),
    ],
  })
}

function focusedRuntimeMatrixReport({
  matrix,
  deploymentPresets,
  providers,
  scenarios,
}) {
  return {
    schema: "arroba.drill.matrix.v1",
    matrix,
    status: "passed",
    dryRun: false,
    startedAt: DEFAULT_STARTED_AT,
    completedAt: DEFAULT_COMPLETED_AT,
    durationMs: 1000,
    metadata: {
      deploymentPresets: deploymentPresets.join(","),
      providers: providers.join(","),
    },
    scenarios,
  }
}

function focusedRuntimeScenario(id, classification, runtimeSignals) {
  return {
    id,
    description: `${id} scenario`,
    requires: [],
    exitCriteria: [],
    status: "passed",
    expectedFailure: false,
    classification,
    runtimeSignals,
    durationMs: 10,
    reason: null,
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
  }
}
