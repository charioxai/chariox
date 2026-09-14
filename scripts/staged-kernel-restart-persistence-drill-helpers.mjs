import { constants as fsConstants } from "node:fs"
import {
  access,
  lstat,
  readFile,
  readlink,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises"
import os from "node:os"
import path from "node:path"

const OWNERSHIP_SCHEMA_VERSION = 1
const OWNERSHIP_FILE_MODE = 0o600
const DRILL_ROOT_PREFIX = "chariox-staged-kernel-restart-"
const MAX_STOP_GRACE_MS = 30_000
const MAX_KILL_GRACE_MS = 10_000
export const STAGED_RESTART_PUBLIC_REQUEST_COUNT = 16

function requirePositiveInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 1) {
    throw new Error(`${label} must be a positive integer`)
  }
  return value
}

function requireAbsolutePath(value, label) {
  if (typeof value !== "string" || !path.isAbsolute(value)) {
    throw new Error(`${label} must be an absolute path`)
  }
  return value
}

function normalizedPath(value) {
  return path.resolve(value)
}

function processHasExited(child) {
  return child?.exitCode != null || child?.signalCode != null
}

function childPid(child) {
  return Number.isSafeInteger(child?.pid) && child.pid > 0 ? child.pid : null
}

function childStatus(child) {
  if (child?.exitCode != null) return `exit code ${child.exitCode}`
  if (child?.signalCode != null) return `signal ${child.signalCode}`
  return "running"
}

function ownershipBinary(ownership) {
  return normalizedPath(requireAbsolutePath(ownership?.binary, "owned child binary"))
}

function ownershipPid(ownership) {
  return requirePositiveInteger(ownership?.pid, "owned child pid")
}

export function createPublicRequestLedger(expectedCount = STAGED_RESTART_PUBLIC_REQUEST_COUNT) {
  if (!Number.isSafeInteger(expectedCount) || expectedCount < 1) {
    throw new Error("public request ledger expected count must be a positive integer")
  }
  let successfulCount = 0
  const labels = []
  return {
    record(label) {
      successfulCount += 1
      labels.push(String(label))
      return successfulCount
    },
    get count() {
      return successfulCount
    },
    assertComplete() {
      if (successfulCount !== expectedCount) {
        throw new Error(
          `restart drill must complete exactly ${expectedCount} public requests, got ${successfulCount}: ${labels.join(", ")}`,
        )
      }
      return successfulCount
    },
  }
}

export function assertSafeDrillRootPath(rootDir) {
  const resolved = normalizedPath(requireAbsolutePath(rootDir, "drill root"))
  const temporaryRoot = normalizedPath(os.tmpdir())
  const relative = path.relative(temporaryRoot, resolved)
  if (
    !relative
    || relative.startsWith(`..${path.sep}`)
    || relative === ".."
    || path.isAbsolute(relative)
    || path.basename(resolved).startsWith(DRILL_ROOT_PREFIX) === false
  ) {
    throw new Error(`refusing to operate on a non-owned drill root: ${resolved}`)
  }
  return resolved
}

export function assertOwnedStatePaths({ rootDir, charioxHome, workspace, tokenFile, ownershipFile }) {
  const root = assertSafeDrillRootPath(rootDir)
  const paths = { charioxHome, workspace, tokenFile, ownershipFile }
  for (const [label, value] of Object.entries(paths)) {
    const resolved = normalizedPath(requireAbsolutePath(value, label))
    const relative = path.relative(root, resolved)
    if (!relative || relative.startsWith(`..${path.sep}`) || relative === ".." || path.isAbsolute(relative)) {
      throw new Error(`${label} must remain inside the owned drill root`)
    }
  }
  return true
}

export async function assertExactCandidateBinary(binary) {
  const candidate = requireAbsolutePath(binary, "candidate binary")
  await access(candidate, fsConstants.X_OK)
  const metadata = await lstat(candidate)
  if (!metadata.isFile()) {
    throw new Error(`candidate binary is not a regular file: ${candidate}`)
  }
  const resolved = await realpath(candidate)
  const resolvedMetadata = await lstat(resolved)
  if (!resolvedMetadata.isFile()) {
    throw new Error(`candidate binary realpath is not a regular file: ${resolved}`)
  }
  return resolved
}

