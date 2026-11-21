import {
  chmodSync,
  closeSync,
  constants,
  fsyncSync,
  ftruncateSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readdirSync,
  renameSync,
  statfsSync,
  unlinkSync,
  writeFileSync,
} from "node:fs"
import { dirname } from "node:path"

import { DATA_ROOT, MAX_VOLUME_SIZE_GB, MIN_VOLUME_SIZE_GB } from "./slice-data-volume-device.mjs"
import { BACKING_FILE_PATH } from "./slice-data-volume-file.mjs"

export const PROTECTED_BOOTSTRAP_ROOT = "/etc/chariox/bootstrap"
export const MANAGED_BOOTSTRAP_PATH = `${PROTECTED_BOOTSTRAP_ROOT}/managed-bootstrap.json`
export const BINDING_PATH = "/var/lib/chariox-data-volume/volume-binding.json"
export const ADMISSION_OBSERVATION_PATH = "/run/chariox-data-volume-observation/observation.json"

const MAX_SAFE_INTEGER = Number.MAX_SAFE_INTEGER
const MAX_BOOTSTRAP_BYTES = 96 * 1024
const ROOT_UID = 0

function fail(message) {
  throw new Error(message)
}

function maybeLstat(path) {
  try {
    return lstatSync(path)
  } catch (error) {
    if (error.code === "ENOENT") return undefined
    throw error
  }
}

function parseJson(value, label) {
  try {
    return JSON.parse(value)
  } catch {
    fail(`${label} returned invalid JSON`)
  }
}

function charioxGroupId() {
  const row = readFileSync("/etc/group", "utf8")
    .split("\n")
    .find((line) => line.startsWith("chariox:"))
  const value = row?.split(":")[2]
  if (!/^(0|[1-9][0-9]*)$/.test(value ?? "")) fail("chariox group is unavailable")
  const gid = Number(value)
  if (!Number.isSafeInteger(gid)) fail("chariox group ID is invalid")
  return gid
}

function validateProtectedInputMetadata(path, expectedGid) {
  const root = lstatSync("/")
  const etc = lstatSync("/etc")
  const chariox = lstatSync("/etc/chariox")
  const parent = lstatSync(PROTECTED_BOOTSTRAP_ROOT)
  const file = lstatSync(path)
  const rootMode = root.mode & 0o7777
  const etcMode = etc.mode & 0o7777
  const charioxMode = chariox.mode & 0o7777
  if (
    root.isSymbolicLink() || !root.isDirectory() || root.uid !== ROOT_UID ||
    (rootMode & 0o022) !== 0 || (rootMode & 0o001) === 0 ||
    etc.isSymbolicLink() || !etc.isDirectory() || etc.uid !== ROOT_UID ||
    (etcMode & 0o022) !== 0 || (etcMode & 0o001) === 0 ||
    chariox.isSymbolicLink() || !chariox.isDirectory() || chariox.uid !== ROOT_UID ||
    (charioxMode & 0o022) !== 0 ||
    !(chariox.gid === expectedGid && (charioxMode & 0o010) !== 0 || (charioxMode & 0o001) !== 0) ||
    parent.isSymbolicLink() || !parent.isDirectory() || parent.uid !== ROOT_UID ||
    parent.gid !== expectedGid || (parent.mode & 0o7777) !== 0o750 ||
    file.isSymbolicLink() || !file.isFile() || file.uid !== ROOT_UID ||
    file.gid !== expectedGid || (file.mode & 0o7777) !== 0o640 ||
    file.size > MAX_BOOTSTRAP_BYTES
  ) {
    fail("protected bootstrap input must be root:chariox, mode 0640 under a root:chariox 0750 directory")
  }
}

function readProtectedInput(path, schemaVersion, expectedGid) {
  if (!maybeLstat(path)) return undefined
  validateProtectedInputMetadata(path, expectedGid)
  const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW)
  let text
  try {
    text = readFileSync(fd, "utf8")
  } finally {
    closeSync(fd)
  }
  let value
  try {
    value = JSON.parse(text)
  } catch {
    fail("protected bootstrap input is invalid JSON")
  }
  if (!value || typeof value !== "object" || Array.isArray(value) || value.schemaVersion !== schemaVersion) {
    fail("protected bootstrap input has an unsupported schema version")
  }
  return parseStorageInput(value.authorityData?.storage)
}

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

