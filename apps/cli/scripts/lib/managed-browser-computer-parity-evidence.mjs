import { createHash } from "node:crypto"
import { constants } from "node:fs"
import { lstat, open, readdir, realpath } from "node:fs/promises"
import path from "node:path"

import {
  isSensitiveDrillKey,
  looksLikeDrillSecretValue,
} from "./drill-secrets.mjs"

export const MANAGED_BROWSER_COMPUTER_PARITY_EVIDENCE_SCHEMA = "chariox.browser_computer.managed_parity.evidence.v1"

const DEFAULT_LIMITS = Object.freeze({
  maxFiles: 2_048,
  maxFileBytes: 16 * 1024 * 1024,
  maxTotalBytes: 64 * 1024 * 1024,
  maxDepth: 32,
})
const SECRET_NAME_PATTERN = /(?:^|[._-])(?:auth|secret|token|password|credential|cookie|authorization|api[-_]?key|access[-_]?key|private[-_]?key)(?:$|[._-])/i
const SECRET_ASSIGNMENT_PATTERN = /\b(?:auth|secret|token|password|credential|cookie|authorization|api[-_]?key|access[-_]?key)\b\s*[:=]\s*["']?(?!<redacted>|redacted\b|null\b|undefined\b)[^\s,;}\]]{3,}/i

export async function assertManagedParityEvidenceRoot(candidate) {
  const resolved = resolveAbsolutePath(candidate)
  let info
  try {
    info = await lstat(resolved)
  } catch {
    throw evidenceFailure("evidence root is unavailable")
  }
  if (!info.isDirectory() || info.isSymbolicLink()) throw evidenceFailure("evidence root must be a real directory")
  try {
    if (await realpath(resolved) !== resolved) throw evidenceFailure("evidence root must not contain symbolic links")
  } catch (error) {
    if (error instanceof ManagedParityEvidenceError) throw error
    throw evidenceFailure("evidence root is unavailable")
  }
  return resolved
}

export async function enumerateManagedParityEvidenceRoot(root, limits = {}) {
  const resolved = await assertManagedParityEvidenceRoot(root)
  const bounded = normalizeLimits(limits)
  const files = []
  let totalBytes = 0

  const visit = async (directory, relativeDirectory = "", depth = 0) => {
    if (depth > bounded.maxDepth) throw evidenceFailure("evidence root exceeds the directory depth limit")
    let entries
    try {
      entries = await readdir(directory, { withFileTypes: true })
    } catch {
      throw evidenceFailure("evidence root could not be enumerated")
    }
    for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
      const relativePath = relativeDirectory ? `${relativeDirectory}/${entry.name}` : entry.name
      if (!safeRelativePath(relativePath) || looksLikeSecretName(entry.name) || entry.isSymbolicLink()) {
        throw evidenceFailure("evidence root contains an unsafe entry")
      }
      const absolutePath = path.join(directory, entry.name)
      if (entry.isDirectory()) {
        await assertManagedParityEvidenceRoot(absolutePath)
        await visit(absolutePath, relativePath, depth + 1)
        continue
      }
      if (!entry.isFile() || files.length >= bounded.maxFiles) {
        throw evidenceFailure("evidence root contains an unsupported or excessive entry")
      }
      const file = await readEvidenceFile(absolutePath, relativePath, bounded.maxFileBytes)
      totalBytes += file.sizeBytes
      if (totalBytes > bounded.maxTotalBytes) throw evidenceFailure("evidence root exceeds the total size limit")
      files.push(file)
    }
  }

  await visit(resolved)
  if (files.length === 0) throw evidenceFailure("evidence root contains no regular evidence files")
  return {
    schema: MANAGED_BROWSER_COMPUTER_PARITY_EVIDENCE_SCHEMA,
    source: "physical-evidence-root",
    enumerationComplete: true,
    enumeratedFileCount: files.length,
    totalBytes,
    limits: { ...bounded },
    files,
  }
}

