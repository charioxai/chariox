import type {
  AgentInstance,
  ArrobaMcpServerConfig,
  ArrobaSkillMetadata,
  McpImportOutcome,
  SkillImportOutcome,
} from "./kernel-types.js"
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
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import { resolveShellAgent } from "./shell-agent-resolver.js"
import {
  formatAgentCapabilityGrants,
  formatMcpImportOutcome,
  formatMcpList,
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
        ? grantAgentCapabilityRequest(context.workspace, agentRef, "mcp", grantName)
        : revokeAgentCapabilityRequest(agentRef, "mcp", grantName)
      const response = await deps.client.send(request)
      const variant = action === "grant" ? "AgentCapabilityGranted" : "AgentCapabilityRevoked"
      const agent = expectVariant<{ agent: AgentInstance }>(response, variant).agent
      return { ok: true, message: `${action === "grant" ? "granted" : "revoked"} MCP ${grantName} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, data: { agent }, contextUpdates: { agentId: agent.id } }
    }
    case "grants":
    case "agent": {
      const agent = await resolveShellAgent(context, deps, name)
      if (!agent.ok) {
        return { ok: false, message: agent.message }
      }
      return { ok: true, message: formatAgentCapabilityGrants(agent.agent, "mcp"), data: { agent: agent.agent } }
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
        ? grantAgentCapabilityRequest(context.workspace, agentRef, "skill", grantName)
        : revokeAgentCapabilityRequest(agentRef, "skill", grantName)
      const response = await deps.client.send(request)
      const variant = action === "grant" ? "AgentCapabilityGranted" : "AgentCapabilityRevoked"
      const agent = expectVariant<{ agent: AgentInstance }>(response, variant).agent
      return { ok: true, message: `${action === "grant" ? "granted" : "revoked"} skill ${grantName} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, data: { agent }, contextUpdates: { agentId: agent.id } }
    }
    case "grants":
    case "agent": {
      const agent = await resolveShellAgent(context, deps, name)
      if (!agent.ok) {
        return { ok: false, message: agent.message }
      }
      return { ok: true, message: formatAgentCapabilityGrants(agent.agent, "skill"), data: { agent: agent.agent } }
    }
    default:
      return { ok: false, message: "usage: skill list|show|install|update|uninstall|import|grant|revoke|grants" }
  }
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

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