function keysOf(value) {
  return value && typeof value === "object" && !Array.isArray(value)
    ? Object.keys(value).sort().join(",")
    : ""
}

function validSerial(value) {
  return typeof value === "string" && value.length <= 16 && /^[1-9][0-9]*$/.test(value) &&
    BigInt(value) <= BigInt(MAX_SAFE_INTEGER)
}

function validSizeGb(value) {
  return Number.isSafeInteger(value) && value >= MIN_VOLUME_SIZE_GB && value <= MAX_VOLUME_SIZE_GB
}

function validBackingFile(value) {
  return keysOf(value) === "inode,majorMinor" &&
    typeof value.majorMinor === "string" && /^\d+:\d+$/.test(value.majorMinor) &&
    typeof value.inode === "string" && /^[1-9][0-9]*$/.test(value.inode)
}

// The storage a machine's managed bootstrap names: a provider volume found by its serial,
// or a file-backed volume on the root filesystem. Exactly one shape is accepted.
export function parseStorageInput(storage) {
  switch (keysOf(storage)) {
    case "expectedDataVolumeSerial,expectedDataVolumeSizeGb":
      if (validSerial(storage.expectedDataVolumeSerial) && validSizeGb(storage.expectedDataVolumeSizeGb)) {
        return { kind: "providerVolume", serial: storage.expectedDataVolumeSerial, sizeGb: storage.expectedDataVolumeSizeGb }
      }
      break
    case "fileDataVolumeSizeGb":
      if (validSizeGb(storage.fileDataVolumeSizeGb)) {
        return { kind: "fileVolume", sizeGb: storage.fileDataVolumeSizeGb }
      }
      break
  }
  fail("protected bootstrap data-volume identity or size is invalid")
}

// The binding keeps the identity admission formatted, so a later boot uses only that volume.
export function parseBinding(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail("data-volume binding is malformed")
  const common = value.schemaVersion === 1 && validSizeGb(value.expectedDataVolumeSizeGb) &&
    typeof value.filesystemUuid === "string" && UUID_PATTERN.test(value.filesystemUuid)
  const shape = { schemaVersion: 1, expectedDataVolumeSizeGb: value.expectedDataVolumeSizeGb, filesystemUuid: value.filesystemUuid }
  switch (keysOf(value)) {
    case "expectedDataVolumeSerial,expectedDataVolumeSizeGb,filesystemUuid,schemaVersion":
      if (!common || !validSerial(value.expectedDataVolumeSerial)) fail("data-volume binding fields are invalid")
      return { kind: "providerVolume", ...shape, expectedDataVolumeSerial: value.expectedDataVolumeSerial }
    case "backingFile,expectedDataVolumeSizeGb,filesystemUuid,schemaVersion":
      if (!common || !validBackingFile(value.backingFile)) fail("data-volume binding fields are invalid")
      return {
        kind: "fileVolume",
        ...shape,
        backingFile: { majorMinor: value.backingFile.majorMinor, inode: value.backingFile.inode },
      }
    default:
      fail("data-volume binding has an unsupported shape")
  }
}

function validateBindingDirectory() {
  const parent = dirname(BINDING_PATH)
  for (const path of ["/var", "/var/lib", parent]) {
    const metadata = lstatSync(path)
    const mode = metadata.mode & 0o7777
    if (
      metadata.isSymbolicLink() || !metadata.isDirectory() || metadata.uid !== ROOT_UID ||
      (mode & 0o022) !== 0 ||
      (path === parent && (metadata.gid !== 0 || mode !== 0o700))
    ) {
      fail("persistent data-volume state directory is not root-only")
    }
  }
}

