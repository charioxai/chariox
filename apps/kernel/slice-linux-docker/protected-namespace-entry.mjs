import { constants, fstatSync, lstatSync, statSync, readFileSync, openSync, closeSync, writeFileSync, fsyncSync, renameSync, mkdirSync, chmodSync, realpathSync } from "node:fs"
import { join, parse } from "node:path"
import { spawnSync } from "node:child_process"

export const DURABLE_LAYOUT_ROOT = "/var/lib/chariox-docker/private-layout"
export const MANAGED_ARCHIVE_ROOT = "/var/lib/chariox-slice-share/.broker-private/artifacts"
const VERIFIED_ROOTS = [DURABLE_LAYOUT_ROOT, MANAGED_ARCHIVE_ROOT]
const ENTRY_PATH = join(DURABLE_LAYOUT_ROOT, "namespace-entry.json")
export const SLICE_CONTAINER_UID = 1001
function refuse() { throw new Error("Protected slice storage requires a verified managed rootless namespace") }

export function mappedSliceOwner({uidMap, gidMap, daemonUid, daemonGid, subuids, subgids, processUid}) {
  if (!Number.isInteger(daemonUid) || !Number.isInteger(daemonGid) || processUid !== daemonUid) refuse()
  const parseMap = text => String(text).trim().split("\n").map(line => line.trim().split(/\s+/).map(Number))
  function verify(text, daemonId, ranges) {
    const rows = parseMap(text)
    if (rows.length !== 2 || rows.some(row => row.length !== 3 || row.some(value => !Number.isSafeInteger(value) || value < 0))) refuse()
    if (rows[0].join(":") !== `0:${daemonId}:1` || rows[1].join(":") !== "1:231072:65536") refuse()
    if (String(ranges).trim().split("\n").filter(line => line.startsWith("chariox-docker:")).join("\n") !== "chariox-docker:231072:65536") refuse()
    return 231072 + SLICE_CONTAINER_UID - 1
  }
  const uid = verify(uidMap, daemonUid, subuids)
  if (uid !== verify(gidMap, daemonGid, subgids)) refuse()
  return uid
}
function identity(path) {
  const metadata = statSync(path, {bigint: true})
  return {dev: String(metadata.dev), ino: String(metadata.ino)}
}
function ancestry(root, daemonUid) {
  if (realpathSync(root) !== root) refuse()
  const records = []
  let path = parse(root).root
  for (const part of ["", ...root.slice(path.length).split("/").filter(Boolean)]) {
    if (part) path = join(path, part)
    const metadata = lstatSync(path)
    if (!metadata.isDirectory() || metadata.isSymbolicLink() || (metadata.mode & 0o022)
        || ![0, daemonUid].includes(metadata.uid)) refuse()
    records.push({path, ...identity(path), hostUid: metadata.uid, mode: metadata.mode & 0o777})
  }
  return records
}

