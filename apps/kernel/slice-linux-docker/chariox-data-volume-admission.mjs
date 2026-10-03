#!/usr/bin/env node

import { spawnSync } from "node:child_process"
import { basename } from "node:path"
import { realpathSync } from "node:fs"
import { fileURLToPath } from "node:url"
import { parseProjectQuotaState } from "./slice-disk-quota-xfs-readback.mjs"
import * as volumeDevice from "./slice-data-volume-device.mjs"
import * as protectedIo from "./slice-data-volume-protected-io.mjs"

export const {
  DATA_ROOT,
  HETZNER_BYTES_PER_GB,
  MIN_VOLUME_SIZE_GB,
  MAX_VOLUME_SIZE_GB,
  expectedVolumeBytes,
  selectHetznerVolumeDevice,
  discoverVolumeDevice,
} = volumeDevice
export const {
  PROTECTED_BOOTSTRAP_ROOT,
  MANAGED_BOOTSTRAP_PATH,
  WORKER_BOOTSTRAP_PATH,
  BINDING_PATH,
} = protectedIo

const systemIo = protectedIo.systemIo

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

const systemCommands = {
  run,
  tryRun: commandResult,
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
  commands.run("/usr/bin/systemd-mount", [
    "--no-ask-password", "--automount=no", "--fsck=no", "--type=xfs", "--options=pquota", uuidPath, DATA_ROOT,
  ])
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
  // Dependents re-run admission; a mount that predates this sandbox is read-only here.
  // Rootless dockerd resets its data root to 0710 (group is its private group).
  const current = commands.run("/usr/bin/stat", ["-c", "%u:%g:%a", DATA_ROOT])
  if ([700, 710].some((mode) => current === `${owner.uid}:${owner.gid}:${mode}`)) return
  commands.run("/usr/bin/chown", ["--", `${owner.uid}:${owner.gid}`, DATA_ROOT])
  commands.run("/usr/bin/chmod", ["0700", DATA_ROOT])
}

export function admitDataVolume({ commands = systemCommands, io = systemIo } = {}) {
  io.clearObservation()
  const input = io.readStorageInput()
  if (!input) fail("Path-1 admission requires a protected storage bootstrap input")
  const wantedBytes = expectedVolumeBytes(input.sizeGb)
  let device = volumeDevice.discoverVolumeDevice(commands, input.serial)
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
    io.writeBinding({
      schemaVersion: 1,
      expectedDataVolumeSerial: input.serial,
      expectedDataVolumeSizeGb: input.sizeGb,
      filesystemUuid,
    })
    mountDataRoot(commands, device, filesystemUuid, true)
  }
  device = volumeDevice.discoverVolumeDevice(commands, input.serial)
  if (device.sizeBytes !== wantedBytes) {
    fail(`Hetzner Volume block capacity changed during admission: expected ${wantedBytes} bytes, observed ${device.sizeBytes}`)
  }
  const verifiedMount = findDataMount(commands)
  const verifiedBinding = binding ?? io.readBinding()
  if (!assertMountMatches(verifiedMount, device, verifiedBinding, commands)) {
    fail("data volume did not mount at the fixed Docker data-root")
  }
  verifyXfsQuotaAndOverlay(commands, DATA_ROOT)
  applyDockerRootOwnership(io, commands)
  io.writeObservation({
    schemaVersion: 1,
    dataVolumeSerial: device.serial,
    dataVolumeSizeGb: device.sizeBytes / HETZNER_BYTES_PER_GB,
    filesystemUuid: verifiedBinding.filesystemUuid,
    devicePath: device.path,
    majorMinor: device.majorMinor,
    mountTarget: DATA_ROOT,
  })
  return {
    admitted: true,
    serial: input.serial,
    sizeGb: input.sizeGb,
    filesystemUuid: verifiedBinding.filesystemUuid,
  }
}

// systemd starts this through the /usr/lib/chariox/current symlink; Node reports the real path.
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const result = admitDataVolume()
    process.stdout.write(`${JSON.stringify(result)}\n`)
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
    process.exitCode = 1
  }
}
