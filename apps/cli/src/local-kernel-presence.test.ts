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

test("MP-08/MP-10 VERSION(P01) finds the protocol lease under explicit CHARIOX_HOME", context => {
  const root = mkdtempSync(join(tmpdir(), "envp01-protocol-home-"))
  const keys = ["CHARIOX_HOME", "CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR", "XDG_CONFIG_HOME"] as const
  const previous = Object.fromEntries(keys.map(key => [key, process.env[key]]))
  context.after(() => {
    for (const key of keys) {
      if (previous[key] === undefined) delete process.env[key]
      else process.env[key] = previous[key]
    }
    rmSync(root, { recursive: true, force: true })
  })
  process.env.CHARIOX_HOME = root
  process.env.XDG_CONFIG_HOME = join(root, "other-config")
  delete process.env.CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR
  const directory = join(root, "kernels", "active")
  mkdirSync(directory, { recursive: true })
  writeFileSync(join(directory, "kernel.json"), JSON.stringify({ schema_version: 1, kernel_id: "old-kernel", machine_id: "machine", host: "127.0.0.1", port: 43121, heartbeat_at_ms: 100000, local_daemon_protocol_version: 435 }))
  assert.equal(loadLocalKernelPresences(undefined, 100001)[0]?.protocolVersion, 435)
})
