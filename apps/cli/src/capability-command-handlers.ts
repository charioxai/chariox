import type {
  AgentInstance,
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
  return `${agentLabel} ${label} grants:\n${grants.map((grant) => {
    const suffix = grant.environment ? ` @ ${grant.environment}` : ""
    return `- ${grant.name}${suffix}`
  }).join("\n")}`
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

export async function handleExtensionSlashCommand(
  deps: CapabilityCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "extension" }>,
): Promise<void> {
  const [action, kind, agentRef, name] = command.args
  if (action !== "grant" && action !== "revoke" && action !== "grants") {
    deps.flashFooter("usage: /extension grant|revoke <mcp|skill|script> <agent-ref> <name> [--env <environment>] | /extension grants <mcp|skill|script> <agent-ref>", "error")
    return
  }
  if (kind !== "mcp" && kind !== "skill" && kind !== "script") return deps.flashFooter("extension kind must be mcp, skill, or script", "error")
  if (action === "grants") {
    const agent = resolveGrantTarget(deps, agentRef, "usage: /extension grants <mcp|skill|script> <agent-ref>")
    if (!agent) return
    deps.appendNotice(formatAgentCapabilityGrants(agent, kind))
    deps.flashFooter(`showing ${kind} grants for ${agent.agent_ref}`, "info")
    return
  }
  const environment = readOption(command.args, "--env")
  if (!agentRef || !name) return deps.flashFooter(`usage: /extension ${action} <mcp|skill|script> <agent-ref> <name> [--env <environment>]`, "error")
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