function validateObservationDirectory() {
  const runtime = lstatSync("/run")
  if (
    runtime.isSymbolicLink() || !runtime.isDirectory() || runtime.uid !== ROOT_UID ||
    (runtime.mode & 0o7777 & 0o022) !== 0
  ) {
    fail("data-volume observation runtime directory is unsafe")
  }
  const directory = dirname(ADMISSION_OBSERVATION_PATH)
  let metadata = maybeLstat(directory)
  if (!metadata) {
    mkdirSync(directory, { mode: 0o755 })
    metadata = lstatSync(directory)
  }
  const mode = metadata.mode & 0o7777
  if (
    metadata.isSymbolicLink() || !metadata.isDirectory() || metadata.uid !== ROOT_UID ||
    (mode & 0o022) !== 0
  ) {
    fail("data-volume observation directory is unsafe")
  }
  if (mode !== 0o755) chmodSync(directory, 0o755)
}

function validateObservationFile(path) {
  const metadata = maybeLstat(path)
  if (!metadata) return undefined
  if (
    metadata.isSymbolicLink() || !metadata.isFile() || metadata.uid !== ROOT_UID ||
    metadata.gid !== 0 || (metadata.mode & 0o7777) !== 0o644 || metadata.size > 4096
  ) {
    fail("data-volume admission observation must be a root-owned read-only regular file")
  }
  return metadata
}

function validateObservation(value) {
  if (
    !value || typeof value !== "object" || Array.isArray(value) ||
    value.schemaVersion !== 1 ||
    (value.backingFile === undefined
      ? typeof value.dataVolumeSerial !== "string" || !/^[1-9][0-9]{0,15}$/.test(value.dataVolumeSerial)
      : value.dataVolumeSerial !== undefined || !validBackingFile(value.backingFile)) ||
    !Number.isSafeInteger(value.dataVolumeSizeGb) ||
    value.dataVolumeSizeGb < MIN_VOLUME_SIZE_GB || value.dataVolumeSizeGb > MAX_VOLUME_SIZE_GB ||
    typeof value.filesystemUuid !== "string" ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value.filesystemUuid) ||
    typeof value.devicePath !== "string" || !value.devicePath.startsWith("/dev/") ||
    typeof value.majorMinor !== "string" || !/^\d+:\d+$/.test(value.majorMinor) ||
    value.mountTarget !== DATA_ROOT
  ) {
    fail("data-volume admission observation fields are invalid")
  }
}

// The backing file's filesystem device as the kernel's major:minor, the pair losetup
// reports for it, and its inode.
function backingFileIdentity() {
  let metadata
  try {
    metadata = lstatSync(BACKING_FILE_PATH, { bigint: true })
  } catch (error) {
    if (error.code === "ENOENT") return undefined
    throw error
  }
  if (
    metadata.isSymbolicLink() || !metadata.isFile() || metadata.uid !== 0n || metadata.gid !== 0n ||
    (metadata.mode & 0o7777n) !== 0o600n || metadata.nlink !== 1n
  ) {
    fail("file-backed data volume must be a root-only regular file with one link")
  }
  const major = ((metadata.dev >> 8n) & 0xfffn) | ((metadata.dev >> 32n) & ~0xfffn)
  const minor = (metadata.dev & 0xffn) | ((metadata.dev >> 12n) & ~0xffn)
  return { majorMinor: `${major}:${minor}`, inode: String(metadata.ino), sizeBytes: Number(metadata.size) }
}

