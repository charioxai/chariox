#!/usr/bin/env node

// MP-07/MP-10: capture Cloud's completed receipt through the normal home kernel.
// This read-only capture is not a fresh-machine or parity acceptance verdict.
import { realpath, writeFile } from "node:fs/promises"
import { basename, dirname, isAbsolute, join, resolve } from "node:path"
import { pathToFileURL } from "node:url"

const DIGEST = /^sha256:[a-f0-9]{64}$/
const COMMIT = /^[a-f0-9]{40}$/
const ID = /^[a-zA-Z0-9][a-zA-Z0-9._:/ -]{0,255}$/
const UUID = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/

function requireValue(condition, message) {
  if (!condition) throw new Error(message)
}

function validId(value) {
  return typeof value === "string" && ID.test(value)
}

function validTimestamp(value) {
  return typeof value === "string" && Number.isFinite(Date.parse(value))
    && new Date(value).toISOString() === value
}

export async function resolveCaptureOutput(output, repo = resolve(import.meta.dirname, "../../..")) {
  requireValue(typeof output === "string" && isAbsolute(output), "capture output must be an absolute external evidence path")
  const canonicalRepo = await realpath(repo)
  const canonicalParent = await realpath(dirname(resolve(output)))
  const canonicalOutput = join(canonicalParent, basename(resolve(output)))
  requireValue(canonicalOutput !== canonicalRepo && !canonicalOutput.startsWith(`${canonicalRepo}/`),
    "capture output must remain outside the repository")
  return canonicalOutput
}

export async function capturePath1CloudReimage({ client, requestContract, binding, now = () => new Date(), timeoutMs = 15_000 }) {
  requireValue(binding && validId(binding.environmentId) && validId(binding.operationId)
    && Number.isSafeInteger(binding.generation) && binding.generation > 1
    && DIGEST.test(binding.releaseDigest) && COMMIT.test(binding.sourceCommit)
    && COMMIT.test(binding.sourceTree), "invalid reviewed reimage binding")
  requireValue(requestContract && typeof requestContract.getManagedEnvironmentReimageReceiptRequest === "function"
    && Number.isSafeInteger(requestContract.minimumProtocolVersion) && requestContract.minimumProtocolVersion > 0,
  "kernel receipt request contract is unavailable")
  requireValue(Number.isSafeInteger(timeoutMs) && timeoutMs > 0 && timeoutMs <= 30_000, "invalid capture timeout")
  let timer
  let response
  try {
    response = await Promise.race([
      client.send(requestContract.getManagedEnvironmentReimageReceiptRequest(binding.environmentId)),
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error("Cloud receipt capture timed out")), timeoutMs) }),
    ])
  } finally {
    clearTimeout(timer)
  }
  const receipt = response?.ManagedEnvironmentReimageReceipt?.receipt
  requireValue(receipt && receipt.environmentId === binding.environmentId
    && receipt.operationId === binding.operationId && receipt.generation === binding.generation
    && receipt.previousGeneration === binding.generation - 1, "Cloud receipt does not match the selected reimage operation")
  requireValue(receipt.status === "fresh_equivalent" && receipt.freshEquivalent === true
    && validId(receipt.receiptId) && DIGEST.test(receipt.receiptDigest), "Cloud reimage receipt is not finalized")
  const source = receipt.sourceEvidence
  const fresh = receipt.runtimeEvidence?.freshnessEvidence
  const old = receipt.runtimeEvidence?.oldKernelIdentityBaseline
  requireValue(receipt.runtimeReleaseDigest === binding.releaseDigest
    && source?.runtimeReleaseDigest === binding.releaseDigest
    && source?.runtimeSourceCommit === binding.sourceCommit
    && source?.runtimeSourceTree === binding.sourceTree
    && fresh?.runtimeReleaseDigest === binding.releaseDigest
    && fresh?.runtimeSourceCommit === binding.sourceCommit
    && fresh?.runtimeSourceTree === binding.sourceTree, "Cloud receipt does not match the reviewed release")
  requireValue(old?.source === "cloud_retained_old_kernel_identity"
    && old.environmentId === binding.environmentId && old.generation === receipt.previousGeneration
    && old.machineId === receipt.oldMachineId && old.kernelId === receipt.oldKernelId
    && UUID.test(old.linuxBootId) && /^[a-f0-9]{32}$/.test(old.osMachineId)
    && UUID.test(fresh.linuxBootId) && /^[a-f0-9]{32}$/.test(fresh.osMachineId)
    && old.linuxBootId !== fresh.linuxBootId && old.osMachineId !== fresh.osMachineId,
  "Cloud receipt lacks rotated host identities")
  requireValue(DIGEST.test(old.runtimeReleaseDigest) && COMMIT.test(old.runtimeSourceCommit)
    && COMMIT.test(old.runtimeSourceTree), "Cloud receipt lacks a valid retained release identity")
  requireValue(validTimestamp(old.observedAt) && validTimestamp(receipt.requestedAt)
    && Date.parse(old.observedAt) <= Date.parse(receipt.requestedAt),
  "Cloud receipt has an invalid retained baseline time")
  for (const kind of ["MachineId", "KernelId", "RelayRealmId", "RelayTargetId", "BootstrapGrantId"]) {
    requireValue(validId(receipt[`old${kind}`]) && validId(receipt[`new${kind}`])
      && receipt[`old${kind}`] !== receipt[`new${kind}`], "Cloud receipt lacks rotated control identities")
  }
  requireValue(/^[1-9][0-9]{0,17}$/.test(receipt.resourceObservation?.rebuildActionId)
    && validId(receipt.providerServerId) && validId(receipt.providerImageId), "Cloud receipt lacks the provider rebuild binding")
  const requiredResidue = ["oldServicesAbsent", "oldProcessesAbsent", "oldStateAbsent",
    "cloudOldMachineRevoked", "cloudOldCredentialsRevoked", "cloudOldTargetsRevoked",
    "cloudOldHeartbeatsAbsent", "cloudOldRelayRealmDisabled", "cloudOldGenerationRetired"]
  requireValue(requiredResidue.every((key) => receipt.residueChecks?.[key] === true)
    && receipt.cleanupState?.oldGenerationRetired === true
    && receipt.cleanupState?.newGenerationEnrolled === true, "Cloud receipt has incomplete retirement evidence")
  const capturedAt = now().toISOString()
  requireValue(validTimestamp(receipt.requestedAt) && validTimestamp(receipt.completedAt)
    && Date.parse(receipt.completedAt) >= Date.parse(receipt.requestedAt)
    && Date.parse(receipt.completedAt) <= Date.parse(capturedAt), "Cloud receipt has invalid completion times")
  // Do not retain free-form evidence, failure text, provider output or credentials.
  return {
    schema: "chariox.path1-cloud-reimage-capture/v1", capturedAt,
    minimumProtocolVersion: requestContract.minimumProtocolVersion,
    environmentId: binding.environmentId, operationId: binding.operationId,
    receiptId: receipt.receiptId, receiptDigest: receipt.receiptDigest,
    generation: receipt.generation, previousGeneration: receipt.previousGeneration,
    providerServerId: receipt.providerServerId, providerImageId: receipt.providerImageId,
    rebuildActionId: receipt.resourceObservation.rebuildActionId,
    release: { digest: binding.releaseDigest, sourceCommit: binding.sourceCommit, sourceTree: binding.sourceTree },
    before: { bootId: old.linuxBootId, machineId: old.osMachineId, observedAt: old.observedAt,
      release: { digest: old.runtimeReleaseDigest, sourceCommit: old.runtimeSourceCommit, sourceTree: old.runtimeSourceTree } },
    after: { bootId: fresh.linuxBootId, machineId: fresh.osMachineId },
    controlIdentities: Object.fromEntries(["MachineId", "KernelId", "RelayRealmId", "RelayTargetId", "BootstrapGrantId"]
      .map((kind) => [kind, { before: receipt[`old${kind}`], after: receipt[`new${kind}`] }])),
    residueChecks: Object.fromEntries(requiredResidue.map((key) => [key, true])),
    requestedAt: receipt.requestedAt, completedAt: receipt.completedAt,
  }
}

