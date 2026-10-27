import { spawn } from "node:child_process"
import { createHash } from "node:crypto"
import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  mkdirSync,
  openSync,
} from "node:fs"
import { dirname, join, resolve } from "node:path"
import { createFileSliceDiskQuotaStateStore } from "./slice-disk-quota-state-store.mjs"
import {
  SLICE_DISK_QUOTA_PROTOCOL_VERSION,
  SLICE_DISK_QUOTA_REQUEST_TIMEOUT_MS,
  SLICE_DISK_QUOTA_STATE_PATH,
  sliceDiskQuotaIdentityKey,
  validateSliceDiskQuotaIdentity,
  validateSliceDiskQuotaRequest,
  validateSliceDiskQuotaState,
} from "./slice-disk-quota-contract.mjs"
import {
  hasMatchingSliceDiskQuotaUnboundedProof,
  removeSliceDiskQuotaUnboundedProof,
  writeSliceDiskQuotaUnboundedProof,
} from "./slice-disk-quota-admission.mjs"

const LOCK_HOLDER_SCRIPT = "process.stdout.write('slice-disk-quota-lock-held\\n'); process.stdin.resume()"
const LOCK_RELEASE_TIMEOUT_MS = 5_000

export const SLICE_DISK_QUOTA_COORDINATION_ROOT = join(dirname(SLICE_DISK_QUOTA_STATE_PATH), "coordination")
// Receipts from the prior broker-only directory are intentionally never read after rollout.
export const SLICE_DISK_QUOTA_UNBOUNDED_PROOF_ROOT = join(dirname(SLICE_DISK_QUOTA_STATE_PATH), "unbounded-proofs")

function fail(message) {
  throw new Error(message)
}

function assertPrivateDirectory(path, ownerUid) {
  mkdirSync(path, { recursive: true, mode: 0o700 })
  const metadata = lstatSync(path)
  if (metadata.isSymbolicLink() || !metadata.isDirectory() || metadata.uid !== ownerUid || (metadata.mode & 0o077) !== 0) {
    fail("managed disk quota coordination directory is unsafe")
  }
}

function ensurePrivateLockFile(path, ownerUid) {
  let fd
  try {
    fd = openSync(path, constants.O_CREAT | constants.O_RDWR | constants.O_NOFOLLOW, 0o600)
    const metadata = fstatSync(fd)
    if (!metadata.isFile() || metadata.nlink !== 1 || metadata.uid !== ownerUid || (metadata.mode & 0o077) !== 0) {
      fail("managed disk quota lock file is unsafe")
    }
  } finally {
    if (fd !== undefined) closeSync(fd)
  }
}

function waitForExit(child) {
  return new Promise((resolveExit) => {
    child.once("exit", (code, signal) => resolveExit({ code, signal }))
  })
}

