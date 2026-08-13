import type {
  AgentInstance,
  CharioxConnectorAdapterDefinition,
  CharioxEnvironmentConfig,
  CharioxConnectorDefinition,
  CharioxCredentialConfig,
  CharioxMcpServerConfig,
  CharioxScriptMetadata,
  CharioxSkillMetadata,
  ExtensionKind,
  McpImportOutcome,
  ProviderCapabilityImportReport,
  SkillImportOutcome,
} from "./kernel-types.js"
import {
  getEnvironmentRequest,
  getConnectorAdapterRequest,
  getConnectorRequest,
  getCredentialRequest,
  getMcpServerRequest,
  getScriptRequest,
  getSkillRequest,
  grantAgentExtensionRequest,
  importMcpServersRequest,
  importProviderCapabilitiesRequest,
  importSkillsRequest,
  installMcpServerRequest,
  listHomeExtensionAuditRequest,
  installSkillRequest,
  listEnvironmentsRequest,
  listConnectorAdaptersRequest,
  listConnectorsRequest,
  listMcpServersRequest,
  listScriptsRequest,
  listSkillsRequest,
  registerEnvironmentRequest,
  registerConnectorAdapterRequest,
  registerConnectorRequest,
  registerScriptRequest,
  removeEnvironmentRequest,
  removeConnectorAdapterRequest,
  removeConnectorRequest,
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
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import { resolveShellAgent } from "./shell-agent-resolver.js"
import {
  formatAgentExtensionGrants,
  formatEnvironmentList,
  formatMcpImportOutcome,
  formatMcpList,
  formatHomeExtensionAuditEvents,
  formatProviderCapabilityImportReport,
  formatRemoteExtensionSyncStatus,
  formatScriptList,
  formatSkillImportOutcome,
  formatSkillList,
} from "./shell-capability-format.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellCapabilityCommandDeps = {
  client: ShellKernelClient
}