export const systemIo = {
  readStorageInput() {
    const gid = charioxGroupId()
    return readProtectedInput(MANAGED_BOOTSTRAP_PATH, 3, gid)
  },
  readBinding() {
    validateBindingDirectory()
    const metadata = maybeLstat(BINDING_PATH)
    if (!metadata) return undefined
    if (
      metadata.isSymbolicLink() || !metadata.isFile() || metadata.uid !== ROOT_UID ||
      metadata.gid !== 0 || (metadata.mode & 0o7777) !== 0o600 ||
      metadata.size > 4096
    ) {
      fail("persistent data-volume binding must be a root-only regular file")
    }
    return parseBinding(parseJson(readFileSync(BINDING_PATH, "utf8"), "data-volume binding"))
  },
  writeBinding(binding) {
    const parent = dirname(BINDING_PATH)
    validateBindingDirectory()
    const temporary = `${BINDING_PATH}.new-${process.pid}`
    const bytes = `${JSON.stringify(binding)}\n`
    writeFileSync(temporary, bytes, { mode: 0o600, flag: "wx" })
    chmodSync(temporary, 0o600)
    const fd = openSync(temporary, constants.O_RDONLY | constants.O_NOFOLLOW)
    try { fsyncSync(fd) } finally { closeSync(fd) }
    renameSync(temporary, BINDING_PATH)
    const directoryFd = openSync(parent, constants.O_RDONLY)
    try { fsyncSync(directoryFd) } finally { closeSync(directoryFd) }
  },
  backingFile() {
    validateBindingDirectory()
    return backingFileIdentity()
  },
  backingFileSpace() {
    validateBindingDirectory()
    const space = statfsSync(dirname(BACKING_FILE_PATH), { bigint: true })
    return Number(space.bavail * space.bsize)
  },
  createBackingFile(bytes) {
    validateBindingDirectory()
    const fd = openSync(BACKING_FILE_PATH, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600)
    try {
      ftruncateSync(fd, bytes)
      fsyncSync(fd)
    } finally { closeSync(fd) }
    const directoryFd = openSync(dirname(BACKING_FILE_PATH), constants.O_RDONLY)
    try { fsyncSync(directoryFd) } finally { closeSync(directoryFd) }
    return backingFileIdentity()
  },
  clearObservation() {
    validateObservationDirectory()
    const metadata = validateObservationFile(ADMISSION_OBSERVATION_PATH)
    if (!metadata) return
    unlinkSync(ADMISSION_OBSERVATION_PATH)
    const directoryFd = openSync(dirname(ADMISSION_OBSERVATION_PATH), constants.O_RDONLY)
    try { fsyncSync(directoryFd) } finally { closeSync(directoryFd) }
  },
  writeObservation(value) {
    validateObservationDirectory()
    validateObservation(value)
    const path = ADMISSION_OBSERVATION_PATH
    const temporary = `${path}.new-${process.pid}`
    const linuxBootId = readFileSync("/proc/sys/kernel/random/boot_id", "utf8").trim()
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(linuxBootId)) {
      fail("Linux boot ID is invalid while recording data-volume admission")
    }
    const bytes = `${JSON.stringify({
      ...value,
      linuxBootId,
    })}\n`
    writeFileSync(temporary, bytes, { mode: 0o644, flag: "wx" })
    chmodSync(temporary, 0o644)
    const fd = openSync(temporary, constants.O_RDONLY | constants.O_NOFOLLOW)
    try { fsyncSync(fd) } finally { closeSync(fd) }
    renameSync(temporary, path)
    const directoryFd = openSync(dirname(path), constants.O_RDONLY)
    try { fsyncSync(directoryFd) } finally { closeSync(directoryFd) }
  },
  ensureMountpoint() {
    let metadata = maybeLstat(DATA_ROOT)
    if (!metadata) {
      mkdirSync(DATA_ROOT, { mode: 0o700 })
      metadata = lstatSync(DATA_ROOT)
    }
    if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
      fail("fixed Docker data-root mountpoint is not a real directory")
    }
    return readdirSync(DATA_ROOT).length === 0
  },
  holders(majorMinor) {
    if (typeof majorMinor !== "string" || !/^\d+:\d+$/.test(majorMinor)) {
      fail("data volume major/minor identity is missing")
    }
    return readdirSync(`/sys/dev/block/${majorMinor}/holders`).length > 0
  },
  dockerOwner() {
    const row = readFileSync("/etc/passwd", "utf8")
      .split("\n")
      .find((line) => line.startsWith("chariox-docker:"))
    const fields = row?.split(":")
    const uid = Number(fields?.[2])
    const gid = Number(fields?.[3])
    if (!Number.isSafeInteger(uid) || uid <= 0 || !Number.isSafeInteger(gid) || gid <= 0) {
      fail("rootless Docker service account is unavailable")
    }
    return { uid, gid }
  },
}
