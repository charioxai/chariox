import { capturePrivateHomeArchive } from "./managed-home-archive-stream.mjs"
import { constants, openSync, closeSync, unlinkSync } from "node:fs"
import { validateCompressedArchive } from "./protected-home-restore.mjs"
import { spawnSync } from "node:child_process"
import { dirname } from "node:path"
import { streamArchiveToProtectedSink } from "./protected-archive-stream.mjs"
import { requireSupportedHomeEntries, verifyHomeVolumeName } from "./protected-layout.mjs"
import { verifyPrivateHostDirectory } from "./protected-host-root.mjs"

function refuse() { throw new Error("Slice save/backup is unavailable for this storage layout; existing saved state is preserved") }
export function verifyHomeEntryMetadata(bytes) {
  const fields = Buffer.from(bytes).toString("utf8").split("\0")
  if (fields.pop() !== "" || fields.length % 3 !== 0) refuse()
  for (let index = 0; index < fields.length; index += 3) {
    const [path, kind, target] = fields.slice(index, index + 3)
    requireSupportedHomeEntries([path])
    if (!["f", "d", "l"].includes(kind)) refuse()
    if (kind === "l") {
      if (target.startsWith("/") || target.split("/").includes("..")) refuse()
      requireSupportedHomeEntries([`${dirname(path) === "." ? "" : `${dirname(path)}/`}${target}`])
    }
  }
}

export function verifyCaptureHelperVolume(helper, volume) {
  const owner = helper.match(/^(chariox-slice-[A-Za-z0-9_.:-]+)-home-archive-[0-9]+$/)?.[1]
  if (!owner) refuse()
  verifyHomeVolumeName(volume, owner)
}

export async function captureProtectedHome({helper, volume, path, docker, environment, maxBytes, reserveBytes, legacy = false, legacyCapture = capturePrivateHomeArchive}) {
  verifyCaptureHelperVolume(helper, volume)
  verifyPrivateHostDirectory(dirname(path), process.getuid())
  const inspected = docker(["container", "inspect", helper])
  if (inspected.status !== 0) refuse()
  const records = JSON.parse(inspected.stdout)
  if (!Array.isArray(records) || records.length !== 1) refuse()
  const info = records[0]
  if (info.Config?.Labels?.["io.chariox.snapshot-helper"] !== helper
      || info.HostConfig?.NetworkMode !== "none" || info.Mounts?.length !== 1
      || info.Mounts[0].Type !== "volume" || info.Mounts[0].Name !== volume
      || info.Mounts[0].Destination !== "/home-src" || info.Mounts[0].RW !== false) refuse()
  if (!legacy) {
  const inventory = docker(["exec", "-u", "root", helper, "find", "-P", "/home-src", "-mindepth", "1", "-printf", "%P\\0%y\\0%l\\0"])
  if (inventory.status !== 0) refuse()
  verifyHomeEntryMetadata(inventory.stdout)
  }
  const stopOwnedHelper = () => docker(["rm", "-f", helper])
  const terminate = () => stopOwnedHelper()
  process.once("SIGTERM", terminate)
  process.once("SIGINT", terminate)
  let completed = false
  try {
    const captured = legacy ? await legacyCapture({command: "/usr/bin/docker",
      args: ["exec", "-u", "root", helper, "tar", "--zstd", "-C", "/home-src", "-cf", "-", "."],
      env: environment, destination: path}) : await streamArchiveToProtectedSink({command: "/usr/bin/docker",
      args: ["exec", "-u", "root", helper, "tar", "--zstd", "--one-file-system", "-cf", "-", "-C", "/home-src", "."],
      environment, path, maxBytes, reserveBytes})
    completed = true
    if (!legacy) {
      const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW)
      try { await validateCompressedArchive(fd, helper, environment) } finally { closeSync(fd) }
    }
    return captured
  } catch (error) {
    // Killing the CLI alone cannot prove the daemon's tar exec has stopped.
    // This exact helper was verified above and owns no durable private mounts.
    stopOwnedHelper()
    if (completed) unlinkSync(path)
    throw error
  } finally {
    process.removeListener("SIGTERM", terminate)
    process.removeListener("SIGINT", terminate)
  }
}

if (process.argv[1]?.endsWith("/protected-home-capture.mjs")) {
  try {
    const environment = {HOME: "/var/lib/chariox-docker/home", PATH: "/usr/bin:/bin", DOCKER_HOST: process.env.DOCKER_HOST}
    const [helper, volume, path, mode] = process.argv.slice(2)
    if (mode !== undefined && mode !== "legacy-release-f") refuse()
    const result = await captureProtectedHome({helper, volume, path, environment, legacy: mode === "legacy-release-f",
      maxBytes: 32 * 1024 ** 3, reserveBytes: 2 * 1024 ** 3,
      docker: args => spawnSync("/usr/bin/docker", args, {env: environment, timeout: 30_000, maxBuffer: 8 * 1024 ** 2})})
    console.log(JSON.stringify(result))
  } catch {
    console.error("Slice home capture was refused; existing saved state is preserved")
    process.exitCode = 1
  }
}
