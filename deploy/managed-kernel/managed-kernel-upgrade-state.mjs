#!/usr/bin/env node

import { lstat, open, readFile, rename, symlink, unlink, writeFile } from "node:fs/promises"
import { basename, dirname, resolve } from "node:path"

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

function fail(message) {
  throw new Error(message)
}

function validDigest(value) {
  return typeof value === "string" && /^sha256:[a-f0-9]{64}$/.test(value)
}

function validIdentifier(value) {
  return typeof value === "string" && /^[a-z0-9][a-z0-9._:-]{0,127}$/.test(value)
}

async function readReceipt(path, expectedDigest) {
  const metadata = await lstat(path, { bigint: true }).catch((error) =>
    fail(`managed bootstrap receipt cannot be read: ${error.message}`),
  )
  if (!metadata.isFile() || metadata.size > BigInt(MAX_RECEIPT_BYTES)) {
    fail("managed bootstrap receipt must be a bounded regular file")
  }
  let receipt
  try {
    receipt = JSON.parse(await readFile(path, "utf8"))
  } catch {
    fail("managed bootstrap receipt is invalid JSON")
  }
  if (!receipt || typeof receipt !== "object" || Array.isArray(receipt)) {
    fail("managed bootstrap receipt is invalid")
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
  return receipt
}

async function fsyncDirectory(path) {
  const handle = await open(path, "r")
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
  if (operation === "prepare-receipt" && values.length === 4) {
    const [source, expectedCurrent, target, destination] = values.map((value, index) =>
      index === 1 || index === 2 ? value : resolve(value),
    )
    if (!validDigest(expectedCurrent) || !validDigest(target) || expectedCurrent === target) {
      fail("managed release digest transition is invalid")
    }
    const receipt = await readReceipt(source, expectedCurrent)
    receipt.runtimeReleaseDigest = target
    await writeFile(destination, `${JSON.stringify(receipt, null, 2)}\n`, { flag: "wx", mode: 0o600 })
    return
  }
  if (operation === "validate-receipt" && values.length === 2) {
    if (!validDigest(values[1])) fail("expected managed release digest is invalid")
    await readReceipt(resolve(values[0]), values[1])
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
  if (operation === "atomic-symlink" && values.length === 2) {
    await atomicSymlink(values[0], resolve(values[1]))
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