export function managedParityEvidenceManifestPath(runDir) {
  const resolved = path.resolve(runDir ?? "")
  const name = path.basename(resolved)
  if (typeof runDir !== "string" || !path.isAbsolute(runDir) || !name || name === "." || name === path.sep) {
    throw evidenceFailure("evidence run directory must be an absolute named directory")
  }
  return path.join(path.dirname(resolved), `.${name}.evidence-manifest.json`)
}

export async function writeManagedParityEvidenceManifest(runDir) {
  const resolved = await assertManagedParityEvidenceRoot(runDir)
  const manifest = await enumerateManagedParityEvidenceRoot(resolved)
  const manifestPath = managedParityEvidenceManifestPath(resolved)
  let handle
  try {
    handle = await open(
      manifestPath,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
      0o600,
    )
    await handle.writeFile(`${JSON.stringify(manifest, null, 2)}\n`, "utf8")
    await handle.sync()
  } catch {
    throw evidenceFailure("evidence manifest could not be written")
  } finally {
    await handle?.close()
  }
  return { manifest, manifestPath }
}

async function readEvidenceFile(filePath, relativePath, maximumBytes) {
  let handle
  try {
    handle = await open(filePath, constants.O_RDONLY | constants.O_NOFOLLOW | (constants.O_NONBLOCK ?? 0))
    const info = await handle.stat()
    if (!info.isFile() || info.isSymbolicLink() || !Number.isSafeInteger(info.size) || info.size < 0 || info.size > maximumBytes) {
      throw evidenceFailure("evidence root contains an unsupported or oversized file")
    }
    const bytes = Buffer.allocUnsafe(info.size)
    let offset = 0
    while (offset < bytes.length) {
      const { bytesRead } = await handle.read(bytes, offset, bytes.length - offset, offset)
      if (bytesRead === 0) throw evidenceFailure("evidence file changed while being scanned")
      offset += bytesRead
    }
    const trailing = Buffer.allocUnsafe(1)
    if ((await handle.read(trailing, 0, 1, offset)).bytesRead !== 0) {
      throw evidenceFailure("evidence file changed while being scanned")
    }
    const contents = bytes.toString("utf8")
    if (looksLikeSecretValue(contents)) throw evidenceFailure("evidence root contains a secret-looking value")
    return {
      relativePath,
      sha256: `sha256:${createHash("sha256").update(bytes).digest("hex")}`,
      sizeBytes: bytes.byteLength,
      scan: { completed: true, forbiddenMatches: 0 },
    }
  } catch (error) {
    if (error instanceof ManagedParityEvidenceError) throw error
    throw evidenceFailure("evidence file could not be scanned")
  } finally {
    await handle?.close()
  }
}

function normalizeLimits(limits) {
  const bounded = {}
  for (const key of Object.keys(DEFAULT_LIMITS)) {
    const value = limits[key] ?? DEFAULT_LIMITS[key]
    if (!Number.isSafeInteger(value) || value < 1 || value > DEFAULT_LIMITS[key]) {
      throw evidenceFailure("evidence scan limits are invalid")
    }
    bounded[key] = value
  }
  return bounded
}

function resolveAbsolutePath(candidate) {
  if (typeof candidate !== "string" || !path.isAbsolute(candidate)) {
    throw evidenceFailure("evidence root must be an absolute path")
  }
  return path.resolve(candidate)
}

function safeRelativePath(value) {
  return typeof value === "string"
    && value.length > 0
    && !value.startsWith("/")
    && !value.includes("\\")
    && value.split("/").every((part) => part.length > 0 && part !== "." && part !== "..");
}

function looksLikeSecretName(value) {
  return isSensitiveDrillKey(value) || SECRET_NAME_PATTERN.test(value)
}

function looksLikeSecretValue(value) {
  return looksLikeDrillSecretValue(value) || SECRET_ASSIGNMENT_PATTERN.test(value)
}

function evidenceFailure(message) {
  return new ManagedParityEvidenceError(message)
}

export class ManagedParityEvidenceError extends Error {
  constructor(message) {
    super(message)
    this.name = "ManagedParityEvidenceError"
    this.code = "evidence_inventory_incomplete"
    this.step = "evidence.inspect"
  }
}
