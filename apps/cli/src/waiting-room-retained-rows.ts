export type WaitingRoomRowFreshness = "cached/refreshing" | "reconnecting"
const reconnectGraceMs = 30_000

export function createWaitingRoomRetainedRows<T extends object>(key: (row: T) => string, nowMs: () => number) {
  const rows = new Map<string, { row: T; missingSinceMs?: number; stale?: boolean; cached: boolean }>()
  return {
    clear() { rows.clear() },
    restore(cached: readonly T[]): Array<T & { displayFreshness?: WaitingRoomRowFreshness }> {
      rows.clear()
      for (const row of cached) rows.set(key(row), { row, cached: true })
      return cached.map(row => ({ ...row, displayFreshness: "cached/refreshing" }))
    },
    reconcile(live: readonly T[], { authoritative = true }: { authoritative?: boolean } = {}): Array<T & { displayFreshness?: WaitingRoomRowFreshness }> {
      const liveIds = new Set(live.map(key))
      for (const [id, item] of rows) {
        if (liveIds.has(id)) continue
        if (!authoritative) {
          // A failed read proves no deletion. Keep last known rows as stale
          // until a fresh authoritative list can reconcile them.
          item.stale = true
          continue
        }
        item.missingSinceMs ??= nowMs()
        if (item.cached || nowMs() - item.missingSinceMs >= reconnectGraceMs) rows.delete(id)
      }
      for (const row of live) rows.set(key(row), { row, cached: false })
      return [...rows.values()].map(item => item.cached
        ? { ...item.row, displayFreshness: "cached/refreshing" }
        : item.missingSinceMs === undefined && !item.stale
        ? item.row as T & { displayFreshness?: WaitingRoomRowFreshness } : { ...item.row, displayFreshness: "reconnecting" })
    },
  }
}
