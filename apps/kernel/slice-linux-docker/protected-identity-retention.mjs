import { constants, openSync, closeSync, fstatSync, readFileSync, writeFileSync, fsyncSync, mkdirSync, readdirSync, lstatSync } from "node:fs"
import { join, dirname, basename, parse } from "node:path"
import { createECDH, createPrivateKey, createPublicKey, randomBytes, sign, verify, timingSafeEqual } from "node:crypto"
import { verifyPrivateHostDirectory } from "./protected-host-root.mjs"
import { requireRetainedRuntimeIdentity, writeProtectedLayoutReceipt, readProtectedLayoutReceipt } from "./protected-layout-store.mjs"

function refuse() { throw new Error("Protected identity retention proof is unavailable; this slice has not started") }
function readPrivateFile(path, owner) {
  if (process.platform !== "linux") refuse()
  const descriptors = []
  try {
    let directory = openSync(parse(path).root, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW)
    descriptors.push(directory)
    for (const component of dirname(path).slice(1).split("/")) {
      directory = openSync(`/proc/${process.pid}/fd/${directory}/${component}`, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW)
      descriptors.push(directory)
      const metadata = fstatSync(directory)
      if (!metadata.isDirectory() || (metadata.uid !== 0 && metadata.uid !== owner) || (metadata.mode & 0o022) !== 0) refuse()
    }
    const parent = fstatSync(directory)
    if (parent.uid !== owner || (parent.mode & 0o077) !== 0) refuse()
    const fd = openSync(`/proc/${process.pid}/fd/${directory}/${basename(path)}`, constants.O_RDONLY | constants.O_NOFOLLOW)
    descriptors.push(fd)
    const metadata = fstatSync(fd)
    if (!metadata.isFile() || metadata.uid !== owner || metadata.nlink !== 1 || (metadata.mode & 0o777) !== 0o600 || metadata.size > 1024 * 1024) refuse()
    return readFileSync(fd)
  } catch { refuse() }
  finally { for (const fd of descriptors.reverse()) closeSync(fd) }
}
function parsePrivateDocument(bytes) {
  try { return JSON.parse(bytes) } catch { refuse() }
}
export function validateBootSelection({identity, registry, machine, host, port, expected}) {
  if (host !== "127.0.0.1" || !Number.isInteger(port) || port < 1 || port > 65535) refuse()
  const endpoint = `${host}:${port}`
  if (!machine || typeof machine.machine_id !== "string" || !machine.machine_id.trim()
      || registry?.version !== 1 || registry.machine_id !== machine.machine_id
      || !registry.kernels || Array.isArray(registry.kernels)
      || Object.keys(registry.kernels).join() !== endpoint) refuse()
  const selected = registry.kernels[endpoint]
  if (!identity || !selected || identity.kernel_id !== selected.kernel_id
      || identity.host !== host || selected.host !== host || identity.port !== port || selected.port !== port
      || typeof selected.kernel_id !== "string" || !/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,179}$/.test(selected.kernel_id)
      || typeof selected.relay_public_key !== "string" || !selected.relay_public_key
      || typeof selected.relay_private_key !== "string" || !selected.relay_private_key
      || identity.relay_public_key !== selected.relay_public_key
      || identity.relay_private_key !== selected.relay_private_key) refuse()
  if (expected && (expected.kernelId !== selected.kernel_id || expected.machineId !== machine.machine_id
      || expected.relayPublicKey !== selected.relay_public_key || expected.host !== host || expected.port !== port)) refuse()
  return {kernelId: selected.kernel_id, machineId: machine.machine_id, relayPublicKey: selected.relay_public_key, host, port}
}
function readBootDocuments(root, identityPath, owner) {
  const buffers = []
  try {
    for (const relative of [identityPath, "kernel/kernels/registry.json", "kernel/machine/identity.json"]) {
      buffers.push(readPrivateFile(join(root, relative), owner))
    }
    return {identity: parsePrivateDocument(buffers[0]), registry: parsePrivateDocument(buffers[1]), machine: parsePrivateDocument(buffers[2])}
  } finally { buffers.forEach(buffer => buffer.fill(0)) }
}
function verifyRestoredIdentity(original, restored) {
  if (original.kernel_id !== restored.kernel_id || original.relay_public_key !== restored.relay_public_key) refuse()
  const originalSecret = Buffer.from(original.relay_private_key ?? "", "base64")
  const secret = Buffer.from(restored.relay_private_key ?? "", "base64")
  try {
    if (secret.length !== 32 || originalSecret.length !== 32 || !timingSafeEqual(secret, originalSecret)) refuse()
    const curve = createECDH("prime256v1")
    curve.setPrivateKey(secret)
    const publicBytes = curve.getPublicKey(undefined, "uncompressed")
    if (publicBytes.toString("base64") !== restored.relay_public_key) refuse()
    const key = createPrivateKey({format: "jwk", key: {
      kty: "EC", crv: "P-256", d: secret.toString("base64url"),
      x: publicBytes.subarray(1, 33).toString("base64url"), y: publicBytes.subarray(33).toString("base64url"),
    }})
    const challenge = randomBytes(32)
    if (!verify("sha256", challenge, createPublicKey(key), sign("sha256", challenge, key))) refuse()
    return {kernelId: restored.kernel_id, relayPublicKey: restored.relay_public_key}
  } finally { secret.fill(0); originalSecret.fill(0) }
}

