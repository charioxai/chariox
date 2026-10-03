// MP-08 / MP-10: Chromium's owned empty sandbox has no unaccounted traffic.
import assert from "node:assert/strict"
import test from "node:test"
import { networkNamespaceAttribution } from "./browser-computer-soak-runtime.mjs"

const emptyLoopback = `header\nheader\nlo: ${Array(16).fill(0).join(" ")}\n`
const inspect = (devices = emptyLoopback, foreign = false) => networkNamespaceAttribution(new Set([100, 102]), 32, {
  currentPid: 100,
  listProc: async () => foreign ? ["100", "102", "103"] : ["100", "102"],
  readNamespace: async candidate => candidate.includes("/100/") ? "net:[main]" : "net:[sandbox]",
  readNetworkDevices: async () => devices,
})

test("MP-08/MP-10 accepts an owned zero-traffic loopback sandbox with retained proof", async () => {
  const value = await inspect()
  assert.equal(value.exclusive, true)
  assert.deepEqual(value.isolatedOwnedNamespaces, [{ namespace: "net:[sandbox]", pids: [102], networkBytes: 0 }])
})

test("MP-08/MP-10 foreign sandbox members and any unaccounted interface or traffic stay RED", async () => {
  assert.equal((await inspect(emptyLoopback.replace("lo:", "eth0:"))).exclusive, false)
  assert.equal((await inspect(emptyLoopback.replace("lo: 0", "lo: 1"))).exclusive, false)
  const foreign = await inspect(emptyLoopback, true)
  assert.equal(foreign.exclusive, false)
  assert.deepEqual(foreign.foreignPids, [103])
})
