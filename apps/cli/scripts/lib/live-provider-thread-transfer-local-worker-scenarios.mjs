import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"

import { LocalIpcClient } from "../../dist/ipc.js"
import {
  materializedProviderEnvironment,
  transferProviderThreadStateToWorker,
  transferProviderStateToWorker,
} from "./live-provider-thread-transfer-provider-state.mjs"
import {
  attachToSessionRequest,
  createSessionRequest,
  endSessionRequest,
  getProviderRunRequest,
  grantAgentExtensionRequest,
  installMcpServerRequest,
  launchProviderRunRequest,
  spawnAgentRequest,
  submitPromptRequest,
  teardownProviderProcessesRequest,
} from "../../dist/ipc-requests.js"
import {
  collectProviderProcesses,
  createDeterministicMcp,
  logStep,
  mcpConfig,
  providerEffort,
  providerModel,
  providerRunSnapshot,
  providerThreadKernelEventSnapshot,
  providerThreadId,
  sendControlRequest,
  variant,
  variantAny,
  waitForActiveProviderRunChange,
  waitForHistoryOutputMarker,
  waitForPromptIdle,
  waitForProviderRun,
  waitForProviderRunEnded,
  waitForRemoteMachine,
} from "./live-provider-thread-transfer-runtime.mjs"

