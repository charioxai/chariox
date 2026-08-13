import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { assertFileContent, destroyWorkspaceLiveSyncAgent, fileExists, gitHead, modelForProvider, resetTrackedWorkspace, sleep, unwrapVariant, workspaceLiveSyncSpawnAgentRequest, workspaceLiveSyncToolNames } from './workspace-live-sync-drill-runtime.mjs'
import { waitForAgentsIdle, waitForCompletionsAndFiles, waitForHistoryNotices, waitForHistoryOutputMarkers, waitForManagedEditResult, waitForManagedEditResults, waitForManagedReadSnapshot, waitForManagedReadSnapshots } from './workspace-live-sync-drill-waiters.mjs'

export async function waitForTrackedFanout({
  client,
  sessionId,
  attachmentId,
  getWorkspaceLiveSyncStatusRequest,
  sourceWorkspace,
  targetWorkspaces,
  provider,
  remoteSourceSideEffects,
  timeoutMs,
  pollMs,
}) {
  const sourceOutputs = path.join(sourceWorkspace, 'outputs')
  const expectedTracked = `line-a\n${provider}-tracked-modified\n`
  const expectedAdded = `${provider}-tracked-added\n`
  const expectedRenamed = `${provider}-tracked-renamed\n`
  const expectedSourceRebase = `alpha\nbeta\n${provider}-tracked-source\nomega\n`
  const expectedTargetRebase = `alpha\n${provider}-tracked-target-local\nbeta\n${provider}-tracked-source\nomega\n`
  const expectedSourceConflict = `one\n${provider}-tracked-source-conflict\nthree\n`
  const expectedTargetConflict = `one\n${provider}-tracked-target-conflict\nthree\n`
  let lastStatus = null
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    lastStatus = unwrapVariant(
      await client.send(getWorkspaceLiveSyncStatusRequest(sessionId)),
      'WorkspaceLiveSyncStatus',
    ).status
    const checks = [
      [path.join(sourceWorkspace, 'tracked.txt'), expectedTracked],
      [path.join(sourceOutputs, `${provider}-tracked-added.txt`), expectedAdded],
      [path.join(sourceOutputs, `${provider}-tracked-renamed.txt`), expectedRenamed],
      [path.join(sourceOutputs, `${provider}-tracked-rebase.txt`), expectedSourceRebase],
      [path.join(sourceOutputs, `${provider}-tracked-conflict.txt`), expectedSourceConflict],
      [path.join(sourceWorkspace, '.charioxignore'), 'ignored/\n*.secret\n'],
    ]
    for (const targetWorkspace of targetWorkspaces) {
      const targetOutputs = path.join(targetWorkspace, 'outputs')
      checks.push(
        [path.join(targetWorkspace, 'tracked.txt'), expectedTracked],
        [path.join(targetOutputs, `${provider}-tracked-added.txt`), expectedAdded],
        [path.join(targetOutputs, `${provider}-tracked-renamed.txt`), expectedRenamed],
        [path.join(targetOutputs, `${provider}-tracked-rebase.txt`), expectedTargetRebase],
        [path.join(targetOutputs, `${provider}-tracked-conflict.txt`), expectedTargetConflict],
        [path.join(targetWorkspace, '.charioxignore'), 'ignored/\n*.secret\n'],
      )
    }
    let contentOk = true
    for (const [filePath, expected] of checks) {
      if (!(await fileExists(filePath)) || (await readFile(filePath, 'utf8')) !== expected) {
        contentOk = false
        break
      }
    }
    if (contentOk) {
      const sourceBinaryPath = path.join(sourceOutputs, `${provider}-tracked-binary.bin`)
      if (!(await fileExists(sourceBinaryPath))) {
        contentOk = false
      } else {
        const sourceBinary = await readFile(sourceBinaryPath)
        contentOk = sourceBinary.length > 0
        for (const targetWorkspace of targetWorkspaces) {
          const targetBinaryPath = path.join(targetWorkspace, 'outputs', `${provider}-tracked-binary.bin`)
          if (!(await fileExists(targetBinaryPath)) || !(await readFile(targetBinaryPath)).equals(sourceBinary)) {
            contentOk = false
            break
          }
        }
      }
    }
    let deletedOk = !(await fileExists(path.join(sourceOutputs, `${provider}-tracked-delete.txt`))) &&
      !(await fileExists(path.join(sourceOutputs, `${provider}-tracked-rename-source.txt`)))
    let ignoredOk = remoteSourceSideEffects
      ? !(await fileExists(path.join(sourceWorkspace, 'ignored', `${provider}-ignored.txt`)))
      : await fileExists(path.join(sourceWorkspace, 'ignored', `${provider}-ignored.txt`))
    let hasTargets = true
    let hasExpectedConflicts = true
    for (const targetWorkspace of targetWorkspaces) {
      const targetOutputs = path.join(targetWorkspace, 'outputs')
      deletedOk = deletedOk &&
        !(await fileExists(path.join(targetOutputs, `${provider}-tracked-delete.txt`))) &&
        !(await fileExists(path.join(targetOutputs, `${provider}-tracked-rename-source.txt`)))
      ignoredOk = ignoredOk && !(await fileExists(path.join(targetWorkspace, 'ignored', `${provider}-ignored.txt`)))
      hasTargets = hasTargets && (lastStatus.targets ?? []).some((target) => target.repo_root === targetWorkspace)
      hasExpectedConflicts = hasExpectedConflicts && (lastStatus.conflicts ?? []).some((conflict) => (
        conflict.target_repo_root === targetWorkspace &&
        conflict.path === `outputs/${provider}-tracked-conflict.txt` &&
        conflict.source_agent_id
      ))
    }
    if (contentOk && deletedOk && ignoredOk && hasTargets && hasExpectedConflicts) return lastStatus
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for tracked workspace live sync fanout; lastStatus=${JSON.stringify(lastStatus)}`)
}

export async function waitForTrackedTargetOriginFanout({
  client,
  sessionId,
  attachmentId,
  getWorkspaceLiveSyncStatusRequest,
  allWorkspaces,
  statusTargetWorkspaces,
  provider,
  timeoutMs,
  pollMs,
}) {
  const expectedText = `${provider}-target-origin-modified\n`
  const expectedAdded = `${provider}-target-origin-added\n`
  let lastStatus = null
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    lastStatus = unwrapVariant(
      await client.send(getWorkspaceLiveSyncStatusRequest(sessionId)),
      'WorkspaceLiveSyncStatus',
    ).status
    let contentOk = true
    for (const workspace of allWorkspaces) {
      const textPath = path.join(workspace, 'target-origin.txt')
      const addedPath = path.join(workspace, 'outputs', `${provider}-target-origin-added.txt`)
      if (!(await fileExists(textPath)) || (await readFile(textPath, 'utf8')) !== expectedText) {
        contentOk = false
        break
      }
      if (!(await fileExists(addedPath)) || (await readFile(addedPath, 'utf8')) !== expectedAdded) {
        contentOk = false
        break
      }
    }
    const hasTargets = statusTargetWorkspaces.every((workspace) =>
      (lastStatus.targets ?? []).some((target) => target.repo_root === workspace)
    )
    if (contentOk && hasTargets) return lastStatus
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for tracked target-origin fanout; lastStatus=${JSON.stringify(lastStatus)}`)
}

