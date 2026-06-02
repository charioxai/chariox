import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance } from "./cli-types.js"
import { parseSlashCommand } from "./commands.js"
import {
  handleExtensionSlashCommand,
  handleMcpSlashCommand,
  type CapabilityCommandHandlerDeps,
} from "./capability-command-handlers.js"

test("extension slash command confirms active home-proxy grants before exposing home execution", async () => {
  const remote = agent({ remote: true })
  const harness = capabilityHarness(remote)

  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension grant script agent-1 lookup --env py") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)
  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension grant script agent-1 lookup --env py --confirm-home-proxy") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)

  assert.deepEqual(harness.grants, ["script:agent-1:lookup:py"])
  assert.match(harness.notices[0] ?? "", /Confirm exposing script lookup to remote agent agent-1/)
  assert.match(harness.notices[0] ?? "", /rerun: \/extension grant script agent-1 lookup --env py --confirm-home-proxy/)
  assert.deepEqual(harness.footers, [
    "error:confirmation required for home-proxy grant",
    "info:granted script lookup to agent-1",
  ])
})

test("extension slash command grants passive remote skills without home-proxy confirmation", async () => {
  const remote = agent({ remote: true })
  const harness = capabilityHarness(remote)

  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension grant skill agent-1 qa") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)

  assert.deepEqual(harness.grants, ["skill:agent-1:qa"])
  assert.deepEqual(harness.notices, [])
  assert.deepEqual(harness.footers, ["info:granted skill qa to agent-1"])
})

test("mcp slash command confirms active home-proxy shorthand grants", async () => {
  const remote = agent({ remote: true })
  const harness = capabilityHarness(remote)

  await handleMcpSlashCommand(harness.deps, parseSlashCommand("/mcp grant agent-1 filesystem") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "mcp" }>)

  assert.deepEqual(harness.grants, [])
  assert.match(harness.notices[0] ?? "", /Confirm exposing mcp filesystem to remote agent agent-1/)
  assert.match(harness.notices[0] ?? "", /rerun: \/mcp grant agent-1 filesystem --confirm-home-proxy/)
  assert.deepEqual(harness.footers, ["error:confirmation required for home-proxy grant"])
})

function capabilityHarness(agent: AgentInstance) {
  const notices: string[] = []
  const footers: string[] = []
  const grants: string[] = []
  const deps: CapabilityCommandHandlerDeps = {
    appendNotice: (message) => notices.push(message),
    flashFooter: (message, tone) => footers.push(`${tone}:${message}`),
    resolveSessionAgent: (reference) => reference === agent.agent_ref || reference === agent.id
      ? { agent, error: null }
      : { agent: null, error: `agent '${reference}' not found` },
    grantAgentMcp: async (agentRef, name) => {
      grants.push(`mcp:${agentRef}:${name}`)
      return agent
    },
    grantAgentSkill: async (agentRef, name) => {
      grants.push(`skill:${agentRef}:${name}`)
      return agent
    },
    grantAgentScript: async (agentRef, name, environment) => {
      grants.push(`script:${agentRef}:${name}:${environment}`)
      return agent
    },
  }
  return { deps, footers, grants, notices }
}

function agent(options: { readonly remote?: boolean } = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "default",
    worktree_id: "/repo",
    remote_execution: options.remote ? {
      worker_kernel_id: "worker-1",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    } : null,
    extension_grants: [],
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 0,
    last_activity_at_ms: 0,
  }
}
