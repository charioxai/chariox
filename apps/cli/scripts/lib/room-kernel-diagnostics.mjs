import { constants } from "node:fs"
import { open, opendir } from "node:fs/promises"
import path from "node:path"

// Retain connection stages, never arbitrary log messages, URLs, or payloads.
const eventsByComponent = new Map([
  ["daemon.slice_private_relay", new Set([
    "home connector thread starting", "home connector thread exited",
  ])],
  ["slice.private_relay", new Set(["failed to start private relay runtime"])],
  ["daemon.relay_client", new Set([
    "attempting relay connection", "relay socket connect timed out",
    "relay socket connected", "relay socket connect failed",
    "relay register sent", "relay socket disconnected",
    "failed to initialize relay event id allocator", "relay connector idle",
  ])],
])

function project(record, relayUrls) {
  if (!eventsByComponent.get(record?.component)?.has(record.message)) return null
  const event = { component: record.component, event: record.message }
  if (Number.isSafeInteger(record.timestamp_ms) && record.timestamp_ms >= 0) {
    event.timestampMs = record.timestamp_ms
  }
  for (const name of ["primary", "private"]) {
    if (typeof relayUrls[name] === "string" && record.relay_url === relayUrls[name]) event.relay = name
  }
  const error = typeof record.error === "string" ? record.error : record.reason
  if (typeof error === "string") {
    event.errorClass = /connection refused/i.test(error) ? "connection-refused"
      : /does not allow requested action/i.test(error) ? "action-not-allowed"
      : /timed? out|timeout/i.test(error) ? "timeout"
      : /closed|close frame|ended/i.test(error) ? "connection-closed"
      : "other"
  }
  return event
}

export async function captureRoomKernelDiagnostics(logDirectory, relayUrls = {}) {
  const result = { status: "captured", filesRead: 0, bytesRead: 0, truncated: false, events: [] }
  const files = []
  try {
    let scanned = 0
    for await (const entry of await opendir(logDirectory)) {
      if (++scanned > 64) { result.truncated = true; break }
      if (entry.isFile() && /^\d+-daemon-\d+(?:-\d+)?\.ndjson$/.test(entry.name)) files.push(entry.name)
    }
  } catch (error) {
    return { ...result, status: error.code === "ENOENT" ? "missing" : "unavailable" }
  }
  files.sort((a, b) => b.localeCompare(a, "en", { numeric: true }))
  if (files.length > 8) result.truncated = true
  for (const name of files.slice(0, 8)) {
    const remaining = 262144 - result.bytesRead
    if (remaining <= 0) { result.truncated = true; break }
    let handle
    try {
      handle = await open(path.join(logDirectory, name), constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK)
      const info = await handle.stat()
      if (!info.isFile()) { result.truncated = true; continue }
      const size = Math.min(info.size, 65536, remaining)
      const offset = info.size - size
      const buffer = Buffer.alloc(size)
      const { bytesRead } = await handle.read(buffer, 0, size, offset)
      result.filesRead++
      result.bytesRead += bytesRead
      if (offset > 0) result.truncated = true
      const text = buffer.subarray(0, bytesRead).toString("utf8")
      // A tail may start mid-record and a live writer may end mid-record.
      const start = offset > 0 ? text.indexOf("\n") + 1 : 0
      const end = text.lastIndexOf("\n")
      if (end < start) continue
      for (const line of text.slice(start, end).split("\n")) {
        try {
          const event = project(JSON.parse(line), relayUrls)
          if (event) {
            result.events.push(event)
            if (result.events.length > 128) {
              result.truncated = true
              result.events.sort((a, b) => (a.timestampMs ?? 0) - (b.timestampMs ?? 0))
              result.events.shift()
            }
          }
        } catch { /* Ignore incomplete or non-JSON records. */ }
      }
    } catch {
      result.truncated = true
    } finally {
      await handle?.close().catch(() => undefined)
    }
  }
  result.events.sort((a, b) => (a.timestampMs ?? 0) - (b.timestampMs ?? 0))
  return result
}
