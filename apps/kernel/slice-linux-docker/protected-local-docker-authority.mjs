import { constants, openSync, closeSync, fstatSync, lstatSync, readFileSync, realpathSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { createHash } from "node:crypto"

const IMAGE = /^sha256:[a-f0-9]{64}$/
const HEX = /^[a-f0-9]{64}$/
const UINT = value => Number.isSafeInteger(value) && value >= 0
function refuse() { throw new Error("Verified local Docker DEV authority is unavailable; existing identities and state are preserved") }

export function validateLocalDevEnrollment(record, uid) {
  const keys = ["version", "topology", "ownerUid", "ownerGid", "engineId", "socket", "helperImageId", "workerImageId", "workerKernelHash", "sourceDigest", "sourceRoot", "controlRoot"]
  if (!record || Object.keys(record).sort().join() !== keys.sort().join()
      || record.version !== 1 || record.topology !== "linux-local-rootful-dev"
      || !UINT(uid) || record.ownerUid !== uid || !UINT(record.ownerGid)
      || typeof record.engineId !== "string" || !/^[A-Za-z0-9][A-Za-z0-9:._-]{0,179}$/.test(record.engineId)
      || !IMAGE.test(record.helperImageId) || !IMAGE.test(record.workerImageId)
      || !HEX.test(record.workerKernelHash) || !IMAGE.test(record.sourceDigest)) refuse()
  const socket = record.socket
  if (!socket || Object.keys(socket).sort().join() !== ["path", "dev", "ino", "uid", "gid", "mode"].sort().join()
      || socket.path !== "/run/docker.sock" || ![socket.dev, socket.ino, socket.uid, socket.gid, socket.mode].every(UINT)
      || socket.uid !== 0 || (socket.mode & 0o007) !== 0 || (socket.mode & 0o600) !== 0o600) refuse()
  if (record.sourceRoot !== `/usr/lib/chariox/slice-local-dev/${record.sourceDigest.slice(7)}`
      || record.controlRoot !== `/var/lib/chariox/slice-local-dev/u-${uid}/private/layout`) refuse()
  return Object.freeze(record)
}

export function requireRootControlledPath(path, directory = false) {
  if (resolve(path) !== path || realpathSync(path) !== path) refuse()
  let parent = dirname(path)
  while (true) {
    const metadata = lstatSync(parent)
    if (!metadata.isDirectory() || metadata.isSymbolicLink() || metadata.uid !== 0 || (metadata.mode & 0o022) !== 0) refuse()
    if (parent === "/") break
    parent = dirname(parent)
  }
  const metadata = lstatSync(path)
  if (metadata.isSymbolicLink() || metadata.uid !== 0 || (metadata.mode & 0o022) !== 0
      || (directory ? !metadata.isDirectory() : !metadata.isFile() || metadata.nlink !== 1)) refuse()
  return metadata
}

export function readLocalDevEnrollment(uid) {
  if (process.platform !== "linux") refuse()
  const path = `/etc/chariox/slice-local-dev/${uid}.json`
  const expected = requireRootControlledPath(path)
  const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW)
  try {
    const actual = fstatSync(fd)
    if (actual.dev !== expected.dev || actual.ino !== expected.ino || actual.size > 64 * 1024) refuse()
    return validateLocalDevEnrollment(JSON.parse(readFileSync(fd, "utf8")), uid)
  } finally { closeSync(fd) }
}

export function verifyLocalRootfulEngine(enrollment, socket, info, uidMap, gidMap, launchSocket = enrollment.socket) {
  if (!launchSocket || !UINT(launchSocket.dev) || !UINT(launchSocket.ino)
      || !socket.isSocket || socket.dev !== launchSocket.dev || socket.ino !== launchSocket.ino
      || socket.uid !== enrollment.socket.uid || socket.gid !== enrollment.socket.gid
      || (socket.mode & 0o777) !== enrollment.socket.mode
      || info?.ID !== enrollment.engineId || info?.OSType !== "linux"
      || !Array.isArray(info.SecurityOptions)
      || info.SecurityOptions.some(option => /(?:rootless|userns)/i.test(String(option)))
      || String(uidMap).trim().replace(/\s+/g, " ") !== "0 0 4294967295"
      || String(gidMap).trim().replace(/\s+/g, " ") !== "0 0 4294967295") refuse()
  return {kind: "linux-local-rootful-dev", controlUid: 0, dataUid: 1001, dataGid: 1001,
    root: enrollment.controlRoot, sourceDigest: enrollment.sourceDigest}
}

