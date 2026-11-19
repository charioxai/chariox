// A file-backed data volume: a sparse image on the root filesystem, attached as a
// loop device at each boot, for machines whose provider attaches no data disk.
// Its identity is the backing file (filesystem device + inode), never a disk serial.

export const BACKING_FILE_PATH = "/var/lib/chariox-data-volume/docker-data.xfs"

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

export function sameBackingFile(left, right) {
  return left.majorMinor === right.majorMinor && left.inode === right.inode
}

// Creates the sparse image once, or returns the one already there; a file of another
// size or identity is never resized or replaced.
export function ensureBackingFile(io, wantedBytes, binding) {
  const existing = io.backingFile()
  if (binding) {
    if (!existing) fail("bound file-backed data volume is missing its backing file; refusing to create a new one")
    if (!sameBackingFile(existing, binding.backingFile)) {
      fail("file-backed data volume backing file was replaced; refusing to use it")
    }
  }
  if (existing) {
    if (existing.sizeBytes !== wantedBytes) {
      fail(`file-backed data volume capacity mismatch: expected ${wantedBytes} bytes, observed ${existing.sizeBytes}`)
    }
    return existing
  }
  const available = io.backingFileSpace()
  if (available < wantedBytes) {
    fail(`the root filesystem has ${available} bytes free; a ${wantedBytes}-byte file-backed data volume needs them. Choose a smaller volume or a machine with a larger disk`)
  }
  return io.createBackingFile(wantedBytes)
}

// Attaches the backing file, or finds its existing attachment, and describes the loop device.
export function attachBackingFile(commands, backingFile) {
  const path = commands.run("/usr/sbin/losetup", ["--find", "--show", "--nooverlap", BACKING_FILE_PATH])
  if (!/^\/dev\/loop[0-9]+$/.test(path)) fail("losetup returned an invalid loop device")
  // lsblk and the by-uuid link read what udev probed; a loop device attached in this boot
  // reads as blank until udev has seen it.
  commands.run("/usr/bin/udevadm", ["settle", "--timeout=30"])
  const loops = parseJson(commands.run("/usr/sbin/losetup", [
    "--list", "--json", "--output", "NAME,BACK-INO,BACK-MAJ:MIN,OFFSET,SIZELIMIT,RO", path,
  ]), "losetup").loopdevices
  if (!Array.isArray(loops) || loops.length !== 1) fail("losetup returned an ambiguous loop device")
  const loop = loops[0]
  if (
    String(loop["back-ino"]) !== backingFile.inode ||
    loop["back-maj:min"] !== backingFile.majorMinor
  ) {
    fail("loop device is not backed by the data volume backing file")
  }
  if (Number(loop.offset) !== 0 || Number(loop.sizelimit) !== 0) {
    fail("loop device maps only part of the data volume backing file")
  }
  if (loop.ro === true || loop.ro === 1 || loop.ro === "1") fail("data volume loop device is read-only")
  const devices = parseJson(commands.run("/usr/bin/lsblk", [
    "--bytes", "--json", "--nodeps", "--output", "PATH,SIZE,FSTYPE,UUID,MAJ:MIN", path,
  ]), "lsblk").blockdevices
  if (!Array.isArray(devices) || devices.length !== 1 || devices[0].path !== path) {
    fail("lsblk returned an ambiguous loop device")
  }
  const device = devices[0]
  const blockdevOutput = commands.run("/usr/sbin/blockdev", ["--getsize64", path])
  if (!/^[1-9][0-9]*$/.test(blockdevOutput) || Number(blockdevOutput) !== Number(device.size)) {
    fail("lsblk and blockdev disagree about the data volume capacity")
  }
  return {
    path,
    sizeBytes: Number(device.size),
    filesystemType: typeof device.fstype === "string" ? device.fstype : "",
    filesystemUuid: typeof device.uuid === "string" ? device.uuid : "",
    majorMinor: device["maj:min"],
  }
}
