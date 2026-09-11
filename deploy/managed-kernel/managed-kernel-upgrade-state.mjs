#!/usr/bin/env node

import { createHash } from "node:crypto"
import { constants } from "node:fs"
import { lstat, open, readFile, readdir, rename, symlink, unlink, writeFile } from "node:fs/promises"
import { basename, dirname, resolve } from "node:path"
import { isDeepStrictEqual } from "node:util"

const MAX_RECEIPT_BYTES = 96 * 1024
const REQUIRED_RECEIPT_KEYS = [
  "confirmedAt",
  "environmentId",
  "kernelId",
  "machineId",
  "relayPublicKey",
  "runtimeReleaseDigest",
  "schemaVersion",
  "status",
]
const OPTIONAL_RECEIPT_KEYS = ["contextPlan"]
const WORKER_RECEIPT_KEYS = [
  "binding", "bindingDigest", "cloudApiUrl", "cloudRelay", "enrollmentReceipt",
  "kind", "relayPublicKey", "schemaVersion", "status",
]
const WORKER_BINDING_KEYS = [
  "allocationId", "expectedHomeKernelId", "userId", "realmId", "workerMachineId",
  "workerKernelId", "imageDigest", "runtimeReleaseDigest", "managerOperationId",
  "managerOperationFence", "managerRequestDigest", "senderKeyThumbprint",
]
const WORKER_ENROLLMENT_KEYS = [
  "grantId", "allocationId", "workerMachineId", "workerKernelId", "imageDigest",
  "runtimeReleaseDigest", "exchangedAt",
]
const WORKER_RELAY_KEYS = [
  "apiUrl", "email", "accountId", "userId", "accountSlug", "realmId", "relayUrl",
  "issuerId", "machineId", "machineAlias", "machineCredential",
]
const RELEASE_OVERRIDE_KEYS = ["bindingDigest", "kind", "runtimeReleaseDigest", "schemaVersion"]
const TRANSITION_POLICY_KEYS = ["protocol", "rollbackTo", "schemaVersion", "upgradeFrom"]
const TRANSITION_POLICY_PATH = "usr/lib/chariox/slice-build-context/apps/kernel/managed-upgrade-protocol-transitions.json"

function fail(message) {
  throw new Error(message)
}

function validDigest(value) {
  return typeof value === "string" && /^sha256:[a-f0-9]{64}$/.test(value)
}

function validIdentifier(value) {
  return typeof value === "string" && /^[a-z0-9][a-z0-9._:-]{0,127}$/.test(value)
}

function validDisposableIdentifier(value) {
  return typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/.test(value)
}

function exactKeys(value, expected) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false
  const actual = Object.keys(value).sort()
  const sortedExpected = [...expected].sort()
  return actual.length === sortedExpected.length
    && actual.every((key, index) => key === sortedExpected[index])
}

function validSecret(value, prefix) {
  return typeof value === "string"
    && value.startsWith(prefix)
    && value.length >= prefix.length + 40
    && /^[A-Za-z0-9_-]+$/.test(value.slice(prefix.length))
}

function validCloudUrl(value) {
  try {
    const url = new URL(value)
    return !url.username && !url.password && !url.search && !url.hash
      && (url.protocol === "https:"
        || (url.protocol === "http:" && new Set(["127.0.0.1", "localhost", "[::1]"]).has(url.hostname)))
  } catch {
    return false
  }
}

function validRelayUrl(value) {
  try {
    const url = new URL(value)
    return !url.username && !url.password && !url.search && !url.hash
      && (url.protocol === "wss:"
        || (url.protocol === "ws:" && new Set(["127.0.0.1", "localhost", "[::1]"]).has(url.hostname)))
  } catch {
    return false
  }
}

function canonicalWorkerBinding(binding) {
  return Object.fromEntries(WORKER_BINDING_KEYS.map((key) => [key, binding[key]]))
}

