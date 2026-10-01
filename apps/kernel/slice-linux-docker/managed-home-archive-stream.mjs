import { spawn } from "node:child_process"
import { createHash } from "node:crypto"
import { closeSync, constants, fsyncSync, openSync, readFileSync, statfsSync, unlinkSync, writeSync } from "node:fs"
import { dirname } from "node:path"

const policy = JSON.parse(readFileSync(new URL("./home-archive-policy.json", import.meta.url), "utf8"))
if (Object.keys(policy).sort().join(",") !== "minimumFreeBytes,schemaVersion"
    || policy.schemaVersion !== 1 || !Number.isSafeInteger(policy.minimumFreeBytes)
    || policy.minimumFreeBytes < 0) throw new Error("invalid home archive policy")
export const HOME_ARCHIVE_MINIMUM_FREE_BYTES = policy.minimumFreeBytes

export function homeArchiveMetadataMatches(metadata, scope, id, sizeBytes) {
  return metadata.schemaVersion === 1 && metadata.scope === scope && metadata.id === id
    && Number.isSafeInteger(metadata.sizeBytes) && metadata.sizeBytes > 0
    && metadata.sizeBytes === sizeBytes && /^[a-f0-9]{64}$/.test(metadata.sha256)
}

// Archive bytes go only from the helper's stdout to private product state.
// Never stage a full home (including provider accounts) in its writable layer.
export async function capturePrivateHomeArchive({ command, args, env, destination,
  maxBytes = null, minimumFreeBytes = HOME_ARCHIVE_MINIMUM_FREE_BYTES, timeoutMs = null,
  availableBytes }) {
  if (maxBytes !== null && (!Number.isSafeInteger(maxBytes) || maxBytes <= 0)
      || !Number.isSafeInteger(minimumFreeBytes) || minimumFreeBytes < 0
      || timeoutMs !== null && (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0)) {
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
  let complete = false
  let timedOut = false
  try {
    child = spawn(command, args, { env, stdio: ["ignore", "pipe", "ignore"] })
    settled = new Promise(resolve => {
      child.once("error", () => resolve({ failed: true }))
      child.once("close", (code, signal) => resolve({ code, signal }))
    })
    if (timeoutMs !== null) timer = setTimeout(() => { timedOut = true; child.kill("SIGKILL") }, timeoutMs)
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
      }
      digest.update(chunk)
      sizeBytes += chunk.length
    }
    const status = await settled
    if (timedOut) throw new Error("slice home archive capture timed out")
    if (status.failed || status.code !== 0 || status.signal) throw new Error("failed to stream slice home archive")
    if (sizeBytes === 0) throw new Error("slice home archive is empty")
    if (available() < BigInt(minimumFreeBytes)) throw new Error("insufficient space after slice home archive")
    fsyncSync(fd)
    complete = true
    return { sizeBytes, sha256: digest.digest("hex") }
  } finally {
    clearTimeout(timer)
    if (!complete) child?.kill("SIGKILL")
    if (settled) await settled
    closeSync(fd)
    if (!complete) unlinkSync(destination)
  }
}