export async function executeMcpCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellCapabilityCommandDeps,
): Promise<ShellCommandResult> {
  const [action, name] = parsed.args
  if (action === "adapter" || action === "adapters") {
    const subaction = parsed.args[1]
    const adapterName = parsed.args[2]
    switch (subaction) {
      case undefined:
      case "list":
      case "ls": {
        const response = await deps.client.send(listConnectorAdaptersRequest())
        const adapters = expectVariant<{ adapters: CharioxConnectorAdapterDefinition[] }>(response, "ConnectorAdaptersListed").adapters
        return { ok: true, message: adapters.length === 0 ? "no connector adapters registered" : adapters.map(formatConnectorAdapterSummary).join("\n"), data: { adapters } }
      }
      case "show": {
        if (!adapterName) return { ok: false, message: "usage: connector adapter show <name>" }
        const response = await deps.client.send(getConnectorAdapterRequest(adapterName))
        const adapter = expectVariant<{ adapter: CharioxConnectorAdapterDefinition }>(response, "ConnectorAdapter").adapter
        return { ok: true, message: JSON.stringify(adapter, null, 2), data: { adapter }, format: "json" }
      }
      case "register": {
        if (!adapterName) return { ok: false, message: "usage: connector adapter register <adapter.yaml>" }
        const response = await deps.client.send(registerConnectorAdapterRequest(adapterName))
        const adapter = expectVariant<{ adapter: CharioxConnectorAdapterDefinition }>(response, "ConnectorAdapterRegistered").adapter
        return { ok: true, message: `registered connector adapter ${adapter.name}`, data: { adapter } }
      }
      case "remove":
      case "unregister": {
        if (!adapterName) return { ok: false, message: `usage: connector adapter ${subaction} <name>` }
        const response = await deps.client.send(removeConnectorAdapterRequest(adapterName))
        const adapter = expectVariant<{ adapter: CharioxConnectorAdapterDefinition }>(response, "ConnectorAdapterRemoved").adapter
        return { ok: true, message: `removed connector adapter ${adapter.name}`, data: { adapter } }
      }
      default:
        return { ok: false, message: "usage: connector adapter list|show|register|remove" }
    }
  }
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listMcpServersRequest(context.workspace))
      const mcps = expectVariant<{ mcps: CharioxMcpServerConfig[] }>(response, "McpServersListed").mcps
      return { ok: true, message: formatMcpList(mcps), data: { mcps } }
    }
    case "show": {
      if (!name) {
        return { ok: false, message: "usage: mcp show <name>" }
      }
      const response = await deps.client.send(getMcpServerRequest(context.workspace, name))
      const mcp = expectVariant<{ mcp: CharioxMcpServerConfig }>(response, "McpServer").mcp
      return { ok: true, message: JSON.stringify(mcp, null, 2), data: { mcp }, format: "json" }
    }
    case "install":
    case "update": {
      const config = parseMcpInstallConfig(action === "install" ? parsed.args : ["install", ...parsed.args.slice(1)])
      if (!config) {
        return { ok: false, message: `usage: mcp ${action} <name> --command <cmd> [--arg value] [--env VAR] | mcp ${action} <name> --url <url> [--bearer-token-env-var VAR]` }
      }
      const request = action === "install"
        ? installMcpServerRequest(context.workspace, config as unknown as Record<string, unknown>)
        : updateMcpServerRequest(context.workspace, config as unknown as Record<string, unknown>)
      const response = await deps.client.send(request)
      const variant = action === "install" ? "McpServerInstalled" : "McpServerUpdated"
      const mcp = expectVariant<{ mcp: CharioxMcpServerConfig }>(response, variant).mcp
      return { ok: true, message: `${action === "install" ? "installed" : "updated"} MCP ${mcp.name}`, data: { mcp } }
    }
    case "uninstall":
    case "remove": {
      if (!name) {
        return { ok: false, message: `usage: mcp ${action} <name>` }
      }
      const response = await deps.client.send(uninstallMcpServerRequest(context.workspace, name))
      const removed = expectVariant<{ name: string }>(response, "McpServerUninstalled").name
      return { ok: true, message: `uninstalled MCP ${removed}`, data: { name: removed } }
    }
    case "import": {
      const provider = name
      const importName = parsed.args[2] ?? null
      if (!provider) {
        return { ok: false, message: "usage: mcp import <codex|opencode|claude> [name]" }
      }
      const response = await deps.client.send(importMcpServersRequest(context.workspace, provider, importName))
      const outcome = expectVariant<{ outcome: McpImportOutcome }>(response, "McpServersImported").outcome
      return { ok: true, message: formatMcpImportOutcome(outcome), data: { outcome } }
    }
    case "grant":
    case "revoke": {
      const agentRef = name
      const grantName = parsed.args[2]
      if (!agentRef || !grantName) {
        return { ok: false, message: `usage: mcp ${action} <agent-ref> <name>` }
      }
      const confirmation = await confirmActiveHomeProxyGrant(parsed, context, deps, action, "mcp", agentRef, grantName)
      if (confirmation) return confirmation
      const request = action === "grant"
        ? grantAgentExtensionRequest(context.workspace, agentRef, "mcp", grantName)
        : revokeAgentExtensionRequest(agentRef, "mcp", grantName)
      const response = await deps.client.send(request)
      const variant = action === "grant" ? "AgentExtensionGranted" : "AgentExtensionRevoked"
      const agent = expectVariant<{ agent: AgentInstance }>(response, variant).agent
      return { ok: true, message: `${action === "grant" ? "granted" : "revoked"} MCP ${grantName} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, data: { agent }, contextUpdates: { agentId: agent.id } }
    }
    case "grants":
    case "agent": {
      const agent = await resolveShellAgent(context, deps, name)
      if (!agent.ok) {
        return { ok: false, message: agent.message }
      }
      return { ok: true, message: formatAgentExtensionGrants(agent.agent, "mcp"), data: { agent: agent.agent } }
    }
    default:
      return { ok: false, message: "usage: mcp list|show|install|update|uninstall|import|grant|revoke|grants" }
  }
}

export async function executeSkillCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellCapabilityCommandDeps,
): Promise<ShellCommandResult> {
  const [action, name] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listSkillsRequest(context.workspace))
      const skills = expectVariant<{ skills: CharioxSkillMetadata[] }>(response, "SkillsListed").skills
      return { ok: true, message: formatSkillList(skills), data: { skills } }
    }
    case "show": {
      if (!name) {
        return { ok: false, message: "usage: skill show <name>" }
      }
      const response = await deps.client.send(getSkillRequest(context.workspace, name))
      const skill = expectVariant<{ skill: CharioxSkillMetadata }>(response, "Skill").skill
      return { ok: true, message: JSON.stringify(skill, null, 2), data: { skill }, format: "json" }
    }
    case "install":
    case "update": {
      if (!name) {
        return { ok: false, message: `usage: skill ${action} <path>` }
      }
      const response = await deps.client.send(action === "install"
        ? installSkillRequest(context.workspace, name)
        : updateSkillRequest(context.workspace, name))
      const variant = action === "install" ? "SkillInstalled" : "SkillUpdated"
      const skill = expectVariant<{ skill: CharioxSkillMetadata }>(response, variant).skill
      return { ok: true, message: `${action === "install" ? "installed" : "updated"} skill ${skill.name}`, data: { skill } }
    }
    case "uninstall":
    case "remove": {
      if (!name) {
        return { ok: false, message: `usage: skill ${action} <name>` }
      }
      const response = await deps.client.send(uninstallSkillRequest(context.workspace, name))
      const skill = expectVariant<{ skill: CharioxSkillMetadata }>(response, "SkillUninstalled").skill
      return { ok: true, message: `uninstalled skill ${skill.name}`, data: { skill } }
    }
    case "import": {
      const provider = name
      const importName = parsed.args[2] ?? null
      if (!provider) {
        return { ok: false, message: "usage: skill import <codex|opencode|claude> [name]" }
      }
      const response = await deps.client.send(importSkillsRequest(context.workspace, provider, importName))
      const outcome = expectVariant<{ outcome: SkillImportOutcome }>(response, "SkillsImported").outcome
      return { ok: true, message: formatSkillImportOutcome(outcome), data: { outcome } }
    }
    case "grant":
    case "revoke": {
      const agentRef = name
      const grantName = parsed.args[2]
      if (!agentRef || !grantName) {
        return { ok: false, message: `usage: skill ${action} <agent-ref> <name>` }
      }
      const request = action === "grant"
        ? grantAgentExtensionRequest(context.workspace, agentRef, "skill", grantName)
        : revokeAgentExtensionRequest(agentRef, "skill", grantName)
      const response = await deps.client.send(request)
      const variant = action === "grant" ? "AgentExtensionGranted" : "AgentExtensionRevoked"
      const agent = expectVariant<{ agent: AgentInstance }>(response, variant).agent
      return { ok: true, message: `${action === "grant" ? "granted" : "revoked"} skill ${grantName} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, data: { agent }, contextUpdates: { agentId: agent.id } }
    }
    case "grants":
    case "agent": {
      const agent = await resolveShellAgent(context, deps, name)
      if (!agent.ok) {
        return { ok: false, message: agent.message }
      }
      return { ok: true, message: formatAgentExtensionGrants(agent.agent, "skill"), data: { agent: agent.agent } }
    }
    default:
      return { ok: false, message: "usage: skill list|show|install|update|uninstall|import|grant|revoke|grants" }
  }
}

