import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises"
import path from "node:path"

const readySchema = "chariox.room_environment.companion_ready.v1"
const resultSchema = "chariox.room_environment.companion_result.v1"

export async function publishRoomDrillCompanionReady(directory, ready) {
  requireRecord(ready, "Room drill companion ready payload")
  if (ready.schema !== readySchema) {
    throw new Error(`Room drill companion ready schema must be ${readySchema}`)
  }
  await mkdir(directory, { recursive: true, mode: 0o700 })
  await rm(path.join(directory, "result.json"), { force: true })
  const readyPath = path.join(directory, "ready.json")
  const temporaryPath = path.join(directory, `.ready-${process.pid}-${Date.now()}.json`)
  await writeFile(temporaryPath, `${JSON.stringify(ready, null, 2)}\n`, { mode: 0o600 })
  await rename(temporaryPath, readyPath)
  return readyPath
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
        if (!isRetryableParseError(error)) throw error
      }
    }
    await sleep(Math.min(pollIntervalMs, Math.max(1, deadline - Date.now())))
  }
  const detail = lastReadError ? `: ${lastReadError.message}` : ""
  throw new Error(`Room drill companion timed out after ${timeoutMs}ms${detail}`)
}

function validateResult(result, options) {
  requireRecord(result, "Room drill companion result")
  if (result.schema !== resultSchema) {
    throw new Error(`Room drill companion result schema must be ${resultSchema}`)
  }
  if (result.sessionId !== options.sessionId) {
    throw new Error(`Room drill companion session mismatch: expected ${options.sessionId}, got ${String(result.sessionId)}`)
  }
  if (result.environmentId !== options.environmentId) {
    throw new Error(`Room drill companion environment mismatch: expected ${options.environmentId}, got ${String(result.environmentId)}`)
  }
  if (result.status !== "passed") {
    throw new Error(`Room drill companion failed: ${typeof result.error === "string" ? result.error : "unknown error"}`)
  }
}

function requireRecord(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object`)
  }
}

function positiveInteger(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`${label} must be a positive integer`)
  }
  return value
}

function isRetryableParseError(error) {
  return error instanceof SyntaxError
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
