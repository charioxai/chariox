import { readFile, readdir } from 'node:fs/promises'
import path from 'node:path'
import { assertFileBytes, assertFileContent, fileExists, sleep, unwrapVariant, workspaceLiveSyncMoveSourceName } from './workspace-live-sync-drill-runtime.mjs'

export async function providerErrorsSince({ historyDir, sinceMs }) {
  if (!historyDir) return []
  const errors = []
  const files = (await readdir(historyDir).catch(() => []))
    .filter((file) => file.endsWith('.jsonl'))
    .map((file) => path.join(historyDir, file))
    .sort()

  for (const file of files) {
    const lines = (await readFile(file, 'utf8').catch(() => '')).split(/\r?\n/)
    for (const line of lines) {
      if (!line.trim()) continue
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if ((entry.timestamp_ms ?? 0) < sinceMs) continue
      if (entry.kind !== 'provider_error' && !isProviderTerminalFailureNotice(entry)) continue
      errors.push({
        file,
        agentId: entry.agent_id ?? null,
        providerRunId: entry.provider_run_id ?? null,
        text: String(entry.text ?? '').trim(),
      })
    }
  }
  return errors
}

export function isProviderTerminalFailureNotice(entry) {
  if (entry.kind !== 'notice') return false
  const text = String(entry.text ?? '').trim()
  if (!text.startsWith('{')) return false
  try {
    const parsed = JSON.parse(text)
    return parsed?.type === 'error' && parsed?.error != null
  } catch {
    return false
  }
}

export async function throwIfProviderError({ historyDir, sinceMs }) {
  const errors = await providerErrorsSince({ historyDir, sinceMs })
  if (errors.length === 0) return
  const summary = errors.map((error) => {
    const owner = [error.agentId, error.providerRunId].filter(Boolean).join('/')
    return `${owner ? `${owner}: ` : ''}${error.text}`
  }).join(' | ')
  throw new Error(`provider error while waiting for workspace live sync drill progress: ${summary}`)
}

export async function waitForCompletionsAndFiles({ client, sessionId, attachmentId, events, expectedCompletionCount, completionSinceMs = 0, requiredFiles, forbiddenFiles, timeoutMs, pollMs, debugSnapshot, historyDir, providerErrorSinceMs = completionSinceMs, beforePoll = null }) {
  const started = Date.now()
  let lastRequiredCount = 0
  let lastMissingRequired = requiredFiles
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    await throwIfProviderError({ historyDir, sinceMs: providerErrorSinceMs })
    if (beforePoll) await beforePoll()
    const forbiddenExisting = []
    for (const forbiddenFile of forbiddenFiles) {
      if (await fileExists(forbiddenFile)) forbiddenExisting.push(forbiddenFile)
    }
    if (forbiddenExisting.length > 0) {
      throw new Error(`direct write unexpectedly created forbidden files: ${forbiddenExisting.join(', ')}`)
    }

    const requiredExisting = []
    const missingRequired = []
    for (const requiredFile of requiredFiles) {
      if (await fileExists(requiredFile)) requiredExisting.push(requiredFile)
      else missingRequired.push(requiredFile)
    }
    lastRequiredCount = requiredExisting.length
    lastMissingRequired = missingRequired
    const completed = events.filter((event) =>
      event.event === 'assistant_message_completed' &&
      ((event.observed_at_ms ?? 0) >= completionSinceMs)
    )
    if (requiredExisting.length === requiredFiles.length && completed.length >= expectedCompletionCount) {
      return completed
    }
    await sleep(pollMs)
  }
  const debug = debugSnapshot ? `; debug=${JSON.stringify(await debugSnapshot())}` : ''
  throw new Error(`timed out waiting for ${expectedCompletionCount} completions and ${requiredFiles.length} required files; required files present=${lastRequiredCount}; missing=${lastMissingRequired.join(', ')}${debug}`)
}

export async function assertFilesAbsent(filePaths, label) {
  const existing = []
  for (const filePath of filePaths) {
    if (await fileExists(filePath)) existing.push(filePath)
  }
  if (existing.length > 0) {
    throw new Error(`${label}: forbidden files exist: ${existing.join(', ')}`)
  }
}

