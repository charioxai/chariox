import { spawnOwned, signalOwnedProcessGroup } from "./owned-process-signals.mjs"
import { createHash } from "node:crypto"
import { closeSync, constants, fsyncSync, fstatSync, lstatSync, openSync, readFileSync, statfsSync, unlinkSync, writeSync } from "node:fs"
import { dirname } from "node:path"

const policy = JSON.parse(readFileSync(new URL("./home-archive-policy.json", import.meta.url), "utf8"))
if (Object.keys(policy).sort().join(",") !== "minimumFreeBytes,progressTimeoutMs,schemaVersion"
    || policy.schemaVersion !== 1 || !Number.isSafeInteger(policy.minimumFreeBytes)
    || policy.minimumFreeBytes < 0 || !Number.isSafeInteger(policy.progressTimeoutMs)
    || policy.progressTimeoutMs <= 0 || policy.progressTimeoutMs > 2_147_483_647) throw new Error("invalid home archive policy")
export const HOME_ARCHIVE_MINIMUM_FREE_BYTES = policy.minimumFreeBytes
export const HOME_ARCHIVE_PROGRESS_TIMEOUT_MS = policy.progressTimeoutMs

// Match only operations that actually verify and restore a saved home. Other
// provisioner actions retain their existing operation deadlines.
export function isHomeArchiveRestoreRequest(request) {
  return request.kind === "provisioner" && ["provision", "restore-state"].includes(request.action)
    && typeof request.environment?.CHARIOX_SLICE_SAVED_HOME_ARCHIVE === "string"
    && request.environment.CHARIOX_SLICE_SAVED_HOME_ARCHIVE.length > 0
}

export function homeArchiveMetadataMatches(metadata, scope, id, sizeBytes) {
  return metadata.schemaVersion === 1 && metadata.scope === scope && metadata.id === id
    && Number.isSafeInteger(metadata.sizeBytes) && metadata.sizeBytes > 0
    && metadata.sizeBytes === sizeBytes && /^[a-f0-9]{64}$/.test(metadata.sha256)
}

// Archive bytes go only from the helper's stdout to private product state.
// Never stage a full home (including provider accounts) in its writable layer.
export async function capturePrivateHomeArchive({ command, args, env, destination,
  maxBytes = null, minimumFreeBytes = HOME_ARCHIVE_MINIMUM_FREE_BYTES, timeoutMs = null,
  progressTimeoutMs = HOME_ARCHIVE_PROGRESS_TIMEOUT_MS, availableBytes, spawnProcess = spawnOwned }) {
  if (maxBytes !== null && (!Number.isSafeInteger(maxBytes) || maxBytes <= 0)
      || !Number.isSafeInteger(minimumFreeBytes) || minimumFreeBytes < 0
      || timeoutMs !== null && (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0 || timeoutMs > 2_147_483_647)
      || !Number.isSafeInteger(progressTimeoutMs) || progressTimeoutMs <= 0 || progressTimeoutMs > 2_147_483_647) {
    throw new Error("invalid home archive capture limits")
  }
  const parent = dirname(destination)
  const available = availableBytes ?? (() => {
    const fs = statfsSync(parent, { bigint: true })
    return fs.bavail * fs.bsize
  })
  if (available() <= BigInt(minimumFreeBytes)) throw new Error("insufficient space for slice home archive")
  const fd = openSync(destination, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | constants.O_NOFOLLOW, 0o600)
  let child
  let settled
  let timer
  let progressTimer
  let stalled = false
  let complete = false
  let timedOut = false
  let producerClosed = false
  const stopProducer = () => {
    if (!child?.pid || producerClosed) return
    // Each Unix producer owns this group, including descendants retaining pipes.
    try { signalOwnedProcessGroup(child, "SIGKILL") }
    catch (error) { if (error.code !== "ESRCH") throw error }
  }
  const armProgress = () => {
    clearTimeout(progressTimer)
    progressTimer = setTimeout(() => { stalled = true; stopProducer() }, progressTimeoutMs)
  }
  try {
    child = spawnProcess(command, args, { env, detached: process.platform !== "win32", retainLeader: true, stdio: ["ignore", "pipe", "ignore"] })
    settled = new Promise(resolve => {
      child.once("error", () => resolve({ failed: true }))
      child.once("close", (code, signal) => { producerClosed = true; resolve({ code, signal }) })
    })
    armProgress()
    if (timeoutMs !== null) timer = setTimeout(() => { timedOut = true; stopProducer() }, timeoutMs)
    let sizeBytes = 0
    const digest = createHash("sha256")
    for await (const chunk of child.stdout) {
      if (!Number.isSafeInteger(sizeBytes + chunk.length) || maxBytes !== null && sizeBytes + chunk.length > maxBytes) throw new Error("slice home archive exceeds its size limit")
      if (available() < BigInt(minimumFreeBytes) + BigInt(chunk.length)) {
        throw new Error("insufficient space for slice home archive")
      }
      let written = 0
      while (written < chunk.length) {
        const count = writeSync(fd, chunk, written, chunk.length - written)
        if (count === 0) throw new Error("home archive write made no progress")
        written += count
        armProgress()
      }
      digest.update(chunk)
      sizeBytes += chunk.length
    }
    const status = await settled
    if (stalled) throw new Error("slice home archive capture made no progress")
    if (timedOut) throw new Error("slice home archive capture timed out")
    if (status.failed || status.code !== 0 || status.signal) throw new Error("failed to stream slice home archive")
    if (sizeBytes === 0) throw new Error("slice home archive is empty")
    if (available() < BigInt(minimumFreeBytes)) throw new Error("insufficient space after slice home archive")
    fsyncSync(fd)
    complete = true
    return { sizeBytes, sha256: digest.digest("hex") }
  } catch (error) {
    if (stalled) throw new Error("slice home archive capture made no progress")
    throw error
  } finally {
    clearTimeout(progressTimer)
    clearTimeout(timer)
    if (!complete) stopProducer()
    if (settled) await settled
    // A replaced name is not our partial and must never be removed.
    const created = fstatSync(fd)
    closeSync(fd)
    if (!complete) {
      try {
        const current = lstatSync(destination)
        if (current.isFile() && current.dev === created.dev && current.ino === created.ino) unlinkSync(destination)
      } catch (error) { if (error.code !== "ENOENT") throw error }
    }
  }
}
