import { spawn, spawnSync } from "node:child_process"
import { openSync, closeSync, fstatSync, constants, createReadStream } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { pipeline } from "node:stream/promises"
import { DURABLE_LAYOUT_ROOT, verifiedProtectedAuthority } from "./protected-authority.mjs"
import { createHomeGenerationStore } from "./protected-home-generation.mjs"

// Reasons name the failed check only; they never carry archive contents or private paths.
export class RestoreRefusal extends Error {
  constructor(reason) {
    super(`Saved slice home cannot be restored safely: ${reason}; existing identity and saved state are preserved`)
    this.reason = reason
  }
}
function refuse(reason) { throw new RestoreRefusal(reason) }
export function verifyFreshRestoreTarget(inspect, helper, volume, inventory) {
  if (inspect.Config?.Labels?.["io.chariox.home-restore-helper"] !== helper
      || inspect.HostConfig?.NetworkMode !== "none" || inspect.Mounts?.length !== 1
      || inspect.Mounts[0].Type !== "volume" || inspect.Mounts[0].Name !== volume
      || inspect.Mounts[0].Destination !== "/home-dst" || inspect.Mounts[0].RW !== true
      || Buffer.from(inventory).length !== 0) refuse("restore target is not a fresh, empty, owned home volume")
}
function waitForChild(child, label) {
  return new Promise((resolve, reject) => {
    child.once("error", error => reject(new RestoreRefusal(`${label} could not start (${error.code ?? "error"})`)))
    child.once("close", (code, signal) => code === 0 ? resolve()
      : reject(new RestoreRefusal(`${label} exited with ${signal ?? `status ${code}`}`)))
  })
}
export async function validateCompressedArchive(fd, helper, environment, abort = () => {}) {
  const decoder = spawn("/usr/bin/docker", ["exec", "-i", "-u", "root", helper, "zstd", "-dc"],
    {env: environment, stdio: ["pipe", "pipe", "ignore"]})
  const validator = spawn("/usr/bin/python3", [join(dirname(fileURLToPath(import.meta.url)), "validate-home-archive.py")],
    {env: {PATH: "/usr/bin:/bin", PYTHONDONTWRITEBYTECODE: "1"}, stdio: ["pipe", "ignore", "ignore"]})
  const children = [decoder, validator]
  const completion = [waitForChild(decoder, "archive decompression"), waitForChild(validator, "archive validation")]
  let timedOut = false
  const timer = setTimeout(() => { timedOut = true; abort(); children.forEach(child => child.kill("SIGKILL")) }, 10 * 60_000)
  try {
    await Promise.all([pipeline(createReadStream("unused", {fd, autoClose: false, start: 0}), decoder.stdin),
      pipeline(decoder.stdout, validator.stdin), ...completion])
  } catch (error) {
    if (timedOut) refuse("archive validation exceeded 10 minutes")
    throw error
  } finally {
    clearTimeout(timer)
    children.forEach(child => { if (child.exitCode === null) child.kill("SIGKILL") })
    await Promise.allSettled(completion)
  }
}
export function requireRestoreVolumeReserve(docker, helper, reserveBytes = 10n * 1024n ** 3n) {
  const result = docker(["exec", "-u", "root", helper, "/usr/bin/stat", "-f", "-c", "%a %S", "/home-dst"])
  const text = Buffer.from(result.stdout ?? "").toString("utf8").trim()
  if (result.status !== 0 || !/^[0-9]+ [0-9]+$/.test(text)) refuse("restore volume free space is unreadable")
  const [available, blockSize] = text.split(" ").map(BigInt)
  if (blockSize <= 0n) refuse("restore volume free space is unreadable")
  if (available * blockSize < reserveBytes) refuse(`restore volume has less than ${reserveBytes / 1024n ** 2n} MiB free`)
}
async function extractCompressedArchive(fd, helper, environment, abort) {
  const child = spawn("/usr/bin/docker", ["exec", "-i", "-u", "root", helper,
    "tar", "--zstd", "--no-same-owner", "--no-same-permissions", "-xf", "-", "-C", "/home-dst"],
    {env: environment, stdio: ["pipe", "ignore", "ignore"]})
  const completion = waitForChild(child, "archive extraction")
  let stopReason
  const stop = reason => { stopReason ??= reason; abort(); child.kill("SIGKILL") }
  const timer = setTimeout(() => stop("archive extraction exceeded 10 minutes"), 10 * 60_000)
  const reserve = setInterval(() => {
    try {
      requireRestoreVolumeReserve(args => spawnSync("/usr/bin/docker", args, {env: environment, timeout: 5_000, maxBuffer: 1024}), helper)
    } catch (error) { stop(`during extraction, ${error.reason ?? "the restore volume reserve check failed"}`) }
  }, 200)
  try { await Promise.all([pipeline(createReadStream("unused", {fd, autoClose: false, start: 0}), child.stdin), completion]) }
  catch (error) {
    if (stopReason) refuse(stopReason)
    throw error
  }
  finally { clearTimeout(timer); clearInterval(reserve); if (child.exitCode === null) child.kill("SIGKILL"); await Promise.allSettled([completion]) }
}

