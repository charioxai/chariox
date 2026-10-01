import { lstatSync, readdirSync, readlinkSync, realpathSync } from "node:fs"
import { join } from "node:path"
import { verifyHomeEntryMetadata } from "./protected-home-capture.mjs"
import { verifyHomeVolumeName } from "./protected-layout.mjs"

function refuse() { throw new Error("Slice save/backup is unavailable for this storage layout; existing saved state is preserved") }
export function requireSafeHomeVolume({volume, docker, volumeRoot = "/var/lib/chariox-docker/data/volumes", maxEntries = 100_000}) {
  verifyHomeVolumeName(volume)
  const result = docker(["volume", "inspect", volume])
  if (result.status !== 0) refuse()
  const records = JSON.parse(result.stdout)
  if (!Array.isArray(records) || records.length !== 1 || records[0].Name !== volume) refuse()
  const path = records[0].Mountpoint
  if (path !== join(volumeRoot, volume, "_data") || realpathSync(path) !== path) refuse()
  const root = lstatSync(path)
  if (!root.isDirectory() || root.isSymbolicLink()) refuse()
  let count = 0
  const deadline = Date.now() + 30_000
  const pending = [""]
  while (pending.length) {
    const directory = pending.pop()
    for (const name of readdirSync(join(path, directory))) {
      if (++count > maxEntries || Date.now() > deadline) refuse()
      const relative = directory ? `${directory}/${name}` : name
      const metadata = lstatSync(join(path, relative))
      const kind = metadata.isSymbolicLink() ? "l" : metadata.isDirectory() ? "d" : metadata.isFile() ? "f" : "unsupported"
      if (metadata.mode & 0o6000) refuse()
      const target = kind === "l" ? readlinkSync(join(path, relative)) : ""
      verifyHomeEntryMetadata(Buffer.from(`${relative}\0${kind}\0${target}\0`))
      if (kind === "d") pending.push(relative)
    }
  }
}
