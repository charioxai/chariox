import type { AgentInstance } from "./kernel-types.js"
import { listAgentsRequest } from "./ipc-requests.js"
import type { ShellContext } from "./shell-core.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellAgentResolverDeps = {
  client: ShellKernelClient
}

export async function resolveShellAgent(
  context: ShellContext,
  deps: ShellAgentResolverDeps,
  agentRef: string | undefined,
): Promise<{ ok: true; agent: AgentInstance } | { ok: false; message: string }> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  const reference = agentRef ?? context.agentId
  if (!reference) {
    return { ok: false, message: "usage: mcp|skill grants <agent-ref>" }
  }
  const response = await deps.client.send(listAgentsRequest(sessionId))
  const agents = expectVariant<{ agents: AgentInstance[] }>(response, "AgentsListed").agents
  const matches = agents.filter((agent) => agent.id === reference || agent.agent_ref === reference || agent.alias === reference)
  if (matches.length === 0) {
    return { ok: false, message: `unknown agent ${reference}` }
  }
  if (matches.length > 1) {
    return { ok: false, message: `agent reference ${reference} is ambiguous` }
  }
  return { ok: true, agent: matches[0]! }
}

export async function tryResolveShellAgent(
  context: ShellContext,
  deps: ShellAgentResolverDeps,
  agentRef: string | undefined,
): Promise<{ ok: true; agent: AgentInstance } | { ok: false }> {
  const result = await resolveShellAgent(context, deps, agentRef)
  return result.ok ? result : { ok: false }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
