import assert from "node:assert/strict"
import test from "node:test"

import {
  formatRemoteExtensionSyncStatusLine,
  remoteExtensionSyncNextAction,
} from "./shell-capability-format.js"

test("remote extension sync formatter renders status and recovery consistently", () => {
  assert.equal(
    remoteExtensionSyncNextAction({ state: "failed" }, "agent-1", "worker-1"),
    "run /extension sync-status agent-1; run /machine kernels worker-1; use /extension sync-retry agent-1 after worker connectivity is healthy",
  )
  assert.equal(
    remoteExtensionSyncNextAction({ state: "missing" }, "agent-1", "worker-1"),
    "run /extension sync-status agent-1; run /machine kernels worker-1; use /extension sync-retry agent-1 after worker connectivity is healthy",
  )
  assert.equal(
    formatRemoteExtensionSyncStatusLine({ state: "failed", manifest_hash: "abcdef1234567890", last_error: "worker offline" }, {
      includeHash: true,
      includeNext: true,
      agentRef: "agent-1",
      workerMachineId: "worker-1",
      errorPrefix: "error=",
    }),
    "failed, hash=abcdef123456, error=worker offline, next=run /extension sync-status agent-1; run /machine kernels worker-1; use /extension sync-retry agent-1 after worker connectivity is healthy",
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
