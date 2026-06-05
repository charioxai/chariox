import assert from "node:assert/strict"
import test from "node:test"

import {
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

test("home extension audit policy gives binding-specific recovery for worker denials", () => {
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.invoke.denied", {
    status: "denied",
    error: "worker mismatch",
    agent_ref: "agent-ref-1",
  }), "run /extension sync-status agent-ref-1; inspect /agent inspect agent-ref-1; retry only after the worker lease and provider run match the current home grant")
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
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.invoke.timeout", {
    status: "timeout",
  }), "split the tool work or increase the home extension timeout before retrying")
  assert.equal(homeExtensionAuditRecoveryAction("home_extension.invoke.cancelled", {
    status: "cancelled",
  }), "retry only if the provider turn still needs this tool result")
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
