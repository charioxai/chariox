import { managedRootlessSliceOwner, SLICE_CONTAINER_UID } from "./protected-rootless-owner.mjs"
import { existsSync, mkdirSync, readdirSync } from "node:fs"
import { join } from "node:path"
import { spawnSync } from "node:child_process"
import { verifyPrivateHostDirectory } from "./protected-host-root.mjs"
import { requireManagedImageProof, requireManagedRuntimeHash } from "./protected-image-proof.mjs"
import { requireRuntimeProof } from "./protected-runtime-proof.mjs"
import { verifyProtectedCaptureLayout } from "./protected-layout.mjs"
import { retainFreshIdentity, requireIdentityRetention } from "./protected-identity-retention.mjs"

export const DURABLE_LAYOUT_ROOT = "/var/lib/chariox-docker/private-layout"
function refuse() { throw new Error("Protected identity retention is required before this slice can start") }

export function verifyFirstBootTopology(info, privateRoot, homeVolume) {
  return verifyProtectedCaptureLayout(info, {version: 1, baseImageId: info.Image, imageId: info.Image,
    containerId: info.Id, privateHostRoot: privateRoot, homeVolume}, new Set([info.Image]))
}

function publishBootPin(docker, container, receipt) {
  const pin = Object.fromEntries(["kernelId", "machineId", "relayPublicKey", "host", "port", "restorationVerified"].map(key => [key, receipt[key]]))
  const script = `import {openSync,writeFileSync,fsyncSync,closeSync,renameSync,constants} from 'node:fs';
    const path='/opt/chariox-slice/identity-retention.json';
    const temporary=path+'.pending';
    const fd=openSync(temporary,constants.O_WRONLY|constants.O_CREAT|constants.O_EXCL|constants.O_NOFOLLOW,0o444);
    try {writeFileSync(fd,await new Promise(resolve=>{let value='';process.stdin.setEncoding('utf8');process.stdin.on('data',chunk=>value+=chunk);process.stdin.on('end',()=>resolve(value))}));fsyncSync(fd)} finally {closeSync(fd)};
    renameSync(temporary,path);
    const directory=openSync('/opt/chariox-slice',constants.O_RDONLY|constants.O_DIRECTORY|constants.O_NOFOLLOW);try{fsyncSync(directory)}finally{closeSync(directory)};`
  if (docker(["exec", "-i", "-u", "0", container, "/usr/bin/node", "--input-type=module", "-e", script], {input: JSON.stringify(pin)}).status !== 0) refuse()
  return receipt
}

export function ensureFirstBootRetention({privateRoot, backupRoot, sliceId, container, port, docker, dataOwner, containerUid = SLICE_CONTAINER_UID}) {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,179}$/.test(sliceId)
      || !/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,179}$/.test(container)
      || !Number.isInteger(port) || port < 1 || port > 65535) refuse()
  verifyPrivateHostDirectory(privateRoot, dataOwner)
  verifyPrivateHostDirectory(backupRoot, process.getuid())
  if (existsSync(join(backupRoot, `${sliceId}.json`))) {
    return publishBootPin(docker, container, requireIdentityRetention({privateRoot, backupRoot, sliceId, dataOwner, port}))
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
  return publishBootPin(docker, container, retainFreshIdentity({privateRoot, backupRoot, sliceId, dataOwner, port}))
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
    const docker = (args, options = {}) => spawnSync("/usr/bin/docker", args, {
      env: {HOME: "/var/lib/chariox-docker/home", PATH: "/usr/bin:/bin", DOCKER_HOST: environment.DOCKER_HOST},
      timeout: 60_000, maxBuffer: 1024 * 1024, input: options.input,
    })
    const inspected = docker(["container", "inspect", environment.CHARIOX_SLICE_NAME])
    if (inspected.status !== 0) refuse()
    const records = JSON.parse(inspected.stdout)
    if (!Array.isArray(records) || records.length !== 1) refuse()
    const info = records[0]
    requireManagedImageProof(join(DURABLE_LAYOUT_ROOT, "images"), environment.CHARIOX_SLICE_BUILD_CONTEXT_DIGEST, info.Image)
    verifyFirstBootTopology(info, privateRoot, environment.CHARIOX_SLICE_HOME_VOLUME ?? `${environment.CHARIOX_SLICE_NAME}-home`)
    const kernelHash = requireManagedRuntimeHash(join(DURABLE_LAYOUT_ROOT, "images"), environment.CHARIOX_SLICE_BUILD_CONTEXT_DIGEST, info.Image)
    requireRuntimeProof(docker, environment.CHARIOX_SLICE_NAME, kernelHash)
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