export async function waitForTrackedConflictFileFanout({
  client,
  sessionId,
  attachmentId,
  getWorkspaceLiveSyncStatusRequest,
  workspaces,
  targetWorkspaces,
  provider,
  expectedContent,
  expectConflictsCleared,
  timeoutMs,
  pollMs,
}) {
  let lastStatus = null
  const relativePath = `outputs/${provider}-tracked-conflict.txt`
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    lastStatus = unwrapVariant(
      await client.send(getWorkspaceLiveSyncStatusRequest(sessionId)),
      'WorkspaceLiveSyncStatus',
    ).status
    let contentOk = true
    for (const workspace of workspaces) {
      const filePath = path.join(workspace, relativePath)
      if (!(await fileExists(filePath)) || (await readFile(filePath, 'utf8')) !== expectedContent) {
        contentOk = false
        break
      }
    }
    const conflicts = lastStatus.conflicts ?? []
    const conflictsCleared = targetWorkspaces.every((targetWorkspace) => !conflicts.some((conflict) => (
      conflict.target_repo_root === targetWorkspace &&
      conflict.path === relativePath
    )))
    if (contentOk && (!expectConflictsCleared || conflictsCleared)) return lastStatus
    await sleep(pollMs)
  }
  const contents = {}
  for (const workspace of workspaces) {
    const filePath = path.join(workspace, relativePath)
    contents[workspace] = await fileExists(filePath) ? await readFile(filePath, 'utf8') : null
  }
  throw new Error(`timed out waiting for tracked conflict file fanout; contents=${JSON.stringify(contents)}; lastStatus=${JSON.stringify(lastStatus)}`)
}

