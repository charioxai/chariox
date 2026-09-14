#!/usr/bin/env node
import assert from "node:assert/strict"
import { randomBytes } from "node:crypto"
import { constants as fsConstants } from "node:fs"
import { access, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import { spawn } from "node:child_process"
import { once } from "node:events"
import os from "node:os"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"

import {
  assertCandidateProtocolOutput,
  assertNoRelayOrProviderEnvironment,
  candidateKernelEndpoint,
  candidateKernelSmokeUsage,
  makeCandidateKernelEnvironment,
  parseCandidateKernelSmokeArgs,
} from "./candidate-kernel-startup-smoke-drill-helpers.mjs"
import { makeAvailablePorts, terminateChild } from "../apps/cli/scripts/lib/drill-runtime-helpers.mjs"

const REQUEST_TIMEOUT_MS = 5_000

function log(name, details = undefined) {
  if (details === undefined) {
    console.log(`[candidate-kernel-smoke] ${name}`)
  } else {
    console.log(`[candidate-kernel-smoke] ${name}`, JSON.stringify(details))
  }
}

function unwrap(response, key) {
  const value = response?.[key]
  if (value === undefined) throw new Error(`kernel response did not contain ${key}`)
  return value
}

async function withTimeout(promise, label, timeoutMs = REQUEST_TIMEOUT_MS) {
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

async function runVersionProbe(binary, timeoutMs, env) {
  const child = spawn(binary, ["--print-local-daemon-protocol-version"], {
    env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  let stdout = ""
  let stderr = ""
  let spawnError = null
  child.stdout.setEncoding("utf8")
  child.stderr.setEncoding("utf8")
  child.stdout.on("data", (chunk) => { stdout = `${stdout}${chunk}`.slice(-4096) })
  child.stderr.on("data", (chunk) => { stderr = `${stderr}${chunk}`.slice(-4096) })
  child.once("error", (error) => { spawnError = error })
  let timedOut = false
  const timer = setTimeout(() => {
    timedOut = true
    child.kill("SIGKILL")
  }, timeoutMs)
  timer.unref?.()
  const [code, signal] = await once(child, "close")
  clearTimeout(timer)
  if (spawnError) throw new Error(`candidate protocol probe could not start: ${spawnError.message}`)
  if (timedOut) throw new Error(`candidate protocol probe timed out after ${timeoutMs}ms`)
  if (code !== 0) {
    throw new Error(`candidate protocol probe exited with ${signal ?? `code ${code}`}: ${stderr.trim()}`)
  }
  return { stdout, stderr, code, signal }
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

function childStatus(child) {
  if (child?.exitCode != null) return `exit code ${child.exitCode}`
  if (child?.signalCode != null) return `signal ${child.signalCode}`
  return "running"
}

async function request(client, value, label) {
  return await withTimeout(client.send(value), label)
}

async function waitForKernel({ LocalIpcClient, listSessionsRequest, endpoint, token, child, timeoutMs }) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    if (child.spawnError || child.exitCode != null || child.signalCode != null) {
      throw new Error(`candidate kernel exited during startup (${childStatus(child)})\n${child.logs.stderr}`)
    }
    const probe = new LocalIpcClient(endpoint, {
      localAuthToken: token,
      controlRequestRetryDeadlineMs: 0,
      controlResponseStallMs: 500,
    })
    try {
      const response = await withTimeout(probe.send(listSessionsRequest()), "kernel startup probe", 1_000)
      unwrap(response, "SessionsListed")
      await probe.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await probe.close().catch(() => {})
      await sleep(100)
    }
  }
  throw new Error(`candidate kernel did not become ready at ${endpoint}: ${lastError?.message ?? lastError}`)
}

async function expectAuthenticationFailure({ LocalIpcClient, listSessionsRequest, endpoint, token }) {
  const client = new LocalIpcClient(endpoint, {
    localAuthToken: `${token}-wrong`,
    controlRequestRetryDeadlineMs: 0,
    controlResponseStallMs: 500,
  })
  try {
    await assert.rejects(
      () => withTimeout(client.send(listSessionsRequest()), "wrong-credential probe", 2_000),
      (error) => error?.code === "authentication_failed",
    )
  } finally {
    await client.close().catch(() => {})
  }
}

function assertNoPromptReplay(session) {
  assert.equal(session.active_provider_run_id ?? null, null, "smoke session must not have a provider run")
  assert.equal(session.active_prompt ?? null, null, "smoke session must not have an active prompt")
  const promptStates = session.prompt_states && typeof session.prompt_states === "object"
    ? Object.values(session.prompt_states)
    : []
  assert.equal(promptStates.some((state) => state?.active_prompt != null), false, "smoke session must not replay a prompt")
}

async function main() {
  const options = parseCandidateKernelSmokeArgs(process.argv.slice(2))
  if (options.help) {
    console.log(candidateKernelSmokeUsage())
    return
  }
  await access(options.binary, fsConstants.X_OK)

  const runId = `${process.pid}-${Date.now()}`
  const ports = await makeAvailablePorts()
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-candidate-kernel-smoke-"))
  const workspace = path.join(rootDir, "workspace")
  const tokenFile = path.join(rootDir, "kernel-local-auth-token")
  const token = `candidate-smoke-${randomBytes(24).toString("hex")}`
  const endpoint = candidateKernelEndpoint(ports.kernelPort)
  const env = makeCandidateKernelEnvironment({
    rootDir,
    ports,
    tokenFile,
    runId,
    baseEnv: process.env,
  })
  assertNoRelayOrProviderEnvironment(env)
  await mkdir(workspace, { recursive: true })
  await writeFile(tokenFile, `${token}\n`, { mode: 0o600 })

  let kernel = null
  let client = null
  let successfulRequests = 0
  let sessionId = null
  let firstAttachmentId = null
  let secondAttachmentId = null
  let detachRequestBuilder = null
  try {
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

    const version = await runVersionProbe(options.binary, Math.min(options.timeoutMs, 5_000), {
      PATH: process.env.PATH || "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
      LANG: "C.UTF-8",
      LC_ALL: "C.UTF-8",
    })
    assertCandidateProtocolOutput(version.stdout, options.expectedProtocol)
    log("protocol-preflight-passed", { expectedProtocol: options.expectedProtocol })

    kernel = startKernel(options.binary, env, workspace)
    await waitForKernel({ LocalIpcClient, listSessionsRequest, endpoint, token, child: kernel, timeoutMs: options.timeoutMs })
    await expectAuthenticationFailure({ LocalIpcClient, listSessionsRequest, endpoint, token })
    log("wrong-credentials-rejected")

    client = new LocalIpcClient(endpoint, {
      localAuthToken: token,
      controlRequestRetryDeadlineMs: 0,
      controlResponseStallMs: 500,
    })
    const health = unwrap(await request(client, getDaemonHealthRequest(), "daemon health"), "DaemonHealth")
    successfulRequests += 1
    assert.equal(health.projection.process.process_id, kernel.pid, "health must identify the candidate process")
    assert.equal(health.projection.transport.relay_last_connected_url, null, "candidate smoke must not connect to a relay")
    assert.equal(health.projection.transport.relay_reconnect_attempts, 0, "candidate smoke must not retry a relay")

    const listedBefore = unwrap(await request(client, listSessionsRequest(), "initial session list"), "SessionsListed")
    successfulRequests += 1
    assert.equal(listedBefore.sessions.length, 0, "fresh candidate state must have no sessions")

    const created = unwrap(
      await request(
        client,
        createSessionRequest(workspace, workspace, "candidate-kernel-smoke"),
        "session create",
      ),
      "SessionCreated",
    )
    successfulRequests += 1
    sessionId = created.session.id
    assert.ok(sessionId, "candidate must create a session")
    assert.ok(created.agent?.id, "candidate session must include its default agent")
    assertNoPromptReplay(created.session)

    const attached = unwrap(
      await request(client, attachToSessionRequest(sessionId, `candidate-smoke-${runId}-first`), "session attach"),
      "SessionAttached",
    )
    successfulRequests += 1
    firstAttachmentId = attached.attachment.id
    assert.ok(firstAttachmentId, "candidate must attach a public client")

    const firstState = unwrap(await request(client, getSessionStateRequest(sessionId), "session state"), "SessionState")
    successfulRequests += 1
    assert.equal(firstState.session.id, sessionId)
    assertNoPromptReplay(firstState.session)
    await request(client, detachRequestBuilder(firstAttachmentId), "first session detach")
    successfulRequests += 1
    firstAttachmentId = null
    await client.close()
    client = null

    client = new LocalIpcClient(endpoint, {
      localAuthToken: token,
      controlRequestRetryDeadlineMs: 0,
      controlResponseStallMs: 500,
    })
    const listedAfterReopen = unwrap(await request(client, listSessionsRequest(), "reopened session list"), "SessionsListed")
    successfulRequests += 1
    assert.ok(listedAfterReopen.sessions.some((session) => session.id === sessionId), "reopened client must see the session")
    const resolved = unwrap(
      await request(client, resolveSessionRequest(sessionId), "session resolve"),
      "SessionResolved",
    )
    successfulRequests += 1
    assert.equal(resolved.session.id, sessionId)
    const reopenedState = unwrap(await request(client, getSessionStateRequest(sessionId), "reopened session state"), "SessionState")
    successfulRequests += 1
    assert.equal(reopenedState.session.id, sessionId)
    assertNoPromptReplay(reopenedState.session)

    const reattached = unwrap(
      await request(client, attachToSessionRequest(sessionId, `candidate-smoke-${runId}-second`), "reopened session attach"),
      "SessionAttached",
    )
    successfulRequests += 1
    secondAttachmentId = reattached.attachment.id
    assert.ok(secondAttachmentId)
    await request(client, detachRequestBuilder(secondAttachmentId), "second session detach")
    successfulRequests += 1
    secondAttachmentId = null
    assert.ok(successfulRequests >= 10, `smoke must complete nonzero public requests, got ${successfulRequests}`)
    log("passed", { endpoint, protocol: options.expectedProtocol, sessionId, successfulRequests })
  } finally {
    if (secondAttachmentId && client && detachRequestBuilder) await request(client, detachRequestBuilder(secondAttachmentId), "cleanup second session detach").catch(() => {})
    if (firstAttachmentId && client && detachRequestBuilder) await request(client, detachRequestBuilder(firstAttachmentId), "cleanup first session detach").catch(() => {})
    await client?.close().catch(() => {})
    await terminateChild(kernel)
    await rm(rootDir, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(`[candidate-kernel-smoke] failed: ${error.stack ?? error.message}`)
  process.exitCode = 1
})
