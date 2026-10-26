// MP-08/MP-10/MP-11: rendered controls use the same kernel prompt authority.
import type { LocalIpcClient } from "../ipc.js"
import { cancelActivePromptRequest, getSessionStateRequest, sendTerminalInputRequest } from "../ipc-requests.js"
export async function sendClaudeRemoteRenderedInput(client: LocalIpcClient, sessionId: string,
  attachmentId: string, providerRunId: string, data: Buffer | string, agentId?: string) {
  // A native PTY interruption alone can leave the admitted home prompt active.
  // Navigation sequences and idle composer Escape still belong to the provider.
  if (data.toString() === "\u001b" && agentId) {
    const state = await client.send<{ SessionState?: { agent_activity?: Record<string, { active_prompt_count: number }> } }>(getSessionStateRequest(sessionId))
    if (state.SessionState?.agent_activity?.[agentId]?.active_prompt_count) {
      const result = await client.send<Record<string, unknown>>(cancelActivePromptRequest(sessionId, attachmentId, agentId))
      if (!("PromptCancelled" in result)) throw new Error("Rendered Claude interruption was not admitted by the kernel")
      return
    }
  }
  await client.send<Record<string, unknown>>(sendTerminalInputRequest(sessionId, attachmentId, data, providerRunId))
}
