import type {
  AgentInstance,
  CharioxConnectorAdapterDefinition,
  CharioxConnectorDefinition,
  CharioxCredentialConfig,
  CharioxEnvironmentConfig,
  CharioxMcpServerConfig,
  CharioxScriptMetadata,
  CharioxSkillMetadata,
  McpImportOutcome,
  SkillImportOutcome,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  getMcpServerRequest,
  getConnectorRequest,
  getConnectorAdapterRequest,
  getCredentialRequest,
  getEnvironmentRequest,
  getScriptRequest,
  getSkillRequest,
  grantAgentExtensionRequest,
  importMcpServersRequest,
  importSkillsRequest,
  installMcpServerRequest,
  listHomeExtensionAuditRequest,
  installSkillRequest,
  listEnvironmentsRequest,
  listConnectorsRequest,
  listConnectorAdaptersRequest,
  listCredentialsRequest,
  listMcpServersRequest,
  listScriptsRequest,
  listSkillsRequest,
  registerEnvironmentRequest,
  registerConnectorRequest,
  registerConnectorAdapterRequest,
  registerCredentialRequest,
  registerScriptRequest,
  removeEnvironmentRequest,
  removeConnectorRequest,
  removeConnectorAdapterRequest,
  removeCredentialRequest,
  removeScriptRequest,
  revokeAgentExtensionRequest,
  syncRemoteExtensionManifestRequest,
  testConnectorRequest,
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
): Promise<CharioxMcpServerConfig[]> {
  const response = await client.send<Record<string, unknown>>(listMcpServersRequest(workspaceTarget))
  return expectVariant<{ mcps: CharioxMcpServerConfig[] }>(response, "McpServersListed").mcps
}

export async function installMcpServer(
  client: LocalIpcClient,
  workspaceTarget: string,
  config: CharioxMcpServerConfig,
): Promise<CharioxMcpServerConfig> {
  const response = await client.send<Record<string, unknown>>(
    installMcpServerRequest(workspaceTarget, config as unknown as Record<string, unknown>),
  )
  return expectVariant<{ mcp: CharioxMcpServerConfig }>(response, "McpServerInstalled").mcp
}

export async function updateMcpServer(
  client: LocalIpcClient,
  workspaceTarget: string,
  config: CharioxMcpServerConfig,
): Promise<CharioxMcpServerConfig> {
  const response = await client.send<Record<string, unknown>>(
    updateMcpServerRequest(workspaceTarget, config as unknown as Record<string, unknown>),
  )
  return expectVariant<{ mcp: CharioxMcpServerConfig }>(response, "McpServerUpdated").mcp
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
): Promise<CharioxMcpServerConfig> {
  const response = await client.send<Record<string, unknown>>(getMcpServerRequest(workspaceTarget, name))
  return expectVariant<{ mcp: CharioxMcpServerConfig }>(response, "McpServer").mcp
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
): Promise<CharioxSkillMetadata[]> {
  const response = await client.send<Record<string, unknown>>(listSkillsRequest(workspaceTarget))
  return expectVariant<{ skills: CharioxSkillMetadata[] }>(response, "SkillsListed").skills
}

export async function installSkill(
  client: LocalIpcClient,
  workspaceTarget: string,
  sourcePath: string,
): Promise<CharioxSkillMetadata> {
  const response = await client.send<Record<string, unknown>>(installSkillRequest(workspaceTarget, sourcePath))
  return expectVariant<{ skill: CharioxSkillMetadata }>(response, "SkillInstalled").skill
}

export async function updateSkill(
  client: LocalIpcClient,
  workspaceTarget: string,
  sourcePath: string,
): Promise<CharioxSkillMetadata> {
  const response = await client.send<Record<string, unknown>>(updateSkillRequest(workspaceTarget, sourcePath))
  return expectVariant<{ skill: CharioxSkillMetadata }>(response, "SkillUpdated").skill
}

