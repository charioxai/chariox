export const DATA_ROOT = "/var/lib/chariox-docker/data"
export const BYTES_PER_GIB = 1024 ** 3
// Cloud validates the size against its adapter limits; the kernel only keeps a sanity range.
export const MIN_VOLUME_SIZE_GB = 1
export const MAX_VOLUME_SIZE_GB = Math.floor(Number.MAX_SAFE_INTEGER / BYTES_PER_GIB)

const MAX_SAFE_INTEGER = Number.MAX_SAFE_INTEGER

function fail(message) {
  throw new Error(message)
}

function parseJson(value, label) {
  try {
    return JSON.parse(value)
  } catch {
    fail(`${label} returned invalid JSON`)
  }
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

export function expectedVolumeBytes(sizeGb) {
  if (!Number.isSafeInteger(sizeGb) || sizeGb < MIN_VOLUME_SIZE_GB || sizeGb > MAX_VOLUME_SIZE_GB) {
    fail("expected data volume size is outside its supported integer-GB range")
  }
  const bytes = BigInt(sizeGb) * BigInt(BYTES_PER_GIB)
  if (bytes <= 0n || bytes > BigInt(MAX_SAFE_INTEGER)) {
    fail("expected data volume capacity exceeds the safe integer range")
  }
  return Number(bytes)
}

export function selectDataVolumeDevice(devices, expectedSerial, rootSourcePath, allowDataRootMount = false) {
  const flattened = flattenBlockDevices(devices)
  const rootPath = normalizeDevicePath(rootSourcePath)
  if (!rootPath) fail("root filesystem source is not a block-device path")
  const rootDevice = physicalRootDevice(flattened, rootPath)
  const matches = flattened.filter(({ device }) => device.serial === expectedSerial)
  if (matches.length !== 1) fail("expected provider volume serial must identify exactly one device")
  const selected = matches[0]
  if (rootDevice === selected.device || blockPath(rootDevice) === blockPath(selected.device)) {
    fail("refusing to use the root filesystem disk as the data volume")
  }
  const dataDisks = flattened.filter(({ device }) =>
    String(device.type).toLowerCase() === "disk" && blockPath(device) !== blockPath(rootDevice),
  )
  if (dataDisks.length !== 1) fail("exactly one data volume disk must be attached")
  if (
    selected.device !== dataDisks[0].device ||
    String(selected.device.type).toLowerCase() !== "disk" ||
    selected.parent ||
    (typeof selected.device.pkname === "string" && selected.device.pkname !== "")
  ) {
    fail("expected provider volume must be a whole data volume disk")
  }
  if (blockChildren(selected.device).length !== 0) {
    fail("data volume must not contain partitions or child devices")
  }
  const readOnly = selected.device.ro === true || selected.device.ro === 1 || selected.device.ro === "1"
  if (readOnly) fail("data volume is read-only")
  const sizeBytes = Number(selected.device.size)
  if (!Number.isSafeInteger(sizeBytes) || sizeBytes <= 0) {
    fail("data volume block capacity is invalid")
  }
  const mountpoints = [
    ...(Array.isArray(selected.device.mountpoints) ? selected.device.mountpoints : []),
    selected.device.mountpoint,
  ].filter((value) => typeof value === "string" && value !== "")
  if (mountpoints.some((value) => !allowDataRootMount || value !== DATA_ROOT)) {
    fail("data volume is already mounted")
  }
  const devicePath = blockPath(selected.device)
  if (!devicePath) fail("data volume device path is missing")
  return {
    path: devicePath,
    serial: selected.device.serial,
    sizeBytes,
    filesystemType: typeof selected.device.fstype === "string" ? selected.device.fstype : "",
    filesystemUuid: typeof selected.device.uuid === "string" ? selected.device.uuid : "",
    majorMinor: selected.device["maj:min"] ?? selected.device.majmin,
    mountpoints,
  }
}

export function discoverVolumeDevice(commands, serial) {
  const lsblk = parseJson(commands.run("/usr/bin/lsblk", [
    "--bytes", "--json", "--tree", "--output", "PATH,TYPE,SERIAL,MODEL,SIZE,RO,FSTYPE,UUID,MOUNTPOINTS,PKNAME,MAJ:MIN",
  ]), "lsblk")
  const rootSource = commands.run("/usr/bin/findmnt", ["--noheadings", "--output", "SOURCE", "--target", "/"])
  const rootSourcePath = commands.run("/usr/bin/realpath", ["-e", rootSource])
  const device = selectDataVolumeDevice(lsblk.blockdevices, serial, rootSourcePath, true)
  const blockdevOutput = commands.run("/usr/sbin/blockdev", ["--getsize64", device.path])
  if (!/^[1-9][0-9]*$/.test(blockdevOutput)) {
    fail("blockdev returned an invalid data volume capacity")
  }
  const blockdevSizeBytes = Number(blockdevOutput)
  if (!Number.isSafeInteger(blockdevSizeBytes) || blockdevSizeBytes !== device.sizeBytes) {
    fail("lsblk and blockdev disagree about the data volume capacity")
  }
  return device
}
