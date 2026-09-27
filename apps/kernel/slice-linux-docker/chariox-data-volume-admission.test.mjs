import assert from "node:assert/strict"
import test from "node:test"

import {
  DATA_ROOT,
  HETZNER_BYTES_PER_GB,
  admitDataVolume,
  expectedVolumeBytes,
  selectHetznerVolumeDevice,
} from "./chariox-data-volume-admission.mjs"

const SERIAL = "12345"
const UUID = "12345678-1234-1234-1234-123456789abc"

function makeDevices({ sizeBytes = 10 * HETZNER_BYTES_PER_GB, filesystem = "", uuid = "", mounted = false } = {}) {
  return [
    {
      path: "/dev/vda",
      type: "disk",
      model: "System disk",
      size: 80 * HETZNER_BYTES_PER_GB,
      children: [{ path: "/dev/vda1", type: "part", pkname: "vda", size: 80 * HETZNER_BYTES_PER_GB }],
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
      children: [],
    },
  ]
}

function fakeAdmission({
  initialFilesystem = "",
  sizeBytes = 10 * HETZNER_BYTES_PER_GB,
  blockdevSizeBytes = sizeBytes,
  busyDevice = false,
  busyFilesystem = false,
  hasHolder = false,
} = {}) {
  let filesystem = initialFilesystem
  let filesystemUuid = initialFilesystem ? UUID : ""
  let mounted = false
  let binding
  const calls = []
  const commands = {
    run(command, args) {
      calls.push([command, args])
      if (command === "/usr/bin/lsblk" && args.includes("--json")) {
        return JSON.stringify({ blockdevices: makeDevices({ sizeBytes, filesystem, uuid: filesystemUuid, mounted }) })
      }
      if (command === "/usr/bin/findmnt" && args.includes("--target")) return "/dev/vda1"
      if (command === "/usr/bin/realpath") {
        return args[1].startsWith("/dev/disk/by-uuid/") ? "/dev/sdb" : args[1]
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
      if (command === "/usr/bin/udevadm") return ""
      if (command === "/usr/bin/systemd-mount") {
        mounted = true
        return ""
      }
      if (command === "/usr/sbin/xfs_quota") {
        return `Project quota state on ${DATA_ROOT} (/dev/sdb)\n  Accounting: ON\n  Enforcement: ON\n`
      }
      if (command === "/usr/sbin/xfs_info") return "meta-data=/dev/sdb isize=512 agcount=4, ftype=1\n"
      if (command === "/usr/bin/chown" || command === "/usr/bin/chmod") return ""
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
                  source: "/dev/sdb",
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
    readStorageInput: () => ({ role: "managed", serial: SERIAL, sizeGb: 10 }),
    readBinding: () => binding,
    writeBinding: (value) => { binding = value },
    ensureMountpoint: () => true,
    holders: () => hasHolder,
    dockerOwner: () => ({ uid: 1001, gid: 1001 }),
  }
  return {
    commands,
    io,
    calls,
    simulateReboot() { mounted = false },
    setFilesystemUse(value) { busyFilesystem = value },
    get binding() { return binding },
  }
}

test("Hetzner API GB maps to exact binary block bytes", () => {
  assert.equal(expectedVolumeBytes(10), 10 * 1024 ** 3)
  assert.throws(() => expectedVolumeBytes(9), /supported integer-GB range/)
  assert.throws(() => expectedVolumeBytes(Number.MAX_SAFE_INTEGER), /supported integer-GB range/)
})

test("selects one matching whole Hetzner Volume disk apart from the root disk", () => {
  const device = selectHetznerVolumeDevice(makeDevices(), SERIAL, "/dev/vda1")
  assert.equal(device.path, "/dev/sdb")
  assert.equal(device.serial, SERIAL)
  assert.equal(device.sizeBytes, 10 * HETZNER_BYTES_PER_GB)
})

test("rejects duplicate volumes, partitions, mounts, and the root disk", () => {
  const devices = makeDevices()
  assert.throws(
    () => selectHetznerVolumeDevice([...devices, { ...devices[1], path: "/dev/sdc", serial: "54321" }], SERIAL, "/dev/vda1"),
    /exactly one Hetzner Volume disk/,
  )
  const partitioned = makeDevices()
  partitioned[1].children = [{ path: "/dev/sdb1", type: "part", pkname: "sdb" }]
  assert.throws(() => selectHetznerVolumeDevice(partitioned, SERIAL, "/dev/vda1"), /partitions or child devices/)
  assert.throws(
    () => selectHetznerVolumeDevice(makeDevices({ mounted: true }), SERIAL, "/dev/vda1"),
    /already mounted/,
  )
  assert.throws(() => selectHetznerVolumeDevice(makeDevices(), SERIAL, "/dev/sdb"), /root filesystem disk/)
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
  assert.equal(fake.calls.filter(([command]) => command === "/usr/sbin/mkfs.xfs").length, 1)
  assert.equal(fake.calls.filter(([command]) => command === "/usr/bin/systemd-mount").length, 1)
  assert.ok(fake.calls.some(([command]) => command === "/usr/sbin/xfs_quota"))

  fake.simulateReboot()
  const rebootResult = admitDataVolume(fake)
  assert.deepEqual(rebootResult, result)
  assert.equal(fake.calls.filter(([command]) => command === "/usr/sbin/mkfs.xfs").length, 1)
  assert.equal(fake.calls.filter(([command]) => command === "/usr/bin/systemd-mount").length, 2)
})

test("capacity mismatch and stale unbound signatures fail before formatting", () => {
  const wrongSize = fakeAdmission({ sizeBytes: 10 * HETZNER_BYTES_PER_GB + 1 })
  assert.throws(() => admitDataVolume(wrongSize), /block capacity mismatch/)
  assert.equal(wrongSize.calls.some(([command]) => command === "/usr/sbin/mkfs.xfs"), false)

  const inconsistent = fakeAdmission({ blockdevSizeBytes: 10 * HETZNER_BYTES_PER_GB - 1 })
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
  assert.equal(fake.calls.filter(([command]) => command === "/usr/sbin/mkfs.xfs").length, 1)
})

test("missing protected storage input fails closed", () => {
  const fake = fakeAdmission()
  fake.io.readStorageInput = () => undefined
  assert.throws(() => admitDataVolume(fake), /requires a protected storage bootstrap input/)
})
