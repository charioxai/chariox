import { managedRootlessSliceOwner, SLICE_CONTAINER_UID } from "./protected-rootless-owner.mjs"
import { existsSync, mkdirSync, readdirSync } from "node:fs"
import { join } from "node:path"
import { spawnSync } from "node:child_process"
import { verifyPrivateHostDirectory } from "./protected-host-root.mjs"
import { requireManagedImageProof } from "./protected-image-proof.mjs"
import { verifyProtectedCaptureLayout } from "./protected-layout.mjs"
import { retainFreshIdentity, requireIdentityRetention } from "./protected-identity-retention.mjs"

export const DURABLE_LAYOUT_ROOT = "/var/lib/chariox-docker/private-layout"
function refuse() { throw new Error("Protected identity retention is required before this slice can start") }

export function ensureFirstBootRetention({privateRoot, backupRoot, sliceId, container, port, docker, dataOwner, containerUid = SLICE_CONTAINER_UID}) {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,179}$/.test(sliceId)
      || !/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,179}$/.test(container)
      || !Number.isInteger(port) || port < 1 || port > 65535) refuse()
  verifyPrivateHostDirectory(privateRoot, dataOwner)
  verifyPrivateHostDirectory(backupRoot, process.getuid())
  if (existsSync(join(backupRoot, `${sliceId}.json`))) {
    return requireIdentityRetention({privateRoot, backupRoot, sliceId, dataOwner})
  }
  // Any prior or partial initialization requires restoration, never new keys.
  if (readdirSync(join(privateRoot, "kernel/kernels")).length !== 0
      || existsSync(join(privateRoot, "kernel/machine/identity.json"))) refuse()
  const result = docker(["exec", "-u", String(containerUid),
    "-e", "CHARIOX_HOME=/var/lib/chariox/slice-private/kernel",
    "-e", "CHARIOX_SLICE_PRIVATE_ROOT=/var/lib/chariox/slice-private", container,
    "/opt/chariox-slice/bin/chariox-kernel", "--prepare-protected-slice-identity", String(port)])
  if (result.status !== 0) refuse()
  // Do not trust or relay child output. Verify the actual protected files instead.
  return retainFreshIdentity({privateRoot, backupRoot, sliceId, dataOwner})
}

if (process.argv[1]?.endsWith("/protected-first-boot.mjs")) {
  try {
    if (process.getuid() !== 0) refuse()
    const environment = process.env
    const sliceId = environment.CHARIOX_SLICE_ID
    const privateRoot = environment.CHARIOX_SLICE_PRIVATE_HOST_ROOT
    if (!sliceId || privateRoot !== join(DURABLE_LAYOUT_ROOT, "homes", sliceId)) refuse()
    const backupRoot = join(DURABLE_LAYOUT_ROOT, "backups")
    verifyPrivateHostDirectory(DURABLE_LAYOUT_ROOT, 0, true)
    try { mkdirSync(backupRoot, {mode: 0o700}) } catch (error) { if (error.code !== "EEXIST") throw error }
    const docker = args => spawnSync("/usr/bin/docker", args, {
      env: {HOME: "/var/lib/chariox-docker/home", PATH: "/usr/bin:/bin", DOCKER_HOST: environment.DOCKER_HOST},
      timeout: 60_000, maxBuffer: 1024 * 1024,
    })
    const inspected = docker(["container", "inspect", environment.CHARIOX_SLICE_NAME])
    if (inspected.status !== 0) refuse()
    const records = JSON.parse(inspected.stdout)
    if (!Array.isArray(records) || records.length !== 1) refuse()
    const info = records[0]
    requireManagedImageProof(join(DURABLE_LAYOUT_ROOT, "images"), environment.CHARIOX_SLICE_BUILD_CONTEXT_DIGEST, info.Image)
    verifyProtectedCaptureLayout(info, {version: 1, baseImageId: info.Image, imageId: info.Image,
      containerId: info.Id, privateHostRoot,
      homeVolume: environment.CHARIOX_SLICE_HOME_VOLUME ?? `${environment.CHARIOX_SLICE_NAME}-home`}, new Set([info.Image]))
    if (environment.DOCKER_HOST !== "unix:///run/chariox-docker/docker.sock") refuse()
    const dataOwner = managedRootlessSliceOwner()
    ensureFirstBootRetention({privateRoot, backupRoot, sliceId, dataOwner,
      container: environment.CHARIOX_SLICE_NAME, port: Number(environment.CHARIOX_SLICE_KERNEL_PORT ?? 43119),
      docker})
  } catch {
    console.error("Protected identity retention is required before this slice can start; existing identity is preserved")
    process.exitCode = 1
  }
}