async function main() {
  const args = process.argv.slice(2)
  requireValue(args.length === 16, "required flags: --kernel --environment --operation --generation --release --commit --tree --output")
  const flags = new Map()
  const allowed = new Set(["--kernel", "--environment", "--operation", "--generation", "--release", "--commit", "--tree", "--output"])
  for (let index = 0; index < args.length; index += 2) {
    requireValue(allowed.has(args[index]) && !flags.has(args[index]) && args[index + 1], "invalid capture arguments")
    flags.set(args[index], args[index + 1])
  }
  const endpoint = flags.get("--kernel")
  requireValue(/^ws:\/\/(?:127\.0\.0\.1|localhost|\[::1\]):[1-9][0-9]*\/?$/.test(endpoint), "capture requires the reviewed local home kernel loopback endpoint")
  const output = await resolveCaptureOutput(flags.get("--output"))
  const [ipc, requests] = await Promise.all([
    import("../../../packages/kernel-client/dist/ipc.js"),
    import("../../../packages/kernel-client/dist/ipc-managed-environment-requests.js"),
  ])
  const { LocalIpcClient } = ipc
  const client = new LocalIpcClient(endpoint, { controlRequestRetryDeadlineMs: 15_000 })
  try {
    const capture = await capturePath1CloudReimage({ client, binding: {
      environmentId: flags.get("--environment"), operationId: flags.get("--operation"),
      generation: Number(flags.get("--generation")), releaseDigest: flags.get("--release"),
      sourceCommit: flags.get("--commit"), sourceTree: flags.get("--tree"),
    }, requestContract: {
      getManagedEnvironmentReimageReceiptRequest: requests.getManagedEnvironmentReimageReceiptRequest,
      minimumProtocolVersion: requests.managedEnvironmentReimageReceiptMinimumProtocolVersion,
    } })
    await writeFile(output, `${JSON.stringify(capture, null, 2)}\n`, { flag: "wx", mode: 0o600 })
    process.stdout.write("Captured finalized Cloud reimage bindings. Full MP-10 evidence remains required.\n")
  } finally {
    await client.close()
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => {
    const generatedClientDist = new URL("../../../packages/kernel-client/dist/", import.meta.url).href
    const missingBuild = error?.code === "ERR_MODULE_NOT_FOUND"
      && typeof error.url === "string" && error.url.startsWith(generatedClientDist)
    process.stderr.write(missingBuild
      ? "Path-1 Cloud capture requires built kernel-client artifacts; run pnpm --workspace-root run build:kernel-client.\n"
      : "Path-1 Cloud capture failed; no acceptance verdict.\n")
    process.exitCode = 1
  })
}
