// MP-08 / MP-10: private, bounded UI catalogs; never stores provider credentials.
import { readFile, mkdir, writeFile, rename, unlink } from "node:fs/promises"
import { dirname, join } from "node:path"
import { preferencesPath } from "./preferences.js"
const file = () => join(dirname(preferencesPath()), "client-cache", "inventories.json")
const maxBytes = 16 * 1024 * 1024
export type InventoryRecord<T> = { key: string; owner: string; savedAt: number; value: T }
export async function readClientInventory<T>(key: string, owner: string): Promise<InventoryRecord<T> | null> {
  try {
    const rows = JSON.parse(await readFile(file(), "utf8")) as InventoryRecord<T>[]
    if (!Array.isArray(rows)) return null
    if (rows.some(row => row.owner !== owner)) { await invalidateClientInventories(); return null }
    return rows.find(row => row.key === key && row.owner === owner) ?? null
  } catch { return null }
}
let writes: Promise<void> = Promise.resolve()
let epoch = 0
export const clientInventoryEpoch = () => epoch
export async function invalidateClientInventories(): Promise<void> {
  epoch++
  await writes.catch(() => {})
  await unlink(file()).catch(() => {})
}
export function writeClientInventory<T>(record: InventoryRecord<T>): Promise<void> {
  const generation = epoch
  writes = writes.catch(() => {}).then(async () => {
    if (Buffer.byteLength(JSON.stringify(record)) > maxBytes || generation !== epoch) return
    let rows: InventoryRecord<unknown>[] = []
    try { const old = JSON.parse(await readFile(file(), "utf8")); if (Array.isArray(old)) rows = old } catch {}
    rows = rows.filter(row => row.key !== record.key && row.owner === record.owner).sort((a, b) => a.savedAt - b.savedAt)
    rows.push(record)
    while (rows.length > 32 || Buffer.byteLength(JSON.stringify(rows)) > maxBytes) rows.shift()
    const destination = file(), temporary = destination + "." + process.pid + ".tmp"
    await mkdir(dirname(destination), { recursive: true, mode: 0o700 })
    await writeFile(temporary, JSON.stringify(rows), { mode: 0o600 })
    if (generation !== epoch) { await unlink(temporary).catch(() => {}); return }
    await rename(temporary, destination)
  }).catch(() => {})
  return writes
}
