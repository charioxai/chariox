#!/usr/bin/env node

// Capture only a current, scoped relay registration observation through the home kernel.
import { resolve } from "node:path"
import { pathToFileURL } from "node:url"
import { validateCaptureOutput, writeCaptureOutput } from "./path1-provider-rebuild-capture.mjs"

export const PATH1_FRESH_RELAY_CAPTURE_SCHEMA = "chariox.path1-fresh-relay-capture/v1"
export const PATH1_FRESH_RELAY_MINIMUM_PROTOCOL_VERSION = 346

const REQUEST_VARIANT = "QueryFreshRemoteMachineKernels"
const RESPONSE_VARIANT = "FreshRemoteMachineKernelsObserved"
const DEFAULT_TIMEOUT_MS = 15_000
const MAX_TIMEOUT_MS = 30_000
const MAX_KERNELS = 256
const MAX_TEXT_LENGTH = 128
const SAFE_TEXT = /^[A-Za-z0-9][A-Za-z0-9._:/ -]{0,127}$/
const SKEW_ALLOWANCE_MS = 1_000

function requireValue(condition, message) {
  if (!condition) throw new Error(message)
}

function record(value, label) {
  requireValue(value !== null && typeof value === "object" && !Array.isArray(value)
    && [Object.prototype, null].includes(Object.getPrototypeOf(value)), `${label} is invalid`)
  return value
}

function safeText(value, label) {
  requireValue(typeof value === "string" && value.length <= MAX_TEXT_LENGTH && SAFE_TEXT.test(value), `${label} is invalid`)
  return value
}

function optionalText(value, label) {
  if (value == null) return null
  return safeText(value, label)
}

function timestamp(value, label) {
  requireValue(Number.isSafeInteger(value) && value >= 0, `${label} is invalid`)
  return value
}

function validateLoopbackEndpoint(value) {
  requireValue(typeof value === "string", "kernel endpoint must be a loopback WebSocket URL")
  let url
  try {
    url = new URL(value)
  } catch {
    throw new Error("kernel endpoint must be a loopback WebSocket URL")
  }
  requireValue(url.protocol === "ws:" && ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname)
    && /^[1-9][0-9]{0,4}$/.test(url.port) && Number(url.port) <= 65_535
    && ["", "/"].includes(url.pathname) && !url.username && !url.password && !url.search && !url.hash,
  "kernel endpoint must be a loopback WebSocket URL")
  return `ws://${url.host}`
}

export function parsePath1FreshRelayCaptureArgs(argv) {
  const allowed = new Set(["--kernel", "--machine", "--output"])
  requireValue(Array.isArray(argv) && argv.length === 6, "usage: --kernel <loopback-url> --machine <machine-ref> --output <absolute-path>")
  const flags = new Map()
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index]
    const value = argv[index + 1]
    requireValue(allowed.has(flag) && !flags.has(flag) && typeof value === "string" && value.length > 0,
      "invalid fresh relay capture arguments")
    flags.set(flag, value)
  }
  requireValue([...allowed].every((flag) => flags.has(flag)), "required flags: --kernel --machine --output")
  return {
    kernelEndpoint: validateLoopbackEndpoint(flags.get("--kernel")),
    machineRef: safeText(flags.get("--machine"), "machine reference"),
    outputPath: flags.get("--output"),
  }
}

function scopedKernel(value, resolvedMachineRef) {
  record(value, "relay kernel registration")
  const kernelId = safeText(value.kernel_id, "kernel id")
  const machineId = safeText(value.machine_id, "kernel machine id")
  const machineAlias = optionalText(value.machine_alias, "machine alias")
  const relayAlias = optionalText(value.relay_alias, "relay alias")
  const kernelAlias = optionalText(value.kernel_alias, "kernel alias")
  requireValue(machineId === resolvedMachineRef || machineAlias === resolvedMachineRef || relayAlias === resolvedMachineRef,
    "fresh relay response contains a kernel outside the resolved machine scope")
  return { kernelId, machineId, machineAlias, relayAlias, kernelAlias }
}

function responsePayload(response) {
  record(response, "fresh relay response")
  const variants = Object.keys(response)
  requireValue(variants.length === 1 && variants[0] === RESPONSE_VARIANT,
    "kernel did not return the fresh relay observation response")
  const payload = record(response[RESPONSE_VARIANT], "fresh relay observation")
  const expected = ["machine_ref", "query_started_at_ms", "query_completed_at_ms", "kernels"]
  requireValue(Object.keys(payload).length === expected.length && expected.every((key) => Object.hasOwn(payload, key)),
    "fresh relay observation has an unexpected shape")
  return payload
}

