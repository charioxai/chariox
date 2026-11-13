import assert from "node:assert/strict"
import test from "node:test"
import type { MachineAllocation, MachineAllocationCapability } from "./machine-allocation-api.js"
import { WaitingRoomMachineAllocationController } from "./waiting-room-machine-allocation-controller.js"

const capability: MachineAllocationCapability = {
  revision: "offer-7", actions: ["acquire"], job_offers: [], job_default_class_id: null, offer: {
    machine_type: "General purpose", vcpu: 4, ram_gb: 8, disk_gb: 50, location: "Site A",
    terms: { amount: "1.00", currency: "EUR", period: "hour" },
  },
}
const preparing: MachineAllocation = { kind: "machine", allocation_ref: "allocation-1", revision: 1, state: "provisioning", available_actions: [], access_state: "pending", access_reason: null, kernel_target: null }
const ready: MachineAllocation = { ...preparing, revision: 2, state: "ready", access_state: "available", kernel_target: { machine_id: "machine-1", kernel_id: "kernel-1", relay_realm_id: "realm-1", key_thumbprint: "key-1" } }
const attempt = { assertActive() {}, allocated(_machine: MachineAllocation) {}, progress(_message: string) {} }

test("machine acquisition sends only the offered revision and request identity, then waits for admitted access", async () => {
  const acquisitions: unknown[] = [], gets: string[] = []
  const controller = new WaitingRoomMachineAllocationController({
    acquire: async (...input) => { acquisitions.push(input); return preparing },
    get: async ref => { gets.push(ref); return ready }, requestId: () => "request-1", now: () => 0, delay: async () => {},
  })
  assert.equal((await controller.acquire(capability, attempt)).kernel_target.kernel_id, "kernel-1")
  assert.deepEqual(acquisitions, [["offer-7", "request-1"]])
  assert.deepEqual(gets, ["allocation-1"])
})

test("a lost acquire response retries the same revision and request identity", async () => {
  const requests: unknown[] = []
  let first = true
  const controller = new WaitingRoomMachineAllocationController({
    acquire: async (...input) => { requests.push(input); if (first) { first = false; throw new Error("connection lost") } return ready },
    get: async () => ready, requestId: () => "request-1", now: () => 0, delay: async () => {},
  })
  await assert.rejects(controller.acquire(capability, attempt), /connection lost/)
  await controller.acquire({ ...capability, revision: "offer-8" }, attempt)
  assert.deepEqual(requests, [["offer-7", "request-1"], ["offer-7", "request-1"]])
})

test("cancelling after acquisition preserves that allocation for the retry", async () => {
  let acquisitions = 0
  const controller = new WaitingRoomMachineAllocationController({
    acquire: async () => { acquisitions++; return preparing }, get: async () => ready,
    requestId: () => "request-1", now: () => 0, delay: async () => {},
  })
  let assertions = 0
  await assert.rejects(controller.acquire(capability, { ...attempt, assertActive() { if (++assertions > 1) throw new Error("selection changed") } }), /selection changed/)
  await controller.acquire(capability, attempt)
  assert.equal(acquisitions, 1)
})

test("a ready allocation remains owned until the connection is committed", async () => {
  let acquisitions = 0, reads = 0
  const controller = new WaitingRoomMachineAllocationController({
    acquire: async () => { acquisitions++; return ready }, get: async () => { reads++; return ready },
    requestId: () => `request-${acquisitions + 1}`, now: () => 0, delay: async () => {},
  })
  await controller.acquire(capability, attempt)
  await controller.acquire(capability, attempt)
  assert.equal(acquisitions, 1)
  assert.equal(reads, 1)
  controller.complete(ready.allocation_ref)
  await controller.acquire(capability, attempt)
  assert.equal(acquisitions, 2)
})
