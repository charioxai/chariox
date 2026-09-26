#!/usr/bin/env node

import { createConnection } from "node:net"
import { resolve } from "node:path"
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

export function requestSliceDiskQuota(request, { socketPath = SLICE_DISK_QUOTA_SOCKET_PATH } = {}) {
  validateSliceDiskQuotaRequest(request)
  return new Promise((resolve, reject) => {
    const socket = createConnection(socketPath)
    let response = ""
    let settled = false
    const finish = (error, value) => {
      if (settled) return
      settled = true
      socket.destroy()
      if (error) reject(error)
      else resolve(value)
    }
    socket.setTimeout(SLICE_DISK_QUOTA_REQUEST_TIMEOUT_MS, () => finish(new Error("managed disk quota allocator request timed out")))
    socket.once("error", error => {
      const unavailable = new Error(`managed disk quota allocator is unavailable: ${error.message}`)
      unavailable.code = error.code
      finish(unavailable)
    })
    socket.on("data", chunk => {
      response += chunk.toString("utf8")
      if (Buffer.byteLength(response) > SLICE_DISK_QUOTA_FRAME_MAX_BYTES) {
        finish(new Error("managed disk quota allocator response is too large"))
        return
      }
      const newline = response.indexOf("\n")
      if (newline < 0) return
      if (response.slice(newline + 1).length !== 0) {
        finish(new Error("managed disk quota allocator response has extra data"))
        return
      }
      try {
        const envelope = JSON.parse(response.slice(0, newline))
        if (envelope.protocolVersion !== SLICE_DISK_QUOTA_PROTOCOL_VERSION) {
          throw new Error("managed disk quota allocator protocol version mismatch")
        }
        if (envelope.ok !== true) throw new Error(envelope.error || "managed disk quota allocator rejected the request")
        finish(undefined, envelope.result)
      } catch (error) {
        finish(error instanceof Error ? error : new Error(String(error)))
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
