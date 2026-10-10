import assert from "node:assert/strict"
import test from "node:test"
import * as requests from "./ipc-disposable-worker-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("disposable controls bind the expected home and require protocol 367", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 490)
  assert.equal(requests.disposableWorkerControlMinimumProtocolVersion, 367)
  assert.equal(requests.managedEnvironmentKeepRunningMinimumProtocolVersion, 367)
  const input = { allocationId: "allocation-1", homeKernelId: "home-1", homeRelayRealmId: "realm-1" }
  for (const [method, name] of [
    [requests.getDisposableWorkerRequest, "GetDisposableWorker"],
    [requests.releaseDisposableWorkerRequest, "ReleaseDisposableWorker"],
    [requests.keepDisposableWorkerRunningRequest, "KeepDisposableWorkerRunning"],
    [requests.prepareDisposableWorkerContextTransferRequest, "PrepareDisposableWorkerContextTransfer"],
  ] as const) assert.deepEqual(method({ ...input, ...{ accountId: "untrusted", actorUserId: "untrusted" } }), { [name]: input })
  assert.deepEqual(requests.keepManagedEnvironmentRunningRequest("environment-1"), { KeepManagedEnvironmentRunning: { environmentId: "environment-1" } })
})

test("create preserves idempotency and explicit context without forwarding authority", () => {
  const input = { clientRequestId: "stable-id", homeKernelId: "home-1", homeRelayRealmId: "realm-1",
    region: "fsn1", computeClass: "worker", architecture: "x86_64", maximumLifetimeSeconds: 14400,
    autoStopPolicy: { minimumRuntimeSeconds: 12600, idleDelaySeconds: null } }
  assert.deepEqual(requests.createDisposableWorkerRequest({ ...input, ...{ accountId: "untrusted" } }), { CreateDisposableWorker: input })
  assert.deepEqual(requests.createDisposableWorkerRequest(input), requests.createDisposableWorkerRequest(input))
  const contextPlan = { sourceTargetId: null, kernelContext: "empty" as const, developmentSetup: { kind: "empty" as const },
    providerAccounts: { kind: "none" as const }, gitCredentials: { kind: "none" as const } }
  assert.deepEqual(requests.createDisposableWorkerRequest({ ...input, contextPlan }).CreateDisposableWorker.contextPlan, contextPlan)
})
