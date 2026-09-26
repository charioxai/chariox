import { chmodSync, chownSync, mkdirSync, readFileSync } from "node:fs"
import { createServer } from "node:net"
import { dirname, resolve } from "node:path"
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

function serviceGid(name) {
  const row = readFileSync("/etc/group", "utf8").split("\n").find((line) => line.startsWith(`${name}:`))
  const gid = row?.split(":")[2]
  if (!/^[0-9]+$/.test(gid ?? "")) fail(`quota socket group ${name} is unavailable`)
  return Number(gid)
}

function createServerService() {
  const stateStore = createFileSliceDiskQuotaStateStore()
  const allocator = createSliceDiskQuotaAllocator({ backend: createSystemSliceDiskQuotaBackend(), stateStore })
  mkdirSync(dirname(SLICE_DISK_QUOTA_SOCKET_PATH), { recursive: true, mode: 0o750 })
  chownSync(dirname(SLICE_DISK_QUOTA_SOCKET_PATH), 0, serviceGid("chariox-docker"))
  chmodSync(dirname(SLICE_DISK_QUOTA_SOCKET_PATH), 0o750)
  const server = createServer((socket) => {
    socket.setTimeout(SLICE_DISK_QUOTA_REQUEST_TIMEOUT_MS, () => socket.destroy())
    let input = ""
    socket.setEncoding("utf8")
    socket.on("data", (chunk) => {
      input += chunk
      if (Buffer.byteLength(input) > SLICE_DISK_QUOTA_FRAME_MAX_BYTES) {
        socket.destroy(new Error("disk quota request is too large"))
        return
      }
      const newline = input.indexOf("\n")
      if (newline < 0) return
      const line = input.slice(0, newline)
      if (input.slice(newline + 1).length !== 0) {
        socket.destroy(new Error("disk quota request must be one frame"))
        return
      }
      try {
        const response = allocator.handle(JSON.parse(line))
        socket.end(`${JSON.stringify({ protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION, ok: true, result: response })}\n`)
      } catch (error) {
        socket.end(`${JSON.stringify({ protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION, ok: false, error: error instanceof Error ? error.message : String(error) })}\n`)
      }
    })
  })
  server.listen(SLICE_DISK_QUOTA_SOCKET_PATH, () => {
    chownSync(SLICE_DISK_QUOTA_SOCKET_PATH, 0, serviceGid("chariox-docker"))
    chmodSync(SLICE_DISK_QUOTA_SOCKET_PATH, 0o660)
  })
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) createServerService()
