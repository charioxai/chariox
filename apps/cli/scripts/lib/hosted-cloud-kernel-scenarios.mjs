import { mkdir } from "node:fs/promises"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"

export async function runHostedSecondKernelAssertions({
  LocalIpcClient,
  requests,
  kernelPath,
  rootDir,
  workspace,
  homeClient,
  ownerProfile,
  ownerClientId,
  apiUrl,
  repoRoot,
  pollTimeoutMs,
  log,
  assert,
  unwrap,
  makeWorkerPorts,
  pairCloudMachineDirect,
  issueMachineRelayToken,
  issueSessionScopedClientToken,
  waitForLocalDaemon,
  allowDevStubProvider,
  waitForRelayTarget,
  waitForRemoteMachine,
  waitForCompletion,
  closeClient,
  terminateChild,
  spawnProcess,
}) {
  const workerPorts = await makeWorkerPorts()
  const workerDaemonId = `hosted-worker-daemon-${process.pid}-${Date.now()}`
  const workerAlias = `hosted-worker-${process.pid}`
  const workerHome = path.join(rootDir, "worker-home")
  const workerArrobaHome = path.join(workerHome, ".arroba")

  log("second-kernel-cloud-pair-machine", { machineId: workerDaemonId, alias: workerAlias })
  await pairCloudMachineDirect({
    profile: ownerProfile,
    machineId: workerDaemonId,
    alias: workerAlias,
  })
  const workerRelayToken = await issueMachineRelayToken({
    profile: ownerProfile,
    machineId: workerDaemonId,
  })
  await mkdir(workerArrobaHome, { recursive: true })
  const workerEnv = {
    ...process.env,
    HOME: workerHome,
    ARROBA_HOME: workerArrobaHome,
    ARROBA_KERNEL_PORT: String(workerPorts.kernelPort),
    ARROBA_MCP_PORT: String(workerPorts.mcpPort),
    ARROBA_OPENCODE_PORT: String(workerPorts.opencodePort),
    ARROBA_CODEX_PORT: String(workerPorts.codexPort),
    ARROBA_RELAY_URL: ownerProfile.relayUrl,
    ARROBA_RELAY_TOKEN: workerRelayToken,
    ARROBA_DAEMON_ID: workerDaemonId,
    ARROBA_DAEMON_ALIAS: workerAlias,
    ARROBA_MACHINE_ID: workerDaemonId,
    ARROBA_MACHINE_ALIAS: workerAlias,
    ARROBA_ACCEPT_REMOTE_LEASES: "1",
    ARROBA_DAEMON_SOCKET: path.join(rootDir, "worker-daemon.sock"),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, "worker-session-history"),
  }

  let worker = null
  let workerClient = null
  const eventLog = []
  try {
    log("start-second-kernel", { workerAlias })
    worker = spawnProcess(kernelPath, [], { cwd: repoRoot, env: workerEnv, name: "worker-kernel" })
    const workerKernelUrl = `ws://127.0.0.1:${workerPorts.kernelPort}/kernel`
    await waitForLocalDaemon(LocalIpcClient, requests, workerKernelUrl, workspace)
    workerClient = new LocalIpcClient(workerKernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    await allowDevStubProvider(homeClient, requests, "second-kernel-home")
    await allowDevStubProvider(workerClient, requests, "second-kernel-worker")

    log("second-kernel-client-token-request", { workerAlias })
    const workerClientToken = await issueSessionScopedClientToken(apiUrl, {
      sessionToken: ownerProfile.cloudSessionToken,
      accountId: ownerProfile.accountId,
      realmId: ownerProfile.realmId,
      subject: ownerClientId,
      userId: ownerProfile.userId,
      clientId: ownerClientId,
      targetDaemonAlias: workerAlias,
    })
    log("second-kernel-client-token-issued", { workerAlias })
    log("second-kernel-relay-target-probe", { workerAlias })
    await waitForRelayTarget(
      LocalIpcClient,
      requests,
      ownerProfile.relayUrl,
      workerClientToken,
      workerAlias,
    )
    log("second-kernel-relay-target-ready", { workerAlias })

    await waitForRemoteMachine(homeClient, requests, workerDaemonId)
    const created = unwrap(
      await homeClient.send(requests.createSessionRequest(workspace, workspace)),
      "SessionCreated",
    )
    const session = created.session
    const attachment = unwrap(
      await homeClient.send(requests.attachToSessionRequest(session.id, `hosted-second-kernel-${Date.now()}`)),
      "SessionAttached",
    ).attachment
    homeClient.onKernelEvent((event) => {
      eventLog.push({ ...event, observed_at_ms: Date.now() })
    })
    log("second-kernel-subscribe-start", { sessionId: session.id, attachmentId: attachment.id })
    await homeClient.subscribeToKernelEvents(session.id, attachment.id)
    log("second-kernel-subscribe-ready", { sessionId: session.id, attachmentId: attachment.id })

    log("second-kernel-spawn-agent-start", { workerDaemonId })
    const spawned = unwrap(
      await homeClient.send(requests.spawnAgentRequest(
        session.id,
        "dev-stub",
        "hosted-worker-agent",
        "hosted-second-kernel",
        workspace,
        "low",
        undefined,
        undefined,
        workerDaemonId,
      )),
      "AgentSpawned",
    )
    log("second-kernel-spawn-agent-ready", { workerDaemonId, agentId: spawned.agent?.id })
    assert(spawned.agent?.remote_execution?.worker_machine_id === workerDaemonId, "remote dev-stub agent should be leased to the second kernel", spawned)
    await homeClient.send(requests.submitPromptRequest(
      session.id,
      attachment.id,
      spawned.agent.id,
      "Reply with exactly HOSTED_SECOND_KERNEL_OK.",
      [],
    ))
    const completed = unwrap(
      await homeClient.send({ CompletePrompt: { session_id: session.id } }),
      "PromptCompleted",
    )
    await waitForCompletion(eventLog, pollTimeoutMs, 0)
    await homeClient.send(requests.endSessionRequest(session.id)).catch(() => {})
    log("second-kernel-pass", {
      machineId: workerDaemonId,
      workerAlias,
      agentId: spawned.agent.id,
      completedPromptId: completed.completion?.completed?.id ?? null,
    })
  } finally {
    await closeClient(workerClient, "worker")
    await terminateChild(worker)
  }
}