export async function readLinuxProcessIdentity(pid) {
  const processId = requirePositiveInteger(pid, "process id")
  const procRoot = `/proc/${processId}`
  let executable
  let commandLine
  let statLine
  try {
    ;[executable, commandLine, statLine] = await Promise.all([
      readlink(`${procRoot}/exe`),
      readFile(`${procRoot}/cmdline`, "utf8"),
      readFile(`${procRoot}/stat`, "utf8"),
    ])
  } catch (error) {
    throw new Error(`could not verify owned child PID ${processId}: ${error?.message ?? error}`)
  }

  const statClose = statLine.lastIndexOf(")")
  if (statClose < 0) {
    throw new Error(`could not parse /proc/${processId}/stat while verifying child ownership`)
  }
  const statFields = statLine.slice(statClose + 2).trim().split(/\s+/)
  const startTimeTicks = statFields[19]
  if (!/^\d+$/.test(startTimeTicks ?? "")) {
    throw new Error(`could not read the start identity for owned child PID ${processId}`)
  }

  const argv = commandLine
    .split("\u0000")
    .filter((argument) => argument.length > 0)
  return {
    pid: processId,
    executable: await realpath(executable),
    argv,
    process_start_time_ticks: startTimeTicks,
  }
}

export function assertProcessIdentity(actual, expected) {
  const expectedStartTimeTicks = expected?.process_start_time_ticks ?? expected?.startTimeTicks
  if (!actual || actual.pid !== expected?.pid) {
    throw new Error(`owned child PID changed: expected ${expected?.pid}, got ${actual?.pid ?? "<missing>"}`)
  }
  const actualExecutable = actual?.executable
  if (typeof actualExecutable !== "string" || normalizedPath(actualExecutable) !== ownershipBinary(expected)) {
    throw new Error(
      `owned child executable changed: expected ${ownershipBinary(expected)}, got ${actualExecutable ?? "<missing>"}`,
    )
  }
  const actualArgv0 = actual.argv?.[0]
  if (!actualArgv0 || normalizedPath(actualArgv0) !== ownershipBinary(expected)) {
    throw new Error(`owned child argv[0] changed: expected ${ownershipBinary(expected)}, got ${actualArgv0 ?? "<missing>"}`)
  }
  const actualStartTimeTicks = actual?.process_start_time_ticks ?? actual?.startTimeTicks
  if (expectedStartTimeTicks != null && String(actualStartTimeTicks) !== String(expectedStartTimeTicks)) {
    throw new Error(
      `owned child start identity changed for PID ${expected.pid}: expected ${expectedStartTimeTicks}, got ${actualStartTimeTicks ?? "<missing>"}`,
    )
  }
  return true
}

export function assertChildHandleOwnership(child, ownership) {
  const pid = childPid(child)
  const expectedPid = ownershipPid(ownership)
  if (pid !== expectedPid) {
    throw new Error(`owned child handle changed: expected PID ${expectedPid}, got ${pid ?? "<missing>"}`)
  }
  const spawnFile = child?.spawnfile
  if (spawnFile && normalizedPath(spawnFile) !== ownershipBinary(ownership)) {
    throw new Error(
      `owned child spawn binary changed: expected ${ownershipBinary(ownership)}, got ${spawnFile}`,
    )
  }
  const spawnArgs = child?.spawnargs
  if (Array.isArray(spawnArgs) && spawnArgs.length > 0 && normalizedPath(spawnArgs[0]) !== ownershipBinary(ownership)) {
    throw new Error(
      `owned child command changed: expected ${ownershipBinary(ownership)}, got ${spawnArgs[0]}`,
    )
  }
  return true
}

export async function claimExactChildProcess({ child, binary }) {
  const resolvedBinary = await assertExactCandidateBinary(binary)
  const pid = childPid(child)
  if (pid == null) throw new Error("candidate child did not provide a child PID")
  const provisional = { pid, binary: resolvedBinary }
  assertChildHandleOwnership(child, provisional)
  if (processHasExited(child)) {
    return { pid, binary: resolvedBinary, process_start_time_ticks: null }
  }
  const identity = await readLinuxProcessIdentity(pid)
  assertProcessIdentity(identity, { ...provisional, process_start_time_ticks: identity.process_start_time_ticks })
  return {
    pid,
    binary: resolvedBinary,
    process_start_time_ticks: identity.process_start_time_ticks,
  }
}