export async function runTrackedTargetOriginPhase({
  client,
  session,
  attachment,
  events,
  provider,
  agent,
  workspace,
  targetWorkspaces,
  historyDir,
  providerHistoryDirs,
  timeoutMs,
  pollMs,
  getSessionStateRequest,
  getWorkspaceLiveSyncStatusRequest,
  submitPromptRequest,
}) {
  const allWorkspaces = [workspace, ...targetWorkspaces]
  const headsBefore = Object.fromEntries(await Promise.all(allWorkspaces.map(async (worktree) => [worktree, await gitHead(worktree)])))
  const completionSinceMs = Date.now()
  const marker = `${provider.toUpperCase()}_TRACKED_WORKSPACE_LIVE_SYNC_TARGET_ORIGIN_DONE`
  const targetOriginScript = [
    `printf '${provider}-target-origin-modified\\n' > target-origin.txt`,
    'mkdir -p outputs',
    `printf '${provider}-target-origin-added\\n' > outputs/${provider}-target-origin-added.txt`,
  ].join('\n')
  await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
    'This is a live Chariox workspace live sync tracked-mode target-origin drill.',
    'Use direct filesystem writes through shell/native file tools. Do not use any Chariox workspace live sync MCP/runtime tools.',
    'Run this exact POSIX shell script once from the current workspace directory:',
    targetOriginScript,
    'Do not inspect, read, list, or verify any files before or after running the script.',
    `After the script completes, reply exactly ${marker} and nothing else.`,
  ].join('\n'), []))

  await waitForCompletionsAndFiles({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    events,
    expectedCompletionCount: events.filter((event) => event.event === 'assistant_message_completed').length + 1,
    completionSinceMs,
    requiredFiles: [
      path.join(targetWorkspaces[0], 'outputs', `${provider}-target-origin-added.txt`),
    ],
    forbiddenFiles: [],
    timeoutMs,
    pollMs,
    historyDir,
    providerErrorSinceMs: completionSinceMs,
  })
  await waitForHistoryOutputMarkers({
    historyDir,
    providerHistoryDirs,
    markerGroups: [[marker]],
    sinceMs: completionSinceMs,
    timeoutMs,
    pollMs,
  })
  await waitForAgentsIdle({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentIds: [agent.id],
    getSessionStateRequest,
    timeoutMs,
    pollMs,
  })
  const status = await waitForTrackedTargetOriginFanout({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    getWorkspaceLiveSyncStatusRequest,
    allWorkspaces,
    statusTargetWorkspaces: targetWorkspaces,
    provider,
    timeoutMs,
    pollMs,
  })
  const headsAfter = Object.fromEntries(await Promise.all(allWorkspaces.map(async (worktree) => [worktree, await gitHead(worktree)])))
  const changedHead = allWorkspaces.find((worktree) => headsAfter[worktree] !== headsBefore[worktree])
  if (changedHead) {
    const headSummary = allWorkspaces.map((worktree) => `${worktree}: ${headsBefore[worktree]} -> ${headsAfter[worktree]}`).join('; ')
    throw new Error(`tracked target-origin fanout unexpectedly created commits; ${headSummary}`)
  }
  return {
    sourceWorkspace: targetWorkspaces[0],
    targetWorkspaces: allWorkspaces.filter((worktree) => worktree !== targetWorkspaces[0]),
    headsBefore,
    headsAfter,
    status,
  }
}

