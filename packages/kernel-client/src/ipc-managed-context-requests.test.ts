import assert from "node:assert/strict"
import test from "node:test"

import {
  getManagedContextLaunchTargetRequest,
  getManagedContextTransferStatusRequest,
  startManagedContextTransferRequest,
} from "./ipc-managed-context-requests.js"
import type { ManagedContextTransferTicket } from "./ipc-managed-environment-requests.js"

test("managed context transfer requests use the shared local daemon shape", () => {
  const ticket: ManagedContextTransferTicket = {
    environmentId: "environment-1",
    contextPlan: {
      schemaVersion: 1,
      contextId: "context-1",
      planDigest: "sha256:plan",
      source: {
        sourceTargetId: "source-target-1",
        relayRealmId: "realm-1",
        machineId: "source-machine",
        kernelId: "source-kernel",
        keyThumbprint: "sha256:source-key",
      },
      kernelContext: "source_kernel",
      developmentSetup: { kind: "empty" },
      providerAccounts: { kind: "none" },
      gitCredentials: { kind: "none" },
    },
    target: {
      relayRealmId: "realm-1",
      machineId: "target-machine",
      kernelId: "target-kernel",
      relayPublicKey: "target-public-key",
      keyThumbprint: "sha256:target-key",
    },
  }

  assert.deepEqual(startManagedContextTransferRequest(ticket), {
    StartManagedContextTransfer: { ticket },
  })
  assert.deepEqual(getManagedContextTransferStatusRequest("context-1"), {
    GetManagedContextTransferStatus: { contextId: "context-1" },
  })
  assert.deepEqual(getManagedContextLaunchTargetRequest("context-1", "sha256:plan"), {
    GetManagedContextLaunchTarget: { contextId: "context-1", planDigest: "sha256:plan" },
  })
})
