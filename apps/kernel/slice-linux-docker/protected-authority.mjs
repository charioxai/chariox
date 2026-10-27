import { lstatSync, readFileSync } from "node:fs"
import { spawnSync } from "node:child_process"
import { readLocalDevEnrollment, verifyLocalRootfulEngine, requireLocalPrivateBarrier, verifyInstalledLocalSource, verifyLocalHelperTopology } from "./protected-local-docker-authority.mjs"
import { readNamespaceEntry, DURABLE_LAYOUT_ROOT as MANAGED_ROOT } from "./protected-namespace-entry.mjs"

const localUid = process.env.CHARIOX_SLICE_LOCAL_DEV_OWNER_UID
export const DURABLE_LAYOUT_ROOT = localUid === undefined ? MANAGED_ROOT
  : readLocalDevEnrollment(Number(localUid)).controlRoot

export function verifiedProtectedAuthority() {
  if (localUid === undefined) {
    const proof = readNamespaceEntry()
    return {kind: "managed-rootless", root: MANAGED_ROOT, controlUid: 0, dataUid: proof.dataUid, dataGid: proof.dataUid}
  }
  if (!/^(0|[1-9][0-9]{0,9})$/.test(localUid) || process.getuid() !== 0) throw new Error("Local DEV broker authority refused")
  const enrollment = readLocalDevEnrollment(Number(localUid))
  requireLocalPrivateBarrier(enrollment)
  verifyInstalledLocalSource(enrollment)
  const metadata = lstatSync("/run/docker.sock")
  const result = spawnSync("/usr/bin/docker", ["info", "--format", "{{json .}}"], {
    env: {PATH: "/usr/bin:/bin", HOME: "/tmp", DOCKER_HOST: "unix:///run/docker.sock"},
    encoding: "utf8", maxBuffer: 1024 * 1024, timeout: 30_000,
  })
  if (result.status !== 0) throw new Error("Local DEV broker authority refused")
  const helper = process.env.CHARIOX_SLICE_LOCAL_DEV_HELPER_NAME
  if (!helper || !new RegExp(`^chariox-local-broker-${enrollment.ownerUid}-[a-z0-9]+$`).test(helper)) throw new Error("Local DEV broker authority refused")
  const inspected = spawnSync("/usr/bin/docker", ["container", "inspect", helper], {
    env: {PATH: "/usr/bin:/bin", HOME: "/tmp", DOCKER_HOST: "unix:///run/docker.sock"},
    encoding: "utf8", maxBuffer: 1024 * 1024, timeout: 30_000,
  })
  if (inspected.status !== 0) throw new Error("Local DEV broker authority refused")
  const records = JSON.parse(inspected.stdout)
  if (!Array.isArray(records) || records.length !== 1) throw new Error("Local DEV broker authority refused")
  verifyLocalHelperTopology(enrollment, records[0])
  const launchSocket = JSON.parse(process.env.CHARIOX_SLICE_LOCAL_DEV_SOCKET_IDENTITY ?? "null")
  return {...verifyLocalRootfulEngine(enrollment, {...metadata, isSocket: metadata.isSocket()},
    JSON.parse(result.stdout), readFileSync("/proc/self/uid_map", "utf8"), readFileSync("/proc/self/gid_map", "utf8"), launchSocket), enrollment}
}
