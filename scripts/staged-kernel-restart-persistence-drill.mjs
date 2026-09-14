#!/usr/bin/env node
import assert from "node:assert/strict"
import { randomBytes } from "node:crypto"
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  writeFile,
} from "node:fs/promises"
import { spawn } from "node:child_process"
import { once } from "node:events"
import os from "node:os"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"
import { pathToFileURL } from "node:url"

import {
  assertCandidateProtocolOutput,
  assertNoRelayOrProviderEnvironment,
  candidateKernelEndpoint,
  makeCandidateKernelEnvironment,
  parseCandidateKernelSmokeArgs,
} from "./candidate-kernel-startup-smoke-drill-helpers.mjs"
import {
  assertNoDuplicateSessionAgents,
  assertRestoredSessionIdentity,
  assertOwnedStatePaths,
  assertExactCandidateBinary,
  claimChildKernel,
  claimExactChildProcess,
  createPublicRequestLedger,
  durableAgentFingerprint,
  durableSessionFingerprint,
  removeOwnedDrillRoot,
  stopOwnedChild,
  withOwnershipPhase,
  writeOwnershipState,
} from "./staged-kernel-restart-persistence-drill-helpers.mjs"
import { makeAvailablePorts, portsAreAvailable } from "../apps/cli/scripts/lib/drill-runtime-helpers.mjs"

const REQUEST_TIMEOUT_MS = 5_000
const PROTOCOL_PROBE_TIMEOUT_MS = 5_000
const STOP_GRACE_MS = 5_000
const KILL_GRACE_MS = 2_000

function log(name, details = undefined) {
  if (details === undefined) {
    console.log(`[staged-kernel-restart] ${name}`)
  } else {
    console.log(`[staged-kernel-restart] ${name}`, JSON.stringify(details))
  }
}

function unwrap(response, key) {
  const value = response?.[key]
  if (value === undefined) throw new Error(`kernel response did not contain ${key}`)
  return value
}

