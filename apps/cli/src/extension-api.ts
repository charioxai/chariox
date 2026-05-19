import type {
  AgentInstance,
  ArrobaEnvironmentConfig,
  ArrobaMcpServerConfig,
  ArrobaScriptMetadata,
  ArrobaSkillMetadata,
  McpImportOutcome,
  SkillImportOutcome,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  getMcpServerRequest,
  getEnvironmentRequest,
  getScriptRequest,
  getSkillRequest,
  grantAgentExtensionRequest,
  importMcpServersRequest,
  importSkillsRequest,
  installMcpServerRequest,
  installSkillRequest,
  listEnvironmentsRequest,
  listMcpServersRequest,
  listScriptsRequest,
  listSkillsRequest,
  registerEnvironmentRequest,
  registerScriptRequest,
  removeEnvironmentRequest,
  removeScriptRequest,
  revokeAgentExtensionRequest,
  uninstallMcpServerRequest,
  uninstallSkillRequest,
  updateMcpServerRequest,
  updateSkillRequest,
  validateScriptRequest,
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
    grantAgentExtensionRequest(workspaceTarget, agentRef, "mcp", name),
  )
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionGranted").agent
}

export async function revokeAgentMcp(
  client: LocalIpcClient,
  agentRef: string,
  name: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(revokeAgentExtensionRequest(agentRef, "mcp", name))
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionRevoked").agent
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
    grantAgentExtensionRequest(workspaceTarget, agentRef, "skill", name),
  )
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionGranted").agent
}

export async function revokeAgentSkill(
  client: LocalIpcClient,
  agentRef: string,
  name: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(revokeAgentExtensionRequest(agentRef, "skill", name))
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionRevoked").agent
}

export async function listEnvironments(
  client: LocalIpcClient,
  workspaceTarget: string,
): Promise<ArrobaEnvironmentConfig[]> {
  const response = await client.send<Record<string, unknown>>(listEnvironmentsRequest(workspaceTarget))
  return expectVariant<{ environments: ArrobaEnvironmentConfig[] }>(response, "EnvironmentsListed").environments
}

export async function getEnvironment(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<ArrobaEnvironmentConfig> {
  const response = await client.send<Record<string, unknown>>(getEnvironmentRequest(workspaceTarget, name))
  return expectVariant<{ environment: ArrobaEnvironmentConfig }>(response, "Environment").environment
}

export async function registerEnvironment(
  client: LocalIpcClient,
  workspaceTarget: string,
  config: ArrobaEnvironmentConfig,
): Promise<ArrobaEnvironmentConfig> {
  const response = await client.send<Record<string, unknown>>(
    registerEnvironmentRequest(workspaceTarget, config as unknown as Record<string, unknown>),
  )
  return expectVariant<{ environment: ArrobaEnvironmentConfig }>(response, "EnvironmentRegistered").environment
}

export async function removeEnvironment(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<ArrobaEnvironmentConfig> {
  const response = await client.send<Record<string, unknown>>(removeEnvironmentRequest(workspaceTarget, name))
  return expectVariant<{ environment: ArrobaEnvironmentConfig }>(response, "EnvironmentRemoved").environment
}

export async function listScripts(
  client: LocalIpcClient,
  workspaceTarget: string,
): Promise<ArrobaScriptMetadata[]> {
  const response = await client.send<Record<string, unknown>>(listScriptsRequest(workspaceTarget))
  return expectVariant<{ scripts: ArrobaScriptMetadata[] }>(response, "ScriptsListed").scripts
}

export async function getScript(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<ArrobaScriptMetadata> {
  const response = await client.send<Record<string, unknown>>(getScriptRequest(workspaceTarget, name))
  return expectVariant<{ script: ArrobaScriptMetadata }>(response, "Script").script
}

export async function validateScript(
  client: LocalIpcClient,
  workspaceTarget: string,
  sourcePath: string,
  environment: string,
  name?: string | null,
): Promise<ArrobaScriptMetadata> {
  const response = await client.send<Record<string, unknown>>(
    validateScriptRequest(workspaceTarget, sourcePath, environment, name),
  )
  return expectVariant<{ script: ArrobaScriptMetadata }>(response, "ScriptValidated").script
}

export async function registerScript(
  client: LocalIpcClient,
  workspaceTarget: string,
  sourcePath: string,
  environment: string,
  name?: string | null,
): Promise<ArrobaScriptMetadata> {
  const response = await client.send<Record<string, unknown>>(
    registerScriptRequest(workspaceTarget, sourcePath, environment, name),
  )
  return expectVariant<{ script: ArrobaScriptMetadata }>(response, "ScriptRegistered").script
}

export async function removeScript(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<ArrobaScriptMetadata> {
  const response = await client.send<Record<string, unknown>>(removeScriptRequest(workspaceTarget, name))
  return expectVariant<{ script: ArrobaScriptMetadata }>(response, "ScriptRemoved").script
}

export async function grantAgentScript(
  client: LocalIpcClient,
  workspaceTarget: string,
  agentRef: string,
  name: string,
  environment: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(
    grantAgentExtensionRequest(workspaceTarget, agentRef, "script", name, environment),
  )
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionGranted").agent
}

export async function revokeAgentScript(
  client: LocalIpcClient,
  agentRef: string,
  name: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(revokeAgentExtensionRequest(agentRef, "script", name))
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionRevoked").agent
}
