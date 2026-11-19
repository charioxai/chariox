import assert from "node:assert/strict"
import test from "node:test"

import {
  DATA_ROOT,
  BYTES_PER_GIB,
  admitDataVolume,
  expectedVolumeBytes,
  selectDataVolumeDevice,
} from "./chariox-data-volume-admission.mjs"
import { parseBinding, parseStorageInput } from "./slice-data-volume-protected-io.mjs"

const SERIAL = "12345"
const UUID = "12345678-1234-1234-1234-123456789abc"
const LOOP = "/dev/loop7"
const BACKING_FILE = { majorMinor: "252:1", inode: "4242" }

function makeDevices({ sizeBytes = 10 * BYTES_PER_GIB, filesystem = "", uuid = "", mounted = false, partitionedVolume = false } = {}) {
  return [
    {
      path: "/dev/vda",
      type: "disk",
      model: "System disk",
      size: 80 * BYTES_PER_GIB,
      children: [{ path: "/dev/vda1", type: "part", pkname: "vda", size: 80 * BYTES_PER_GIB }],
    },
    {
      path: "/dev/sdb",
      type: "disk",
      model: "Volume",
      serial: SERIAL,
      size: sizeBytes,
      ro: false,
      fstype: filesystem,
      uuid,
      mountpoints: mounted ? [DATA_ROOT] : [],
      "maj:min": "8:16",
      children: partitionedVolume
        ? [{ path: "/dev/sdb1", type: "part", pkname: "sdb", size: sizeBytes }]
        : [],
    },
  ]
}