export async function claimChildKernel({
  child,
  binary,
  runId,
  generation,
  rootDir,
  charioxHome,
  workspace,
  tokenFile,
  endpoint,
  ownershipFile,
  phase = "running",
}) {
  const claimed = await claimExactChildProcess({ child, binary })
  const resolvedBinary = claimed.binary
  const pid = claimed.pid
  const ownership = {
    schema_version: OWNERSHIP_SCHEMA_VERSION,
    run_id: String(runId),
    generation: requirePositiveInteger(generation, "kernel generation"),
    phase: String(phase),
    pid,
    binary: resolvedBinary,
    process_start_time_ticks: claimed.process_start_time_ticks,
    root_dir: requireAbsolutePath(rootDir, "drill root"),
    chariox_home: requireAbsolutePath(charioxHome, "CHARIOX_HOME"),
    workspace: requireAbsolutePath(workspace, "kernel workspace"),
    token_file: requireAbsolutePath(tokenFile, "kernel auth token file"),
    endpoint: String(endpoint),
    ownership_file: requireAbsolutePath(ownershipFile, "ownership file"),
  }
  assertOwnedStatePaths({
    rootDir: ownership.root_dir,
    charioxHome: ownership.chariox_home,
    workspace: ownership.workspace,
    tokenFile: ownership.token_file,
    ownershipFile: ownership.ownership_file,
  })
  return ownership
}

export async function verifyOwnedChild(child, ownership, readProcessIdentity = readLinuxProcessIdentity) {
  assertChildHandleOwnership(child, ownership)
  if (processHasExited(child)) return { running: false, status: childStatus(child) }
  if (ownership?.process_start_time_ticks == null) {
    throw new Error(`owned child PID ${ownershipPid(ownership)} has no start identity`)
  }
  const actual = await readProcessIdentity(ownershipPid(ownership))
  assertProcessIdentity(actual, ownership)
  return { running: true, status: "running", identity: actual }
}

export async function writeOwnershipState(ownershipFile, ownership) {
  requireAbsolutePath(ownershipFile, "ownership file")
  try {
    const existing = await lstat(ownershipFile)
    if (!existing.isFile() || existing.isSymbolicLink() || existing.nlink !== 1) {
      throw new Error(`ownership state path is not a private regular file: ${ownershipFile}`)
    }
  } catch (error) {
    if (error?.code !== "ENOENT") throw error
  }
  const payload = `${JSON.stringify(ownership, null, 2)}\n`
  await writeFile(ownershipFile, payload, { mode: OWNERSHIP_FILE_MODE })
  const metadata = await lstat(ownershipFile)
  if (!metadata.isFile() || (metadata.mode & 0o077) !== 0 || metadata.nlink !== 1) {
    throw new Error(`ownership state is not a private regular file: ${ownershipFile}`)
  }
  return ownership
}

export function withOwnershipPhase(ownership, phase) {
  return { ...ownership, phase: String(phase) }
}

function waitForChildExit(child, timeoutMs) {
  requirePositiveInteger(timeoutMs, "child exit timeout")
  if (processHasExited(child)) {
    return Promise.resolve({ exitCode: child.exitCode ?? null, signalCode: child.signalCode ?? null })
  }
  return new Promise((resolve, reject) => {
    let timer
    let settled = false
    const finish = (error, result) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      if (error) reject(error)
      else resolve(result)
    }
    timer = setTimeout(() => {
      finish(new Error(`owned child did not exit after ${timeoutMs}ms`))
    }, timeoutMs)
    child.once("exit", (code, signal) => finish(null, { exitCode: code, signalCode: signal }))
    child.once("error", (error) => {
      if (processHasExited(child)) {
        finish(null, { exitCode: child.exitCode ?? null, signalCode: child.signalCode ?? null })
      } else {
        finish(new Error(`owned child emitted an error before exit: ${error?.message ?? error}`))
      }
    })
  })
}

