import type {
  AgentInstance,
  ArrobaConnectorAdapterDefinition,
  ArrobaConnectorDefinition,
  ArrobaCredentialConfig,
  ArrobaEnvironmentConfig,
  ArrobaMcpServerConfig,
  ArrobaScriptMetadata,
  ArrobaSkillMetadata,
  McpImportOutcome,
  SkillImportOutcome,
  ExtensionKind,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { ResolvedAgentReference } from "./session-agent-resolver.js"

type FooterTone = "info" | "error"

export type CapabilityCommandHandlerDeps = {
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  listMcpServers?: () => Promise<ArrobaMcpServerConfig[]>
  installMcpServer?: (config: ArrobaMcpServerConfig) => Promise<ArrobaMcpServerConfig>
  updateMcpServer?: (config: ArrobaMcpServerConfig) => Promise<ArrobaMcpServerConfig>
  uninstallMcpServer?: (name: string) => Promise<string>
  importMcpServers?: (provider: string, name?: string | null) => Promise<McpImportOutcome>
  getMcpServer?: (name: string) => Promise<ArrobaMcpServerConfig>
  grantAgentMcp?: (agentRef: string, name: string) => Promise<AgentInstance>
  revokeAgentMcp?: (agentRef: string, name: string) => Promise<AgentInstance>
  listSkills?: () => Promise<ArrobaSkillMetadata[]>
  installSkill?: (sourcePath: string) => Promise<ArrobaSkillMetadata>
  updateSkill?: (sourcePath: string) => Promise<ArrobaSkillMetadata>
  uninstallSkill?: (name: string) => Promise<ArrobaSkillMetadata>
  importSkills?: (provider: string, name?: string | null) => Promise<SkillImportOutcome>
  getSkill?: (name: string) => Promise<ArrobaSkillMetadata>
  grantAgentSkill?: (agentRef: string, name: string) => Promise<AgentInstance>
  revokeAgentSkill?: (agentRef: string, name: string) => Promise<AgentInstance>
  listEnvironments?: () => Promise<ArrobaEnvironmentConfig[]>
  getEnvironment?: (name: string) => Promise<ArrobaEnvironmentConfig>
  registerEnvironment?: (config: ArrobaEnvironmentConfig) => Promise<ArrobaEnvironmentConfig>
  removeEnvironment?: (name: string) => Promise<ArrobaEnvironmentConfig>
  listScripts?: () => Promise<ArrobaScriptMetadata[]>
  getScript?: (name: string) => Promise<ArrobaScriptMetadata>
  validateScript?: (sourcePath: string, environment: string, name?: string | null) => Promise<ArrobaScriptMetadata>
  registerScript?: (sourcePath: string, environment: string, name?: string | null) => Promise<ArrobaScriptMetadata>
  removeScript?: (name: string) => Promise<ArrobaScriptMetadata>
  grantAgentScript?: (agentRef: string, name: string, environment: string) => Promise<AgentInstance>
  revokeAgentScript?: (agentRef: string, name: string) => Promise<AgentInstance>
  listCredentials?: () => Promise<ArrobaCredentialConfig[]>
  getCredential?: (id: string) => Promise<ArrobaCredentialConfig>
  setCredentialSecret?: (key: string, value: string) => Promise<string>
  readSecret?: (prompt: string) => Promise<string>
  registerCredential?: (sourcePath: string) => Promise<ArrobaCredentialConfig>
  removeCredential?: (id: string) => Promise<ArrobaCredentialConfig>
  listConnectors?: () => Promise<ArrobaConnectorDefinition[]>
  getConnector?: (name: string) => Promise<ArrobaConnectorDefinition>
  registerConnector?: (sourcePath: string) => Promise<ArrobaConnectorDefinition>
  removeConnector?: (name: string) => Promise<ArrobaConnectorDefinition>
  listConnectorAdapters?: () => Promise<ArrobaConnectorAdapterDefinition[]>
  getConnectorAdapter?: (name: string) => Promise<ArrobaConnectorAdapterDefinition>
  registerConnectorAdapter?: (sourcePath: string) => Promise<ArrobaConnectorAdapterDefinition>
  removeConnectorAdapter?: (name: string) => Promise<ArrobaConnectorAdapterDefinition>
  testConnector?: (name: string, operation: string, input: Record<string, unknown>, credential?: string | null, allow?: string | null) => Promise<Record<string, unknown>>
  grantAgentConnector?: (agentRef: string, name: string, credential?: string | null, maxSafety?: string | null) => Promise<AgentInstance>
  revokeAgentConnector?: (agentRef: string, name: string) => Promise<AgentInstance>
  syncRemoteExtensionManifest?: (agentRef: string) => Promise<AgentInstance>
  listHomeExtensionAudit?: (agentRef: string, limit?: number | null) => Promise<Record<string, unknown>[]>
}

export const parseMcpInstallConfig = (args: string[]): ArrobaMcpServerConfig | null => {
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

function parseEnvironmentConfig(args: string[]): ArrobaEnvironmentConfig | null {
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
  return { sourcePath, environment, name: readOption(args, "--name") }
}

function readOption(args: string[], flag: string): string | null {
  const index = args.indexOf(flag)
  if (index === -1) return null
  const value = args[index + 1]
  return value && !value.startsWith("--") ? value : null
}

export function formatAgentCapabilityGrants(agent: AgentInstance, kind: ExtensionKind): string {
  const grants = (agent.extension_grants ?? []).filter((grant) => grant.kind === kind)
  const label = kind === "mcp" ? "MCP" : kind
  const agentLabel = `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
  if (grants.length === 0) {
    return `${agentLabel} has no ${label} grants.`
  }
  const placement = agent.remote_execution
    ? kind === "skill" ? "skill snapshot" : "home-proxy"
    : "worker-local"
  const sync = agent.remote_execution
    ? `\n\nremote extension sync: ${formatRemoteExtensionSyncStatusLine(agent.remote_extension_manifest_sync)}`
    : ""
  return `${agentLabel} ${label} grants:\n${grants.map((grant) => {
    const parts = [
      placement,
      grant.environment ? `env=${grant.environment}` : null,
      grant.credential ? `credential=${grant.credential}` : null,
      grant.max_safety ? `allow=${grant.max_safety}` : null,
    ].filter(Boolean)
    const suffix = parts.length > 0 ? ` (${parts.join(", ")})` : ""
    return `- ${grant.name}${suffix}`
  }).join("\n")}${sync}`
}

export async function handleMcpSlashCommand(
  deps: CapabilityCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "mcp" }>,
): Promise<void> {
  const [action] = command.args
  if (!action || action === "list" || action === "ls") {
    if (!deps.listMcpServers) {
      deps.flashFooter("MCP registry is not available in this daemon", "error")
      return
    }
    const mcps = await deps.listMcpServers()
    deps.appendNotice(mcps.length === 0 ? "No Arroba-managed MCPs installed." : mcps.map(formatMcpSummary).join("\n"))
    deps.flashFooter(`listed ${mcps.length} MCP${mcps.length === 1 ? "" : "s"}`, "info")
    return
  }
  if (action === "show") {
    const name = command.args[1]
    if (!name || !deps.getMcpServer) {
      deps.flashFooter("usage: /mcp show <name>", "error")
      return
    }
    const mcp = await deps.getMcpServer(name)
    deps.appendNotice(formatMcpDetails(mcp))
    deps.flashFooter(`showing MCP ${mcp.name}`, "info")
    return
  }
  if (action === "install") {
    if (!deps.installMcpServer) {
      deps.flashFooter("MCP install is not available in this daemon", "error")
      return
    }
    const config = parseMcpInstallConfig(command.args)
    if (!config) {
      deps.flashFooter("usage: /mcp install <name> --command <cmd> [--arg value] [--env VAR] | /mcp install <name> --url <url> [--bearer-token-env-var VAR]", "error")
      return
    }
    const mcp = await deps.installMcpServer(config)
    deps.flashFooter(`installed MCP ${mcp.name}`, "info")
    return
  }
  if (action === "update") {
    if (!deps.updateMcpServer) {
      deps.flashFooter("MCP update is not available in this daemon", "error")
      return
    }
    const config = parseMcpInstallConfig(["install", ...command.args.slice(1)])
    if (!config) {
      deps.flashFooter("usage: /mcp update <name> --command <cmd> [--arg value] [--env VAR] | /mcp update <name> --url <url> [--bearer-token-env-var VAR]", "error")
      return
    }
    const mcp = await deps.updateMcpServer(config)
    deps.flashFooter(`updated MCP ${mcp.name}`, "info")
    return
  }
  if (action === "uninstall" || action === "remove") {
    const name = command.args[1]
    if (!name || !deps.uninstallMcpServer) {
      deps.flashFooter(`usage: /mcp ${action} <name>`, "error")
      return
    }
    const removedName = await deps.uninstallMcpServer(name)
    deps.flashFooter(`uninstalled MCP ${removedName}`, "info")
    return
  }
  if (action === "import") {
    const provider = command.args[1]
    const name = command.args[2] ?? null
    if (!provider || !deps.importMcpServers) {
      deps.flashFooter("usage: /mcp import <codex|opencode> [name]", "error")
      return
    }
    const outcome = await deps.importMcpServers(provider, name)
    deps.appendNotice(formatMcpImportOutcome(outcome))
    deps.flashFooter(`imported ${outcome.imported.length} MCP${outcome.imported.length === 1 ? "" : "s"} from ${provider}`, "info")
    return
  }
  if (action === "grant" || action === "revoke") {
    const agentRef = command.args[1]
    const name = command.args[2]
    const handler = action === "grant" ? deps.grantAgentMcp : deps.revokeAgentMcp
    if (!agentRef || !name || !handler) {
      deps.flashFooter(`usage: /mcp ${action} <agent-ref> <name>`, "error")
      return
    }
    const agent = await handler(agentRef, name)
    deps.flashFooter(`${action === "grant" ? "granted" : "revoked"} MCP ${name} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, "info")
    return
  }
  if (action === "grants" || action === "agent") {
    const agent = resolveGrantTarget(deps, command.args[1], `usage: /mcp ${action} <agent-ref>`)
    if (!agent) return
    deps.appendNotice(formatAgentCapabilityGrants(agent, "mcp"))
    deps.flashFooter(`showing MCP grants for ${agent.agent_ref}`, "info")
    return
  }
  deps.flashFooter("usage: /mcp list | /mcp show <name> | /mcp install ... | /mcp update ... | /mcp uninstall <name> | /mcp import <codex|opencode> [name] | /mcp grant <agent-ref> <name> | /mcp revoke <agent-ref> <name> | /mcp grants <agent-ref>", "error")
}

