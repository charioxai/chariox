#!/usr/bin/env node

import { spawnSync } from "node:child_process"
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
  writeFileSync,
} from "node:fs"
import { basename, dirname } from "node:path"
import { pathToFileURL } from "node:url"
import { parseProjectQuotaState } from "./slice-disk-quota-xfs-readback.mjs"

export const PROTECTED_BOOTSTRAP_ROOT = "/etc/chariox/bootstrap"
export const MANAGED_BOOTSTRAP_PATH = `${PROTECTED_BOOTSTRAP_ROOT}/managed-bootstrap.json`
export const WORKER_BOOTSTRAP_PATH = `${PROTECTED_BOOTSTRAP_ROOT}/disposable-worker-bootstrap.json`
export const DATA_ROOT = "/var/lib/chariox-docker/data"
export const BINDING_PATH = "/var/lib/chariox-data-volume/volume-binding.json"
// Hetzner CSI SizeBytes at 0e4a9849184294b1629d304dc9b6d336dbb6969c uses 1024^3 per API GB.
export const HETZNER_BYTES_PER_GB = 1024 ** 3
export const MIN_VOLUME_SIZE_GB = 10
export const MAX_VOLUME_SIZE_GB = 10_000
const MAX_SAFE_INTEGER = Number.MAX_SAFE_INTEGER
const MAX_BOOTSTRAP_BYTES = 96 * 1024
const ROOT_UID = 0

function fail(message) {
  throw new Error(message)
}

function commandResult(command, args) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: 1024 * 1024,
    timeout: 30_000,
    env: { PATH: "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" },
  })
  if (result.error) fail(`${basename(command)} failed: ${result.error.message}`)
  return {
    status: result.status ?? 1,
    stdout: result.stdout ?? "",
    stderr: result.stderr ?? "",
  }
}

function run(command, args) {
  const result = commandResult(command, args)
  if (result.status !== 0) {
    fail(`${basename(command)} failed: ${result.stderr.trim() || `exit ${result.status}`}`)
  }
  return result.stdout.trim()
}