export async function runTrackedConflictResolutionPhase({
  client,
  session,
  attachment,
  events,
  provider,
  sourceAgent,
  resolverAgent,
  workspace,
  targetWorkspaces,
  historyDir,
  providerHistoryDirs,
  timeoutMs,
  pollMs,
  getSessionStateRequest,
  getWorkspaceLiveSyncStatusRequest,
  submitPromptRequest,
}) {
  const allWorkspaces = [workspace, ...targetWorkspaces]
  const relativePath = `outputs/${provider}-tracked-conflict.txt`
  const sourceSideContent = `one\n${provider}-tracked-source-conflict\nthree\n`
  const resolvedContent = `one\n${provider}-tracked-resolved\nthree\n`
  const targetConflictPaths = targetWorkspaces.map((targetWorkspace) =>
    path.join(targetWorkspace, relativePath)
  )

  const alignSinceMs = Date.now()
  const alignMarker = `${provider.toUpperCase()}_TRACKED_WORKSPACE_LIVE_SYNC_CONFLICT_ALIGNED`
  await client.send(submitPromptRequest(session.id, attachment.id, resolverAgent.id, [
    'This is a live Chariox workspace live sync tracked-mode conflict alignment drill.',
    'Use direct filesystem writes through shell/native file tools. Do not use any Chariox workspace live sync MCP/runtime tools.',
    `Run a direct write in the current workspace so that ${relativePath} becomes exactly "one\\n${provider}-tracked-source-conflict\\nthree\\n".`,
    ...targetConflictPaths.slice(1).map((targetPath) =>
      `Also run a direct write so that ${targetPath} becomes exactly "one\\n${provider}-tracked-source-conflict\\nthree\\n".`
    ),
    `After that direct filesystem write completes, reply exactly ${alignMarker} and nothing else.`,
  ].join('\n'), []))

  await waitForCompletionsAndFiles({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    events,
    expectedCompletionCount: 1,
    completionSinceMs: alignSinceMs,
    requiredFiles: targetConflictPaths,
    forbiddenFiles: [],
    timeoutMs,
    pollMs,
    historyDir,
    providerErrorSinceMs: alignSinceMs,
  })
  await waitForHistoryOutputMarkers({
    historyDir,
    providerHistoryDirs,
    markerGroups: [[alignMarker]],
    sinceMs: alignSinceMs,
    timeoutMs,
    pollMs,
  })
  await waitForAgentsIdle({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentIds: [resolverAgent.id],
    getSessionStateRequest,
    timeoutMs,
    pollMs,
  })
  const alignedStatus = await waitForTrackedConflictFileFanout({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    getWorkspaceLiveSyncStatusRequest,
    workspaces: allWorkspaces,
    targetWorkspaces,
    provider,
    expectedContent: sourceSideContent,
    expectConflictsCleared: false,
    timeoutMs,
    pollMs,
  })

  const resolveSinceMs = Date.now()
  const resolveMarker = `${provider.toUpperCase()}_TRACKED_WORKSPACE_LIVE_SYNC_CONFLICT_RESOLVED`
  await client.send(submitPromptRequest(session.id, attachment.id, sourceAgent.id, [
    'This is a live Chariox workspace live sync tracked-mode conflict resolution drill.',
    'Use direct filesystem writes through shell/native file tools. Do not use any Chariox workspace live sync MCP/runtime tools.',
    `Run a direct write in the current workspace so that ${relativePath} becomes exactly "one\\n${provider}-tracked-resolved\\nthree\\n".`,
    `After that direct filesystem write completes, reply exactly ${resolveMarker} and nothing else.`,
  ].join('\n'), []))

  await waitForCompletionsAndFiles({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    events,
    expectedCompletionCount: 1,
    completionSinceMs: resolveSinceMs,
    requiredFiles: [path.join(workspace, relativePath)],
    forbiddenFiles: [],
    timeoutMs,
    pollMs,
    historyDir,
    providerErrorSinceMs: resolveSinceMs,
  })
  await waitForHistoryOutputMarkers({
    historyDir,
    providerHistoryDirs,
    markerGroups: [[resolveMarker]],
    sinceMs: resolveSinceMs,
    timeoutMs,
    pollMs,
  })
  await waitForAgentsIdle({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentIds: [sourceAgent.id],
    getSessionStateRequest,
    timeoutMs,
    pollMs,
  })
  const resolvedStatus = await waitForTrackedConflictFileFanout({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    getWorkspaceLiveSyncStatusRequest,
    workspaces: allWorkspaces,
    targetWorkspaces,
    provider,
    expectedContent: resolvedContent,
    expectConflictsCleared: true,
    timeoutMs,
    pollMs,
  })

  return {
    alignedStatus,
    resolvedStatus,
    resolvedContent,
  }
}