function validateDisposableWorkerReceipt(receipt) {
  const binding = receipt?.binding
  const enrollment = receipt?.enrollmentReceipt
  const relay = receipt?.cloudRelay
  const bindingDigest = exactKeys(binding, WORKER_BINDING_KEYS)
    ? `sha256:${createHash("sha256").update(JSON.stringify(canonicalWorkerBinding(binding))).digest("hex")}`
    : null
  if (
    !exactKeys(receipt, WORKER_RECEIPT_KEYS)
    || receipt.schemaVersion !== 1
    || receipt.kind !== "disposable_worker"
    || receipt.status !== "exchanged"
    || !validCloudUrl(receipt.cloudApiUrl)
    || typeof receipt.relayPublicKey !== "string"
    || !receipt.relayPublicKey.trim()
    || !validDigest(receipt.bindingDigest)
    || bindingDigest !== receipt.bindingDigest
    || !WORKER_BINDING_KEYS.filter((key) => key !== "managerOperationFence")
      .every((key) => new Set([
        "imageDigest", "runtimeReleaseDigest", "managerRequestDigest", "senderKeyThumbprint",
      ]).has(key)
        ? validDigest(binding[key])
        : validDisposableIdentifier(binding[key]))
    || !Number.isSafeInteger(binding.managerOperationFence)
    || binding.managerOperationFence < 1
    || !exactKeys(enrollment, WORKER_ENROLLMENT_KEYS)
    || !validDisposableIdentifier(enrollment.grantId)
    || enrollment.allocationId !== binding.allocationId
    || enrollment.workerMachineId !== binding.workerMachineId
    || enrollment.workerKernelId !== binding.workerKernelId
    || enrollment.imageDigest !== binding.imageDigest
    || enrollment.runtimeReleaseDigest !== binding.runtimeReleaseDigest
    || typeof enrollment.exchangedAt !== "string"
    || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(enrollment.exchangedAt)
    || !Number.isFinite(Date.parse(enrollment.exchangedAt))
    || !exactKeys(relay, WORKER_RELAY_KEYS)
    || relay.apiUrl !== receipt.cloudApiUrl
    || relay.userId !== binding.userId
    || relay.realmId !== binding.realmId
    || relay.machineId !== binding.workerMachineId
    || !validRelayUrl(relay.relayUrl)
    || !validSecret(relay.machineCredential, "mcred_")
    || ![relay.accountId, relay.accountSlug, relay.issuerId].every(validIdentifier)
    || typeof relay.email !== "string" || !relay.email.trim() || relay.email.length > 320
    || typeof relay.machineAlias !== "string" || !relay.machineAlias.trim() || relay.machineAlias.length > 256
  ) fail("disposable worker bootstrap receipt is invalid")
}

async function readOptionalReleaseOverride(path, bindingDigest) {
  if (!path) return null
  const metadata = await lstat(path).catch((error) => {
    if (error.code === "ENOENT") return null
    throw error
  })
  if (!metadata) return null
  if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.size > MAX_RECEIPT_BYTES) {
    fail("disposable worker release override must be a bounded regular file")
  }
  let value
  try {
    value = JSON.parse(await readFile(path, "utf8"))
  } catch {
    fail("disposable worker release override is invalid JSON")
  }
  if (!exactKeys(value, RELEASE_OVERRIDE_KEYS)
    || value.schemaVersion !== 1
    || value.kind !== "disposable_worker_release"
    || value.bindingDigest !== bindingDigest
    || !validDigest(value.runtimeReleaseDigest)) {
    fail("disposable worker release override is invalid")
  }
  return value
}

