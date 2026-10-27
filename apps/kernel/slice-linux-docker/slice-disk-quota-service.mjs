import { chmodSync, mkdirSync, realpathSync } from "node:fs"
import { createServer } from "node:net"
import { dirname } from "node:path"
import { TextDecoder } from "node:util"
import { fileURLToPath } from "node:url"
import {
  SLICE_DISK_QUOTA_FRAME_MAX_BYTES,
  SLICE_DISK_QUOTA_PROTOCOL_VERSION,
  SLICE_DISK_QUOTA_REQUEST_TIMEOUT_MS,
  SLICE_DISK_QUOTA_SOCKET_PATH,
  validateSliceDiskQuotaRequest,
} from "./slice-disk-quota-contract.mjs"
import { createSliceDiskQuotaAllocator } from "./slice-disk-quota-allocator.mjs"
import { createSliceDiskQuotaCoordinator } from "./slice-disk-quota-coordinator.mjs"
import { createFileSliceDiskQuotaStateStore } from "./slice-disk-quota-state-store.mjs"
import { createSystemSliceDiskQuotaBackend } from "./slice-disk-quota-xfs-backend.mjs"

function fail(message) {
  throw new Error(message)
}

export function handleSliceDiskQuotaConnection(socket, allocator, {
  coordinator = createSliceDiskQuotaCoordinator(),
  requestTimeoutMs = SLICE_DISK_QUOTA_REQUEST_TIMEOUT_MS,
  inactivityTimeoutMs = requestTimeoutMs,
} = {}) {
  let chunks = []
  let receivedBytes = 0
  let settled = false
  let requestEnded = false
  let absoluteDeadline
  const cancellation = new AbortController()

  const cleanup = () => {
    clearTimeout(absoluteDeadline)
    socket.setTimeout(0)
  }
  const closeWithoutResponse = () => {
    if (settled) return
    settled = true
    cancellation.abort()
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
    if (settled || requestEnded) return
    requestEnded = true
    const input = Buffer.concat(chunks, receivedBytes)
    chunks = []
    const newline = input.indexOf(0x0a)
    if (newline < 0 || newline !== input.length - 1) {
      closeWithoutResponse()
      return
    }
    void (async () => {
      try {
        const body = new TextDecoder("utf-8", { fatal: true }).decode(input.subarray(0, newline))
        const request = JSON.parse(body)
        validateSliceDiskQuotaRequest(request)
        const result = request.operation === "reserve" || request.operation === "release"
          ? await coordinator.runReservation(request.identity, () => allocator.handle(request), { signal: cancellation.signal })
          : allocator.handle(request)
        if (settled) return
        settled = true
        cleanup()
        socket.end(`${JSON.stringify({ protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION, ok: true, result })}\n`)
      } catch (error) {
        if (settled) return
        settled = true
        cleanup()
        socket.end(`${JSON.stringify({
          protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
          ok: false,
          error: error instanceof Error ? error.message : String(error),
        })}\n`)
      }
    })()
  })
  socket.once("close", closeWithoutResponse)
}

function createServerService() {
  const stateStore = createFileSliceDiskQuotaStateStore()
  const allocator = createSliceDiskQuotaAllocator({ backend: createSystemSliceDiskQuotaBackend(), stateStore })
  const coordinator = createSliceDiskQuotaCoordinator({ stateStore })
  mkdirSync(dirname(SLICE_DISK_QUOTA_SOCKET_PATH), { recursive: true, mode: 0o750 })
  chmodSync(dirname(SLICE_DISK_QUOTA_SOCKET_PATH), 0o750)
  const server = createServer({ allowHalfOpen: true }, (socket) => handleSliceDiskQuotaConnection(socket, allocator, { coordinator }))
  server.listen(SLICE_DISK_QUOTA_SOCKET_PATH, () => {
    chmodSync(SLICE_DISK_QUOTA_SOCKET_PATH, 0o660)
  })
}

// Started through the /usr/lib/chariox/current symlink; compare real paths.
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) createServerService()
