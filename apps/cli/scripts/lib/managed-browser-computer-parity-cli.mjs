import { createHash, randomUUID } from "node:crypto"
import { lstat, open, readFile, unlink } from "node:fs/promises"
import path from "node:path"
import { pathToFileURL } from "node:url"

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
  const info = await lstat(configPath)
  if (!info.isFile() || info.isSymbolicLink()) {
    throw new Error("managed parity config must be a regular file")
  }
  if ((info.mode & 0o077) !== 0) {
    throw new Error("managed parity config must not be accessible by group or other")
  }
  return JSON.parse(await readFile(configPath, "utf8"))
}

export async function verifyManagedParityAdapterModule(modulePath, imported, expected) {
  const info = await lstat(modulePath)
  if (!info.isFile() || info.isSymbolicLink()) {
    throw new Error("managed parity adapter module must be a regular file")
  }
  const bytes = await readFile(modulePath)
  const sha256 = `sha256:${createHash("sha256").update(bytes).digest("hex")}`
  if (sha256 !== expected?.sha256) {
    throw new Error("managed parity adapter module hash does not match reviewed pin")
  }
  const identity = imported?.MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY
  if (identity !== expected?.identity) {
    throw new Error("managed parity adapter module identity does not match reviewed pin")
  }
  return { identity, sha256, verifiedBy: "chariox-harness-loader" }
}

export async function loadReviewedManagedParityAdapterModule(
  modulePath,
  expected,
  load = (filePath) => import(pathToFileURL(filePath).href),
) {
  return loadReviewedManagedParityModule({
    modulePath,
    expected,
    load,
    role: "adapter",
    identityExport: "MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY",
  })
}

export async function loadReviewedManagedParityInspectorModule(
  modulePath,
  expected,
  load = (filePath) => import(pathToFileURL(filePath).href),
) {
  return loadReviewedManagedParityModule({
    modulePath,
    expected,
    load,
    role: "inspector",
    identityExport: "MANAGED_BROWSER_COMPUTER_PARITY_INSPECTOR_IDENTITY",
  })
}

export function isReviewedManagedParityModuleVerification(verification, role) {
  if (!verification || (typeof verification !== "object" && typeof verification !== "function")) return false
  const proof = reviewedModuleProofs.get(verification)
  return proof?.role === role
    && proof.identity === verification.identity
    && proof.sha256 === verification.sha256
}

async function loadReviewedManagedParityModule({ modulePath, expected, load, role, identityExport }) {
  const { snapshotPath, sha256 } = await createReviewedModuleSnapshot(modulePath)
  try {
    if (sha256 !== expected?.sha256) throw new Error(`managed parity ${role} module hash does not match reviewed pin`)
    const imported = await load(snapshotPath)
    const identity = imported?.[identityExport]
    if (identity !== expected?.identity) throw new Error(`managed parity ${role} module identity does not match reviewed pin`)
    const verification = Object.freeze({ identity, sha256, verifiedBy: "chariox-harness-loader" })
    reviewedModuleProofs.set(verification, { role, identity, sha256 })
    return { imported, verification }
  } finally {
    await unlink(snapshotPath, { force: true })
  }
}

async function createReviewedModuleSnapshot(modulePath) {
  const info = await lstat(modulePath)
  if (!info.isFile() || info.isSymbolicLink()) {
    throw new Error("managed parity adapter module must be a regular file")
  }
  const bytes = await readFile(modulePath)
  const sha256 = `sha256:${createHash("sha256").update(bytes).digest("hex")}`
  const extension = path.extname(modulePath)
  const snapshotPath = path.join(
    path.dirname(modulePath),
    `.${path.basename(modulePath)}.${randomUUID()}${extension}`,
  )
  let handle = null
  let created = false
  try {
    handle = await open(snapshotPath, "wx", 0o600)
    created = true
    await handle.writeFile(bytes)
    await handle.sync()
  } catch (error) {
    if (created) await unlink(snapshotPath, { force: true })
    throw error
  } finally {
    await handle?.close()
  }
  return { snapshotPath, sha256 }
}

function assertOutsideRepository(candidate, repoRoot) {
  const relative = path.relative(path.resolve(repoRoot), path.resolve(candidate))
  if (relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative))) {
    throw new Error("managed parity evidence root must stay outside the repository")
  }
}