export async function runTrackedOutsideWorkspacePhase({
  client,
  session,
  attachment,
  events,
  provider,
  agent,
  siblingWritePath,
  historyDir,
  providerHistoryDirs,
  timeoutMs,
  pollMs,
  submitPromptRequest,
  respondToInteractionRequest,
  getSessionStateRequest,
  remoteSourceSideEffects,
}) {
  const completionSinceMs = Date.now()
  const marker = `${provider.toUpperCase()}_TRACKED_OUTSIDE_WORKSPACE_DONE`
  const siblingWriteRelativePath = `../sibling-repo/${provider}-tracked-sibling.txt`
  const outsideWorkspaceScript = [
    'mkdir -p ../sibling-repo',
    `printf '${provider}-tracked-sibling\\n' > ${siblingWriteRelativePath}`,
  ].join('\n')
  const respondedInteractionIds = new Set()
  const allowOutsideWorkspacePermission = async () => {
    const state = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
    const currentSession = state.session ?? state
    const interactions = currentSession.active_interactions ?? []
    for (const interaction of interactions) {
      if (interaction.agent_id !== agent.id || interaction.kind !== 'permission' || respondedInteractionIds.has(interaction.id)) {
        continue
      }
      const message = String(interaction.message ?? '')
      if (!message.includes(siblingWritePath) && !message.includes(`${provider}-tracked-sibling`)) {
        continue
      }
      respondedInteractionIds.add(interaction.id)
      await client.send(respondToInteractionRequest(session.id, interaction.id, 'allow_once'))
      await sleep(pollMs)
    }
  }
  await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
    'This is a live Chariox workspace live sync outside-workspace permission drill.',
    'Run this exact POSIX shell script once from the current workspace directory:',
    outsideWorkspaceScript,
    'Do not edit files in the current workspace.',
    'Do not inspect, read, list, or verify any files before or after running the script.',
    `After the script completes, reply exactly ${marker} and nothing else.`,
  ].join('\n'), []))
  await waitForCompletionsAndFiles({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    events,
    expectedCompletionCount: 1,
    completionSinceMs,
    requiredFiles: remoteSourceSideEffects ? [] : [siblingWritePath],
    forbiddenFiles: [],
    timeoutMs,
    pollMs,
    historyDir,
    providerErrorSinceMs: completionSinceMs,
    beforePoll: allowOutsideWorkspacePermission,
  })
  await waitForHistoryOutputMarkers({
    historyDir,
    providerHistoryDirs,
    markerGroups: [[marker]],
    sinceMs: completionSinceMs,
    timeoutMs,
    pollMs,
  })
  await waitForAgentsIdle({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentIds: [agent.id],
    getSessionStateRequest,
    timeoutMs,
    pollMs,
  })
  if (!remoteSourceSideEffects) {
    await assertFileContent(siblingWritePath, `${provider}-tracked-sibling\n`)
  }
}

