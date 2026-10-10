import assert from "node:assert/strict"
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { loadLocalKernelPresences, localKernelEndpoint } from "./local-kernel-presence.js"

test("local kernel presence exposes only fresh lease endpoints", (context) => {
  const directory = mkdtempSync(join(tmpdir(), "chariox-kernel-presence-"))
  context.after(() => rmSync(directory, { force: true, recursive: true }))
  mkdirSync(directory, { recursive: true })
  writeFileSync(join(directory, "kernel-a.json"), JSON.stringify({
    schema_version: 1,
    kernel_id: "kernel-a",
    kernel_alias: "Local A",
    machine_id: "machine-a",
    machine_alias: "Laptop",
    host: "127.0.0.1",
    port: 43_121,
    heartbeat_at_ms: 100_000,
    local_daemon_protocol_version: 388,
  }))
  writeFileSync(join(directory, "stale.json"), JSON.stringify({
    schema_version: 1,
    kernel_id: "stale",
    machine_id: "machine-a",
    host: "127.0.0.1",
    port: 43_122,
    heartbeat_at_ms: 1,
  }))

  const presences = loadLocalKernelPresences(directory, 100_500)

  assert.equal(presences.length, 1)
  assert.equal(presences[0]?.kernelId, "kernel-a")
  assert.equal(presences[0]?.protocolVersion, 388)
  assert.equal(localKernelEndpoint(presences[0]!), "ws://127.0.0.1:43121/kernel")
})

test("local kernel presence formats IPv6 endpoints", () => {
  assert.equal(localKernelEndpoint({
    kernelId: "kernel-a",
    machineId: "machine-a",
    host: "::1",
    port: 43_121,
    heartbeatAtMs: 1,
  }), "ws://[::1]:43121/kernel")
})

test("MP-08 local kernel presence resolves under CHARIOX_HOME like the kernel", (context) => {
  const home = mkdtempSync(join(tmpdir(), "chariox-kernel-presence-home-"))
  const previous = { charioxHome: process.env.CHARIOX_HOME, xdg: process.env.XDG_CONFIG_HOME, explicit: process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR }
  context.after(() => {
    for (const [key, value] of [["CHARIOX_HOME", previous.charioxHome], ["XDG_CONFIG_HOME", previous.xdg], ["CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR", previous.explicit]] as const) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
    rmSync(home, { force: true, recursive: true })
  })
  delete process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR
  process.env.CHARIOX_HOME = home
  process.env.XDG_CONFIG_HOME = join(home, "elsewhere")
  mkdirSync(join(home, "kernels", "active"), { recursive: true })
  writeFileSync(join(home, "kernels", "active", "kernel-h.json"), JSON.stringify({
    schema_version: 1,
    kernel_id: "kernel-h",
    machine_id: "machine-h",
    host: "127.0.0.1",
    port: 43_118,
    heartbeat_at_ms: Date.now(),
  }))

  assert.deepEqual(loadLocalKernelPresences().map((presence) => presence.kernelId), ["kernel-h"])
})
