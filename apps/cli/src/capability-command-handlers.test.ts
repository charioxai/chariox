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

test("extension slash command renders remote sync status, retry, and audit counts", async () => {
  const remote = agent({
    remote: true,
    extension_grants: [{ kind: "script", name: "lookup", environment: "py" }],
    remote_extension_manifest_sync: {
      state: "failed",
      manifest_hash: "abcdef1234567890",
      last_error: "worker offline",
      pending_revoke: true,
    },
  })
  const retried = agent({
    remote: true,
    extension_grants: [{ kind: "script", name: "lookup", environment: "py" }],
    remote_extension_manifest_sync: {
      state: "synced",
      manifest_hash: "fedcba9876543210",
      pending_revoke: false,
    },
  })
  const auditEvents = [{
    kind: "home_extension.invoke.denied",
    timestamp_ms: 1780000000000,
    payload: {
      status: "denied",
      agent_ref: "agent-1",
      worker_machine_id: "machine-1",
      tool: { kind: "script", name: "lookup", tool_name: "lookup", safety: "read" },
      error: "worker mismatch",
    },
  }]
  const harness = capabilityHarness(remote, { auditEvents, retriedAgent: retried })

  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension sync-status agent-1") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)
  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension sync-retry agent-1") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)
  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension audit agent-1 --limit 1") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)

  assert.equal(harness.syncRetries, 1)
  assert.deepEqual(harness.auditRequests, [{ agentRef: "agent-1", limit: 1 }])
  assert.match(harness.notices[0] ?? "", /agent-1 remote extension sync: failed, pending revoke, worker offline/)
  assert.match(harness.notices[0] ?? "", /manifest hash: abcdef1234567890/)
  assert.match(harness.notices[0] ?? "", /revoke state: pending worker acknowledgement/)
  assert.match(harness.notices[0] ?? "", /next: keep the home revoke in place; run \/extension sync-status agent-1; run \/machine kernels machine-1 if the revoke stays pending; use \/extension sync-retry agent-1 after the worker reconnects/)
  assert.match(harness.notices[1] ?? "", /agent-1 remote extension sync: synced/)
  assert.match(harness.notices[1] ?? "", /manifest hash: fedcba9876543210/)
  assert.match(harness.notices[2] ?? "", /home_extension\.invoke\.denied lookup denied/)
  assert.match(harness.notices[2] ?? "", /boundary: home grants, validates, and executes; worker receives projected tools only; credentials stay home/)
  assert.match(harness.notices[2] ?? "", /tool: script:lookup as=lookup safety=read/)
  assert.match(harness.notices[2] ?? "", /error: worker mismatch/)
  assert.deepEqual(harness.footers, [
    "info:showing remote extension sync for agent-1",
    "info:retried remote extension sync for agent-1",
    "info:home extension audit for agent-1: 1 event",
  ])
})

test("extension slash command reports empty home extension audit accurately", async () => {
  const remote = agent({ remote: true })
  const harness = capabilityHarness(remote, { auditEvents: [] })

  await handleExtensionSlashCommand(harness.deps, parseSlashCommand("/extension audit agent-1") as Extract<ReturnType<typeof parseSlashCommand>, { kind: "extension" }>)

  assert.deepEqual(harness.notices, ["no home extension audit events"])
  assert.deepEqual(harness.footers, ["info:home extension audit for agent-1: 0 events"])
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

function capabilityHarness(agent: AgentInstance, options: {
  readonly auditEvents?: Record<string, unknown>[]
  readonly retriedAgent?: AgentInstance
} = {}) {
  const notices: string[] = []
  const footers: string[] = []
  const grants: string[] = []
  const auditRequests: Array<{ agentRef: string; limit: number | null | undefined }> = []
  let syncRetries = 0
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
    syncRemoteExtensionManifest: async () => {
      syncRetries += 1
      return options.retriedAgent ?? agent
    },
    listHomeExtensionAudit: async (agentRef, limit) => {
      auditRequests.push({ agentRef, limit })
      return options.auditEvents ?? []
    },
  }
  return { auditRequests, deps, footers, grants, notices, get syncRetries() { return syncRetries } }
}

function agent(options: Partial<AgentInstance> & { readonly remote?: boolean } = {}): AgentInstance {
  const { remote, ...overrides } = options
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "default",
    worktree_id: "/repo",
    remote_execution: remote ? {
      worker_kernel_id: "worker-1",
      worker_machine_id: "machine-1",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    } : null,
    extension_grants: [],
    remote_extension_manifest_sync: null,
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 0,
    last_activity_at_ms: 0,
    ...overrides,
  }
}
