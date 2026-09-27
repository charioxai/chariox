import assert from "node:assert/strict"

// Use the submitted prompt identity, not any older completed provider turn.
export async function waitForRoomProviderSettlement(input, agentId, promptId) {
  const { client, requests, sessionId } = input
  assert.ok(typeof promptId === "string" && promptId.length > 0, "submitted provider prompt lacks an identity")
  const settlement = await input.waitFor(async () => {
    const response = await input.withTimeout(client.send(requests.getSessionHistoryOutlineRequest(
      sessionId, [agentId], 2)), 5_000, "provider settlement history")
    const turns = response?.SessionHistoryOutline?.agents?.find((agent) => agent.agent_id === agentId)?.turns ?? []
    const turn = turns.find((item) => item.prompt_id === promptId)
    if (!turn || turn.lifecycle !== "completed") return false
    const failed = turn.summary?.entry?.kind === "provider_error"
      || (turn.entries ?? []).some((item) => item.entry?.kind === "provider_error")
      || (turn.blobs ?? []).some((item) => item.kind === "provider_error")
    if (failed) return { failed: true }
    const state = await input.withTimeout(client.send(requests.getSessionStateRequest(sessionId)),
      5_000, "provider settlement state")
    const agent = state?.SessionState?.session?.agents?.find((item) => item.id === agentId)
    if (agent?.is_processing !== false) return false
    assert.ok(typeof turn.turn_id === "string" && turn.turn_id.length > 0, "completed provider turn lacks an identity")
    return { promptId, turnId: turn.turn_id, lifecycle: "completed", agentIdle: true }
  }, 60_000, "official provider turn did not settle after the Room action")
  if (settlement.failed) throw new Error("official provider turn failed after the Room action")
  return settlement
}
