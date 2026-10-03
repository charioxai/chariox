import { createHash, randomUUID } from "node:crypto"
import { createReadStream } from "node:fs"
import { homedir } from "node:os"
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path"
import { link, lstat, mkdir, open, readFile, readdir, rename, rm, rmdir, writeFile } from "node:fs/promises"

const BARRIER_SCHEMA_VERSION = 1
const MAX_HISTORY_RECORDS = 16_384
const MAX_BUILD_REFERENCE_LENGTH = 1024

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

const defaultBarrierFileOps = { open, lstat, readFile, rename, rm }

function barrierWriteError(error, disposition) {
  const wrapped = new Error(error instanceof Error ? error.message : String(error), { cause: error })
  wrapped.barrierDisposition = disposition
  return wrapped
}

async function syncDirectory(path, fileOps = defaultBarrierFileOps) {
  const directory = await fileOps.open(path, "r")
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
  const validDirectoryIdentity = (identity) => identity && typeof identity === "object" &&
    !Array.isArray(identity) && Object.keys(identity).sort().join(",") === "dev,ino" &&
    typeof identity.dev === "string" && /^\d+$/.test(identity.dev) &&
    typeof identity.ino === "string" && /^\d+$/.test(identity.ino)
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
      new Set(barrier.historyBaseline).size !== barrier.historyBaseline.length ||
      barrier.historyBaseline.some((record) => typeof record !== "string" || !record || record.length > MAX_BUILD_REFERENCE_LENGTH) ||
      !barrier.builderFingerprint || typeof barrier.builderFingerprint !== "object" ||
      !Array.isArray(barrier.builderFingerprint.nodes) || barrier.builderFingerprint.nodes.length === 0 ||
      (barrier.buildStarted !== undefined && typeof barrier.buildStarted !== "boolean") ||
      ((barrier.runDirectoryIdentity !== undefined || barrier.pendingDirectoryIdentity !== undefined) &&
        (!validDirectoryIdentity(barrier.runDirectoryIdentity) ||
          !validDirectoryIdentity(barrier.pendingDirectoryIdentity))) ||
      (barrier.buildStarted === false &&
        (barrier.buildRef !== null ||
          !validDirectoryIdentity(barrier.runDirectoryIdentity) ||
          !validDirectoryIdentity(barrier.pendingDirectoryIdentity)))) {
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

async function readReaper(reaperPath) {
  try {
    const reaper = parseJson(await readFile(reaperPath, "utf8"), "release builder reaper")
    const keys = reaper && typeof reaper === "object" && !Array.isArray(reaper)
      ? Object.keys(reaper).sort()
      : []
    if (!reaper || typeof reaper !== "object" || Array.isArray(reaper) ||
        keys.join(",") !== "ownerPid,ownerToken,pid,token" ||
        !Number.isSafeInteger(reaper.pid) || reaper.pid <= 0 ||
        typeof reaper.token !== "string" || !/^[a-f0-9-]{36}$/.test(reaper.token) ||
        !Number.isSafeInteger(reaper.ownerPid) || reaper.ownerPid <= 0 ||
        typeof reaper.ownerToken !== "string" || !/^[a-f0-9-]{36}$/.test(reaper.ownerToken)) {
      throw new Error("release builder reaper is malformed")
    }
    return reaper
  } catch (error) {
    if (isMissing(error)) return null
    throw error
  }
}

function sameOwner(left, right) {
  return left?.pid === right?.pid && left?.token === right?.token
}

function sameReaper(left, right) {
  return left?.pid === right?.pid && left?.token === right?.token &&
    left?.ownerPid === right?.ownerPid && left?.ownerToken === right?.ownerToken
}

function reaperBelongsToOwner(reaper, owner) {
  return reaper.ownerPid === owner.pid && reaper.ownerToken === owner.token
}

async function restoreReaperMarker(stalePath, reaperPath) {
  try {
    await link(stalePath, reaperPath)
    await rm(stalePath, { force: false })
  } catch {}
}

async function removeAbandonedReaperMarker(lockDirectory, owner, observedReaper) {
  if (pidIsRunning(observedReaper.pid)) {
    throw new Error("managed release builder lease is already being reclaimed")
  }
  if (!reaperBelongsToOwner(observedReaper, owner)) {
    throw new Error("managed release builder lease changed during reclaim")
  }

  const reaperPath = join(lockDirectory, "reaper.json")
  const currentOwner = await readOwner(lockDirectory)
  const currentReaper = await readReaper(reaperPath)
  if (!sameOwner(currentOwner, owner) || !sameReaper(currentReaper, observedReaper)) {
    throw new Error("managed release builder lease changed during reclaim")
  }

  const stalePath = `${lockDirectory}.reaper-stale-${randomUUID()}`
  try {
    await rename(reaperPath, stalePath)
  } catch {
    throw new Error("managed release builder lease changed during reclaim")
  }
  let movedReaper
  try {
    movedReaper = await readReaper(stalePath)
  } catch (error) {
    await restoreReaperMarker(stalePath, reaperPath)
    throw error
  }
  if (!sameReaper(movedReaper, observedReaper)) {
    await restoreReaperMarker(stalePath, reaperPath)
    throw new Error("managed release builder lease changed during reclaim")
  }
  if (pidIsRunning(movedReaper.pid)) {
    await restoreReaperMarker(stalePath, reaperPath)
    throw new Error("managed release builder lease is already being reclaimed")
  }

  const ownerAfterMove = await readOwner(lockDirectory)
  if (!sameOwner(ownerAfterMove, owner)) {
    await rm(stalePath, { force: false })
    throw new Error("managed release builder lease changed during reclaim")
  }
  await rm(stalePath, { force: false })
}

async function removeReclaimedLeaseDirectory(staleDirectory, owner, reaper) {
  const entries = (await readdir(staleDirectory)).sort()
  if (entries.length !== 2 || entries[0] !== "owner.json" || entries[1] !== "reaper.json") {
    throw new Error("managed release builder lease contains unexpected files")
  }
  const currentOwner = await readOwner(staleDirectory)
  const currentReaper = await readReaper(join(staleDirectory, "reaper.json"))
  if (!sameOwner(currentOwner, owner) || !sameReaper(currentReaper, reaper) ||
      currentReaper.pid !== process.pid || pidIsRunning(owner.pid)) {
    throw new Error("managed release builder lease changed during reclaim")
  }
  await rm(join(staleDirectory, "owner.json"), { force: false })
  await rm(join(staleDirectory, "reaper.json"), { force: false })
  await rmdir(staleDirectory)
}

async function releaseOwnedLeaseDirectory(lockDirectory, expectedOwner) {
  const owner = await readOwner(lockDirectory)
  if (!sameOwner(owner, expectedOwner)) return
  const entries = await readdir(lockDirectory)
  if (entries.length !== 1 || entries[0] !== "owner.json") {
    throw new Error("managed release builder lease contains unexpected files")
  }
  const currentOwner = await readOwner(lockDirectory)
  if (!sameOwner(currentOwner, expectedOwner)) return
  await rm(join(lockDirectory, "owner.json"), { force: false })
  await rmdir(lockDirectory)
}

export async function acquireManagedReleaseBuilderLease({
  home = process.env.HOME ?? homedir(),
  builderName,
  fileOps = defaultBarrierFileOps,
}) {
  if (typeof home !== "string" || !isAbsolute(home) || typeof builderName !== "string" || !builderName) {
    throw new Error("managed release state location or builder identity is malformed")
  }
  if (!fileOps || typeof fileOps.open !== "function" || typeof fileOps.lstat !== "function" ||
      typeof fileOps.readFile !== "function" || typeof fileOps.rename !== "function" ||
      typeof fileOps.rm !== "function") {
    throw new Error("managed release barrier file operations are malformed")
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
    const abandonedReaper = await readReaper(reaperPath)
    if (abandonedReaper) await removeAbandonedReaperMarker(lockDirectory, owner, abandonedReaper)

    const currentOwner = await readOwner(lockDirectory)
    if (!sameOwner(currentOwner, owner)) {
      throw new Error("managed release builder lease changed during reclaim")
    }
    const reaper = {
      pid: process.pid,
      token: randomUUID(),
      ownerPid: owner.pid,
      ownerToken: owner.token,
    }
    try {
      await writeFile(reaperPath, JSON.stringify(reaper), {
        flag: "wx",
        mode: 0o600,
      })
    } catch {
      throw new Error("managed release builder lease is already being reclaimed")
    }
    const ownerWithMarker = await readOwner(lockDirectory)
    const currentReaper = await readReaper(reaperPath)
    if (!sameOwner(ownerWithMarker, owner) || !sameReaper(currentReaper, reaper)) {
      throw new Error("managed release builder lease changed during reclaim")
    }
    const staleDirectory = `${lockDirectory}.stale-${reaper.token}`
    try {
      await rename(lockDirectory, staleDirectory)
    } catch {
      throw new Error("managed release builder lease changed during reclaim")
    }
    await removeReclaimedLeaseDirectory(staleDirectory, owner, reaper)
    await mkdir(lockDirectory, { mode: 0o700 })
  }

  const token = randomUUID()
  try {
    await writeFile(join(lockDirectory, "owner.json"), JSON.stringify({ pid: process.pid, token }), {
      flag: "wx",
      mode: 0o600,
    })
  } catch (error) {
    try { await rmdir(lockDirectory) } catch {}
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
    async writeBarrier(barrier, { replace = false } = {}) {
      validateBarrier(barrier, builderName)
      const tempPath = `${barrierPath}.${process.pid}.${randomUUID()}.tmp`
      let handle
      let tempIdentity
      let tempCreated = false
      let renameAttempted = false
      let renamed = false
      try {
        handle = await fileOps.open(tempPath, "wx", 0o600)
        tempCreated = true
        const metadata = await handle.stat()
        if (!metadata.isFile()) throw new Error("managed release barrier temporary path is not a regular file")
        tempIdentity = { dev: String(metadata.dev), ino: String(metadata.ino) }
        await handle.writeFile(JSON.stringify(barrier))
        await handle.sync()
        await handle.close()
        handle = undefined

        if (!replace) {
          try {
            await fileOps.lstat(barrierPath)
            throw new Error("managed release unresolved build barrier already exists")
          } catch (error) {
            if (!isMissing(error)) throw error
          }
        }
        renameAttempted = true
        await fileOps.rename(tempPath, barrierPath)
        renamed = true
        await syncDirectory(stateDirectory, fileOps)
      } catch (error) {
        let tempProvenUnpublished = !tempCreated
        let cleanupFailed = false
        if (handle) {
          try {
            await handle.close()
            handle = undefined
          } catch {
            cleanupFailed = true
          }
        }
        if (tempCreated && tempIdentity && !renamed) {
          try {
            const metadata = await fileOps.lstat(tempPath)
            if (metadata.isSymbolicLink() || !metadata.isFile() ||
                String(metadata.dev) !== tempIdentity.dev || String(metadata.ino) !== tempIdentity.ino) {
              throw new Error("managed release barrier temporary path changed during write")
            }
            tempProvenUnpublished = true
            await fileOps.rm(tempPath, { force: false })
          } catch (cleanupError) {
            if (!isMissing(cleanupError)) cleanupFailed = true
          }
        } else if (tempCreated && !renamed) {
          cleanupFailed = true
        }

        let targetDisposition = "unknown"
        try {
          const metadata = await fileOps.lstat(barrierPath)
          if (!metadata.isSymbolicLink() && metadata.isFile()) {
            const current = validateBarrier(
              parseJson(await fileOps.readFile(barrierPath, "utf8"), "unresolved build barrier"),
              builderName,
            )
            targetDisposition = JSON.stringify(current) === JSON.stringify(barrier)
              ? "persisted"
              : "unknown"
          }
        } catch (inspectionError) {
          if (isMissing(inspectionError)) targetDisposition = "absent"
          else cleanupFailed = true
        }
        const disposition = renamed || targetDisposition === "persisted"
          ? "persisted"
          : targetDisposition === "absent" && !cleanupFailed &&
              (!renameAttempted || tempProvenUnpublished)
            ? "absent"
            : "unknown"
        throw barrierWriteError(error, disposition)
      }
    },
    async removeBarrier() {
      await fileOps.rm(barrierPath, { force: true })
      await syncDirectory(stateDirectory, fileOps)
    },
    async removeRunArtifacts(barrier) {
      validateBarrier(barrier, builderName)
      const runDirectory = expectedRunDirectory(stateDirectory, barrier.invocationId)
      const pendingDirectory = expectedPendingDirectory(barrier)
      if (barrier.runDirectory !== runDirectory || barrier.pendingDirectory !== pendingDirectory) {
        throw new Error("unresolved build artifact paths do not match the invocation identity")
      }
      const artifactPaths = [
        [runDirectory, barrier.runDirectoryIdentity],
        [pendingDirectory, barrier.pendingDirectoryIdentity],
      ]
      const existingArtifacts = []
      for (const [path, expectedIdentity] of artifactPaths) {
        try {
          const metadata = await lstat(path)
          if (metadata.isSymbolicLink()) throw new Error("unresolved build artifact path is a symlink")
          if (!metadata.isDirectory()) throw new Error("unresolved build artifact path is not a directory")
          if (expectedIdentity &&
              (String(metadata.dev) !== expectedIdentity.dev || String(metadata.ino) !== expectedIdentity.ino)) {
            throw new Error("unresolved build artifact path changed since invocation")
          }
          if (barrier.buildStarted === false && !expectedIdentity) {
            throw new Error("unresolved prepared build artifact has no ownership identity")
          }
          existingArtifacts.push(path)
        } catch (error) {
          if (!isMissing(error)) throw error
        }
      }
      for (const path of existingArtifacts) {
        await rm(path, { recursive: true, force: false })
      }
    },
    async createRunDirectory(invocationId) {
      const path = expectedRunDirectory(stateDirectory, invocationId)
      await mkdir(path, { mode: 0o700 })
      return path
    },
    async release() {
      await releaseOwnedLeaseDirectory(lockDirectory, { pid: process.pid, token })
    },
  }
}

export function parseBuildHistoryList(output) {
  if (typeof output !== "string") throw new Error("Buildx history list is malformed")
  const contents = output.trim()
  if (!contents) return []
  const rows = contents.split(/\r?\n/)
  if (rows.length > MAX_HISTORY_RECORDS) {
    throw new Error("Buildx history list exceeds the record limit")
  }
  const refs = []
  const seen = new Set()
  for (const row of rows) {
    const record = parseJson(row.trim(), "Buildx history list record")
    if (!record || typeof record !== "object" || Array.isArray(record) ||
        typeof record.ref !== "string" || !record.ref || record.ref.length > MAX_BUILD_REFERENCE_LENGTH ||
        typeof record.name !== "string" || record.name.length > 4096 ||
        typeof record.status !== "string" || !["completed", "error", "running"].includes(record.status.toLowerCase()) ||
        typeof record.created_at !== "string" || !Number.isFinite(Date.parse(record.created_at)) ||
        !Number.isSafeInteger(record.total_steps) || record.total_steps < 0 ||
        !Number.isSafeInteger(record.completed_steps) || record.completed_steps < 0 ||
        !Number.isSafeInteger(record.cached_steps) || record.cached_steps < 0) {
      throw new Error("Buildx history list contains a malformed record")
    }
    const [builder, node, id, ...extra] = record.ref.split("/")
    if (!builder || !node || !id || extra.length !== 0 || /\s/.test(record.ref)) {
      throw new Error("Buildx history list contains a malformed full reference")
    }
    const status = record.status.toLowerCase()
    if (Object.hasOwn(record, "completed_at")) {
      if (typeof record.completed_at !== "string" || !Number.isFinite(Date.parse(record.completed_at)) || status === "running") {
        throw new Error("Buildx history list contains an invalid completion timestamp")
      }
    } else if (status !== "running") {
      throw new Error("Buildx history list is missing a completion timestamp")
    }
    if (seen.has(record.ref)) {
      throw new Error("Buildx history list contains a duplicate full reference")
    }
    seen.add(record.ref)
    refs.push(record.ref)
  }
  return refs
}

function parseBuildReference(reference, builderName, nodes) {
  if (typeof reference !== "string" || reference.length > 1024) return null
  const [builder, node, id, ...extra] = reference.split("/")
  if (extra.length !== 0 || !id || id.length > 256 || /\s/.test(id) || builder !== builderName) return null
  if (!nodes.some((candidate) => candidate.name === node)) return null
  return { builder, node, id }
}

function inspectInvocationBindingFailure(record, barrier, expectedId) {
  if (!record || typeof record !== "object" || Array.isArray(record)) {
    return "Buildx history inspect did not return a JSON record"
  }
  if (record.Ref !== expectedId) return "Buildx history inspect reference does not match the listed build"
  if (typeof record.Context !== "string" || !record.Context ||
      resolve(barrier.sourceDirectory, record.Context) !== barrier.sourceDirectory) {
    return "Buildx history inspect is missing or mismatches the invocation source context binding"
  }
  if (record.Target !== "managed-release-artifacts") {
    return "Buildx history inspect is missing or mismatches the managed release target binding"
  }
  if (typeof record.StartedAt !== "string" || !Number.isFinite(Date.parse(record.StartedAt))) {
    return "Buildx history inspect is missing the invocation start timestamp"
  }
  const startedAt = Date.parse(record.StartedAt)
  const invocationStart = Date.parse(barrier.startedAt)
  if (startedAt < invocationStart - 5 * 60_000 || startedAt > Date.now() + 5 * 60_000) {
    return "Buildx history inspect start timestamp does not match the invocation"
  }
  return null
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
  if (barrier.buildStarted === false) {
    return { settled: true, buildRef: null, status: "not_started" }
  }

  if (barrier.buildRef !== null) {
    const buildReference = parseBuildReference(barrier.buildRef, barrier.builderName, barrier.builderFingerprint.nodes)
    if (!buildReference) {
      return { settled: false, reason: "build reference does not match the leased builder" }
    }
    const record = await historyInspect(buildReference.id, barrier.sourceDirectory)
    const bindingFailure = inspectInvocationBindingFailure(record, barrier, buildReference.id)
    if (bindingFailure) return { settled: false, reason: bindingFailure }
    return isTerminalBuildHistoryRecord(record)
      ? { settled: true, buildRef: barrier.buildRef, status: record.Status }
      : { settled: false, reason: "build history has no terminal status" }
  }

  const refs = await historyList()
  if (!Array.isArray(refs) || refs.length > MAX_HISTORY_RECORDS) {
    return { settled: false, reason: "Buildx history list is malformed or exceeds the record limit" }
  }
  const currentRefs = new Set()
  for (const ref of refs) {
    if (!parseBuildReference(ref, barrier.builderName, barrier.builderFingerprint.nodes) || currentRefs.has(ref)) {
      return { settled: false, reason: "Buildx history list reference does not match the leased builder or is duplicated" }
    }
    currentRefs.add(ref)
  }
  const baseline = new Set(barrier.historyBaseline)
  for (const ref of baseline) {
    if (!parseBuildReference(ref, barrier.builderName, barrier.builderFingerprint.nodes)) {
      return { settled: false, reason: "Buildx history baseline does not match the leased builder" }
    }
  }
  const newRefs = refs.filter((ref) => !baseline.has(ref))
  if (newRefs.length === 0) return { settled: false, reason: "no new history record is visible yet" }

  const matches = []
  const bindingFailures = []
  for (const ref of newRefs) {
    const buildReference = parseBuildReference(ref, barrier.builderName, barrier.builderFingerprint.nodes)
    const record = await historyInspect(buildReference.id, barrier.sourceDirectory)
    const bindingFailure = inspectInvocationBindingFailure(record, barrier, buildReference.id)
    if (bindingFailure) bindingFailures.push(bindingFailure)
    else matches.push({ ref, record })
  }
  if (matches.length !== 1) {
    const reason = matches.length === 0 && newRefs.length === 1
      ? bindingFailures[0]
      : "history does not identify exactly one invocation"
    return { settled: false, reason }
  }
  const [{ ref, record }] = matches
  if (!isTerminalBuildHistoryRecord(record)) return { settled: false, reason: "build history has no terminal status" }
  return { settled: true, buildRef: ref, status: record.Status }
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
