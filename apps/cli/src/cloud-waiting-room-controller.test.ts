import assert from "node:assert/strict"
import test from "node:test"
import type { CloudClient } from "./cloud-client.js"
import { createCloudWaitingRoomController } from "./cloud-waiting-room-controller.js"

test("detached waiting-room discovery is single-flight and cannot overwrite a completed kernel pivot", async () => {
  let release!: () => void, connected = false, calls = 0, writes = 0
  const pending = new Promise<void>(resolve => { release = resolve })
  const client = { profile: async () => ({accountId: "account"}), directory: async () => { calls++; await pending; return [{daemonId: "kernel", machineId: "machine", status: "ONLINE"}] } } as unknown as CloudClient
  const refresh = createCloudWaitingRoomController({client, isKernelConnected: () => connected, setMachines: () => {writes++}, setKernels: () => {writes++}, setStatus: () => {}, reconcile: () => {writes++}})
  const a = refresh(), b = refresh()
  await Promise.resolve(); await Promise.resolve()
  assert.equal(calls, 1)
  connected = true; release(); await Promise.all([a,b])
  assert.equal(writes, 0)
})

test("signed-out detached waiting room clears only directory data and never creates or logs in a kernel", async () => {
  let machines: unknown[] = ["stale"], kernels: unknown[] = ["stale"], status = ""
  const client = { profile: async () => null, directory: async () => {throw new Error("signed-out discovery cannot authenticate")} } as unknown as CloudClient
  const refresh = createCloudWaitingRoomController({client, isKernelConnected: () => false, setMachines: rows => {machines = rows}, setKernels: rows => {kernels = rows}, setStatus: value => {status = value}, reconcile: () => {}})
  await refresh()
  assert.equal(machines.length, 0); assert.equal(kernels.length, 0); assert.equal(status, "ready")
})


test("an absent profile resolving after attachment cannot clear the kernel directory", async () => {
  let release!: () => void, connected = false, writes = 0
  const pending = new Promise<void>(resolve => {release = resolve})
  const client = {profile: async () => {await pending; return null}} as unknown as CloudClient
  const refresh = createCloudWaitingRoomController({client, isKernelConnected: () => connected, setMachines: () => {writes++}, setKernels: () => {writes++}, setStatus: () => {writes++}, reconcile: () => {writes++}})
  const result = refresh(); connected = true; release(); await result
  assert.equal(writes, 0)
})
