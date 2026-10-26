import assert from "node:assert/strict"
import test from "node:test"
import * as requests from "./ipc-disposable-worker-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("disposable controls bind the expected home and require protocol 367", () => {

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

// MP-08 / MP-10 / MP-11: contract-only 533 kernels reject Skills before dispatch.
test("Skills V2 controls require the connected runtime capability", async () => {
  const request = { SkillsV2: { action: "catalog", project_id: null, agent_id: null } }
  assert.equal(requests.isGuardedKernelControl(request), true)
  let queries = 0
  for (const capabilities of [[], ["skills_v2_contract"], ["skills_v2_runtime"]]) {
    const send = async (query: unknown) => {
      queries++
      assert.deepEqual(query, { RelayStatus: null })
      return { RelayStatus: { status: { daemon_id: "home", machine_id: "machine", capabilities } } }
    }
    if (capabilities.includes("skills_v2_runtime")) await requests.requireKernelControlCapability(send, request)
    else await assert.rejects(requests.requireKernelControlCapability(send, request), /skills_v2_runtime/)
  }
  assert.equal(queries, 3)
})

// MP-08 / MP-10 / MP-11: a 571 kernel advertises skills_v2_runtime but cannot decode the 581 repository update.
test("Skills repository update needs the 581 capability and tells the person to update the kernel", async () => {
  const update = { SkillsV2: { action: "resolve_repository_update", package_id: "package-1" } }
  const status = (capabilities: string[]) => async () => ({ RelayStatus: { status: { daemon_id: "home", machine_id: "machine", capabilities } } })
  await assert.rejects(
    requests.requireKernelControlCapability(status(["skills_v2_runtime"]), update),
    /Repository updates need a newer kernel: update the kernel, then run \/skills update-package again/,
  )
  await requests.requireKernelControlCapability(status(["skills_v2_runtime", "skills_repository_update_v581"]), update)
  // The other Skills requests keep working against a kernel without the update capability.
  await requests.requireKernelControlCapability(status(["skills_v2_runtime"]), { SkillsV2: { action: "catalog", project_id: null, agent_id: null } })
})
