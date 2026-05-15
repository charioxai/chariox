import {
  normalizeRuntimeSession,
  type AgentInstance,
  type RuntimeSession,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  aliasAgentRequest,
  updateAgentConfigRequest,
  updateAgentProfileRequest,
  updateAgentSubstitutesRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export type UpdateAgentConfigOptions = {
  executionMode?: "build" | "plan" | null
  clearExecutionMode?: boolean
  permissionLevel?: "required" | "yolo" | null
  clearPermissionLevel?: boolean
}

export type UpdateAgentProfileOptions = {
  provider?: string | null
  model?: string | null
  effort?: string | null
  clearEffort?: boolean
}

export async function aliasAgent(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  alias: string,
): Promise<{ session: RuntimeSession; agent: AgentInstance }> {
  const response = await client.send<Record<string, unknown>>(aliasAgentRequest(sessionId, agentId, alias))
  const payload = expectVariant<{ session: RuntimeSession; agent: AgentInstance }>(response, "AgentAliased")
  return {
    ...payload,
    session: normalizeRuntimeSession(payload.session),
  }
}

export async function updateAgentConfig(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  options: UpdateAgentConfigOptions,
): Promise<{ session: RuntimeSession; agent: AgentInstance }> {
  const response = await client.send<Record<string, unknown>>(
    updateAgentConfigRequest({
      sessionId,
      agentId,
      ...(options.executionMode !== undefined ? { executionMode: options.executionMode } : {}),
      ...(options.clearExecutionMode !== undefined ? { clearExecutionMode: options.clearExecutionMode } : {}),
      ...(options.permissionLevel !== undefined ? { permissionLevel: options.permissionLevel } : {}),
      ...(options.clearPermissionLevel !== undefined ? { clearPermissionLevel: options.clearPermissionLevel } : {}),
    }),
  )
  const payload = expectVariant<{ session: RuntimeSession; agent: AgentInstance }>(response, "AgentConfigUpdated")
  return {
    ...payload,
    session: normalizeRuntimeSession(payload.session),
  }
}

export async function updateAgentProfile(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  options: UpdateAgentProfileOptions,
): Promise<{ session: RuntimeSession; agent: AgentInstance }> {
  const response = await client.send<Record<string, unknown>>(
    updateAgentProfileRequest({
      sessionId,
      agentId,
      ...(options.provider !== undefined ? { provider: options.provider } : {}),
      ...(options.model !== undefined ? { model: options.model } : {}),
      ...(options.effort !== undefined ? { effort: options.effort } : {}),
      ...(options.clearEffort !== undefined ? { clearEffort: options.clearEffort } : {}),
    }),
  )
  const payload = expectVariant<{ session: RuntimeSession; agent: AgentInstance }>(response, "AgentProfileUpdated")
  return {
    ...payload,
    session: normalizeRuntimeSession(payload.session),
  }
}

export async function updateAgentSubstitutes(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  action: Record<string, unknown>,
): Promise<{ session: RuntimeSession; agent: AgentInstance }> {
  const response = await client.send<Record<string, unknown>>(
    updateAgentSubstitutesRequest({
      sessionId,
      agentId,
      action: action as never,
    }),
  )
  const payload = expectVariant<{ session: RuntimeSession; agent: AgentInstance }>(response, "AgentConfigUpdated")
  return {
    ...payload,
    session: normalizeRuntimeSession(payload.session),
  }
}
