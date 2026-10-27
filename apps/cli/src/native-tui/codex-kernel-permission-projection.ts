import type { LocalIpcClient } from "../ipc.js"
import type { RuntimeInteraction } from "../cli-types.js"
import { getSessionStateRequest, respondToInteractionRequest } from "../ipc-requests.js"
import type { CodexJsonRpcMessage } from "./codex-json-rpc.js"
import type { CodexNativeBindingState } from "./codex-turn-submission.js"
import { codexNativePermissionChoice } from "./codex-permission.js"

const prefix = "chariox-kernel-permission:"

export const isCodexKernelPermissionResponse = (message: CodexJsonRpcMessage): boolean =>
  typeof message.id === "string" && message.id.startsWith(prefix)

// MP-08 / MP-10 / MP-11: Provider requests arrive on the kernel's connection.
// The native display sees a projection of that exact pending interaction;
// its answer never goes directly to the provider or grants another turn.
export function createCodexKernelPermissionProjection(options: {
  client: LocalIpcClient
  sessionId: string
  agentId: string
  bindState: CodexNativeBindingState
  broadcast: (message: unknown) => void
  beforeProjectPrompt?: (promptId: string) => void
  debug?: (label: string, payload: unknown) => void
}) {
  const pending = new Map<string, { interactionId: string, runId: string, promptId: string, answered: boolean, resolved: boolean, threadId: string, turnId: string }>()
  const finish = (id: string, bound: NonNullable<ReturnType<typeof pending.get>>, declined: boolean) => {
    if (bound.resolved) return
    bound.resolved = true
    options.broadcast({ jsonrpc: "2.0", method: "serverRequest/resolved", params: { threadId: bound.threadId, requestId: id } })
    options.broadcast({ jsonrpc: "2.0", method: "item/completed", params: {
      threadId: bound.threadId, turnId: bound.turnId, completedAtMs: Date.now(),
      item: { type: "commandExecution", id, command: "Chariox permission request", cwd: process.cwd(), processId: null,
        status: declined ? "declined" : "completed", commandActions: [], aggregatedOutput: null, exitCode: null, durationMs: null },
    } })
  }
  const project = (interactions: RuntimeInteraction[]) => {
    const live = new Set<string>()
    if (interactions.length) options.debug?.("native_kernel_permission_candidates", { count: interactions.length, runId: options.bindState.run?.id, mappedTurns: options.bindState.displayTurns?.size })
    for (const interaction of interactions) {
      const origin = interaction.native_origin
      if (interaction.agent_id !== options.agentId || interaction.kind !== "permission"
        || origin?.scope !== "prompt" || origin.provider_run_id !== options.bindState.run?.id) { options.debug?.("native_kernel_permission_scope_rejected", { ownAgent: interaction.agent_id === options.agentId, originScope: origin?.scope, runMatches: origin?.provider_run_id === options.bindState.run?.id }); continue }
      const display = [...(options.bindState.displayTurns ?? [])]
        .find(([, turn]) => turn.promptId === origin.prompt_id)
      if (!display) { options.debug?.("native_kernel_permission_display_missing", { promptId: origin.prompt_id }); continue }
      const id = prefix + interaction.id
      live.add(id)
      if (pending.has(id)) continue
      options.beforeProjectPrompt?.(origin.prompt_id)
      pending.set(id, { interactionId: interaction.id, runId: origin.provider_run_id, promptId: origin.prompt_id, answered: false, resolved: false, threadId: display[1].threadId, turnId: display[0] })
      options.debug?.("native_kernel_permission_projected", { interactionId: interaction.id, threadId: display[1].threadId, turnId: display[0], requestedAtMs: interaction.requested_at_ms, finiteTimestamp: Number.isFinite(interaction.requested_at_ms) })
      options.broadcast({ jsonrpc: "2.0", method: "item/started", params: {
        threadId: display[1].threadId, turnId: display[0], item: {
          type: "commandExecution", id, command: "Chariox permission request", cwd: process.cwd(),
          processId: null, status: "inProgress", commandActions: [], aggregatedOutput: null, exitCode: null, durationMs: null,
        },
      } })
      options.broadcast({ jsonrpc: "2.0", id, method: "item/commandExecution/requestApproval", params: {
        threadId: display[1].threadId, turnId: display[0], itemId: id,
        startedAtMs: interaction.requested_at_ms, command: interaction.message,
        reason: `${interaction.title ?? "Chariox permission required"}\n${interaction.message}`,
      } })
    }
    for (const [id, bound] of pending) if (!live.has(id)) { finish(id, bound, false); pending.delete(id) }
  }
  const respond = async (message: CodexJsonRpcMessage): Promise<boolean> => {
    if (typeof message.id !== "string" || !message.id.startsWith(prefix)) return false
    const bound = pending.get(message.id)
    const choice = codexNativePermissionChoice(message)
    // Synthetic replies, including stale ones, must never reach upstream.
    options.debug?.("native_kernel_permission_reply", { known: !!bound, choice })
    if (!bound || bound.answered || !choice) return true
    bound.answered = true
    try {
      const state = await options.client.send<{ SessionState: { session: { active_interactions?: RuntimeInteraction[] } } }>(getSessionStateRequest(options.sessionId))
      const current = state.SessionState?.session.active_interactions?.find(i => i.id === bound.interactionId)
      const origin = current?.native_origin
      if (current?.agent_id !== options.agentId || current.kind !== "permission"
        || origin?.scope !== "prompt" || origin.provider_run_id !== bound.runId
        || options.bindState.run?.id !== bound.runId || origin.prompt_id !== bound.promptId
        || !current.choices.some(c => c.id === choice)) { finish(message.id, bound, true); return true }
      await options.client.send(respondToInteractionRequest(options.sessionId, current.id, choice))
      finish(message.id, bound, choice === "deny")
    } finally {
      // Keep concurrent replies fenced; a failed RPC leaves the reply retryable.
      bound.answered = bound.resolved
    }
    return true
  }
  return { project, respond }
}
