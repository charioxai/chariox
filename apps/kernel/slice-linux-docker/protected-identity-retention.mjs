import { constants, openSync, closeSync, fstatSync, readFileSync, writeFileSync, fsyncSync, mkdirSync, readdirSync, lstatSync } from "node:fs"
import { join } from "node:path"
import { createECDH, createPrivateKey, createPublicKey, randomBytes, sign, verify } from "node:crypto"
import { verifyPrivateHostDirectory } from "./protected-host-root.mjs"
import { requireRetainedRuntimeIdentity, writeProtectedLayoutReceipt, readProtectedLayoutReceipt } from "./protected-layout-store.mjs"

function refuse() { throw new Error("Protected identity retention proof is unavailable; this slice has not started") }
function readPrivateFile(path, owner) {
  const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW)
  try {
    const metadata = fstatSync(fd)
    if (!metadata.isFile() || metadata.uid !== owner || metadata.nlink !== 1 || (metadata.mode & 0o077) !== 0 || metadata.size > 1024 * 1024) refuse()
    return readFileSync(fd)
  } finally { closeSync(fd) }
}
function verifyRestoredIdentity(original, restored) {
  if (original.daemon_id !== restored.daemon_id || original.relay_public_key !== restored.relay_public_key) refuse()
  const secret = Buffer.from(restored.relay_private_key ?? "", "base64")
  try {
    if (secret.length !== 32) refuse()
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
    return {kernelId: restored.daemon_id, relayPublicKey: restored.relay_public_key}
  } finally { secret.fill(0) }
}

// Host-only first-use barrier. The destination is a durable protected backup
// root, never an image, workspace, broker scratch directory or evidence path.
// Private material stays in these files and process memory; return public proof.
export function retainFreshIdentity({privateRoot, backupRoot, sliceId, dataOwner}) {
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
  const originalBytes = readPrivateFile(join(privateRoot, identityPath), dataOwner)
  const restoredBytes = readPrivateFile(join(destination, identityPath), process.getuid())
  try {
    const proof = verifyRestoredIdentity(JSON.parse(originalBytes), JSON.parse(restoredBytes))
    const receipt = {version: 1, sliceId, ...proof, identityPaths: files,
      backupPath: destination, restorationVerified: true, backupScope: "same-host"}
    writeProtectedLayoutReceipt(backupRoot, sliceId, receipt)
    return receipt
  } finally { originalBytes.fill(0); restoredBytes.fill(0) }
}

export function requireIdentityRetention({privateRoot, backupRoot, sliceId, dataOwner}) {
  const receipt = readProtectedLayoutReceipt(backupRoot, sliceId)
  if (receipt.restorationVerified !== true || receipt.backupScope !== "same-host"
      || receipt.backupPath !== join(backupRoot, sliceId)
      || !Array.isArray(receipt.identityPaths)) refuse()
  verifyPrivateHostDirectory(receipt.backupPath, process.getuid())
  const identityPaths = receipt.identityPaths.filter(path => path.endsWith("/identity.json") && path.startsWith("kernel/kernels/"))
  if (identityPaths.length !== 1) refuse()
  requireRetainedRuntimeIdentity(privateRoot, identityPaths, dataOwner)
  requireRetainedRuntimeIdentity(receipt.backupPath, identityPaths, process.getuid())
  const original = readPrivateFile(join(privateRoot, identityPaths[0]), dataOwner)
  const restored = readPrivateFile(join(receipt.backupPath, identityPaths[0]), process.getuid())
  try {
    const proof = verifyRestoredIdentity(JSON.parse(original), JSON.parse(restored))
    if (proof.kernelId !== receipt.kernelId || proof.relayPublicKey !== receipt.relayPublicKey) refuse()
    return receipt
  } finally { original.fill(0); restored.fill(0) }
}