export async function runHostedTokenRotationAssertions({
  requests,
  homeClient,
  verificationClient,
  sessionId,
  log,
  assert,
  unwrap,
}) {
  log("token-rotation-start", { sessionId })
  const assertSessionReachable = async (label) => {
    const listed = unwrap(
      await verificationClient.send(requests.listSessionsRequest()),
      "SessionsListed",
    )
    assert(
      (listed.sessions ?? []).some((session) => session.id === sessionId),
      `token rotation probe should list session during ${label}`,
      listed,
    )
  }

  await assertSessionReachable("before-rotation")
  let probeCount = 0
  let probeFailure = null
  const probeUntilMs = Date.now() + 15_000
  const probeTask = (async () => {
    while (Date.now() < probeUntilMs) {
      try {
        await assertSessionReachable("rotation")
        probeCount += 1
      } catch (error) {
        probeFailure = error
        break
      }
      await sleep(100)
    }
  })()

  await sleep(500)
  const rotated = unwrap(
    await homeClient.send(requests.connectCloudRelayRequest()),
    "CloudRelayConnected",
  )
  log("token-rotation-issued", {
    tokenExpiresAt: rotated.token?.token_expires_at ?? null,
  })
  await probeTask
  if (probeFailure) {
    throw probeFailure
  }
  await assertSessionReachable("after-rotation")
  log("token-rotation-pass", { probeCount })
}
