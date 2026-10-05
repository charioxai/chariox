// MP-11: parse review-touched drill entries without starting any drill/provider.
import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { fileURLToPath } from "node:url"
import test from "node:test"

const files = [
  "apps/cli/scripts/lib/drill-matrix-report-aggregate.mjs",
  "apps/cli/scripts/lib/drill-matrix-report-shared.mjs",
  "apps/cli/scripts/lib/drill-matrix-report.mjs",
  "apps/cli/scripts/lib/drill-matrix-report.test/validation-and-discovery.test.mjs",
  "apps/cli/scripts/lib/hosted-cloud-kernel-scenarios.mjs",
  "apps/cli/scripts/lib/live-external-provider-live-parity-evidence.mjs",
  "apps/cli/scripts/lib/live-provider-thread-transfer-provider-state.mjs",
  "apps/cli/scripts/lib/live-provider-thread-transfer-runtime.mjs",
  "apps/cli/scripts/lib/live-provider-thread-transfer-runtime.test.mjs",
  "apps/cli/scripts/lib/native-tui-remote-execution.mjs",
  "apps/cli/scripts/lib/native-tui-remote-execution.test.mjs",
  "apps/cli/scripts/lib/private-drill-runtime.mjs",
  "apps/cli/scripts/lib/remote-home-extension-hetzner-helpers.mjs",
  "apps/cli/scripts/live-cloud-publication-deployment-drill.mjs",
  "apps/cli/scripts/live-connector-extension-agent-drill.mjs",
  "apps/cli/scripts/live-external-provider-session-import-drill.mjs",
  "apps/cli/scripts/live-multi-user-cli-workflow-drill.mjs",
  "apps/cli/scripts/live-provider-thread-transfer-drill.mjs",
  "apps/cli/scripts/live-relay-freeform-multi-user-drill.mjs",
  "apps/cli/scripts/live-remote-home-extension-drill.mjs",
  "apps/cli/scripts/live-remote-native-tui-drill.mjs",
  "apps/cli/scripts/live-remote-workspace-live-sync-drill.mjs",
  "apps/cli/scripts/live-remote-workspace-live-sync-permission-drill.mjs",
  "apps/cli/scripts/live-room-environment-pointer-click-drill.mjs",
  "apps/cli/scripts/live-script-extension-agent-drill.mjs",
  "apps/cli/scripts/live-unattached-agents-tui-parity-drill.mjs",
  "apps/cli/scripts/live-workspace-live-sync-permission-drill.mjs",
  "apps/cli/scripts/public-provider-run-protocol-drill.mjs",
  "apps/kernel/managed-upgrade-protocol-transitions.test.mjs",
  "apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe.mjs",
  "apps/kernel/slice-linux-docker/docker/public-runtime-diagnostics.mjs",
  "apps/kernel/slice-linux-docker/provision-command-guard.test.mjs",
  "apps/kernel/slice-linux-docker/provision-disk-quota.test.mjs",
  "apps/kernel/slice-linux-docker/provision-resource-limits.test.mjs",
  "apps/kernel/slice-linux-docker/provision-slice-disk-quota-xfs-backend.test.mjs",
  "experiments/mcp-isolation-spike/src/opencode-native-config-acceptance.mjs",
  "experiments/secret-handoff/src/secret-harness.mjs",
  "scripts/build-macos-pkg.mjs",
  "scripts/build-macos-pkg.test.mjs",
  "scripts/managed-kernel-broker-viewer.test.mjs",
  "scripts/managed-kernel-release.test.mjs",
  "scripts/managed-kernel-upgrade.test.mjs",
  "scripts/mp11-native-probe-diagnostics.test.mjs",
  "scripts/mp11-private-runtime.test.mjs",
  "scripts/mp11-profile-materialization.test.mjs",
  "scripts/mp11-provider-run-boundary.test.mjs",
  "scripts/mp11-provider-transcript-boundary.test.mjs",
  "scripts/mp11-public-runtime-diagnostics.test.mjs",
  "scripts/mp11-remote-environment.test.mjs",
  "scripts/mp11-script-entrypoints.test.mjs",
  "scripts/mp11-secret-prototype.test.mjs",
  "scripts/mp11-slice-auth-publication.test.mjs",
  "scripts/mp11-slice-setup-cleanup.test.mjs",
  "scripts/publication-credential-entrypoint.test.mjs",
  "scripts/release-keychain-state.mjs",
  "scripts/release-keychain-state.test.mjs",
  "scripts/slice-relay-identity-contract.test.mjs"
]
for (const file of files) {
  test(`MP-11 script entry parses: ${file}`, () => {
    const path = fileURLToPath(new URL(`../${file}`, import.meta.url))
    const result = spawnSync(process.execPath, ["--check", path], { encoding: "utf8" })
    assert.equal(result.status, 0, result.stderr)
  })
}
