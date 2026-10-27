import {
  constants,
  closeSync,
  fsyncSync,
  fstatSync,
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

const emptyState = () => ({
  schemaVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
  nextProjectId: SLICE_DISK_QUOTA_PROJECT_ID_MIN,
  reservations: {},
})

export function createFileSliceDiskQuotaStateStore(path = SLICE_DISK_QUOTA_STATE_PATH, { ownerUid = process.getuid?.() ?? 0 } = {}) {
  const statePath = resolve(path)
  const stateDirectory = dirname(statePath)

  function read({ allowMissing }) {
    let directory
    try {
      directory = lstatSync(stateDirectory)
    } catch (error) {
      if (error?.code === "ENOENT" && allowMissing) return undefined
      throw error
    }
    if (directory.isSymbolicLink() || !directory.isDirectory() || directory.uid !== ownerUid || (directory.mode & 0o077) !== 0) {
      fail("managed disk quota state directory is unsafe")
    }

    let fd
    try {
      fd = openSync(statePath, constants.O_RDONLY | constants.O_NOFOLLOW)
    } catch (error) {
      if (error?.code === "ENOENT" && allowMissing) return undefined
      throw error
    }
    try {
      const metadata = fstatSync(fd)
      if (!metadata.isFile() || metadata.nlink !== 1 || metadata.uid !== ownerUid || (metadata.mode & 0o077) !== 0) {
        fail("managed disk quota durable state file is unsafe")
      }
      return JSON.parse(readFileSync(fd, "utf8"))
    } finally {
      closeSync(fd)
    }
  }

  return {
    load() {
      return read({ allowMissing: true }) ?? emptyState()
    },
    loadRequired() {
      const state = read({ allowMissing: false })
      return validateSliceDiskQuotaState(state)
    },
    save(state) {
      mkdirSync(stateDirectory, { recursive: true, mode: 0o700 })
      const directory = lstatSync(stateDirectory)
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
      const directoryFd = openSync(stateDirectory, "r")
      try { fsyncSync(directoryFd) } finally { closeSync(directoryFd) }
    },
  }
}