export async function handleSkillSlashCommand(
  deps: CapabilityCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "skill" }>,
): Promise<void> {
  const [action] = command.args
  if (!action || action === "list" || action === "ls") {
    if (!deps.listSkills) {
      deps.flashFooter("skill registry is not available in this daemon", "error")
      return
    }
    const skills = await deps.listSkills()
    deps.appendNotice(skills.length === 0 ? "No Arroba-managed skills installed." : skills.map(formatSkillSummary).join("\n"))
    deps.flashFooter(`listed ${skills.length} skill${skills.length === 1 ? "" : "s"}`, "info")
    return
  }
  if (action === "show") {
    const name = command.args[1]
    if (!name || !deps.getSkill) {
      deps.flashFooter("usage: /skill show <name>", "error")
      return
    }
    const skill = await deps.getSkill(name)
    deps.appendNotice(formatSkillDetails(skill))
    deps.flashFooter(`showing skill ${skill.name}`, "info")
    return
  }
  if (action === "install") {
    const sourcePath = command.args[1]
    if (!sourcePath || !deps.installSkill) {
      deps.flashFooter("usage: /skill install <path>", "error")
      return
    }
    const skill = await deps.installSkill(sourcePath)
    deps.flashFooter(`installed skill ${skill.name}`, "info")
    return
  }
  if (action === "update") {
    const sourcePath = command.args[1]
    if (!sourcePath || !deps.updateSkill) {
      deps.flashFooter("usage: /skill update <path>", "error")
      return
    }
    const skill = await deps.updateSkill(sourcePath)
    deps.flashFooter(`updated skill ${skill.name}`, "info")
    return
  }
  if (action === "uninstall" || action === "remove") {
    const name = command.args[1]
    if (!name || !deps.uninstallSkill) {
      deps.flashFooter(`usage: /skill ${action} <name>`, "error")
      return
    }
    const skill = await deps.uninstallSkill(name)
    deps.flashFooter(`uninstalled skill ${skill.name}`, "info")
    return
  }
  if (action === "import") {
    const provider = command.args[1]
    const name = command.args[2] ?? null
    if (!provider || !deps.importSkills) {
      deps.flashFooter("usage: /skill import <codex|opencode> [name]", "error")
      return
    }
    const outcome = await deps.importSkills(provider, name)
    deps.appendNotice(formatSkillImportOutcome(outcome))
    deps.flashFooter(`imported ${outcome.imported.length} skill${outcome.imported.length === 1 ? "" : "s"} from ${provider}`, "info")
    return
  }
  if (action === "grant" || action === "revoke") {
    const agentRef = command.args[1]
    const name = command.args[2]
    const handler = action === "grant" ? deps.grantAgentSkill : deps.revokeAgentSkill
    if (!agentRef || !name || !handler) {
      deps.flashFooter(`usage: /skill ${action} <agent-ref> <name>`, "error")
      return
    }
    const agent = await handler(agentRef, name)
    deps.flashFooter(`${action === "grant" ? "granted" : "revoked"} skill ${name} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, "info")
    return
  }
  if (action === "grants" || action === "agent") {
    const agent = resolveGrantTarget(deps, command.args[1], `usage: /skill ${action} <agent-ref>`)
    if (!agent) return
    deps.appendNotice(formatAgentCapabilityGrants(agent, "skill"))
    deps.flashFooter(`showing skill grants for ${agent.agent_ref}`, "info")
    return
  }
  deps.flashFooter("usage: /skill list | /skill show <name> | /skill install <path> | /skill update <path> | /skill uninstall <name> | /skill import <codex|opencode> [name] | /skill grant <agent-ref> <name> | /skill revoke <agent-ref> <name> | /skill grants <agent-ref>", "error")
}

export async function handleEnvironmentSlashCommand(
  deps: CapabilityCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "env" }>,
): Promise<void> {
  const [action] = command.args
  if (!action || action === "list" || action === "ls") {
    if (!deps.listEnvironments) return deps.flashFooter("environment registry is not available in this daemon", "error")
    const environments = await deps.listEnvironments()
    deps.appendNotice(environments.length === 0 ? "No Arroba environments registered." : environments.map(formatEnvironmentSummary).join("\n"))
    deps.flashFooter(`listed ${environments.length} environment${environments.length === 1 ? "" : "s"}`, "info")
    return
  }
  if (action === "show") {
    const name = command.args[1]
    if (!name || !deps.getEnvironment) return deps.flashFooter("usage: /env show <name>", "error")
    const environment = await deps.getEnvironment(name)
    deps.appendNotice(JSON.stringify(environment, null, 2))
    deps.flashFooter(`showing environment ${environment.name}`, "info")
    return
  }
  if (action === "register") {
    if (!deps.registerEnvironment) return deps.flashFooter("environment registration is not available in this daemon", "error")
    const config = parseEnvironmentConfig(command.args)
    if (!config) return deps.flashFooter("usage: /env register <name> --python <python-path> | /env register <name> --node <node-path> [--package-root <dir>]", "error")
    const environment = await deps.registerEnvironment(config)
    deps.flashFooter(`registered environment ${environment.name}`, "info")
    return
  }
  if (action === "set") {
    const key = command.args[1]
    if (!key || !deps.setCredentialSecret) return deps.flashFooter("usage: /credential set <vault-key>", "error")
    if (!deps.readSecret) return deps.flashFooter("credential set requires hidden input support", "error")
    const value = await deps.readSecret(`credential ${key}: `)
    if (!value) return deps.flashFooter("credential value must not be empty", "error")
    await deps.setCredentialSecret(key, value)
    deps.flashFooter(`credential ${key} stored in OS keychain`, "info")
    return
  }
  if (action === "remove" || action === "unregister") {
    const name = command.args[1]
    if (!name || !deps.removeEnvironment) return deps.flashFooter(`usage: /env ${action} <name>`, "error")
    const environment = await deps.removeEnvironment(name)
    deps.flashFooter(`removed environment ${environment.name}`, "info")
    return
  }
  deps.flashFooter("usage: /env list | /env show <name> | /env register <name> --python <path> | /env remove <name>", "error")
}

export async function handleScriptSlashCommand(
  deps: CapabilityCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "script" }>,
): Promise<void> {
  const [action] = command.args
  if (!action || action === "list" || action === "ls") {
    if (!deps.listScripts) return deps.flashFooter("script registry is not available in this daemon", "error")
    const scripts = await deps.listScripts()
    deps.appendNotice(scripts.length === 0 ? "No Arroba scripts registered." : scripts.map(formatScriptSummary).join("\n"))
    deps.flashFooter(`listed ${scripts.length} script${scripts.length === 1 ? "" : "s"}`, "info")
    return
  }
  if (action === "show") {
    const name = command.args[1]
    if (!name || !deps.getScript) return deps.flashFooter("usage: /script show <name>", "error")
    const script = await deps.getScript(name)
    deps.appendNotice(JSON.stringify(script, null, 2))
    deps.flashFooter(`showing script ${script.name}`, "info")
    return
  }
  if (action === "validate" || action === "register") {
    const parsed = parseScriptRegistrationArgs(command.args)
    const handler = action === "validate" ? deps.validateScript : deps.registerScript
    if (!parsed || !handler) return deps.flashFooter(`usage: /script ${action} <path> --env <environment> [--name <name>]`, "error")
    const script = await handler(parsed.sourcePath, parsed.environment, parsed.name)
    deps.flashFooter(`${action === "validate" ? "validated" : "registered"} script ${script.name}`, "info")
    return
  }
  if (action === "remove" || action === "unregister") {
    const name = command.args[1]
    if (!name || !deps.removeScript) return deps.flashFooter(`usage: /script ${action} <name>`, "error")
    const script = await deps.removeScript(name)
    deps.flashFooter(`removed script ${script.name}`, "info")
    return
  }
  if (action === "grant" || action === "revoke") {
    const agentRef = command.args[1]
    const name = command.args[2]
    if (action === "grant") {
      const environment = readOption(command.args, "--env")
      if (!agentRef || !name || !environment || !deps.grantAgentScript) return deps.flashFooter("usage: /script grant <agent-ref> <name> --env <environment>", "error")
      const agent = await deps.grantAgentScript(agentRef, name, environment)
      deps.flashFooter(`granted script ${name} to ${agent.agent_ref}`, "info")
      return
    }
    if (!agentRef || !name || !deps.revokeAgentScript) return deps.flashFooter("usage: /script revoke <agent-ref> <name>", "error")
    const agent = await deps.revokeAgentScript(agentRef, name)
    deps.flashFooter(`revoked script ${name} from ${agent.agent_ref}`, "info")
    return
  }
  if (action === "grants" || action === "agent") {
    const agent = resolveGrantTarget(deps, command.args[1], `usage: /script ${action} <agent-ref>`)
    if (!agent) return
    deps.appendNotice(formatAgentCapabilityGrants(agent, "script"))
    deps.flashFooter(`showing script grants for ${agent.agent_ref}`, "info")
    return
  }
  deps.flashFooter("usage: /script list | /script show <name> | /script validate <path> --env <environment> [--name <name>] | /script register <path> --env <environment> [--name <name>] | /script remove <name> | /script grant <agent-ref> <name> --env <environment> | /script revoke <agent-ref> <name>", "error")
}

export async function handleCredentialSlashCommand(
  deps: CapabilityCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "credential" }>,
): Promise<void> {
  const [action] = command.args
  if (!action || action === "list" || action === "ls") {
    if (!deps.listCredentials) return deps.flashFooter("credential registry is not available in this daemon", "error")
    const credentials = await deps.listCredentials()
    deps.appendNotice(credentials.length === 0 ? "No Arroba credentials registered." : credentials.map(formatCredentialSummary).join("\n"))
    deps.flashFooter(`listed ${credentials.length} credential${credentials.length === 1 ? "" : "s"}`, "info")
    return
  }
  if (action === "show") {
    const id = command.args[1]
    if (!id || !deps.getCredential) return deps.flashFooter("usage: /credential show <id>", "error")
    const credential = await deps.getCredential(id)
    deps.appendNotice(JSON.stringify(credential, null, 2))
    deps.flashFooter(`showing credential ${credential.id}`, "info")
    return
  }
  if (action === "register") {
    const sourcePath = command.args[1]
    if (!sourcePath || !deps.registerCredential) return deps.flashFooter("usage: /credential register <file.yaml>", "error")
    const credential = await deps.registerCredential(sourcePath)
    deps.flashFooter(`registered credential ${credential.id}`, "info")
    return
  }
  if (action === "remove" || action === "unregister") {
    const id = command.args[1]
    if (!id || !deps.removeCredential) return deps.flashFooter(`usage: /credential ${action} <id>`, "error")
    const credential = await deps.removeCredential(id)
    deps.flashFooter(`removed credential ${credential.id}`, "info")
    return
  }
  deps.flashFooter("usage: /credential list | /credential show <id> | /credential set <vault-key> | /credential register <file.yaml> | /credential remove <id>", "error")
}

export async function handleConnectorSlashCommand(
  deps: CapabilityCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "connector" }>,
): Promise<void> {
  const [action] = command.args
  if (action === "adapter" || action === "adapters") {
    const subaction = command.args[1]
    if (!subaction || subaction === "list" || subaction === "ls") {
      if (!deps.listConnectorAdapters) return deps.flashFooter("connector adapter registry is not available in this daemon", "error")
      const adapters = await deps.listConnectorAdapters()
      deps.appendNotice(adapters.length === 0 ? "No Arroba connector adapters registered." : adapters.map(formatConnectorAdapterSummary).join("\n"))
      deps.flashFooter(`listed ${adapters.length} connector adapter${adapters.length === 1 ? "" : "s"}`, "info")
      return
    }
    if (subaction === "show") {
      const name = command.args[2]
      if (!name || !deps.getConnectorAdapter) return deps.flashFooter("usage: /connector adapter show <name>", "error")
      const adapter = await deps.getConnectorAdapter(name)
      deps.appendNotice(JSON.stringify(adapter, null, 2))
      deps.flashFooter(`showing connector adapter ${adapter.name}`, "info")
      return
    }
    if (subaction === "register") {
      const sourcePath = command.args[2]
      if (!sourcePath || !deps.registerConnectorAdapter) return deps.flashFooter("usage: /connector adapter register <adapter.yaml>", "error")
      const adapter = await deps.registerConnectorAdapter(sourcePath)
      deps.flashFooter(`registered connector adapter ${adapter.name}`, "info")
      return
    }
    if (subaction === "remove" || subaction === "unregister") {
      const name = command.args[2]
      if (!name || !deps.removeConnectorAdapter) return deps.flashFooter(`usage: /connector adapter ${subaction} <name>`, "error")
      const adapter = await deps.removeConnectorAdapter(name)
      deps.flashFooter(`removed connector adapter ${adapter.name}`, "info")
      return
    }
    deps.flashFooter("usage: /connector adapter list | /connector adapter show <name> | /connector adapter register <adapter.yaml> | /connector adapter remove <name>", "error")
    return
  }
  if (!action || action === "list" || action === "ls") {
    if (!deps.listConnectors) return deps.flashFooter("connector registry is not available in this daemon", "error")
    const connectors = await deps.listConnectors()
    deps.appendNotice(connectors.length === 0 ? "No Arroba connectors registered." : connectors.map(formatConnectorSummary).join("\n"))
    deps.flashFooter(`listed ${connectors.length} connector${connectors.length === 1 ? "" : "s"}`, "info")
    return
  }
  if (action === "show") {
    const name = command.args[1]
    if (!name || !deps.getConnector) return deps.flashFooter("usage: /connector show <name>", "error")
    const connector = await deps.getConnector(name)
    deps.appendNotice(JSON.stringify(connector, null, 2))
    deps.flashFooter(`showing connector ${connector.name}`, "info")
    return
  }
  if (action === "register") {
    const sourcePath = command.args[1]
    if (!sourcePath || !deps.registerConnector) return deps.flashFooter("usage: /connector register <file.yaml>", "error")
    const connector = await deps.registerConnector(sourcePath)
    deps.flashFooter(`registered connector ${connector.name}`, "info")
    return
  }
  if (action === "remove" || action === "unregister") {
    const name = command.args[1]
    if (!name || !deps.removeConnector) return deps.flashFooter(`usage: /connector ${action} <name>`, "error")
    const connector = await deps.removeConnector(name)
    deps.flashFooter(`removed connector ${connector.name}`, "info")
    return
  }
  if (action === "test") {
    const name = command.args[1]
    const operation = command.args[2]
    const inputText = readOption(command.args, "--input") ?? "{}"
    const credential = readOption(command.args, "--credential")
    const allow = readOption(command.args, "--allow")
    if (!name || !operation || !deps.testConnector) return deps.flashFooter("usage: /connector test <name> <operation> [--credential <id>] [--allow read|write|destructive] --input '<json>'", "error")
    const input = JSON.parse(inputText) as Record<string, unknown>
    const execution = await deps.testConnector(name, operation, input, credential, allow)
    deps.appendNotice(JSON.stringify(execution, null, 2))
    deps.flashFooter(`tested connector ${name}.${operation}`, "info")
    return
  }
  if (action === "doctor") {
    const name = command.args[1]
    if (!name || !deps.getConnector) return deps.flashFooter("usage: /connector doctor <name> [--credential <id>]", "error")
    const connector = await deps.getConnector(name)
    const credentialId = readOption(command.args, "--credential")
    const credential = credentialId && deps.getCredential ? await deps.getCredential(credentialId) : null
    deps.appendNotice(formatConnectorDoctor(connector, credentialId, credential))
    deps.flashFooter(`checked connector ${connector.name}`, "info")
    return
  }
  if (action === "grant" || action === "revoke") {
    const agentRef = command.args[1]
    const name = command.args[2]
    if (action === "grant") {
      if (!agentRef || !name || !deps.grantAgentConnector) return deps.flashFooter("usage: /connector grant <agent-ref> <name> [--credential <id>] [--allow read|write|destructive]", "error")
      const agent = await deps.grantAgentConnector(agentRef, name, readOption(command.args, "--credential"), readOption(command.args, "--allow"))
      deps.flashFooter(`granted connector ${name} to ${agent.agent_ref}`, "info")
      return
    }
    if (!agentRef || !name || !deps.revokeAgentConnector) return deps.flashFooter("usage: /connector revoke <agent-ref> <name>", "error")
    const agent = await deps.revokeAgentConnector(agentRef, name)
    deps.flashFooter(`revoked connector ${name} from ${agent.agent_ref}`, "info")
    return
  }
  if (action === "grants" || action === "agent") {
    const agent = resolveGrantTarget(deps, command.args[1], `usage: /connector ${action} <agent-ref>`)
    if (!agent) return
    deps.appendNotice(formatAgentCapabilityGrants(agent, "connector"))
    deps.flashFooter(`showing connector grants for ${agent.agent_ref}`, "info")
    return
  }
  deps.flashFooter("usage: /connector list | /connector show <name> | /connector register <file.yaml> | /connector adapter list|register|show|remove | /connector remove <name> | /connector doctor <name> [--credential <id>] | /connector test <name> <operation> --input '<json>' | /connector grant <agent-ref> <name> | /connector revoke <agent-ref> <name>", "error")
}

export async function handleExtensionSlashCommand(
  deps: CapabilityCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "extension" }>,
): Promise<void> {
  const [action, kind, agentRef, name] = command.args
  if (action === "sync-status") {
    const agent = resolveGrantTarget(deps, kind, "usage: /extension sync-status <agent-ref>")
    if (!agent) return
    deps.appendNotice(formatRemoteExtensionSyncStatus(agent))
    deps.flashFooter(`showing remote extension sync for ${agent.agent_ref}`, "info")
    return
  }
  if (action === "sync-retry") {
    if (!kind || !deps.syncRemoteExtensionManifest) return deps.flashFooter("usage: /extension sync-retry <agent-ref>", "error")
    const agent = await deps.syncRemoteExtensionManifest(kind)
    deps.appendNotice(formatRemoteExtensionSyncStatus(agent))
    deps.flashFooter(`retried remote extension sync for ${agent.agent_ref}`, "info")
    return
  }
  if (action === "audit") {
    if (!kind || !deps.listHomeExtensionAudit) return deps.flashFooter("usage: /extension audit <agent-ref> [--limit <count>]", "error")
    const events = await deps.listHomeExtensionAudit(kind, readNumberOption(command.args, "--limit"))
    deps.appendNotice(formatHomeExtensionAuditEvents(events))
    deps.flashFooter(`showing home extension audit for ${kind}`, "info")
    return
  }
  if (action !== "grant" && action !== "revoke" && action !== "grants") {
    deps.flashFooter("usage: /extension grant|revoke <mcp|skill|script|connector> <agent-ref> <name> [--env <environment>] [--credential <id>] [--allow read|write|destructive] | /extension grants <kind> <agent-ref> | /extension sync-status|sync-retry|audit <agent-ref>", "error")
    return
  }
  if (kind !== "mcp" && kind !== "skill" && kind !== "script" && kind !== "connector") return deps.flashFooter("extension kind must be mcp, skill, script, or connector", "error")
  if (action === "grants") {
    const agent = resolveGrantTarget(deps, agentRef, "usage: /extension grants <mcp|skill|script|connector> <agent-ref>")
    if (!agent) return
    deps.appendNotice(formatAgentCapabilityGrants(agent, kind))
    deps.flashFooter(`showing ${kind} grants for ${agent.agent_ref}`, "info")
    return
  }
  const environment = readOption(command.args, "--env")
  if (!agentRef || !name) return deps.flashFooter(`usage: /extension ${action} <mcp|skill|script|connector> <agent-ref> <name> [--env <environment>]`, "error")
  if (kind === "mcp") {
    const handler = action === "grant" ? deps.grantAgentMcp : deps.revokeAgentMcp
    if (!handler) return deps.flashFooter(`MCP ${action} is not available`, "error")
    const agent = await handler(agentRef, name)
    deps.flashFooter(`${action === "grant" ? "granted" : "revoked"} MCP ${name} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, "info")
    return
  }
  if (kind === "skill") {
    const handler = action === "grant" ? deps.grantAgentSkill : deps.revokeAgentSkill
    if (!handler) return deps.flashFooter(`skill ${action} is not available`, "error")
    const agent = await handler(agentRef, name)
    deps.flashFooter(`${action === "grant" ? "granted" : "revoked"} skill ${name} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, "info")
    return
  }
  if (kind === "connector") {
    const handler = action === "grant" ? deps.grantAgentConnector : deps.revokeAgentConnector
    if (!handler) return deps.flashFooter(`connector ${action} is not available`, "error")
    const agent = action === "grant"
      ? await deps.grantAgentConnector!(agentRef, name, readOption(command.args, "--credential"), readOption(command.args, "--allow"))
      : await deps.revokeAgentConnector!(agentRef, name)
    deps.flashFooter(`${action === "grant" ? "granted" : "revoked"} connector ${name} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, "info")
    return
  }
  if (action === "grant") {
    if (!environment || !deps.grantAgentScript) return deps.flashFooter("usage: /extension grant script <agent-ref> <name> --env <environment>", "error")
    const agent = await deps.grantAgentScript(agentRef, name, environment)
    deps.flashFooter(`granted script ${name} to ${agent.agent_ref}`, "info")
    return
  }
  if (!deps.revokeAgentScript) return deps.flashFooter("script revoke is not available", "error")
  const agent = await deps.revokeAgentScript(agentRef, name)
  deps.flashFooter(`revoked script ${name} from ${agent.agent_ref}`, "info")
}

