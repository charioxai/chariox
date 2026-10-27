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
  moveAgentToRemoteRequest,
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
): Promise<{ session: RuntimeSession; agent: AgentInstance | null }> {
  const response = await client.send<Record<string, unknown>>(
    createSessionRequest(workspace, worktree, alias, agentDefaults, sliceRef ?? null),
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
    spawnAgentRequest(sessionId, provider, alias, model, worktree, effort, mode, permissions, undefined, undefined, sliceRef),
  )
  const agent = expectVariant<{ agent: AgentInstance }>(response, "AgentSpawned").agent
  return machineRef
    ? moveNativeAgentToRemote(client, sessionId, agent.id, machineRef)
    : agent
}

export async function prepareCreatedNativeAgent(
  client: LocalIpcClient,
  sessionId: string,
  agent: AgentInstance,
  alias: string | undefined,
  machineRef: string | undefined,
): Promise<AgentInstance> {
  const placed = machineRef
    ? await moveNativeAgentToRemote(client, sessionId, agent.id, machineRef)
    : agent
  return maybeAliasNativeAgent(client, sessionId, placed, alias)
}

async function moveNativeAgentToRemote(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  machineRef: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(
    moveAgentToRemoteRequest(sessionId, agentId, machineRef),
  )
  return expectVariant<{ agent: AgentInstance }>(response, "AgentMovedToRemote").agent
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