export async function executeEnvironmentCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellCapabilityCommandDeps,
): Promise<ShellCommandResult> {
  const [action, name] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listEnvironmentsRequest(context.workspace))
      const environments = expectVariant<{ environments: CharioxEnvironmentConfig[] }>(response, "EnvironmentsListed").environments
      return { ok: true, message: formatEnvironmentList(environments), data: { environments } }
    }
    case "show": {
      if (!name) return { ok: false, message: "usage: env show <name>" }
      const response = await deps.client.send(getEnvironmentRequest(context.workspace, name))
      const environment = expectVariant<{ environment: CharioxEnvironmentConfig }>(response, "Environment").environment
      return { ok: true, message: JSON.stringify(environment, null, 2), data: { environment }, format: "json" }
    }
    case "register": {
      const config = parseEnvironmentConfig(parsed.args)
      if (!config) {
        return { ok: false, message: "usage: env register <name> --python <python-path> | env register <name> --node <node-path> [--package-root <dir>]" }
      }
      const response = await deps.client.send(registerEnvironmentRequest(context.workspace, config as unknown as Record<string, unknown>))
      const environment = expectVariant<{ environment: CharioxEnvironmentConfig }>(response, "EnvironmentRegistered").environment
      return { ok: true, message: `registered environment ${environment.name}`, data: { environment } }
    }
    case "remove":
    case "unregister": {
      if (!name) return { ok: false, message: `usage: env ${action} <name>` }
      const response = await deps.client.send(removeEnvironmentRequest(context.workspace, name))
      const environment = expectVariant<{ environment: CharioxEnvironmentConfig }>(response, "EnvironmentRemoved").environment
      return { ok: true, message: `removed environment ${environment.name}`, data: { environment } }
    }
    default:
      return { ok: false, message: "usage: env list|show|register|remove" }
  }
}

