import {
  normalizeRuntimeSession,
  type AgentInstance,
  type RuntimeAttachment,
  type RuntimeSession,
  type SessionAgentDefaults,
} from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import {
  aliasAgentRequest,
  attachToSessionRequest,
  createSessionRequest,
  resolveSessionRequest,
  spawnAgentRequest,
} from "../ipc-requests.js"

export type NativeProviderId = "claude" | "codex" | "opencode"
export type NativeExecutionMode = "build" | "plan"
export type NativePermissionLevel = "required" | "yolo"

export async function createNativeSession(
  client: LocalIpcClient,
  workspace: string,
  worktree: string,
  alias: string | undefined,
  agentDefaults: SessionAgentDefaults,
  sliceRef?: string,
  machineRef?: string,
): Promise<{ session: RuntimeSession; agent: AgentInstance | null }> {
  // The kernel places the agent on the machine, so the machine's provider chooses its model.
  const response = await client.send<Record<string, unknown>>(
    createSessionRequest(workspace, worktree, alias, agentDefaults, sliceRef ?? null, null, machineRef ?? null),
  )
  const payload = expectVariant<{ session: RuntimeSession; agent?: AgentInstance | null }>(response, "SessionCreated")
  return {
    session: normalizeRuntimeSession(payload.session),
    agent: payload.agent ?? null,
  }
}

export async function resolveNativeSession(
  client: LocalIpcClient,
  sessionRef: string,
  workspace: string,
): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(resolveSessionRequest(sessionRef, workspace))
  return normalizeRuntimeSession(expectVariant<{ session: RuntimeSession }>(response, "SessionResolved").session)
}

export async function attachNativeSession(
  client: LocalIpcClient,
  sessionId: string,
  clientId: string,
): Promise<RuntimeAttachment> {
  const response = await client.send<Record<string, unknown>>(attachToSessionRequest(sessionId, clientId))
  return expectVariant<{ attachment: RuntimeAttachment }>(response, "SessionAttached").attachment
}

export async function spawnNativeAgent(
  client: LocalIpcClient,
  sessionId: string,
  provider: NativeProviderId,
  alias: string | undefined,
  model: string | null | undefined,
  worktree: string | undefined,
  effort: string | null | undefined,
  mode: NativeExecutionMode,
  permissions: NativePermissionLevel,
  machineRef?: string,
  sliceRef?: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(
    spawnAgentRequest(sessionId, provider, alias, model, worktree, effort, mode, permissions, machineRef, undefined, sliceRef),
  )
  return expectVariant<{ agent: AgentInstance }>(response, "AgentSpawned").agent
}

export async function prepareCreatedNativeAgent(
  client: LocalIpcClient,
  sessionId: string,
  agent: AgentInstance,
  alias: string | undefined,
): Promise<AgentInstance> {
  return maybeAliasNativeAgent(client, sessionId, agent, alias)
}

async function maybeAliasNativeAgent(
  client: LocalIpcClient,
  sessionId: string,
  agent: AgentInstance,
  alias: string | undefined,
): Promise<AgentInstance> {
  if (!alias || agent.alias === alias) return agent
  const response = await client.send<Record<string, unknown>>(aliasAgentRequest(sessionId, agent.id, alias))
  return expectVariant<{ agent: AgentInstance }>(response, "AgentAliased").agent
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`expected ${variant} response, received ${JSON.stringify(response)}`)
  }
  return response[variant] as T
}