// Runs as the dedicated daemon user BEFORE entering its user namespace. Public
// metadata only; it neither inspects nor creates runtime identities or backups.
export function prepareNamespaceEntry(targetPid) {
  if (!/^[1-9][0-9]*$/.test(String(targetPid)) || process.getuid() === 0) refuse()
  const id = option => {
    const result = spawnSync("/usr/bin/id", [option, "chariox-docker"], {encoding: "utf8", timeout: 5000})
    if (result.status !== 0 || !/^\d+\s*$/.test(result.stdout)) refuse()
    return Number(result.stdout.trim())
  }
  const daemonUid = id("-u"), daemonGid = id("-g")
  if (process.getuid() !== daemonUid || process.getgid() !== daemonGid) refuse()
  const pidPath = "/run/chariox-docker/dockerd-rootless/child_pid"
  const pidMetadata = lstatSync(pidPath)
  if (!pidMetadata.isFile() || pidMetadata.isSymbolicLink() || pidMetadata.uid !== daemonUid
      || (pidMetadata.mode & 0o022) || pidMetadata.size > 64 || readFileSync(pidPath, "utf8").trim() !== String(targetPid)) refuse()
  const proc = `/proc/${targetPid}`
  const processUid = Number(readFileSync(`${proc}/status`, "utf8").match(/^Uid:\s+(\d+)\s/m)?.[1])
  const maps = {uidMap: readFileSync(`${proc}/uid_map`, "utf8"), gidMap: readFileSync(`${proc}/gid_map`, "utf8"),
    daemonUid, daemonGid, processUid, subuids: readFileSync("/etc/subuid", "utf8"), subgids: readFileSync("/etc/subgid", "utf8")}
  const hostDataUid = mappedSliceOwner(maps)
  // Exact mode despite the service umask; the check below requires 0711.
  try { mkdirSync(DURABLE_LAYOUT_ROOT, {mode: 0o711}); chmodSync(DURABLE_LAYOUT_ROOT, 0o711) } catch (error) { if (error.code !== "EEXIST") throw error }
  const ancestors = ancestry(DURABLE_LAYOUT_ROOT, daemonUid)
  if (ancestors.at(-1).hostUid !== daemonUid || ancestors.at(-1).mode !== 0o711) refuse()
  const sinkAncestors = ancestry(MANAGED_ARCHIVE_ROOT, daemonUid)
  if (sinkAncestors.at(-1).hostUid !== daemonUid || sinkAncestors.at(-1).mode !== 0o700) refuse()
  const sharedRoot = sinkAncestors.find(anchor => anchor.path === "/var/lib/chariox-slice-share")
  const brokerRoot = sinkAncestors.find(anchor => anchor.path === "/var/lib/chariox-slice-share/.broker-private")
  if (sharedRoot?.hostUid !== 0 || sharedRoot.mode !== 0o710 || brokerRoot?.hostUid !== 0 || brokerRoot.mode !== 0o711) refuse()
  const verifiedAncestors = [...new Map([...ancestors, ...sinkAncestors].map(anchor => [anchor.path, anchor])).values()]
  const receipt = {version: 1, daemonUid, daemonGid, dataUid: SLICE_CONTAINER_UID, hostDataUid,
    uidMap: maps.uidMap, gidMap: maps.gidMap, ancestors: verifiedAncestors,
    namespaces: Object.fromEntries(["user", "mnt", "net"].map(name => [name, identity(`${proc}/ns/${name}`)]))}
  const temporary = `${ENTRY_PATH}.${process.pid}.pending`
  const fd = openSync(temporary, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600)
  try { writeFileSync(fd, JSON.stringify(receipt)); fsyncSync(fd) } finally { closeSync(fd) }
  renameSync(temporary, ENTRY_PATH)
  const directory = openSync(DURABLE_LAYOUT_ROOT, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW)
  try { fsyncSync(directory) } finally { closeSync(directory) }
}

export function verifyNamespaceAnchorDocuments(receipt) {
  const expected = new Set()
  for (const root of VERIFIED_ROOTS) {
    let path = parse(root).root
    expected.add(path)
    for (const part of root.slice(path.length).split("/").filter(Boolean)) { path = join(path, part); expected.add(path) }
  }
  if (!Array.isArray(receipt.ancestors) || receipt.ancestors.length !== expected.size) refuse()
  for (const anchor of receipt.ancestors) {
    if (!expected.delete(anchor.path) || !/^[0-9]+$/.test(anchor.dev) || !/^[1-9][0-9]*$/.test(anchor.ino)
        || ![0, receipt.daemonUid].includes(anchor.hostUid) || !Number.isInteger(anchor.mode)
        || anchor.mode < 0 || anchor.mode > 0o777 || (anchor.mode & 0o022)) refuse()
  }
  const required = [[DURABLE_LAYOUT_ROOT, receipt.daemonUid, 0o711], [MANAGED_ARCHIVE_ROOT, receipt.daemonUid, 0o700],
    ["/var/lib/chariox-slice-share", 0, 0o710], ["/var/lib/chariox-slice-share/.broker-private", 0, 0o711]]
  for (const [path, uid, mode] of required) {
    const anchor = receipt.ancestors.find(value => value.path === path)
    if (anchor?.hostUid !== uid || anchor.mode !== mode) refuse()
  }
  return receipt
}

