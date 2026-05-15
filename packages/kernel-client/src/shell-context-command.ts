import type { RuntimeSession } from "./kernel-types.js"
import { getSessionStateRequest } from "./ipc-requests.js"
import type { ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  parseExecutionMode,
  parsePermissionLevel,
} from "./shell-agent-policy.js"

type ShellContextCommandDeps = {
  client: {
    send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  }
}

export async function executeContextCommand(
  context: ShellContext,
  deps: ShellContextCommandDeps,
): Promise<ShellCommandResult> {
  let session: RuntimeSession | null = null
  if (context.sessionId) {
    try {
      const response = await deps.client.send(getSessionStateRequest(context.sessionId))
      session = expectSessionState(response)
    } catch {
      session = null
    }
  }
  return { ok: true, message: formatShellContext(context, session), data: { context, session } }
}

function formatShellContext(context: ShellContext, session: RuntimeSession | null = null): string {
  const currentAgent = context.agentId
    ? session?.agents.find((agent) => agent.id === context.agentId || agent.agent_ref === context.agentId || agent.alias === context.agentId) ?? null
    : null
  const currentAgentId = currentAgent?.id ?? context.agentId ?? null
  const currentAgentBusy = currentAgentId
    ? Boolean(session?.prompt_states?.[currentAgentId]?.active_prompt)
      || Boolean(session?.prompt_states?.[currentAgentId]?.queued_prompts?.length)
      || Boolean(session?.active_prompt?.target_agent_id === currentAgentId)
      || Boolean(session?.queued_prompts?.some((prompt) => prompt.target_agent_id === currentAgentId))
      || Boolean(currentAgent?.is_processing)
    : false
  const agentLabel = currentAgent
    ? `${currentAgent.agent_ref}${currentAgent.alias ? ` (${currentAgent.alias})` : ""}${currentAgentBusy ? " (busy)" : ""}`
    : `${context.agentId ?? "-"}${currentAgentBusy ? " (busy)" : ""}`
  const sessionMode = parseExecutionMode(session?.config_state?.values?.["agents.mode"]) ?? "build"
  const sessionPermissions = parsePermissionLevel(session?.config_state?.values?.["agents.permissions"]) ?? "yolo"
  const effectiveAgentMode = currentAgent?.execution_mode_override ?? sessionMode
  const effectiveAgentPermissions = currentAgent?.permission_level_override ?? sessionPermissions
  const lines = [
    `workspace: ${context.workspace}`,
    `worktree: ${context.worktree}`,
    `session: ${context.sessionId ?? "-"}`,
    `attachment: ${context.attachmentId ?? "-"}`,
    `agent: ${agentLabel}`,
    `mode: ${currentAgent ? `${effectiveAgentMode} (agent${currentAgent.execution_mode_override ? "-override" : "-session"})` : sessionMode}`,
    `permissions: ${currentAgent ? `${effectiveAgentPermissions} (agent${currentAgent.permission_level_override ? "-override" : "-session"})` : sessionPermissions}`,
    `workflow: ${context.workflowId ?? "-"}`,
    `provider: ${context.provider}`,
    `model: ${context.model}`,
    `effort: ${context.effort}`,
  ]
  const variables = Object.entries(context.variables)
  if (variables.length === 0) {
    lines.push("vars: -")
  } else {
    lines.push("vars:")
    for (const [name, value] of variables) {
      lines.push(`  $${name} = ${value}`)
    }
  }
  return lines.join("\n")
}

function expectSessionState(response: Record<string, unknown>): RuntimeSession {
  if ("SessionState" in response) {
    return (response.SessionState as { session: RuntimeSession }).session
  }
  return expectVariant<{ session: RuntimeSession }>(response, "SessionStateLoaded").session
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
