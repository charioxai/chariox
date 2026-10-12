#!/usr/bin/env node
import { appendFileSync, existsSync, readFileSync, unlinkSync, writeFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { createHash } from "node:crypto"
import { createConnection, createServer } from "node:net"
import { setTimeout as setCallbackTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

// MP-08 / MP-10 / MP-11: the kernel must settle a prompt rejected by its hook.
let contextFailureEvent = null
function reportContextFailure() {
  if (!contextFailureEvent) return
  try { appendFileSync(process.env.CHARIOX_CLAUDE_NATIVE_EVENTS, JSON.stringify(contextFailureEvent) + "\n") } catch {}
}
const hookWatchdog = setCallbackTimeout(() => {
  try {
    appendFileSync(`${process.env.CHARIOX_CLAUDE_NATIVE_EVENTS}.watchdog`, JSON.stringify({
      at: new Date().toISOString(),
      reason: "hook_event_not_resolved"
    }) + "\n")
  } catch {}
  reportContextFailure()
  process.exit(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK !== undefined ? 2 : 0)
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
  const sessionId = typeof input.session_id === "string" ? input.session_id.trim() : ""
  if (sessionId && process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT) {
    writeFileSync(`${process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT}.session-start-id`, sessionId)
  }
  try { unlinkSync(join(dirname(process.argv[1]), "mcp-config.json")) } catch {}
}
const hookContextRequestId = eventName === "UserPromptSubmit" || eventName === "PreToolUse" || eventName === "PermissionRequest"
  ? `${Date.now()}-${process.pid}-${Math.random().toString(36).slice(2)}`
  : null
const hookContextDeadlineMs = Date.now() + 5000
if (!contextOnlyHook && eventName === "UserPromptSubmit") {
  contextFailureEvent = {
    hook_event_name: "CharioxContextFailed",
    hook_context_request_id: hookContextRequestId,
    session_id: input.session_id ?? null,
  }
}
if (!contextOnlyHook) {
  appendFileSync(process.env.CHARIOX_CLAUDE_NATIVE_EVENTS, JSON.stringify({
    at: new Date().toISOString(),
    hook_event_name: eventName,
    session_id: input.session_id ?? null,
    hook_context_request_id: hookContextRequestId,
    ...(eventName === "UserPromptSubmit" ? { hook_context_deadline_ms: hookContextDeadlineMs } : {}),
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

async function freshContext() {
  // Structured print turns already carry the kernel-dispatched envelope.
  // The primary hook freezes it once for this invocation's chunk delivery.
  if (process.env.CHARIOX_CLAUDE_PROMPT_CONTEXT_SOURCE === "dispatched") {
    return readFileSync(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT, "utf8")
  }
  const responseDir = process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES
  if (!hookContextRequestId || !responseDir) throw new Error("missing context response route")
  const responseFile = join(responseDir, `${hookContextRequestId}.txt`)
  const deadline = hookContextDeadlineMs
  while (Date.now() < deadline) {
    if (existsSync(responseFile)) {
      const context = readFileSync(responseFile, "utf8")
      unlinkSync(responseFile)
      return context
    }
    await sleep(25)
  }
  throw new Error("context response timed out")
}

// Claude runs the eight chunk hooks in parallel and waits for all of them.
// Share this invocation's immutable response over a private local socket;
// a file containing the preceding turn's context is never a response.
let contextChunkDeliverySocket

async function currentContextChunk() {
  if (contextChunkIndex === null) return utf8ContextChunk(await freshContext(), 0)
  const contextDirectory = dirname(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT)
  const invocation = createHash("sha256").update(raw).digest("hex").slice(0, 24)
  process.chdir(contextDirectory)
  const socketPath = `./.prompt-context-${invocation}.sock`
  const deadline = Date.now() + 6000
  if (contextOnlyHook) {
    while (Date.now() < deadline) {
      try {
        return await new Promise((resolve, reject) => {
          const socket = createConnection(socketPath)
          let response = ""
          socket.setTimeout(Math.max(1, deadline - Date.now()), () => socket.destroy(new Error("context chunk timed out")))
          socket.on("error", reject)
          socket.on("connect", () => socket.write(JSON.stringify({ index: contextChunkIndex }) + "\n"))
          socket.on("data", data => {
            response += data
            if (response.length > 12_000) { socket.destroy(new Error("invalid context chunk")); return }
            if (!response.endsWith("\n")) return
            try {
              const packet = JSON.parse(response)
              if (!packet.accepted || typeof packet.context !== "string") throw new Error("context was not accepted")
              contextChunkDeliverySocket = socket
              socket.removeAllListeners("data")
              resolve(packet.context)
            } catch (error) { socket.destroy(); reject(error) }
          })
          socket.on("end", () => reject(new Error("context chunk closed before delivery")))
        })
      } catch { await sleep(25) }
    }
    throw new Error("current context chunk unavailable")
  }
  // A previous crashed hook can leave its socket inode, but a live one must
  // never be removed or replaced by another prompt invocation.
  if (existsSync(socketPath)) {
    const stale = await new Promise(resolve => {
      const probe = createConnection(socketPath)
      probe.on("connect", () => { probe.destroy(); resolve(false) })
      probe.on("error", error => resolve(error.code === "ECONNREFUSED"))
    })
    if (!stale) throw new Error("context invocation already active")
    unlinkSync(socketPath)
  }
  let context
  let contextError
  let delivered = 0
  const deliveredIndexes = new Set()
  const peers = new Set()
  const server = createServer(socket => {
    peers.add(socket)
    socket.on("close", () => peers.delete(socket))
    socket.on("error", () => {})
    socket.setTimeout(Math.max(1, deadline - Date.now()), () => socket.destroy())
    let request = ""
    socket.on("data", async data => {
      request += data
      if (request.length > 128) { socket.destroy(); return }
      if (!request.endsWith("\n")) return
      socket.removeAllListeners("data")
      let index
      try { index = JSON.parse(request).index } catch { socket.destroy(); return }
      if (!Number.isInteger(index) || index < 1 || index > 7 || deliveredIndexes.has(index)) { socket.destroy(); return }
      deliveredIndexes.add(index)
      while (context === undefined && !contextError && Date.now() < deadline) await sleep(10)
      let acknowledgement = ""
      socket.on("data", data => {
        acknowledgement += data
        if (acknowledgement === "delivered\n") { delivered += 1; socket.end() }
        else if (acknowledgement.length > 16) socket.destroy()
      })
      socket.write(JSON.stringify({ accepted: context !== undefined && !contextError, context: utf8ContextChunk(context ?? "", index) }) + "\n")
    })
  })
  try {
    await new Promise((resolve, reject) => { server.once("error", reject); server.listen(socketPath, resolve) })
    try { context = await freshContext() } catch (error) { contextError = error }
    while (delivered < 7 && Date.now() < deadline) await sleep(10)
    if (contextError || context === undefined || delivered !== 7) throw new Error("current context was not delivered to every hook")
    return utf8ContextChunk(context, 0)
  } finally {
    for (const peer of peers) peer.destroy()
    await new Promise(resolve => server.close(resolve))
    try { unlinkSync(socketPath) } catch {}
  }
}

if (eventName === "UserPromptSubmit") {
  try {
    const additionalContext = await currentContextChunk()
    await new Promise((resolve, reject) => process.stdout.write(
      JSON.stringify({ hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext } }),
      error => error ? reject(error) : resolve(),
    ))
    if (contextChunkDeliverySocket) {
      await new Promise((resolve, reject) => contextChunkDeliverySocket.end("delivered\n", error => error ? reject(error) : resolve()))
    }
    if (!contextOnlyHook && process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_RECEIPTS === "1") {
      appendFileSync(process.env.CHARIOX_CLAUDE_NATIVE_EVENTS, JSON.stringify({
        hook_event_name: "CharioxContextDelivered",
        hook_context_request_id: hookContextRequestId,
        session_id: input.session_id ?? null,
      }) + "\n")
    }
    process.exit(0)
  } catch {
    reportContextFailure()
    process.stderr.write("Chariox could not prepare current prompt context. Retry after the kernel is ready.\n")
    process.exit(2)
  }
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
