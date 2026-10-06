// MP-08 / MP-10 / MP-11. Official kernel client only; no provider execution seam.
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { performance } from 'node:perf_hooks'
import { createHash } from 'node:crypto'

export function unwrap(response, kind) {
  if (!response?.[kind]) throw new Error(`MP-08 / MP-10 / MP-11 expected ${kind}`)
  return response[kind]
}

export async function openKernelClient({ clientRoot, kernelUrl }) {
  if (!path.isAbsolute(clientRoot)) throw new Error('MP-10 absolute built kernel client required')
  const load = name => import(pathToFileURL(path.join(clientRoot, 'dist', `${name}.js`)).href)
  const [{ LocalIpcClient }, requests, helpers] = await Promise.all([load('ipc'), load('ipc-requests'), load('session-history-fragments')])
  if (typeof helpers.assembleSessionHistoryFinalMessage !== 'function') throw new Error('MP-10 round-2 fragment helper build required')
  return { client: new LocalIpcClient(kernelUrl), requests, helpers }
}

async function agentOutline({ client, requests }, { sessionId, agentId }, count, cursor = null) {
  const outline = unwrap(await client.send(requests.getSessionHistoryOutlineRequest(sessionId, [agentId], count, cursor)), 'SessionHistoryOutline')
  return outline.agents.find(agent => agent.agent_id === agentId) ?? { turns: [] }
}

export async function submitBenchmarkPrompt(api, { sessionId, agentId, attachmentId, prompt }, submit = request => api.client.send(request)) {
  const baselinePromptIds = [], cursors = new Set()
  let cursor = null
  do {
    const outline = await agentOutline(api, { sessionId, agentId }, 20, cursor)
    baselinePromptIds.push(...outline.turns.map(turn => turn.prompt_id).filter(Boolean))
    cursor = outline.next_cursor ?? null
    if (cursor) {
      const key = JSON.stringify(cursor)
      if (cursors.has(key)) throw new Error('MP-10 repeated admission history cursor')
      cursors.add(key)
    }
  } while (cursor)
  const submitted = unwrap(await submit(api.requests.submitPromptRequest(sessionId, attachmentId, agentId, prompt, [])), 'PromptSubmitted')
  const outcomes = ['Started', 'Queued'].filter(kind => submitted.outcome?.[kind])
  const admissionOutcome = outcomes[0], promptId = submitted.outcome?.[admissionOutcome]?.prompt?.id
  if (outcomes.length !== 1 || !promptId) throw new Error('MP-10 submitted prompt identity missing; never replay')
  return { sessionId, agentId, promptId, admissionOutcome, baselinePromptIds,
    submittedPromptSha256: createHash('sha256').update(prompt).digest('hex') }
}

export async function getTurn(api, identity) {
  const outline = await agentOutline(api, identity, identity.admissionOutcome === 'Queued' ? 20 : 2)
  if (identity.admissionOutcome !== 'Queued') return outline.turns.find(turn => turn.prompt_id === identity.promptId) ?? null
  if (!Array.isArray(identity.baselinePromptIds) || !identity.submittedPromptSha256) throw new Error('MP-10 queued admission correlation missing')
  const matches = []
  for (const turn of outline.turns) {
    if (!turn.prompt_id || identity.baselinePromptIds.includes(turn.prompt_id) || turn.user_prompt?.entry?.kind !== 'user_prompt') continue
    const original = turn.user_prompt
    const entries = original.fragment_start === 0 && original.fragment_end === original.total_chars
      ? [] : await loadTurnHistory(api, { ...identity, turn })
    const fragments = [original, ...entries.filter(item => item.entry_index === original.entry_index && item.entry.kind === 'user_prompt')]
    const text = api.helpers.assembleSessionHistoryEntry(fragments)
    if (createHash('sha256').update(text).digest('hex') === identity.submittedPromptSha256) matches.push(turn)
  }
  if (matches.length > 1 || (matches[0] && identity.promotedPromptId && matches[0].prompt_id !== identity.promotedPromptId)) {
    throw new Error('MP-10 ambiguous queued benchmark turn; never replay')
  }
  if (matches[0]) identity.promotedPromptId = matches[0].prompt_id
  return matches[0] ?? null
}

export async function loadTurnHistory({ client, requests }, { sessionId, agentId, turn }) {
  const entries = [...(turn.entries ?? [])]
  for (const blobId of new Set((turn.blobs ?? []).map(blob => blob.blob_id))) {
    const content = unwrap(await client.send(requests.getSessionHistoryBlobContentRequest(sessionId, agentId, blobId)), 'SessionHistoryBlobContent')
    if (content.blob_id !== blobId) throw new Error('MP-10 history blob identity mismatch')
    entries.push(...content.entries)
  }
  return entries
}

// Audit complete original events, never JSON fragments or summary previews.
export function assembleTurnEntries(entries, helpers) {
  const groups = new Map()
  for (const item of entries) {
    const group = groups.get(item.entry_index) ?? []
    group.push(item); groups.set(item.entry_index, group)
  }
  return [...groups].sort(([a], [b]) => a - b).map(([, group]) => {
    const text = helpers.assembleSessionHistoryEntry(group)
    return { ...group[0], fragment_start: 0, fragment_end: Array.from(text).length,
      total_chars: Array.from(text).length, entry: { ...group[0].entry, text } }
  })
}

export async function waitForSettlement(api, identity, {
  maxMs, cancel, guard = async () => null, observe = async () => {},
  now = () => performance.now(), pause = ms => new Promise(resolve => setTimeout(resolve, ms)),
  cancellationMs = 120000,
}) {
  if (!Number.isFinite(maxMs) || maxMs <= 0) throw new Error('MP-10 positive settlement budget required')
  const started = now()
  const observationErrors = []
  let turn = null, cancellation = null, cancelStarted = null
  for (;;) {
    const cause = await guard()
    if (!cancellation && (cause || now() - started >= maxMs)) {
      cancellation = { cause: cause?.cause ?? 'wall_time', requestedAtElapsedMs: now() - started }
      await cancel(); cancelStarted = now()
    }
    turn = await getTurn(api, identity)
    if (['completed', 'failed', 'cancelled'].includes(turn?.lifecycle)) break
    if (cancellation && now() - cancelStarted >= cancellationMs) break
    // Evaluator observation is separate from provider settlement. Callers must
    // not feed these results back to the provider or turn errors into passes.
    try { await observe() }
    catch (error) { observationErrors.push({ errorClass: error.name, errorCode: error.code ?? null, elapsedMs: now() - started }) }
    await pause(1000)
  }
  return { turn, elapsedMs: now() - started, cancellation, observationErrors }
}
