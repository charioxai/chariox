import assert from "node:assert/strict"
import test from "node:test"

import { LocalIpcClient } from "./ipc.js"
import type { LocalKernelPresence } from "./local-kernel-presence.js"
import { selectLocalKernelClient } from "./local-kernel-selection.js"

const presence: LocalKernelPresence = {
  kernelId: "home-1",
  kernelAlias: "home",
  machineId: "machine-1",
  host: "127.0.0.1",
  port: 43_118,
  heartbeatAtMs: 1,
}

function statusClient(answer: () => Promise<unknown>) {
  const client = new LocalIpcClient("/unused-kernel.sock")
  let closed = false
  client.send = (async () => answer()) as LocalIpcClient["send"]
  client.close = async () => { closed = true }
  return { client, closed: () => closed }
}

test("MP-08 selects the local endpoint only after the kernel proves its identity", async () => {
  const endpoints: string[] = []
  const local = statusClient(async () => ({ RelayStatus: { status: { daemon_id: "home-1" } } }))
  const selection = await selectLocalKernelClient(
    { kernelId: "home-1" },
    (endpoint) => {
      endpoints.push(endpoint)
      return local.client
    },
    () => [presence],
  )
  assert.equal(selection?.client, local.client)
  assert.equal(selection?.endpoint, "ws://127.0.0.1:43118/kernel")
  assert.deepEqual(endpoints, ["ws://127.0.0.1:43118/kernel"])
  assert.equal(local.closed(), false)
})

test("MP-08 alias-only targets never select a same-named kernel from another realm", async () => {
  let probes = 0
  const local = statusClient(async () => ({ RelayStatus: { status: { daemon_id: "home-1" } } }))
  assert.equal(await selectLocalKernelClient({ kernelAlias: "home" }, () => { probes++; return local.client }, () => [presence]), null)
  assert.equal(probes, 0)
})

test("MP-08 stale local presence falls back", async () => {
  const unreachable = statusClient(async () => {
    throw new Error("connect ECONNREFUSED")
  })
  assert.equal(await selectLocalKernelClient({ kernelId: "home-1" }, () => unreachable.client, () => [presence]), null)
  assert.equal(unreachable.closed(), true)
})

test("MP-08 a kernel without a local presence is never probed", async () => {
  let created = false
  const selection = await selectLocalKernelClient(
    { kernelId: "remote-kernel" },
    () => {
      created = true
      return new LocalIpcClient("/unused-kernel.sock")
    },
    () => [presence],
  )
  assert.equal(selection, null)
  assert.equal(created, false)
})