async function withTimeout(promise, label, timeoutMs) {
  let timer
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs}ms`)), timeoutMs)
        timer.unref?.()
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

function remainingMs(deadline, label, capMs) {
  const remaining = deadline - Date.now()
  if (remaining <= 0) throw new Error(`${label} exceeded the bounded drill timeout`)
  return Math.max(1, Math.min(remaining, capMs))
}

function childStatus(child) {
  if (child?.spawnError) return `spawn error ${child.spawnError.message}`
  if (child?.exitCode != null) return `exit code ${child.exitCode}`
  if (child?.signalCode != null) return `signal ${child.signalCode}`
  return "running"
}

export function processHasExited(child) {
  return child?.exitCode != null || child?.signalCode != null
}

function startKernel(binary, env, cwd) {
  const child = spawn(binary, [], {
    cwd,
    env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  child.logs = { stdout: "", stderr: "" }
  child.spawnError = null
  child.stdout.setEncoding("utf8")
  child.stderr.setEncoding("utf8")
  child.stdout.on("data", (chunk) => { child.logs.stdout = `${child.logs.stdout}${chunk}`.slice(-8192) })
  child.stderr.on("data", (chunk) => { child.logs.stderr = `${child.logs.stderr}${chunk}`.slice(-8192) })
  child.once("error", (error) => { child.spawnError = error })
  return child
}

export async function runVersionProbe(binary, timeoutMs) {
  const child = spawn(binary, ["--print-local-daemon-protocol-version"], {
    env: {
      PATH: process.env.PATH || "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
      LANG: "C.UTF-8",
      LC_ALL: "C.UTF-8",
    },
    stdio: ["ignore", "pipe", "pipe"],
  })
  let stdout = ""
  let stderr = ""
  child.stdout.setEncoding("utf8")
  child.stderr.setEncoding("utf8")
  child.stdout.on("data", (chunk) => { stdout = `${stdout}${chunk}`.slice(-4096) })
  child.stderr.on("data", (chunk) => { stderr = `${stderr}${chunk}`.slice(-4096) })
  child.once("error", (error) => { child.spawnError = error })

  let ownership
  const close = once(child, "close")
  try {
    ownership = await claimExactChildProcess({ child, binary })
    const result = await withTimeout(close, "candidate protocol probe", timeoutMs)
    if (child.spawnError) throw new Error(`candidate protocol probe could not start: ${child.spawnError.message}`)
    const [code, signal] = result
    if (code !== 0) {
      throw new Error(`candidate protocol probe exited with ${signal ?? `code ${code}`}: ${stderr.trim()}`)
    }
    return { stdout, stderr, code, signal }
  } catch (error) {
    if (ownership && !processHasExited(child)) {
      await stopOwnedChild(child, ownership, {
        graceMs: Math.min(timeoutMs, STOP_GRACE_MS),
        killGraceMs: Math.min(timeoutMs, KILL_GRACE_MS),
      })
    }
    throw error
  }
}

async function request(client, value, label, deadline) {
  return await withTimeout(
    client.send(value),
    label,
    remainingMs(deadline, label, REQUEST_TIMEOUT_MS),
  )
}

export async function waitForKernel({ LocalIpcClient, listSessionsRequest, endpoint, token, child, deadline }) {
  let lastError = null
  while (Date.now() < deadline) {
    if (child.spawnError || processHasExited(child)) {
      throw new Error(`candidate kernel exited during startup (${childStatus(child)})\n${child.logs.stderr}`)
    }
    const probe = new LocalIpcClient(endpoint, {
      localAuthToken: token,
      controlRequestRetryDeadlineMs: 0,
      controlResponseStallMs: 500,
    })
    try {
      const response = await withTimeout(
        probe.send(listSessionsRequest()),
        "kernel startup probe",
        Math.min(1_000, remainingMs(deadline, "kernel startup", 1_000)),
      )
      unwrap(response, "SessionsListed")
      await probe.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await probe.close().catch(() => {})
      await sleep(Math.min(100, Math.max(1, deadline - Date.now())))
    }
  }
  throw new Error(`candidate kernel did not become ready at ${endpoint}: ${lastError?.message ?? lastError}`)
}

export async function expectAuthenticationFailure({ LocalIpcClient, listSessionsRequest, endpoint, token, deadline }) {
  const client = new LocalIpcClient(endpoint, {
    localAuthToken: `${token}-wrong`,
    controlRequestRetryDeadlineMs: 0,
    controlResponseStallMs: 500,
  })
  try {
    await assert.rejects(
      () => withTimeout(
        client.send(listSessionsRequest()),
        "wrong-credential probe",
        Math.min(2_000, remainingMs(deadline, "wrong-credential probe", 2_000)),
      ),
      (error) => error?.code === "authentication_failed",
    )
  } finally {
    await client.close().catch(() => {})
  }
}

function assertNoPromptReplay(session) {
  assert.equal(session.active_provider_run_id ?? null, null, "restart smoke session must not have a provider run")
  assert.equal(session.active_prompt ?? null, null, "restart smoke session must not have an active prompt")
  const promptStates = session.prompt_states && typeof session.prompt_states === "object"
    ? Object.values(session.prompt_states)
    : []
  assert.equal(promptStates.some((state) => state?.active_prompt != null), false, "restart smoke session must not replay a prompt")
}

async function ensurePortsAreAvailable(ports, label) {
  if (!(await portsAreAvailable(ports))) {
    throw new Error(`${label} drill ports are already occupied; refusing to signal any existing process`)
  }
}

async function ensureAuthTokenFile(tokenFile, token) {
  try {
    const existing = await readFile(tokenFile, "utf8")
    if (existing.trim() !== token) {
      throw new Error(`kernel auth token file has unexpected contents: ${tokenFile}`)
    }
  } catch (error) {
    if (error?.code !== "ENOENT") throw error
    await writeFile(tokenFile, `${token}\n`, { flag: "wx", mode: 0o600 })
  }
  const metadata = await lstat(tokenFile)
  if (
    !metadata.isFile()
    || metadata.isSymbolicLink()
    || (metadata.mode & 0o077) !== 0
    || metadata.nlink !== 1
    || (process.getuid && metadata.uid !== process.getuid())
  ) {
    throw new Error(`kernel auth token file is not a private drill-owned file: ${tokenFile}`)
  }
}

export async function startOwnedKernel({ binary, env, authToken, ports, workspace, rootDir, tokenFile, ownershipFile, endpoint, runId, generation }) {
  await ensurePortsAreAvailable(ports, "kernel")
  await ensureAuthTokenFile(tokenFile, authToken)
  const child = startKernel(binary, env, workspace)
  let ownership = null
  try {
    ownership = await claimChildKernel({
      child,
      binary,
      runId,
      generation,
      rootDir,
      charioxHome: env.CHARIOX_HOME,
      workspace,
      tokenFile,
      endpoint,
      ownershipFile,
      phase: "starting",
    })
    await writeOwnershipState(ownershipFile, ownership)
    return { child, ownership }
  } catch (error) {
    const failure = error instanceof Error ? error : new Error(String(error))
    failure.child = child
    failure.ownership = ownership
    if (!processHasExited(child)) {
      failure.message = `${failure.message}; child PID ${child.pid ?? "<missing>"} ownership was not established, so no signal was sent`
    }
    throw failure
  }
}

export async function stopAndReleaseKernel({ child, ownership, ownershipFile, ports, deadline, label }) {
  if (!child) return null
  if (!ownership) {
    throw new Error(`${label} child ownership was not established; refusing to signal an unverified process`)
  }
  await writeOwnershipState(ownershipFile, withOwnershipPhase(ownership, "stopping"))
  const result = await stopOwnedChild(child, ownership, {
    graceMs: Math.min(STOP_GRACE_MS, Math.max(1, deadline - Date.now())),
    killGraceMs: Math.min(KILL_GRACE_MS, Math.max(1, deadline - Date.now())),
  })
  if (!processHasExited(child)) {
    throw new Error(`${label} cleanup returned before child PID ${ownership.pid} exited`)
  }
  await ensurePortsAreAvailable(ports, `${label} after stop`)
  await writeOwnershipState(ownershipFile, withOwnershipPhase(ownership, "stopped"))
  return result
}

function assertKernelHealth(health, child, expectedProtocol) {
  assert.equal(health.projection.process.process_id, child.pid, "health must identify the owned candidate process")
  assert.equal(health.projection.transport.relay_last_connected_url, null, "restart drill must not connect to a relay")
  assert.equal(health.projection.transport.relay_reconnect_attempts, 0, "restart drill must not retry a relay")
  assert.equal(typeof expectedProtocol, "number")
}

async function main() {
  const options = parseCandidateKernelSmokeArgs(process.argv.slice(2))
  if (options.help) {
    console.log(
      "Usage: node scripts/staged-kernel-restart-persistence-drill.mjs --binary PATH --expected-protocol N [--timeout-ms MS]",
    )
    return
  }

  const binary = await assertExactCandidateBinary(options.binary)
  const runId = `${process.pid}-${Date.now()}`
  const deadline = Date.now() + options.timeoutMs
  const ports = await makeAvailablePorts()
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-staged-kernel-restart-"))
  const workspace = path.join(rootDir, "workspace")
  const tokenFile = path.join(rootDir, "kernel-local-auth-token")
  const ownershipFile = path.join(rootDir, "child-ownership.json")
  const token = `staged-restart-${randomBytes(24).toString("hex")}`
  const endpoint = candidateKernelEndpoint(ports.kernelPort)

  let env = null
  let first = null
  let second = null
  let client = null
  let detachRequestBuilder = null
  const publicRequestLedger = createPublicRequestLedger()
  let sessionId = null
  let agentId = null
  let expectedSession = null
  let expectedAgent = null
  let failure = null
  let passedDetails = null
  const cleanupErrors = []

  try {
    env = makeCandidateKernelEnvironment({
      rootDir,
      ports,
      tokenFile,
      runId,
      baseEnv: process.env,
    })
    assertNoRelayOrProviderEnvironment(env)
    assertOwnedStatePaths({
      rootDir,
      charioxHome: env.CHARIOX_HOME,
      workspace,
      tokenFile,
      ownershipFile,
    })
    await mkdir(workspace, { recursive: true })

    const [{ LocalIpcClient }, requests] = await Promise.all([
      import("../packages/kernel-client/dist/ipc.js"),
      import("../packages/kernel-client/dist/ipc-requests.js"),
    ])
    const {
      attachToSessionRequest,
      createSessionRequest,
      detachFromSessionRequest: buildDetachRequest,
      getDaemonHealthRequest,
      getSessionStateRequest,
      listSessionsRequest,
      resolveSessionRequest,
    } = requests
    detachRequestBuilder = buildDetachRequest

    const version = await runVersionProbe(binary, remainingMs(deadline, "protocol preflight", PROTOCOL_PROBE_TIMEOUT_MS))
    assertCandidateProtocolOutput(version.stdout, options.expectedProtocol)
    log("protocol-preflight-passed", { binary, expectedProtocol: options.expectedProtocol })

    try {
      first = await startOwnedKernel({
        binary,
        env,
        authToken: token,
        ports,
        workspace,
        rootDir,
        tokenFile,
        ownershipFile,
        endpoint,
        runId,
        generation: 1,
      })
    } catch (error) {
      if (error?.child) first = { child: error.child, ownership: error.ownership }
      throw error
    }
    await waitForKernel({
      LocalIpcClient,
      listSessionsRequest,
      endpoint,
      token,
      child: first.child,
      deadline,
    })
    await writeOwnershipState(ownershipFile, withOwnershipPhase(first.ownership, "ready"))
    await expectAuthenticationFailure({ LocalIpcClient, listSessionsRequest, endpoint, token, deadline })
    log("wrong-credentials-rejected")

    client = new LocalIpcClient(endpoint, {
      localAuthToken: token,
      controlRequestRetryDeadlineMs: 0,
      controlResponseStallMs: 500,
    })
    const health = unwrap(await request(client, getDaemonHealthRequest(), "first daemon health", deadline), "DaemonHealth")
    publicRequestLedger.record("first daemon health")
    assertKernelHealth(health, first.child, options.expectedProtocol)

    const listedBefore = unwrap(await request(client, listSessionsRequest(), "initial session list", deadline), "SessionsListed")
    publicRequestLedger.record("initial session list")
    assert.equal(listedBefore.sessions.length, 0, "fresh staged state must have no sessions")

    const created = unwrap(
      await request(
        client,
        createSessionRequest(workspace, workspace, "staged-kernel-restart"),
        "session create",
        deadline,
      ),
      "SessionCreated",
    )
    publicRequestLedger.record("session create")
    sessionId = created.session.id
    agentId = created.agent.id
    assert.ok(sessionId, "staged kernel must create a session")
    assert.ok(agentId, "staged kernel must create a default agent")
    assert.equal(created.agent.session_id, sessionId, "created agent must belong to created session")
    assertNoPromptReplay(created.session)

    const attached = unwrap(
      await request(client, attachToSessionRequest(sessionId, `staged-restart-${runId}-first`), "first session attach", deadline),
      "SessionAttached",
    )
    publicRequestLedger.record("first session attach")
    const firstAttachmentId = attached.attachment.id
    assert.ok(firstAttachmentId, "staged kernel must attach the public client")

    const firstState = unwrap(await request(client, getSessionStateRequest(sessionId), "first session state", deadline), "SessionState")
    publicRequestLedger.record("first session state")
    assert.equal(firstState.session.id, sessionId)
    assertNoPromptReplay(firstState.session)
    assertNoDuplicateSessionAgents(firstState.session, agentId)
    const firstAgent = firstState.session.agents.find((agent) => agent.id === agentId)
    assert.ok(firstAgent, "created agent must be in the public session state")
    assert.equal(firstAgent.agent_ref, created.agent.agent_ref, "created agent reference must remain exact")
    expectedSession = durableSessionFingerprint(firstState.session)
    expectedAgent = durableAgentFingerprint(firstAgent)

    await request(client, detachRequestBuilder(firstAttachmentId), "first session detach", deadline)
    publicRequestLedger.record("first session detach")
    const detachedState = unwrap(await request(client, getSessionStateRequest(sessionId), "detached session state", deadline), "SessionState")
    publicRequestLedger.record("detached session state")
    assert.equal(detachedState.session.attachment_ids.length, 0, "first child must have no attachment before stop")
    assertNoPromptReplay(detachedState.session)
    assert.equal(JSON.stringify(durableSessionFingerprint(detachedState.session)), JSON.stringify(expectedSession))
    await client.close()
    client = null

    const firstOwnership = first.ownership
    const firstStop = await stopAndReleaseKernel({
      child: first.child,
      ownership: first.ownership,
      ownershipFile,
      ports,
      deadline,
      label: "first candidate kernel",
    })
    log("child-stopped", { generation: 1, pid: first.ownership.pid, signal: firstStop.signal, forced: firstStop.forced })
    first = null

    // The auth file is one-shot by design. Re-create the same private file
    // with the same token, while retaining the same external CHARIOX_HOME.
    try {
      second = await startOwnedKernel({
        binary,
        env,
        authToken: token,
        ports,
        workspace,
        rootDir,
        tokenFile,
        ownershipFile,
        endpoint,
        runId,
        generation: 2,
      })
    } catch (error) {
      if (error?.child) second = { child: error.child, ownership: error.ownership }
      throw error
    }
    assert.ok(
      second.ownership.pid !== firstOwnership.pid
        || second.ownership.process_start_time_ticks !== firstOwnership.process_start_time_ticks,
      "restart must create a new child process identity",
    )
    await waitForKernel({
      LocalIpcClient,
      listSessionsRequest,
      endpoint,
      token,
      child: second.child,
      deadline,
    })
    await writeOwnershipState(ownershipFile, withOwnershipPhase(second.ownership, "ready"))
    await expectAuthenticationFailure({ LocalIpcClient, listSessionsRequest, endpoint, token, deadline })
    log("restarted-child-authenticated")

    client = new LocalIpcClient(endpoint, {
      localAuthToken: token,
      controlRequestRetryDeadlineMs: 0,
      controlResponseStallMs: 500,
    })
    const restartedHealth = unwrap(
      await request(client, getDaemonHealthRequest(), "restarted daemon health", deadline),
      "DaemonHealth",
    )
    publicRequestLedger.record("restarted daemon health")
    assertKernelHealth(restartedHealth, second.child, options.expectedProtocol)

    const listedAfterRestart = unwrap(
      await request(client, listSessionsRequest(), "post-restart session list", deadline),
      "SessionsListed",
    )
    publicRequestLedger.record("post-restart session list")
    const resolvedAfterRestart = unwrap(
      await request(client, resolveSessionRequest(sessionId), "post-restart session resolve", deadline),
      "SessionResolved",
    )
    publicRequestLedger.record("post-restart session resolve")
    const stateAfterRestart = unwrap(
      await request(client, getSessionStateRequest(sessionId), "post-restart session state", deadline),
      "SessionState",
    )
    publicRequestLedger.record("post-restart session state")
    assertNoPromptReplay(stateAfterRestart.session)
    assertRestoredSessionIdentity({
      listedSessions: listedAfterRestart.sessions,
      resolvedSession: resolvedAfterRestart.session,
      stateSession: stateAfterRestart.session,
      expectedSession,
      expectedAgent,
    })

    const reattached = unwrap(
      await request(
        client,
        attachToSessionRequest(sessionId, `staged-restart-${runId}-after-process-restart`),
        "post-restart session attach",
        deadline,
      ),
      "SessionAttached",
    )
    publicRequestLedger.record("post-restart session attach")
    const reattachedId = reattached.attachment.id
    assert.ok(reattachedId, "restarted kernel must attach the public client")
    assert.equal(reattached.attachment.session_id, sessionId)
    const reattachedState = unwrap(
      await request(client, getSessionStateRequest(sessionId), "post-restart reattached state", deadline),
      "SessionState",
    )
    publicRequestLedger.record("post-restart reattached state")
    assert.equal(reattachedState.session.attachment_ids.filter((id) => id === reattachedId).length, 1)
    assertNoDuplicateSessionAgents(reattachedState.session, agentId)
    const listedAfterReattach = unwrap(
      await request(client, listSessionsRequest(), "post-restart reattached session list", deadline),
      "SessionsListed",
    )
    publicRequestLedger.record("post-restart reattached session list")
    assertRestoredSessionIdentity({
      listedSessions: listedAfterReattach.sessions,
      resolvedSession: reattachedState.session,
      stateSession: reattachedState.session,
      expectedSession,
      expectedAgent,
    })
    await request(client, detachRequestBuilder(reattachedId), "post-restart session detach", deadline)
    publicRequestLedger.record("post-restart session detach")
    const finalState = unwrap(await request(client, getSessionStateRequest(sessionId), "final session state", deadline), "SessionState")
    publicRequestLedger.record("final session state")
    assert.equal(finalState.session.attachment_ids.length, 0, "restarted kernel must remove the reattached client")
    assertNoPromptReplay(finalState.session)
    assertNoDuplicateSessionAgents(finalState.session, agentId)
    assert.equal(JSON.stringify(durableSessionFingerprint(finalState.session)), JSON.stringify(expectedSession))
    publicRequestLedger.assertComplete()
    passedDetails = {
      endpoint,
      protocol: options.expectedProtocol,
      firstPid: firstStop.pid,
      restartedPid: second.ownership.pid,
      sessionId,
      agentId,
      successfulRequests: publicRequestLedger.count,
    }
  } catch (error) {
    failure = error
  } finally {
    await client?.close().catch((error) => cleanupErrors.push(new Error(`client cleanup failed: ${error.message}`)))
    if (second?.child) {
      try {
        await stopAndReleaseKernel({
          child: second.child,
          ownership: second.ownership,
          ownershipFile,
          ports,
          deadline: Date.now() + Math.max(STOP_GRACE_MS + KILL_GRACE_MS, 1),
          label: "restarted candidate kernel",
        })
      } catch (error) {
        cleanupErrors.push(error)
      }
    }
    if (first?.child && !processHasExited(first.child)) {
      try {
        await stopAndReleaseKernel({
          child: first.child,
          ownership: first.ownership,
          ownershipFile,
          ports,
          deadline: Date.now() + Math.max(STOP_GRACE_MS + KILL_GRACE_MS, 1),
          label: "first candidate kernel recovery cleanup",
        })
      } catch (error) {
        cleanupErrors.push(error)
      }
    }
    const runningChild = [first?.child, second?.child].find((child) => child && !processHasExited(child))
    if (runningChild) {
      cleanupErrors.push(new Error(`cleanup left candidate child PID ${runningChild.pid} running; owned root was retained`))
    } else {
      try {
        await removeOwnedDrillRoot(rootDir)
      } catch (error) {
        cleanupErrors.push(error)
      }
    }
  }

  if (cleanupErrors.length > 0) {
    const detail = cleanupErrors.map((error) => error.stack ?? error.message).join("\n")
    const cleanupFailure = new Error(`staged kernel restart drill cleanup failed:\n${detail}`)
    if (failure) {
      failure = new AggregateError([failure, cleanupFailure], "staged kernel restart drill failed with cleanup errors")
    } else {
      failure = cleanupFailure
    }
  }
  if (failure) throw failure
  if (passedDetails) log("passed", passedDetails)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(`[staged-kernel-restart] failed: ${error.stack ?? error.message}`)
    process.exitCode = 1
  })
}