function fakeAdmission({
  initialFilesystem = "",
  sizeBytes = 10 * BYTES_PER_GIB,
  blockdevSizeBytes = sizeBytes,
  busyDevice = false,
  busyFilesystem = false,
  hasHolder = false,
  partitionedVolume = false,
  fileVolume = false,
  rootFreeBytes = 100 * BYTES_PER_GIB,
} = {}) {
  const volumePath = fileVolume ? LOOP : "/dev/sdb"
  let backingFile
  let attached = false
  // udev probes a newly attached loop device; until then lsblk reports no filesystem.
  let probed = false
  let filesystem = initialFilesystem
  let filesystemUuid = initialFilesystem ? UUID : ""
  let mounted = false
  let ownership = "0:0:755"
  let binding
  let observation
  const calls = []
  const commands = {
    run(command, args) {
      calls.push([command, args])
      if (command === "/usr/sbin/losetup" && args.includes("--find")) {
        assert.ok(backingFile, "losetup attaches only an existing backing file")
        if (!attached) probed = false
        attached = true
        return LOOP
      }
      if (command === "/usr/sbin/losetup" && args.includes("--list")) {
        return JSON.stringify({ loopdevices: [{
          name: LOOP, "back-ino": Number(backingFile.inode), "back-maj:min": backingFile.majorMinor, offset: 0, sizelimit: 0, ro: false,
        }] })
      }
      if (command === "/usr/bin/lsblk" && args.includes("--nodeps")) {
        assert.ok(attached, "the loop device exists only while attached")
        const known = probed && filesystem
        return JSON.stringify({ blockdevices: [{ path: LOOP, size: backingFile.sizeBytes, fstype: known ? filesystem : null, uuid: known ? filesystemUuid : null, "maj:min": "7:7" }] })
      }
      if (command === "/usr/bin/lsblk" && args.includes("--json")) {
        return JSON.stringify({ blockdevices: makeDevices({ sizeBytes, filesystem, uuid: filesystemUuid, mounted, partitionedVolume }) })
      }
      if (command === "/usr/bin/findmnt" && args.includes("--target")) return "/dev/vda1"
      if (command === "/usr/bin/realpath") {
        return args[1].startsWith("/dev/disk/by-uuid/") ? volumePath : args[1]
      }
      if (command === "/usr/bin/lsblk") return mounted ? `${DATA_ROOT}\n` : ""
      if (command === "/usr/sbin/blockdev") return String(blockdevSizeBytes)
      if (command === "/usr/sbin/wipefs") return filesystem ? `${filesystem}\n` : ""
      if (command === "/usr/sbin/mkfs.xfs") {
        filesystem = "xfs"
        filesystemUuid = UUID
        return ""
      }
      if (command === "/usr/sbin/blkid") return UUID
      if (command === "/usr/bin/udevadm") {
        probed = attached
        return ""
      }
      if (command === "/usr/bin/systemd-mount") {
        mounted = true
        return ""
      }
      if (command === "/usr/sbin/xfs_quota") {
        return `Project quota state on ${DATA_ROOT} (/dev/sdb)\n  Accounting: ON\n  Enforcement: ON\n`
      }
      if (command === "/usr/sbin/xfs_info") return "meta-data=/dev/sdb isize=512 agcount=4, ftype=1\n"
      if (command === "/usr/bin/stat") return ownership
      if (command === "/usr/bin/chown" || command === "/usr/bin/chmod") {
        ownership = "1001:1001:700"
        return ""
      }
      throw new Error(`unexpected command: ${command} ${args.join(" ")}`)
    },
    tryRun(command, args) {
      calls.push([command, args])
      if (command === "/usr/bin/fuser") {
        const busy = args.includes("--mount") ? busyFilesystem : busyDevice
        return { status: busy ? 0 : 1, stdout: "", stderr: "" }
      }
      if (command === "/usr/bin/findmnt" && args.includes("--mountpoint")) {
        return mounted
          ? {
              status: 0,
              stdout: JSON.stringify({
                filesystems: [{
                  target: DATA_ROOT,
                  source: volumePath,
                  fstype: "xfs",
                  options: "rw,prjquota",
                }],
              }),
              stderr: "",
            }
          : { status: 1, stdout: "", stderr: "" }
      }
      throw new Error(`unexpected probe: ${command} ${args.join(" ")}`)
    },
  }
  const io = {
    readStorageInput: () => parseStorageInput(fileVolume
      ? { fileDataVolumeSizeGb: 10 }
      : { expectedDataVolumeSerial: SERIAL, expectedDataVolumeSizeGb: 10 }),
    readBinding: () => binding && parseBinding(binding),
    backingFile: () => backingFile,
    backingFileSpace: () => rootFreeBytes,
    createBackingFile: (bytes) => {
      assert.equal(backingFile, undefined)
      backingFile = { ...BACKING_FILE, sizeBytes: bytes }
      return backingFile
    },
    writeBinding: (value) => { binding = value },
    ensureMountpoint: () => true,
    holders: () => hasHolder,
    dockerOwner: () => ({ uid: 1001, gid: 1001 }),
    clearObservation: () => { observation = undefined },
    writeObservation: (value) => { observation = value },
  }
  return {
    commands,
    io,
    calls,
    simulateReboot() { mounted = false; attached = false },
    replaceBackingFile() { backingFile = { ...backingFile, inode: "4343" } },
    simulateDockerStart() { ownership = "1001:1001:710" },
    setFilesystemUse(value) { busyFilesystem = value },
    setOwnership(value) { ownership = value },
    get binding() { return binding },
    get observation() { return observation },
  }
}

test("Provider GB maps to exact binary block bytes", () => {
  assert.equal(expectedVolumeBytes(10), 10 * 1024 ** 3)
  assert.throws(() => expectedVolumeBytes(0), /supported integer-GB range/)
  assert.throws(() => expectedVolumeBytes(Number.MAX_SAFE_INTEGER), /supported integer-GB range/)
})

test("selects one matching whole data volume disk apart from the root disk", () => {
  const device = selectDataVolumeDevice(makeDevices(), SERIAL, "/dev/vda1")
  assert.equal(device.path, "/dev/sdb")
  assert.equal(device.serial, SERIAL)
  assert.equal(device.sizeBytes, 10 * BYTES_PER_GIB)
})

test("selects a data volume whatever model name the platform reports", () => {
  const devices = makeDevices()
  devices[1].model = "NVMe Data Disk"
  assert.equal(selectDataVolumeDevice(devices, SERIAL, "/dev/vda1").path, "/dev/sdb")
})