export async function executeScriptCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellCapabilityCommandDeps,
): Promise<ShellCommandResult> {
  const [action, name] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listScriptsRequest(context.workspace))
      const scripts = expectVariant<{ scripts: CharioxScriptMetadata[] }>(response, "ScriptsListed").scripts
      return { ok: true, message: formatScriptList(scripts), data: { scripts } }
    }
    case "show": {
      if (!name) return { ok: false, message: "usage: script show <name>" }
      const response = await deps.client.send(getScriptRequest(context.workspace, name))
      const script = expectVariant<{ script: CharioxScriptMetadata }>(response, "Script").script
      return { ok: true, message: JSON.stringify(script, null, 2), data: { script }, format: "json" }
    }
    case "validate":
    case "register": {
      const scriptArgs = parseScriptRegistrationArgs(parsed.args)
      if (!scriptArgs) {
        return { ok: false, message: `usage: script ${action} <path> --env <environment> [--name <name>]` }
      }
      const request = action === "validate"
        ? validateScriptRequest(context.workspace, scriptArgs.sourcePath, scriptArgs.environment, scriptArgs.name)
        : registerScriptRequest(context.workspace, scriptArgs.sourcePath, scriptArgs.environment, scriptArgs.name)
      const response = await deps.client.send(request)
      const variant = action === "validate" ? "ScriptValidated" : "ScriptRegistered"
      const script = expectVariant<{ script: CharioxScriptMetadata }>(response, variant).script
      return { ok: true, message: `${action === "validate" ? "validated" : "registered"} script ${script.name}`, data: { script } }
    }
    case "remove":
    case "unregister": {
      if (!name) return { ok: false, message: `usage: script ${action} <name>` }
      const response = await deps.client.send(removeScriptRequest(context.workspace, name))
      const script = expectVariant<{ script: CharioxScriptMetadata }>(response, "ScriptRemoved").script
      return { ok: true, message: `removed script ${script.name}`, data: { script } }
    }
    case "grant":
    case "revoke": {
      const agentRef = name
      const scriptName = parsed.args[2]
      const environment = readOption(parsed.args, "--env")
      if (!agentRef || !scriptName || (action === "grant" && !environment)) {
        return { ok: false, message: `usage: script ${action} <agent-ref> <name>${action === "grant" ? " --env <environment>" : ""}` }
      }
      const confirmation = await confirmActiveHomeProxyGrant(parsed, context, deps, action, "script", agentRef, scriptName)
      if (confirmation) return confirmation
      const request = action === "grant"
        ? grantAgentExtensionRequest(context.workspace, agentRef, "script", scriptName, environment)
        : revokeAgentExtensionRequest(agentRef, "script", scriptName)
      const response = await deps.client.send(request)
      const variant = action === "grant" ? "AgentExtensionGranted" : "AgentExtensionRevoked"
      const agent = expectVariant<{ agent: AgentInstance }>(response, variant).agent
      return { ok: true, message: `${action === "grant" ? "granted" : "revoked"} script ${scriptName} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, data: { agent }, contextUpdates: { agentId: agent.id } }
    }
    case "grants":
    case "agent": {
      const agent = await resolveShellAgent(context, deps, name)
      if (!agent.ok) return { ok: false, message: agent.message }
      return { ok: true, message: formatAgentExtensionGrants(agent.agent, "script"), data: { agent: agent.agent } }
    }
    default:
      return { ok: false, message: "usage: script list|show|validate|register|remove|grant|revoke|grants" }
  }
}

export async function executeConnectorCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellCapabilityCommandDeps,
): Promise<ShellCommandResult> {
  const [action, name] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listConnectorsRequest())
      const connectors = expectVariant<{ connectors: CharioxConnectorDefinition[] }>(response, "ConnectorsListed").connectors
      return { ok: true, message: connectors.length === 0 ? "no connectors registered" : connectors.map(formatConnectorSummary).join("\n"), data: { connectors } }
    }
    case "show": {
      if (!name) return { ok: false, message: "usage: connector show <name>" }
      const response = await deps.client.send(getConnectorRequest(name))
      const connector = expectVariant<{ connector: CharioxConnectorDefinition }>(response, "Connector").connector
      return { ok: true, message: JSON.stringify(connector, null, 2), data: { connector }, format: "json" }
    }
    case "register": {
      if (!name) return { ok: false, message: "usage: connector register <file.yaml>" }
      const response = await deps.client.send(registerConnectorRequest(name))
      const connector = expectVariant<{ connector: CharioxConnectorDefinition }>(response, "ConnectorRegistered").connector
      return { ok: true, message: `registered connector ${connector.name}`, data: { connector } }
    }
    case "remove":
    case "unregister": {
      if (!name) return { ok: false, message: `usage: connector ${action} <name>` }
      const response = await deps.client.send(removeConnectorRequest(name))
      const connector = expectVariant<{ connector: CharioxConnectorDefinition }>(response, "ConnectorRemoved").connector
      return { ok: true, message: `removed connector ${connector.name}`, data: { connector } }
    }
    case "test": {
      const operation = parsed.args[2]
      const inputText = readOption(parsed.args, "--input") ?? "{}"
      if (!name || !operation) return { ok: false, message: "usage: connector test <name> <operation> [--credential <id>] [--allow read|write|destructive] --input '<json>'" }
      const response = await deps.client.send(testConnectorRequest(name, operation, JSON.parse(inputText), readOption(parsed.args, "--credential"), readOption(parsed.args, "--allow")))
      const execution = expectVariant<{ execution: Record<string, unknown> }>(response, "ConnectorTested").execution
      return { ok: true, message: JSON.stringify(execution, null, 2), data: { execution }, format: "json" }
    }
    case "doctor": {
      if (!name) return { ok: false, message: "usage: connector doctor <name> [--credential <id>]" }
      const connectorResponse = await deps.client.send(getConnectorRequest(name))
      const connector = expectVariant<{ connector: CharioxConnectorDefinition }>(connectorResponse, "Connector").connector
      const credentialId = readOption(parsed.args, "--credential")
      let credential: CharioxCredentialConfig | null = null
      if (credentialId) {
        const credentialResponse = await deps.client.send(getCredentialRequest(credentialId))
        credential = expectVariant<{ credential: CharioxCredentialConfig }>(credentialResponse, "Credential").credential
      }
      return { ok: true, message: formatConnectorDoctor(connector, credentialId, credential), data: { connector, credential } }
    }
    case "grant":
    case "revoke": {
      const agentRef = name
      const connectorName = parsed.args[2]
      if (!agentRef || !connectorName) return { ok: false, message: `usage: connector ${action} <agent-ref> <name>` }
      const confirmation = await confirmActiveHomeProxyGrant(parsed, context, deps, action, "connector", agentRef, connectorName)
      if (confirmation) return confirmation
      const request = action === "grant"
        ? grantAgentExtensionRequest(context.workspace, agentRef, "connector", connectorName, null, {
          credential: readOption(parsed.args, "--credential"),
          maxSafety: readOption(parsed.args, "--allow"),
        })
        : revokeAgentExtensionRequest(agentRef, "connector", connectorName)
      const response = await deps.client.send(request)
      const variant = action === "grant" ? "AgentExtensionGranted" : "AgentExtensionRevoked"
      const agent = expectVariant<{ agent: AgentInstance }>(response, variant).agent
      return { ok: true, message: `${action === "grant" ? "granted" : "revoked"} connector ${connectorName} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, data: { agent }, contextUpdates: { agentId: agent.id } }
    }
    case "grants":
    case "agent": {
      const agent = await resolveShellAgent(context, deps, name)
      if (!agent.ok) return { ok: false, message: agent.message }
      return { ok: true, message: formatAgentExtensionGrants(agent.agent, "connector"), data: { agent: agent.agent } }
    }
    default:
      return { ok: false, message: "usage: connector list|show|register|adapter|remove|doctor|test|grant|revoke|grants" }
  }
}