function acquireFlock(path, { flockPath, nodePath, waitMs, signal }) {
  if (process.platform !== "linux") {
    throw new Error("managed disk quota coordination requires Linux flock")
  }
  if (signal?.aborted) return Promise.reject(new Error("managed disk quota coordination was cancelled"))
  const waitSeconds = Math.max(1, Math.ceil(waitMs / 1000))
  const child = spawn(flockPath, [
    "--no-fork",
    "--exclusive",
    "--wait",
    String(waitSeconds),
    path,
    nodePath,
    "-e",
    LOCK_HOLDER_SCRIPT,
  ], {
    cwd: "/",
    env: { PATH: "/usr/bin:/bin", LANG: "C" },
    stdio: ["pipe", "pipe", "ignore"],
  })
  let ready = false
  let exited
  let output = ""
  child.once("spawn", () => {
    if (signal?.aborted) child.kill("SIGTERM")
  })
  const exitPromise = waitForExit(child).then((result) => {
    exited = result
    return result
  })

  const acquired = new Promise((resolveAcquired, rejectAcquired) => {
    let finished = false
    const finish = (error) => {
      if (finished) return
      finished = true
      clearTimeout(timer)
      signal?.removeEventListener("abort", onAbort)
      if (error) rejectAcquired(error)
      else {
        ready = true
        resolveAcquired()
      }
    }
    const onAbort = () => {
      child.kill("SIGTERM")
      finish(new Error("managed disk quota coordination was cancelled"))
    }
    const timer = setTimeout(() => {
      child.kill("SIGTERM")
      finish(new Error("managed disk quota coordination lock timed out"))
    }, waitMs + 2_000)
    if (signal?.aborted) {
      onAbort()
      return
    }
    signal?.addEventListener("abort", onAbort, { once: true })
    child.once("error", () => finish(new Error("managed disk quota coordination lock could not start")))
    child.stdout.on("data", (chunk) => {
      output += chunk.toString("utf8")
      if (output.length > 96) {
        finish(new Error("managed disk quota coordination lock handshake is invalid"))
        child.kill("SIGTERM")
        return
      }
      const newline = output.indexOf("\n")
      if (newline < 0) return
      if (output !== "slice-disk-quota-lock-held\n") {
        finish(new Error("managed disk quota coordination lock handshake is invalid"))
        child.kill("SIGTERM")
        return
      }
      finish()
    })
    child.once("exit", (code) => {
      if (!ready) finish(new Error(code === 1
        ? "managed disk quota coordination lock timed out"
        : "managed disk quota coordination lock exited before acquisition"))
    })
  })

  return acquired.then(() => {
    if (exited) throw new Error("managed disk quota coordination lock exited unexpectedly")
    let released = false
    return {
      isHeld: () => !released && exited === undefined && child.exitCode === null,
      async release() {
        if (released) return
        released = true
        child.stdin.end()
        let timer
        const releaseDeadline = new Promise((_, reject) => {
          timer = setTimeout(() => {
            child.kill("SIGTERM")
            reject(new Error("managed disk quota coordination lock did not release"))
          }, LOCK_RELEASE_TIMEOUT_MS)
        })
        let result
        try {
          result = await Promise.race([exitPromise, releaseDeadline])
        } finally {
          clearTimeout(timer)
        }
        if (result.code !== 0) fail("managed disk quota coordination lock failed to release")
      },
    }
  })
}