export async function uninstallSkill(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<CharioxSkillMetadata> {
  const response = await client.send<Record<string, unknown>>(uninstallSkillRequest(workspaceTarget, name))
  return expectVariant<{ skill: CharioxSkillMetadata }>(response, "SkillUninstalled").skill
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
): Promise<CharioxSkillMetadata> {
  const response = await client.send<Record<string, unknown>>(getSkillRequest(workspaceTarget, name))
  return expectVariant<{ skill: CharioxSkillMetadata }>(response, "Skill").skill
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
): Promise<CharioxEnvironmentConfig[]> {
  const response = await client.send<Record<string, unknown>>(listEnvironmentsRequest(workspaceTarget))
  return expectVariant<{ environments: CharioxEnvironmentConfig[] }>(response, "EnvironmentsListed").environments
}

export async function getEnvironment(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<CharioxEnvironmentConfig> {
  const response = await client.send<Record<string, unknown>>(getEnvironmentRequest(workspaceTarget, name))
  return expectVariant<{ environment: CharioxEnvironmentConfig }>(response, "Environment").environment
}

export async function registerEnvironment(
  client: LocalIpcClient,
  workspaceTarget: string,
  config: CharioxEnvironmentConfig,
): Promise<CharioxEnvironmentConfig> {
  const response = await client.send<Record<string, unknown>>(
    registerEnvironmentRequest(workspaceTarget, config as unknown as Record<string, unknown>),
  )
  return expectVariant<{ environment: CharioxEnvironmentConfig }>(response, "EnvironmentRegistered").environment
}

export async function removeEnvironment(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<CharioxEnvironmentConfig> {
  const response = await client.send<Record<string, unknown>>(removeEnvironmentRequest(workspaceTarget, name))
  return expectVariant<{ environment: CharioxEnvironmentConfig }>(response, "EnvironmentRemoved").environment
}

export async function listScripts(
  client: LocalIpcClient,
  workspaceTarget: string,
): Promise<CharioxScriptMetadata[]> {
  const response = await client.send<Record<string, unknown>>(listScriptsRequest(workspaceTarget))
  return expectVariant<{ scripts: CharioxScriptMetadata[] }>(response, "ScriptsListed").scripts
}

export async function getScript(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<CharioxScriptMetadata> {
  const response = await client.send<Record<string, unknown>>(getScriptRequest(workspaceTarget, name))
  return expectVariant<{ script: CharioxScriptMetadata }>(response, "Script").script
}

export async function validateScript(
  client: LocalIpcClient,
  workspaceTarget: string,
  sourcePath: string,
  environment: string,
  name?: string | null,
): Promise<CharioxScriptMetadata> {
  const response = await client.send<Record<string, unknown>>(
    validateScriptRequest(workspaceTarget, sourcePath, environment, name),
  )
  return expectVariant<{ script: CharioxScriptMetadata }>(response, "ScriptValidated").script
}

export async function registerScript(
  client: LocalIpcClient,
  workspaceTarget: string,
  sourcePath: string,
  environment: string,
  name?: string | null,
): Promise<CharioxScriptMetadata> {
  const response = await client.send<Record<string, unknown>>(
    registerScriptRequest(workspaceTarget, sourcePath, environment, name),
  )
  return expectVariant<{ script: CharioxScriptMetadata }>(response, "ScriptRegistered").script
}

export async function removeScript(
  client: LocalIpcClient,
  workspaceTarget: string,
  name: string,
): Promise<CharioxScriptMetadata> {
  const response = await client.send<Record<string, unknown>>(removeScriptRequest(workspaceTarget, name))
  return expectVariant<{ script: CharioxScriptMetadata }>(response, "ScriptRemoved").script
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

export async function listCredentials(client: LocalIpcClient): Promise<CharioxCredentialConfig[]> {
  const response = await client.send<Record<string, unknown>>(listCredentialsRequest())
  return expectVariant<{ credentials: CharioxCredentialConfig[] }>(response, "CredentialsListed").credentials
}

export async function getCredential(client: LocalIpcClient, id: string): Promise<CharioxCredentialConfig> {
  const response = await client.send<Record<string, unknown>>(getCredentialRequest(id))
  return expectVariant<{ credential: CharioxCredentialConfig }>(response, "Credential").credential
}

export async function registerCredential(client: LocalIpcClient, sourcePath: string): Promise<CharioxCredentialConfig> {
  const response = await client.send<Record<string, unknown>>(registerCredentialRequest(sourcePath))
  return expectVariant<{ credential: CharioxCredentialConfig }>(response, "CredentialRegistered").credential
}

export async function removeCredential(client: LocalIpcClient, id: string): Promise<CharioxCredentialConfig> {
  const response = await client.send<Record<string, unknown>>(removeCredentialRequest(id))
  return expectVariant<{ credential: CharioxCredentialConfig }>(response, "CredentialRemoved").credential
}

export async function listConnectors(client: LocalIpcClient): Promise<CharioxConnectorDefinition[]> {
  const response = await client.send<Record<string, unknown>>(listConnectorsRequest())
  return expectVariant<{ connectors: CharioxConnectorDefinition[] }>(response, "ConnectorsListed").connectors
}

export async function listConnectorAdapters(client: LocalIpcClient): Promise<CharioxConnectorAdapterDefinition[]> {
  const response = await client.send<Record<string, unknown>>(listConnectorAdaptersRequest())
  return expectVariant<{ adapters: CharioxConnectorAdapterDefinition[] }>(response, "ConnectorAdaptersListed").adapters
}

export async function getConnectorAdapter(client: LocalIpcClient, name: string): Promise<CharioxConnectorAdapterDefinition> {
  const response = await client.send<Record<string, unknown>>(getConnectorAdapterRequest(name))
  return expectVariant<{ adapter: CharioxConnectorAdapterDefinition }>(response, "ConnectorAdapter").adapter
}

export async function registerConnectorAdapter(client: LocalIpcClient, sourcePath: string): Promise<CharioxConnectorAdapterDefinition> {
  const response = await client.send<Record<string, unknown>>(registerConnectorAdapterRequest(sourcePath))
  return expectVariant<{ adapter: CharioxConnectorAdapterDefinition }>(response, "ConnectorAdapterRegistered").adapter
}

export async function removeConnectorAdapter(client: LocalIpcClient, name: string): Promise<CharioxConnectorAdapterDefinition> {
  const response = await client.send<Record<string, unknown>>(removeConnectorAdapterRequest(name))
  return expectVariant<{ adapter: CharioxConnectorAdapterDefinition }>(response, "ConnectorAdapterRemoved").adapter
}

export async function getConnector(client: LocalIpcClient, name: string): Promise<CharioxConnectorDefinition> {
  const response = await client.send<Record<string, unknown>>(getConnectorRequest(name))
  return expectVariant<{ connector: CharioxConnectorDefinition }>(response, "Connector").connector
}

export async function registerConnector(client: LocalIpcClient, sourcePath: string): Promise<CharioxConnectorDefinition> {
  const response = await client.send<Record<string, unknown>>(registerConnectorRequest(sourcePath))
  return expectVariant<{ connector: CharioxConnectorDefinition }>(response, "ConnectorRegistered").connector
}

export async function removeConnector(client: LocalIpcClient, name: string): Promise<CharioxConnectorDefinition> {
  const response = await client.send<Record<string, unknown>>(removeConnectorRequest(name))
  return expectVariant<{ connector: CharioxConnectorDefinition }>(response, "ConnectorRemoved").connector
}

export async function testConnector(
  client: LocalIpcClient,
  name: string,
  operation: string,
  input: Record<string, unknown>,
  credential?: string | null,
  allow?: string | null,
): Promise<Record<string, unknown>> {
  const response = await client.send<Record<string, unknown>>(testConnectorRequest(name, operation, input, credential, allow))
  return expectVariant<{ execution: Record<string, unknown> }>(response, "ConnectorTested").execution
}

export async function grantAgentConnector(
  client: LocalIpcClient,
  workspaceTarget: string,
  agentRef: string,
  name: string,
  credential?: string | null,
  maxSafety?: string | null,
): Promise<AgentInstance> {
  const options: { credential?: string | null; maxSafety?: string | null } = {}
  if (credential !== undefined) options.credential = credential
  if (maxSafety !== undefined) options.maxSafety = maxSafety
  const response = await client.send<Record<string, unknown>>(
    grantAgentExtensionRequest(workspaceTarget, agentRef, "connector", name, null, options),
  )
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionGranted").agent
}

export async function revokeAgentConnector(
  client: LocalIpcClient,
  agentRef: string,
  name: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(revokeAgentExtensionRequest(agentRef, "connector", name))
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionRevoked").agent
}

export async function syncRemoteExtensionManifest(
  client: LocalIpcClient,
  agentRef: string,
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(syncRemoteExtensionManifestRequest(agentRef))
  return expectVariant<{ agent: AgentInstance }>(response, "RemoteExtensionManifestSynced").agent
}

export async function listHomeExtensionAudit(
  client: LocalIpcClient,
  agentRef: string,
  limit?: number | null,
): Promise<Record<string, unknown>[]> {
  const response = await client.send<Record<string, unknown>>(listHomeExtensionAuditRequest(agentRef, limit))
  return expectVariant<{ events: Record<string, unknown>[] }>(response, "HomeExtensionAuditListed").events
}
