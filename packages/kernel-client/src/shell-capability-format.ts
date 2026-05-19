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

export function formatMcpList(mcps: ArrobaMcpServerConfig[]): string {
  if (mcps.length === 0) {
    return "no MCP servers installed"
  }
  return mcps.map((mcp) => {
    const enabled = mcp.enabled === false ? "disabled" : "enabled"
    const transport = Object.keys(mcp.transport ?? {})[0] ?? "transport"
    return `${mcp.name} [${enabled}] ${transport}`
  }).join("\n")
}

export function formatSkillList(skills: ArrobaSkillMetadata[]): string {
  if (skills.length === 0) {
    return "no skills installed"
  }
  return skills.map((skill) => {
    const description = skill.short_description || skill.description || skill.path
    return `${skill.name} - ${description}`
  }).join("\n")
}

export function formatMcpImportOutcome(outcome: McpImportOutcome): string {
  const lines: string[] = []
  if (outcome.imported.length > 0) {
    lines.push(`Imported MCPs: ${outcome.imported.map((mcp) => mcp.name).join(", ")}`)
  }
  if (outcome.skipped.length > 0) {
    lines.push("Skipped MCPs:")
    lines.push(...outcome.skipped.map((skip) => `- ${skip.name}: ${skip.reason}`))
  }
  return lines.length === 0 ? "No MCPs imported." : lines.join("\n")
}

export function formatSkillImportOutcome(outcome: SkillImportOutcome): string {
  const lines: string[] = []
  if (outcome.imported.length > 0) {
    lines.push(`Imported skills: ${outcome.imported.map((skill) => skill.name).join(", ")}`)
  }
  if (outcome.skipped.length > 0) {
    lines.push("Skipped skills:")
    lines.push(...outcome.skipped.map((skip) => `- ${skip.name}: ${skip.reason}`))
  }
  return lines.length === 0 ? "No skills imported." : lines.join("\n")
}

export function formatEnvironmentList(environments: ArrobaEnvironmentConfig[]): string {
  if (environments.length === 0) {
    return "no environments registered"
  }
  return environments.map((environment) => {
    const runtime = typeof environment.runtime?.type === "string"
      ? environment.runtime.type
      : Object.keys(environment.runtime ?? {})[0] ?? "runtime"
    return `${environment.name} [${runtime}]`
  }).join("\n")
}

export function formatScriptList(scripts: ArrobaScriptMetadata[]): string {
  if (scripts.length === 0) {
    return "no scripts registered"
  }
  return scripts.map((script) => `${script.name} [${script.runtime}] - ${script.description}`).join("\n")
}

export function formatAgentExtensionGrants(agent: AgentInstance, kind: ExtensionKind): string {
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
