import assert from "node:assert/strict"
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type { WaitingRoomInventory } from "./waiting-room-inventory-api.js"
import { createWaitingRoomInventoryCache } from "./waiting-room-inventory-cache.js"

test("waiting room inventory cache keeps one revisioned snapshot per kernel", () => {
  const directory = mkdtempSync(join(tmpdir(), "chariox-waiting-room-cache-"))
  try {
    let now = 1_000
    const cache = createWaitingRoomInventoryCache(directory, () => now)
    cache.persist(inventory("kernel-a", "structure-1", "activity-1"))
    cache.persist(inventory("kernel-a", "structure-1", "activity-1"))
    assert.deepEqual(readdirSync(directory), ["kernel-a.json"])

    now = 2_000
    cache.persist(inventory("kernel-a", "structure-2", "activity-2"))
    const persisted = JSON.parse(readFileSync(join(directory, "kernel-a.json"), "utf8"))
    assert.equal(persisted.savedAtMs, 2_000)
    assert.equal(persisted.inventory.activityRevision, "activity-2")

    const relaunched = createWaitingRoomInventoryCache(directory, () => now)
    assert.equal(relaunched.load()[0]?.kernelId, "kernel-a")
    assert.equal(relaunched.load()[0]?.sessions[0]?.kernel_id, "kernel-a")
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test("waiting room inventory cache bounds revision and activity timer state", () => {
  const directory = mkdtempSync(join(tmpdir(), "chariox-waiting-room-cache-bound-"))
  const scheduled: Array<{ callback: () => void; cancelled: boolean }> = []
  const timers = {
    setTimeout: ((callback: () => void) => {
      const pending = { callback, cancelled: false }
      scheduled.push(pending)
      return { unref() {} } as NodeJS.Timeout
    }) as typeof setTimeout,
    clearTimeout: (() => {
      const pending = scheduled.find((candidate) => !candidate.cancelled)
      if (pending) {
        pending.cancelled = true
      }
    }) as typeof clearTimeout,
  }
  try {
    let now = 1_000
    const cache = createWaitingRoomInventoryCache(directory, () => now, timers)
    cache.persist(inventory("kernel-0", "structure-1", "activity-1"))
    cache.persist(inventory("kernel-0", "structure-1", "activity-2"))

    for (let index = 1; index <= 64; index += 1) {
      now += 1
      cache.persist(inventory(`kernel-${index}`, "structure-1", "activity-1"))
    }

    assert.equal(scheduled[0]?.cancelled, true)
    assert.ok(readdirSync(directory).length <= 64)
    scheduled[0]?.callback()
    assert.ok(readdirSync(directory).length <= 64)

    now = 9_000
    cache.persist(inventory("kernel-0", "structure-1", "activity-1"))
    const revisited = JSON.parse(readFileSync(join(directory, "kernel-0.json"), "utf8"))
    assert.equal(revisited.savedAtMs, 9_000)
    assert.ok(readdirSync(directory).length <= 64)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

function inventory(
  kernelId: string,
  structuralVersion: string,
  activityRevision: string,
): WaitingRoomInventory {
  return {
    schemaVersion: 11,
    inventoryVersion: `${structuralVersion}:${activityRevision}`,
    structuralVersion,
    activityRevision,
    kernelId,
    machineId: "machine-a",
    sessions: [{
      id: "session-a",
      project_id: "project-default",
      kernel_id: kernelId,
      machine_id: "machine-a",
      workspace_id: "workspace-a",
      worktree_id: "worktree-a",
      created_at_ms: 10,
      status: "active",
      connected_cli_count: 0,
    }],
    relayStatus: {
      configured: true,
      connected: true,
      relay_token_configured: true,
      daemon_id: kernelId,
      machine_id: "machine-a",
    },
    remoteMachines: [],
    remoteKernels: [],
    terminals: [],
    slices: [],
  }
}