export function verifyNamespaceEntryDocuments(receipt, current) {
  if (receipt?.version !== 1 || current.uid !== 0 || current.gid !== 0 || receipt.dataUid !== SLICE_CONTAINER_UID
      || !Number.isInteger(receipt.daemonUid) || receipt.daemonUid <= 0
      || !Number.isInteger(receipt.daemonGid) || receipt.daemonGid <= 0
      || current.uidMap !== receipt.uidMap || current.gidMap !== receipt.gidMap) refuse()
  const mapped = mappedSliceOwner({uidMap: receipt.uidMap, gidMap: receipt.gidMap,
    daemonUid: receipt.daemonUid, daemonGid: receipt.daemonGid, processUid: receipt.daemonUid,
    subuids: current.subuids, subgids: current.subgids})
  if (mapped !== receipt.hostDataUid || !Array.isArray(receipt.ancestors) || receipt.ancestors.length < 3) refuse()
  verifyNamespaceAnchorDocuments(receipt)
  for (const name of ["user", "mnt", "net"]) {
    if (JSON.stringify(receipt.namespaces?.[name]) !== JSON.stringify(current.namespaces?.[name])) refuse()
  }
  return receipt
}
export function readNamespaceEntry() {
  const directory = lstatSync(DURABLE_LAYOUT_ROOT)
  if (!directory.isDirectory() || directory.isSymbolicLink() || directory.uid !== 0 || (directory.mode & 0o777) !== 0o711) refuse()
  const fd = openSync(ENTRY_PATH, constants.O_RDONLY | constants.O_NOFOLLOW)
  let receipt
  try {
    const metadata = fstatSync(fd)
    if (!metadata.isFile() || metadata.nlink !== 1 || metadata.uid !== 0 || (metadata.mode & 0o077) || metadata.size > 64 * 1024) refuse()
    receipt = JSON.parse(readFileSync(fd, "utf8"))
  } finally { closeSync(fd) }
  verifyNamespaceEntryDocuments(receipt, {uid: process.getuid(), gid: process.getgid(),
    uidMap: readFileSync("/proc/self/uid_map", "utf8"), gidMap: readFileSync("/proc/self/gid_map", "utf8"),
    subuids: readFileSync("/etc/subuid", "utf8"), subgids: readFileSync("/etc/subgid", "utf8"),
    namespaces: Object.fromEntries(["user", "mnt", "net"].map(name => [name, identity(`/proc/self/ns/${name}`)]))})
  for (const anchor of receipt.ancestors) {
    const metadata = lstatSync(anchor.path)
    const mappedOwner = anchor.hostUid === receipt.daemonUid ? 0 : 65534
    if (!metadata.isDirectory() || metadata.isSymbolicLink() || metadata.uid !== mappedOwner
        || (metadata.mode & 0o777) !== anchor.mode || anchor.mode & 0o022
        || ![0, receipt.daemonUid].includes(anchor.hostUid)
        || JSON.stringify(identity(anchor.path)) !== JSON.stringify({dev: anchor.dev, ino: anchor.ino})) refuse()
  }
  return receipt
}
export function isVerifiedHostAncestor(path, metadata) {
  return matchesVerifiedHostAncestor(readNamespaceEntry(), path, metadata)
}
export function matchesVerifiedHostAncestor(receipt, path, metadata) {
  const anchor = receipt.ancestors.find(record => record.path === path)
  return anchor?.hostUid === 0 && metadata.uid === 65534
    && String(metadata.dev) === anchor.dev && String(metadata.ino) === anchor.ino
    && (metadata.mode & 0o777) === anchor.mode
}
if (process.argv[1]?.endsWith("/protected-namespace-entry.mjs")) {
  try { if (process.argv[2] !== "--prepare") refuse(); prepareNamespaceEntry(process.argv[3]) }
  catch { console.error("Managed rootless namespace proof is unavailable; private slice state is unchanged"); process.exitCode = 1 }
}