async function readReceipt(path, expectedDigest, releaseOverridePath = null) {
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW).catch((error) =>
    fail(`managed bootstrap receipt cannot be read: ${error.message}`),
  )
  let receipt
  let receiptBytes
  try {
    const metadata = await handle.stat({ bigint: true })
    if (!metadata.isFile() || metadata.size > BigInt(MAX_RECEIPT_BYTES)) {
      fail("managed bootstrap receipt must be a bounded regular file")
    }
    const bytes = Buffer.alloc(MAX_RECEIPT_BYTES + 1)
    let offset = 0
    while (offset < bytes.length) {
      const { bytesRead } = await handle.read(bytes, offset, bytes.length - offset, offset)
      if (bytesRead === 0) break
      offset += bytesRead
    }
    if (offset > MAX_RECEIPT_BYTES) fail("managed bootstrap receipt must be a bounded regular file")
    receiptBytes = bytes.subarray(0, offset)
    receipt = JSON.parse(receiptBytes.toString("utf8"))
  } catch {
    fail("managed bootstrap receipt is invalid JSON")
  } finally {
    await handle.close()
  }
  if (!receipt || typeof receipt !== "object" || Array.isArray(receipt)) {
    fail("managed bootstrap receipt is invalid")
  }
  if (receipt.kind === "disposable_worker") {
    validateDisposableWorkerReceipt(receipt)
    const releaseOverride = await readOptionalReleaseOverride(releaseOverridePath, receipt.bindingDigest)
    const effectiveDigest = releaseOverride?.runtimeReleaseDigest ?? receipt.binding.runtimeReleaseDigest
    if (effectiveDigest !== expectedDigest) {
      fail("disposable worker release state does not pin the expected release")
    }
    return { kind: "disposable_worker", receipt, bytes: receiptBytes, releaseOverride }
  }
  if (releaseOverridePath && await lstat(releaseOverridePath).then(() => true, (error) => {
    if (error.code === "ENOENT") return false
    throw error
  })) {
    fail("ordinary managed bootstrap receipt cannot use a disposable worker release override")
  }
  const keys = Object.keys(receipt)
  if (
    REQUIRED_RECEIPT_KEYS.some((key) => !keys.includes(key)) ||
    keys.some((key) => !REQUIRED_RECEIPT_KEYS.includes(key) && !OPTIONAL_RECEIPT_KEYS.includes(key))
  ) {
    fail("managed bootstrap receipt contains unsupported fields")
  }
  if (
    receipt.schemaVersion !== 1 ||
    receipt.status !== "confirmed" ||
    typeof receipt.confirmedAt !== "string" ||
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(receipt.confirmedAt) ||
    !Number.isFinite(Date.parse(receipt.confirmedAt)) ||
    ["environmentId", "machineId", "kernelId"].some((key) => !validIdentifier(receipt[key])) ||
    typeof receipt.relayPublicKey !== "string" ||
    !receipt.relayPublicKey.trim() ||
    !validDigest(receipt.runtimeReleaseDigest)
  ) {
    fail("managed bootstrap receipt is not a confirmed registered-kernel receipt")
  }
  if (receipt.runtimeReleaseDigest !== expectedDigest) {
    fail("managed bootstrap receipt does not pin the expected release")
  }
  return { kind: "managed_environment", receipt, bytes: receiptBytes, releaseOverride: null }
}

async function fsyncDirectory(path) {
  const handle = await open(path, "r")
  try {
    await handle.sync()
  } finally {
    await handle.close()
  }
}

async function syncTree(path) {
  const metadata = await lstat(path)
  if (metadata.isSymbolicLink()) fail("managed kernel durable state contains a symbolic link")
  if (metadata.isDirectory()) {
    for (const entry of await readdir(path)) await syncTree(`${path}/${entry}`)
    await fsyncDirectory(path)
    return
  }
  if (!metadata.isFile()) fail("managed kernel durable state contains an unsupported file type")
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW)
  try {
    await handle.sync()
  } finally {
    await handle.close()
  }
}

async function writeExclusive(path, bytes, mode, owner) {
  const handle = await open(path, "wx", mode)
  try {
    if (owner) {
      await handle.chown(owner.uid, owner.gid)
      await handle.chmod(mode)
    }
    await handle.writeFile(bytes)
    await handle.sync()
  } finally {
    await handle.close()
  }
}

async function atomicFile(source, destination) {
  const sourceMetadata = await lstat(source)
  if (!sourceMetadata.isFile() || sourceMetadata.size > MAX_RECEIPT_BYTES) {
    fail("staged managed bootstrap receipt is invalid")
  }
  const destinationMetadata = await lstat(destination)
  if (!destinationMetadata.isFile()) fail("installed managed bootstrap receipt is invalid")
  const destinationMode = destinationMetadata.mode & 0o777
  if ((destinationMode & 0o022) !== 0) {
    fail("installed managed bootstrap receipt permissions are unsafe")
  }
  const temporary = `${destination}.upgrade-new`
  await unlink(temporary).catch((error) => {
    if (error.code !== "ENOENT") throw error
  })
  await writeExclusive(temporary, await readFile(source), destinationMode, {
    uid: destinationMetadata.uid,
    gid: destinationMetadata.gid,
  })
  await rename(temporary, destination)
  await fsyncDirectory(dirname(destination))
}

