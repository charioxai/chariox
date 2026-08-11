const providerOutputKinds = new Set(['provider_output', 'assistant_message'])

function unwrap(response, key) {
  return response?.[key] ?? response
}

function appendProviderOutputText(chunks, pageEntry) {
  const entry = pageEntry?.entry ?? pageEntry
  if (!providerOutputKinds.has(entry?.kind)) return
  if (typeof entry.text === 'string') chunks.push(entry.text)
}

export async function providerHistoryTextForPrompt(client, requests, sessionId, agentId, promptId) {
  const outline = unwrap(
    await client.send(requests.getSessionHistoryOutlineRequest(sessionId, [agentId], 8)),
    'SessionHistoryOutline',
  )
  const agent = outline.agents?.find((entry) => entry.agent_id === agentId)
  const turn = agent?.turns?.find((entry) => entry.prompt_id === promptId)
  if (!turn) return ''

  const chunks = []
  const items = [
    ...(turn.entries ?? []).map((entry) => ({ sequence: entry.entry_index ?? 0, entry })),
    ...(turn.blobs ?? []).map((blob) => ({ sequence: blob.sequence_start ?? 0, blob })),
    ...(turn.summary ? [{ sequence: turn.summary.entry_index ?? Number.MAX_SAFE_INTEGER, entry: turn.summary }] : []),
  ].sort((left, right) => left.sequence - right.sequence)
  for (const item of items) {
    if (item.entry) {
      appendProviderOutputText(chunks, item.entry)
      continue
    }
    if (!item.blob?.blob_id) continue
    const content = unwrap(
      await client.send(requests.getSessionHistoryBlobContentRequest(sessionId, agentId, item.blob.blob_id)),
      'SessionHistoryBlobContent',
    )
    for (const entry of content.entries ?? []) appendProviderOutputText(chunks, entry)
  }
  return chunks.join('')
}
