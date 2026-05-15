import type {
  AgentInstance,
  ArrobaMcpServerConfig,
  ArrobaSkillMetadata,
  McpImportOutcome,
  SkillImportOutcome,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  getMcpServerRequest,
  getSkillRequest,
  grantAgentCapabilityRequest,
  importMcpServersRequest,
  importSkillsRequest,
  installMcpServerRequest,
  installSkillRequest,
  listMcpServersRequest,
  listSkillsRequest,
  revokeAgentCapabilityRequest,
  uninstallMcpServerRequest,
  uninstallSkillRequest,
  updateMcpServerRequest,
  updateSkillRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export async function listMcpServers(
  client: LocalIpcClient,
  workspaceTarget: string,
): Promise<ArrobaMcpServerConfig[]> {
  const response = await client.send<Record<string, unknown>>(listMcpServersRequest(workspaceTarget))
  return expectVariant<{ mcps: ArrobaMcpServerConfig[] }>(response, "McpServersListed").mcps
}

export async function installMcpServer(
  client: LocalIpcClient,
  workspaceTarget: string,
  config: ArrobaMcpServerConfig,
): Promise<ArrobaMcpServerConfig> {
  const response = await client.send<Record<string, unknown>>(
    installMcpServerRequest(workspaceTarget, config as unknown as Record<string, unknown>),
  )
  return expectVariant<{ mcp: ArrobaMcpServerConfig }>(response, "McpServerInstalled").mcp
}

export async function updateMcpServer(
  client: LocalIpcClient,
  workspaceTarget: string,
  config: ArrobaMcpServerConfig,
): Promise<ArrobaMcpServerConfig> {
  const response = await client.send<Record<string, unknown>>(
    updateMcpServerRequest(workspaceTarget, config as unknown as Record<string, unknown>),
  )
  return expectVariant<{ mcp: ArrobaMcpServerConfig }>(response, "McpServerUpdated").mcp
}

export async function uninstallMcpServer(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<string> {
  const response = await client.send<Record<string, unknown>>(uninstallMcpServerRequest(workspaceTarget, name))
  return expectVariant<{ name: string }>(response, "McpServerUninstalled").name
}

export async function importMcpServers(
  client: LocalIpcClient,
  workspaceTarget: string,
  provider: string,
  name?: string | null,
): Promise<McpImportOutcome> {
  const response = await client.send<Record<string, unknown>>(importMcpServersRequest(workspaceTarget, provider, name))
  return expectVariant<{ outcome: McpImportOutcome }>(response, "McpServersImported").outcome
}

export async function getMcpServer(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<ArrobaMcpServerConfig> {
  const response = await client.send<Record<string, unknown>>(getMcpServerRequest(workspaceTarget, name))
  return expectVariant<{ mcp: ArrobaMcpServerConfig }>(response, "McpServer").mcp
}

export async function grantAgentMcp(
  client: LocalIpcClient,
  workspaceTarget: string,
  agentRef: string,
  name: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(
    grantAgentCapabilityRequest(workspaceTarget, agentRef, "mcp", name),
  )
  return expectVariant<{ agent: AgentInstance }>(response, "AgentCapabilityGranted").agent
}

export async function revokeAgentMcp(
  client: LocalIpcClient,
  agentRef: string,
  name: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(revokeAgentCapabilityRequest(agentRef, "mcp", name))
  return expectVariant<{ agent: AgentInstance }>(response, "AgentCapabilityRevoked").agent
}

export async function listSkills(
  client: LocalIpcClient,
  workspaceTarget: string,
): Promise<ArrobaSkillMetadata[]> {
  const response = await client.send<Record<string, unknown>>(listSkillsRequest(workspaceTarget))
  return expectVariant<{ skills: ArrobaSkillMetadata[] }>(response, "SkillsListed").skills
}

export async function installSkill(
  client: LocalIpcClient,
  workspaceTarget: string,
  sourcePath: string,
): Promise<ArrobaSkillMetadata> {
  const response = await client.send<Record<string, unknown>>(installSkillRequest(workspaceTarget, sourcePath))
  return expectVariant<{ skill: ArrobaSkillMetadata }>(response, "SkillInstalled").skill
}

export async function updateSkill(
  client: LocalIpcClient,
  workspaceTarget: string,
  sourcePath: string,
): Promise<ArrobaSkillMetadata> {
  const response = await client.send<Record<string, unknown>>(updateSkillRequest(workspaceTarget, sourcePath))
  return expectVariant<{ skill: ArrobaSkillMetadata }>(response, "SkillUpdated").skill
}

export async function uninstallSkill(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<ArrobaSkillMetadata> {
  const response = await client.send<Record<string, unknown>>(uninstallSkillRequest(workspaceTarget, name))
  return expectVariant<{ skill: ArrobaSkillMetadata }>(response, "SkillUninstalled").skill
}

export async function importSkills(
  client: LocalIpcClient,
  workspaceTarget: string,
  provider: string,
  name?: string | null,
): Promise<SkillImportOutcome> {
  const response = await client.send<Record<string, unknown>>(importSkillsRequest(workspaceTarget, provider, name))
  return expectVariant<{ outcome: SkillImportOutcome }>(response, "SkillsImported").outcome
}

export async function getSkill(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<ArrobaSkillMetadata> {
  const response = await client.send<Record<string, unknown>>(getSkillRequest(workspaceTarget, name))
  return expectVariant<{ skill: ArrobaSkillMetadata }>(response, "Skill").skill
}

export async function grantAgentSkill(
  client: LocalIpcClient,
  workspaceTarget: string,
  agentRef: string,
  name: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(
    grantAgentCapabilityRequest(workspaceTarget, agentRef, "skill", name),
  )
  return expectVariant<{ agent: AgentInstance }>(response, "AgentCapabilityGranted").agent
}

export async function revokeAgentSkill(
  client: LocalIpcClient,
  agentRef: string,
  name: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(revokeAgentCapabilityRequest(agentRef, "skill", name))
  return expectVariant<{ agent: AgentInstance }>(response, "AgentCapabilityRevoked").agent
}