async function atomicSidecar(source, destination, ownerTemplate) {
  const sourceMetadata = await lstat(source)
  if (!sourceMetadata.isFile() || sourceMetadata.size > MAX_RECEIPT_BYTES) {
    fail("staged managed kernel sidecar is invalid")
  }
  const templateMetadata = await lstat(ownerTemplate)
  if (!templateMetadata.isFile()) fail("managed kernel sidecar owner template is invalid")
  const destinationMetadata = await lstat(destination).catch((error) => {
    if (error.code === "ENOENT") return null
    throw error
  })
  if (destinationMetadata && (!destinationMetadata.isFile() || destinationMetadata.isSymbolicLink())) {
    fail("installed managed kernel sidecar is invalid")
  }
  const mode = destinationMetadata
    ? destinationMetadata.mode & 0o777
    : templateMetadata.mode & 0o777
  if ((mode & 0o022) !== 0) fail("installed managed kernel sidecar permissions are unsafe")
  const temporary = `${destination}.upgrade-new`
  await unlink(temporary).catch((error) => {
    if (error.code !== "ENOENT") throw error
  })
  await writeExclusive(temporary, await readFile(source), mode, {
    uid: destinationMetadata?.uid ?? templateMetadata.uid,
    gid: destinationMetadata?.gid ?? templateMetadata.gid,
  })
  await rename(temporary, destination)
  await fsyncDirectory(dirname(destination))
}

async function removeStateFile(path) {
  const metadata = await lstat(path).catch((error) => {
    if (error.code === "ENOENT") return null
    throw error
  })
  if (!metadata) return
  if (metadata.isSymbolicLink() || !metadata.isFile()) fail("managed kernel state sidecar is invalid")
  await unlink(path)
  await fsyncDirectory(dirname(path))
}

async function readTransitionPolicy(releaseRoot, expectedProtocol) {
  const path = resolve(releaseRoot, TRANSITION_POLICY_PATH)
  const rootPrefix = `${resolve(releaseRoot)}/`
  if (!path.startsWith(rootPrefix)) fail("managed kernel protocol transition policy path is unsafe")
  const metadata = await lstat(path).catch(() => null)
  if (!metadata || metadata.isSymbolicLink() || !metadata.isFile() || metadata.size > 4096) {
    fail("managed kernel protocol transition policy is missing or invalid")
  }
  let policy
  try {
    policy = JSON.parse(await readFile(path, "utf8"))
  } catch {
    fail("managed kernel protocol transition policy is invalid JSON")
  }
  const validProtocolList = (value) => Array.isArray(value)
    && value.length > 0
    && value.length <= 16
    && value.every((item, index) => Number.isSafeInteger(item) && item > 0
      && (index === 0 || value[index - 1] < item))
  if (!exactKeys(policy, TRANSITION_POLICY_KEYS)
    || policy.schemaVersion !== 1
    || policy.protocol !== expectedProtocol
    || !validProtocolList(policy.upgradeFrom)
    || !validProtocolList(policy.rollbackTo)
    || !policy.upgradeFrom.includes(expectedProtocol)
    || !policy.rollbackTo.includes(expectedProtocol)) {
    fail("managed kernel protocol transition policy is invalid")
  }
  return policy
}

async function validateProtocolTransition(currentRoot, currentProtocol, targetRoot, targetProtocol) {
  if (!Number.isSafeInteger(currentProtocol) || currentProtocol < 1
    || !Number.isSafeInteger(targetProtocol) || targetProtocol < 1) {
    fail("managed kernel protocol transition is invalid")
  }
  if (currentProtocol === targetProtocol) return
  const supported = targetProtocol > currentProtocol
    ? (await readTransitionPolicy(targetRoot, targetProtocol)).upgradeFrom.includes(currentProtocol)
    : (await readTransitionPolicy(currentRoot, currentProtocol)).rollbackTo.includes(targetProtocol)
  if (!supported) {
    fail(`local daemon protocol transition ${currentProtocol} to ${targetProtocol} is not explicitly supported`)
  }
}

async function atomicText(contents, destination) {
  const destinationMetadata = await lstat(destination)
  if (!destinationMetadata.isFile()) fail("managed kernel transaction phase is invalid")
  const temporary = `${destination}.new`
  await unlink(temporary).catch((error) => {
    if (error.code !== "ENOENT") throw error
  })
  await writeExclusive(temporary, Buffer.from(`${contents}\n`), 0o600)
  await rename(temporary, destination)
  await fsyncDirectory(dirname(destination))
}

