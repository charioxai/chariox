import assert from "node:assert/strict"
import test from "node:test"
import type { LocalIpcClient } from "./ipc.js"
import { acquireMachineAllocation, getMachineAllocation, machineAllocationCapability } from "./machine-allocation-api.js"

test("TUI acquisition and observation use only the kernel ControlPlane seam", async () => {
  const requests: unknown[] = []
  const machine = { kind: "machine", allocation_ref: "allocation-1", revision: 1, state: "provisioning", available_actions: [], access_state: "pending", access_reason: null, kernel_target: null }
  const capability = { revision: "offer-7", actions: ["acquire"], offer: null }
  const client = { async send(request: any) {
    requests.push(request)
    return { ControlPlane: { result: request.ControlPlane.action === "capabilities" ? { kind: "capabilities", machine_allocation: capability } : machine } }
  } } as unknown as LocalIpcClient
  assert.deepEqual(await machineAllocationCapability(client), capability)
  assert.deepEqual(await acquireMachineAllocation(client, "offer-7", "request-1"), machine)
  assert.deepEqual(await getMachineAllocation(client, "allocation-1"), machine)
  assert.deepEqual(requests, [
    { ControlPlane: { action: "capabilities" } },
    { ControlPlane: { action: "acquire", capability_revision: "offer-7", request_id: "request-1" } },
    { ControlPlane: { action: "get", allocation_ref: "allocation-1" } },
  ])
})
