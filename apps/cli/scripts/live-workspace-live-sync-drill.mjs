import { spawn } from 'node:child_process'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'
import { parseArgs, printHelp } from './lib/workspace-live-sync-drill-options.mjs'
import { assertFileBytes, assertFileContent, fileExists, initGitWorktree, initManagedTargetWorkspace, initTrackedWorkspace, loadCliModules, makePorts, modelForProvider, resolveBinary, resolveRemoteWorkerKernelRef, runAfterFixtureCommand, sleep, spawnWorkspaceLiveSyncPhaseAgents, terminateChild, unwrap, unwrapVariant, waitForLocalDaemon, workspaceLiveSyncMoveSourceName, workspaceLiveSyncSpawnAgentRequest, workspaceLiveSyncToolNames, wrapClientSendWithTimeout } from './lib/workspace-live-sync-drill-runtime.mjs'
import { assertFilesAbsent, assertManagedTargetFanout, managedTargetFanoutSnapshot, waitForAgentsIdle, waitForCompletionCount, waitForCompletionsAndFiles, waitForFilesAbsent, waitForHistoryNotices, waitForHistoryOutputMarkers, waitForManagedToolExpectationsAndFiles, waitForPromptPhase } from './lib/workspace-live-sync-drill-waiters.mjs'
import { runLiveCollisionAndExternalChecks } from './lib/workspace-live-sync-drill-collision-scenarios.mjs'
import { runTrackedWorkspaceLiveSyncDrill } from './lib/workspace-live-sync-drill-scenarios.mjs'
import {
  prepareWorkspaceLiveSyncDaemonEnvironment,
  removeWorkspaceLiveSyncProviderProfile,
} from './lib/workspace-live-sync-drill-environment.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  if (options.providers.length === 0) {
    throw new Error('at least one provider is required')
  }
  if (options.mode === 'tracked' && options.providers.length !== 1) {
    throw new Error('tracked live drill currently runs one provider at a time')
  }

  const runtimeDir = path.join(
    cliRoot,
    `.tmp-live-workspace-live-sync-drill-${process.pid}-${Date.now()}`,
  )
  // Keep the live workspace out of OS temp directories: Codex read-only mode may
  // allow TMPDIR writes, which would make the negative direct-write probe invalid.
  const rootDir = options.rootDir ?? path.join(cliRoot, 'target', 'live-workspace-live-sync-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const siblingWorkspace = path.join(rootDir, 'sibling-repo')
  const targetWorkspace = path.join(rootDir, 'target-workspace')
  const targetCount = options.mode === 'off' ? 0 : (options.mode === 'tracked' ? options.trackedTargetCount : options.managedTargetCount)
  const targetWorkspaces = [
    targetWorkspace,
    ...Array.from(
      { length: Math.max(0, targetCount - 1) },
      (_, index) => path.join(rootDir, `target-workspace-${index + 2}`),
    ),
  ].slice(0, targetCount)
  const outputsDir = path.join(workspace, 'outputs')
  await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(runtimeDir, { recursive: true })
  await prepareDrillArtifacts(rootDir)
  await mkdir(outputsDir, { recursive: true })
  await writeFile(path.join(workspace, 'seed.txt'), 'seed-value-42\n', 'utf8')
  for (const provider of options.providers) {
    await writeFile(path.join(outputsDir, `${provider}-delete-me.txt`), 'delete-me\n', 'utf8')
    await writeFile(path.join(outputsDir, `${provider}-opaque-delete-me.bin`), Buffer.from([9, 8, 7]))
  }
  if (options.mode === 'tracked') {
    await initTrackedWorkspace(workspace, options.providers[0])
    await mkdir(siblingWorkspace, { recursive: true })
    await writeFile(path.join(siblingWorkspace, 'README.md'), 'sibling repo\n', 'utf8')
    await initGitWorktree(siblingWorkspace)
    for (const target of targetWorkspaces) {
      await initTrackedWorkspace(target, options.providers[0], options.targetBranch)
    }
  } else {
    await initGitWorktree(workspace)
    await mkdir(siblingWorkspace, { recursive: true })
    await writeFile(path.join(siblingWorkspace, 'README.md'), 'sibling repo\n', 'utf8')
    await initGitWorktree(siblingWorkspace)
    for (const target of targetWorkspaces) {
      await initManagedTargetWorkspace(target, options.providers)
    }
  }
  if (options.afterFixtureCommand) {
    await runAfterFixtureCommand(options.afterFixtureCommand, {
      rootDir,
      workspace,
      siblingWorkspace,
      targetWorkspace,
      targetWorkspaces,
      mode: options.mode,
    })
  }

  const { LocalIpcClient, requests } = await loadCliModules(runtimeDir, cliRoot)
  const {
    attachToSessionRequest,
    attachWorkspaceLinkRequest,
    createWorkspaceLinkRequest,
    createSessionRequest,
    destroyAgentRequest,
    endSessionRequest,
    getWorkspaceLiveSyncStatusRequest,
    getSessionStateRequest,
    listProviderProcessesRequest,
    respondToInteractionRequest,
    setWorkspaceLiveSyncModeRequest,
    spawnAgentRequest,
    submitPromptRequest,
  } = requests

  let daemonChild = null
  let daemonProfile = null
  let kernelUrl = options.kernel
  const startedAt = Date.now()
  const historyDir = options.historyDir ?? path.join(rootDir, 'history')
  let succeeded = false
  if (options.spawnDaemon) {
    const ports = makePorts()
    kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
    const daemonBinary = await resolveBinary(
      path.join(repoRoot, 'apps/kernel/target/debug/chariox-kernel'),
      path.join(repoRoot, 'apps/kernel/Cargo.toml'),
      'chariox-kernel',
    )
    const daemonId = `workspace-live-sync-drill-${process.pid}-${Date.now()}`
    daemonProfile = await prepareWorkspaceLiveSyncDaemonEnvironment({
      rootDir,
      daemonId,
      providers: options.providers,
    })
    daemonChild = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ...daemonProfile.env,
        CHARIOX_KERNEL_PORT: String(ports.kernelPort),
        CHARIOX_MCP_PORT: String(ports.mcpPort),
        CHARIOX_OPENCODE_PORT: String(ports.opencodePort),
        CHARIOX_CODEX_PORT: String(ports.codexPort),
        CHARIOX_DAEMON_ID: daemonId,
        CHARIOX_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
        CHARIOX_SESSION_HISTORY_DIR: historyDir,
      },
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    await waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace, workspace)
  }

  const client = new LocalIpcClient(kernelUrl)
  wrapClientSendWithTimeout(client, options.timeoutMs)
  const events = []
  let workerKernelRef = null
  let sessionId = null
  let failure = null
  try {
    workerKernelRef = await resolveRemoteWorkerKernelRef(
      client,
      requests,
      options.machineRef,
      options.providers,
      options.timeoutMs,
      options.pollMs,
    )
    const session = unwrap(await client.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
    sessionId = session.id
    if (setWorkspaceLiveSyncModeRequest && options.mode !== 'off') {
      await client.send(setWorkspaceLiveSyncModeRequest(session.id, options.mode))
    }
    const attachment = unwrap(
      await client.send(attachToSessionRequest(session.id, `workspace-live-sync-drill-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
    await client.subscribeToKernelEvents(session.id, attachment.id)

    if (options.mode === 'tracked' || targetWorkspaces.length > 0) {
      const linkName = `${options.mode}-live-sync-${Date.now()}`
      await client.send(createWorkspaceLinkRequest(session.id, linkName))
      await client.send(attachWorkspaceLinkRequest(session.id, linkName, workspace))
      for (const target of targetWorkspaces) {
        await client.send(attachWorkspaceLinkRequest(session.id, linkName, target))
      }
    }

    const agents = await spawnWorkspaceLiveSyncPhaseAgents({
      client,
      sessionId: session.id,
      providers: options.providers,
      modelForProvider: (provider) => modelForProvider(provider, options),
      workspace,
      kernelRef: workerKernelRef,
      spawnAgentRequest,
      aliasSuffix: 'positive',
    })
    const targetOriginAgents = options.mode === 'tracked' && options.trackedBidirectional
      ? await spawnWorkspaceLiveSyncPhaseAgents({
          client,
          sessionId: session.id,
          providers: options.providers,
          modelForProvider: (provider) => modelForProvider(provider, options),
          workspace: targetWorkspace,
          kernelRef: options.remoteSourceSideEffects ? null : workerKernelRef,
          spawnAgentRequest,
          aliasSuffix: 'target-origin',
        })
      : []

    if (options.mode === 'off') {
      const completionSinceMs = Date.now()
      const requiredFiles = []
      const directFiles = []
      for (const { provider, agent } of agents) {
        const directFile = path.join(outputsDir, `${provider}-direct.txt`)
        const siblingFile = path.join(siblingWorkspace, `${provider}-sibling.txt`)
        requiredFiles.push(directFile, siblingFile)
        directFiles.push({ provider, directFile, siblingFile })
        await client.send(submitPromptRequest(
          session.id,
          attachment.id,
          agent.id,
          [
            'This is a Workspace Live Sync off-mode drill.',
            'Use only direct filesystem writes through shell/native file tools. Do not use any Chariox workspace live sync MCP/runtime tools.',
            `Create outputs/${provider}-direct.txt in the current repository with exactly "${provider}-off-selected-root\\n".`,
            `Create ../sibling-repo/${provider}-sibling.txt with exactly "${provider}-off-sibling-root\\n".`,
            `After both direct writes complete, reply exactly ${provider.toUpperCase()}_OFF_DIRECT_WRITES_DONE and nothing else.`,
          ].join(' '),
          [],
        ))
      }
      await waitForCompletionsAndFiles({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        events,
        expectedCompletionCount: agents.length,
        completionSinceMs,
        requiredFiles,
        forbiddenFiles: [],
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        historyDir,
      })
      const status = unwrapVariant(await client.send(getWorkspaceLiveSyncStatusRequest(session.id)), 'WorkspaceLiveSyncStatus').status
      const finalState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
      const processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []
      console.log(JSON.stringify({
        status: 'ok',
        mode: 'off-workspace-live-sync-live-drill',
        kernelUrl,
        machineRef: options.machineRef,
        workspace,
        siblingWorkspace,
        providers: options.providers,
        model: options.model,
        providerModels: Object.fromEntries(options.providers.map((provider) => [
          provider,
          modelForProvider(provider, options),
        ])),
        durationMs: Date.now() - startedAt,
        agents: agents.map(({ provider, agent }) => ({
          id: agent.id,
          alias: agent.alias,
          provider,
        })),
        files: await Promise.all(directFiles.map(async ({ provider, directFile, siblingFile }) => ({
          provider,
          directContent: await readFile(directFile, 'utf8'),
          siblingContent: await readFile(siblingFile, 'utf8'),
        }))),
        workspaceLiveSyncStatus: status,
        completionCount: events.filter((event) => event.event === 'assistant_message_completed').length,
        terminalEventCount: events.filter((event) => event.event === 'terminal_output').length,
        providerProcesses: processes.map((process) => ({
          processId: process.process_id,
          provider: process.provider,
          pid: process.pid ?? null,
          ownerRunIds: process.owner_provider_run_ids || [],
        })),
        focusedAgentId: finalState.session?.focused_agent_id ?? finalState.focused_agent_id ?? null,
      }, null, 2))
      succeeded = true
      return
    }

    if (options.mode === 'tracked') {
      await runTrackedWorkspaceLiveSyncDrill({
        client,
        session,
        attachment,
        events,
        provider: options.providers[0],
        agent: agents[0].agent,
        targetOriginAgent: targetOriginAgents[0],
        spawnedSessionId: agents[0].spawnedSessionId,
        workspace,
        siblingWorkspace,
        targetWorkspace,
        targetWorkspaces,
        historyDir,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        machineRef: options.machineRef,
        getSessionStateRequest,
        getWorkspaceLiveSyncStatusRequest,
        listProviderProcessesRequest,
        respondToInteractionRequest,
        submitPromptRequest,
        startedAt,
        kernelUrl,
        options,
      })
      succeeded = true
      return
    }
    const debugSessionSnapshot = async () => {
      const state = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
      const currentSession = state.session ?? state
      const promptStates = currentSession.prompt_states ?? {}
      return {
        events: events.reduce((counts, event) => {
          counts[event.event] = (counts[event.event] ?? 0) + 1
          return counts
        }, {}),
        lastTerminalOutput: events
          .filter((event) => event.event === 'terminal_output')
          .slice(-3)
          .map((event) => String(event.text ?? event.data ?? event.output ?? '').slice(0, 500)),
        agents: (currentSession.agents ?? []).map((agent) => ({
          id: agent.id,
          alias: agent.alias,
          state: agent.state,
          is_processing: agent.is_processing,
          provider_run_id: agent.provider_run_id ?? null,
          prompt: {
            active: promptStates[agent.id]?.active_prompt != null,
            queued: (promptStates[agent.id]?.queued_prompts ?? []).length,
          },
        })),
      }
    }

    const positiveFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}.txt`))
    const movedFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}-moved.txt`))
    const opaqueMovedFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}-opaque-moved.bin`))
    const directFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}-direct.txt`))
    const runPositivePromptPhase = async ({ provider, agent, prompt, requiredFiles, label, marker, managedToolExpectations = [] }) => {
      const completionSinceMs = Date.now()
      await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
      if (managedToolExpectations.length > 0) {
        await waitForManagedToolExpectationsAndFiles({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          historyDir,
          sinceMs: completionSinceMs,
          expectations: managedToolExpectations,
          requiredFiles,
          forbiddenFiles: directFiles,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
          debugSnapshot: debugSessionSnapshot,
          historyDir,
          providerErrorSinceMs: completionSinceMs,
        })
      } else {
        await waitForPromptPhase({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          events,
          expectedCompletionCount: 1,
          completionSinceMs,
          requiredFiles,
          forbiddenFiles: directFiles,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
          debugSnapshot: debugSessionSnapshot,
          historyDir,
          providerErrorSinceMs: completionSinceMs,
        })
        await waitForHistoryOutputMarkers({
          historyDir,
          providerHistoryDirs: options.providerHistoryDirs,
          markerGroups: [[marker]],
          sinceMs: completionSinceMs,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
      }
      if (!options.machineRef) {
        await waitForAgentsIdle({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          agentIds: [agent.id],
          getSessionStateRequest,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
        await sleep(4_000)
      }
      await assertFilesAbsent(directFiles, `${provider} ${label} direct-write check`)
    }

    for (const { provider, agent } of agents) {
      const written = `${provider}-workspace-live-sync-write-ok: seed-value-42`
      const edited = `${provider}-workspace-live-sync-edit-ok: seed-value-42`
      const sourceName = workspaceLiveSyncMoveSourceName(provider)
      const patchInitial = provider === 'opencode' ? `source-start-${provider}\n` : `patch-start-${provider}\n`
      const patchMoved = provider === 'opencode' ? patchInitial : `patch-moved-${provider}\n`
      const tools = workspaceLiveSyncToolNames(provider)
      const patchText = [
        '*** Begin Patch',
        `*** Add File: outputs/${sourceName}`,
        `+${patchInitial.trimEnd()}`,
        '*** End Patch',
      ].join('\n')
      const opaqueBytes = Buffer.from([0, provider.length, 255, 10])
      const opaqueBase64 = opaqueBytes.toString('base64')
      await runPositivePromptPhase({
        provider,
        agent,
        label: 'text read/write',
        marker: `${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_TEXT_WRITE_DONE`,
        managedToolExpectations: [
          { toolSuffix: 'write_artifact', path: `outputs/${provider}.txt` },
        ],
        requiredFiles: [path.join(outputsDir, `${provider}.txt`)],
        prompt: [
          'This is a live Chariox workspace live sync positive text read/write smoke test.',
          'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Chariox file write path.',
          'Use only the Chariox MCP/runtime tools for file I/O.',
          `Step 1: call \`${tools.read}\` exactly once with JSON arguments {"path":"seed.txt","domain":"text"}.`,
          `Step 2: call \`${tools.write}\` exactly once with JSON arguments {"path":"outputs/${provider}.txt","content_text":${JSON.stringify(written)},"domain":"text"}.`,
          'The content_text value must end at seed-value-42; do not append a newline or a literal backslash-n sequence.',
          `Only after both steps succeed and outputs/${provider}.txt exists, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_TEXT_WRITE_DONE and nothing else.`,
          `If any workspace live sync tool reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop.`,
        ].join('\n'),
      })
      await assertFileContent(path.join(outputsDir, `${provider}.txt`), written)

      await runPositivePromptPhase({
        provider,
        agent,
        label: 'text read/edit',
        marker: `${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_TEXT_EDIT_DONE`,
        managedToolExpectations: [
          { toolSuffix: 'edit_artifact', path: `outputs/${provider}.txt` },
        ],
        requiredFiles: [path.join(outputsDir, `${provider}.txt`)],
        prompt: [
          'This is a live Chariox workspace live sync positive text edit smoke test.',
          'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Chariox file write path.',
          'Use only the Chariox MCP/runtime tools for file I/O.',
          `Step 1: call \`${tools.read}\` exactly once with JSON arguments {"path":"outputs/${provider}.txt","domain":"text"}. Use the snapshot_id returned by this call for outputs/${provider}.txt.`,
          `Step 2: call \`${tools.edit}\` exactly once with JSON arguments {"path":"outputs/${provider}.txt","old_text":${JSON.stringify(written)},"new_text":${JSON.stringify(edited)},"domain":"text","snapshot_id":"THE_OUTPUT_SNAPSHOT_ID_FROM_STEP_1"}. Replace THE_OUTPUT_SNAPSHOT_ID_FROM_STEP_1 with the exact snapshot_id returned in step 1. Do not reuse the seed.txt snapshot or any snapshot from an earlier turn.`,
          `Only after the edit succeeds and outputs/${provider}.txt contains the new text, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_TEXT_EDIT_DONE and nothing else.`,
          `If any workspace live sync tool reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop.`,
        ].join('\n'),
      })
      await assertFileContent(path.join(outputsDir, `${provider}.txt`), edited)

      const patchPrompt = [
          'This is a live Chariox workspace live sync positive text patch smoke test.',
          'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Chariox file write path.',
          'Use only the Chariox MCP/runtime tools for file I/O.',
          `Call \`${tools.applyPatch}\` exactly once with JSON arguments {"patch_text":${JSON.stringify(patchText)},"domain":"text"}.`,
          `Only after outputs/${sourceName} exists, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_PATCH_DONE and nothing else.`,
          `If the workspace live sync tool reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop.`,
        ].join('\n')
      if (provider === 'opencode') {
        await writeFile(path.join(outputsDir, sourceName), patchInitial, 'utf8')
        for (const targetWorkspace of targetWorkspaces) {
          await writeFile(path.join(targetWorkspace, 'outputs', sourceName), patchInitial, 'utf8')
        }
      } else {
        await runPositivePromptPhase({
          provider,
          agent,
          label: 'text patch',
          marker: `${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_PATCH_DONE`,
          managedToolExpectations: [
            { toolSuffix: 'patch_artifact' },
          ],
          requiredFiles: [path.join(outputsDir, sourceName)],
          prompt: patchPrompt,
        })
      }
      await assertFileContent(path.join(outputsDir, sourceName), patchInitial)

      await runPositivePromptPhase({
        provider,
        agent,
        label: 'text move',
        marker: `${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_MOVE_DONE`,
        managedToolExpectations: [
          {
            toolSuffix: 'move_artifact',
            fromPath: `outputs/${sourceName}`,
            toPath: `outputs/${provider}-moved.txt`,
          },
        ],
        requiredFiles: [path.join(outputsDir, `${provider}-moved.txt`)],
        prompt: [
          'This is a live Chariox workspace live sync positive move smoke test.',
          'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Chariox file write path.',
          'Use only the Chariox MCP/runtime tools for file I/O.',
          provider === 'opencode'
            ? `Call \`${tools.move}\` exactly once with JSON arguments {"from_path":"outputs/${sourceName}","to_path":"outputs/${provider}-moved.txt","domain":"text"}.`
            : `Call \`${tools.move}\` exactly once with JSON arguments {"from_path":"outputs/${sourceName}","to_path":"outputs/${provider}-moved.txt","old_text":${JSON.stringify(patchInitial)},"new_text":${JSON.stringify(patchMoved)},"domain":"text"}.`,
          `Only after outputs/${provider}-moved.txt exists and outputs/${sourceName} is gone, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_MOVE_DONE and nothing else.`,
          `If the workspace live sync tool reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop.`,
        ].join('\n'),
      })
      await assertFileContent(path.join(outputsDir, `${provider}-moved.txt`), patchMoved)
      if (await fileExists(path.join(outputsDir, sourceName))) {
        throw new Error(`managed move left source file behind: outputs/${sourceName}`)
      }

      await runPositivePromptPhase({
        provider,
        agent,
        label: 'opaque write',
        marker: `${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_OPAQUE_WRITE_DONE`,
        managedToolExpectations: [
          { toolSuffix: 'write_artifact', path: `outputs/${provider}-opaque.bin` },
        ],
        requiredFiles: [path.join(outputsDir, `${provider}-opaque.bin`)],
        prompt: [
          'This is a live Chariox workspace live sync positive opaque write smoke test.',
          'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Chariox file write path.',
          'Use only the Chariox MCP/runtime tools for file I/O.',
          'Every tool call in this turn must use `"domain":"opaque"`.',
          `Call \`${tools.write}\` exactly once with JSON arguments {"path":"outputs/${provider}-opaque.bin","content_base64":${JSON.stringify(opaqueBase64)},"domain":"opaque"}.`,
          `Only after outputs/${provider}-opaque.bin exists, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_OPAQUE_WRITE_DONE and nothing else.`,
          `If the workspace live sync tool reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop.`,
        ].join('\n'),
      })
      await assertFileBytes(path.join(outputsDir, `${provider}-opaque.bin`), opaqueBytes)

      await runPositivePromptPhase({
        provider,
        agent,
        label: 'opaque read/move',
        marker: `${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_OPAQUE_MOVE_DONE`,
        managedToolExpectations: [
          {
            toolSuffix: 'move_artifact',
            fromPath: `outputs/${provider}-opaque.bin`,
            toPath: `outputs/${provider}-opaque-moved.bin`,
          },
        ],
        requiredFiles: [path.join(outputsDir, `${provider}-opaque-moved.bin`)],
        prompt: [
          'This is a live Chariox workspace live sync positive opaque read/move smoke test.',
          'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Chariox file write path.',
          'Use only the Chariox MCP/runtime tools for file I/O.',
          'Every tool call in this turn must use `"domain":"opaque"`.',
          'The opaque move call does not need text content. If your tool schema requires old_text or new_text fields, leave them empty.',
          `Step 1: call \`${tools.read}\` exactly once with JSON arguments {"path":"outputs/${provider}-opaque.bin","domain":"opaque"} and verify the returned content_base64 is ${JSON.stringify(opaqueBase64)}.`,
          `Step 2: call \`${tools.move}\` exactly once with JSON arguments {"from_path":"outputs/${provider}-opaque.bin","to_path":"outputs/${provider}-opaque-moved.bin","domain":"opaque"}.`,
          `Only after outputs/${provider}-opaque-moved.bin exists and outputs/${provider}-opaque.bin is gone, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_OPAQUE_MOVE_DONE and nothing else.`,
          `If any workspace live sync tool reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop.`,
        ].join('\n'),
      })
      await assertFileBytes(path.join(outputsDir, `${provider}-opaque-moved.bin`), opaqueBytes)
      if (await fileExists(path.join(outputsDir, `${provider}-opaque.bin`))) {
        throw new Error(`managed opaque move left source file behind: outputs/${provider}-opaque.bin`)
      }
    }
    if (!options.machineRef) {
      await waitForAgentsIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentIds: agents.map(({ agent }) => agent.id),
        getSessionStateRequest,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
    }
    for (const provider of options.providers) {
      await assertFileContent(
        path.join(outputsDir, `${provider}.txt`),
        `${provider}-workspace-live-sync-edit-ok: seed-value-42`,
      )
      await assertFileContent(
        path.join(outputsDir, `${provider}-moved.txt`),
        provider === 'opencode' ? `source-start-${provider}\n` : `patch-moved-${provider}\n`,
      )
      await assertFileBytes(path.join(outputsDir, `${provider}-opaque-moved.bin`), [0, provider.length, 255, 10])
      const sourceName = workspaceLiveSyncMoveSourceName(provider)
      if (await fileExists(path.join(outputsDir, sourceName))) {
        throw new Error(`managed move left source file behind: outputs/${sourceName}`)
      }
      if (await fileExists(path.join(outputsDir, `${provider}-opaque.bin`))) {
        throw new Error(`managed opaque move left source file behind: outputs/${provider}-opaque.bin`)
      }
    }
    await assertManagedTargetFanout(targetWorkspaces, options.providers, { deletesApplied: false })
    if (targetWorkspaces.length > 0) {
      await waitForHistoryNotices({
        historyDir,
        sinceMs: startedAt,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        requirements: options.providers.map((provider) => ({
          label: `${provider} managed fanout summary`,
          includes: [
            'Workspace live sync managed summary',
            `outputs/${provider}.txt`,
            'target user',
            'Next action:',
          ],
        })),
      })
    }

    if (options.positiveOnly) {
      const files = []
      for (const provider of options.providers) {
        const filePath = path.join(outputsDir, `${provider}.txt`)
        files.push({
          provider,
          relativePath: `outputs/${provider}.txt`,
          content: await readFile(filePath, 'utf8'),
          movedRelativePath: `outputs/${provider}-moved.txt`,
          movedContent: await readFile(path.join(outputsDir, `${provider}-moved.txt`), 'utf8'),
          opaqueMovedRelativePath: `outputs/${provider}-opaque-moved.bin`,
          opaqueMovedHex: (await readFile(path.join(outputsDir, `${provider}-opaque-moved.bin`))).toString('hex'),
          patchSourceFileExists: await fileExists(path.join(outputsDir, workspaceLiveSyncMoveSourceName(provider))),
          opaqueMoveSourceFileExists: await fileExists(path.join(outputsDir, `${provider}-opaque.bin`)),
          deletedFileExists: await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`)),
          opaqueDeletedFileExists: await fileExists(path.join(outputsDir, `${provider}-opaque-delete-me.bin`)),
          directWriteFileExists: await fileExists(path.join(outputsDir, `${provider}-direct.txt`)),
        })
      }
      const finalState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
      const processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []
      console.log(JSON.stringify({
        status: 'ok',
        mode: 'workspace-live-sync-live-drill',
        kernelUrl,
        machineRef: options.machineRef,
        workspace,
        providers: options.providers,
        model: options.model,
        providerModels: Object.fromEntries(options.providers.map((provider) => [
          provider,
          modelForProvider(provider, options),
        ])),
        durationMs: Date.now() - startedAt,
        agents: agents.map(({ provider, agent }) => ({
          id: agent.id,
          alias: agent.alias,
          provider,
        })),
        managedTargets: await managedTargetFanoutSnapshot(targetWorkspaces, options.providers),
        completionCount: events.filter((event) => event.event === 'assistant_message_completed').length,
        terminalEventCount: events.filter((event) => event.event === 'terminal_output').length,
        files,
        collisionAndExternalChecks: [],
        providerProcesses: processes.map((process) => ({
          processId: process.process_id,
          provider: process.provider,
          pid: process.pid ?? null,
          ownerRunIds: process.owner_provider_run_ids || [],
        })),
        focusedAgentId: finalState.session?.focused_agent_id ?? finalState.focused_agent_id ?? null,
      }, null, 2))
      succeeded = true
      return
    }

    const deleteAgents = []
    if (options.machineRef) {
      for (const { provider } of agents) {
        const deleteAgent = unwrapVariant(
          await client.send(workspaceLiveSyncSpawnAgentRequest(
            spawnAgentRequest,
            session.id,
            provider,
            `${provider}-workspace-live-sync-delete`,
            modelForProvider(provider, options),
            workspace,
            'low',
            workerKernelRef,
          )),
          'AgentSpawned',
        ).agent
        deleteAgents.push({ provider, agent: deleteAgent })
      }
    } else {
      deleteAgents.push(...agents)
    }

    const deletePrompts = []
    for (const { provider, agent } of deleteAgents) {
      const tools = workspaceLiveSyncToolNames(provider)
      const prompt = [
        'This is a live Chariox workspace live sync delete smoke test.',
        'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Chariox file write path.',
        `Call \`${tools.delete}\` with JSON arguments {"path":"outputs/${provider}-delete-me.txt","domain":"text"} to delete the pre-existing delete-me file.`,
        `Then call \`${tools.delete}\` with JSON arguments {"path":"outputs/${provider}-opaque-delete-me.bin","domain":"opaque"} to delete the pre-existing opaque delete-me file.`,
        `After the tool succeeds, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_DELETE_DONE and nothing else.`,
        `If any delete reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop. Do not recreate, write, or repair deleted files.`,
      ].join('\n')
      deletePrompts.push({ provider, agent, prompt })
    }
    if (options.machineRef) {
      for (const { provider, agent, prompt } of deletePrompts) {
        const completionSinceMs = Date.now()
        await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
        await waitForManagedToolExpectationsAndFiles({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          historyDir,
          sinceMs: completionSinceMs,
          expectations: [
            { toolSuffix: 'delete_artifact', path: `outputs/${provider}-delete-me.txt` },
            { toolSuffix: 'delete_artifact', path: `outputs/${provider}-opaque-delete-me.bin` },
          ],
          requiredFiles: [],
          forbiddenFiles: directFiles,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
          debugSnapshot: debugSessionSnapshot,
        })
        await waitForFilesAbsent({
          filePaths: [
            path.join(outputsDir, `${provider}-delete-me.txt`),
            path.join(outputsDir, `${provider}-opaque-delete-me.bin`),
          ],
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
        await waitForHistoryOutputMarkers({
          historyDir,
          providerHistoryDirs: options.providerHistoryDirs,
          markerGroups: [[`${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_DELETE_DONE`]],
          sinceMs: completionSinceMs,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
        })
      }
    } else {
      const completionSinceMs = Date.now()
      for (const { agent, prompt } of deletePrompts) {
        await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
      }
      for (const { provider } of deletePrompts) {
        await waitForManagedToolExpectationsAndFiles({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          historyDir,
          sinceMs: completionSinceMs,
          expectations: [
            { toolSuffix: 'delete_artifact', path: `outputs/${provider}-delete-me.txt` },
            { toolSuffix: 'delete_artifact', path: `outputs/${provider}-opaque-delete-me.bin` },
          ],
          requiredFiles: [],
          forbiddenFiles: directFiles,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
          debugSnapshot: debugSessionSnapshot,
        })
      }
    }
    if (!options.machineRef) {
      await waitForAgentsIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentIds: deleteAgents.map(({ agent }) => agent.id),
        getSessionStateRequest,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
    }
    for (const provider of options.providers) {
      if (await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`))) {
        throw new Error(`managed delete left file behind: outputs/${provider}-delete-me.txt`)
      }
      if (await fileExists(path.join(outputsDir, `${provider}-opaque-delete-me.bin`))) {
        throw new Error(`managed opaque delete left file behind: outputs/${provider}-opaque-delete-me.bin`)
      }
    }

    const negativeAgents = options.machineRef ? deleteAgents : agents

    const negativePrompts = []
    for (const { provider, agent } of negativeAgents) {
      const tools = workspaceLiveSyncToolNames(provider)
      const prompt = [
        'This is a live Chariox workspace live sync negative smoke test.',
        `Try to create outputs/${provider}-direct.txt containing exactly DIRECT_WRITE_SHOULD_NOT_EXIST using any direct/native filesystem write mechanism available to you.`,
        `Do not use \`${tools.write}\`, \`${tools.edit}\`, or any other Chariox workspace live sync tool for this negative test.`,
        `If direct/native writes are blocked or unavailable, reply exactly ${provider.toUpperCase()}_DIRECT_WRITE_BLOCKED and nothing else.`,
      ].join('\n')
      negativePrompts.push({ provider, agent, prompt })
    }

    if (options.machineRef) {
      for (const { provider, agent, prompt } of negativePrompts) {
        const completionSinceMs = Date.now()
        await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
        await waitForHistoryOutputMarkers({
          historyDir,
          providerHistoryDirs: options.providerHistoryDirs,
          markerGroups: [[`${provider.toUpperCase()}_DIRECT_WRITE_BLOCKED`]],
          sinceMs: completionSinceMs,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
        })
        await waitForFilesAbsent({
          filePaths: directFiles,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
      }
    } else {
      const completionSinceMs = Date.now()
      for (const { agent, prompt } of negativePrompts) {
        await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
      }
      await waitForCompletionsAndFiles({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        events,
        expectedCompletionCount: negativeAgents.length,
        completionSinceMs,
        requiredFiles: positiveFiles,
        forbiddenFiles: directFiles,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        historyDir,
        providerErrorSinceMs: completionSinceMs,
      })
      await waitForHistoryOutputMarkers({
        historyDir,
        providerHistoryDirs: options.providerHistoryDirs,
        markerGroups: negativeAgents.map(({ provider }) => [`${provider.toUpperCase()}_DIRECT_WRITE_BLOCKED`]),
        sinceMs: completionSinceMs,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
    }
    if (!options.machineRef) {
      await waitForAgentsIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentIds: negativeAgents.map(({ agent }) => agent.id),
        getSessionStateRequest,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
    }
    await assertFilesAbsent(directFiles, 'negative workspace live sync direct-write check')

    const collisionAgents = options.machineRef ? await spawnWorkspaceLiveSyncPhaseAgents({
      client,
      sessionId: session.id,
      providers: options.providers,
      modelForProvider: (provider) => modelForProvider(provider, options),
      workspace,
      kernelRef: workerKernelRef,
      spawnAgentRequest,
      aliasSuffix: 'collision',
    }) : agents
    const collisionAndExternalChecks = await runLiveCollisionAndExternalChecks({
      client,
      session,
      attachment,
      events,
      agents: collisionAgents,
      modelForProvider: (provider) => modelForProvider(provider, options),
      machineRef: options.machineRef,
      kernelRef: workerKernelRef,
      workspace,
      outputsDir,
      historyDir,
      providerHistoryDirs: options.providerHistoryDirs,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      getSessionStateRequest,
      spawnAgentRequest,
      destroyAgentRequest,
      submitPromptRequest,
    })
    await assertFilesAbsent(directFiles, 'final workspace live sync direct-write check')
    await assertManagedTargetFanout(targetWorkspaces, options.providers, { deletesApplied: true })

    const files = []
    for (const provider of options.providers) {
      const filePath = path.join(outputsDir, `${provider}.txt`)
      files.push({
        provider,
        relativePath: `outputs/${provider}.txt`,
        content: await readFile(filePath, 'utf8'),
        movedRelativePath: `outputs/${provider}-moved.txt`,
        movedContent: await readFile(path.join(outputsDir, `${provider}-moved.txt`), 'utf8'),
        opaqueMovedRelativePath: `outputs/${provider}-opaque-moved.bin`,
        opaqueMovedHex: (await readFile(path.join(outputsDir, `${provider}-opaque-moved.bin`))).toString('hex'),
        patchSourceFileExists: await fileExists(path.join(outputsDir, workspaceLiveSyncMoveSourceName(provider))),
        opaqueMoveSourceFileExists: await fileExists(path.join(outputsDir, `${provider}-opaque.bin`)),
        deletedFileExists: await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`)),
        opaqueDeletedFileExists: await fileExists(path.join(outputsDir, `${provider}-opaque-delete-me.bin`)),
        directWriteFileExists: await fileExists(path.join(outputsDir, `${provider}-direct.txt`)),
      })
    }
    const finalState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
    const processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'workspace-live-sync-live-drill',
      kernelUrl,
      machineRef: options.machineRef,
      workspace,
      managedTargets: await managedTargetFanoutSnapshot(targetWorkspaces, options.providers),
      providers: options.providers,
      model: options.model,
      providerModels: Object.fromEntries(options.providers.map((provider) => [
        provider,
        modelForProvider(provider, options),
      ])),
      durationMs: Date.now() - startedAt,
      agents: agents.map(({ provider, agent }) => ({
        id: agent.id,
        alias: agent.alias,
        provider,
      })),
      completionCount: events.filter((event) => event.event === 'assistant_message_completed').length,
      terminalEventCount: events.filter((event) => event.event === 'terminal_output').length,
      files,
      collisionAndExternalChecks,
      providerProcesses: processes.map((process) => ({
        processId: process.process_id,
        provider: process.provider,
        pid: process.pid ?? null,
        ownerRunIds: process.owner_provider_run_ids || [],
      })),
      focusedAgentId: finalState.session?.focused_agent_id ?? finalState.focused_agent_id ?? null,
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (sessionId) {
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client.close().catch(() => {})
    await terminateChild(daemonChild)
    await removeWorkspaceLiveSyncProviderProfile(daemonProfile).catch(() => {})
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'workspace-live-sync',
        mode: options.mode,
        providers: options.providers.join(','),
        managedTargetCount: options.managedTargetCount,
        trackedTargetCount: options.trackedTargetCount,
        trackedBidirectional: options.trackedBidirectional,
        machineRef: options.machineRef ?? 'local',
        runtimeDir,
      },
      log: (name, details) => console.log(`[workspace-live-sync-drill] ${name}`, JSON.stringify(details)),
    })
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`workspace live sync drill transient CLI modules kept at ${runtimeDir}`)
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
