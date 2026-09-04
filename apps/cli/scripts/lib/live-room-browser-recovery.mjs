import assert from "node:assert/strict"

export function assertRoomBrowserRecoveryActions(actions, submission, baselineSequence) {
  const attempts = actions.filter((item) => item.actor_id === submission.actor_id
    && item.sequence > baselineSequence && item.sequence <= submission.sequence)
    .sort((a, b) => a.sequence - b.sequence)
  assert.deepEqual(attempts.map((item) => [item.kind, item.state]), [
    ["click", "completed"], ["fill", "failed"], ["fill", "completed"], ["submit", "completed"],
  ], "recovery requires exactly replacement, rejected fill, fresh fill and submit in order")
  assert.equal(new Set(attempts.map((item) => item.action_id)).size, 4)
  for (const [index, item] of attempts.entries()) {
    assert.equal(item.mode, "browser")
    assert.ok(Number.isSafeInteger(item.sequence) && item.sequence > (attempts[index - 1]?.sequence ?? baselineSequence))
    assert.deepEqual(item.targets, submission.targets, "recovery must stay in the same tab")
  }
  const [replacement, stale, fill] = attempts
  assert.deepEqual(stale.outcome, { status: "failed", code: "controller_failure" })
  return { replacement, stale, fill }
}

// Inspect bounded provider-tool history, not the user's prompt or the model's
// claim. Only return a boolean; error payloads never enter drill evidence.
export async function observeRoomStaleToolError(input, agentId, priorTurnIds) {
  const { client, requests, sessionId } = input
  const deadline = Date.now() + 5_000
  const request = async (value, variant) => {
    const remaining = deadline - Date.now()
    assert.ok(remaining > 0, "stale tool history deadline")
    let response
    try { response = await input.withTimeout(client.send(value), remaining, "stale tool history") }
    catch { throw new Error("stale tool history unavailable") }
    assert.ok(response?.[variant], "stale tool history unavailable")
    return response[variant]
  }
  const outline = await request(requests.getSessionHistoryOutlineRequest(sessionId, [agentId], 2), "SessionHistoryOutline")
  const turns = outline.agents?.find((item) => item.agent_id === agentId)?.turns ?? []
  let loaded = 0
  let chars = 0
  let inspectedEntries = 0
  let inspectedChars = 0
  const seen = new Set()
  const inspect = (item) => {
    if (!item?.entry || inspectedEntries >= 256 || inspectedChars >= 131072) return false
    inspectedEntries += 1
    if (item?.entry?.kind !== "provider_tool" || typeof item.entry.text !== "string" || item.entry.text.length > 32768) return false
    inspectedChars += item.entry.text.length
    if (inspectedChars > 131072) return false
    let tool
    try { tool = JSON.parse(item.entry.text) } catch { return false }
    if (!tool?.tool?.endsWith?.("slice_browser_fill") || tool.input?.text !== "STALE ATTEMPT MUST NOT LAND") return false
    const hasCode = (value) => typeof value === "string" && /\b(?:environment_)?stale_element_reference\b/.test(value)
    if (hasCode(tool.error)) return true
    if (!hasCode(tool.output)) return false
    if (tool.status === "failed" || tool.status === "error") return true
    // Codex can finish an MCP request successfully while its result isError
    // records the rejected action. Successful output text alone is not proof.
    try { return JSON.parse(tool.output)?.isError === true } catch { return false }
  }
  for (const turn of turns.slice(0, 2)) {
    if (typeof turn.turn_id !== "string" || priorTurnIds.has(turn.turn_id)) continue
    if ((turn.entries ?? []).slice(0, 256).some(inspect) || inspect(turn.summary)) return true
    for (const blob of (turn.blobs ?? []).slice(0, 16)) {
      if (blob.kind !== "provider_tool" || typeof blob.blob_id !== "string" || seen.has(blob.blob_id)
        || !Number.isSafeInteger(blob.total_chars) || blob.total_chars < 0 || blob.total_chars > 32768
        || loaded >= 8 || chars + blob.total_chars > 131072 || inspectedEntries >= 256 || inspectedChars >= 131072) continue
      seen.add(blob.blob_id)
      loaded += 1
      chars += blob.total_chars
      const content = await request(requests.getSessionHistoryBlobContentRequest(sessionId, agentId, blob.blob_id), "SessionHistoryBlobContent")
      if ((content.entries ?? []).slice(0, 256).some(inspect)) return true
    }
  }
  return false
}