test("rejects duplicate volumes, partitions, mounts, and the root disk", () => {
  const devices = makeDevices()
  assert.throws(
    () => selectDataVolumeDevice([...devices, { ...devices[1], path: "/dev/sdc", serial: "54321" }], SERIAL, "/dev/vda1"),
    /exactly one data volume disk/,
  )
  const partitioned = makeDevices()
  partitioned[1].children = [{ path: "/dev/sdb1", type: "part", pkname: "sdb" }]
  assert.throws(() => selectDataVolumeDevice(partitioned, SERIAL, "/dev/vda1"), /partitions or child devices/)
  assert.throws(
    () => selectDataVolumeDevice(makeDevices({ mounted: true }), SERIAL, "/dev/vda1"),
    /already mounted/,
  )
  assert.throws(() => selectDataVolumeDevice(makeDevices(), SERIAL, "/dev/sdb"), /root filesystem disk/)
})

test("production lsblk tree output with a data-volume partition fails before formatting", () => {
  const fake = fakeAdmission({ partitionedVolume: true })
  assert.throws(() => admitDataVolume(fake), /partitions or child devices/)
  assert.equal(fake.calls.some(([command]) => command === "/usr/sbin/mkfs.xfs"), false)
})

test("fresh matching blank volume is formatted once, bound, mounted, and quota-verified across reboot", () => {
  const fake = fakeAdmission()
  const result = admitDataVolume(fake)
  assert.ok(fake.calls.some(([command, args]) =>
    command === "/usr/bin/lsblk" && args.includes("--json") && args.includes("--tree"),
  ))
  assert.deepEqual(result, {
    admitted: true,
    serial: SERIAL,
    sizeGb: 10,
    filesystemUuid: UUID,
  })
  assert.equal(fake.binding.expectedDataVolumeSerial, SERIAL)
  assert.equal(fake.binding.expectedDataVolumeSizeGb, 10)
  assert.equal(fake.binding.filesystemUuid, UUID)
  assert.deepEqual(fake.observation, {
    schemaVersion: 1,
    dataVolumeSerial: SERIAL,
    dataVolumeSizeGb: 10,
    filesystemUuid: UUID,
    devicePath: "/dev/sdb",
    majorMinor: "8:16",
    mountTarget: DATA_ROOT,
  })
  assert.equal(fake.calls.filter(([command]) => command === "/usr/sbin/mkfs.xfs").length, 1)
  assert.equal(fake.calls.filter(([command]) => command === "/usr/bin/systemd-mount").length, 1)
  assert.ok(fake.calls.some(([command]) => command === "/usr/sbin/xfs_quota"))

  fake.simulateReboot()
  const rebootResult = admitDataVolume(fake)
  assert.deepEqual(rebootResult, result)
  assert.equal(fake.calls.filter(([command]) => command === "/usr/sbin/mkfs.xfs").length, 1)
  assert.equal(fake.calls.filter(([command]) => command === "/usr/bin/systemd-mount").length, 2)

  // A dependent restart re-runs admission on the live mount: verify without writing.
  const writes = () => fake.calls.filter(([command]) => command === "/usr/bin/chown" || command === "/usr/bin/chmod").length
  const before = writes()
  assert.deepEqual(admitDataVolume(fake), result)
  assert.equal(writes(), before)
  // Rootless dockerd leaves its data root at 0710; a restart still writes nothing.
  fake.simulateDockerStart()
  assert.deepEqual(admitDataVolume(fake), result)
  assert.equal(writes(), before)
  assert.equal(fake.calls.filter(([command]) => command === "/usr/bin/systemd-mount").length, 2)
})