export async function runTrackedWorkspaceLiveSyncDrill({
  client,
  session,
  attachment,
  events,
  provider,
  agent,
  targetOriginAgent,
  spawnedSessionId,
  workspace,
  siblingWorkspace,
  targetWorkspace,
  targetWorkspaces,
  historyDir,
  timeoutMs,
  pollMs,
  machineRef,
  getSessionStateRequest,
  getWorkspaceLiveSyncStatusRequest,
  listProviderProcessesRequest,
  respondToInteractionRequest,
  submitPromptRequest,
  startedAt,
  kernelUrl,
  options,
}) {
  const trackedTargetWorkspaces = targetWorkspaces ?? [targetWorkspace]
  let bidirectional = null
  if (options.trackedBidirectional) {
    if (!targetOriginAgent) {
      throw new Error('tracked bidirectional drill requires a target-origin agent')
    }
    bidirectional = await runTrackedTargetOriginPhase({
      client,
      session,
      attachment,
      events,
      provider,
      agent: targetOriginAgent.agent,
      workspace,
      targetWorkspaces: trackedTargetWorkspaces,
      historyDir,
      providerHistoryDirs: options.providerHistoryDirs,
      timeoutMs,
      pollMs,
      getSessionStateRequest,
      getWorkspaceLiveSyncStatusRequest,
      submitPromptRequest,
    })
    for (const worktree of [workspace, ...trackedTargetWorkspaces]) {
      await resetTrackedWorkspace(worktree)
    }
  }
  const sourceHeadBefore = await gitHead(workspace)
  const targetHeadsBefore = Object.fromEntries(await Promise.all(trackedTargetWorkspaces.map(async (target) => [target, await gitHead(target)])))
  for (const target of trackedTargetWorkspaces) {
    await writeFile(
      path.join(target, 'outputs', `${provider}-tracked-rebase.txt`),
      `alpha\n${provider}-tracked-target-local\nbeta\nomega\n`,
      'utf8',
    )
    await writeFile(
      path.join(target, 'outputs', `${provider}-tracked-conflict.txt`),
      `one\n${provider}-tracked-target-conflict\nthree\n`,
      'utf8',
    )
  }
  const linkedState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
  const linkedSession = linkedState.session ?? linkedState
  const linkedAgents = linkedSession.agents ?? []
  const sessionAgent = linkedAgents.find((candidate) => candidate.id === agent.id)
  if (!sessionAgent) {
    throw new Error(`tracked drill spawned agent ${agent.id} session=${agent.session_id ?? agent.sessionId ?? 'unknown'} but current session ${session.id} has agents=${linkedAgents.map((candidate) => candidate.id).join(',')}; spawnedSessionId=${spawnedSessionId ?? 'unknown'}`)
  }

  const completionSinceMs = Date.now()
  const marker = `${provider.toUpperCase()}_TRACKED_WORKSPACE_LIVE_SYNC_DONE`
  const siblingWritePath = path.join(siblingWorkspace, `${provider}-tracked-sibling.txt`)
  const trackedScript = [
    `printf 'line-a\\n${provider}-tracked-modified\\n' > tracked.txt`,
    'mkdir -p outputs ignored',
    `printf '${provider}-tracked-added\\n' > outputs/${provider}-tracked-added.txt`,
    `printf '\\000\\005\\377\\012' > outputs/${provider}-tracked-binary.bin`,
    `rm -f outputs/${provider}-tracked-delete.txt`,
    `mv outputs/${provider}-tracked-rename-source.txt outputs/${provider}-tracked-renamed.txt`,
    `printf '${provider}-tracked-renamed\\n' > outputs/${provider}-tracked-renamed.txt`,
    `printf 'alpha\\nbeta\\n${provider}-tracked-source\\nomega\\n' > outputs/${provider}-tracked-rebase.txt`,
    `printf 'one\\n${provider}-tracked-source-conflict\\nthree\\n' > outputs/${provider}-tracked-conflict.txt`,
    `printf '${provider}-ignored\\n' > ignored/${provider}-ignored.txt`,
  ].join('\n')
  await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
    'This is a live Chariox workspace live sync tracked-mode drill.',
    'Use direct filesystem writes through shell/native file tools. Do not use any Chariox workspace live sync MCP/runtime tools.',
    'Run this exact POSIX shell script once from the current workspace directory:',
    trackedScript,
    'Do not inspect, read, list, or verify any files before or after running the script.',
    `After the script completes, reply exactly ${marker} and nothing else.`,
  ].join('\n'), []))

  await waitForCompletionsAndFiles({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    events,
    expectedCompletionCount: 1,
    completionSinceMs,
    requiredFiles: [
      path.join(workspace, 'outputs', `${provider}-tracked-added.txt`),
      path.join(workspace, 'outputs', `${provider}-tracked-renamed.txt`),
      ...(options.remoteSourceSideEffects ? [] : [
        path.join(workspace, 'ignored', `${provider}-ignored.txt`),
      ]),
    ],
    forbiddenFiles: [],
    timeoutMs,
    pollMs,
    historyDir,
    providerErrorSinceMs: completionSinceMs,
  })
  await waitForHistoryOutputMarkers({
    historyDir,
    providerHistoryDirs: options.providerHistoryDirs,
    markerGroups: [[marker]],
    sinceMs: completionSinceMs,
    timeoutMs,
    pollMs,
  })
  await waitForAgentsIdle({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentIds: [agent.id],
    getSessionStateRequest,
    timeoutMs,
    pollMs,
  })

  const conflictStatus = await waitForTrackedFanout({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    getWorkspaceLiveSyncStatusRequest,
    sourceWorkspace: workspace,
    targetWorkspaces: trackedTargetWorkspaces,
    provider,
    remoteSourceSideEffects: options.remoteSourceSideEffects,
    timeoutMs,
    pollMs,
  })
  await waitForHistoryNotices({
    historyDir,
    sinceMs: completionSinceMs,
    timeoutMs,
    pollMs,
    requirements: [
      {
        label: 'tracked turn summary',
        includes: [
          'Workspace live sync tracked turn summary',
          `source agent \`${agent.id}\``,
          `outputs/${provider}-tracked-added.txt`,
          'Next action:',
        ],
      },
      {
        label: 'tracked conflict notice',
        includes: [
          'Workspace live sync conflict',
          `outputs/${provider}-tracked-conflict.txt`,
          'Next action: assign a resolver agent',
        ],
      },
    ],
  })
  await runTrackedOutsideWorkspacePhase({
    client,
    session,
    attachment,
    events,
    provider,
    agent,
    siblingWritePath,
    historyDir,
    providerHistoryDirs: options.providerHistoryDirs,
    timeoutMs,
    pollMs,
    submitPromptRequest,
    respondToInteractionRequest,
    getSessionStateRequest,
    remoteSourceSideEffects: options.remoteSourceSideEffects,
  })
  let resolution = null
  if (options.trackedBidirectional) {
    resolution = await runTrackedConflictResolutionPhase({
      client,
      session,
      attachment,
      events,
      provider,
      sourceAgent: agent,
      resolverAgent: targetOriginAgent.agent,
      workspace,
      targetWorkspaces: trackedTargetWorkspaces,
      historyDir,
      providerHistoryDirs: options.providerHistoryDirs,
      timeoutMs,
      pollMs,
      getSessionStateRequest,
      getWorkspaceLiveSyncStatusRequest,
      submitPromptRequest,
    })
  }
  await waitForAgentsIdle({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentIds: [agent.id],
    getSessionStateRequest,
    timeoutMs,
    pollMs,
  })
  const status = resolution?.resolvedStatus ?? conflictStatus
  const sourceHeadAfter = await gitHead(workspace)
  const targetHeadsAfter = Object.fromEntries(await Promise.all(trackedTargetWorkspaces.map(async (target) => [target, await gitHead(target)])))
  const changedTargetHead = trackedTargetWorkspaces.find((target) => targetHeadsAfter[target] !== targetHeadsBefore[target])
  if (sourceHeadAfter !== sourceHeadBefore || changedTargetHead) {
    const targetHeadSummary = trackedTargetWorkspaces.map((target) => `${target}: ${targetHeadsBefore[target]} -> ${targetHeadsAfter[target]}`).join('; ')
    throw new Error(`tracked workspace live sync unexpectedly created commits; source ${sourceHeadBefore} -> ${sourceHeadAfter}; targets ${targetHeadSummary}`)
  }
  const outsideTurnPath = path.join(workspace, 'outputs', `${provider}-outside-turn.txt`)
  const outsideTurnTargetPaths = trackedTargetWorkspaces.map((target) => path.join(target, 'outputs', `${provider}-outside-turn.txt`))
  await writeFile(outsideTurnPath, `${provider}-outside-turn-change\n`, 'utf8')
  const outsideTurnStarted = Date.now()
  while (Date.now() - outsideTurnStarted < Math.min(5_000, timeoutMs)) {
    await client.send({ PumpTerminalOutput: { session_id: session.id, attachment_id: attachment.id } }).catch(() => {})
    await client.send(getWorkspaceLiveSyncStatusRequest(session.id)).catch(() => {})
    for (const outsideTurnTargetPath of outsideTurnTargetPaths) {
      if (await fileExists(outsideTurnTargetPath)) {
        throw new Error(`outside-turn tracked workspace change unexpectedly synced to target: ${outsideTurnTargetPath}`)
      }
    }
    await sleep(Math.min(500, pollMs))
  }
  const finalState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
  const processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []
  console.log(JSON.stringify({
    status: 'ok',
    mode: 'tracked-workspace-live-sync-live-drill',
    kernelUrl,
    machineRef,
    workspace,
    targetWorkspace,
    targetWorkspaces: trackedTargetWorkspaces,
    targetBranch: options.targetBranch,
    providers: [provider],
    model: options.model,
    providerModels: { [provider]: modelForProvider(provider, options) },
    durationMs: Date.now() - startedAt,
    agent: {
      id: agent.id,
      alias: agent.alias,
      provider,
    },
    completionCount: events.filter((event) => event.event === 'assistant_message_completed').length,
    terminalEventCount: events.filter((event) => event.event === 'terminal_output').length,
    tracked: {
      sourceTrackedContent: await readFile(path.join(workspace, 'tracked.txt'), 'utf8'),
      targetTrackedContent: await readFile(path.join(targetWorkspace, 'tracked.txt'), 'utf8'),
      targetAddedContent: await readFile(path.join(targetWorkspace, 'outputs', `${provider}-tracked-added.txt`), 'utf8'),
      targetRenamedContent: await readFile(path.join(targetWorkspace, 'outputs', `${provider}-tracked-renamed.txt`), 'utf8'),
      targetBinaryHex: (await readFile(path.join(targetWorkspace, 'outputs', `${provider}-tracked-binary.bin`))).toString('hex'),
      sourceRebaseContent: await readFile(path.join(workspace, 'outputs', `${provider}-tracked-rebase.txt`), 'utf8'),
      targetRebaseContent: await readFile(path.join(targetWorkspace, 'outputs', `${provider}-tracked-rebase.txt`), 'utf8'),
      sourceConflictContent: await readFile(path.join(workspace, 'outputs', `${provider}-tracked-conflict.txt`), 'utf8'),
      targetConflictContent: await readFile(path.join(targetWorkspace, 'outputs', `${provider}-tracked-conflict.txt`), 'utf8'),
      targetDeleteFileExists: await fileExists(path.join(targetWorkspace, 'outputs', `${provider}-tracked-delete.txt`)),
      targetRenameSourceFileExists: await fileExists(path.join(targetWorkspace, 'outputs', `${provider}-tracked-rename-source.txt`)),
      sourceIgnoredFileExists: await fileExists(path.join(workspace, 'ignored', `${provider}-ignored.txt`)),
      targetIgnoredFileExists: await fileExists(path.join(targetWorkspace, 'ignored', `${provider}-ignored.txt`)),
      outsideTurnSourceFileExists: await fileExists(outsideTurnPath),
      outsideTurnTargetFileExists: await fileExists(outsideTurnTargetPaths[0]),
      outsideTurnTargetFileExistsByTarget: Object.fromEntries(await Promise.all(outsideTurnTargetPaths.map(async (targetPath) => [targetPath, await fileExists(targetPath)]))),
      siblingRepoWriteContent: options.remoteSourceSideEffects ? null : await readFile(siblingWritePath, 'utf8'),
      sourceSideEffectsLocation: options.remoteSourceSideEffects ? 'remote-worker' : 'local-mirror',
      sourceHeadBefore,
      sourceHeadAfter,
      targetHeadBefore: targetHeadsBefore[targetWorkspace],
      targetHeadAfter: targetHeadsAfter[targetWorkspace],
      targetHeadsBefore,
      targetHeadsAfter,
      sourceCharioxignore: await readFile(path.join(workspace, '.charioxignore'), 'utf8'),
      targetCharioxignore: await readFile(path.join(targetWorkspace, '.charioxignore'), 'utf8'),
      targetCharioxignores: Object.fromEntries(await Promise.all(trackedTargetWorkspaces.map(async (target) => [target, await readFile(path.join(target, '.charioxignore'), 'utf8')]))),
    },
    bidirectional,
    resolution,
    conflictStatus,
    workspaceLiveSyncStatus: status,
    providerProcesses: processes.map((process) => ({
      processId: process.process_id,
      provider: process.provider,
      pid: process.pid ?? null,
      ownerRunIds: process.owner_provider_run_ids || [],
    })),
    focusedAgentId: finalState.session?.focused_agent_id ?? finalState.focused_agent_id ?? null,
  }, null, 2))
}