export function requireLocalPrivateBarrier(enrollment) {
  const barrier = join(dirname(enrollment.controlRoot))
  const metadata = requireRootControlledPath(barrier, true)
  if ((metadata.mode & 0o777) !== 0o700) refuse()
  return barrier
}

export function verifyInstalledLocalSource(enrollment) {
  requireRootControlledPath(enrollment.sourceRoot, true)
  const path = join(enrollment.sourceRoot, "source-manifest.json")
  const metadata = requireRootControlledPath(path)
  if ((metadata.mode & 0o222) !== 0 || metadata.size > 1024 * 1024) refuse()
  const bytes = readFileSync(path)
  if (`sha256:${createHash("sha256").update(bytes).digest("hex")}` !== enrollment.sourceDigest) refuse()
  const manifest = JSON.parse(bytes)
  if (manifest.version !== 1 || Object.keys(manifest).sort().join() !== "files,version"
      || !Array.isArray(manifest.files) || manifest.files.length < 1 || manifest.files.length > 1000) refuse()
  const seen = new Set()
  for (const entry of manifest.files) {
    if (!entry || Object.keys(entry).sort().join() !== "path,sha256"
        || typeof entry.path !== "string" || !/^[A-Za-z0-9_.-]+(?:\/[A-Za-z0-9_.-]+)*$/.test(entry.path)
        || entry.path.split("/").some(value => value === "." || value === "..")
        || entry.path === "source-manifest.json" || seen.has(entry.path) || !HEX.test(entry.sha256)) refuse()
    seen.add(entry.path)
    const file = join(enrollment.sourceRoot, entry.path)
    const current = requireRootControlledPath(file)
    if ((current.mode & 0o222) !== 0 || (entry.path === ".local-public-tools/node" && !(current.mode & 0o111)) || current.size > (entry.path === ".local-public-tools/node" ? 128 : entry.path === ".local-public-tools/docker" ? 64 : 16) * 1024 * 1024
        || createHash("sha256").update(readFileSync(file)).digest("hex") !== entry.sha256) refuse()
  }
  return manifest
}

export function verifyLocalHelperTopology(enrollment, info) {
  if (!info || info.Image !== enrollment.helperImageId || info.Config?.User !== "0:0"
      || info.HostConfig?.Privileged !== false || info.HostConfig?.ReadonlyRootfs !== true
      || info.HostConfig?.NetworkMode !== "none" || info.HostConfig?.PidMode
      || info.HostConfig?.IpcMode === "host" || info.HostConfig?.UsernsMode
      || JSON.stringify(info.HostConfig?.CapDrop?.map(value => value.toUpperCase()).sort()) !== '["ALL"]'
      || JSON.stringify(info.HostConfig?.CapAdd?.map(value => value.toUpperCase()).sort()) !== '["CHOWN","DAC_OVERRIDE","FOWNER"]'
      || !info.HostConfig?.SecurityOpt?.includes("no-new-privileges")
      || JSON.stringify(Object.entries(info.HostConfig?.Tmpfs ?? {}).sort()) !== JSON.stringify([
        ["/run/chariox-slice-broker", "rw,nosuid,nodev,size=32m"], ["/tmp", "rw,nosuid,nodev,size=128m"],
      ])
      || !Array.isArray(info.Mounts) || info.Mounts.length !== 5) refuse()
  const required = new Map([
    ["/run/docker.sock", {source: "/run/docker.sock", rw: true}],
    [enrollment.sourceRoot, {source: enrollment.sourceRoot, rw: false}],
    [`/etc/chariox/slice-local-dev/${enrollment.ownerUid}.json`, {source: `/etc/chariox/slice-local-dev/${enrollment.ownerUid}.json`, rw: false}],
    [dirname(enrollment.controlRoot), {source: dirname(enrollment.controlRoot), rw: true}],
  ])
  let transport = false
  for (const mount of info.Mounts) {
    const expected = required.get(mount.Destination)
    if (expected) {
      if (mount.Type !== "bind" || mount.Source !== expected.source || mount.RW !== expected.rw) refuse()
      required.delete(mount.Destination)
    } else {
      const prefix = `/tmp/chariox-local-broker-${enrollment.ownerUid}-`
      if (transport || mount.Type !== "bind" || !mount.Destination.startsWith(prefix)
          || !/^[A-Za-z0-9]+$/.test(mount.Destination.slice(prefix.length))
          || mount.Source !== mount.Destination || mount.RW !== true) refuse()
      transport = true
    }
  }
  if (required.size || !transport) refuse()
  return true
}