if (process.argv[1]?.endsWith("/protected-home-restore.mjs")) {
  let fd
  let abort = () => {}
  let succeeded = false
  const onSignal = () => { abort(); process.exitCode = 1 }
  try {
    const [helper, volume, archive] = process.argv.slice(2)
    const container = process.env.CHARIOX_SLICE_NAME
    const generation = process.env.CHARIOX_SLICE_RESTORE_GENERATION
    if (!/^chariox-slice-[A-Za-z0-9_.:-]+$/.test(container ?? "") || !/^[a-f0-9]{32}$/.test(generation ?? "")
        || volume !== `${container}-home-g${generation}`
        || !helper.startsWith(`${container}-home-restore-`)
        || !/^[0-9]+$/.test(helper.slice(`${container}-home-restore-`.length))
        || !/^\/proc\/[1-9][0-9]*\/fd\/[0-9]+$/.test(archive)) refuse("restore arguments are invalid")
    // This kernel-owned proc descriptor is pinned and hash-verified by the
    // broker. Following it opens the same archive inode, never a caller path.
    fd = openSync(archive, constants.O_RDONLY)
    const metadata = fstatSync(fd)
    if (!metadata.isFile() || metadata.uid !== process.getuid() || metadata.nlink !== 1
        || (metadata.mode & 0o077) !== 0 || metadata.size <= 0 || metadata.size > 32 * 1024 ** 3) {
      refuse("pinned archive is not a private, single-link file of 1 byte to 32 GiB")
    }
    const environment = {HOME: "/var/lib/chariox-docker/home", PATH: "/usr/bin:/bin", DOCKER_HOST: process.env.DOCKER_HOST}
    const docker = args => spawnSync("/usr/bin/docker", args, {env: environment, timeout: 30_000, maxBuffer: 1024 * 1024})
    const inspected = docker(["container", "inspect", helper])
    const inventory = docker(["exec", "-u", "root", helper, "find", "-P", "/home-dst", "-mindepth", "1", "-maxdepth", "1", "-printf", "%P\\0"])
    if (inspected.status !== 0 || inventory.status !== 0) refuse("restore helper cannot be inspected")
    const records = JSON.parse(inspected.stdout)
    if (!Array.isArray(records) || records.length !== 1) refuse("restore helper is ambiguous")
    verifyFreshRestoreTarget(records[0], helper, volume, inventory.stdout)
    let stopped = false
    abort = () => {
      if (!stopped) { stopped = true; docker(["rm", "-f", helper]) }
    }
    process.once("SIGTERM", onSignal)
    process.once("SIGINT", onSignal)
    const authority = verifiedProtectedAuthority()
    requireRestoreVolumeReserve(docker, helper)
    await validateCompressedArchive(fd, helper, environment, abort)
    await extractCompressedArchive(fd, helper, environment, abort)
    if (docker(["exec", "-u", "root", helper, "chown", "-R", `${authority.dataUid}:${authority.dataGid}`, "/home-dst"]).status !== 0) {
      refuse("restored home ownership could not be set")
    }
    if (docker(["exec", "-u", "root", helper, "/usr/bin/sync", "-f", "/home-dst"]).status !== 0) refuse("restored home could not be synced")
    createHomeGenerationStore(DURABLE_LAYOUT_ROOT).complete({
      container, token: generation, volume, digest: process.env.CHARIOX_SLICE_RESTORE_DIGEST,
    })
    succeeded = true
  } catch (error) {
    // Name the failed check for the operator; other errors keep only their bounded message.
    const reason = error instanceof RestoreRefusal ? error.reason : String(error?.message ?? error).slice(0, 300)
    console.error(`Saved slice home restore was refused (${reason}); existing identity and saved state are preserved`)
    process.exitCode = 1
  } finally {
    if (!succeeded) abort()
    process.removeListener("SIGTERM", onSignal)
    process.removeListener("SIGINT", onSignal)
    if (fd !== undefined) closeSync(fd)
  }
}