export async function stopOwnedChild(child, ownership, {
  graceMs = 5_000,
  killGraceMs = 2_000,
  verify = verifyOwnedChild,
  readProcessIdentity = readLinuxProcessIdentity,
} = {}) {
  requirePositiveInteger(graceMs, "child stop grace")
  requirePositiveInteger(killGraceMs, "child kill grace")
  if (graceMs > MAX_STOP_GRACE_MS) throw new Error(`child stop grace must be at most ${MAX_STOP_GRACE_MS}ms`)
  if (killGraceMs > MAX_KILL_GRACE_MS) throw new Error(`child kill grace must be at most ${MAX_KILL_GRACE_MS}ms`)
  assertChildHandleOwnership(child, ownership)
  if (processHasExited(child)) {
    return { pid: ownershipPid(ownership), signal: null, forced: false, status: childStatus(child) }
  }

  await verify(child, ownership, readProcessIdentity)
  if (processHasExited(child)) {
    return { pid: ownershipPid(ownership), signal: null, forced: false, status: childStatus(child) }
  }

  let termSent = false
  try {
    termSent = child.kill("SIGTERM")
  } catch (error) {
    throw new Error(`failed to send SIGTERM to owned child PID ${ownershipPid(ownership)}: ${error?.message ?? error}`)
  }
  let termError = null
  try {
    await waitForChildExit(child, graceMs)
  } catch (error) {
    termError = error
  }
  if (processHasExited(child)) {
    return { pid: ownershipPid(ownership), signal: "SIGTERM", forced: false, sent: termSent, status: childStatus(child) }
  }

  // A PID can be reused while a graceful stop is pending. Re-check the
  // executable and kernel start identity immediately before SIGKILL.
  await verify(child, ownership, readProcessIdentity)
  if (processHasExited(child)) {
    return { pid: ownershipPid(ownership), signal: "SIGTERM", forced: false, sent: termSent, status: childStatus(child) }
  }
  let killSent = false
  try {
    killSent = child.kill("SIGKILL")
  } catch (error) {
    throw new Error(`failed to send SIGKILL to owned child PID ${ownershipPid(ownership)}: ${error?.message ?? error}`)
  }
  try {
    await waitForChildExit(child, killGraceMs)
  } catch (error) {
    throw new Error(
      `owned child cleanup failed for PID ${ownershipPid(ownership)} after SIGTERM (${termError?.message ?? "timed out"}) and SIGKILL (${error?.message ?? error})`,
    )
  }
  if (!processHasExited(child)) {
    throw new Error(`owned child cleanup failed: PID ${ownershipPid(ownership)} is still running after SIGKILL`)
  }
  return { pid: ownershipPid(ownership), signal: "SIGKILL", forced: true, sent: killSent, status: childStatus(child) }
}

export async function removeOwnedDrillRoot(rootDir) {
  const resolved = assertSafeDrillRootPath(rootDir)
  await rm(resolved, { recursive: true, force: false })
  try {
    await lstat(resolved)
  } catch (error) {
    if (error?.code === "ENOENT") return true
    throw new Error(`could not verify drill root cleanup at ${resolved}: ${error?.message ?? error}`)
  }
  throw new Error(`drill root cleanup failed; owned root still exists: ${resolved}`)
}

export function durableAgentFingerprint(agent) {
  if (!agent || typeof agent !== "object") throw new Error("session did not contain a durable agent")
  return {
    id: agent.id,
    agent_ref: agent.agent_ref,
    session_id: agent.session_id,
    role: agent.role ?? "standard",
    alias: agent.alias ?? null,
    provider: agent.provider,
    model: agent.model ?? null,
    effort: agent.effort ?? null,
    account_profile: agent.account_profile ?? null,
    primary_provider: agent.primary_provider ?? null,
    primary_model: agent.primary_model ?? null,
    primary_effort: agent.primary_effort ?? null,
    execution_mode_override: agent.execution_mode_override ?? null,
    permission_level_override: agent.permission_level_override ?? null,
    workspace_id: agent.workspace_id ?? null,
    worktree_id: agent.worktree_id ?? null,
    visible_in_freeform: agent.visible_in_freeform ?? true,
    state: agent.state,
    is_processing: agent.is_processing,
    grid_row: agent.grid_row,
    grid_col: agent.grid_col,
    grid_row_span: agent.grid_row_span,
    grid_col_span: agent.grid_col_span,
    created_at_ms: agent.created_at_ms,
  }
}

