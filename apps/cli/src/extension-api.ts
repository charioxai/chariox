import type {
  AgentExtensionCatalog,
  AgentInstance,
  ArrobaConnectorAdapterDefinition,
  ArrobaConnectorDefinition,
  ArrobaCredentialConfig,
  ArrobaEnvironmentConfig,
  ArrobaMcpServerConfig,
  ArrobaScriptMetadata,
  ArrobaSkillMetadata,
  ExtensionCatalogSource,
  ExtensionSource,
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
  listAgentExtensionCatalogRequest,
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
  source: ExtensionSource = "home",
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(
    grantAgentExtensionRequest(workspaceTarget, agentRef, "mcp", name, null, { source }),
  )
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionGranted").agent
}

export async function revokeAgentMcp(
  client: LocalIpcClient,
  agentRef: string,
  name: string,
  source: ExtensionSource = "home",
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(revokeAgentExtensionRequest(agentRef, "mcp", name, source))
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
  source: ExtensionSource = "home",
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(
    grantAgentExtensionRequest(workspaceTarget, agentRef, "skill", name, null, { source }),
  )
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionGranted").agent
}

export async function revokeAgentSkill(
  client: LocalIpcClient,
  agentRef: string,
  name: string,
  source: ExtensionSource = "home",
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(revokeAgentExtensionRequest(agentRef, "skill", name, source))
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
  source: ExtensionSource = "home",
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(
    grantAgentExtensionRequest(workspaceTarget, agentRef, "script", name, environment, { source }),
  )
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionGranted").agent
}

export async function revokeAgentScript(
  client: LocalIpcClient,
  agentRef: string,
  name: string,
  source: ExtensionSource = "home",
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(revokeAgentExtensionRequest(agentRef, "script", name, source))
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionRevoked").agent
}

export async function listCredentials(client: LocalIpcClient): Promise<ArrobaCredentialConfig[]> {
  const response = await client.send<Record<string, unknown>>(listCredentialsRequest())
  return expectVariant<{ credentials: ArrobaCredentialConfig[] }>(response, "CredentialsListed").credentials
}

export async function getCredential(client: LocalIpcClient, id: string): Promise<ArrobaCredentialConfig> {
  const response = await client.send<Record<string, unknown>>(getCredentialRequest(id))
  return expectVariant<{ credential: ArrobaCredentialConfig }>(response, "Credential").credential
}

export async function registerCredential(client: LocalIpcClient, sourcePath: string): Promise<ArrobaCredentialConfig> {
  const response = await client.send<Record<string, unknown>>(registerCredentialRequest(sourcePath))
  return expectVariant<{ credential: ArrobaCredentialConfig }>(response, "CredentialRegistered").credential
}

export async function removeCredential(client: LocalIpcClient, id: string): Promise<ArrobaCredentialConfig> {
  const response = await client.send<Record<string, unknown>>(removeCredentialRequest(id))
  return expectVariant<{ credential: ArrobaCredentialConfig }>(response, "CredentialRemoved").credential
}

export async function listConnectors(client: LocalIpcClient): Promise<ArrobaConnectorDefinition[]> {
  const response = await client.send<Record<string, unknown>>(listConnectorsRequest())
  return expectVariant<{ connectors: ArrobaConnectorDefinition[] }>(response, "ConnectorsListed").connectors
}

export async function listConnectorAdapters(client: LocalIpcClient): Promise<ArrobaConnectorAdapterDefinition[]> {
  const response = await client.send<Record<string, unknown>>(listConnectorAdaptersRequest())
  return expectVariant<{ adapters: ArrobaConnectorAdapterDefinition[] }>(response, "ConnectorAdaptersListed").adapters
}

export async function getConnectorAdapter(client: LocalIpcClient, name: string): Promise<ArrobaConnectorAdapterDefinition> {
  const response = await client.send<Record<string, unknown>>(getConnectorAdapterRequest(name))
  return expectVariant<{ adapter: ArrobaConnectorAdapterDefinition }>(response, "ConnectorAdapter").adapter
}

export async function registerConnectorAdapter(client: LocalIpcClient, sourcePath: string): Promise<ArrobaConnectorAdapterDefinition> {
  const response = await client.send<Record<string, unknown>>(registerConnectorAdapterRequest(sourcePath))
  return expectVariant<{ adapter: ArrobaConnectorAdapterDefinition }>(response, "ConnectorAdapterRegistered").adapter
}

export async function removeConnectorAdapter(client: LocalIpcClient, name: string): Promise<ArrobaConnectorAdapterDefinition> {
  const response = await client.send<Record<string, unknown>>(removeConnectorAdapterRequest(name))
  return expectVariant<{ adapter: ArrobaConnectorAdapterDefinition }>(response, "ConnectorAdapterRemoved").adapter
}

export async function getConnector(client: LocalIpcClient, name: string): Promise<ArrobaConnectorDefinition> {
  const response = await client.send<Record<string, unknown>>(getConnectorRequest(name))
  return expectVariant<{ connector: ArrobaConnectorDefinition }>(response, "Connector").connector
}

export async function registerConnector(client: LocalIpcClient, sourcePath: string): Promise<ArrobaConnectorDefinition> {
  const response = await client.send<Record<string, unknown>>(registerConnectorRequest(sourcePath))
  return expectVariant<{ connector: ArrobaConnectorDefinition }>(response, "ConnectorRegistered").connector
}

export async function removeConnector(client: LocalIpcClient, name: string): Promise<ArrobaConnectorDefinition> {
  const response = await client.send<Record<string, unknown>>(removeConnectorRequest(name))
  return expectVariant<{ connector: ArrobaConnectorDefinition }>(response, "ConnectorRemoved").connector
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
  source: ExtensionSource = "home",
): Promise<AgentInstance> {
  const options: { credential?: string | null; maxSafety?: string | null; source: ExtensionSource } = { source }
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
  source: ExtensionSource = "home",
): Promise<AgentInstance> {
  const response = await client.send<Record<string, unknown>>(revokeAgentExtensionRequest(agentRef, "connector", name, source))
  return expectVariant<{ agent: AgentInstance }>(response, "AgentExtensionRevoked").agent
}

export async function listAgentExtensionCatalog(
  client: LocalIpcClient,
  agentRef: string,
  source: ExtensionCatalogSource = "all",
): Promise<AgentExtensionCatalog> {
  const response = await client.send<Record<string, unknown>>(listAgentExtensionCatalogRequest(agentRef, source))
  return expectVariant<{ catalog: AgentExtensionCatalog }>(response, "AgentExtensionCatalogListed").catalog
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
