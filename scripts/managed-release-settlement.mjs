import { createHash, randomUUID } from "node:crypto"
import { createReadStream } from "node:fs"
import { homedir } from "node:os"
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path"
import { lstat, mkdir, open, readFile, readdir, rename, rm, writeFile } from "node:fs/promises"

const BARRIER_SCHEMA_VERSION = 1
const MAX_HISTORY_RECORDS = 16_384

function builderKey(name) {
  return createHash("sha256").update(name).digest("hex")
}

function pidIsRunning(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    return error?.code !== "ESRCH"
  }
}

function isMissing(error) {
  return error?.code === "ENOENT"
}

async function syncDirectory(path) {
  const directory = await open(path, "r")
  try {
    await directory.sync()
  } finally {
    await directory.close()
  }
}

function parseJson(value, description) {
  try {
    return JSON.parse(value)
  } catch {
    throw new Error(`${description} is malformed`)
  }
}

function expectedRunDirectory(stateDirectory, invocationId) {
  if (typeof invocationId !== "string" || !/^[a-f0-9-]{36}$/.test(invocationId)) {
    throw new Error("unresolved build invocation identity is malformed")
  }
  return join(stateDirectory, `run-${invocationId}`)
}

function expectedPendingDirectory(barrier) {
  if (typeof barrier.outputPath !== "string" || !isAbsolute(barrier.outputPath)) {
    throw new Error("unresolved build output path is malformed")
  }
  return join(dirname(barrier.outputPath), `.new-${basename(barrier.outputPath)}-${barrier.invocationId}`)
}

function validateBarrier(barrier, builderName) {
  if (!barrier || typeof barrier !== "object" || Array.isArray(barrier) ||
      barrier.schemaVersion !== BARRIER_SCHEMA_VERSION || barrier.builderName !== builderName ||
      typeof barrier.invocationId !== "string" || !/^[a-f0-9-]{36}$/.test(barrier.invocationId) ||
      typeof barrier.sourceCommit !== "string" || !/^[a-f0-9]{40}$/.test(barrier.sourceCommit) ||
      typeof barrier.sourceTree !== "string" || !/^[a-f0-9]{40}$/.test(barrier.sourceTree) ||
      typeof barrier.sourceDigest !== "string" || !/^sha256:[a-f0-9]{64}$/.test(barrier.sourceDigest) ||
      typeof barrier.sourceDirectory !== "string" || !isAbsolute(barrier.sourceDirectory) ||
      typeof barrier.runDirectory !== "string" || !isAbsolute(barrier.runDirectory) ||
      typeof barrier.outputPath !== "string" || !isAbsolute(barrier.outputPath) ||
      typeof barrier.pendingDirectory !== "string" || !isAbsolute(barrier.pendingDirectory) ||
      typeof barrier.startedAt !== "string" || !Number.isFinite(Date.parse(barrier.startedAt)) ||
      !Array.isArray(barrier.historyBaseline) || barrier.historyBaseline.length > MAX_HISTORY_RECORDS ||
      barrier.historyBaseline.some((record) => typeof record !== "string" || !record || record.length > 256) ||
      !barrier.builderFingerprint || typeof barrier.builderFingerprint !== "object" ||
      !Array.isArray(barrier.builderFingerprint.nodes) || barrier.builderFingerprint.nodes.length === 0) {
    throw new Error("unresolved build barrier is malformed")
  }
  if (barrier.runDirectory !== expectedRunDirectory(dirname(barrier.runDirectory), barrier.invocationId) ||
      barrier.sourceDirectory !== join(barrier.runDirectory, "source") ||
      barrier.pendingDirectory !== expectedPendingDirectory(barrier) ||
      barrier.builderFingerprint.name !== builderName) {
    throw new Error("unresolved build barrier does not match its invocation paths or builder")
  }
  if (barrier.buildRef !== null && typeof barrier.buildRef !== "string") {
    throw new Error("unresolved build reference is malformed")
  }
  return barrier
}

async function readOwner(lockDirectory) {
  const ownerPath = join(lockDirectory, "owner.json")
  try {
    const owner = parseJson(await readFile(ownerPath, "utf8"), "release builder lease")
    if (!owner || !Number.isSafeInteger(owner.pid) || owner.pid <= 0 ||
        typeof owner.token !== "string" || !/^[a-f0-9-]{36}$/.test(owner.token)) {
      throw new Error("release builder lease is malformed")
    }
    return owner
  } catch (error) {
    if (isMissing(error)) return null
    throw error
  }
}

