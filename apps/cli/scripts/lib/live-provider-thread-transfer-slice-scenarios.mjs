import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"

import { LocalIpcClient } from "../../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  createSliceRequest,
  deleteSliceRequest,
  endSessionRequest,
  getProviderRunRequest,
  getSessionStateRequest,
  getSliceRequest,
  getSliceLogsRequest,
  launchProviderRunRequest,
  listSliceAuditRequest,
  moveAgentToLocalRequest,
  moveAgentToRemoteRequest,
  resetSliceStateRequest,
  saveSliceStateRequest,
  spawnAgentRequest,
  startSliceRequest,
  submitPromptRequest,
} from "../../dist/ipc-requests.js"
import {
  transferProviderStateFromSlice,
  transferProviderStateToSlice,
} from "./live-provider-thread-transfer-provider-state.mjs"
import {
  collectProviderProcesses,
  logStep,
  providerEffort,
  providerModel,
  providerRunSnapshot,
  providerThreadKernelEventSnapshot,
  providerThreadId,
  realProviderEnv,
  sendControlRequest,
  sliceRecordSnapshot,
  sliceRestartContinuityChecks,
  sliceSavedStateSnapshot,
  sliceShutdownCheckpointChecks,
  variant,
  variantAny,
  waitForHistoryOutputMarker,
  waitForPromptIdle,
  waitForProviderRun,
  waitForProviderRunEnded,
  waitForSessionActiveProviderRun,
  waitForSliceWorkerProvider,
  withTimeout,
} from "./live-provider-thread-transfer-runtime.mjs"

