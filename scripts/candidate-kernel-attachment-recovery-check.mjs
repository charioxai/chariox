#!/usr/bin/env node
import assert from "node:assert/strict"
import { randomBytes } from "node:crypto"
import {
  mkdir,
  mkdtemp,
} from "node:fs/promises"
import os from "node:os"
import path from "node:path"

import {
  assertCandidateProtocolOutput,
  assertNoRelayOrProviderEnvironment,
  candidateKernelEndpoint,
  candidateKernelSmokeUsage,
  makeCandidateKernelEnvironment,
  parseCandidateKernelSmokeArgs,
} from "./candidate-kernel-startup-smoke-drill-helpers.mjs"
import {
  assertAttachmentRecoveryState,
  assertNoSubmitPromptRequest,
  assertStaleAttachmentRejected,
  createAttachmentRecoveryLedger,
} from "./candidate-kernel-attachment-recovery-helpers.mjs"
import {
  assertExactCandidateBinary,
  assertOwnedStatePaths,
  durableAgentFingerprint,
  durableSessionFingerprint,
  removeOwnedDrillRoot,
  withOwnershipPhase,
  writeOwnershipState,
} from "./staged-kernel-restart-persistence-drill-helpers.mjs"
import {
  expectAuthenticationFailure,
  processHasExited,
  runVersionProbe,
  startOwnedKernel,
  stopAndReleaseKernel,
  waitForKernel,
} from "./staged-kernel-restart-persistence-drill.mjs"
import { makeAvailablePorts } from "../apps/cli/scripts/lib/drill-runtime-helpers.mjs"

const REQUEST_TIMEOUT_MS = 5_000
const PROTOCOL_PROBE_TIMEOUT_MS = 5_000

