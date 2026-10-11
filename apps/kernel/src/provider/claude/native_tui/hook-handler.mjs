#!/usr/bin/env node
import { appendFileSync, existsSync, readFileSync, renameSync, unlinkSync, writeFileSync } from "node:fs"
import { createHash } from "node:crypto"
import { execFileSync } from "node:child_process"
import { dirname, join } from "node:path"
import { setTimeout as setCallbackTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

const hookWatchdog = setCallbackTimeout(() => {
  try {
    appendFileSync(`${process.env.CHARIOX_CLAUDE_NATIVE_EVENTS}.watchdog`, JSON.stringify({
      at: new Date().toISOString(),
      reason: "hook_event_not_resolved"
    }) + "\n")
  } catch {}
  process.exit(0)
}, 7000)

async function readHookInput() {
  const chunks = []
  let settled = false
  return await new Promise((resolve) => {
    const finish = () => {
      if (settled) return
      settled = true
      resolve(Buffer.concat(chunks).toString("utf8"))
    }
    process.stdin.on("data", (chunk) => chunks.push(chunk))
    process.stdin.once("end", finish)
    process.stdin.once("error", finish)
    process.stdin.resume()
    setCallbackTimeout(finish, 1000)
  })
}

let nativePromptId = null
try { nativePromptId = readFileSync(`${process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT}.approval-origin`, "utf8").trim() || null } catch {}
const raw = await readHookInput()
let input = {}
try {
  input = raw.trim() ? JSON.parse(raw) : {}
} catch (error) {
  input = { hook_event_name: "parse_error", raw, error: String(error) }
}
const eventName = input.hook_event_name ?? "unknown"
const parsedContextChunkIndex = Number.parseInt(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK ?? "", 10)
const contextChunkIndex = Number.isInteger(parsedContextChunkIndex) && parsedContextChunkIndex >= 0
  ? parsedContextChunkIndex
  : null
const contextOnlyHook = contextChunkIndex !== null && contextChunkIndex > 0
if (!contextOnlyHook && eventName === "SessionStart") {
  try { unlinkSync(join(dirname(process.argv[1]), "mcp-config.json")) } catch {}
}
const hookContextRequestId = eventName === "UserPromptSubmit" || eventName === "PreToolUse" || eventName === "PermissionRequest"
  ? `${Date.now()}-${process.pid}-${Math.random().toString(36).slice(2)}`
  : null
// MP-08 / MP-10 / MP-11: Esc can terminate this hook before Claude writes
// a user transcript record. Receipt the exact hook request through this same
// native bridge, then preserve the provider's normal termination outcome.
if (!contextOnlyHook && eventName === "UserPromptSubmit" && hookContextRequestId) {
  for (const signal of ["SIGINT", "SIGTERM"]) {
    const interrupted = () => {
      try {
        appendFileSync(process.env.CHARIOX_CLAUDE_NATIVE_EVENTS, JSON.stringify({
          at: new Date().toISOString(),
          hook_event_name: "UserPromptSubmitInterrupted",
          hook_context_request_id: hookContextRequestId,
          signal,
        }) + "\n")
      } finally {
        process.removeListener(signal, interrupted)
        if (!Number.isSafeInteger(process.pid) || process.pid <= 1) process.exit(1)
        process.kill(process.pid, signal)
      }
    }
    process.on(signal, interrupted)
  }
}

// MP-08 / MP-10 / MP-11: keep the sole admission/interrupt hook alive while
// context-only helpers finish. Helpers never acquire prompt-control authority.
const expectedContextChunks = Number.parseInt(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNKS ?? "1", 10)
if (!Number.isSafeInteger(expectedContextChunks) || expectedContextChunks < 1 || expectedContextChunks > 8) {
  throw new Error("invalid native context hook count")
}
const batchEnabled = eventName === "UserPromptSubmit" && contextChunkIndex !== null && expectedContextChunks > 1
const batchPath = `${process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT}.hook-batch.json`
const batchFingerprint = createHash("sha256").update(JSON.stringify([
  input.session_id ?? null, input.transcript_path ?? null, input.prompt ?? null,
])).digest("hex")
let primaryBatch = null
function publishBatch(batch) {
  const temporary = `${batchPath}.${process.pid}.tmp`
  writeFileSync(temporary, JSON.stringify(batch), { mode: 0o600 })
  renameSync(temporary, batchPath)
}
function readLiveBatch() {
  try {
    const batch = JSON.parse(readFileSync(batchPath, "utf8"))
    if (batch.closed || batch.fingerprint !== batchFingerprint || !/^[a-zA-Z0-9_-]{1,128}$/.test(batch.id)
        || !Number.isSafeInteger(batch.pid) || batch.pid <= 1 || !Number.isFinite(batch.deadline)) return null
    // Reject exited/reused PIDs; inspect only the exact hook's command, never
    // signal it or expose its arguments. This works on both Linux and macOS.
    const command = execFileSync("ps", ["-p", String(batch.pid), "-o", "args="], {
      encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], timeout: 1000,
    })
    return command.includes(process.argv[1]) ? batch : null
  } catch { return null }
}
function chunkReceiptPath(batch, index) {
  return join(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES, `${batch.id}.context-${index}.done`)
}
if (batchEnabled && contextChunkIndex === 0) {
  if (!Number.isSafeInteger(process.pid) || process.pid <= 1) process.exit(1)
  primaryBatch = { id: hookContextRequestId, pid: process.pid, fingerprint: batchFingerprint,
    deadline: Date.now() + 5000, closed: false }
  publishBatch(primaryBatch)
  process.on("exit", () => {
    try {
      const current = JSON.parse(readFileSync(batchPath, "utf8"))
      if (current.id === primaryBatch.id) publishBatch({ ...primaryBatch, closed: true })
      for (let index = 1; index < expectedContextChunks; index++) {
        try { unlinkSync(chunkReceiptPath(primaryBatch, index)) } catch {}
      }
    } catch {}
  })
}
async function finishContextBatch() {
  if (!batchEnabled) return
  if (primaryBatch) {
    while (Date.now() < primaryBatch.deadline) {
      if (Array.from({ length: expectedContextChunks - 1 }, (_, index) => index + 1)
          .every(index => existsSync(chunkReceiptPath(primaryBatch, index)))) return
      await sleep(10)
    }
  } else {
    const deadline = Date.now() + 5000
    while (Date.now() < deadline) {
      const batch = readLiveBatch()
      if (batch) {
        writeFileSync(chunkReceiptPath(batch, contextChunkIndex), "", { mode: 0o600 })
        return
      }
      await sleep(10)
    }
  }
}