export async function runSliceRestartScenario({ provider, root, kernelUrl, options }) {
  const shutdownThenStart = options.drill === "slice-shutdown"
  const lifecycleLabel = shutdownThenStart ? "slice shutdown and explicit start" : "slice restart"
  const workspace = path.join(root, provider, "workspace")
  await mkdir(workspace, { recursive: true })
  await writeFile(path.join(workspace, "README.md"), `# Slice restart provider thread transfer drill for ${provider}\n`, "utf8")

  const client = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  const result = {
    drill: options.drill,
    provider,
    status: "failed",
    started_at_ms: Date.now(),
    evidence: {
      scope: shutdownThenStart
        ? "home-managed local Docker slice save/shutdown followed by explicit start with the same Chariox agent record"
        : "home-managed local Docker slice save/restart with the same Chariox agent record",
      same_chariox_agent_record: true,
    },
    checks: {},
    errors: [],
  }

  let sessionId = null
  let sliceId = null
  const kernelEvents = []
  try {
    client.onKernelEvent((event) => {
      kernelEvents.push(providerThreadKernelEventSnapshot(event))
    })

    const sliceName = `provider-thread-slice-${provider.replaceAll("-", "_")}-${process.pid}`
    logStep(result, provider, "create-slice", { sliceName, workspace })
    const createdSlice = variant(
      await withTimeout(
        client.send(createSliceRequest({
          name: sliceName,
          backend: "local_docker",
          os: "linux",
          displayMode: "headless",
          workspaceId: workspace,
          worktreeId: workspace,
          workspaceMount: workspace,
        })),
        `create slice for ${provider}`,
        Math.min(options.timeoutMs, 120_000),
      ),
      "SliceCreated",
    ).slice
    sliceId = createdSlice.id
    result.evidence.slice_created = sliceRecordSnapshot(createdSlice)

    logStep(result, provider, "start-slice", { sliceId })
    const startedSlice = variant(
      await withTimeout(
        client.send(startSliceRequest(sliceId)),
        `start slice for ${provider}`,
        options.timeoutMs,
      ),
      "SliceStarted",
    ).slice
    result.evidence.slice_started = sliceRecordSnapshot(startedSlice)
    const readySlice = await waitForSliceWorkerProvider({
      client,
      sliceRef: sliceId,
      provider,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
    })
    result.evidence.slice_ready_before_restart = sliceRecordSnapshot(readySlice)

    result.evidence.provider_account_transfer = {
      path: "kernel_execution_lease_materialization",
      account_profile: "default",
    }

    logStep(result, provider, "create-session", { workspace })
    const session = variant(
      await client.send(createSessionRequest(workspace, workspace, `provider-thread-slice-${provider}`)),
      "SessionCreated",
    ).session
    sessionId = session.id
    const attachment = variant(
      await client.send(attachToSessionRequest(session.id, `provider-thread-slice-${provider}-${Date.now()}`)),
      "SessionAttached",
    ).attachment
    await client.subscribeToKernelEvents(session.id, attachment.id)

    const model = providerModel(provider, options)
    const effort = providerEffort(provider)
    logStep(result, provider, "spawn-slice-agent", { model, effort, sliceId })
    const agent = variant(
      await client.send(spawnAgentRequest(
        session.id,
        provider,
        `${provider}-slice-transfer`,
        model,
        workspace,
        effort,
        undefined,
        undefined,
        undefined,
        undefined,
        sliceId,
      )),
      "AgentSpawned",
    ).agent
    result.evidence.agent = {
      id: agent.id,
      alias: agent.alias,
      provider: agent.provider,
      model: agent.model,
      effort: agent.effort,
      remote_execution: agent.remote_execution ?? null,
    }

    logStep(result, provider, "launch-slice-provider-run")
    const launched = variantAny(
      await client.send(launchProviderRunRequest(
        session.id,
        provider,
        "default",
        model,
        effort,
        agent.id,
        { nativeTui: true },
      )),
      "ProviderRunLaunchAccepted",
      "ProviderRunLaunched",
    ).provider_run
    let beforeRun = await waitForProviderRun({
      client,
      providerRunId: launched.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
      requireThreadId: false,
    })

    const rememberMarker = `THREAD_TRANSFER_SLICE_${provider.replaceAll("-", "_").toUpperCase()}_${process.pid}_${Date.now()}`
    const readyMarker = `${rememberMarker}_READY`
    logStep(result, provider, "submit-slice-marker", { marker: rememberMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        agent.id,
        [
          `Remember this exact marker for a slice restart check: ${rememberMarker}`,
          "Reply with that marker followed immediately by the suffix `_READY`, and nothing else.",
        ].join("\n"),
        [],
      ),
      `submit slice marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
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
    if (!beforeThreadId) throw new Error(`provider ${provider} did not expose a provider thread id before slice restart`)
    const sliceBeforeRestart = variant(await client.send(getSliceRequest(sliceId)), "Slice").slice
    result.evidence.before = providerRunSnapshot(beforeRun)
    result.evidence.slice_before_restart = sliceRecordSnapshot(sliceBeforeRestart)
    result.evidence.remember_marker = rememberMarker

    const saveMode = shutdownThenStart ? "shutdown" : "restart_agents"
    logStep(result, provider, shutdownThenStart ? "save-slice-state-shutdown" : "save-slice-state-restart-agents", {
      sliceId,
      providerSessionId: beforeThreadId,
    })
    const savedState = variant(
      await withTimeout(
        client.send(saveSliceStateRequest(sliceId, saveMode, "this_slice")),
        `save ${lifecycleLabel} for ${provider}`,
        options.timeoutMs,
      ),
      "SliceStateSaved",
    )
    result.evidence.slice_state_saved = {
      slice: sliceRecordSnapshot(savedState.slice),
      state: sliceSavedStateSnapshot(savedState.state),
    }

    let restartedSlice = savedState.slice
    if (shutdownThenStart) {
      const parkedRun = await waitForProviderRunEnded({
        client,
        providerRunId: beforeRun.id,
        timeoutMs: Math.min(options.timeoutMs, 60_000),
        pollMs: options.pollMs,
      })
      const stoppedState = variantAny(
        await client.send(getSessionStateRequest(session.id)),
        "SessionState",
        "SessionStateLoaded",
      )
      const stoppedSession = stoppedState.session ?? stoppedState
      result.evidence.provider_after_shutdown = providerRunSnapshot(parkedRun)
      result.evidence.session_after_shutdown = {
        id: stoppedSession.id ?? null,
        active_provider_run_id: stoppedSession.active_provider_run_id ?? null,
      }
      Object.assign(result.checks, sliceShutdownCheckpointChecks({
        savedSlice: savedState.slice,
        parkedRun,
        stoppedSession,
      }))
      if (!result.checks.slice_shutdown_checkpoint_valid) {
        throw new Error(`slice shutdown did not leave ${sliceId} stopped with provider run ${beforeRun.id} parked`)
      }

      logStep(result, provider, "start-slice-explicitly", { sliceId })
      restartedSlice = variant(
        await withTimeout(
          client.send(startSliceRequest(sliceId)),
          `explicitly start slice for ${provider}`,
          options.timeoutMs,
        ),
        "SliceStarted",
      ).slice
      result.evidence.slice_started_explicitly = sliceRecordSnapshot(restartedSlice)
      result.checks.slice_explicit_start_completed = String(restartedSlice.status ?? "").toLowerCase() === "running"
      if (!result.checks.slice_explicit_start_completed) {
        throw new Error(`explicit start did not return ${sliceId} to running state`)
      }
    }

    const afterRun = await waitForSessionActiveProviderRun({
      client,
      sessionId: session.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
    })
    const afterThreadId = providerThreadId(afterRun)
    const afterState = variantAny(await client.send(getSessionStateRequest(session.id)), "SessionState", "SessionStateLoaded")
    const afterSession = afterState.session ?? afterState
    const afterAgent = (afterSession.agents ?? []).find((entry) => entry.id === agent.id)
    result.evidence.after = providerRunSnapshot(afterRun)
    result.evidence.agent_after_restart = {
      id: afterAgent?.id ?? null,
      alias: afterAgent?.alias ?? null,
      remote_execution: afterAgent?.remote_execution ?? null,
    }
    result.checks.same_chariox_agent_record = afterAgent?.id === agent.id
    result.checks.slice_working_directory_before = beforeRun.working_directory ?? null
    result.checks.slice_working_directory_after = afterRun.working_directory ?? null
    result.checks.slice_working_directory_preserved = (
      beforeRun.working_directory === "/workspace"
      && afterRun.working_directory === beforeRun.working_directory
    )
    result.checks.provider_thread_id_before = beforeThreadId
    result.checks.provider_thread_id_after = afterThreadId
    result.checks.provider_thread_id_preserved = beforeThreadId === afterThreadId
    result.checks.provider_run_relaunched = beforeRun.id !== afterRun.id
    result.checks.account_profile_preserved = beforeRun.account_profile === afterRun.account_profile
    result.checks.execution_mode_preserved = beforeRun.execution_mode === afterRun.execution_mode
    result.checks.permission_level_preserved = beforeRun.permission_level === afterRun.permission_level
    const beforeBinding = agent.remote_execution ?? null
    const afterBinding = afterAgent?.remote_execution ?? null
    Object.assign(result.checks, sliceRestartContinuityChecks({
      beforeRun,
      afterRun,
      beforeBinding,
      afterBinding,
      sliceBeforeRestart,
      restartedSlice,
      savedState: savedState.state,
    }))
    if (!result.checks.same_chariox_agent_record) {
      throw new Error(`${lifecycleLabel} did not preserve same Chariox agent record ${agent.id}`)
    }
    if (!result.checks.slice_working_directory_preserved) {
      throw new Error(
        `slice provider working directory changed across restart: before=${beforeRun.working_directory ?? "<unset>"} after=${afterRun.working_directory ?? "<unset>"}`,
      )
    }
    if (!result.checks.provider_thread_id_preserved) {
      throw new Error(`provider thread id changed across ${lifecycleLabel}: before=${beforeThreadId} after=${afterThreadId}`)
    }
    if (!result.checks.provider_run_relaunched) {
      throw new Error(`${lifecycleLabel} reused stale provider run ${beforeRun.id}`)
    }
    if (!result.checks.account_profile_preserved) {
      throw new Error(
        `provider account changed across ${lifecycleLabel}: before=${beforeRun.account_profile ?? "<unset>"} after=${afterRun.account_profile ?? "<unset>"}`,
      )
    }
    if (!result.checks.execution_mode_preserved || !result.checks.permission_level_preserved) {
      throw new Error(
        `provider execution authority changed across ${lifecycleLabel}: mode ${beforeRun.execution_mode ?? "<unset>"}->${afterRun.execution_mode ?? "<unset>"}, permission ${beforeRun.permission_level ?? "<unset>"}->${afterRun.permission_level ?? "<unset>"}`,
      )
    }
    if (!result.checks.agent_binding_repaired) {
      throw new Error(`${lifecycleLabel} did not replace the remote execution binding for ${agent.id}`)
    }
    if (!result.checks.slice_worker_identity_preserved) {
      throw new Error(`${lifecycleLabel} changed durable worker identity for ${sliceId}`)
    }
    if (!result.checks.slice_restart_timeline_valid) {
      throw new Error(`${lifecycleLabel} timestamps do not prove save-before-relaunch ordering for ${agent.id}`)
    }
    if (!result.checks.slice_restart_completed) {
      throw new Error(`${lifecycleLabel} did not produce a running slice with fresh execution for ${agent.id}`)
    }

    const recallMarker = `${rememberMarker}_SLICE_RECALLED`
    logStep(result, provider, "submit-slice-recall-marker", { marker: recallMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        agent.id,
        `If you remember the marker from before the ${lifecycleLabel}, reply with it followed immediately by the suffix \`_SLICE_RECALLED\`. Do not include any other text.`,
        [],
      ),
      `submit slice recall marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
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
    result.checks.slice_recall_marker_observed = true
    result.evidence.recall_marker = recallMarker
    await waitForPromptIdle({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    const settledAfterRun = variant(
      await client.send(getProviderRunRequest(afterRun.id)),
      "ProviderRun",
    ).provider_run
    result.checks.slice_provider_run_remained_active = String(settledAfterRun.state ?? "").toLowerCase() !== "ended"
    if (!result.checks.slice_provider_run_remained_active) {
      throw new Error(`slice provider run ${afterRun.id} ended during the recall turn`)
    }

    result.evidence.provider_processes = await collectProviderProcesses(client, provider)
    result.evidence.kernel_events = kernelEvents.slice(-50)
    result.status = "passed"
    return result
  } catch (error) {
    result.errors.push(error.stack ?? error.message ?? String(error))
    result.evidence.kernel_events = kernelEvents.slice(-50)
    if (sliceId) {
      result.evidence.slice_failure_diagnostics = await collectSliceFailureDiagnostics(
        client,
        sliceId,
      )
    }
    result.evidence.provider_processes = await collectProviderProcesses(client, provider).catch((processError) => ({
      error: processError.message ?? String(processError),
    }))
    return result
  } finally {
    if (sessionId) {
      await client.send(endSessionRequest(sessionId)).catch((error) => {
        result.evidence.session_cleanup_error = error.message ?? String(error)
      })
    }
    if (sliceId && !(options.keepSliceOnFailure && result.status !== "passed")) {
      await cleanupSliceRuntime(client, sliceId, result.evidence, { resetSavedState: true })
      failResultOnSliceCleanupErrors(result, { resetSavedState: true })
    } else if (sliceId) {
      result.evidence.slice_left_running_for_debug = sliceId
    }
    await client.close().catch(() => {})
  }
}

export async function runSliceShutdownScenario(args) {
  return await runSliceRestartScenario(args)
}

export async function runLiveMigrateToSliceScenario({ provider, root, kernelUrl, options }) {
  const roundTrip = options.drill === "live-migrate-roundtrip-slice"
  const workspace = path.join(root, provider, "workspace")
  await mkdir(workspace, { recursive: true })
  await writeFile(path.join(workspace, "README.md"), `# Live local-to-slice provider thread transfer drill for ${provider}\n`, "utf8")

  const client = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  const result = {
    drill: options.drill,
    provider,
    status: "failed",
    started_at_ms: Date.now(),
    evidence: {
      scope: roundTrip
        ? "provider thread starts on the main machine, the same Chariox agent record is moved to a local Docker slice and then back to local execution, and both provider runs resume the captured provider thread"
        : "provider thread starts on the main machine, the same Chariox agent record is moved to a local Docker slice, and the slice provider run resumes the captured provider thread",
      same_chariox_agent_record_required: true,
      same_provider_thread_required: true,
    },
    checks: {},
    errors: [],
  }

  let sessionId = null
  let sliceId = null
  const kernelEvents = []
  try {
    client.onKernelEvent((event) => {
      kernelEvents.push(providerThreadKernelEventSnapshot(event))
    })

    logStep(result, provider, "create-local-session", { workspace })
    const session = variant(
      await client.send(createSessionRequest(workspace, workspace, `provider-thread-live-migrate-${provider}`)),
      "SessionCreated",
    ).session
    sessionId = session.id
    const attachment = variant(
      await client.send(attachToSessionRequest(session.id, `provider-thread-live-migrate-${provider}-${Date.now()}`)),
      "SessionAttached",
    ).attachment
    await client.subscribeToKernelEvents(session.id, attachment.id)

    const model = providerModel(provider, options)
    const effort = providerEffort(provider)
    logStep(result, provider, "spawn-local-agent", { model, effort })
    const agent = variant(
      await client.send(spawnAgentRequest(
        session.id,
        provider,
        `${provider}-live-migrate`,
        model,
        workspace,
        effort,
      )),
      "AgentSpawned",
    ).agent
    result.evidence.agent_before_migration = {
      id: agent.id,
      alias: agent.alias,
      provider: agent.provider,
      model: agent.model,
      effort: agent.effort,
      remote_execution: agent.remote_execution ?? null,
    }

    logStep(result, provider, "launch-local-provider-run")
    const launched = variantAny(
      await client.send(launchProviderRunRequest(session.id, provider, "default", model, effort, agent.id)),
      "ProviderRunLaunchAccepted",
      "ProviderRunLaunched",
    ).provider_run
    let localRun = await waitForProviderRun({
      client,
      providerRunId: launched.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
      requireThreadId: false,
    })

    const rememberMarker = `THREAD_TRANSFER_LIVE_SLICE_${provider.replaceAll("-", "_").toUpperCase()}_${process.pid}_${Date.now()}`
    const readyMarker = `${rememberMarker}_READY`
    logStep(result, provider, "submit-local-marker", { marker: rememberMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        agent.id,
        [
          `Remember this exact marker for a live local-to-slice migration check: ${rememberMarker}`,
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
      agentId: agent.id,
      marker: readyMarker,
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
    localRun = variant(await client.send(getProviderRunRequest(localRun.id)), "ProviderRun").provider_run
    const beforeThreadId = providerThreadId(localRun)
    if (!beforeThreadId) throw new Error(`provider ${provider} did not expose a provider thread id before live slice migration`)
    result.evidence.local_before = providerRunSnapshot(localRun)
    result.evidence.remember_marker = rememberMarker

    const sliceName = `provider-thread-live-${provider.replaceAll("-", "_")}-${process.pid}`
    logStep(result, provider, "create-slice", { sliceName, workspace })
    const createdSlice = variant(
      await withTimeout(
        client.send(createSliceRequest({
          name: sliceName,
          backend: "local_docker",
          os: "linux",
          displayMode: "headless",
          workspaceId: workspace,
          worktreeId: workspace,
          workspaceMount: workspace,
        })),
        `create live migration slice for ${provider}`,
        Math.min(options.timeoutMs, 120_000),
      ),
      "SliceCreated",
    ).slice
    sliceId = createdSlice.id
    result.evidence.slice_created = sliceRecordSnapshot(createdSlice)

    logStep(result, provider, "start-slice", { sliceId })
    const startedSlice = variant(
      await withTimeout(
        client.send(startSliceRequest(sliceId)),
        `start live migration slice for ${provider}`,
        options.timeoutMs,
      ),
      "SliceStarted",
    ).slice
    result.evidence.slice_started = sliceRecordSnapshot(startedSlice)
    const readySlice = await waitForSliceWorkerProvider({
      client,
      sliceRef: sliceId,
      provider,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
    })
    result.evidence.slice_ready = sliceRecordSnapshot(readySlice)

    logStep(result, provider, "transfer-provider-state-to-slice", { sliceName })
    result.evidence.provider_state_transfer = await transferProviderStateToSlice({
      provider,
      root,
      sliceName,
      timeoutMs: options.timeoutMs,
      providerEnv: options.providerStateSourceEnv ?? realProviderEnv(),
    })

    result.evidence.provider_account_transfer = {
      path: "kernel_execution_lease_materialization",
      account_profile: "default",
    }

    const machineRef = readySlice.worker_machine_id ?? `slice:${sliceId}`
    logStep(result, provider, "move-same-agent-to-slice", { agentId: agent.id, machineRef })
    const movedAgent = variant(
      await withTimeout(
        client.send(moveAgentToRemoteRequest(session.id, agent.id, machineRef)),
        `move same agent to slice for ${provider}`,
        Math.min(options.timeoutMs, 120_000),
      ),
      "AgentMovedToRemote",
    ).agent
    result.evidence.agent_after_move = {
      id: movedAgent.id,
      alias: movedAgent.alias,
      provider: movedAgent.provider,
      model: movedAgent.model,
      effort: movedAgent.effort,
      remote_execution: movedAgent.remote_execution ?? null,
    }
    result.checks.same_chariox_agent_record_after_move = movedAgent.id === agent.id
    if (!result.checks.same_chariox_agent_record_after_move) {
      throw new Error(`move returned a different Chariox agent record: before=${agent.id} after=${movedAgent.id}`)
    }
    if (!movedAgent.remote_execution) {
      throw new Error(`agent ${agent.id} was not remote-backed after move to slice`)
    }
    const localAfterMove = await waitForProviderRunEnded({
      client,
      providerRunId: localRun.id,
      timeoutMs: Math.min(options.timeoutMs, 60_000),
      pollMs: Math.min(options.pollMs, 250),
    })
    result.evidence.local_after_move = providerRunSnapshot(localAfterMove)
    result.checks.local_run_ended_by_move = String(localAfterMove.state ?? "").toLowerCase() === "ended"
    if (!result.checks.local_run_ended_by_move) {
      throw new Error(`local provider run ${localRun.id} was not ended by move: ${JSON.stringify(result.evidence.local_after_move)}`)
    }

    logStep(result, provider, "launch-slice-provider-run", { providerSessionId: beforeThreadId })
    const sliceLaunched = variantAny(
      await client.send(launchProviderRunRequest(
        session.id,
        provider,
        "default",
        model,
        effort,
        agent.id,
        { providerSessionId: beforeThreadId, nativeTui: true },
      )),
      "ProviderRunLaunchAccepted",
      "ProviderRunLaunched",
    ).provider_run
    const sliceRun = await waitForProviderRun({
      client,
      providerRunId: sliceLaunched.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
      requireThreadId: true,
    })
    const afterThreadId = providerThreadId(sliceRun)
    result.evidence.slice_after = providerRunSnapshot(sliceRun)
    result.checks.provider_thread_id_before = beforeThreadId
    result.checks.provider_thread_id_after = afterThreadId
    result.checks.provider_thread_id_preserved = beforeThreadId === afterThreadId
    if (!result.checks.provider_thread_id_preserved) {
      throw new Error(`provider thread id changed across live slice migration: before=${beforeThreadId} after=${afterThreadId}`)
    }

    const recallMarker = `${rememberMarker}_SLICE_MIGRATED`
    logStep(result, provider, "submit-slice-recall-marker", { marker: recallMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        agent.id,
        "If you remember the marker from before the local-to-slice migration, reply with it followed immediately by the suffix `_SLICE_MIGRATED`. Do not include any other text.",
        [],
      ),
      `submit slice migration recall marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
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

    if (roundTrip) {
      const reverseMarker = `${rememberMarker}_RETURN_CONTEXT`
      const reverseReadyMarker = `${reverseMarker}_READY`
      logStep(result, provider, "submit-slice-return-marker", { marker: reverseMarker })
      await sendControlRequest(
        kernelUrl,
        submitPromptRequest(
          session.id,
          attachment.id,
          agent.id,
          [
            `Remember this exact marker for the slice-to-local return check: ${reverseMarker}`,
            "Reply with that marker followed immediately by the suffix `_READY`, and nothing else.",
          ].join("\n"),
          [],
        ),
        `submit slice return marker prompt for ${provider}`,
        Math.min(options.timeoutMs, 60_000),
      )
      await waitForHistoryOutputMarker({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentId: agent.id,
        marker: reverseReadyMarker,
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

      logStep(result, provider, "transfer-provider-state-from-slice", { sliceName })
      result.evidence.provider_state_reverse_transfer = await transferProviderStateFromSlice({
        provider,
        root,
        sliceName,
        timeoutMs: options.timeoutMs,
        providerEnv: options.providerStateSourceEnv ?? realProviderEnv(),
      })

      const sliceRunBeforeReturn = variant(await client.send(getProviderRunRequest(sliceRun.id)), "ProviderRun").provider_run
      result.evidence.slice_before_return = providerRunSnapshot(sliceRunBeforeReturn)
      logStep(result, provider, "move-same-agent-to-local", { agentId: agent.id })
      const returnedAgent = variant(
        await withTimeout(
          client.send(moveAgentToLocalRequest(session.id, agent.id)),
          `move same agent back local for ${provider}`,
          Math.min(options.timeoutMs, 120_000),
        ),
        "AgentMovedToLocal",
      ).agent
      result.evidence.agent_after_return = {
        id: returnedAgent.id,
        alias: returnedAgent.alias,
        provider: returnedAgent.provider,
        model: returnedAgent.model,
        effort: returnedAgent.effort,
        remote_execution: returnedAgent.remote_execution ?? null,
      }
      result.checks.same_chariox_agent_record_after_return = returnedAgent.id === agent.id
      result.checks.agent_local_after_return = returnedAgent.remote_execution == null
      if (!result.checks.same_chariox_agent_record_after_return) {
        throw new Error(`return move returned a different Chariox agent record: before=${agent.id} after=${returnedAgent.id}`)
      }
      if (!result.checks.agent_local_after_return) {
        throw new Error(`agent ${agent.id} was still remote-backed after move back to local`)
      }

      const sliceRunAfterReturn = await waitForProviderRunEnded({
        client,
        providerRunId: sliceRun.id,
        timeoutMs: Math.min(options.timeoutMs, 60_000),
        pollMs: Math.min(options.pollMs, 250),
      })
      result.evidence.slice_after_return = providerRunSnapshot(sliceRunAfterReturn)
      result.checks.slice_run_ended_by_return_move = String(sliceRunAfterReturn.state ?? "").toLowerCase() === "ended"
      if (!result.checks.slice_run_ended_by_return_move) {
        throw new Error(`slice provider run ${sliceRun.id} was not ended by return move: ${JSON.stringify(result.evidence.slice_after_return)}`)
      }

      logStep(result, provider, "launch-returned-local-provider-run", { providerSessionId: afterThreadId })
      const returnedLaunched = variantAny(
        await client.send(launchProviderRunRequest(
          session.id,
          provider,
          "default",
          model,
          effort,
          agent.id,
          { providerSessionId: afterThreadId, nativeTui: true },
        )),
        "ProviderRunLaunchAccepted",
        "ProviderRunLaunched",
      ).provider_run
      const returnedLocalRun = await waitForProviderRun({
        client,
        providerRunId: returnedLaunched.id,
        timeoutMs: Math.min(options.timeoutMs, 180_000),
        pollMs: options.pollMs,
        requireThreadId: true,
      })
      const returnedThreadId = providerThreadId(returnedLocalRun)
      result.evidence.local_after_return = providerRunSnapshot(returnedLocalRun)
      result.checks.provider_thread_id_returned = returnedThreadId
      result.checks.provider_thread_id_preserved_after_return = beforeThreadId === returnedThreadId
      if (!result.checks.provider_thread_id_preserved_after_return) {
        throw new Error(`provider thread id changed across slice-to-local return: before=${beforeThreadId} returned=${returnedThreadId}`)
      }

      const returnRecallMarker = `${rememberMarker}_LOCAL_RETURNED`
      logStep(result, provider, "submit-returned-local-recall-marker", { marker: returnRecallMarker })
      await sendControlRequest(
        kernelUrl,
        submitPromptRequest(
          session.id,
          attachment.id,
          agent.id,
          `If you remember both ${rememberMarker} and ${reverseMarker} after returning from the slice to local execution, reply with the first marker followed immediately by the suffix \`_LOCAL_RETURNED\`. Do not include any other text.`,
          [],
        ),
        `submit returned local recall marker prompt for ${provider}`,
        Math.min(options.timeoutMs, 60_000),
      )
      await waitForHistoryOutputMarker({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentId: agent.id,
        marker: returnRecallMarker,
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
      result.evidence.return_marker = reverseMarker
      result.evidence.return_recall_marker = returnRecallMarker
      result.checks.return_recall_marker_observed = true
    }

    const finalState = variantAny(await client.send(getSessionStateRequest(session.id)), "SessionState", "SessionStateLoaded")
    const finalSession = finalState.session ?? finalState
    const finalAgent = (finalSession.agents ?? []).find((entry) => entry.id === agent.id)
    const localRunFinal = variant(await client.send(getProviderRunRequest(localRun.id)), "ProviderRun").provider_run
    result.evidence.agent_final = {
      id: finalAgent?.id ?? null,
      alias: finalAgent?.alias ?? null,
      provider: finalAgent?.provider ?? null,
      model: finalAgent?.model ?? null,
      effort: finalAgent?.effort ?? null,
      remote_execution: finalAgent?.remote_execution ?? null,
      provider_resume_state: finalAgent?.provider_resume_state ?? null,
    }
    result.evidence.local_run_final = providerRunSnapshot(localRunFinal)
    result.checks.same_chariox_agent_record_final = finalAgent?.id === agent.id
    result.checks.local_run_still_ended_after_slice_launch = String(localRunFinal.state ?? "").toLowerCase() === "ended"
    result.checks.agent_execution_location_final = finalAgent?.remote_execution == null ? "local" : "remote"
    const expectedModel = provider === "codex" && agent.model && !agent.model.startsWith("codex/")
      ? `codex/${agent.model}`
      : agent.model
    result.checks.agent_original_provider_config_restored = finalAgent?.provider === agent.provider
      && (finalAgent?.model === agent.model || finalAgent?.model === expectedModel)
      && finalAgent?.effort === agent.effort
    result.checks.slice_recall_marker_observed = true
    result.evidence.recall_marker = recallMarker
    if (!result.checks.same_chariox_agent_record_final) {
      throw new Error(`same Chariox agent record was not present after migration: ${agent.id}`)
    }
    if (!result.checks.local_run_still_ended_after_slice_launch) {
      throw new Error(`old local provider run was not still ended after slice launch: ${JSON.stringify(result.evidence.local_run_final)}`)
    }
    if (roundTrip && result.checks.agent_execution_location_final !== "local") {
      throw new Error(`agent ${agent.id} did not finish local after round trip: ${JSON.stringify(result.evidence.agent_final)}`)
    }
    if (!result.checks.agent_original_provider_config_restored) {
      throw new Error(`agent ${agent.id} provider config changed across migration: ${JSON.stringify(result.evidence.agent_final)}`)
    }

    result.evidence.provider_processes = await collectProviderProcesses(client, provider)
    result.evidence.kernel_events = kernelEvents.slice(-80)
    result.status = "passed"
    return result
  } catch (error) {
    result.errors.push(error.stack ?? error.message ?? String(error))
    result.evidence.kernel_events = kernelEvents.slice(-80)
    if (sliceId) {
      result.evidence.slice_failure_diagnostics = await collectSliceFailureDiagnostics(
        client,
        sliceId,
      )
    }
    result.evidence.provider_processes = await collectProviderProcesses(client, provider).catch((processError) => ({
      error: processError.message ?? String(processError),
    }))
    return result
  } finally {
    if (sessionId) {
      await client.send(endSessionRequest(sessionId)).catch((error) => {
        result.evidence.session_cleanup_error = error.message ?? String(error)
      })
    }
    if (sliceId && !(options.keepSliceOnFailure && result.status !== "passed")) {
      await cleanupSliceRuntime(client, sliceId, result.evidence)
      failResultOnSliceCleanupErrors(result)
    } else if (sliceId) {
      result.evidence.slice_left_running_for_debug = sliceId
    }
    await client.close().catch(() => {})
  }
}

export async function cleanupSliceRuntime(
  client,
  sliceId,
  evidence,
  { resetSavedState = false } = {},
) {
  if (resetSavedState) {
    await client.send(resetSliceStateRequest(sliceId)).catch((error) => {
      evidence.slice_state_cleanup_error = error.message ?? String(error)
    })
  }
  await client.send(deleteSliceRequest(sliceId)).catch((error) => {
    evidence.slice_cleanup_error = error.message ?? String(error)
  })
}

export function failResultOnSliceCleanupErrors(result, { resetSavedState = false } = {}) {
  const errors = [
    ...(resetSavedState ? [result.evidence.slice_state_cleanup_error] : []),
    result.evidence.slice_cleanup_error,
  ].filter(Boolean)
  if (errors.length === 0) return
  result.status = "failed"
  result.errors.push(`slice cleanup failed: ${errors.join(": ")}`)
}

async function collectSliceFailureDiagnostics(client, sliceId) {
  const diagnostics = {}
  try {
    const logs = variant(
      await client.send(getSliceLogsRequest(sliceId, 200)),
      "SliceLogs",
    )
    diagnostics.logs = (logs.entries ?? []).map((entry) => ({
      source: entry.source ?? null,
      text: entry.text ?? "",
      truncated: entry.truncated ?? false,
    }))
  } catch (error) {
    diagnostics.logs_error = error.message ?? String(error)
  }
  try {
    const audit = variant(
      await client.send(listSliceAuditRequest(sliceId, 100)),
      "SliceAuditListed",
    )
    diagnostics.audit = audit.events ?? []
  } catch (error) {
    diagnostics.audit_error = error.message ?? String(error)
  }
  return diagnostics
}