test("a file-backed volume is created once, bound by its backing file and UUID, and attached again after a reboot", () => {
  const fake = fakeAdmission({ fileVolume: true })
  const result = admitDataVolume(fake)
  assert.deepEqual(result, { admitted: true, backingFile: BACKING_FILE, sizeGb: 10, filesystemUuid: UUID })
  assert.deepEqual(fake.binding, {
    schemaVersion: 1,
    backingFile: BACKING_FILE,
    expectedDataVolumeSizeGb: 10,
    filesystemUuid: UUID,
  })
  assert.deepEqual(fake.observation, {
    schemaVersion: 1,
    backingFile: BACKING_FILE,
    dataVolumeSizeGb: 10,
    filesystemUuid: UUID,
    devicePath: LOOP,
    majorMinor: "7:7",
    mountTarget: DATA_ROOT,
  })
  assert.ok(fake.calls.some(([command, args]) => command === "/usr/sbin/mkfs.xfs" && args.at(-1) === LOOP))
  assert.ok(fake.calls.some(([command, args]) =>
    command === "/usr/bin/systemd-mount" && args.includes("--options=pquota") && args.at(-1) === DATA_ROOT,
  ))

  fake.simulateReboot()
  assert.deepEqual(admitDataVolume(fake), result)
  assert.equal(fake.calls.filter(([command]) => command === "/usr/sbin/mkfs.xfs").length, 1)
  assert.equal(fake.calls.filter(([command]) => command === "/usr/bin/systemd-mount").length, 2)
  // Each admission attaches the backing file before and after mounting (losetup reuses a live
  // attachment) and reads it only after udev has probed it.
  const attachAndSettle = fake.calls
    .filter(([command, args]) => (command === "/usr/sbin/losetup" && args.includes("--find")) || command === "/usr/bin/udevadm")
    .map(([command]) => command === "/usr/bin/udevadm" ? "settle" : "attach")
  assert.deepEqual(attachAndSettle, ["attach", "settle", "settle", "attach", "settle", "attach", "settle", "attach", "settle"])
})

test("a replaced backing file or a volume of the other kind is refused before formatting", () => {
  const replaced = fakeAdmission({ fileVolume: true })
  admitDataVolume(replaced)
  replaced.simulateReboot()
  replaced.replaceBackingFile()
  assert.throws(() => admitDataVolume(replaced), /backing file was replaced/)
  assert.equal(replaced.calls.filter(([command]) => command === "/usr/sbin/mkfs.xfs").length, 1)

  const provider = fakeAdmission()
  admitDataVolume(provider)
  const file = fakeAdmission({ fileVolume: true })
  file.io.readBinding = provider.io.readBinding
  assert.throws(() => admitDataVolume(file), /identity does not match the protected bootstrap input/)
  assert.equal(file.calls.length, 0)
})

test("a root filesystem without room for the file-backed volume refuses before creating it", () => {
  const fake = fakeAdmission({ fileVolume: true, rootFreeBytes: 10 * BYTES_PER_GIB - 1 })
  assert.throws(() => admitDataVolume(fake), /root filesystem has .* bytes free.*larger disk/)
  assert.equal(fake.calls.length, 0)
})

test("the storage input and the binding accept exactly one shape of each volume kind", () => {
  assert.deepEqual(parseStorageInput({ fileDataVolumeSizeGb: 51 }), { kind: "fileVolume", sizeGb: 51 })
  assert.deepEqual(
    parseStorageInput({ expectedDataVolumeSerial: SERIAL, expectedDataVolumeSizeGb: 10 }),
    { kind: "providerVolume", serial: SERIAL, sizeGb: 10 },
  )
  for (const invalid of [
    undefined,
    {},
    { fileDataVolumeSizeGb: 0 },
    { fileDataVolumeSizeGb: "51" },
    { fileDataVolumeSizeGb: 51, expectedDataVolumeSerial: SERIAL },
    { expectedDataVolumeSerial: "0", expectedDataVolumeSizeGb: 10 },
  ]) {
    assert.throws(() => parseStorageInput(invalid), /data-volume identity or size is invalid/)
  }
  const file = { schemaVersion: 1, backingFile: BACKING_FILE, expectedDataVolumeSizeGb: 10, filesystemUuid: UUID }
  assert.equal(parseBinding(file).kind, "fileVolume")
  assert.throws(() => parseBinding({ ...file, backingFile: { ...BACKING_FILE, sizeBytes: 1 } }), /fields are invalid/)
  assert.throws(() => parseBinding({ ...file, expectedDataVolumeSerial: SERIAL }), /unsupported shape/)
})