export async function capturePath1FreshRelayObservation({
  client,
  machineRef,
  send = (request) => client.send(request),
  now = Date.now,
  timeoutMs = DEFAULT_TIMEOUT_MS,
}) {
  safeText(machineRef, "machine reference")
  requireValue(client && typeof client.send === "function" && typeof send === "function", "local kernel client is required")
  requireValue(Number.isSafeInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= MAX_TIMEOUT_MS,
    "capture timeout is outside the allowed bound")
  const queryStartedLocallyAtMs = timestamp(now(), "local query start time")
  let timer
  let response
  try {
    response = await Promise.race([
      send({ [REQUEST_VARIANT]: { machine_ref: machineRef } }),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("fresh relay query timed out")), timeoutMs)
      }),
    ])
  } catch (error) {
    const message = error instanceof Error ? error.message : ""
    const unsupportedVariant = message.toLowerCase().includes("unknown variant") && message.includes(REQUEST_VARIANT)
    if (unsupportedVariant
      || message.includes(`requires kernel protocol ${PATH1_FRESH_RELAY_MINIMUM_PROTOCOL_VERSION} or newer`)) {
      throw new Error(`home kernel requires local daemon protocol ${PATH1_FRESH_RELAY_MINIMUM_PROTOCOL_VERSION} or newer`)
    }
    if (message === "fresh relay query timed out") throw error
    throw new Error("fresh relay observation request failed")
  } finally {
    clearTimeout(timer)
  }

  const capturedAtMs = timestamp(now(), "local query completion time")
  const payload = responsePayload(response)
  const resolvedMachineRef = safeText(payload.machine_ref, "resolved machine reference")
  const queryStartedAtMs = timestamp(payload.query_started_at_ms, "relay query start time")
  const queryCompletedAtMs = timestamp(payload.query_completed_at_ms, "relay query completion time")
  requireValue(queryStartedAtMs <= queryCompletedAtMs
    && queryStartedAtMs >= queryStartedLocallyAtMs - SKEW_ALLOWANCE_MS
    && queryCompletedAtMs <= capturedAtMs + SKEW_ALLOWANCE_MS
    && capturedAtMs - queryCompletedAtMs <= timeoutMs + SKEW_ALLOWANCE_MS
    && queryCompletedAtMs - queryStartedAtMs <= timeoutMs,
  "fresh relay observation timestamps are outside the bounded query interval")
  requireValue(Array.isArray(payload.kernels) && payload.kernels.length <= MAX_KERNELS,
    "fresh relay observation kernel list is invalid")
  const kernels = payload.kernels.map((kernel) => scopedKernel(kernel, resolvedMachineRef))
  const kernelIds = new Set(kernels.map((kernel) => kernel.kernelId))
  requireValue(kernelIds.size === kernels.length, "fresh relay observation contains duplicate kernel registrations")
  kernels.sort((left, right) => left.kernelId.localeCompare(right.kernelId))

  return {
    schema: PATH1_FRESH_RELAY_CAPTURE_SCHEMA,
    minimumProtocolVersion: PATH1_FRESH_RELAY_MINIMUM_PROTOCOL_VERSION,
    capturedAtMs,
    scope: { requestedMachineRef: machineRef, resolvedMachineRef, queryStartedAtMs, queryCompletedAtMs },
    kernels,
    limitation: "Current scoped relay registration observation only; it does not prove historical heartbeat-ID absence or full MP-10 acceptance.",
  }
}

async function main() {
  const args = parsePath1FreshRelayCaptureArgs(process.argv.slice(2))
  const output = await validateCaptureOutput(args.outputPath)
  const [{ LocalIpcClient }, { sendWithProtocolMinimum }] = await Promise.all([
    import("../../../packages/kernel-client/dist/ipc.js"),
    import("../dist/protocol-minimum-diagnostic.js"),
  ])
  const client = new LocalIpcClient(args.kernelEndpoint, { controlRequestRetryDeadlineMs: DEFAULT_TIMEOUT_MS })
  try {
    const send = (request) => sendWithProtocolMinimum(
      (value) => client.send(value), request,
      { capability: "fresh relay machine kernel observation", requestVariant: REQUEST_VARIANT,
        minimumProtocolVersion: PATH1_FRESH_RELAY_MINIMUM_PROTOCOL_VERSION },
    )
    const capture = await capturePath1FreshRelayObservation({ client, machineRef: args.machineRef, send })
    await writeCaptureOutput(output, capture)
    process.stdout.write("Captured a current scoped relay registration observation; this is not full MP-10 evidence.\n")
  } finally {
    await client.close()
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(() => {
    process.stderr.write("Path-1 fresh relay capture failed; no MP-10 acceptance verdict.\n")
    process.exitCode = 1
  })
}
