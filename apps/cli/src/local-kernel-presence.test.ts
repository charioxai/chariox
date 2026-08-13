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
