import assert from "node:assert/strict"
import test from "node:test"
import type { RuntimeSession } from "@chariox/kernel-client/kernel-types"
import type { ManagedEnvironmentSummary } from "@chariox/kernel-client/ipc-managed-environment-requests"
import { prepareWaitingRoomEnrolledLaunch } from "./waiting-room-enrolled-launch.js"
import type { WaitingRoomLaunchConfig } from "./waiting-room-controller.js"

function harness(projectSelection?: WaitingRoomLaunchConfig["projectSelection"]) {
  const calls: string[] = []
  let active = true
  const assertActive = () => { if (!active) throw new Error("cancelled") }
  const launch: WaitingRoomLaunchConfig = {
    provider: "opencode", model: "user-model", effort: "high", accountProfile: "user-account",
    ownerMachineRef: "managed:environment:environment-1", ownerKernelRef: "",
    workerKernelRef: "selected-worker", sliceRef: "selected-slice", workspaceLiveSyncMode: "tracked",
    ...(projectSelection ? { projectSelection } : {}),
    managedEnvironment: { kind: "existing", environmentId: "environment-1" },
  }
  const prepared = prepareWaitingRoomEnrolledLaunch({
    launch,
    prepared: { kind: "enrolled", kernelId: "connected-kernel",
      environment: { runtimeMachineId: "enrolled-machine" } as ManagedEnvironmentSummary,
      commit: async () => { calls.push("commit") }, rollback: async () => { calls.push("rollback") } },
    assertActive,
    prepareProjectEnvironment: async (_session, check) => { check(); calls.push("common-setup") },
  })
  return { launch, prepared, calls, cancel: () => { active = false } }
}

test("MP-02/MP-08/MP-11 enrolled CLI launch preserves all ordinary configuration", async () => {
  const h = harness({ kind: "new" })
  const { managedEnvironment: _managed, ...ordinary } = h.launch
  assert.deepEqual(h.prepared.launch, { ...ordinary, ownerMachineRef: "enrolled-machine", ownerKernelRef: "connected-kernel" })
  await h.prepared.prepareProject({ project_id: "created-project" } as RuntimeSession)
  await h.prepared.commit()
  assert.deepEqual(h.calls, ["common-setup", "commit"])
})

test("MP-02/MP-08/MP-11 enrolled CLI launch uses the selected Project binding", async () => {
  const h = harness({ kind: "existing", project_id: "selected-project" })
  await assert.rejects(h.prepared.prepareProject({ project_id: "original-transfer-project" } as RuntimeSession), /different Project/)
  assert.deepEqual(h.calls, [])
  await h.prepared.prepareProject({ project_id: "selected-project" } as RuntimeSession)
  assert.deepEqual(h.calls, ["common-setup"])
})

test("MP-08/MP-11 enrolled CLI launch keeps cancellation and rollback ownership", async () => {
  const h = harness({ kind: "new" })
  h.cancel()
  await assert.rejects(h.prepared.prepareProject({} as RuntimeSession), /cancelled/)
  await h.prepared.rollback()
  assert.deepEqual(h.calls, ["rollback"])
})

test("MP-08/MP-11 enrolled CLI launch without Project selection avoids transfer setup", async () => {
  const h = harness()
  await h.prepared.prepareProject({} as RuntimeSession)
  assert.deepEqual(h.calls, [])
})
