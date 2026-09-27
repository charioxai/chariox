import { chmodSync, mkdirSync } from "node:fs"
import { createServer } from "node:net"
import { dirname, resolve } from "node:path"
import { TextDecoder } from "node:util"
import { pathToFileURL } from "node:url"
import {
  SLICE_DISK_QUOTA_FRAME_MAX_BYTES,
  SLICE_DISK_QUOTA_PROTOCOL_VERSION,
  SLICE_DISK_QUOTA_REQUEST_TIMEOUT_MS,
  SLICE_DISK_QUOTA_SOCKET_PATH,
} from "./slice-disk-quota-contract.mjs"
import { createSliceDiskQuotaAllocator } from "./slice-disk-quota-allocator.mjs"
import { createFileSliceDiskQuotaStateStore } from "./slice-disk-quota-state-store.mjs"
import { createSystemSliceDiskQuotaBackend } from "./slice-disk-quota-xfs-backend.mjs"

function fail(message) {
  throw new Error(message)
}

export function handleSliceDiskQuotaConnection(socket, allocator, {
  requestTimeoutMs = SLICE_DISK_QUOTA_REQUEST_TIMEOUT_MS,
  inactivityTimeoutMs = requestTimeoutMs,
} = {}) {
  let chunks = []
  let receivedBytes = 0
  let settled = false
  let absoluteDeadline

  const cleanup = () => {
    clearTimeout(absoluteDeadline)
    socket.setTimeout(0)
  }
  const closeWithoutResponse = () => {
    if (settled) return
    settled = true
    cleanup()
    socket.destroy()
  }
  if ([requestTimeoutMs, inactivityTimeoutMs].some((timeoutMs) => (
    !Number.isSafeInteger(timeoutMs) || timeoutMs <= 0 || timeoutMs > 2_147_483_647
  ))) {
    throw new RangeError("managed disk quota request timeouts must be positive bounded integers")
  }
  socket.setTimeout(inactivityTimeoutMs, closeWithoutResponse)
  absoluteDeadline = setTimeout(closeWithoutResponse, requestTimeoutMs)
  socket.on("error", closeWithoutResponse)
  socket.on("data", (chunk) => {
    if (settled) return
    receivedBytes += chunk.length
    if (receivedBytes > SLICE_DISK_QUOTA_FRAME_MAX_BYTES) {
      closeWithoutResponse()
      return
    }
    chunks.push(Buffer.from(chunk))
  })
  socket.once("end", () => {
    if (settled) return
    const input = Buffer.concat(chunks, receivedBytes)
    chunks = []
    const newline = input.indexOf(0x0a)
    if (newline < 0 || newline !== input.length - 1) {
      closeWithoutResponse()
      return
    }
    settled = true
    cleanup()
    try {
      const body = new TextDecoder("utf-8", { fatal: true }).decode(input.subarray(0, newline))
      const result = allocator.handle(JSON.parse(body))
      socket.end(`${JSON.stringify({ protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION, ok: true, result })}\n`)
    } catch (error) {
      socket.end(`${JSON.stringify({
        protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
        ok: false,
        error: error instanceof Error ? error.message : String(error),
      })}\n`)
    }
  })
  socket.once("close", closeWithoutResponse)
}

function createServerService() {
  const stateStore = createFileSliceDiskQuotaStateStore()
  const allocator = createSliceDiskQuotaAllocator({ backend: createSystemSliceDiskQuotaBackend(), stateStore })
  mkdirSync(dirname(SLICE_DISK_QUOTA_SOCKET_PATH), { recursive: true, mode: 0o750 })
  chmodSync(dirname(SLICE_DISK_QUOTA_SOCKET_PATH), 0o750)
  const server = createServer({ allowHalfOpen: true }, (socket) => handleSliceDiskQuotaConnection(socket, allocator))
  server.listen(SLICE_DISK_QUOTA_SOCKET_PATH, () => {
    chmodSync(SLICE_DISK_QUOTA_SOCKET_PATH, 0o660)
  })
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) createServerService()