function maybeLstat(path) {
  try {
    return lstatSync(path)
  } catch (error) {
    if (error.code === "ENOENT") return undefined
    throw error
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

export function expectedVolumeBytes(sizeGb) {
  if (!Number.isSafeInteger(sizeGb) || sizeGb < MIN_VOLUME_SIZE_GB || sizeGb > MAX_VOLUME_SIZE_GB) {
    fail("expected Hetzner Volume size is outside its supported integer-GB range")
  }
  const bytes = BigInt(sizeGb) * BigInt(HETZNER_BYTES_PER_GB)
  if (bytes <= 0n || bytes > BigInt(MAX_SAFE_INTEGER)) {
    fail("expected Hetzner Volume capacity exceeds the safe integer range")
  }
  return Number(bytes)
}

function blockPath(device) {
  return typeof device.path === "string" ? device.path : undefined
}

function blockChildren(device) {
  return Array.isArray(device.children) ? device.children : []
}

function flattenBlockDevices(devices) {
  if (!Array.isArray(devices)) fail("lsblk returned no block-device list")
  const flattened = []
  const visit = (device, parent) => {
    if (!device || typeof device !== "object") fail("lsblk returned a malformed block device")
    flattened.push({ device, parent })
    for (const child of blockChildren(device)) visit(child, device)
  }
  for (const device of devices) visit(device, undefined)
  return flattened
}

function normalizeDevicePath(value) {
  if (typeof value !== "string" || !value.startsWith("/dev/")) return undefined
  return value
}

function findDeviceByPath(flattened, wanted) {
  return flattened.find(({ device }) => blockPath(device) === wanted)
}

function physicalRootDevice(flattened, rootSourcePath) {
  let current = findDeviceByPath(flattened, rootSourcePath)
  if (!current) fail("root filesystem source is not present in lsblk output")
  const visited = new Set()
  while (true) {
    const path = blockPath(current.device)
    if (!path || visited.has(path)) fail("root disk ancestry is malformed")
    visited.add(path)
    let parent = current.parent
    if (!parent && typeof current.device.pkname === "string" && current.device.pkname !== "") {
      const parentPath = current.device.pkname.startsWith("/dev/")
        ? current.device.pkname
        : `/dev/${current.device.pkname}`
      parent = findDeviceByPath(flattened, parentPath)?.device
      if (!parent) fail("root disk ancestry cannot be established")
    }
    if (!parent) {
      if (String(current.device.type).toLowerCase() !== "disk") {
        fail("root filesystem physical disk cannot be established")
      }
      return current.device
    }
    current = flattened.find((entry) => entry.device === parent)
      ?? { device: parent, parent: undefined }
  }
}

export function selectHetznerVolumeDevice(devices, expectedSerial, rootSourcePath, allowDataRootMount = false) {
  const flattened = flattenBlockDevices(devices)
  const volumeDisks = flattened.filter(({ device }) =>
    String(device.type).toLowerCase() === "disk" && String(device.model ?? "").trim() === "Volume",
  )
  if (volumeDisks.length !== 1) fail("exactly one Hetzner Volume disk must be attached")
  const matches = flattened.filter(({ device }) => device.serial === expectedSerial)
  if (matches.length !== 1) fail("expected provider volume serial must identify exactly one device")
  const selected = matches[0]
  if (
    selected.device !== volumeDisks[0].device ||
    String(selected.device.type).toLowerCase() !== "disk" ||
    String(selected.device.model ?? "").trim() !== "Volume" ||
    selected.parent ||
    (typeof selected.device.pkname === "string" && selected.device.pkname !== "")
  ) {
    fail("expected provider volume must be a whole Hetzner Volume disk")
  }
  if (blockChildren(selected.device).length !== 0) {
    fail("Hetzner Volume must not contain partitions or child devices")
  }
  const rootPath = normalizeDevicePath(rootSourcePath)
  if (!rootPath) fail("root filesystem source is not a block-device path")
  const rootDevice = physicalRootDevice(flattened, rootPath)
  if (rootDevice === selected.device || blockPath(rootDevice) === blockPath(selected.device)) {
    fail("refusing to use the root filesystem disk as the data volume")
  }
  const readOnly = selected.device.ro === true || selected.device.ro === 1 || selected.device.ro === "1"
  if (readOnly) fail("Hetzner Volume is read-only")
  const sizeBytes = Number(selected.device.size)
  if (!Number.isSafeInteger(sizeBytes) || sizeBytes <= 0) {
    fail("Hetzner Volume block capacity is invalid")
  }
  const mountpoints = [
    ...(Array.isArray(selected.device.mountpoints) ? selected.device.mountpoints : []),
    selected.device.mountpoint,
  ].filter((value) => typeof value === "string" && value !== "")
  if (mountpoints.some((value) => !allowDataRootMount || value !== DATA_ROOT)) {
    fail("Hetzner Volume is already mounted")
  }
  const devicePath = blockPath(selected.device)
  if (!devicePath) fail("Hetzner Volume device path is missing")
  return {
    path: devicePath,
    serial: expectedSerial,
    sizeBytes,
    filesystemType: typeof selected.device.fstype === "string" ? selected.device.fstype : "",
    filesystemUuid: typeof selected.device.uuid === "string" ? selected.device.uuid : "",
    majorMinor: selected.device["maj:min"] ?? selected.device.majmin,
    mountpoints,
  }
}

function parseJson(value, label) {
  try {
    return JSON.parse(value)
  } catch {
    fail(`${label} returned invalid JSON`)
  }
}

function findDataMount(commands) {
  const result = commands.tryRun("/usr/bin/findmnt", [
    "--json", "--mountpoint", DATA_ROOT, "--output", "TARGET,SOURCE,FSTYPE,OPTIONS",
  ])
  if (result.status === 1) return undefined
  if (result.status !== 0) fail("could not verify the Docker data-root mount")
  const filesystems = parseJson(result.stdout, "findmnt").filesystems
  if (!Array.isArray(filesystems) || filesystems.length !== 1) {
    fail("findmnt returned an ambiguous Docker data-root mount")
  }
  return filesystems[0]
}

function assertMountMatches(mount, device, binding, commands) {
  if (!mount) return false
  const target = mount.target ?? mount.TARGET
  const source = mount.source ?? mount.SOURCE
  const filesystemType = mount.fstype ?? mount.FSTYPE
  const options = String(mount.options ?? mount.OPTIONS ?? "").split(",")
  if (target !== DATA_ROOT || filesystemType !== "xfs" || !options.some((value) => ["pquota", "prjquota"].includes(value))) {
    fail("Docker data-root must be the fixed XFS mount with project quotas enabled")
  }
  const sourcePath = commands.run("/usr/bin/realpath", ["-e", source])
  const deviceRealPath = commands.run("/usr/bin/realpath", ["-e", device.path])
  if (sourcePath !== deviceRealPath) fail("mounted data-root source does not match the expected Hetzner Volume")
  if (binding && binding.filesystemUuid !== device.filesystemUuid) {
    fail("mounted data-root filesystem UUID changed")
  }
  return true
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

const systemIo = {
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

const systemCommands = {
  run,
  tryRun(command, args) {
    return commandResult(command, args)
  },
}

function filesystemSignatures(commands, devicePath) {
  return commands.run("/usr/sbin/wipefs", ["--no-act", "--noheadings", "--output", "TYPE", devicePath])
    .split(/\r?\n/)
    .map((value) => value.trim().toLowerCase())
    .filter(Boolean)
}

function assertDeviceNotBusy(device, io, commands, mountedAtDataRoot) {
  const mountpoints = commands.run("/usr/bin/lsblk", ["--noheadings", "--raw", "--output", "MOUNTPOINTS", device.path])
    .split(/\r?\n/)
    .map((value) => value.trim())
    .filter(Boolean)
  if (mountpoints.some((value) => !mountedAtDataRoot || value !== DATA_ROOT)) {
    fail("Hetzner Volume has an unexpected mount or partition use")
  }
  if (io.holders(device.majorMinor)) fail("Hetzner Volume is in use by another block device")
  if (mountedAtDataRoot) {
    const filesystemUse = commands.tryRun("/usr/bin/fuser", ["--mount", "--", DATA_ROOT])
    if (filesystemUse.status === 0) fail("Hetzner Volume filesystem is in use by a process")
    if (filesystemUse.status !== 1) fail("could not verify that the Hetzner Volume filesystem is unused")
  }
  const inUse = commands.tryRun("/usr/bin/fuser", ["--", device.path])
  if (inUse.status === 0) fail("Hetzner Volume is in use by a process")
  if (inUse.status !== 1) fail("could not verify that the Hetzner Volume is unused")
}

function readVolumeUuid(commands, devicePath) {
  const uuid = commands.run("/usr/sbin/blkid", ["--output", "value", "--match-tag", "UUID", devicePath])
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(uuid)) {
    fail("XFS filesystem UUID could not be read")
  }
  return uuid
}

function mountDataRoot(commands, device, filesystemUuid, waitForUdev = false) {
  if (waitForUdev) commands.run("/usr/bin/udevadm", ["settle", "--timeout=30"])
  const uuidPath = `/dev/disk/by-uuid/${filesystemUuid}`
  const uuidDevice = commands.run("/usr/bin/realpath", ["-e", uuidPath])
  const selectedDevice = commands.run("/usr/bin/realpath", ["-e", device.path])
  if (uuidDevice !== selectedDevice) {
    fail("filesystem UUID does not resolve to the expected Hetzner Volume")
  }
  // Ask PID 1 to own the mount so it survives this hardened service's private
  // filesystem namespace and participates in RequiresMountsFor dependencies.
  commands.run("/usr/bin/systemd-mount", [
    "--no-ask-password",
    "--automount=no",
    "--fsck=no",
    "--type=xfs",
    "--options=pquota",
    uuidPath,
    DATA_ROOT,
  ])
}

function discoverVolumeDevice(commands, serial) {
  const lsblk = parseJson(commands.run("/usr/bin/lsblk", [
    "--bytes", "--json", "--output", "PATH,TYPE,SERIAL,MODEL,SIZE,RO,FSTYPE,UUID,MOUNTPOINTS,PKNAME,MAJ:MIN",
  ]), "lsblk")
  const rootSource = commands.run("/usr/bin/findmnt", ["--noheadings", "--output", "SOURCE", "--target", "/"])
  const rootSourcePath = commands.run("/usr/bin/realpath", ["-e", rootSource])
  const device = selectHetznerVolumeDevice(lsblk.blockdevices, serial, rootSourcePath, true)
  const blockdevOutput = commands.run("/usr/sbin/blockdev", ["--getsize64", device.path])
  if (!/^[1-9][0-9]*$/.test(blockdevOutput)) {
    fail("blockdev returned an invalid Hetzner Volume capacity")
  }
  const blockdevSizeBytes = Number(blockdevOutput)
  if (!Number.isSafeInteger(blockdevSizeBytes) || blockdevSizeBytes !== device.sizeBytes) {
    fail("lsblk and blockdev disagree about the Hetzner Volume capacity")
  }
  return device
}

function verifyXfsQuotaAndOverlay(commands, mountpoint) {
  const quotaOutput = commands.run("/usr/sbin/xfs_quota", [
    "-x", "-D", "/dev/null", "-P", "/dev/null", "-c", "state -p", mountpoint,
  ])
  const quotaState = parseProjectQuotaState(quotaOutput)
  if (quotaState.accounting !== true || quotaState.enforcement !== true) {
    fail("XFS project quota accounting and enforcement are not both active")
  }
  const info = commands.run("/usr/sbin/xfs_info", [mountpoint])
  if (!/(^|[\s,])ftype=1([\s,]|$)/.test(info)) {
    fail("managed Docker XFS filesystem must report ftype=1")
  }
}

function applyDockerRootOwnership(io, commands) {
  const owner = io.dockerOwner()
  commands.run("/usr/bin/chown", ["--", `${owner.uid}:${owner.gid}`, DATA_ROOT])
  commands.run("/usr/bin/chmod", ["0700", DATA_ROOT])
}

export function admitDataVolume({ commands = systemCommands, io = systemIo } = {}) {
  const input = io.readStorageInput()
  if (!input) fail("Path-1 admission requires a protected storage bootstrap input")
  const wantedBytes = expectedVolumeBytes(input.sizeGb)
  let device = discoverVolumeDevice(commands, input.serial)
  if (device.sizeBytes !== wantedBytes) {
    fail(`Hetzner Volume block capacity mismatch: expected ${wantedBytes} bytes, observed ${device.sizeBytes}`)
  }
  const binding = io.readBinding()
  if (binding && (
    binding.expectedDataVolumeSerial !== input.serial ||
    binding.expectedDataVolumeSizeGb !== input.sizeGb
  )) {
    fail("persistent data-volume identity does not match the protected bootstrap input")
  }
  const mount = findDataMount(commands)
  const mountedAtDataRoot = assertMountMatches(mount, device, binding, commands)
  assertDeviceNotBusy(device, io, commands, mountedAtDataRoot)
  const signatures = filesystemSignatures(commands, device.path)
  if (binding) {
    if (
      device.filesystemType !== "xfs" || device.filesystemUuid !== binding.filesystemUuid ||
      signatures.length !== 1 || signatures[0] !== "xfs"
    ) {
      fail("bound Hetzner Volume filesystem identity changed; refusing to format")
    }
    if (!mountedAtDataRoot) {
      if (!io.ensureMountpoint()) fail("unmounted Docker data-root contains unbound files")
      mountDataRoot(commands, device, binding.filesystemUuid)
    }
  } else {
    if (mountedAtDataRoot || device.filesystemType || device.filesystemUuid || signatures.length !== 0) {
      fail("unbound Hetzner Volume is not provably blank; refusing to format")
    }
    assertDeviceNotBusy(device, io, commands, false)
    if (!io.ensureMountpoint()) fail("Docker data-root contains files before initial volume admission")
    commands.run("/usr/sbin/mkfs.xfs", ["-f", "-n", "ftype=1", device.path])
    const filesystemUuid = readVolumeUuid(commands, device.path)
    const newBinding = {
      schemaVersion: 1,
      expectedDataVolumeSerial: input.serial,
      expectedDataVolumeSizeGb: input.sizeGb,
      filesystemUuid,
    }
    io.writeBinding(newBinding)
    mountDataRoot(commands, device, filesystemUuid, true)
  }
  device = discoverVolumeDevice(commands, input.serial)
  if (device.sizeBytes !== wantedBytes) {
    fail(`Hetzner Volume block capacity changed during admission: expected ${wantedBytes} bytes, observed ${device.sizeBytes}`)
  }
  const verifiedMount = findDataMount(commands)
  if (!assertMountMatches(verifiedMount, device, binding ?? io.readBinding(), commands)) {
    fail("data volume did not mount at the fixed Docker data-root")
  }
  verifyXfsQuotaAndOverlay(commands, DATA_ROOT)
  applyDockerRootOwnership(io, commands)
  return { admitted: true, serial: input.serial, sizeGb: input.sizeGb, filesystemUuid: (binding ?? io.readBinding()).filesystemUuid }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const result = admitDataVolume()
    process.stdout.write(`${JSON.stringify(result)}\n`)
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
    process.exitCode = 1
  }
}
