// MP-08 / MP-10: reuse UI inventories immediately, reconcile old copies privately.
import { readClientInventory, writeClientInventory, clientInventoryEpoch } from "./client-inventory-cache.js"
export function createComposerInventoryLoader<T extends object>(
  scope: () => Promise<{ key: string; owner: string }>,
  fetch: (cached?: T) => Promise<T>,
  selection: () => string = () => "",
): () => Promise<T & { reconciliation?: Promise<T> }> {
  let flight: Promise<T & { reconciliation?: Promise<T> }> | undefined
  let selected = selection()
  let observedEpoch = clientInventoryEpoch(), loadedAt = 0
  return () => {
    if (selected !== selection() || observedEpoch !== clientInventoryEpoch() || Date.now() - loadedAt >= 4 * 60 * 60 * 1000) {
      flight = undefined
      selected = selection()
      observedEpoch = clientInventoryEpoch()
      loadedAt = Date.now()
    }
    if (flight) return flight
    const generation = clientInventoryEpoch(), requestedSelection = selection()
    const task = (async () => {
      const { key, owner } = await scope()
      if (selection() !== requestedSelection) throw new Error("composer selection superseded")
      const stored = await readClientInventory<T>(key, owner)
      if (selection() !== requestedSelection) throw new Error("composer selection superseded")
      const refresh = async () => {
        const value = await fetch(stored?.value)
        if (generation === clientInventoryEpoch() && selection() === requestedSelection) void writeClientInventory({ key, owner, savedAt: Date.now(), value })
        return value
      }
      if (!stored) return refresh()
      const reconciliation = Date.now() - stored.savedAt >= 4 * 60 * 60 * 1000 ? refresh() : undefined
      if (reconciliation) void reconciliation.catch(() => {})
      return { ...stored.value, ...(reconciliation ? { reconciliation } : {}) }
    })()
    const current: Promise<T & { reconciliation?: Promise<T> }> = task.catch(error => {
      if (flight === current) flight = undefined
      throw error
    })
    flight = current
    return current
  }
}
