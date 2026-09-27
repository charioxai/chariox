import {
  closeSync,
  fsyncSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  renameSync,
  writeFileSync,
} from "node:fs"
import { dirname, resolve } from "node:path"
import {
  SLICE_DISK_QUOTA_PROJECT_ID_MIN,
  SLICE_DISK_QUOTA_PROTOCOL_VERSION,
  SLICE_DISK_QUOTA_STATE_PATH,
  validateSliceDiskQuotaState,
} from "./slice-disk-quota-contract.mjs"

function fail(message) {
  throw new Error(message)
}

export function createFileSliceDiskQuotaStateStore(path = SLICE_DISK_QUOTA_STATE_PATH, { ownerUid = process.getuid?.() ?? 0 } = {}) {
  const statePath = resolve(path)
  return {
    load() {
      let metadata
      try {
        metadata = lstatSync(statePath)
      } catch (error) {
        if (error?.code === "ENOENT") return {
          schemaVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
          nextProjectId: SLICE_DISK_QUOTA_PROJECT_ID_MIN,
          reservations: {},
        }
        throw error
      }
      if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.nlink !== 1 || metadata.uid !== ownerUid || (metadata.mode & 0o077) !== 0) {
        fail("managed disk quota durable state file is unsafe")
      }
      return JSON.parse(readFileSync(statePath, "utf8"))
    },
    save(state) {
      mkdirSync(dirname(statePath), { recursive: true, mode: 0o700 })
      const directory = lstatSync(dirname(statePath))
      if (directory.isSymbolicLink() || !directory.isDirectory() || directory.uid !== ownerUid || (directory.mode & 0o077) !== 0) {
        fail("managed disk quota state directory is unsafe")
      }
      const temporary = `${statePath}.tmp-${process.pid}`
      const fd = openSync(temporary, "wx", 0o600)
      try {
        writeFileSync(fd, `${JSON.stringify(state)}\n`)
        fsyncSync(fd)
      } finally {
        closeSync(fd)
      }
      renameSync(temporary, statePath)
      const directoryFd = openSync(dirname(statePath), "r")
      try { fsyncSync(directoryFd) } finally { closeSync(directoryFd) }
    },
  }
}
