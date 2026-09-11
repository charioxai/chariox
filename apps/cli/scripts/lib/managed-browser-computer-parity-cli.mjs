import { createHash } from "node:crypto"
import { constants } from "node:fs"
import { open, realpath } from "node:fs/promises"
import path from "node:path"

const MAX_CONFIG_BYTES = 1024 * 1024
const MAX_REVIEWED_MODULE_BYTES = 4 * 1024 * 1024
const reviewedModuleProofs = new WeakMap()

export function parseManagedBrowserComputerParityArgs(argv, { repoRoot }) {
  const values = new Map()
  const allowed = new Set(["--config", "--transport-module", "--inspector-module", "--evidence-root"])
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index]
    if (!allowed.has(flag)) throw new Error(`unknown managed parity argument ${flag}`)
    const value = argv[++index]
    if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
    values.set(flag, path.resolve(value))
  }
  for (const flag of allowed) {
    if (!values.has(flag)) throw new Error(`${flag} is required`)
  }
  const evidenceRoot = values.get("--evidence-root")
  assertOutsideRepository(evidenceRoot, repoRoot)
  return {
    configPath: values.get("--config"),
    transportModulePath: values.get("--transport-module"),
    inspectorModulePath: values.get("--inspector-module"),
    evidenceRoot,
  }
}

export async function readPrivateManagedParityConfig(configPath) {
  const { bytes, info } = await readBoundedRegularFile(configPath, MAX_CONFIG_BYTES, "managed parity config")
  if ((info.mode & 0o077) !== 0) {
    throw new Error("managed parity config must not be accessible by group or other")
  }
  return JSON.parse(bytes.toString("utf8"))
}

export async function validateManagedParityEvidenceDirectory(candidate, repoRoot) {
  const resolved = path.resolve(candidate)
  let canonical
  try {
    canonical = await realpath(resolved)
  } catch {
    throw new Error("managed parity evidence directory must already exist")
  }
  if (canonical !== resolved) throw new Error("managed parity evidence directory path must not contain symbolic links")
  const handle = await open(resolved, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW)
  await handle.close()
  assertOutsideRepository(canonical, await realpath(repoRoot))
  return canonical
}

export async function verifyManagedParityAdapterModule(modulePath, imported, expected) {
  const { bytes } = await readBoundedRegularFile(modulePath, MAX_REVIEWED_MODULE_BYTES, "managed parity adapter module")
  const sha256 = `sha256:${createHash("sha256").update(bytes).digest("hex")}`
  if (sha256 !== expected?.sha256) throw new Error("managed parity adapter module hash does not match reviewed pin")
  const identity = imported?.MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY
  if (identity !== expected?.identity) throw new Error("managed parity adapter module identity does not match reviewed pin")
  return { identity, sha256, verifiedBy: "chariox-harness-loader" }
}

export async function loadReviewedManagedParityAdapterModule(modulePath, expected) {
  if (arguments.length !== 2) throw new Error("managed parity loader injection is forbidden")
  return loadReviewedManagedParityModule(modulePath, expected, "adapter")
}

export async function loadReviewedManagedParityInspectorModule(modulePath, expected) {
  if (arguments.length !== 2) throw new Error("managed parity loader injection is forbidden")
  return loadReviewedManagedParityModule(modulePath, expected, "inspector")
}

export function isReviewedManagedParityModuleVerification(verification, role) {
  const stored = reviewedModuleProofs.get(verification)
  return stored?.role === role && stored.identity === verification?.identity && stored.sha256 === verification?.sha256
}

async function loadReviewedManagedParityModule(modulePath, expected, role) {
  const { bytes } = await readBoundedRegularFile(modulePath, MAX_REVIEWED_MODULE_BYTES, `managed parity ${role} module`)
  const sha256 = `sha256:${createHash("sha256").update(bytes).digest("hex")}`
  if (sha256 !== expected?.sha256) throw new Error(`managed parity ${role} module hash does not match reviewed pin`)
  const imported = await import(`data:text/javascript;base64,${bytes.toString("base64")}`)
  const identityExport = role === "adapter"
    ? "MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY"
    : "MANAGED_BROWSER_COMPUTER_PARITY_INSPECTOR_IDENTITY"
  const identity = imported?.[identityExport]
  if (identity !== expected?.identity) throw new Error(`managed parity ${role} module identity does not match reviewed pin`)
  const verification = Object.freeze({ identity, sha256, verifiedBy: "chariox-harness-loader" })
  reviewedModuleProofs.set(verification, { identity, sha256, role })
  return { imported, verification }
}

async function readBoundedRegularFile(candidate, maximumBytes, label) {
  const resolved = path.resolve(candidate)
  let canonical
  try {
    canonical = await realpath(resolved)
  } catch {
    throw new Error(`${label} must be a regular file`)
  }
  if (canonical !== resolved) throw new Error(`${label} path must not contain symbolic links`)
  const handle = await open(resolved, constants.O_RDONLY | constants.O_NOFOLLOW)
  try {
    const info = await handle.stat()
    if (!info.isFile()) throw new Error(`${label} must be a regular file`)
    if (!Number.isSafeInteger(info.size) || info.size < 1 || info.size > maximumBytes) throw new Error(`${label} exceeds the bounded size limit`)
    const bytes = Buffer.allocUnsafe(info.size)
    let offset = 0
    while (offset < bytes.length) {
      const { bytesRead } = await handle.read(bytes, offset, bytes.length - offset, offset)
      if (bytesRead === 0) throw new Error(`${label} changed while being read`)
      offset += bytesRead
    }
    const trailing = Buffer.allocUnsafe(1)
    if ((await handle.read(trailing, 0, 1, offset)).bytesRead !== 0) throw new Error(`${label} changed while being read`)
    return { bytes, info }
  } finally {
    await handle.close()
  }
}

function assertOutsideRepository(candidate, repoRoot) {
  const relative = path.relative(path.resolve(repoRoot), path.resolve(candidate))
  if (relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative))) {
    throw new Error("managed parity evidence root must stay outside the repository")
  }
}