// Host-only first-use barrier. The destination is a durable protected backup
// root, never an image, workspace, broker scratch directory or evidence path.
// Private material stays in these files and process memory; return public proof.
export function retainFreshIdentity({privateRoot, backupRoot, sliceId, dataOwner, port}) {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,179}$/.test(sliceId)) refuse()
  verifyPrivateHostDirectory(privateRoot, dataOwner)
  verifyPrivateHostDirectory(backupRoot, process.getuid())
  const kernels = join(privateRoot, "kernel/kernels")
  verifyPrivateHostDirectory(kernels, dataOwner)
  const names = readdirSync(kernels).filter(name => name !== "registry.json")
  if (names.length !== 1 || !/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,179}$/.test(names[0])) refuse()
  if (!lstatSync(join(kernels, names[0])).isDirectory()) refuse()
  const identityPath = `kernel/kernels/${names[0]}/identity.json`
  const files = [identityPath, "kernel/kernels/registry.json", "kernel/machine/identity.json"]
  requireRetainedRuntimeIdentity(privateRoot, [identityPath, "kernel/kernels/registry.json"], dataOwner)
  // Exclusive creation means interrupted or previous retention is never replaced.
  const destination = join(backupRoot, sliceId)
  mkdirSync(destination, {mode: 0o700})
  for (const directory of ["kernel", "kernel/kernels", `kernel/kernels/${names[0]}`, "kernel/machine"]) {
    mkdirSync(join(destination, directory), {mode: 0o700})
  }
  for (const relative of files) {
    const contents = readPrivateFile(join(privateRoot, relative), dataOwner)
    try {
      const fd = openSync(join(destination, relative), constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600)
      try { writeFileSync(fd, contents); fsyncSync(fd) } finally { closeSync(fd) }
    } finally { contents.fill(0) }
  }
  for (const directory of [`kernel/kernels/${names[0]}`, "kernel/kernels", "kernel/machine", "kernel", ""]) {
    const fd = openSync(join(destination, directory), constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW)
    try { fsyncSync(fd) } finally { closeSync(fd) }
  }
  const original = readBootDocuments(privateRoot, identityPath, dataOwner)
  const restored = readBootDocuments(destination, identityPath, process.getuid())
  const selected = validateBootSelection({...original, host: "127.0.0.1", port})
  validateBootSelection({...restored, host: "127.0.0.1", port, expected: selected})
  const proof = verifyRestoredIdentity(original.identity, restored.identity)
  const receipt = {version: 1, sliceId, ...selected, ...proof, identityPaths: files,
    backupPath: destination, restorationVerified: true, backupScope: "same-host"}
  writeProtectedLayoutReceipt(backupRoot, sliceId, receipt)
  return receipt
}

export function requireIdentityRetention({privateRoot, backupRoot, sliceId, dataOwner, port}) {
  const receipt = readProtectedLayoutReceipt(backupRoot, sliceId)
  if (receipt.restorationVerified !== true || receipt.backupScope !== "same-host"
      || receipt.backupPath !== join(backupRoot, sliceId)
      || !Array.isArray(receipt.identityPaths)) refuse()
  verifyPrivateHostDirectory(receipt.backupPath, process.getuid())
  const identityPaths = receipt.identityPaths.filter(path => path.endsWith("/identity.json") && path.startsWith("kernel/kernels/"))
  const expectedIdentityPath = `kernel/kernels/${receipt.kernelId}/identity.json`
  const expectedPaths = [expectedIdentityPath, "kernel/kernels/registry.json", "kernel/machine/identity.json"]
  if (identityPaths.length !== 1 || identityPaths[0] !== expectedIdentityPath
      || JSON.stringify(receipt.identityPaths) !== JSON.stringify(expectedPaths)) refuse()
  requireRetainedRuntimeIdentity(privateRoot, identityPaths, dataOwner)
  requireRetainedRuntimeIdentity(receipt.backupPath, identityPaths, process.getuid())
  const original = readBootDocuments(privateRoot, identityPaths[0], dataOwner)
  const restored = readBootDocuments(receipt.backupPath, identityPaths[0], process.getuid())
  if (port !== undefined && port !== receipt.port) refuse()
  validateBootSelection({...original, host: receipt.host, port: receipt.port, expected: receipt})
  validateBootSelection({...restored, host: receipt.host, port: receipt.port, expected: receipt})
  const proof = verifyRestoredIdentity(original.identity, restored.identity)
  if (proof.kernelId !== receipt.kernelId || proof.relayPublicKey !== receipt.relayPublicKey) refuse()
  return receipt
}