export async function acquireManagedReleaseBuilderLease({ home = process.env.HOME ?? homedir(), builderName }) {
  if (typeof home !== "string" || !isAbsolute(home) || typeof builderName !== "string" || !builderName) {
    throw new Error("managed release state location or builder identity is malformed")
  }
  const stateDirectory = resolve(home, ".local", "state", "chariox", "managed-kernel-release")
  await mkdir(stateDirectory, { recursive: true, mode: 0o700 })
  const state = await lstat(stateDirectory)
  if (!state.isDirectory() || (state.mode & 0o077) !== 0) {
    throw new Error("managed release state directory must be a private directory")
  }

  const key = builderKey(builderName)
  const barrierPath = join(stateDirectory, `builder-${key}.unresolved.json`)
  const lockDirectory = join(stateDirectory, `builder-${key}.lock`)
  try {
    await mkdir(lockDirectory, { mode: 0o700 })
  } catch (error) {
    if (error?.code !== "EEXIST") throw error
    const owner = await readOwner(lockDirectory)
    if (owner && pidIsRunning(owner.pid)) {
      throw new Error("managed release builder already has an active invocation")
    }
    if (!owner) {
      throw new Error("managed release builder lease is incomplete and cannot be reclaimed safely")
    }
    const reaperPath = join(lockDirectory, "reaper.json")
    try {
      await writeFile(reaperPath, JSON.stringify({ pid: process.pid, token: randomUUID() }), {
        flag: "wx",
        mode: 0o600,
      })
    } catch {
      throw new Error("managed release builder lease is already being reclaimed")
    }
    const currentOwner = await readOwner(lockDirectory)
    if (currentOwner?.pid !== owner.pid || currentOwner.token !== owner.token) {
      throw new Error("managed release builder lease changed during reclaim")
    }
    const staleDirectory = `${lockDirectory}.stale-${randomUUID()}`
    try {
      await rename(lockDirectory, staleDirectory)
    } catch {
      throw new Error("managed release builder lease changed during reclaim")
    }
    await rm(staleDirectory, { recursive: true, force: false })
    await mkdir(lockDirectory, { mode: 0o700 })
  }

  const token = randomUUID()
  try {
    await writeFile(join(lockDirectory, "owner.json"), JSON.stringify({ pid: process.pid, token }), {
      flag: "wx",
      mode: 0o600,
    })
  } catch (error) {
    await rm(lockDirectory, { recursive: true, force: true })
    throw error
  }

  return {
    stateDirectory,
    barrierPath,
    async readBarrier() {
      try {
        return validateBarrier(parseJson(await readFile(barrierPath, "utf8"), "unresolved build barrier"), builderName)
      } catch (error) {
        if (isMissing(error)) return null
        throw error
      }
    },
    async writeBarrier(barrier) {
      validateBarrier(barrier, builderName)
      const tempPath = `${barrierPath}.${process.pid}.${randomUUID()}.tmp`
      try {
        const handle = await open(tempPath, "wx", 0o600)
        try {
          await handle.writeFile(JSON.stringify(barrier))
          await handle.sync()
        } finally {
          await handle.close()
        }
        await rename(tempPath, barrierPath)
      } catch (error) {
        await rm(tempPath, { force: true })
        throw error
      }
      await syncDirectory(stateDirectory)
    },
    async removeBarrier() {
      await rm(barrierPath, { force: true })
      await syncDirectory(stateDirectory)
    },
    async removeRunArtifacts(barrier) {
      validateBarrier(barrier, builderName)
      const runDirectory = expectedRunDirectory(stateDirectory, barrier.invocationId)
      const pendingDirectory = expectedPendingDirectory(barrier)
      if (barrier.runDirectory !== runDirectory || barrier.pendingDirectory !== pendingDirectory) {
        throw new Error("unresolved build artifact paths do not match the invocation identity")
      }
      for (const path of [runDirectory, pendingDirectory]) {
        try {
          const metadata = await lstat(path)
          if (metadata.isSymbolicLink()) throw new Error("unresolved build artifact path is a symlink")
          if (!metadata.isDirectory()) throw new Error("unresolved build artifact path is not a directory")
          await rm(path, { recursive: true, force: false })
        } catch (error) {
          if (!isMissing(error)) throw error
        }
      }
    },
    async createRunDirectory(invocationId) {
      const path = expectedRunDirectory(stateDirectory, invocationId)
      await mkdir(path, { mode: 0o700 })
      return path
    },
    async release() {
      const owner = await readOwner(lockDirectory)
      if (owner?.pid === process.pid && owner.token === token) {
        await rm(lockDirectory, { recursive: true, force: false })
      }
    },
  }
}

export function parseBuildHistoryList(output) {
  const records = parseJson(output, "Buildx history list")
  if (!Array.isArray(records) || records.length > MAX_HISTORY_RECORDS) {
    throw new Error("Buildx history list is malformed or exceeds the record limit")
  }
  const ids = []
  const seen = new Set()
  for (const record of records) {
    if (!record || typeof record !== "object" || typeof record.ID !== "string" || !record.ID ||
        record.ID.length > 256 || /[\s/]/.test(record.ID) || seen.has(record.ID)) {
      throw new Error("Buildx history list contains an invalid or duplicate record ID")
    }
    seen.add(record.ID)
    ids.push(record.ID)
  }
  return ids
}

function parseBuildReference(reference, builderName, nodes) {
  if (typeof reference !== "string" || reference.length > 1024) return null
  const [builder, node, id, ...extra] = reference.split("/")
  if (extra.length !== 0 || !id || id.length > 256 || /\s/.test(id) || builder !== builderName) return null
  if (!nodes.some((candidate) => candidate.name === node)) return null
  return { builder, node, id }
}