export async function managedTargetFanoutSnapshot(targetWorkspaces, providers) {
  return Promise.all(targetWorkspaces.map(async (targetWorkspace) => {
    const outputsDir = path.join(targetWorkspace, 'outputs')
    return {
      targetWorkspace,
      providers: await Promise.all(providers.map(async (provider) => ({
        provider,
        content: await readFile(path.join(outputsDir, `${provider}.txt`), 'utf8'),
        movedContent: await readFile(path.join(outputsDir, `${provider}-moved.txt`), 'utf8'),
        opaqueMovedHex: (await readFile(path.join(outputsDir, `${provider}-opaque-moved.bin`))).toString('hex'),
        patchSourceFileExists: await fileExists(path.join(outputsDir, workspaceLiveSyncMoveSourceName(provider))),
        opaqueMoveSourceFileExists: await fileExists(path.join(outputsDir, `${provider}-opaque.bin`)),
        deletedFileExists: await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`)),
        opaqueDeletedFileExists: await fileExists(path.join(outputsDir, `${provider}-opaque-delete-me.bin`)),
        directWriteFileExists: await fileExists(path.join(outputsDir, `${provider}-direct.txt`)),
      }))),
    }
  }))
}

export async function assertManagedTargetFanout(targetWorkspaces, providers, { deletesApplied }) {
  for (const targetWorkspace of targetWorkspaces) {
    const outputsDir = path.join(targetWorkspace, 'outputs')
    for (const provider of providers) {
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
        throw new Error(`managed target fanout left patch source behind for ${provider} in ${targetWorkspace}`)
      }
      if (await fileExists(path.join(outputsDir, `${provider}-opaque.bin`))) {
        throw new Error(`managed target fanout left opaque move source behind for ${provider} in ${targetWorkspace}`)
      }
      if (deletesApplied) {
        if (await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`))) {
          throw new Error(`managed target fanout left deleted text file behind for ${provider} in ${targetWorkspace}`)
        }
        if (await fileExists(path.join(outputsDir, `${provider}-opaque-delete-me.bin`))) {
          throw new Error(`managed target fanout left deleted opaque file behind for ${provider} in ${targetWorkspace}`)
        }
      } else {
        await assertFileContent(path.join(outputsDir, `${provider}-delete-me.txt`), 'delete-me\n')
        await assertFileBytes(path.join(outputsDir, `${provider}-opaque-delete-me.bin`), [9, 8, 7])
      }
      if (await fileExists(path.join(outputsDir, `${provider}-direct.txt`))) {
        throw new Error(`managed target fanout created forbidden direct-write file for ${provider} in ${targetWorkspace}`)
      }
    }
  }
}

export async function waitForFilesAbsent({ filePaths, timeoutMs, pollMs }) {
  const started = Date.now()
  let existing = filePaths
  while (Date.now() - started < timeoutMs) {
    existing = []
    for (const filePath of filePaths) {
      if (await fileExists(filePath)) existing.push(filePath)
    }
    if (existing.length === 0) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for workspace live sync files to be absent; still present=${existing.join(', ')}`)
}

export async function waitForCompletionCount({ client, sessionId, attachmentId, events, expectedCompletionCount, completionSinceMs = 0, timeoutMs, pollMs, historyDir, providerErrorSinceMs = completionSinceMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    await throwIfProviderError({ historyDir, sinceMs: providerErrorSinceMs })
    const completed = events.filter((event) =>
      event.event === 'assistant_message_completed' &&
      ((event.observed_at_ms ?? 0) >= completionSinceMs)
    )
    if (completed.length >= expectedCompletionCount) return completed
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${expectedCompletionCount} completions`)
}

export async function waitForPromptPhase({
  client,
  sessionId,
  attachmentId,
  events,
  expectedCompletionCount,
  completionSinceMs,
  requiredFiles,
  forbiddenFiles,
  timeoutMs,
  pollMs,
  debugSnapshot,
  historyDir,
  providerErrorSinceMs = completionSinceMs,
}) {
  await waitForCompletionsAndFiles({
    client,
    sessionId,
    attachmentId,
    events,
    expectedCompletionCount,
    completionSinceMs,
    requiredFiles,
    forbiddenFiles,
    timeoutMs,
    pollMs,
    debugSnapshot,
    historyDir,
    providerErrorSinceMs,
  })
}

export async function historyProviderOutputMarkerGroups({ historyDir, providerHistoryDirs, markerGroups, sinceMs }) {
  const remaining = markerGroups.map((markers) => [...markers])
  const outputs = await historyProviderOutputsSince({ historyDir, providerHistoryDirs, sinceMs })

  return remaining.filter((markers) => !outputs.some((output) => markers.some((marker) => output.includes(marker))))
}

export async function historyProviderOutputsSince({ historyDir, providerHistoryDirs, sinceMs }) {
  const outputByKey = new Map()
  const files = (await managedHistoryFiles(managedHistoryDirs(historyDir, providerHistoryDirs))).sort()

  for (const file of files) {
    const lines = (await readFile(file, 'utf8').catch(() => '')).split(/\r?\n/)
    for (const line of lines) {
      if (!line.trim()) continue
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if ((entry.timestamp_ms ?? 0) < sinceMs) continue
      if (entry.kind !== 'provider_output' || typeof entry.text !== 'string') continue
      const key = `${file}:${entry.merge_key ?? entry.timestamp_ms ?? outputByKey.size}`
      outputByKey.set(key, `${outputByKey.get(key) ?? ''}${entry.text}`)
    }
  }

  return Array.from(outputByKey.values())
}

export async function throwIfProviderFailureMarker({ historyDir, providerHistoryDirs, sinceMs }) {
  const outputs = await historyProviderOutputsSince({ historyDir, providerHistoryDirs, sinceMs })
  const failure = outputs.find((output) => /\b[A-Z0-9_]+_WORKSPACE_LIVE_SYNC_FAILED\b/.test(output))
  if (failure) {
    throw new Error(`provider reported workspace live sync failure marker: ${failure}`)
  }
}

export async function pumpTerminalOutputIfAvailable({ client, sessionId, attachmentId }) {
  if (!client || !sessionId || !attachmentId) return
  await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
}

export async function waitForHistoryOutputMarkers({ historyDir, providerHistoryDirs, markerGroups, sinceMs, timeoutMs, pollMs, client, sessionId, attachmentId }) {
  const started = Date.now()
  let missing = markerGroups
  while (Date.now() - started < timeoutMs) {
    await pumpTerminalOutputIfAvailable({ client, sessionId, attachmentId })
    await throwIfProviderError({ historyDir, sinceMs })
    await throwIfProviderFailureMarker({ historyDir, providerHistoryDirs, sinceMs })
    missing = await historyProviderOutputMarkerGroups({ historyDir, providerHistoryDirs, markerGroups, sinceMs })
    if (missing.length === 0) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider output markers: ${missing.map((markers) => markers.join(' or ')).join(', ')}`)
}

export async function historyNoticeMessagesSince({ historyDir, sinceMs }) {
  const messages = []
  const files = (await readdir(historyDir).catch(() => []))
    .filter((file) => file.endsWith('.jsonl'))
    .map((file) => path.join(historyDir, file))
    .sort()

  for (const file of files) {
    const lines = (await readFile(file, 'utf8').catch(() => '')).split(/\r?\n/)
    for (const line of lines) {
      if (!line.trim()) continue
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if ((entry.timestamp_ms ?? 0) < sinceMs) continue
      if (entry.kind !== 'notice' || typeof entry.text !== 'string') continue
      messages.push(entry.text)
    }
  }
  return messages
}

export async function waitForHistoryNotices({ historyDir, requirements, sinceMs, timeoutMs, pollMs, client, sessionId, attachmentId }) {
  const started = Date.now()
  let messages = []
  let missing = requirements
  while (Date.now() - started < timeoutMs) {
    await pumpTerminalOutputIfAvailable({ client, sessionId, attachmentId })
    await throwIfProviderError({ historyDir, sinceMs })
    messages = await historyNoticeMessagesSince({ historyDir, sinceMs })
    missing = requirements.filter((requirement) =>
      !messages.some((message) => requirement.includes.every((needle) => message.includes(needle)))
    )
    if (missing.length === 0) return messages
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for workspace live sync runtime notices: missing=${missing.map((requirement) => requirement.label).join(', ')}; notices=${JSON.stringify(messages)}`)
}

export async function providerToolUpdatesSince({ historyDir, sinceMs }) {
  const updates = []
  const toolStarts = new Map()
  const files = (await readdir(historyDir).catch(() => []))
    .filter((file) => file.endsWith('.jsonl'))
    .map((file) => path.join(historyDir, file))
    .sort()

  for (const file of files) {
    const lines = (await readFile(file, 'utf8').catch(() => '')).split(/\r?\n/)
    for (const line of lines) {
      if (!line.trim()) continue
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if ((entry.timestamp_ms ?? 0) < sinceMs) continue
      if (entry.kind !== 'provider_tool' || typeof entry.text !== 'string') continue
      try {
        let update = JSON.parse(entry.text)
        const toolKey = entry.merge_key ?? update.id
        if (update.status === 'started' && update.input !== undefined) {
          toolStarts.set(toolKey, { input: update.input, tool: update.tool })
        } else if (toolStarts.has(toolKey)) {
          const startedUpdate = toolStarts.get(toolKey)
          update = {
            ...update,
            input: update.input ?? startedUpdate.input,
            tool: update.tool === 'tool_result' ? startedUpdate.tool : update.tool,
          }
        }
        updates.push(update)
      } catch {
        continue
      }
    }
  }
  return updates
}

export function providerToolUpdateMatches(update, expectation) {
  const tool = String(update.tool ?? '')
  if (!tool.endsWith(expectation.toolSuffix)) return false
  if (update.status !== 'completed') return false
  if (expectation.path != null && update.input?.path !== expectation.path) return false
  if (expectation.fromPath != null && update.input?.from_path !== expectation.fromPath) return false
  if (expectation.toPath != null && update.input?.to_path !== expectation.toPath) return false
  if (expectation.requireApplied === false) return true
  try {
    return parseManagedToolOutput(update.output)?.applied === true
  } catch {
    return false
  }
}

export async function waitForManagedToolExpectationsAndFiles({
  client,
  sessionId,
  attachmentId,
  historyDir,
  sinceMs,
  expectations,
  requiredFiles,
  forbiddenFiles,
  timeoutMs,
  pollMs,
  debugSnapshot,
}) {
  const started = Date.now()
  let missingExpectations = expectations
  let lastRequiredCount = 0
  let lastMissingRequired = requiredFiles
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    await throwIfProviderError({ historyDir, sinceMs })
    const forbiddenExisting = []
    for (const forbiddenFile of forbiddenFiles) {
      if (await fileExists(forbiddenFile)) forbiddenExisting.push(forbiddenFile)
    }
    if (forbiddenExisting.length > 0) {
      throw new Error(`direct write unexpectedly created forbidden files: ${forbiddenExisting.join(', ')}`)
    }

    const requiredExisting = []
    const missingRequired = []
    for (const requiredFile of requiredFiles) {
      if (await fileExists(requiredFile)) requiredExisting.push(requiredFile)
      else missingRequired.push(requiredFile)
    }
    lastRequiredCount = requiredExisting.length
    lastMissingRequired = missingRequired

    const updates = await providerToolUpdatesSince({ historyDir, sinceMs })
    const toolErrors = updates.filter((update) => update.status === 'error')
    if (toolErrors.length > 0) {
      throw new Error(`provider tool error while waiting for managed tool results: ${JSON.stringify(toolErrors)}`)
    }
    missingExpectations = expectations.filter((expectation) =>
      !updates.some((update) => providerToolUpdateMatches(update, expectation))
    )
    if (missingExpectations.length === 0 && requiredExisting.length === requiredFiles.length) return updates
    await sleep(pollMs)
  }
  const missingTools = missingExpectations
    .map((expectation) => `${expectation.toolSuffix}:${expectation.path ?? `${expectation.fromPath ?? ''}->${expectation.toPath ?? ''}`}`)
    .join(', ')
  const debug = debugSnapshot ? `; debug=${JSON.stringify(await debugSnapshot())}` : ''
  throw new Error(`timed out waiting for managed tool results and files; missing tools=${missingTools}; required files present=${lastRequiredCount}; missing=${lastMissingRequired.join(', ')}${debug}`)
}

export async function waitForAgentsIdle({ client, sessionId, attachmentId, agentIds, getSessionStateRequest, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const state = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = state.session ?? state
    const promptStates = session.prompt_states ?? {}
    const agents = session.agents ?? []
    const allIdle = agentIds.every((agentId) => {
      const agent = agents.find((candidate) => candidate.id === agentId)
      const promptState = promptStates[agentId] ?? {}
      const noPrompt =
        (promptState.active_prompt == null) &&
        ((promptState.queued_prompts ?? []).length === 0)
      return agent && !agent.is_processing && agent.state !== 'Working' && noPrompt
    })
    if (allIdle) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for agents to become idle: ${agentIds.join(', ')}`)
}

export function parseManagedToolOutput(rawOutput) {
  if (typeof rawOutput !== 'string') return null
  const parsed = JSON.parse(rawOutput)
  if (parsed?.structuredContent) return parsed.structuredContent
  const text = parsed?.content?.find?.((entry) => entry?.type === 'text' && typeof entry.text === 'string')?.text
  if (text) return JSON.parse(text)
  return parsed
}

export function managedHistoryDirs(historyDir, providerHistoryDirs = []) {
  return [...new Set([historyDir, ...providerHistoryDirs].filter(Boolean))]
}

export async function managedHistoryFiles(historyDirs) {
  const nested = await Promise.all(historyDirs.map(async (dir) => {
    const files = await readdir(dir).catch(() => [])
    return files
      .filter((file) => file.endsWith('.jsonl'))
      .map((file) => path.join(dir, file))
  }))
  return nested.flat()
}

export async function managedProviderToolUpdatesSince({ historyDir, providerHistoryDirs, sinceMs }) {
  const updates = []
  const toolStarts = new Map()
  const historyDirs = managedHistoryDirs(historyDir, providerHistoryDirs)
  const files = (await managedHistoryFiles(historyDirs)).sort()
  for (const file of files) {
    const lines = (await readFile(file, 'utf8').catch(() => '')).split(/\r?\n/)
    for (const line of lines) {
      if (!line.trim()) continue
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if ((entry.timestamp_ms ?? 0) < sinceMs) continue
      if (entry.kind !== 'provider_tool' || typeof entry.text !== 'string') continue
      try {
        let update = JSON.parse(entry.text)
        const toolKey = entry.merge_key ?? update.id
        if (update.status === 'started' && update.input !== undefined) {
          toolStarts.set(toolKey, { input: update.input, tool: update.tool })
        } else if (toolStarts.has(toolKey)) {
          const startedUpdate = toolStarts.get(toolKey)
          update = {
            ...update,
            input: update.input ?? startedUpdate.input,
            tool: update.tool === 'tool_result' ? startedUpdate.tool : update.tool,
          }
        }
        updates.push({ update, entry })
      } catch {
        continue
      }
    }
  }
  return updates
}

export async function waitForManagedReadSnapshot({ historyDir, providerHistoryDirs, artifactPath, timeoutMs, pollMs, client, sessionId, attachmentId }) {
  const snapshots = await waitForManagedReadSnapshots({
    historyDir,
    providerHistoryDirs,
    artifactPath,
    count: 1,
    sinceMs: 0,
    timeoutMs,
    pollMs,
    client,
    sessionId,
    attachmentId,
  })
  return snapshots[0]
}

export async function waitForManagedReadSnapshots({ historyDir, providerHistoryDirs, artifactPath, count, sinceMs, timeoutMs, pollMs, client, sessionId, attachmentId }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await pumpTerminalOutputIfAvailable({ client, sessionId, attachmentId })
    const snapshots = []
    const entries = await managedProviderToolUpdatesSince({ historyDir, providerHistoryDirs, sinceMs })
    for (const { update, entry } of entries) {
      const tool = String(update.tool ?? '')
      if (!tool.endsWith('read_artifact') || update.status !== 'completed') continue
      if (update.input?.path !== artifactPath) continue
      try {
        const output = parseManagedToolOutput(update.output)
        if (typeof output?.snapshot_id === 'string') {
          snapshots.push({
            ...output,
            agent_id: entry.agent_id ?? null,
            provider_run_id: entry.provider_run_id ?? null,
          })
        }
      } catch {
        continue
      }
    }
    if (snapshots.length >= count) return snapshots.slice(0, count)
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${count} managed read snapshots for ${artifactPath}`)
}

export async function waitForManagedEditResult({ historyDir, providerHistoryDirs, artifactPath, sinceMs, timeoutMs, pollMs, client, sessionId, attachmentId }) {
  const results = await waitForManagedEditResults({
    historyDir,
    providerHistoryDirs,
    artifactPath,
    sinceMs,
    count: 1,
    timeoutMs,
    pollMs,
    client,
    sessionId,
    attachmentId,
  })
  return results[0]
}

export async function waitForManagedEditResults({ historyDir, providerHistoryDirs, artifactPath, sinceMs, count, timeoutMs, pollMs, client, sessionId, attachmentId }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await pumpTerminalOutputIfAvailable({ client, sessionId, attachmentId })
    const results = []
    const entries = await managedProviderToolUpdatesSince({ historyDir, providerHistoryDirs, sinceMs })
    for (const { update } of entries) {
      const tool = String(update.tool ?? '')
      if (!tool.endsWith('edit_artifact') || !['completed', 'error', 'failed'].includes(update.status)) continue
      if (update.input?.path !== artifactPath) continue
      try {
        results.push(parseManagedToolOutput(update.output))
      } catch {
        continue
      }
    }
    if (results.length >= count) return results.slice(0, count)
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${count} managed edit results for ${artifactPath}`)
}