test("capacity mismatch and stale unbound signatures fail before formatting", () => {
  const wrongSize = fakeAdmission({ sizeBytes: 10 * BYTES_PER_GIB + 1 })
  assert.throws(() => admitDataVolume(wrongSize), /block capacity mismatch/)
  assert.equal(wrongSize.calls.some(([command]) => command === "/usr/sbin/mkfs.xfs"), false)

  const inconsistent = fakeAdmission({ blockdevSizeBytes: 10 * BYTES_PER_GIB - 1 })
  assert.throws(() => admitDataVolume(inconsistent), /lsblk and blockdev disagree/)
  assert.equal(inconsistent.calls.some(([command]) => command === "/usr/sbin/mkfs.xfs"), false)

  const stale = fakeAdmission({ initialFilesystem: "xfs" })
  assert.throws(() => admitDataVolume(stale), /not provably blank/)
  assert.equal(stale.calls.some(([command]) => command === "/usr/sbin/mkfs.xfs"), false)
})

test("rejects a volume held by another block device or process", () => {
  const held = fakeAdmission({ hasHolder: true })
  assert.throws(() => admitDataVolume(held), /in use by another block device/)
  assert.equal(held.calls.some(([command]) => command === "/usr/sbin/mkfs.xfs"), false)

  const openDevice = fakeAdmission({ busyDevice: true })
  assert.throws(() => admitDataVolume(openDevice), /in use by a process/)
  assert.equal(openDevice.calls.some(([command]) => command === "/usr/sbin/mkfs.xfs"), false)
})

test("rejects a mounted volume while any process is using its filesystem", () => {
  const fake = fakeAdmission()
  admitDataVolume(fake)
  fake.setFilesystemUse(true)
  assert.throws(() => admitDataVolume(fake), /filesystem is in use by a process/)
  assert.equal(fake.observation, undefined)
  assert.equal(fake.calls.filter(([command]) => command === "/usr/sbin/mkfs.xfs").length, 1)
})

test("missing protected storage input fails closed", () => {
  const fake = fakeAdmission()
  fake.io.readStorageInput = () => undefined
  assert.throws(() => admitDataVolume(fake), /requires a protected storage bootstrap input/)
})

test("admission runs when systemd starts it through the release symlink", async () => {
  const { mkdtempSync, symlinkSync, rmSync } = await import("node:fs")
  const { tmpdir } = await import("node:os")
  const { join, dirname } = await import("node:path")
  const { spawnSync } = await import("node:child_process")
  const { fileURLToPath } = await import("node:url")
  const root = mkdtempSync(join(tmpdir(), "admission-symlink-"))
  try {
    symlinkSync(dirname(fileURLToPath(import.meta.url)), join(root, "current"))
    const result = spawnSync(process.execPath, [join(root, "current", "chariox-data-volume-admission.mjs")], { encoding: "utf8" })
    // Without a protected bootstrap input the entrypoint must fail loudly, never exit 0 silently.
    assert.equal(result.status, 1, result.stderr)
    assert.ok(result.stderr.trim().length > 0)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test("admission sandbox never bind-mounts the data root it must find unmounted", async () => {
  const { readFile } = await import("node:fs/promises")
  const unit = await readFile(new URL("./chariox-data-volume-admission.service", import.meta.url), "utf8")
  const writable = unit.split("\n").filter(line => line.startsWith("ReadWritePaths=")).flatMap(line => line.slice(15).split(/\s+/))
  assert.equal(writable.some(path => "/var/lib/chariox-docker/data".startsWith(path)), false)
})


test("MP-02/MP-08/MP-11 read-only owner/mode drift reports an actionable mismatch", () => {
  const fake = fakeAdmission()
  admitDataVolume(fake)
  fake.setOwnership("0:0:755")
  const run = fake.commands.run
  fake.commands.run = (command, args) => {
    if (command === "/usr/bin/chown" || command === "/usr/bin/chmod") throw new Error("Read-only file system")
    return run(command, args)
  }
  assert.throws(() => admitDataVolume(fake), /data-root ownership\/mode mismatch.*stop.*repair/i)
  assert.equal(fake.observation, undefined)
})