function inspectMatchesInvocation(record, barrier) {
  if (!record || typeof record !== "object" || Array.isArray(record) ||
      record.Context !== barrier.sourceDirectory || record.Target !== "managed-release-artifacts" ||
      typeof record.StartedAt !== "string" || !Number.isFinite(Date.parse(record.StartedAt))) return false
  const startedAt = Date.parse(record.StartedAt)
  const invocationStart = Date.parse(barrier.startedAt)
  if (startedAt < invocationStart - 5 * 60_000 || startedAt > Date.now() + 5 * 60_000) return false
  return typeof record.Ref === "string" && record.Ref.length > 0 && record.Ref.length <= 256 && !/[\s/]/.test(record.Ref)
}

export function isTerminalBuildHistoryRecord(record) {
  if (!record || typeof record !== "object" || typeof record.Status !== "string" ||
      typeof record.CompletedAt !== "string" || !Number.isFinite(Date.parse(record.CompletedAt))) return false
  if (typeof record.StartedAt !== "string" || !Number.isFinite(Date.parse(record.StartedAt)) ||
      Date.parse(record.CompletedAt) < Date.parse(record.StartedAt)) return false
  return new Set(["completed", "success", "error", "failed", "canceled", "cancelled"]).has(record.Status.toLowerCase())
}

export async function reconcileManagedReleaseBuild({ barrier, currentBuilderFingerprint, historyList, historyInspect }) {
  validateBarrier(barrier, barrier.builderName)
  if (JSON.stringify(currentBuilderFingerprint) !== JSON.stringify(barrier.builderFingerprint)) {
    return { settled: false, reason: "builder fingerprint changed" }
  }

  if (barrier.buildRef !== null) {
    const buildReference = parseBuildReference(barrier.buildRef, barrier.builderName, barrier.builderFingerprint.nodes)
    if (!buildReference) {
      return { settled: false, reason: "build reference does not match the leased builder" }
    }
    const record = await historyInspect(buildReference.id)
    if (record.Ref !== buildReference.id || !inspectMatchesInvocation(record, barrier)) {
      return { settled: false, reason: "build history identity does not match the invocation" }
    }
    return isTerminalBuildHistoryRecord(record)
      ? { settled: true, buildRef: barrier.buildRef, status: record.Status }
      : { settled: false, reason: "build history has no terminal status" }
  }

  const ids = await historyList()
  const baseline = new Set(barrier.historyBaseline)
  const newIds = ids.filter((id) => !baseline.has(id))
  if (newIds.length === 0) return { settled: false, reason: "no completed history record is visible yet" }

  const matches = []
  for (const id of newIds) {
    const record = await historyInspect(id)
    if (record.Ref === id && inspectMatchesInvocation(record, barrier)) matches.push({ id, record })
  }
  if (matches.length !== 1) return { settled: false, reason: "history does not identify exactly one invocation" }
  const [{ id, record }] = matches
  if (!isTerminalBuildHistoryRecord(record)) return { settled: false, reason: "build history has no terminal status" }
  return { settled: true, buildRef: record.Ref, status: record.Status }
}

export function validateBuildMetadata(metadata, builderName, nodes) {
  if (!metadata || typeof metadata !== "object" || Array.isArray(metadata) ||
      !parseBuildReference(metadata["buildx.build.ref"], builderName, nodes)) {
    throw new Error("Buildx result metadata does not contain a reference for the inspected builder")
  }
  return metadata["buildx.build.ref"]
}

export function isStateDirectoryExternal(stateDirectory, repository) {
  const relativePath = relative(resolve(repository), resolve(stateDirectory))
  return relativePath === ".." || relativePath.startsWith(`..${sep}`) || isAbsolute(relativePath)
}

export async function hashManagedReleaseSource(directory) {
  const digest = createHash("sha256")
  async function walk(path, prefix) {
    const entries = await readdir(path, { withFileTypes: true })
    entries.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0)
    for (const entry of entries) {
      const child = join(path, entry.name)
      const relativePath = prefix ? `${prefix}/${entry.name}` : entry.name
      const metadata = await lstat(child)
      if (metadata.isSymbolicLink()) throw new Error("managed release source contains a symlink")
      if (metadata.isDirectory()) {
        digest.update(`directory\0${relativePath}\0`)
        await walk(child, relativePath)
      } else if (metadata.isFile()) {
        digest.update(`file\0${relativePath}\0${metadata.mode & 0o111}\0`)
        for await (const chunk of createReadStream(child)) digest.update(chunk)
        digest.update("\0")
      } else {
        throw new Error("managed release source contains an unsupported filesystem entry")
      }
    }
  }
  const root = await lstat(directory)
  if (root.isSymbolicLink() || !root.isDirectory()) throw new Error("managed release source directory is unavailable")
  await walk(directory, "")
  return `sha256:${digest.digest("hex")}`
}
