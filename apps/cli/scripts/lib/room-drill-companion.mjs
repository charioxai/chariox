import { randomUUID } from "node:crypto"
import { constants } from "node:fs"
import { chmod, lstat, mkdir, open, readFile, realpath, rename, rm } from "node:fs/promises"
import path from "node:path"

const readySchema = "chariox.room_environment.companion_ready.v1"
const resultSchema = "chariox.room_environment.companion_result.v1"

export async function publishRoomDrillCompanionReady(directory, ready) {
  requireRecord(ready, "Room drill companion ready payload")
  if (ready.schema !== readySchema) {
    throw new Error(`Room drill companion ready schema must be ${readySchema}`)
  }
  const resolvedDirectory = await ensurePrivateDirectory(directory)
  await rm(path.join(resolvedDirectory, "result.json"), { force: true })
  const readyPath = path.join(resolvedDirectory, "ready.json")
  const temporaryPath = path.join(resolvedDirectory, `.ready-${randomUUID()}.json`)
  let handle = null
  try {
    handle = await open(
      temporaryPath,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | (constants.O_NOFOLLOW ?? 0),
      0o600,
    )
    await handle.writeFile(`${JSON.stringify(ready, null, 2)}\n`, "utf8")
    await handle.close()
    handle = null
    await chmod(temporaryPath, 0o600)
    await rename(temporaryPath, readyPath)
  } finally {
    await handle?.close().catch(() => undefined)
    await rm(temporaryPath, { force: true }).catch(() => undefined)
  }
  return readyPath
}

async function ensurePrivateDirectory(directory) {
  const resolved = requireAbsoluteDirectory(directory)
  await rejectExistingSymlinkPath(resolved)
  await mkdir(resolved, { recursive: true, mode: 0o700 })
  const entry = await lstat(resolved)
  if (!entry.isDirectory() || entry.isSymbolicLink()) {
    throw new Error("Room drill companion coordination path must be a real directory")
  }
  if (await realpath(resolved) !== resolved) {
    throw new Error("Room drill companion coordination path must not contain symbolic links")
  }
  const uid = process.getuid?.()
  if (uid !== undefined && entry.uid !== uid) {
    throw new Error("Room drill companion coordination directory owner mismatch")
  }
  if (uid !== undefined && (entry.mode & 0o077) !== 0) {
    throw new Error("Room drill companion coordination directory must be private")
  }
  return resolved
}

function requireAbsoluteDirectory(directory) {
  if (typeof directory !== "string" || !directory.trim() || !path.isAbsolute(directory)) {
    throw new Error("Room drill companion coordination directory must be absolute")
  }
  return path.resolve(directory)
}

async function rejectExistingSymlinkPath(resolved) {
  const parsed = path.parse(resolved)
  let current = parsed.root
  for (const part of resolved.slice(parsed.root.length).split(path.sep).filter(Boolean)) {
    current = path.join(current, part)
    let entry
    try {
      entry = await lstat(current)
    } catch (error) {
      if (error?.code === "ENOENT") return
      throw error
    }
    if (entry.isSymbolicLink()) {
      throw new Error("Room drill companion coordination path must not contain symbolic links")
    }
    if (!entry.isDirectory()) {
      throw new Error("Room drill companion coordination path must be a directory")
    }
  }
}

export async function waitForRoomDrillCompanionResult(directory, options) {
  const timeoutMs = positiveInteger(options.timeoutMs, "Room drill companion timeout")
  const pollIntervalMs = positiveInteger(options.pollIntervalMs ?? 100, "Room drill companion poll interval")
  const resultPath = path.join(directory, "result.json")
  const deadline = Date.now() + timeoutMs
  let lastReadError = null
  while (Date.now() < deadline) {
    try {
      const result = JSON.parse(await readFile(resultPath, "utf8"))
      validateResult(result, options)
      return result
    } catch (error) {
      if (error?.code !== "ENOENT") {
        lastReadError = error
        if (!(error instanceof SyntaxError) && !(error instanceof CompanionIncompleteError)) {
          throw error
        }
      }
    }
    await (options.sleep ?? sleep)(Math.min(pollIntervalMs, Math.max(1, deadline - Date.now())))
  }
  const detail = lastReadError ? `: ${lastReadError.message}` : ""
  throw new Error(`Room drill companion timed out after ${timeoutMs}ms${detail}`)
}

function validateResult(result, options) {
  requireRecord(result, "Room drill companion result", CompanionIncompleteError)
  if (typeof result.schema !== "string") {
    throw new CompanionIncompleteError("Room drill companion result schema is incomplete")
  }
  if (result.schema !== resultSchema) {
    throw new Error(`Room drill companion result schema must be ${resultSchema}`)
  }
  if (typeof result.sessionId !== "string") {
    throw new CompanionIncompleteError("Room drill companion result session is incomplete")
  }
  if (result.sessionId !== options.sessionId) {
    throw new Error(`Room drill companion session mismatch: expected ${options.sessionId}, got ${String(result.sessionId)}`)
  }
  if (typeof result.environmentId !== "string") {
    throw new CompanionIncompleteError("Room drill companion result environment is incomplete")
  }
  if (result.environmentId !== options.environmentId) {
    throw new Error(`Room drill companion environment mismatch: expected ${options.environmentId}, got ${String(result.environmentId)}`)
  }
  if (typeof result.status !== "string") {
    throw new CompanionIncompleteError("Room drill companion result status is incomplete")
  }
  if (result.status !== "passed") {
    if (result.status === "failed") {
      throw new CompanionFailureError(
        `Room drill companion failed: ${typeof result.error === "string" ? result.error : "unknown error"}`,
      )
    }
    throw new Error(`Room drill companion result status is unsupported: ${result.status}`)
  }
}

function requireRecord(value, label, ErrorType = Error) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new ErrorType(`${label} must be an object`)
  }
}

function positiveInteger(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`${label} must be a positive integer`)
  }
  return value
}

class CompanionFailureError extends Error {
  constructor(message) {
    super(message)
    this.name = "CompanionFailureError"
  }
}

class CompanionIncompleteError extends Error {
  constructor(message) {
    super(message)
    this.name = "CompanionIncompleteError"
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