if (!contextOnlyHook) {
  appendFileSync(process.env.CHARIOX_CLAUDE_NATIVE_EVENTS, JSON.stringify({
    at: new Date().toISOString(),
    hook_event_name: eventName,
    hook_context_request_id: hookContextRequestId,
    native_prompt_id: nativePromptId,
    prompt: input.prompt ?? null,
    transcript_path: input.transcript_path ?? null,
    permission_mode: input.permission_mode ?? null,
    tool_name: input.tool_name ?? null,
    tool_input: input.tool_input ?? null,
    tool_response: input.tool_response ?? null,
    error: input.error ?? null,
    error_details: eventName === "StopFailure" ? input.error_details ?? null : null,
    last_assistant_message: eventName === "StopFailure" ? input.last_assistant_message ?? null : null,
  }) + "\n")
}

function utf8ContextChunk(value, chunkIndex, maximumBytes = 6000) {
  const chunks = []
  let chunk = ""
  let chunkBytes = 0
  for (const scalar of value) {
    const scalarBytes = Buffer.byteLength(scalar, "utf8")
    if (chunk && chunkBytes + scalarBytes > maximumBytes) {
      chunks.push(chunk)
      chunk = ""
      chunkBytes = 0
    }
    chunk += scalar
    chunkBytes += scalarBytes
  }
  if (chunk) chunks.push(chunk)
  return chunks[chunkIndex] ?? ""
}

if (eventName === "UserPromptSubmit") {
  let additionalContext = ""
  try {
    additionalContext = readFileSync(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT, "utf8")
  } catch {}
  if (!additionalContext && contextChunkIndex === 0 && hookContextRequestId && process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES) {
    const responseFile = join(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES, `${hookContextRequestId}.txt`)
    const deadline = Date.now() + 5000
    while (Date.now() < deadline) {
      if (existsSync(responseFile)) {
        additionalContext = readFileSync(responseFile, "utf8")
        writeFileSync(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT, additionalContext)
        try { unlinkSync(responseFile) } catch {}
        break
      }
      await sleep(50)
    }
  }
  if (batchEnabled && contextOnlyHook) {
    const deadline = Date.now() + 5000
    while (Date.now() < deadline) {
      const batch = readLiveBatch()
      if (batch?.ready) {
        try { additionalContext = readFileSync(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT, "utf8") } catch {}
        break // An empty approved context is ready too.
      }
      await sleep(10)
    }
  } else if (!additionalContext && contextOnlyHook) {
    const deadline = Date.now() + 5000
    while (Date.now() < deadline) {
      try { additionalContext = readFileSync(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT, "utf8") } catch {}
      if (additionalContext) break
      await sleep(50)
    }
  }
  if (primaryBatch) {
    primaryBatch.ready = true
    publishBatch(primaryBatch)
  }
  additionalContext = utf8ContextChunk(additionalContext, contextChunkIndex ?? 0)
  await finishContextBatch()
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "UserPromptSubmit",
      additionalContext
    }
  }))
  process.exit(0)
} else if (eventName === "PreToolUse" || eventName === "PermissionRequest") {
  if (input.permission_mode === "bypassPermissions") {
    if (eventName === "PermissionRequest") {
      // PermissionRequestHookSpecificOutput nests the PermissionResult under
      // `decision`. PreToolUse uses a separate event-specific output shape.
      process.stdout.write(JSON.stringify({
        hookSpecificOutput: {
          hookEventName: "PermissionRequest",
          decision: {
            behavior: "allow"
          }
        }
      }))
    }
    process.exit(0)
  }
  // The kernel bridges only events that name a tool; waiting on any other
  // event would hold Claude for the full decision deadline.
  if (typeof input.tool_name !== "string" || !input.tool_name.trim()) {
    process.exit(0)
  }
  clearTimeout(hookWatchdog)
  const responseDir = process.env.CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES
  const responseFile = responseDir && hookContextRequestId
    ? join(responseDir, `${hookContextRequestId}.json`)
    : null
  if (responseFile) {
    // CLAUDE_NATIVE_PERMISSION_HOOK_WAIT_SECS
    const deadline = Date.now() + 330000
    while (Date.now() < deadline) {
      if (existsSync(responseFile)) {
        try {
          const decision = JSON.parse(readFileSync(responseFile, "utf8"))
          try { unlinkSync(responseFile) } catch {}
          if (decision?.behavior) {
            process.stdout.write(JSON.stringify({
              hookSpecificOutput: {
                hookEventName: eventName,
                decision: {
                  behavior: decision.behavior,
                  ...(decision.behavior === "deny" ? { message: decision.message ?? "Denied through Chariox." } : {})
                }
              }
            }))
          }
        } catch {}
        break
      }
      await sleep(50)
    }
  }
}
process.exit(0)
