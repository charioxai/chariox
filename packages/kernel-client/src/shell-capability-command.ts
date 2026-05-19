import type {
  AgentInstance,
  ArrobaEnvironmentConfig,
  ArrobaMcpServerConfig,
  ArrobaScriptMetadata,
  ArrobaSkillMetadata,
  ExtensionKind,
  McpImportOutcome,
  SkillImportOutcome,
} from "./kernel-types.js"
import {
  getEnvironmentRequest,
  getMcpServerRequest,
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
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import { resolveShellAgent } from "./shell-agent-resolver.js"
import {
  formatAgentExtensionGrants,
  formatEnvironmentList,
  formatMcpImportOutcome,
  formatMcpList,
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
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listMcpServersRequest(context.workspace))
      const mcps = expectVariant<{ mcps: ArrobaMcpServerConfig[] }>(response, "McpServersListed").mcps
      return { ok: true, message: formatMcpList(mcps), data: { mcps } }
    }
    case "show": {
      if (!name) {
        return { ok: false, message: "usage: mcp show <name>" }
      }
      const response = await deps.client.send(getMcpServerRequest(context.workspace, name))
      const mcp = expectVariant<{ mcp: ArrobaMcpServerConfig }>(response, "McpServer").mcp
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
      const mcp = expectVariant<{ mcp: ArrobaMcpServerConfig }>(response, variant).mcp
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
        return { ok: false, message: "usage: mcp import <codex|opencode> [name]" }
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
      const skills = expectVariant<{ skills: ArrobaSkillMetadata[] }>(response, "SkillsListed").skills
      return { ok: true, message: formatSkillList(skills), data: { skills } }
    }
    case "show": {
      if (!name) {
        return { ok: false, message: "usage: skill show <name>" }
      }
      const response = await deps.client.send(getSkillRequest(context.workspace, name))
      const skill = expectVariant<{ skill: ArrobaSkillMetadata }>(response, "Skill").skill
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
      const skill = expectVariant<{ skill: ArrobaSkillMetadata }>(response, variant).skill
      return { ok: true, message: `${action === "install" ? "installed" : "updated"} skill ${skill.name}`, data: { skill } }
    }
    case "uninstall":
    case "remove": {
      if (!name) {
        return { ok: false, message: `usage: skill ${action} <name>` }
      }
      const response = await deps.client.send(uninstallSkillRequest(context.workspace, name))
      const skill = expectVariant<{ skill: ArrobaSkillMetadata }>(response, "SkillUninstalled").skill
      return { ok: true, message: `uninstalled skill ${skill.name}`, data: { skill } }
    }
    case "import": {
      const provider = name
      const importName = parsed.args[2] ?? null
      if (!provider) {
        return { ok: false, message: "usage: skill import <codex|opencode> [name]" }
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
      const environments = expectVariant<{ environments: ArrobaEnvironmentConfig[] }>(response, "EnvironmentsListed").environments
      return { ok: true, message: formatEnvironmentList(environments), data: { environments } }
    }
    case "show": {
      if (!name) return { ok: false, message: "usage: env show <name>" }
      const response = await deps.client.send(getEnvironmentRequest(context.workspace, name))
      const environment = expectVariant<{ environment: ArrobaEnvironmentConfig }>(response, "Environment").environment
      return { ok: true, message: JSON.stringify(environment, null, 2), data: { environment }, format: "json" }
    }
    case "register": {
      const config = parseEnvironmentConfig(parsed.args)
      if (!config) {
        return { ok: false, message: "usage: env register <name> --python <python-path> | env register <name> --node <node-path> [--package-root <dir>]" }
      }
      const response = await deps.client.send(registerEnvironmentRequest(context.workspace, config as unknown as Record<string, unknown>))
      const environment = expectVariant<{ environment: ArrobaEnvironmentConfig }>(response, "EnvironmentRegistered").environment
      return { ok: true, message: `registered environment ${environment.name}`, data: { environment } }
    }
    case "remove":
    case "unregister": {
      if (!name) return { ok: false, message: `usage: env ${action} <name>` }
      const response = await deps.client.send(removeEnvironmentRequest(context.workspace, name))
      const environment = expectVariant<{ environment: ArrobaEnvironmentConfig }>(response, "EnvironmentRemoved").environment
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
      const scripts = expectVariant<{ scripts: ArrobaScriptMetadata[] }>(response, "ScriptsListed").scripts
      return { ok: true, message: formatScriptList(scripts), data: { scripts } }
    }
    case "show": {
      if (!name) return { ok: false, message: "usage: script show <name>" }
      const response = await deps.client.send(getScriptRequest(context.workspace, name))
      const script = expectVariant<{ script: ArrobaScriptMetadata }>(response, "Script").script
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
      const script = expectVariant<{ script: ArrobaScriptMetadata }>(response, variant).script
      return { ok: true, message: `${action === "validate" ? "validated" : "registered"} script ${script.name}`, data: { script } }
    }
    case "remove":
    case "unregister": {
      if (!name) return { ok: false, message: `usage: script ${action} <name>` }
      const response = await deps.client.send(removeScriptRequest(context.workspace, name))
      const script = expectVariant<{ script: ArrobaScriptMetadata }>(response, "ScriptRemoved").script
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

export async function executeExtensionCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellCapabilityCommandDeps,
): Promise<ShellCommandResult> {
  const [action, kind, agentRef, name] = parsed.args
  if (action !== "grant" && action !== "revoke" && action !== "grants") {
    return { ok: false, message: "usage: extension grant|revoke <mcp|skill|script> <agent-ref> <name> [--env <environment>] | extension grants <mcp|skill|script> [agent-ref]" }
  }
  if (!isExtensionKind(kind)) {
    return { ok: false, message: "extension kind must be mcp, skill, or script" }
  }
  if (action === "grants") {
    const agent = await resolveShellAgent(context, deps, agentRef)
    if (!agent.ok) return { ok: false, message: agent.message }
    return { ok: true, message: formatAgentExtensionGrants(agent.agent, kind), data: { agent: agent.agent } }
  }
  if (!agentRef || !name) {
    return { ok: false, message: `usage: extension ${action} <mcp|skill|script> <agent-ref> <name> [--env <environment>]` }
  }
  const environment = readOption(parsed.args, "--env")
  if (action === "grant" && kind === "script" && !environment) {
    return { ok: false, message: "usage: extension grant script <agent-ref> <name> --env <environment>" }
  }
  const request = action === "grant"
    ? grantAgentExtensionRequest(context.workspace, agentRef, kind, name, environment)
    : revokeAgentExtensionRequest(agentRef, kind, name)
  const response = await deps.client.send(request)
  const variant = action === "grant" ? "AgentExtensionGranted" : "AgentExtensionRevoked"
  const agent = expectVariant<{ agent: AgentInstance }>(response, variant).agent
  return { ok: true, message: `${action === "grant" ? "granted" : "revoked"} ${kind} ${name} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, data: { agent }, contextUpdates: { agentId: agent.id } }
}

function parseMcpInstallConfig(args: string[]): ArrobaMcpServerConfig | null {
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

function isExtensionKind(value: string | undefined): value is ExtensionKind {
  return value === "mcp" || value === "skill" || value === "script"
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
