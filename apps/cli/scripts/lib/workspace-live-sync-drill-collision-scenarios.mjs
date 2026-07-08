import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { assertFileContent, destroyWorkspaceLiveSyncAgent, fileExists, gitHead, modelForProvider, resetTrackedWorkspace, sleep, unwrapVariant, workspaceLiveSyncSpawnAgentRequest, workspaceLiveSyncToolNames } from './workspace-live-sync-drill-runtime.mjs'
import { waitForAgentsIdle, waitForCompletionsAndFiles, waitForHistoryNotices, waitForHistoryOutputMarkers, waitForManagedEditResult, waitForManagedEditResults, waitForManagedReadSnapshot, waitForManagedReadSnapshots } from './workspace-live-sync-drill-waiters.mjs'

export async function runLiveCollisionAndExternalChecks({
  client,
  session,
  attachment,
  events,
  agents,
  modelForProvider,
  machineRef,
  kernelRef,
  workspace,
  outputsDir,
  historyDir,
  providerHistoryDirs,
  timeoutMs,
  pollMs,
  getSessionStateRequest,
  spawnAgentRequest,
  destroyAgentRequest,
  submitPromptRequest,
}) {
  const checks = []

  for (const { provider, agent } of agents) {
    const colliderProvider = agents.find((candidate) => candidate.provider !== provider)?.provider ?? provider
    const overlapPath = path.join(outputsDir, `${provider}-overlap.txt`)
    await writeFile(overlapPath, 'one\nTARGET\nthree\n', 'utf8')
    const collider = unwrapVariant(
      await client.send(workspaceLiveSyncSpawnAgentRequest(
        spawnAgentRequest,
        session.id,
        colliderProvider,
        `${provider}-workspace-live-sync-collider-${colliderProvider}`,
        modelForProvider(colliderProvider),
        workspace,
        'low',
        kernelRef,
      )),
      'AgentSpawned',
      'AgentSpawned',
    ).agent
    const firstNewText = `FROM_${provider.toUpperCase()}_A`
    const secondNewText = `FROM_${provider.toUpperCase()}_B`
    const tools = workspaceLiveSyncToolNames(provider)
    const overlapSameAreaReadStartedAt = Date.now()
    for (const [editAgent, label] of [[agent, 'A'], [collider, 'B']]) {
      await client.send(submitPromptRequest(session.id, attachment.id, editAgent.id, [
        'This is a live Arroba workspace live sync overlapping-writer drill.',
        'Use only Arroba workspace live sync. Do not use shell commands or native filesystem writes.',
        `First call \`${tools.read}\` exactly once with JSON arguments {"path":"outputs/${provider}-overlap.txt","domain":"text"}.`,
        `Then reply exactly ${provider.toUpperCase()}_OVERLAP_${label}_READ_DONE.`,
      ].join('\n'), []))
    }
    const overlapReadSnapshots = await waitForManagedReadSnapshots({
      historyDir,
      providerHistoryDirs,
      artifactPath: `outputs/${provider}-overlap.txt`,
      sinceMs: overlapSameAreaReadStartedAt,
      count: machineRef ? 1 : 2,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    for (const snapshot of overlapReadSnapshots) {
      if (snapshot.content_text !== 'one\nTARGET\nthree\n') {
        throw new Error(`overlap drill read after a write for ${provider}: ${JSON.stringify(overlapReadSnapshots)}`)
      }
    }
    const snapshotByAgentId = new Map(overlapReadSnapshots.map((snapshot) => [snapshot.agent_id, snapshot.snapshot_id]))
    const overlapSameAreaEditStartedAt = Date.now()
    for (const [editAgent, label, newText] of [[agent, 'A', firstNewText], [collider, 'B', secondNewText]]) {
      const snapshotId = snapshotByAgentId.get(editAgent.id) ?? overlapReadSnapshots[0]?.snapshot_id
      const prompt = [
        'Continue the live Arroba workspace live sync overlapping-writer drill.',
        'Use only Arroba workspace live sync. Do not use shell commands or native filesystem writes.',
        'Do not read, reread, or retry.',
        'Do not include a range field.',
        `Call \`${tools.edit}\` exactly once with JSON arguments {"path":"outputs/${provider}-overlap.txt","old_text":"TARGET","new_text":${JSON.stringify(newText)},"domain":"text","snapshot_id":${JSON.stringify(snapshotId)}}.`,
        `Then reply exactly ${provider.toUpperCase()}_OVERLAP_${label}_DONE if applied, or ${provider.toUpperCase()}_OVERLAP_${label}_BLOCKED if rejected.`,
      ].join('\n')
      await client.send(submitPromptRequest(session.id, attachment.id, editAgent.id, prompt, []))
    }
    const overlapEditResults = await waitForManagedEditResults({
      historyDir,
      providerHistoryDirs,
      artifactPath: `outputs/${provider}-overlap.txt`,
      sinceMs: overlapSameAreaEditStartedAt,
      // Remote lease histories can mirror one side of a cross-provider collision
      // without the matching provider_tool record even though the file mutation
      // has landed. The final content assertion below still verifies that one
      // write won and the losing edit did not corrupt the file.
      count: machineRef ? 1 : 2,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      providerHistoryDirs,
      markerGroups: [
        [
          `${provider.toUpperCase()}_OVERLAP_A_DONE`,
          `${provider.toUpperCase()}_OVERLAP_A_BLOCKED`,
        ],
        [
          `${provider.toUpperCase()}_OVERLAP_B_DONE`,
          `${provider.toUpperCase()}_OVERLAP_B_BLOCKED`,
        ],
      ],
      sinceMs: overlapSameAreaEditStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    await destroyWorkspaceLiveSyncAgent({
      client,
      destroyAgentRequest,
      sessionId: session.id,
      agent: collider,
    })
    if (!machineRef) {
      const appliedCount = overlapEditResults.filter((result) => result?.applied === true).length
      const conflictCount = overlapEditResults.filter((result) => result?.applied === false && result?.reason?.kind === 'conflict').length
      if (appliedCount !== 1 || conflictCount !== 1) {
        throw new Error(`overlap drill expected one applied edit and one conflict for ${provider}: ${JSON.stringify(overlapEditResults)}`)
      }
    }
    const overlapContent = await readFile(overlapPath, 'utf8')
    const allowedOverlapContents = new Set([
      `one\n${firstNewText}\nthree\n`,
      `one\n${secondNewText}\nthree\n`,
    ])
    if (!allowedOverlapContents.has(overlapContent)) {
      throw new Error(`overlap drill produced unexpected content for ${provider}: ${JSON.stringify(overlapContent)}`)
    }
    checks.push({
      provider,
      scenario: 'overlap_same_area',
      relativePath: `outputs/${provider}-overlap.txt`,
      finalContent: overlapContent,
      expectedOneOf: Array.from(allowedOverlapContents),
    })

    const nonOverlapPath = path.join(outputsDir, `${provider}-external-nonoverlap.txt`)
    const nonOverlapBase = 'header\nalpha\nTARGET\nomega\nfooter\n'
    const nonOverlapExternallyChanged = 'intro\nheader\nalpha\nTARGET\nomega\nfooter\noutro\n'
    const nonOverlapExpected = 'intro\nheader\nalpha\nREPLACED\nomega\nfooter\noutro\n'
    await writeFile(nonOverlapPath, nonOverlapBase, 'utf8')
    const nonOverlapReadStartedAt = Date.now()
    await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
      'This is a live Arroba workspace live sync external non-overlap drill.',
      'Use only Arroba workspace live sync.',
      `Call \`${tools.read}\` exactly once with JSON arguments {"path":"outputs/${provider}-external-nonoverlap.txt","domain":"text"}.`,
      `Remember the returned snapshot_id for the next turn. Then reply exactly ${provider.toUpperCase()}_EXTERNAL_NONOVERLAP_READ_DONE.`,
    ].join('\n'), []))
    const nonOverlapRead = await waitForManagedReadSnapshot({
      historyDir,
      providerHistoryDirs,
      artifactPath: `outputs/${provider}-external-nonoverlap.txt`,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      providerHistoryDirs,
      markerGroups: [[`${provider.toUpperCase()}_EXTERNAL_NONOVERLAP_READ_DONE`]],
      sinceMs: nonOverlapReadStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    if (nonOverlapRead.content_text !== nonOverlapBase) {
      throw new Error(`external non-overlap read happened after external write for ${provider}: ${JSON.stringify(nonOverlapRead.content_text)}`)
    }
    if (!machineRef) {
      await waitForAgentsIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentIds: [agent.id],
        getSessionStateRequest,
        timeoutMs,
        pollMs,
      })
    }
    await writeFile(nonOverlapPath, nonOverlapExternallyChanged, 'utf8')
    const nonOverlapEditStartedAt = Date.now()
    await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
      'Continue the external non-overlap drill.',
      'Use only Arroba workspace live sync. Do not reread the artifact.',
      `Use this exact snapshot_id: ${nonOverlapRead.snapshot_id}`,
      `Call \`${tools.edit}\` exactly once with JSON arguments {"path":"outputs/${provider}-external-nonoverlap.txt","old_text":"TARGET","new_text":"REPLACED","domain":"text","snapshot_id":${JSON.stringify(nonOverlapRead.snapshot_id)}}.`,
      `Then reply exactly ${provider.toUpperCase()}_EXTERNAL_NONOVERLAP_EDIT_DONE.`,
    ].join('\n'), []))
    const nonOverlapEdit = await waitForManagedEditResult({
      historyDir,
      providerHistoryDirs,
      artifactPath: `outputs/${provider}-external-nonoverlap.txt`,
      sinceMs: nonOverlapEditStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      providerHistoryDirs,
      markerGroups: [[`${provider.toUpperCase()}_EXTERNAL_NONOVERLAP_EDIT_DONE`]],
      sinceMs: nonOverlapEditStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    if (nonOverlapEdit?.applied !== true) {
      throw new Error(`external non-overlap edit was not applied for ${provider}: ${JSON.stringify(nonOverlapEdit)}`)
    }
    await assertFileContent(nonOverlapPath, nonOverlapExpected)
    checks.push({
      provider,
      scenario: 'external_non_overlap_rebase',
      relativePath: `outputs/${provider}-external-nonoverlap.txt`,
      finalContent: nonOverlapExpected,
    })

    const overlapExternalPath = path.join(outputsDir, `${provider}-external-overlap.txt`)
    const externalOverlapBase = 'one\nTARGET\nthree\n'
    const externalOverlapExpected = 'one\nEXTERNAL\nthree\n'
    await writeFile(overlapExternalPath, externalOverlapBase, 'utf8')
    const overlapReadStartedAt = Date.now()
    await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
      'This is a live Arroba workspace live sync external overlap drill.',
      'Use only Arroba workspace live sync.',
      `Call \`${tools.read}\` exactly once with JSON arguments {"path":"outputs/${provider}-external-overlap.txt","domain":"text"}.`,
      `Remember the returned snapshot_id for the next turn. Then reply exactly ${provider.toUpperCase()}_EXTERNAL_OVERLAP_READ_DONE.`,
    ].join('\n'), []))
    const overlapRead = await waitForManagedReadSnapshot({
      historyDir,
      providerHistoryDirs,
      artifactPath: `outputs/${provider}-external-overlap.txt`,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      providerHistoryDirs,
      markerGroups: [[`${provider.toUpperCase()}_EXTERNAL_OVERLAP_READ_DONE`]],
      sinceMs: overlapReadStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    if (overlapRead.content_text !== externalOverlapBase) {
      throw new Error(`external overlap read happened after external write for ${provider}: ${JSON.stringify(overlapRead.content_text)}`)
    }
    if (!machineRef) {
      await waitForAgentsIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentIds: [agent.id],
        getSessionStateRequest,
        timeoutMs,
        pollMs,
      })
    }
    await writeFile(overlapExternalPath, externalOverlapExpected, 'utf8')
    const overlapEditStartedAt = Date.now()
    await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
      'Continue the external overlap drill.',
      'Use only Arroba workspace live sync. Do not reread the artifact and do not retry if rejected.',
      `Use this exact snapshot_id: ${overlapRead.snapshot_id}`,
      `Call \`${tools.edit}\` exactly once with JSON arguments {"path":"outputs/${provider}-external-overlap.txt","old_text":"TARGET","new_text":"AGENT","domain":"text","snapshot_id":${JSON.stringify(overlapRead.snapshot_id)}}.`,
      `Then reply exactly ${provider.toUpperCase()}_EXTERNAL_OVERLAP_BLOCKED if rejected, or ${provider.toUpperCase()}_EXTERNAL_OVERLAP_UNEXPECTED_APPLIED if applied.`,
    ].join('\n'), []))
    const overlapEdit = await waitForManagedEditResult({
      historyDir,
      providerHistoryDirs,
      artifactPath: `outputs/${provider}-external-overlap.txt`,
      sinceMs: overlapEditStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      providerHistoryDirs,
      markerGroups: [[
        `${provider.toUpperCase()}_EXTERNAL_OVERLAP_BLOCKED`,
        `${provider.toUpperCase()}_EXTERNAL_OVERLAP_UNEXPECTED_APPLIED`,
      ]],
      sinceMs: overlapEditStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    if (overlapEdit?.applied !== false || overlapEdit?.reason?.kind !== 'conflict') {
      throw new Error(`external overlap edit was not rejected as a conflict for ${provider}: ${JSON.stringify(overlapEdit)}`)
    }
    await assertFileContent(overlapExternalPath, externalOverlapExpected)
    checks.push({
      provider,
      scenario: 'external_overlap_rejected',
      relativePath: `outputs/${provider}-external-overlap.txt`,
      finalContent: externalOverlapExpected,
    })
  }

  return checks
}