export async function executeExtensionCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellCapabilityCommandDeps,
): Promise<ShellCommandResult> {
  const [action, kind, agentRef, name] = parsed.args
  if (action === "import") {
    if (kind !== "providers") {
      return { ok: false, message: "usage: extension import providers [--provider codex|opencode|claude] [--kind all|mcp|skill] [--name <capability>] [--dry-run]" }
    }
    const response = await deps.client.send(importProviderCapabilitiesRequest(context.workspace, {
      providers: readRepeatedOption(parsed.args, "--provider"),
      kind: readOption(parsed.args, "--kind"),
      name: readOption(parsed.args, "--name"),
      dryRun: parsed.args.includes("--dry-run"),
    }))
    const report = expectVariant<{ report: ProviderCapabilityImportReport }>(response, "ProviderCapabilitiesImported").report
    return { ok: true, message: formatProviderCapabilityImportReport(report), data: { report } }
  }
  if (action === "sync-status") {
    const agent = await resolveShellAgent(context, deps, kind)
    if (!agent.ok) return { ok: false, message: agent.message }
    return { ok: true, message: formatRemoteExtensionSyncStatus(agent.agent), data: { agent: agent.agent } }
  }
  if (action === "sync-retry") {
    if (!kind) return { ok: false, message: "usage: extension sync-retry <agent-ref>" }
    const response = await deps.client.send(syncRemoteExtensionManifestRequest(kind))
    const agent = expectVariant<{ agent: AgentInstance }>(response, "RemoteExtensionManifestSynced").agent
    return { ok: true, message: formatRemoteExtensionSyncStatus(agent), data: { agent }, contextUpdates: { agentId: agent.id } }
  }
  if (action === "audit") {
    if (!kind) return { ok: false, message: "usage: extension audit <agent-ref> [--limit <count>]" }
    const response = await deps.client.send(listHomeExtensionAuditRequest(kind, readNumberOption(parsed.args, "--limit")))
    const events = expectVariant<{ events: Record<string, unknown>[] }>(response, "HomeExtensionAuditListed").events
    return { ok: true, message: formatHomeExtensionAuditEvents(events), data: { events } }
  }
  if (action !== "grant" && action !== "revoke" && action !== "grants") {
    return { ok: false, message: "usage: extension grant|revoke <mcp|skill|script|connector> <agent-ref> <name> [--env <environment>] [--credential <id>] [--allow read|write|destructive] | extension grants <kind> [agent-ref] | extension import providers [--provider codex|opencode|claude] [--kind all|mcp|skill] [--name <capability>] [--dry-run] | extension sync-status|sync-retry|audit <agent-ref>" }
  }
  if (!isExtensionKind(kind)) {
    return { ok: false, message: "extension kind must be mcp, skill, script, or connector" }
  }
  if (action === "grants") {
    const agent = await resolveShellAgent(context, deps, agentRef)
    if (!agent.ok) return { ok: false, message: agent.message }
    return { ok: true, message: formatAgentExtensionGrants(agent.agent, kind), data: { agent: agent.agent } }
  }
  if (!agentRef || !name) {
    return { ok: false, message: `usage: extension ${action} <mcp|skill|script|connector> <agent-ref> <name> [--env <environment>]` }
  }
  const environment = readOption(parsed.args, "--env")
  if (action === "grant" && kind === "script" && !environment) {
    return { ok: false, message: "usage: extension grant script <agent-ref> <name> --env <environment>" }
  }
  const confirmation = await confirmActiveHomeProxyGrant(parsed, context, deps, action, kind, agentRef, name)
  if (confirmation) return confirmation
  const request = action === "grant"
    ? grantAgentExtensionRequest(context.workspace, agentRef, kind, name, environment, {
      credential: readOption(parsed.args, "--credential"),
      maxSafety: readOption(parsed.args, "--allow"),
    })
    : revokeAgentExtensionRequest(agentRef, kind, name)
  const response = await deps.client.send(request)
  const variant = action === "grant" ? "AgentExtensionGranted" : "AgentExtensionRevoked"
  const agent = expectVariant<{ agent: AgentInstance }>(response, variant).agent
  return { ok: true, message: `${action === "grant" ? "granted" : "revoked"} ${kind} ${name} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, data: { agent }, contextUpdates: { agentId: agent.id } }
}

function parseMcpInstallConfig(args: string[]): CharioxMcpServerConfig | null {
  const name = args[1]
  if (!name) return null
  let command: string | null = null
  let url: string | null = null
  const mcpArgs: string[] = []
  const envVars: string[] = []
  let bearerTokenEnvVar: string | null = null
  for (let index = 2; index < args.length; index += 1) {
    const arg = args[index]
    const next = args[index + 1]
    if (arg === "--command" && next) {
      command = next
      index += 1
    } else if (arg === "--arg" && next) {
      mcpArgs.push(next)
      index += 1
    } else if (arg === "--env" && next) {
      envVars.push(next)
      index += 1
    } else if (arg === "--url" && next) {
      url = next
      index += 1
    } else if (arg === "--bearer-token-env-var" && next) {
      bearerTokenEnvVar = next
      index += 1
    } else {
      return null
    }
  }
  if (command && !url) {
    return {
      name,
      transport: { type: "stdio", command, args: mcpArgs, env: {}, env_vars: envVars },
      enabled: true,
      required: false,
    }
  }
  if (url && !command) {
    return {
      name,
      transport: {
        type: "streamable_http",
        url,
        bearer_token_env_var: bearerTokenEnvVar,
        http_headers: {},
        env_http_headers: {},
      },
      enabled: true,
      required: false,
    }
  }
  return null
}

function parseEnvironmentConfig(args: string[]): CharioxEnvironmentConfig | null {
  const name = args[1]
  if (!name) return null
  const python = readOption(args, "--python")
  const node = readOption(args, "--node")
  const packageRoot = readOption(args, "--package-root")
  if (python && !node) {
    return { name, runtime: { type: "python", python } }
  }
  if (node && !python) {
    return { name, runtime: { type: "node", node, ...(packageRoot ? { package_root: packageRoot } : {}) } }
  }
  return null
}

function parseScriptRegistrationArgs(args: string[]): { sourcePath: string; environment: string; name?: string | null } | null {
  const sourcePath = args[1]
  const environment = readOption(args, "--env")
  if (!sourcePath || !environment) return null
  return {
    sourcePath,
    environment,
    name: readOption(args, "--name"),
  }
}

function readOption(args: string[], flag: string): string | null {
  const index = args.indexOf(flag)
  if (index === -1) return null
  const value = args[index + 1]
  return value && !value.startsWith("--") ? value : null
}

function readRepeatedOption(args: string[], flag: string): string[] {
  const values: string[] = []
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === flag) {
      const value = args[index + 1]
      if (value && !value.startsWith("--")) {
        values.push(value)
        index += 1
      }
    }
  }
  return values
}

async function confirmActiveHomeProxyGrant(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellCapabilityCommandDeps,
  action: "grant" | "revoke",
  kind: ExtensionKind,
  agentRef: string,
  name: string,
): Promise<ShellCommandResult | null> {
  if (action !== "grant" || kind === "skill") {
    return null
  }
  const agent = await resolveShellAgent(context, deps, agentRef)
  if (!agent.ok) {
    return { ok: false, message: agent.message }
  }
  if (!agent.agent.remote_execution) {
    return null
  }
  if (parsed.args.includes("--confirm-home-proxy")) {
    return null
  }
  const command = parsed.normalized.includes("--confirm-home-proxy")
    ? parsed.normalized
    : `${parsed.normalized} --confirm-home-proxy`
  return {
    ok: false,
    message: [
      `Confirm exposing ${kind} ${name} to remote agent ${agent.agent.agent_ref}; home keeps credentials local and executes calls on this machine.`,
      `rerun: ${command}`,
    ].join("\n"),
    data: { agent: agent.agent },
  }
}

function readNumberOption(args: string[], flag: string): number | null {
  const value = readOption(args, flag)
  if (!value) return null
  const parsed = Number(value)
  return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : null
}

function isExtensionKind(value: string | undefined): value is ExtensionKind {
  return value === "mcp" || value === "skill" || value === "script" || value === "connector"
}

function formatConnectorSummary(connector: CharioxConnectorDefinition): string {
  const operations = Array.isArray(connector.operations) ? connector.operations.length : 0
  return `${connector.name} [${connector.adapter}, ${operations} op${operations === 1 ? "" : "s"}] - ${connector.description}`
}

function formatConnectorAdapterSummary(adapter: CharioxConnectorAdapterDefinition): string {
  return `${adapter.name} - ${adapter.description ?? adapter.adapter_protocol}`
}

function formatConnectorDoctor(
  connector: CharioxConnectorDefinition,
  credentialId: string | null,
  credential: CharioxCredentialConfig | null,
): string {
  const findings: string[] = []
  const ok = (message: string) => findings.push(`ok: ${message}`)
  const warn = (message: string) => findings.push(`warn: ${message}`)
  if (connector.kind !== "connector") warn("kind is not connector")
  if (connector.adapter) ok(`adapter ${connector.adapter}`)
  const operationCount = Array.isArray(connector.operations) ? connector.operations.length : 0
  if (operationCount > 0) ok(`${operationCount} operation${operationCount === 1 ? "" : "s"} configured`)
  else warn("no operations configured")
  if (connector.timeout_ms && connector.timeout_ms > 0) ok(`timeout ${connector.timeout_ms}ms`)
  if (connector.max_response_bytes && connector.max_response_bytes > 0) ok(`response cap ${connector.max_response_bytes} bytes`)
  const requiresCredential = connector.credential?.required === true
  if (requiresCredential && !credentialId) warn("connector requires a credential; pass --credential <id>")
  if (credentialId && !credential) warn(`credential ${credentialId} could not be loaded`)
  if (credential) {
    const uses = credential.allowed_uses ?? []
    if (uses.length === 0 || uses.includes("connector")) ok(`credential ${credential.id} allows connector`)
    else warn(`credential ${credential.id} does not allow connector`)
    const injectionKind = typeof credential.injection?.kind === "string" ? credential.injection.kind : "unknown"
    if (injectionKind === "pty") warn(`credential ${credential.id} is configured for terminal injection`)
    else ok(`credential ${credential.id} injection is ${injectionKind}`)
  }
  return [`${connector.name} connector doctor`, ...findings].join("\n")
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
