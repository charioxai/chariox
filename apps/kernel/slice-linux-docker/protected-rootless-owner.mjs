import { readFileSync, lstatSync } from "node:fs"
import { spawnSync } from "node:child_process"
export const SLICE_CONTAINER_UID = 1001
function refuse() { throw new Error("Protected slice storage requires a verified managed rootless Docker mapping") }
export function mappedSliceOwner({uidMap, gidMap, daemonUid, daemonGid, subuids, subgids, processUid}) {
  if (!Number.isInteger(daemonUid) || !Number.isInteger(daemonGid) || processUid !== daemonUid) refuse()
  const parse = text => String(text).trim().split("\n").map(line => line.trim().split(/\s+/).map(Number))
  const verifyMap = (text, daemonId, ranges) => {
    const rows = parse(text)
    if (rows.length !== 2 || rows.some(row => row.length !== 3 || row.some(value => !Number.isSafeInteger(value) || value < 0))) refuse()
    if (rows[0].join(":") !== `0:${daemonId}:1` || rows[1].join(":") !== "1:231072:65536") refuse()
    if (String(ranges).trim().split("\n").filter(line => line.startsWith("chariox-docker:")).join("\n") !== "chariox-docker:231072:65536") refuse()
    return 231072 + SLICE_CONTAINER_UID - 1
  }
  const uid = verifyMap(uidMap, daemonUid, subuids)
  const gid = verifyMap(gidMap, daemonGid, subgids)
  if (uid !== gid) refuse()
  return uid
}
export function managedRootlessSliceOwner() {
  if (process.getuid() !== 0) refuse()
  const id = option => {
    const result = spawnSync("/usr/bin/id", [option, "chariox-docker"], {encoding: "utf8", timeout: 5000})
    if (result.status !== 0 || !/^\d+\s*$/.test(result.stdout)) refuse()
    return Number(result.stdout.trim())
  }
  const daemonUid = id("-u"), daemonGid = id("-g")
  const state = "/run/chariox-docker/dockerd-rootless/child_pid"
  const metadata = lstatSync(state)
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.uid !== daemonUid || (metadata.mode & 0o022) !== 0 || metadata.size > 64) refuse()
  const pid = readFileSync(state, "utf8").trim()
  if (!/^[1-9][0-9]*$/.test(pid)) refuse()
  const status = readFileSync(`/proc/${pid}/status`, "utf8")
  const processUid = Number(status.match(/^Uid:\s+(\d+)\s/m)?.[1])
  if (!lstatSync(`/proc/${pid}/ns/user`).isSymbolicLink()) refuse()
  return mappedSliceOwner({daemonUid, daemonGid, processUid,
    uidMap: readFileSync(`/proc/${pid}/uid_map`, "utf8"), gidMap: readFileSync(`/proc/${pid}/gid_map`, "utf8"),
    subuids: readFileSync("/etc/subuid", "utf8"), subgids: readFileSync("/etc/subgid", "utf8")})
}
