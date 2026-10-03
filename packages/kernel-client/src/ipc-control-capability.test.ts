import assert from "node:assert/strict"
import test from "node:test"
import { requireKernelControlCapability } from "./ipc-disposable-worker-requests.js"

const request = { CreateDisposableWorker: { homeKernelId: "home-1" } }
const status = (capabilities?: unknown, daemon_id = "home-1") => ({
  RelayStatus: { status: { daemon_id, machine_id: "machine-1", capabilities } },
})

test("disposable controls reject unrelated 351 and 365 peers without capabilities", async () => {
  for (const protocol_version of [351, 365]) {
    await assert.rejects(requireKernelControlCapability(async () => ({
      ...status(), protocol_version,
    }), request), /does not support/)
  }
})

test("capability admission binds the connected home and expected machine", async () => {
  const capability = ["disposable_worker_control_v1"]
  await requireKernelControlCapability(async () => status(capability), request,
    { daemonId: "home-1", machineId: "machine-1" })
  await assert.rejects(requireKernelControlCapability(async () => status(capability, "other"), request), /another kernel/)
  await assert.rejects(requireKernelControlCapability(async () => status(capability), request,
    { daemonId: "home-1", machineId: "other" }), /another machine/)
})

test("keep-running requires its own marker and reads fresh status each time", async () => {
  let calls = 0
  const send = async (query: unknown) => {
    assert.deepEqual(query, { RelayStatus: null })
    return status(calls++ === 0 ? ["managed_environment_keep_running_v1"] : [])
  }
  const keep = { KeepManagedEnvironmentRunning: { environmentId: "environment-1" } }
  await requireKernelControlCapability(send, keep)
  await assert.rejects(requireKernelControlCapability(send, keep), /does not support/)
  assert.equal(calls, 2)
  await assert.rejects(requireKernelControlCapability(async () => status(["disposable_worker_control_v1"]), keep), /does not support/)
})

test("unrelated requests do not add a preflight; malformed status fails closed", async () => {
  await requireKernelControlCapability(async () => { throw new Error("unexpected query") }, { RelayStatus: null })
  for (const response of [null, {}, { Error: "offline" }, status("disposable_worker_control_v1"), status(["unknown"]), status(["disposable_worker_control_v1", null])]) {
    await assert.rejects(requireKernelControlCapability(async () => response, request))
  }
})