function formatRemoteExtensionSyncStatus(agent: AgentInstance): string {
  const agentLabel = `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
  if (!agent.remote_execution) {
    return `${agentLabel} is worker-local; no home-proxy manifest is projected.`
  }
  const status = agent.remote_extension_manifest_sync
  const rows = [
    `${agentLabel} remote extension sync: ${formatRemoteExtensionSyncStatusLine(status)}`,
    `worker kernel: ${agent.remote_execution.worker_kernel_id}`,
    `worker machine: ${agent.remote_execution.worker_machine_id}`,
    `leased agent: ${agent.remote_execution.leased_agent_id}`,
    `active worker run: ${agent.remote_execution.active_worker_provider_run_id ?? "none"}`,
  ]
  if (status?.manifest_hash) rows.push(`manifest hash: ${status.manifest_hash}`)
  if (status?.last_synced_at_ms) rows.push(`last synced: ${new Date(status.last_synced_at_ms).toISOString()}`)
  if (status?.last_attempted_at_ms) rows.push(`last attempted: ${new Date(status.last_attempted_at_ms).toISOString()}`)
  if (status?.last_error) rows.push(`last error: ${status.last_error}`)
  if (status?.pending_revoke) rows.push("revoke state: pending worker acknowledgement")
  const nextAction = remoteExtensionSyncNextAction(status)
  if (nextAction) rows.push(`next: ${nextAction}`)
  return rows.join("\n")
}

function formatRemoteExtensionSyncStatusLine(status: AgentInstance["remote_extension_manifest_sync"]): string {
  if (!status) return "pending"
  const revoke = status.pending_revoke ? ", pending revoke" : ""
  const error = status.last_error ? `, ${status.last_error}` : ""
  return `${status.state}${revoke}${error}`
}

function remoteExtensionSyncNextAction(status: AgentInstance["remote_extension_manifest_sync"]): string | null {
  if (!status || status.state === "pending" || status.state === "syncing") {
    return "wait for the worker manifest update; retry if it does not settle"
  }
  if (status.pending_revoke) {
    return "keep the home revoke in place; retry sync after the worker reconnects"
  }
  if (status.state === "failed" || status.state === "stale") {
    return "check worker connectivity, then run /extension sync-retry for this agent"
  }
  return null
}

export function formatHomeExtensionAuditEvents(events: readonly Record<string, unknown>[]): string {
  if (events.length === 0) return "no home extension audit events"
  return events.map((event) => {
    const payload = typeof event.payload === "object" && event.payload ? event.payload as Record<string, unknown> : {}
    const tool = typeof payload.tool === "object" && payload.tool ? payload.tool as Record<string, unknown> : {}
    const grant = typeof payload.grant === "object" && payload.grant ? payload.grant as Record<string, unknown> : {}
    const status = typeof payload.status === "string" ? ` ${payload.status}` : ""
    const toolName = typeof tool.tool_name === "string"
      ? ` ${tool.tool_name}`
      : typeof grant.name === "string" ? ` ${grant.name}` : ""
    const at = typeof event.timestamp_ms === "number" ? new Date(event.timestamp_ms).toISOString() : "unknown-time"
    const rows = [`${at} ${String(event.kind ?? "event")}${toolName}${status}`]
    const actor = [
      fieldPart("home", payload.home_user_id),
      fieldPart("caller", payload.caller_user_id),
      fieldPart("agent", payload.agent_id ?? payload.home_agent_id),
      fieldPart("lease", payload.lease_id),
      fieldPart("worker", payload.worker_kernel_id),
      fieldPart("run", payload.worker_provider_run_id ?? payload.active_worker_provider_run_id),
    ].filter(Boolean)
    if (actor.length > 0) rows.push(`  actor: ${actor.join(" ")}`)
    if (Object.keys(tool).length > 0) {
      const details = [
        typeof tool.kind === "string" && typeof tool.name === "string" ? `${tool.kind}:${tool.name}` : null,
        fieldPart("as", tool.tool_name),
        fieldPart("safety", tool.safety),
        fieldPart("timeout", typeof tool.timeout_sec === "number" ? `${tool.timeout_sec}s` : tool.timeout_sec),
        fieldPart("hash", tool.version_hash),
      ].filter(Boolean)
      if (details.length > 0) rows.push(`  tool: ${details.join(" ")}`)
    }
    if (Object.keys(grant).length > 0) {
      const details = [
        typeof grant.kind === "string" && typeof grant.name === "string" ? `${grant.kind}:${grant.name}` : null,
        fieldPart("env", grant.environment),
        typeof grant.credential_present === "boolean" ? `credential=${grant.credential_present ? "yes" : "no"}` : null,
        fieldPart("allow", grant.max_safety),
      ].filter(Boolean)
      if (details.length > 0) rows.push(`  grant: ${details.join(" ")}`)
    }
    const result = [
      fieldPart("ok", payload.ok),
      fieldPart("bytes", payload.result_bytes),
      fieldPart("duration", typeof payload.duration_ms === "number" ? `${payload.duration_ms}ms` : payload.duration_ms),
    ].filter(Boolean)
    if (result.length > 0) rows.push(`  result: ${result.join(" ")}`)
    if (typeof payload.error === "string" && payload.error) rows.push(`  error: ${payload.error}`)
    return rows.join("\n")
  }).join("\n")
}

function fieldPart(label: string, value: unknown): string | null {
  if (value === null || value === undefined || value === "") return null
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
    return `${label}=${value}`
  }
  return null
}

function readNumberOption(args: string[], flag: string): number | null {
  const value = readOption(args, flag)
  if (!value) return null
  const parsed = Number(value)
  return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : null
}

function formatMcpSummary(mcp: ArrobaMcpServerConfig): string {
  const transportType = typeof mcp.transport?.type === "string" ? mcp.transport.type : "unknown"
  const status = mcp.enabled === false ? "disabled" : "enabled"
  return `${mcp.name} [${transportType}, ${status}]`
}

function formatSkillSummary(skill: ArrobaSkillMetadata): string {
  const summary = skill.short_description ?? skill.description
  return `${skill.name}: ${summary}`
}

function formatEnvironmentSummary(environment: ArrobaEnvironmentConfig): string {
  const runtime = typeof environment.runtime?.type === "string"
    ? environment.runtime.type
    : Object.keys(environment.runtime ?? {})[0] ?? "runtime"
  return `${environment.name} [${runtime}]`
}

function formatScriptSummary(script: ArrobaScriptMetadata): string {
  return `${script.name} [${script.runtime}]: ${script.description}`
}

function formatCredentialSummary(credential: ArrobaCredentialConfig): string {
  const uses = (credential.allowed_uses ?? []).join(",") || "any"
  return `${credential.id} [${uses}]${credential.description ? `: ${credential.description}` : ""}`
}

function formatConnectorSummary(connector: ArrobaConnectorDefinition): string {
  const operationCount = Array.isArray(connector.operations) ? connector.operations.length : 0
  return `${connector.name} [${connector.adapter}, ${operationCount} op${operationCount === 1 ? "" : "s"}]: ${connector.description}`
}

function formatConnectorAdapterSummary(adapter: ArrobaConnectorAdapterDefinition): string {
  return `${adapter.name}: ${adapter.description ?? adapter.adapter_protocol}`
}

function formatConnectorDoctor(
  connector: ArrobaConnectorDefinition,
  credentialId: string | null,
  credential: ArrobaCredentialConfig | null,
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

function formatMcpDetails(mcp: ArrobaMcpServerConfig): string {
  return JSON.stringify(mcp, null, 2)
}

function formatSkillDetails(skill: ArrobaSkillMetadata): string {
  return [
    `${skill.name}: ${skill.description}`,
    skill.short_description ? `short: ${skill.short_description}` : null,
    `path: ${skill.path}`,
  ].filter(Boolean).join("\n")
}

function formatMcpImportOutcome(outcome: McpImportOutcome): string {
  const lines: string[] = []
  if (outcome.imported.length > 0) {
    lines.push(`Imported MCPs: ${outcome.imported.map((mcp) => mcp.name).join(", ")}`)
  }
  if (outcome.skipped.length > 0) {
    lines.push("Skipped MCPs:")
    for (const skip of outcome.skipped) {
      lines.push(`- ${skip.name}: ${skip.reason}`)
    }
  }
  return lines.length === 0 ? "No MCPs imported." : lines.join("\n")
}

function formatSkillImportOutcome(outcome: SkillImportOutcome): string {
  const lines: string[] = []
  if (outcome.imported.length > 0) {
    lines.push(`Imported skills: ${outcome.imported.map((skill) => skill.name).join(", ")}`)
  }
  if (outcome.skipped.length > 0) {
    lines.push("Skipped skills:")
    for (const skip of outcome.skipped) {
      const suffix = skip.path ? ` (${skip.path})` : ""
      lines.push(`- ${skip.name}${suffix}: ${skip.reason}`)
    }
  }
  return lines.length === 0 ? "No skills imported." : lines.join("\n")
}

function resolveGrantTarget(
  deps: CapabilityCommandHandlerDeps,
  agentRef: string | undefined,
  usage: string,
): AgentInstance | null {
  if (!agentRef) {
    deps.flashFooter(usage, "error")
    return null
  }
  const resolved = deps.resolveSessionAgent(agentRef)
  if (!resolved.agent) {
    deps.flashFooter(resolved.error ?? `unknown agent ${agentRef}`, "error")
    return null
  }
  return resolved.agent
}
