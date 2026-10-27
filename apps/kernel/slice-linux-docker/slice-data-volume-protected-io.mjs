import {
  chmodSync,
  closeSync,
  constants,
  fsyncSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readdirSync,
  renameSync,
  unlinkSync,
  writeFileSync,
} from "node:fs"
import { dirname } from "node:path"

import { DATA_ROOT, MAX_VOLUME_SIZE_GB, MIN_VOLUME_SIZE_GB } from "./slice-data-volume-device.mjs"

export const PROTECTED_BOOTSTRAP_ROOT = "/etc/chariox/bootstrap"
export const MANAGED_BOOTSTRAP_PATH = `${PROTECTED_BOOTSTRAP_ROOT}/managed-bootstrap.json`
export const WORKER_BOOTSTRAP_PATH = `${PROTECTED_BOOTSTRAP_ROOT}/disposable-worker-bootstrap.json`
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
  const serial = value.expectedDataVolumeSerial
  const sizeGb = value.expectedDataVolumeSizeGb
  if (
    typeof serial !== "string" || serial.length > 16 || !/^[1-9][0-9]*$/.test(serial) ||
    BigInt(serial) > BigInt(MAX_SAFE_INTEGER) ||
    !Number.isSafeInteger(sizeGb) || sizeGb < MIN_VOLUME_SIZE_GB || sizeGb > MAX_VOLUME_SIZE_GB
  ) {
    fail("protected bootstrap data-volume identity or size is invalid")
  }
  return { role: path === MANAGED_BOOTSTRAP_PATH ? "managed" : "worker", serial, sizeGb }
}

function parseBinding(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail("data-volume binding is malformed")
  const keys = Object.keys(value).sort().join(",")
  if (keys !== "expectedDataVolumeSerial,expectedDataVolumeSizeGb,filesystemUuid,schemaVersion") {
    fail("data-volume binding has an unsupported shape")
  }
  if (
    value.schemaVersion !== 1 || typeof value.expectedDataVolumeSerial !== "string" ||
    !/^[1-9][0-9]*$/.test(value.expectedDataVolumeSerial) ||
    BigInt(value.expectedDataVolumeSerial) > BigInt(MAX_SAFE_INTEGER) ||
    !Number.isSafeInteger(value.expectedDataVolumeSizeGb) ||
    value.expectedDataVolumeSizeGb < MIN_VOLUME_SIZE_GB ||
    value.expectedDataVolumeSizeGb > MAX_VOLUME_SIZE_GB ||
    typeof value.filesystemUuid !== "string" ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value.filesystemUuid)
  ) {
    fail("data-volume binding fields are invalid")
  }
  return {
    schemaVersion: 1,
    expectedDataVolumeSerial: value.expectedDataVolumeSerial,
    expectedDataVolumeSizeGb: value.expectedDataVolumeSizeGb,
    filesystemUuid: value.filesystemUuid,
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
    value.schemaVersion !== 1 || typeof value.dataVolumeSerial !== "string" ||
    !/^[1-9][0-9]{0,15}$/.test(value.dataVolumeSerial) ||
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

export const systemIo = {
  readStorageInput() {
    const gid = charioxGroupId()
    const managed = readProtectedInput(MANAGED_BOOTSTRAP_PATH, 3, gid)
    const worker = readProtectedInput(WORKER_BOOTSTRAP_PATH, 2, gid)
    if (managed && worker) fail("both protected managed and worker bootstrap inputs are present")
    return managed ?? worker
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
      fail("Hetzner Volume major/minor identity is missing")
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
