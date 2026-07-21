import {
  chmodSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"

import type { WaitingRoomInventory } from "./waiting-room-inventory-api.js"

const cacheSchemaVersion = 1
const inventorySchemaVersion = 10
const cacheRetentionMs = 30 * 24 * 60 * 60 * 1_000
const maximumCachedKernels = 64
const activityPersistDebounceMs = 5_000

type CachedWaitingRoomInventory = {
  readonly cacheSchemaVersion: number
  readonly savedAtMs: number
  readonly inventory: WaitingRoomInventory
}

export type WaitingRoomInventoryCache = {
  load(): WaitingRoomInventory[]
  persist(inventory: WaitingRoomInventory): void
}

type WaitingRoomInventoryCacheTimers = {
  readonly setTimeout?: typeof setTimeout
  readonly clearTimeout?: typeof clearTimeout
}

export function createWaitingRoomInventoryCache(
  directory = defaultWaitingRoomInventoryCacheDir(),
  nowMs: () => number = Date.now,
  timers: WaitingRoomInventoryCacheTimers = {},
): WaitingRoomInventoryCache {
  const scheduleTimeout = timers.setTimeout ?? setTimeout
  const cancelTimeout = timers.clearTimeout ?? clearTimeout
  const versions = new Map<string, string>()
  const structuralVersions = new Map<string, string>()
  const pendingActivity = new Map<string, WaitingRoomInventory>()
  const activityTimers = new Map<string, NodeJS.Timeout>()
  const trackedKernelIds = new Set<string>()

  function rememberKernel(kernelId: string): void {
    trackedKernelIds.delete(kernelId)
    trackedKernelIds.add(kernelId)
    while (trackedKernelIds.size > maximumCachedKernels) {
      const oldestKernelId = trackedKernelIds.values().next().value
      if (!oldestKernelId) {
        break
      }
      trackedKernelIds.delete(oldestKernelId)
      versions.delete(oldestKernelId)
      structuralVersions.delete(oldestKernelId)
      pendingActivity.delete(oldestKernelId)
      const timer = activityTimers.get(oldestKernelId)
      if (timer) {
        cancelTimeout(timer)
        activityTimers.delete(oldestKernelId)
      }
    }
  }

  function load(): WaitingRoomInventory[] {
    const now = nowMs()
    const files = cacheFileRecords(directory)
    for (const file of files.slice(maximumCachedKernels)) {
      try {
        unlinkSync(file.path)
      } catch {
        // Best-effort bounded retention.
      }
    }
    const inventories = files.slice(0, maximumCachedKernels).reverse().flatMap(({ path }) => {
      try {
        const cached = JSON.parse(readFileSync(path, "utf8")) as CachedWaitingRoomInventory
        if (
          cached.cacheSchemaVersion !== cacheSchemaVersion
          || now - cached.savedAtMs > cacheRetentionMs
          || !validInventory(cached.inventory)
        ) {
          unlinkSync(path)
          return []
        }
        versions.set(cached.inventory.kernelId, versionKey(cached.inventory))
        structuralVersions.set(cached.inventory.kernelId, cached.inventory.structuralVersion)
        rememberKernel(cached.inventory.kernelId)
        return [cached.inventory]
      } catch {
        return []
      }
    })
    return inventories.sort((left, right) => newestSessionAt(right) - newestSessionAt(left))
  }

  function persist(inventory: WaitingRoomInventory): void {
    if (!validInventory(inventory)) {
      return
    }
    const nextVersion = versionKey(inventory)
    if (versions.get(inventory.kernelId) === nextVersion) {
      return
    }
    if (structuralVersions.get(inventory.kernelId) === inventory.structuralVersion) {
      pendingActivity.set(inventory.kernelId, inventory)
      if (!activityTimers.has(inventory.kernelId)) {
        const timer = scheduleTimeout(() => {
          activityTimers.delete(inventory.kernelId)
          const pending = pendingActivity.get(inventory.kernelId)
          pendingActivity.delete(inventory.kernelId)
          if (pending) {
            writeNow(pending)
          }
        }, activityPersistDebounceMs)
        timer.unref()
        activityTimers.set(inventory.kernelId, timer)
      }
      return
    }
    pendingActivity.delete(inventory.kernelId)
    const timer = activityTimers.get(inventory.kernelId)
    if (timer) {
      cancelTimeout(timer)
      activityTimers.delete(inventory.kernelId)
    }
    writeNow(inventory)
  }

  function writeNow(inventory: WaitingRoomInventory): void {
    try {
      mkdirSync(directory, { recursive: true, mode: 0o700 })
      chmodSync(directory, 0o700)
      const path = join(directory, `${safeKernelId(inventory.kernelId)}.json`)
      const temporaryPath = `${path}.${process.pid}.tmp`
      writeFileSync(temporaryPath, JSON.stringify({
        cacheSchemaVersion,
        savedAtMs: nowMs(),
        inventory,
      } satisfies CachedWaitingRoomInventory), { mode: 0o600 })
      chmodSync(temporaryPath, 0o600)
      renameSync(temporaryPath, path)
      versions.set(inventory.kernelId, versionKey(inventory))
      structuralVersions.set(inventory.kernelId, inventory.structuralVersion)
      rememberKernel(inventory.kernelId)
      pruneCache(directory)
    } catch {
      // Cache persistence must never prevent the TUI from reaching a kernel.
    }
  }

  return { load, persist }
}

function validInventory(inventory: WaitingRoomInventory | null | undefined): inventory is WaitingRoomInventory {
  return inventory?.schemaVersion === inventorySchemaVersion
    && Boolean(inventory.kernelId?.trim())
    && Boolean(inventory.machineId?.trim())
    && Boolean(inventory.structuralVersion?.trim())
    && Boolean(inventory.activityRevision?.trim())
    && Array.isArray(inventory.sessions)
}

function versionKey(inventory: WaitingRoomInventory): string {
  return `${inventory.structuralVersion}:${inventory.activityRevision}`
}

function safeKernelId(kernelId: string): string {
  return kernelId.replace(/[^a-zA-Z0-9._-]/g, "_")
}

function newestSessionAt(inventory: WaitingRoomInventory): number {
  return Math.max(0, ...inventory.sessions.map((session) => session.last_used_at_ms ?? session.created_at_ms ?? 0))
}

function cacheFiles(directory: string): string[] {
  try {
    return readdirSync(directory, { withFileTypes: true })
      .filter((entry) => entry.isFile() && entry.name.endsWith(".json"))
      .map((entry) => join(directory, entry.name))
  } catch {
    return []
  }
}

function pruneCache(directory: string): void {
  const files = cacheFileRecords(directory)
  for (const file of files.slice(maximumCachedKernels)) {
    try {
      unlinkSync(file.path)
    } catch {
      // Best-effort bounded retention.
    }
  }
}

function cacheFileRecords(directory: string): Array<{ path: string; modifiedAtMs: number }> {
  return cacheFiles(directory)
    .flatMap((path) => {
      try {
        return [{ path, modifiedAtMs: statSync(path).mtimeMs }]
      } catch {
        return []
      }
    })
    .sort((left, right) => right.modifiedAtMs - left.modifiedAtMs)
}

function defaultWaitingRoomInventoryCacheDir(): string {
  const explicit = process.env.ARROBA_WAITING_ROOM_INVENTORY_CACHE_DIR?.trim()
  if (explicit) {
    return explicit
  }
  const xdgCacheHome = process.env.XDG_CACHE_HOME?.trim()
  return join(xdgCacheHome || join(homedir(), ".cache"), "arroba", "waiting-room", "kernels")
}