export function durableSessionFingerprint(session) {
  if (!session || typeof session !== "object") throw new Error("public response did not contain a durable session")
  const agents = Array.isArray(session.agents) ? session.agents : []
  return {
    id: session.id,
    project_id: session.project_id,
    alias: session.alias ?? null,
    workspace_id: session.workspace_id,
    worktree_id: session.worktree_id,
    host_machine_id: session.host_machine_id ?? null,
    host_daemon_id: session.host_daemon_id ?? null,
    created_at_ms: session.created_at_ms,
    status: session.status,
    agent_defaults: session.agent_defaults ?? null,
    focused_agent_id: session.focused_agent_id ?? null,
    max_agents: session.max_agents,
    config_state: session.config_state
      ? {
          version: session.config_state.version,
          values: session.config_state.values ?? {},
        }
      : null,
    workspace_live_sync_mode: session.workspace_live_sync_mode ?? null,
    agents: agents.map(durableAgentFingerprint).sort((left, right) => String(left.id).localeCompare(String(right.id))),
  }
}

export function assertRestoredSessionIdentity({
  listedSessions,
  resolvedSession,
  stateSession,
  expectedSession,
  expectedAgent,
}) {
  const listed = Array.isArray(listedSessions) ? listedSessions : []
  const expectedSessionId = expectedSession?.id
  if (!expectedSessionId) throw new Error("restart acceptance is missing the created session identity")
  const listedMatch = listed.find((session) => session?.id === expectedSessionId)
  if (!listedMatch) {
    throw new Error(`created session ${expectedSessionId} was lost after process restart`)
  }
  if (listed.length !== 1) {
    throw new Error(`session list was duplicated or contaminated after process restart: expected 1, got ${listed.length}`)
  }
  if (JSON.stringify(durableSessionFingerprint(listedMatch)) !== JSON.stringify(expectedSession)) {
    throw new Error(`created session ${expectedSessionId} was replaced after process restart`)
  }
  if (!resolvedSession || resolvedSession.id !== expectedSessionId) {
    throw new Error(`created session ${expectedSessionId} could not be resolved after process restart`)
  }
  if (JSON.stringify(durableSessionFingerprint(resolvedSession)) !== JSON.stringify(expectedSession)) {
    throw new Error(`resolved session ${expectedSessionId} was replaced after process restart`)
  }
  if (!stateSession || stateSession.id !== expectedSessionId) {
    throw new Error(`created session ${expectedSessionId} was lost from session state after process restart`)
  }
  if (JSON.stringify(durableSessionFingerprint(stateSession)) !== JSON.stringify(expectedSession)) {
    throw new Error(`session state for ${expectedSessionId} was replaced after process restart`)
  }
  const expectedAgentId = expectedAgent?.id
  const restoredAgents = Array.isArray(stateSession.agents) ? stateSession.agents : []
  const matchingAgents = restoredAgents.filter((agent) => agent?.id === expectedAgentId)
  if (matchingAgents.length !== 1 || restoredAgents.length !== 1) {
    throw new Error(
      `created agent ${expectedAgentId ?? "<missing>"} was lost or duplicated after process restart`,
    )
  }
  if (JSON.stringify(durableAgentFingerprint(matchingAgents[0])) !== JSON.stringify(expectedAgent)) {
    throw new Error(`created agent ${expectedAgentId} was replaced after process restart`)
  }
  return matchingAgents[0]
}

export function assertNoDuplicateSessionAgents(session, expectedAgentId) {
  const agents = Array.isArray(session?.agents) ? session.agents : []
  const ids = agents.map((agent) => agent?.id)
  if (new Set(ids).size !== ids.length) throw new Error("session contains duplicate agent identities after reattachment")
  if (ids.length !== 1 || ids[0] !== expectedAgentId) {
    throw new Error(`session agent identity changed after reattachment: expected only ${expectedAgentId}`)
  }
  return true
}
