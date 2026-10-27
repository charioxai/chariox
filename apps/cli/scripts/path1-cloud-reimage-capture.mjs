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

async function boundedRead(client, request, deadline) {
  requireValue(Date.now() < deadline, "Cloud receipt capture timed out")
  let timer
  try {
    return await Promise.race([
      client.send(request),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("Cloud receipt capture timed out")), Math.max(0, deadline - Date.now()))
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

async function verifyRetainedController({ client, requestContract, binding, receipt, deadline }) {
  requireValue(validId(binding.sharedControllerTargetId)
    && ["getManagedEnvironmentRequest", "relayStatusRequest", "cloudRelayStatusRequest"]
      .every((name) => typeof requestContract[name] === "function"),
  "shared realm requires an explicit controller target and product binding reads")
  const read = (request) => boundedRead(client, request, deadline)
  const before = (await read(requestContract.relayStatusRequest()))?.RelayStatus?.status
  const environment = (await read(requestContract.getManagedEnvironmentRequest(binding.environmentId)))?.ManagedEnvironment?.environment
  const profile = (await read(requestContract.cloudRelayStatusRequest()))?.CloudRelayStatus?.profile
  const after = (await read(requestContract.relayStatusRequest()))?.RelayStatus?.status
  const source = environment?.contextPlan?.source
  requireValue(environment?.environmentId === binding.environmentId
    && environment.runtimeGeneration === binding.generation
    && environment.runtimeReleaseDigest === binding.releaseDigest
    && environment.runtimeMachineId === receipt.newMachineId
    && environment.runtimeKernelId === receipt.newKernelId
    && environment.runtimeRelayRealmId === receipt.newRelayRealmId
    && validId(environment.accountId)
    && source?.sourceTargetId === binding.sharedControllerTargetId
    && source.relayRealmId === receipt.oldRelayRealmId
    && validId(source.machineId) && validId(source.kernelId)
    && source.machineId !== receipt.oldMachineId && source.machineId !== receipt.newMachineId
    && source.kernelId !== receipt.oldKernelId && source.kernelId !== receipt.newKernelId
    && /^[a-f0-9]{64}$/.test(source.keyThumbprint),
  "shared realm controller is not bound to the current managed context")
  requireValue(before?.configured === true && before.connected === true
    && after?.configured === true && after.connected === true
    && before.machine_id === source.machineId && after.machine_id === source.machineId
    && before.daemon_id === source.kernelId && after.daemon_id === source.kernelId
    && profile?.machine_id === source.machineId
    && profile.realm_id === source.relayRealmId && profile.account_id === environment.accountId,
  "shared realm controller does not match the connected local kernel and Cloud account")
  // Never retain the profile, which may contain credentials. These existing
  // reads bracket kernel identity, not physical WebSocket continuity.
  return { sourceTargetId: source.sourceTargetId, machineId: source.machineId,
    kernelId: source.kernelId, relayRealmId: source.relayRealmId, accountId: environment.accountId }
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
  const deadline = Date.now() + timeoutMs
  const response = await boundedRead(client,
    requestContract.getManagedEnvironmentReimageReceiptRequest(binding.environmentId), deadline)
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
  for (const kind of ["MachineId", "KernelId", "RelayTargetId", "BootstrapGrantId"]) {
    requireValue(validId(receipt[`old${kind}`]) && validId(receipt[`new${kind}`])
      && receipt[`old${kind}`] !== receipt[`new${kind}`], "Cloud receipt lacks rotated control identities")
  }
  requireValue(validId(receipt.oldRelayRealmId) && validId(receipt.newRelayRealmId),
    "Cloud receipt lacks relay realm identities")
  const sharedRealm = receipt.oldRelayRealmId === receipt.newRelayRealmId
  requireValue(sharedRealm
    ? receipt.residueChecks?.cloudOldRelayRealmDisabled === false
    : receipt.residueChecks?.cloudOldRelayRealmDisabled === true && binding.sharedControllerTargetId === undefined,
  "Cloud receipt has incomplete realm retirement evidence")
  requireValue(/^[1-9][0-9]{0,17}$/.test(receipt.resourceObservation?.rebuildActionId)
    && validId(receipt.providerServerId) && validId(receipt.providerImageId), "Cloud receipt lacks the provider rebuild binding")
  const requiredResidue = ["oldServicesAbsent", "oldProcessesAbsent", "oldStateAbsent",
    "cloudOldMachineRevoked", "cloudOldCredentialsRevoked", "cloudOldTargetsRevoked",
    "cloudOldHeartbeatsAbsent", "cloudOldGenerationRetired"]
  requireValue(requiredResidue.every((key) => receipt.residueChecks?.[key] === true)
    && receipt.cleanupState?.oldGenerationRetired === true
    && receipt.cleanupState?.newGenerationEnrolled === true, "Cloud receipt has incomplete retirement evidence")
  const retainedController = sharedRealm
    ? await verifyRetainedController({ client, requestContract, binding, receipt, deadline })
    : null
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
    residueChecks: { ...Object.fromEntries(requiredResidue.map((key) => [key, true])),
      cloudOldRelayRealmDisabled: receipt.residueChecks.cloudOldRelayRealmDisabled },
    ...(retainedController ? { retainedController } : {}),
    requestedAt: receipt.requestedAt, completedAt: receipt.completedAt,
  }
}

async function main() {
  const args = process.argv.slice(2)
  requireValue(args.length === 16 || args.length === 18, "required flags: --kernel --environment --operation --generation --release --commit --tree --output; optional: --shared-controller-target")
  const flags = new Map()
  const required = ["--kernel", "--environment", "--operation", "--generation", "--release", "--commit", "--tree", "--output"]
  const allowed = new Set([...required, "--shared-controller-target"])
  for (let index = 0; index < args.length; index += 2) {
    requireValue(allowed.has(args[index]) && !flags.has(args[index]) && args[index + 1], "invalid capture arguments")
    flags.set(args[index], args[index + 1])
  }
  requireValue(required.every((flag) => flags.has(flag)), "missing required capture argument")
  const endpoint = flags.get("--kernel")
  requireValue(/^ws:\/\/(?:127\.0\.0\.1|localhost|\[::1\]):[1-9][0-9]*\/?$/.test(endpoint), "capture requires the reviewed local home kernel loopback endpoint")
  const output = await resolveCaptureOutput(flags.get("--output"))
  const [ipc, requests, relayRequests] = await Promise.all([
    import("../../../packages/kernel-client/dist/ipc.js"),
    import("../../../packages/kernel-client/dist/ipc-managed-environment-requests.js"),
    import("../../../packages/kernel-client/dist/ipc-relay-control-requests.js"),
  ])
  const { LocalIpcClient } = ipc
  const client = new LocalIpcClient(endpoint, { controlRequestRetryDeadlineMs: 0 })
  try {
    const capture = await capturePath1CloudReimage({ client, binding: {
      environmentId: flags.get("--environment"), operationId: flags.get("--operation"),
      generation: Number(flags.get("--generation")), releaseDigest: flags.get("--release"),
      sourceCommit: flags.get("--commit"), sourceTree: flags.get("--tree"),
      ...(flags.has("--shared-controller-target") ? { sharedControllerTargetId: flags.get("--shared-controller-target") } : {}),
    }, requestContract: {
      getManagedEnvironmentReimageReceiptRequest: requests.getManagedEnvironmentReimageReceiptRequest,
      minimumProtocolVersion: requests.managedEnvironmentReimageReceiptMinimumProtocolVersion,
      getManagedEnvironmentRequest: requests.getManagedEnvironmentRequest,
      relayStatusRequest: relayRequests.relayStatusRequest,
      cloudRelayStatusRequest: relayRequests.cloudRelayStatusRequest,
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