export function createSliceDiskQuotaCoordinator({
  stateStore = createFileSliceDiskQuotaStateStore(),
  statePath = SLICE_DISK_QUOTA_STATE_PATH,
  coordinationRoot = SLICE_DISK_QUOTA_COORDINATION_ROOT,
  proofRoot = SLICE_DISK_QUOTA_UNBOUNDED_PROOF_ROOT,
  ownerUid = process.getuid?.() ?? 0,
  flockPath = "/usr/bin/flock",
  nodePath = process.execPath,
  lockWaitMs = SLICE_DISK_QUOTA_REQUEST_TIMEOUT_MS,
} = {}) {
  if (!stateStore || typeof stateStore.loadRequired !== "function" || typeof stateStore.save !== "function") {
    throw new TypeError("managed disk quota coordination requires a strict durable state store")
  }
  if (!Number.isSafeInteger(lockWaitMs) || lockWaitMs <= 0 || lockWaitMs > 2_147_483_647) {
    throw new RangeError("managed disk quota coordination timeout must be a positive bounded integer")
  }
  const resolvedStatePath = resolve(statePath)
  const resolvedCoordinationRoot = resolve(coordinationRoot)
  const resolvedProofRoot = resolve(proofRoot)
  const lockDirectory = (path) => {
    assertPrivateDirectory(dirname(path), ownerUid)
    ensurePrivateLockFile(path, ownerUid)
  }
  const activeLocks = new WeakSet()

  function assertLock(lock, containerName) {
    if (!lock || typeof lock !== "object" || !activeLocks.has(lock) || !lock.lease.isHeld()) {
      fail("managed disk quota coordination lock is not held")
    }
    if (containerName && lock.containerName !== containerName) {
      fail("managed disk quota coordination identity does not match its lock")
    }
  }

  function containerLockPath(containerName) {
    validateSliceDiskQuotaRequest({
      protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
      operation: "ensure_before_start",
      containerName,
    })
    const digest = createHash("sha256").update(containerName).digest("hex")
    return join(resolvedCoordinationRoot, `container-${digest}.lock`)
  }

  async function withPathLock(path, action, signal) {
    lockDirectory(path)
    const lease = await acquireFlock(path, {
      flockPath,
      nodePath,
      waitMs: lockWaitMs,
      signal,
    })
    try {
      return await action(lease)
    } finally {
      await lease.release()
    }
  }

  async function withContainerLock(containerName, action, { signal } = {}) {
    if (typeof action !== "function") throw new TypeError("managed disk quota lock requires an action")
    const path = containerLockPath(containerName)
    return withPathLock(path, async (lease) => {
      const lock = { containerName, lease }
      activeLocks.add(lock)
      try {
        return await action(lock)
      } finally {
        activeLocks.delete(lock)
      }
    }, signal)
  }

  async function withStateLock(lock, action, { signal } = {}) {
    assertLock(lock)
    const path = join(resolvedCoordinationRoot, "allocator-state.lock")
    return withPathLock(path, async (lease) => {
      assertLock(lock)
      if (!lease.isHeld()) fail("managed disk quota state lock was lost")
      return action()
    }, signal)
  }

  async function readValidatedState(lock) {
    assertLock(lock)
    return withStateLock(lock, () => validateSliceDiskQuotaState(stateStore.loadRequired()))
  }

  async function hasReservation(lock, containerName = lock?.containerName) {
    assertLock(lock, containerName)
    const state = await readValidatedState(lock)
    return Object.values(state.reservations).some((record) => record.identity.containerName === containerName)
  }

  async function assertUnbounded(lock, { allowMissing = false } = {}) {
    assertLock(lock)
    let state
    try {
      state = await readValidatedState(lock)
    } catch (error) {
      // An online allocator may preserve a normal unbounded operation when state has never been written;
      // undefined is not a receipt and strict capture/use still requires loadRequired() to succeed.
      if (allowMissing && error?.code === "ENOENT") return undefined
      throw error
    }
    if (Object.values(state.reservations).some((record) => record.identity.containerName === lock.containerName)) {
      fail("managed slice has a durable disk quota reservation")
    }
    return state
  }

  async function assertBounded(lock, { identity, limits } = {}) {
    assertLock(lock)
    const state = await readValidatedState(lock)
    const record = Object.values(state.reservations).find((candidate) => candidate.identity.containerName === lock.containerName)
    if (!record) {
      fail("managed disk quota allocator response has no durable reservation")
    }
    if (identity && sliceDiskQuotaIdentityKey(record.identity) !== sliceDiskQuotaIdentityKey(identity)) {
      fail("managed disk quota reservation identity does not match the broker request")
    }
    if (limits && (
      record.limits.writableLayerBytes !== limits.writableLayerBytes ||
      record.limits.persistentHomeBytes !== limits.persistentHomeBytes
    )) {
      fail("managed disk quota reservation limits changed before broker provisioning")
    }
    return state
  }

  async function resolveUnboundedProof(lock, identity, binding) {
    validateSliceDiskQuotaIdentity(identity)
    assertLock(lock, identity.containerName)
    return withStateLock(lock, () => {
      const state = validateSliceDiskQuotaState(stateStore.loadRequired())
      if (Object.values(state.reservations).some((record) => record.identity.containerName === identity.containerName)) {
        return undefined
      }
      if (!hasMatchingSliceDiskQuotaUnboundedProof(resolvedProofRoot, identity, binding)) return undefined
      return { containerId: binding.containerId }
    })
  }

  async function captureUnboundedProof(lock, identity, binding) {
    validateSliceDiskQuotaIdentity(identity)
    assertLock(lock, identity.containerName)
    return withStateLock(lock, () => {
      const state = validateSliceDiskQuotaState(stateStore.loadRequired())
      if (Object.values(state.reservations).some((record) => record.identity.containerName === identity.containerName)) {
        fail("managed disk quota reservation prevents an unbounded proof")
      }
      writeSliceDiskQuotaUnboundedProof(resolvedProofRoot, identity, binding)
      return true
    })
  }

  function revokeUnboundedProof(lock, identity) {
    validateSliceDiskQuotaIdentity(identity)
    assertLock(lock, identity.containerName)
    removeSliceDiskQuotaUnboundedProof(resolvedProofRoot, identity)
  }

  async function runReservation(identity, operation, { signal } = {}) {
    validateSliceDiskQuotaIdentity(identity)
    if (typeof operation !== "function") throw new TypeError("managed disk quota reservation requires an operation")
    return withContainerLock(identity.containerName, (lock) => withStateLock(lock, async () => {
      assertLock(lock, identity.containerName)
      revokeUnboundedProof(lock, identity)
      return operation()
    }, { signal }), { signal })
  }

  return {
    withContainerLock,
    withStateLock,
    assertLockHeld: (lock) => assertLock(lock),
    readValidatedState,
    hasReservation,
    assertUnbounded,
    assertBounded,
    resolveUnboundedProof,
    captureUnboundedProof,
    revokeUnboundedProof,
    runReservation,
    paths: {
      state: resolvedStatePath,
      coordination: resolvedCoordinationRoot,
      proof: resolvedProofRoot,
    },
  }
}