export async function runLocalReloadScenario({ provider, root, kernelUrl, options }) {
  const workspace = path.join(root, provider, "workspace")
  const outputsDir = path.join(workspace, "outputs")
  await mkdir(outputsDir, { recursive: true })
  await writeFile(path.join(workspace, "README.md"), `# Provider thread transfer drill for ${provider}\n`, "utf8")

  const client = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  const result = {
    drill: "local-reload",
    provider,
    status: "failed",
    started_at_ms: Date.now(),
    evidence: {},
    checks: {},
    errors: [],
  }

  let sessionId = null
  const kernelEvents = []
  try {
    client.onKernelEvent((event) => {
      kernelEvents.push(providerThreadKernelEventSnapshot(event))
    })
    logStep(result, provider, "create-session", { workspace })
    const session = variant(
      await client.send(createSessionRequest(workspace, workspace, `provider-thread-${provider}`)),
      "SessionCreated",
    ).session
    sessionId = session.id
    logStep(result, provider, "attach-session", { sessionId })
    const attachment = variant(
      await client.send(attachToSessionRequest(session.id, `provider-thread-${provider}-${Date.now()}`)),
      "SessionAttached",
    ).attachment
    await client.subscribeToKernelEvents(session.id, attachment.id)

    logStep(result, provider, "install-mcp")
    const mcpName = `thread_transfer_probe_${provider.replaceAll("-", "_")}_${process.pid}`
    const mcpPath = await createDeterministicMcp(path.join(root, provider), mcpName)
    const installedMcp = variant(
      await client.send(installMcpServerRequest(workspace, mcpConfig(mcpName, mcpPath))),
      "McpServerInstalled",
    ).mcp
    result.evidence.installed_mcp = installedMcp

    const model = providerModel(provider, options)
    const effort = providerEffort(provider)
    logStep(result, provider, "spawn-agent", { model, effort })
    const agent = variant(
      await client.send(spawnAgentRequest(
        session.id,
        provider,
        `${provider}-thread-transfer`,
        model,
        workspace,
        effort,
      )),
      "AgentSpawned",
    ).agent
    result.evidence.agent = {
      id: agent.id,
      alias: agent.alias,
      provider: agent.provider,
      model: agent.model,
      effort: agent.effort,
    }

    logStep(result, provider, "launch-provider-run")
    const launched = variantAny(
      await client.send(launchProviderRunRequest(session.id, provider, "default", model, effort, agent.id)),
      "ProviderRunLaunchAccepted",
      "ProviderRunLaunched",
    ).provider_run
    logStep(result, provider, "wait-provider-run-ready", { providerRunId: launched.id })
    let beforeRun = await waitForProviderRun({
      client,
      providerRunId: launched.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
      requireThreadId: false,
    })

    const rememberMarker = `THREAD_TRANSFER_${provider.replaceAll("-", "_").toUpperCase()}_${process.pid}_${Date.now()}`
    const readyMarker = `${rememberMarker}_READY`
    logStep(result, provider, "submit-initial-marker", { marker: rememberMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        agent.id,
        [
          `Remember this exact marker for a later recall check: ${rememberMarker}`,
          "Reply with that marker followed immediately by the suffix `_READY`, and nothing else.",
        ].join("\n"),
        [],
      ),
      `submit initial marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
    logStep(result, provider, "initial-marker-submit-accepted")
    await waitForHistoryOutputMarker({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      marker: readyMarker,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      historyDir: options.historyDir,
    })
    logStep(result, provider, "initial-marker-observed")
    await waitForPromptIdle({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    beforeRun = variant(await client.send(getProviderRunRequest(beforeRun.id)), "ProviderRun").provider_run
    const beforeThreadId = providerThreadId(beforeRun)
    if (!beforeThreadId) {
      throw new Error(`provider ${provider} did not expose a provider thread id before reload`)
    }
    result.evidence.before = providerRunSnapshot(beforeRun)
    result.evidence.remember_marker = rememberMarker

    logStep(result, provider, "grant-mcp", { mcpName })
    const grantResponse = await sendControlRequest(
      kernelUrl,
      grantAgentExtensionRequest(workspace, agent.id, "mcp", mcpName),
      `grant MCP ${mcpName}`,
      Math.min(options.timeoutMs, 60_000),
    )
    result.evidence.granted_agent = variant(grantResponse, "AgentExtensionGranted").agent
    logStep(result, provider, "wait-provider-reload", { previousRunId: beforeRun.id })
    const afterRun = await waitForActiveProviderRunChange({
      client,
      sessionId: session.id,
      previousRunId: beforeRun.id,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    const afterThreadId = providerThreadId(afterRun)
    result.evidence.after = providerRunSnapshot(afterRun)
    result.checks.provider_run_changed = beforeRun.id !== afterRun.id
    result.checks.provider_thread_id_before = beforeThreadId
    result.checks.provider_thread_id_after = afterThreadId
    result.checks.provider_thread_id_preserved = beforeThreadId === afterThreadId
    result.checks.mcp_loaded_after_reload = (afterRun.mcp_servers ?? []).some((server) => server.name === mcpName)

    if (!result.checks.provider_thread_id_preserved) {
      throw new Error(`provider thread id changed across reload: before=${beforeThreadId} after=${afterThreadId}`)
    }
    if (!result.checks.mcp_loaded_after_reload) {
      throw new Error(`reloaded run did not include MCP ${mcpName}`)
    }

    if (!options.skipRecallPrompt) {
      const recallMarker = `${rememberMarker}_SECOND_TURN_RECALLED`
      logStep(result, provider, "submit-recall-marker", { marker: recallMarker })
      await sendControlRequest(
        kernelUrl,
        submitPromptRequest(
          session.id,
          attachment.id,
          agent.id,
          "If you remember the marker from the previous turn, reply with it followed immediately by the suffix `_SECOND_TURN_RECALLED`. Do not include any other text.",
          [],
        ),
        `submit recall marker prompt for ${provider}`,
        Math.min(options.timeoutMs, 60_000),
      )
      logStep(result, provider, "recall-marker-submit-accepted")
      await waitForHistoryOutputMarker({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentId: agent.id,
        marker: recallMarker,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        historyDir: options.historyDir,
      })
      await waitForPromptIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentId: agent.id,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
      result.checks.post_reload_recall_marker_observed = true
      result.evidence.recall_marker = recallMarker
      logStep(result, provider, "recall-marker-observed")
    }

    result.evidence.provider_processes = await collectProviderProcesses(client, provider)
    result.evidence.kernel_events = kernelEvents.slice(-50)
    result.status = "passed"
    return result
  } catch (error) {
    result.errors.push(error.stack ?? error.message ?? String(error))
    result.evidence.kernel_events = kernelEvents.slice(-50)
    result.evidence.provider_processes = await collectProviderProcesses(client, provider).catch((processError) => ({
      error: processError.message ?? String(processError),
    }))
    return result
  } finally {
    if (sessionId) {
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client.close().catch(() => {})
  }
}

export async function selectWorkerKernel(client, workerMachineId, provider, timeoutMs, pollMs) {
  const kernels = await waitForRemoteMachine(client, workerMachineId, timeoutMs, pollMs)
  const selected = kernels.find((kernel) => {
    const providers = kernel.available_providers ?? []
    return kernel.accepting_remote_leases && providers.includes(provider)
  })
  if (!selected) {
    throw new Error(`no worker kernel on ${workerMachineId} advertises provider ${provider}: ${JSON.stringify(kernels, null, 2)}`)
  }
  return selected
}

export async function runWorkerResumeScenario({
  provider,
  root,
  kernelUrl,
  historyDir,
  workerMachineId,
  workerKernelId,
  workerKernelUrl,
  workerStorageRoot,
  sourceProviderEnv,
  destinationProviderEnv,
  options,
}) {
  const workspace = path.join(root, provider, "workspace")
  await mkdir(workspace, { recursive: true })
  await writeFile(path.join(workspace, "README.md"), `# Worker resume provider thread transfer drill for ${provider}\n`, "utf8")

  const client = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  const result = {
    drill: "worker-resume",
    provider,
    status: "failed",
    started_at_ms: Date.now(),
    evidence: {
      worker_machine_id: workerMachineId,
      worker_kernel_id: workerKernelId,
      worker_state: options.workerState,
      scope: "same-host worker with a kernel-materialized credential profile and explicit provider-thread state transfer; not a standard slice",
      same_chariox_agent_record: false,
    },
    checks: {},
    errors: [],
  }

  let sessionId = null
  const kernelEvents = []
  try {
    client.onKernelEvent((event) => {
      kernelEvents.push(providerThreadKernelEventSnapshot(event))
    })

    logStep(result, provider, "create-session", { workspace })
    const session = variant(
      await client.send(createSessionRequest(workspace, workspace, `provider-thread-worker-${provider}`)),
      "SessionCreated",
    ).session
    sessionId = session.id
    const attachment = variant(
      await client.send(attachToSessionRequest(session.id, `provider-thread-worker-${provider}-${Date.now()}`)),
      "SessionAttached",
    ).attachment
    await client.subscribeToKernelEvents(session.id, attachment.id)

    const model = providerModel(provider, options)
    const effort = providerEffort(provider)
    logStep(result, provider, "spawn-local-agent", { model, effort })
    const localAgent = variant(
      await client.send(spawnAgentRequest(
        session.id,
        provider,
        `${provider}-local-source`,
        model,
        workspace,
        effort,
      )),
      "AgentSpawned",
    ).agent
    result.evidence.local_agent = { id: localAgent.id, alias: localAgent.alias, model: localAgent.model }

    logStep(result, provider, "launch-local-provider-run")
    const localLaunched = variantAny(
      await client.send(launchProviderRunRequest(session.id, provider, "default", model, effort, localAgent.id)),
      "ProviderRunLaunchAccepted",
      "ProviderRunLaunched",
    ).provider_run
    let localRun = await waitForProviderRun({
      client,
      providerRunId: localLaunched.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
      requireThreadId: false,
    })

    const rememberMarker = `THREAD_TRANSFER_WORKER_${provider.replaceAll("-", "_").toUpperCase()}_${process.pid}_${Date.now()}`
    const readyMarker = `${rememberMarker}_READY`
    logStep(result, provider, "submit-local-marker", { marker: rememberMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        localAgent.id,
        [
          `Remember this exact marker for a worker resume check: ${rememberMarker}`,
          "Reply with that marker followed immediately by the suffix `_READY`, and nothing else.",
        ].join("\n"),
        [],
      ),
      `submit local marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
    await waitForHistoryOutputMarker({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: localAgent.id,
      marker: readyMarker,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      historyDir,
    })
    await waitForPromptIdle({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: localAgent.id,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    localRun = variant(await client.send(getProviderRunRequest(localRun.id)), "ProviderRun").provider_run
    const beforeThreadId = providerThreadId(localRun)
    if (!beforeThreadId) throw new Error(`provider ${provider} did not expose a provider thread id before worker resume`)
    result.evidence.local_before = providerRunSnapshot(localRun)
    result.evidence.remember_marker = rememberMarker

    logStep(result, provider, "teardown-local-provider-process", { providerRunId: localRun.id })
    const teardown = variant(
      await sendControlRequest(
        kernelUrl,
        teardownProviderProcessesRequest(provider, true),
        `teardown local ${provider} provider process`,
        Math.min(options.timeoutMs, 60_000),
      ),
      "ProviderProcessesTornDown",
    )
    result.evidence.local_teardown = teardown
    const localAfterTeardown = await waitForProviderRunEnded({
      client,
      providerRunId: localRun.id,
      timeoutMs: Math.min(options.timeoutMs, 60_000),
      pollMs: Math.min(options.pollMs, 250),
    })
    result.evidence.local_after_teardown = providerRunSnapshot(localAfterTeardown)
    result.checks.local_run_ended_before_remote_launch = String(localAfterTeardown.state ?? "").toLowerCase() === "ended"
    if (!result.checks.local_run_ended_before_remote_launch) {
      throw new Error(`local provider run ${localRun.id} was not ended before remote launch: ${JSON.stringify(result.evidence.local_after_teardown)}`)
    }

    if (options.workerState === "isolated" && provider !== "codex") {
      logStep(result, provider, "transfer-provider-state-to-worker")
      result.evidence.provider_state_transfer = await transferProviderStateToWorker({
        provider,
        sourceProviderEnv,
        destinationProviderEnv,
      })
      result.checks.provider_state_transferred = result.evidence.provider_state_transfer.copied.length > 0
      if (!result.checks.provider_state_transferred) {
        throw new Error(`provider ${provider} exposed no state to transfer to the isolated worker`)
      }
    }

    logStep(result, provider, "spawn-remote-agent", { workerKernelId })
    const remoteAgent = variant(
      await client.send(spawnAgentRequest(
        session.id,
        provider,
        `${provider}-worker-resume`,
        model,
        workspace,
        effort,
        undefined,
        undefined,
        workerKernelId,
      )),
      "AgentSpawned",
    ).agent
    result.evidence.remote_agent = {
      id: remoteAgent.id,
      alias: remoteAgent.alias,
      remote_execution: remoteAgent.remote_execution ?? null,
    }

    if (provider === "codex") {
      logStep(result, provider, "transfer-provider-thread-state-to-worker", {
        providerSessionId: beforeThreadId,
      })
      const destinationMaterializedEnv = materializedProviderEnvironment({
        provider,
        storageRoot: workerStorageRoot,
        ownerUserId: "local",
        profileId: localRun.account_profile ?? "default",
      })
      result.evidence.provider_state_transfer = await transferProviderThreadStateToWorker({
        provider,
        providerSessionId: beforeThreadId,
        sourceProviderEnv,
        destinationProviderEnv: destinationMaterializedEnv,
      })
      result.checks.provider_state_transferred = result.evidence.provider_state_transfer.copied.length > 0
    }

    logStep(result, provider, "launch-remote-provider-run", { providerSessionId: beforeThreadId })
    const remoteLaunched = variantAny(
      await client.send(launchProviderRunRequest(
        session.id,
        provider,
        "default",
        model,
        effort,
        remoteAgent.id,
        { providerSessionId: beforeThreadId, nativeTui: true },
      )),
      "ProviderRunLaunchAccepted",
      "ProviderRunLaunched",
    ).provider_run
    const remoteRun = await waitForProviderRun({
      client,
      providerRunId: remoteLaunched.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
      requireThreadId: true,
    })
    result.evidence.remote_after = providerRunSnapshot(remoteRun)
    result.checks.worker_working_directory_preserved = remoteRun.working_directory === workspace
    if (!result.checks.worker_working_directory_preserved) {
      throw new Error(
        `worker provider resumed in ${remoteRun.working_directory ?? "<unset>"}; expected ${workspace}`,
      )
    }
    const afterThreadId = providerThreadId(remoteRun)
    result.checks.provider_thread_id_before = beforeThreadId
    result.checks.provider_thread_id_after = afterThreadId
    result.checks.provider_thread_id_preserved = beforeThreadId === afterThreadId
    if (!result.checks.provider_thread_id_preserved) {
      throw new Error(`provider thread id changed across worker resume: before=${beforeThreadId} after=${afterThreadId}`)
    }

    const recallMarker = `${rememberMarker}_WORKER_RECALLED`
    logStep(result, provider, "submit-worker-recall-marker", { marker: recallMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        remoteAgent.id,
        "If you remember the marker from the previous local provider thread turn, reply with it followed immediately by the suffix `_WORKER_RECALLED`. Do not include any other text.",
        [],
      ),
      `submit worker recall marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
    await waitForHistoryOutputMarker({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: remoteAgent.id,
      marker: recallMarker,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      historyDir,
    })
    result.checks.worker_recall_marker_observed = true
    result.evidence.recall_marker = recallMarker
    await waitForPromptIdle({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: remoteAgent.id,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    const settledRemoteRun = variant(
      await client.send(getProviderRunRequest(remoteRun.id)),
      "ProviderRun",
    ).provider_run
    result.checks.worker_provider_run_remained_active = String(settledRemoteRun.state ?? "").toLowerCase() !== "ended"
    if (!result.checks.worker_provider_run_remained_active) {
      throw new Error(`worker provider run ${remoteRun.id} ended during the recall turn`)
    }

    result.evidence.provider_processes = await collectProviderProcesses(client, provider)
    result.evidence.kernel_events = kernelEvents.slice(-50)
    result.status = "passed"
    return result
  } catch (error) {
    result.errors.push(error.stack ?? error.message ?? String(error))
    result.evidence.kernel_events = kernelEvents.slice(-50)
    result.evidence.provider_processes = await collectProviderProcesses(client, provider).catch((processError) => ({
      error: processError.message ?? String(processError),
    }))
    return result
  } finally {
    if (workerKernelUrl) {
      try {
        result.evidence.worker_provider_cleanup = variant(
          await sendControlRequest(
            workerKernelUrl,
            teardownProviderProcessesRequest(provider, true),
            `tear down worker ${provider} provider process`,
            Math.min(options.timeoutMs, 60_000),
          ),
          "ProviderProcessesTornDown",
        )
      } catch (error) {
        result.errors.push(`worker provider cleanup failed: ${error.message ?? String(error)}`)
        result.status = "failed"
      }
    }
    if (sessionId) {
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client.close().catch(() => {})
  }
}