function log(name, details = undefined) {
  if (details === undefined) {
    console.log(`[candidate-kernel-attachment-recovery] ${name}`)
  } else {
    console.log(`[candidate-kernel-attachment-recovery] ${name}`, JSON.stringify(details))
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

async function request(client, value, label, deadline) {
  assertNoSubmitPromptRequest(value, label)
  return await withTimeout(
    client.send(value),
    label,
    remainingMs(deadline, label, REQUEST_TIMEOUT_MS),
  )
}


function assertKernelHealth(health, child) {
  assert.equal(health.projection.process.process_id, child.pid, "health must identify the owned candidate process")
  assert.equal(health.projection.transport.relay_last_connected_url, null, "attachment recovery must not connect to a relay")
  assert.equal(health.projection.transport.relay_reconnect_attempts, 0, "attachment recovery must not retry a relay")
}

function assertTerminalOutput(response, label) {
  const output = unwrap(response, "TerminalOutput")
  assert.ok(Array.isArray(output.records), `${label} must return terminal output records`)
  return output
}

async function main() {
  const options = parseCandidateKernelSmokeArgs(process.argv.slice(2))
  if (options.help) {
    console.log(candidateKernelSmokeUsage().replace(
      "candidate-kernel-startup-smoke-drill.mjs",
      "candidate-kernel-attachment-recovery-check.mjs",
    ))
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
  const token = `candidate-attachment-recovery-${randomBytes(24).toString("hex")}`
  const endpoint = candidateKernelEndpoint(ports.kernelPort)
  const ledger = createAttachmentRecoveryLedger()

  let env = null
  let kernel = null
  let clientA = null
  let clientB = null
  let sessionId = null
  let agentId = null
  let attachmentA = null
  let attachmentB = null
  let replacementAttachment = null
  let expectedSession = null
  let expectedAgent = null
  let failure = null
  let passedDetails = null
  let recoveryController = null
  let recoveryDisconnected = false
  let recoveryCompleted = false
  let recoveryReset = false
  let recoverySynced = false
  let recoveryPaneRefreshes = 0
  let recoveryBusyCleared = false
  let recoveryResetPromise = null
  let recoveredAttachment = null
  let recoveredSession = null
  const recoveryFailures = []
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

    const [{ LocalIpcClient }, requests, terminalRequests] = await Promise.all([
      import("../packages/kernel-client/dist/ipc.js"),
      import("../packages/kernel-client/dist/ipc-requests.js"),
      import("../packages/kernel-client/dist/ipc-terminal-runtime-requests.js"),
    ])
    let createKernelRestartRecoveryController
    try {
      ({ createKernelRestartRecoveryController } = await import(
        "../apps/cli/dist/kernel-restart-recovery-controller.js",
      ))
    } catch (error) {
      throw new Error(
        `candidate attachment recovery requires the prebuilt CLI recovery controller at apps/cli/dist/kernel-restart-recovery-controller.js: ${error?.message ?? error}`,
      )
    }
    const {
      attachToSessionRequest,
      createSessionRequest,
      detachFromSessionRequest,
      getDaemonHealthRequest,
      getSessionStateRequest,
      listSessionsRequest,
    } = requests
    const { pumpTerminalOutputRequest } = terminalRequests

    const version = await runVersionProbe(
      binary,
      remainingMs(deadline, "protocol preflight", PROTOCOL_PROBE_TIMEOUT_MS),
    )
    assertCandidateProtocolOutput(version.stdout, options.expectedProtocol)
    log("protocol-preflight-passed", { binary, expectedProtocol: options.expectedProtocol })

    try {
      kernel = await startOwnedKernel({
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
      if (error?.child) kernel = { child: error.child, ownership: error.ownership }
      throw error
    }
    await waitForKernel({
      LocalIpcClient,
      listSessionsRequest,
      endpoint,
      token,
      child: kernel.child,
      deadline,
    })
    await writeOwnershipState(ownershipFile, withOwnershipPhase(kernel.ownership, "ready"))
    await expectAuthenticationFailure({ LocalIpcClient, listSessionsRequest, endpoint, token, deadline })
    log("wrong-credentials-rejected")

    const clientOptions = {
      localAuthToken: token,
      controlRequestRetryDeadlineMs: 0,
      controlResponseStallMs: 500,
    }
    clientA = new LocalIpcClient(endpoint, clientOptions)
    clientB = new LocalIpcClient(endpoint, clientOptions)

    const health = unwrap(
      await request(clientA, getDaemonHealthRequest(), "first daemon health", deadline),
      "DaemonHealth",
    )
    ledger.record("first daemon health")
    assertKernelHealth(health, kernel.child)

    const initialList = unwrap(
      await request(clientA, listSessionsRequest(), "initial session list", deadline),
      "SessionsListed",
    )
    ledger.record("initial session list")
    assert.equal(initialList.sessions.length, 0, "fresh candidate state must have no sessions")

    const created = unwrap(
      await request(
        clientA,
        createSessionRequest(workspace, workspace, "candidate-kernel-attachment-recovery"),
        "session create",
        deadline,
      ),
      "SessionCreated",
    )
    ledger.record("session create")
    sessionId = created.session.id
    agentId = created.agent.id
    assert.ok(sessionId, "candidate must create a session")
    assert.ok(agentId, "candidate session must include its default agent")
    assert.equal(created.agent.session_id, sessionId, "created agent must belong to the created session")

    const attachmentAResponse = unwrap(
      await request(
        clientA,
        attachToSessionRequest(sessionId, `candidate-attachment-recovery-${runId}-a`),
        "attachment A attach",
        deadline,
      ),
      "SessionAttached",
    )
    ledger.record("attachment A attach")
    attachmentA = attachmentAResponse.attachment
    assert.ok(attachmentA.id, "attachment A must have a public identity")
    assert.equal(attachmentA.session_id, sessionId)

    const stateWithA = unwrap(
      await request(clientA, getSessionStateRequest(sessionId), "attachment A state", deadline),
      "SessionState",
    )
    ledger.record("attachment A state")
    const stateWithAAgent = stateWithA.session.agents.find((agent) => agent.id === agentId)
    assert.ok(stateWithAAgent, "created agent must be present after attachment A")
    expectedSession = durableSessionFingerprint(stateWithA.session)
    expectedAgent = durableAgentFingerprint(stateWithAAgent)
    assertAttachmentRecoveryState({
      session: stateWithA.session,
      expectedSessionId: sessionId,
      expectedAttachmentIds: [attachmentA.id],
      expectedSession,
      expectedAgent,
    })

    const attachmentBResponse = unwrap(
      await request(
        clientB,
        attachToSessionRequest(sessionId, `candidate-attachment-recovery-${runId}-b`),
        "attachment B attach",
        deadline,
      ),
      "SessionAttached",
    )
    ledger.record("attachment B attach")
    attachmentB = attachmentBResponse.attachment
    assert.ok(attachmentB.id, "attachment B must have a public identity")
    assert.notEqual(attachmentB.id, attachmentA.id, "two real clients must receive distinct attachment identities")
    assert.equal(attachmentB.session_id, sessionId)
    await clientA.subscribeToKernelEvents(sessionId, attachmentA.id)
    await clientB.subscribeToKernelEvents(sessionId, attachmentB.id)

    const twoAttachmentState = unwrap(
      await request(clientB, getSessionStateRequest(sessionId), "two-attachment authoritative state", deadline),
      "SessionState",
    )
    ledger.record("two-attachment authoritative state")
    assertAttachmentRecoveryState({
      session: twoAttachmentState.session,
      expectedSessionId: sessionId,
      expectedAttachmentIds: [attachmentA.id, attachmentB.id],
      expectedSession,
      expectedAgent,
    })

    recoveryController = createKernelRestartRecoveryController({
      initialDelayMs: 1,
      maxDelayMs: 10,
      isClosing: () => false,
      isAttached: () => true,
      isDisconnected: () => recoveryDisconnected,
      getSessionId: () => sessionId,
      getSessionState: async (recoverySessionId) => {
        const state = unwrap(
          await request(
            clientA,
            getSessionStateRequest(recoverySessionId),
            "controller recovery authoritative state",
            deadline,
          ),
          "SessionState",
        )
        ledger.record("controller recovery authoritative state")
        assertAttachmentRecoveryState({
          session: state.session,
          expectedSessionId: sessionId,
          expectedAttachmentIds: [attachmentB.id],
          expectedSession,
          expectedAgent,
        })
        return state.session
      },
      attachToSession: async (recoverySessionId) => {
        const attached = unwrap(
          await request(
            clientA,
            attachToSessionRequest(
              recoverySessionId,
              `candidate-attachment-recovery-${runId}-a-recovered`,
            ),
            "controller recovery replacement attach",
            deadline,
          ),
          "SessionAttached",
        )
        ledger.record("controller recovery replacement attach")
        return attached.attachment
      },
      projectSession: (session) => session,
      applyAttachment: (attachment) => {
        recoveredAttachment = attachment
      },
      applySession: (session) => {
        recoveredSession = session
      },
      resetKernelEventSubscription: () => {
        recoveryReset = true
        recoveryResetPromise = clientA.unsubscribeFromKernelEvents()
      },
      syncKernelEventSubscription: async () => {
        await recoveryResetPromise
        assert.ok(recoveredAttachment?.id, "controller must apply the replacement before resubscribing")
        await clientA.subscribeToKernelEvents(sessionId, recoveredAttachment.id)
        recoverySynced = true
      },
      refreshAgentPanes: async () => {
        recoveryPaneRefreshes += 1
      },
      clearLocalBusyStateForAuthoritativeIdle: () => {
        recoveryBusyCleared = true
      },
      onRecovered: () => {
        recoveryCompleted = true
        recoveryDisconnected = false
      },
      onAttemptFailed: (_sessionId, error) => {
        recoveryFailures.push(error instanceof Error ? error.message : String(error))
      },
      sleep: async (delayMs) => {
        const delay = Math.max(1, Math.min(delayMs, deadline - Date.now()))
        await new Promise((resolve) => {
          const timer = setTimeout(resolve, delay)
          timer.unref?.()
        })
      },
    })

    assertTerminalOutput(
      await request(
        clientB,
        pumpTerminalOutputRequest(sessionId, attachmentB.id),
        "sibling pump while both attached",
        deadline,
      ),
      "sibling pump while both attached",
    )
    ledger.record("sibling pump while both attached")

    const detachedA = unwrap(
      await request(clientA, detachFromSessionRequest(attachmentA.id), "attachment A detach", deadline),
      "SessionDetached",
    )
    ledger.record("attachment A detach")
    assert.equal(detachedA.attachment.id, attachmentA.id)
    assert.equal(detachedA.attachment.session_id, sessionId)

    await assert.rejects(
      () => request(
        clientA,
        pumpTerminalOutputRequest(sessionId, attachmentA.id),
        "stale attachment output",
        deadline,
      ),
      (error) => assertStaleAttachmentRejected(error),
    )
    log("stale-attachment-rejected", { attachmentId: attachmentA.id, code: "attachment_not_in_session" })

    // This is the real CLI recovery controller. The stale request is the
    // disconnect signal; the controller owns the state lookup, replacement
    // attach, and recovery callbacks below.
    recoveryDisconnected = true
    const recoveryPromise = recoveryController.recover()
    assert.ok(recoveryPromise, "stale attachment must trigger the production recovery controller")
    await recoveryPromise
    assert.deepEqual(recoveryFailures, [], "production recovery controller must recover without a failed retry")
    assert.equal(recoveryCompleted, true, "production recovery controller did not report recovery")
    assert.equal(recoveryReset, true, "production recovery controller did not reset its event subscription")
    assert.equal(recoverySynced, true, "production recovery controller did not resync its event subscription")
    assert.equal(recoveryPaneRefreshes, 1, "production recovery controller did not refresh its session projection")
    assert.equal(recoveryBusyCleared, true, "production recovery controller did not clear stale local busy state")
    assert.equal(recoveredSession?.id, sessionId)
    assertAttachmentRecoveryState({
      session: recoveredSession,
      expectedSessionId: sessionId,
      expectedAttachmentIds: [attachmentB.id],
      expectedSession,
      expectedAgent,
    })
    replacementAttachment = recoveredAttachment
    assert.ok(replacementAttachment.id, "controller recovery must return a replacement attachment")
    assert.notEqual(replacementAttachment.id, attachmentA.id, "recovery must replace the stale attachment identity")
    assert.notEqual(replacementAttachment.id, attachmentB.id, "recovery must not replace the sibling attachment")
    assert.equal(replacementAttachment.session_id, sessionId)

    const postRecoveryState = unwrap(
      await request(clientB, getSessionStateRequest(sessionId), "post-recovery authoritative state", deadline),
      "SessionState",
    )
    ledger.record("post-recovery authoritative state")
    assertAttachmentRecoveryState({
      session: postRecoveryState.session,
      expectedSessionId: sessionId,
      expectedAttachmentIds: [attachmentB.id, replacementAttachment.id],
      expectedSession,
      expectedAgent,
    })

    const postRecoveryList = unwrap(
      await request(clientB, listSessionsRequest(), "post-recovery session list", deadline),
      "SessionsListed",
    )
    ledger.record("post-recovery session list")
    assert.equal(postRecoveryList.sessions.length, 1, "attachment recovery must not duplicate the session")
    assertAttachmentRecoveryState({
      session: postRecoveryList.sessions[0],
      expectedSessionId: sessionId,
      expectedAttachmentIds: [attachmentB.id, replacementAttachment.id],
      expectedSession,
      expectedAgent,
    })

    assertTerminalOutput(
      await request(
        clientB,
        pumpTerminalOutputRequest(sessionId, attachmentB.id),
        "sibling pump after replacement",
        deadline,
      ),
      "sibling pump after replacement",
    )
    ledger.record("sibling pump after replacement")

    const detachedReplacement = unwrap(
      await request(
        clientA,
        detachFromSessionRequest(replacementAttachment.id),
        "replacement attachment detach",
        deadline,
      ),
      "SessionDetached",
    )
    ledger.record("replacement attachment detach")
    assert.equal(detachedReplacement.attachment.id, replacementAttachment.id)

    const detachedSibling = unwrap(
      await request(clientB, detachFromSessionRequest(attachmentB.id), "sibling attachment detach", deadline),
      "SessionDetached",
    )
    ledger.record("sibling attachment detach")
    assert.equal(detachedSibling.attachment.id, attachmentB.id)

    const finalState = unwrap(
      await request(clientB, getSessionStateRequest(sessionId), "final authoritative state", deadline),
      "SessionState",
    )
    ledger.record("final authoritative state")
    assertAttachmentRecoveryState({
      session: finalState.session,
      expectedSessionId: sessionId,
      expectedAttachmentIds: [],
      expectedSession,
      expectedAgent,
    })
    ledger.assertComplete()
    passedDetails = {
      endpoint,
      protocol: options.expectedProtocol,
      childPid: kernel.ownership.pid,
      sessionId,
      agentId,
      attachmentAId: attachmentA.id,
      attachmentBId: attachmentB.id,
      replacementAttachmentId: replacementAttachment.id,
      staleAttachmentError: "attachment_not_in_session",
      successfulRequests: ledger.count,
    }
  } catch (error) {
    failure = error
  } finally {
    for (const [label, client] of [["client A", clientA], ["client B", clientB]]) {
      try {
        await client?.close()
      } catch (error) {
        cleanupErrors.push(new Error(`${label} cleanup failed: ${error?.message ?? error}`))
      }
    }
    if (kernel?.child) {
      if (!kernel.ownership) {
        cleanupErrors.push(new Error("candidate kernel child ownership was not established; owned root was retained"))
      } else if (!processHasExited(kernel.child)) {
        try {
          const stop = await stopAndReleaseKernel({
            child: kernel.child,
            ownership: kernel.ownership,
            ownershipFile,
            ports,
            deadline: Date.now() + 7_000,
            label: "candidate kernel",
          })
          log("child-stopped", { pid: stop.pid, signal: stop.signal, forced: stop.forced })
        } catch (error) {
          cleanupErrors.push(error)
        }
      }
    }
    const runningChild = kernel?.child && !processHasExited(kernel.child) ? kernel.child : null
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
    const cleanupFailure = new Error(`candidate kernel attachment recovery cleanup failed:\n${detail}`)
    if (failure) {
      failure = new AggregateError(
        [failure, cleanupFailure],
        "candidate kernel attachment recovery failed with cleanup errors",
      )
    } else {
      failure = cleanupFailure
    }
  }
  if (failure) throw failure
  if (passedDetails) log("passed", passedDetails)
}

main().catch((error) => {
  console.error(`[candidate-kernel-attachment-recovery] failed: ${error.stack ?? error.message}`)
  process.exitCode = 1
})
