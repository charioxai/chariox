import assert from "node:assert/strict"
import test from "node:test"

import {
  formatRemoteExtensionSyncStatus,
  formatRemoteExtensionSyncStatusLine,
  remoteExtensionSyncNextAction,
} from "./shell-capability-format.js"
import {
  homeExtensionAuditAgentRef,
  homeExtensionAuditRecoveryAction,
} from "./home-extension-audit-policy.js"

test("remote extension sync formatter renders status and recovery consistently", () => {
  assert.equal(
    remoteExtensionSyncNextAction({ state: "failed" }, "agent-1", "worker-1"),
    "home keeps stale home-proxy calls blocked; run /extension sync-status agent-1; run /machine kernels worker-1; use /extension sync-retry agent-1 after worker connectivity is healthy",
  )
  assert.equal(
    remoteExtensionSyncNextAction({ state: "missing" }, "agent-1", "worker-1"),
    "home keeps stale home-proxy calls blocked; run /extension sync-status agent-1; run /machine kernels worker-1; use /extension sync-retry agent-1 after worker connectivity is healthy",
  )
  assert.equal(
    remoteExtensionSyncNextAction({ state: "failed" }, "agent-1", null),
    "home keeps stale home-proxy calls blocked; run /extension sync-status agent-1; run /kernel remote-runtime to identify worker connectivity; use /extension sync-retry agent-1 after worker connectivity is healthy",
  )
  assert.equal(
    remoteExtensionSyncNextAction({ state: "synced", pending_revoke: true }, "agent-1", null),
    "keep the home revoke in place; run /extension sync-status agent-1; run /kernel remote-runtime to identify worker connectivity if the revoke stays pending; use /extension sync-retry agent-1 after the worker reconnects",
  )
  assert.equal(
    formatRemoteExtensionSyncStatusLine({ state: "failed", manifest_hash: "abcdef1234567890", last_error: "worker offline" }, {
      includeHash: true,
      includeNext: true,
      agentRef: "agent-1",
      workerMachineId: "worker-1",
      errorPrefix: "error=",
    }),
    "failed, hash=abcdef123456, error=worker offline, next=home keeps stale home-proxy calls blocked; run /extension sync-status agent-1; run /machine kernels worker-1; use /extension sync-retry agent-1 after worker connectivity is healthy",
  )
  assert.equal(
    formatRemoteExtensionSyncStatusLine({ state: "synced", manifest_hash: "abcdef1234567890" }, {
      includeHash: true,
      includeNext: true,
      agentRef: "agent-1",
    }),
    "synced, hash=abcdef123456",
  )
})

test("remote extension sync status keeps final revokes visible after grants are gone", () => {
  const output = formatRemoteExtensionSyncStatus({
    id: "agent-1",
    agent_ref: "agent-ref-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "default",
    worktree_id: "/repo",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 0,
    last_activity_at_ms: 0,
    extension_grants: [],
    remote_execution: {
      worker_kernel_id: "worker-1",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
    remote_extension_manifest_sync: {
      state: "failed",
      manifest_hash: "empty-hash",
      pending_revoke: true,
      last_error: "worker offline",
    },
  })

  assert.match(output, /agent-ref-1 remote extension sync: failed, pending revoke, worker offline/)
  assert.match(output, /grants: 0 grants \(final revoke pending\)/)
  assert.match(output, /revoke state: pending worker acknowledgement/)
  assert.match(output, /next: keep the home revoke in place; run \/extension sync-status agent-ref-1; run \/machine kernels machine-1 if the revoke stays pending; use \/extension sync-retry agent-ref-1 after the worker reconnects/)
})

test("home extension audit policy gives binding-specific recovery for worker denials", () => {
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.invoke.denied", {
    status: "denied",
    error: "worker mismatch",
    agent_ref: "agent-ref-1",
  }), "run /extension sync-status agent-ref-1; inspect /agent inspect agent-ref-1; retry only after the worker lease and provider run match the current home grant")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.invoke.denied", {
    status: "denied",
    error: "worker mismatch",
    agent_ref: "agent-ref-1",
    worker_machine_id: "machine-1",
  }), "run /extension sync-status agent-ref-1; run /machine kernels machine-1; inspect /agent inspect agent-ref-1; retry only after the worker lease and provider run match the current home grant")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.invoke.denied", {
    status: "denied",
    error: "worker mismatch",
  }), "identify the affected agent in /kernel remote-runtime or the home extension audit, then retry only after the worker lease and provider run match the current home grant")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.invoke.denied", {
    status: "denied",
    error: "worker mismatch",
    agent_ref: "<agent>",
  }), "identify the affected agent in /kernel remote-runtime or the home extension audit, then retry only after the worker lease and provider run match the current home grant")
})

test("home extension audit policy handles terminal invocation outcomes", () => {
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.invoke.replayed", {
    status: "replayed",
  }), "cached idempotent result was returned; no retry needed")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.manifest.failed", {
    agent_ref: "agent-ref-1",
  }), "home keeps stale home-proxy calls blocked; run /extension sync-status agent-ref-1; use /extension sync-retry agent-ref-1 after worker connectivity is healthy")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.manifest.failed", {
    agent_ref: "agent-ref-1",
    worker_machine_id: "machine-1",
    pending_revoke: true,
  }), "keep the home revoke in place; run /extension sync-status agent-ref-1; run /machine kernels machine-1 if the revoke stays pending; use /extension sync-retry agent-ref-1 after the worker reconnects")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.manifest.retry_scheduled", {
    pending_revoke: true,
  }), "keep the home revoke in place; identify the affected agent in /kernel remote-runtime, then retry manifest sync after the worker reconnects")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.manifest.synced", {
    revoke_acknowledged: true,
  }), "worker acknowledged the revoke; home will continue denying calls for the removed grant")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.invoke.timeout", {
    status: "timeout",
  }), "split the tool work or increase the home extension timeout before retrying")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.invoke.cancelled", {
    status: "cancelled",
  }), "retry only if the provider turn still needs this tool result")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.invoke.cancelled", {
    status: "not_in_flight",
  }), "no matching in-flight home extension call was found; if the provider turn still waits for this tool, refresh /kernel remote-runtime and retry cancellation from the current turn")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.invoke.failed", {
    status: "failed",
  }), "inspect the home-side tool configuration and logs, then retry")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.grant.created", {}), null)
})

test("home extension audit policy resolves stable agent references", () => {
  assert.equal(homeExtensionAuditAgentRef({ agent_ref: " agent-ref-1 " }), "agent-ref-1")
  assert.equal(homeExtensionAuditAgentRef({ agent_id: "agent-1" }), "agent-1")
  assert.equal(homeExtensionAuditAgentRef({ home_agent_id: "home-agent-1" }), "home-agent-1")
  assert.equal(homeExtensionAuditAgentRef({}), "affected agent")
  assert.equal(homeExtensionAuditAgentRef({ agent_ref: "<agent>" }), "affected agent")
})