async function atomicSymlink(target, destination) {
  const destinationMetadata = await lstat(destination)
  if (!destinationMetadata.isSymbolicLink()) fail("installed managed release link is invalid")
  const temporary = `${destination}.upgrade-new`
  await unlink(temporary).catch((error) => {
    if (error.code !== "ENOENT") throw error
  })
  await symlink(target, temporary)
  await rename(temporary, destination)
  await fsyncDirectory(dirname(destination))
}

async function run(args) {
  const [operation, ...values] = args
  if (operation === "prepare-receipt" && values.length === 6) {
    const [source, expectedCurrent, target, destination, releaseOverridePath, targetOverridePath] =
      values.map((value, index) => index === 1 || index === 2 ? value : resolve(value))
    if (!validDigest(expectedCurrent) || !validDigest(target) || expectedCurrent === target) {
      fail("managed release digest transition is invalid")
    }
    const state = await readReceipt(source, expectedCurrent, releaseOverridePath)
    if (state.kind === "managed_environment") {
      state.receipt.runtimeReleaseDigest = target
      await writeFile(destination, `${JSON.stringify(state.receipt, null, 2)}\n`, { flag: "wx", mode: 0o600 })
    } else {
      await writeFile(destination, state.bytes, { flag: "wx", mode: 0o600 })
      await writeFile(targetOverridePath, `${JSON.stringify({
        schemaVersion: 1,
        kind: "disposable_worker_release",
        bindingDigest: state.receipt.bindingDigest,
        runtimeReleaseDigest: target,
      }, null, 2)}\n`, { flag: "wx", mode: 0o600 })
    }
    return
  }
  if (operation === "validate-receipt" && (values.length === 2 || values.length === 3)) {
    if (!validDigest(values[1])) fail("expected managed release digest is invalid")
    await readReceipt(resolve(values[0]), values[1], values[2] ? resolve(values[2]) : null)
    return
  }
  if (operation === "validate-receipt-match" && (values.length === 3 || values.length === 5)) {
    if (!validDigest(values[2])) fail("expected managed release digest is invalid")
    const actual = await readReceipt(
      resolve(values[0]), values[2], values[3] ? resolve(values[3]) : null,
    )
    const expected = await readReceipt(
      resolve(values[1]), values[2], values[4] ? resolve(values[4]) : null,
    )
    if (!isDeepStrictEqual(actual.receipt, expected.receipt)
      || !isDeepStrictEqual(actual.releaseOverride, expected.releaseOverride)) {
      fail("managed bootstrap receipt identity changed during upgrade")
    }
    return
  }
  if (operation === "atomic-file" && values.length === 2) {
    await atomicFile(resolve(values[0]), resolve(values[1]))
    return
  }
  if (operation === "atomic-text" && values.length === 2) {
    await atomicText(values[0], resolve(values[1]))
    return
  }
  if (operation === "atomic-sidecar" && values.length === 3) {
    await atomicSidecar(resolve(values[0]), resolve(values[1]), resolve(values[2]))
    return
  }
  if (operation === "remove-state-file" && values.length === 1) {
    await removeStateFile(resolve(values[0]))
    return
  }
  if (operation === "validate-protocol-transition" && values.length === 4) {
    await validateProtocolTransition(
      resolve(values[0]), Number(values[1]), resolve(values[2]), Number(values[3]),
    )
    return
  }
  if (operation === "atomic-symlink" && values.length === 2) {
    await atomicSymlink(values[0], resolve(values[1]))
    return
  }
  if (operation === "sync-tree" && values.length === 1) {
    await syncTree(resolve(values[0]))
    return
  }
  if (operation === "sync-directory" && values.length === 1) {
    await fsyncDirectory(resolve(values[0]))
    return
  }
  fail("unsupported managed kernel upgrade state operation")
}

try {
  await run(process.argv.slice(2))
} catch (error) {
  const message = error instanceof Error ? error.message : String(error)
  process.stderr.write(`${basename(process.argv[1])}: ${message}\n`)
  process.exitCode = 1
}
