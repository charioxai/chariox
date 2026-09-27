#!/usr/bin/env node

import { createConnection } from "node:net"
import { resolve } from "node:path"
import { TextDecoder } from "node:util"
import { pathToFileURL } from "node:url"
import {
  SLICE_DISK_QUOTA_FRAME_MAX_BYTES,
  SLICE_DISK_QUOTA_PROTOCOL_VERSION,
  SLICE_DISK_QUOTA_REQUEST_TIMEOUT_MS,
  SLICE_DISK_QUOTA_SOCKET_PATH,
  validateSliceDiskQuotaRequest,
} from "./slice-disk-quota-contract.mjs"

export function sliceDiskQuotaIdentityFromEnvironment(environment = process.env) {
  return {
    ownerKernelId: environment.CHARIOX_SLICE_OWNER_KERNEL_ID,
    ownerMachineId: environment.CHARIOX_SLICE_OWNER_MACHINE_ID,
    sliceId: environment.CHARIOX_SLICE_ID,
    containerName: environment.CHARIOX_SLICE_NAME,
    homeVolumeName: environment.CHARIOX_SLICE_HOME_VOLUME,
  }
}

export function requestSliceDiskQuota(request, {
  socketPath = SLICE_DISK_QUOTA_SOCKET_PATH,
  requestTimeoutMs = SLICE_DISK_QUOTA_REQUEST_TIMEOUT_MS,
  inactivityTimeoutMs = requestTimeoutMs,
} = {}) {
  validateSliceDiskQuotaRequest(request)
  if ([requestTimeoutMs, inactivityTimeoutMs].some((timeoutMs) => (
    !Number.isSafeInteger(timeoutMs) || timeoutMs <= 0 || timeoutMs > 2_147_483_647
  ))) {
    throw new RangeError("managed disk quota request timeouts must be positive bounded integers")
  }
  return new Promise((resolve, reject) => {
    const socket = createConnection(socketPath)
    let response = Buffer.alloc(0)
    let settled = false
    let absoluteDeadline
    const finish = (error, value) => {
      if (settled) return
      settled = true
      clearTimeout(absoluteDeadline)
      socket.destroy()
      if (error) reject(error)
      else resolve(value)
    }
    socket.setTimeout(inactivityTimeoutMs, () => finish(new Error("managed disk quota allocator request timed out")))
    absoluteDeadline = setTimeout(
      () => finish(new Error("managed disk quota allocator request exceeded its absolute deadline")),
      requestTimeoutMs,
    )
    socket.once("error", error => {
      const unavailable = new Error(`managed disk quota allocator is unavailable: ${error.message}`)
      unavailable.code = error.code
      finish(unavailable)
    })
    socket.once("end", () => {
      if (settled) return
      try {
        const newline = response.indexOf(0x0a)
        if (newline < 0) throw new Error("managed disk quota allocator response ended before its frame was complete")
        if (newline !== response.length - 1) throw new Error("managed disk quota allocator response has extra data")
        const body = new TextDecoder("utf-8", { fatal: true }).decode(response.subarray(0, newline))
        const envelope = JSON.parse(body)
        if (!envelope || typeof envelope !== "object" || Array.isArray(envelope)) {
          throw new Error("managed disk quota allocator response is invalid")
        }
        if (envelope.protocolVersion !== SLICE_DISK_QUOTA_PROTOCOL_VERSION) {
          throw new Error("managed disk quota allocator protocol version mismatch")
        }
        if (envelope.ok === true) {
          const keys = Object.keys(envelope).sort()
          if (keys.length !== 3 || keys[0] !== "ok" || keys[1] !== "protocolVersion" || keys[2] !== "result") {
            throw new Error("managed disk quota allocator response is invalid")
          }
          finish(undefined, envelope.result)
          return
        }
        if (envelope.ok === false) {
          const keys = Object.keys(envelope).sort()
          if (keys.length !== 3 || keys[0] !== "error" || keys[1] !== "ok" || keys[2] !== "protocolVersion" || typeof envelope.error !== "string") {
            throw new Error("managed disk quota allocator response is invalid")
          }
          throw new Error(envelope.error || "managed disk quota allocator rejected the request")
        }
        throw new Error("managed disk quota allocator response is invalid")
      } catch (error) {
        finish(error instanceof Error ? error : new Error(String(error)))
      }
    })
    socket.once("close", () => {
      if (!settled) finish(new Error("managed disk quota allocator closed before completing its response"))
    })
    socket.on("data", chunk => {
      if (settled) return
      if (response.length + chunk.length > SLICE_DISK_QUOTA_FRAME_MAX_BYTES) {
        finish(new Error("managed disk quota allocator response is too large"))
        return
      }
      response = Buffer.concat([response, chunk])
      const newline = response.indexOf(0x0a)
      if (newline >= 0 && newline !== response.length - 1) {
        finish(new Error("managed disk quota allocator response has extra data"))
      }
    })
    socket.once("connect", () => socket.end(`${JSON.stringify(request)}\n`))
  })
}

const scriptPath = process.argv[1] ? pathToFileURL(resolve(process.argv[1])).href : ""
if (import.meta.url === scriptPath) {
  const operation = process.argv[2]
  if (!new Set(["apply_home", "apply_layer", "ensure_before_start"]).has(operation)) {
    process.stderr.write("disk quota client operation is not allowed\n")
    process.exitCode = 64
  } else {
    const identity = sliceDiskQuotaIdentityFromEnvironment()
    const request = operation === "ensure_before_start"
      ? { protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION, operation, containerName: identity.containerName }
      : { protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION, operation, identity }
    try {
      const result = await requestSliceDiskQuota(request)
      if (operation === "ensure_before_start" && result.bounded === false) {
        process.stdout.write("unbounded\n")
      } else {
        process.stdout.write("verified\n")
      }
    } catch (error) {
      process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
      process.exitCode = 1
    }
  }
}
