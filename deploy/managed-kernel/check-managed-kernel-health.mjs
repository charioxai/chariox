#!/usr/bin/env node

import { connect } from "node:net"
import { constants } from "node:fs"
import { open } from "node:fs/promises"
import { basename, join } from "node:path"

const MAX_RECEIPT_BYTES = 96 * 1024
const MAX_PRESENCE_BYTES = 32 * 1024

function fail(message) {
  throw new Error(message)
}

function connectOnce(host, port, timeoutMs) {
  return new Promise((resolvePromise) => {
    const socket = connect({ host, port })
    const finish = (connected) => {
      socket.destroy()
      resolvePromise(connected)
    }
    socket.setTimeout(Math.min(timeoutMs, 500), () => finish(false))
    socket.once("connect", () => finish(true))
    socket.once("error", () => finish(false))
  })
}

async function readBoundedJson(path, maximumBytes) {
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW).catch(() => null)
  if (!handle) return null
  try {
    const metadata = await handle.stat()
    if (!metadata.isFile() || metadata.size > maximumBytes) return null
    const bytes = Buffer.alloc(maximumBytes + 1)
    let offset = 0
    while (offset < bytes.length) {
      const { bytesRead } = await handle.read(bytes, offset, bytes.length - offset, offset)
      if (bytesRead === 0) break
      offset += bytesRead
    }
    if (offset > maximumBytes) return null
    const value = JSON.parse(bytes.subarray(0, offset).toString("utf8"))
    return value && typeof value === "object" && !Array.isArray(value) ? value : null
  } catch {
    return null
  } finally {
    await handle.close()
  }
}

function validIdentifier(value) {
  return typeof value === "string" && /^[a-z0-9][a-z0-9._:-]{0,127}$/.test(value)
}

async function matchingPresence(receiptPath, presenceRoot, host, port, protocol, notBeforeMs) {
  const receipt = await readBoundedJson(receiptPath, MAX_RECEIPT_BYTES)
  if (
    receipt?.schemaVersion !== 1 ||
    receipt.status !== "confirmed" ||
    !validIdentifier(receipt.kernelId) ||
    !validIdentifier(receipt.machineId)
  ) return false
  const presence = await readBoundedJson(
    join(presenceRoot, `${receipt.kernelId}.json`),
    MAX_PRESENCE_BYTES,
  )
  const now = Date.now()
  const matchingHost = host === "localhost"
    ? new Set(["127.0.0.1", "localhost", "::1"]).has(presence?.host)
    : presence?.host === host
  if (
    presence?.schema_version !== 1 ||
    presence.kernel_id !== receipt.kernelId ||
    presence.machine_id !== receipt.machineId ||
    !matchingHost ||
    presence.port !== port ||
    presence.local_daemon_protocol_version !== protocol ||
    presence.relay_connected !== true ||
    !Number.isSafeInteger(presence.process_id) ||
    presence.process_id < 1 ||
    !Number.isSafeInteger(presence.started_at_ms) ||
    !Number.isSafeInteger(presence.heartbeat_at_ms) ||
    presence.started_at_ms < notBeforeMs ||
    presence.heartbeat_at_ms < presence.started_at_ms ||
    presence.heartbeat_at_ms < notBeforeMs ||
    presence.heartbeat_at_ms > now + 5_000 ||
    now - presence.heartbeat_at_ms > 15_000
  ) return false
  try {
    process.kill(presence.process_id, 0)
  } catch {
    return false
  }
  return connectOnce(host, port, 500)
}

async function checkManagedKernel(host, portText, timeoutText, receiptPath, presenceRoot, protocolText, notBeforeText) {
  if (!new Set(["127.0.0.1", "localhost", "::1"]).has(host)) {
    fail("managed kernel health host must be loopback")
  }
  const port = Number(portText)
  const timeoutMs = Number(timeoutText)
  const protocol = Number(protocolText)
  const notBeforeMs = Number(notBeforeText)
  if (!Number.isInteger(port) || port < 1 || port > 65535) fail("managed kernel health port is invalid")
  if (!Number.isInteger(timeoutMs) || timeoutMs < 100 || timeoutMs > 120_000) {
    fail("managed kernel health timeout is invalid")
  }
  if (!Number.isSafeInteger(protocol) || protocol < 1) fail("managed kernel health protocol is invalid")
  if (!Number.isSafeInteger(notBeforeMs) || notBeforeMs < 1) fail("managed kernel health start time is invalid")
  const deadline = Date.now() + timeoutMs
  do {
    if (await matchingPresence(receiptPath, presenceRoot, host, port, protocol, notBeforeMs)) return
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 100))
  } while (Date.now() < deadline)
  fail("managed kernel did not publish a fresh matching healthy presence")
}

try {
  if (process.argv.length !== 9) {
    fail("usage: check-managed-kernel-health <host> <port> <timeout-ms> <receipt> <presence-root> <protocol> <not-before-ms>")
  }
  await checkManagedKernel(...process.argv.slice(2))
} catch (error) {
  const message = error instanceof Error ? error.message : String(error)
  process.stderr.write(`${basename(process.argv[1])}: ${message}\n`)
  process.exitCode = 1
}
