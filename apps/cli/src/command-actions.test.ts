import assert from "node:assert/strict"
import { mkdtemp, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { createCommandActionHandlers, formatAgentCapabilityGrants, formatAgentListSummary, formatHomeExtensionAuditEvents, parseMcpInstallConfig, parseRequestedViewLayout } from "./command-actions.js"
import { parseSlashCommand } from "./commands.js"
import type { AgentInstance, ProviderProcessInfo, WorkflowQueuedPrompt, RuntimeAttachment, RuntimeProviderRun, RuntimeSession, WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import { makeAgent, makeCommandDeps, makeSession, runGit } from "./command-actions-test-support.js"

test("parseRequestedViewLayout handles summary, invalid, and set cases", () => {
  assert.deepEqual(parseRequestedViewLayout("", "split"), { kind: "summary" })
  assert.deepEqual(parseRequestedViewLayout("grid", "split"), { kind: "invalid" })
  assert.deepEqual(parseRequestedViewLayout("individual", "split"), {
    kind: "set",
    layout: "individual",
  })
})

test("/trigger creates, changes, removes and lists the focused agent's triggers", async () => {
  const calls: string[] = []
  const trigger = {
    id: "trigger-1",
    agent_id: "agent-1",
    source: { kind: "schedule" as const, interval_seconds: 3, repeat: "every" as const, next_run_at_ms: 3_000 },
    busy_policy: "steer" as const,
    prompt: "Check the build",
    enabled: true,
    created_at_ms: 0,
    fired: 2,
    coalesced: 0,
  }
  const triggerSession = makeSession({ agent_triggers: [trigger] })
  const handlers = createCommandActionHandlers(makeCommandDeps({
    sessionState: () => triggerSession,
    supportsAgentTriggers: async () => true,
    sendAgentTriggerRequest: async (request: Record<string, unknown>) => {
      calls.push(JSON.stringify(request))
      return { session: triggerSession }
    },
    appendNotice: (message: string) => calls.push(`notice:${message}`),
    flashFooter: (message: string, tone: string) => calls.push(`${tone}:${message}`),
  }))
  const run = (raw: string) => {
    const command = parseSlashCommand(raw)
    assert.equal(command?.kind, "trigger")
    return handlers.handleTriggerCommand(command as Extract<typeof command, { kind: "trigger" }>)
  }

  await run("/trigger every 0.05 --steer Check the build")
  await run("/wait-in 1 Check once")
  await run("/trigger off trigger-1")
  await run("/trigger queue trigger-1")
  await run("/trigger remove trigger-1")
  await run("/trigger")

  assert.deepEqual(calls.slice(0, 10), [
    JSON.stringify({ CreateAgentTrigger: { session_id: "session-1", agent_id: "agent-1", source: { kind: "schedule", interval_seconds: 3, repeat: "every" }, busy_policy: "steer", prompt: "Check the build" } }),
    "info:trigger every 0.05 minutes · steer",
    JSON.stringify({ CreateAgentTrigger: { session_id: "session-1", agent_id: "agent-1", source: { kind: "schedule", interval_seconds: 60, repeat: "once" }, busy_policy: "queue", prompt: "Check once" } }),
    "info:trigger in 1 minute · queue",
    JSON.stringify({ UpdateAgentTrigger: { session_id: "session-1", trigger_id: "trigger-1", enabled: false } }),
    "info:trigger trigger-1 off",
    JSON.stringify({ UpdateAgentTrigger: { session_id: "session-1", trigger_id: "trigger-1", busy_policy: "queue" } }),
    "info:trigger trigger-1 queue",
    JSON.stringify({ DeleteAgentTrigger: { session_id: "session-1", trigger_id: "trigger-1" } }),
    "info:trigger trigger-1 removed",
  ])
  assert.match(calls[10] ?? "", /^notice:Triggers\ntrigger-1  every 3s · steer · next .* · 2 fired$/)
})

test("/trigger list retains unconfirmed firings after their once trigger retired", async () => {
  const notices: string[] = []
  const delivery = { prompt_id: "firing-1", trigger_id: "retired-trigger", agent_id: "agent-1", origin: "schedule", prompt: "Review the restart invariant", busy_policy: "steer" as const, state: "not_confirmed" as const, recorded_at_ms: 1 }
  const session = makeSession({ agent_triggers: [], trigger_deliveries: [
    delivery, { ...delivery, prompt_id: "already-resent", state: "resent" },
    { ...delivery, prompt_id: "other-agent", agent_id: "agent-2" },
  ] })
  const handlers = createCommandActionHandlers(makeCommandDeps({
    sessionState: () => session, supportsAgentTriggers: async () => true,
    appendNotice: (message: string) => notices.push(message),
  }))
  const command = parseSlashCommand("/trigger list")!
  assert.equal(command.kind, "trigger")
  await handlers.handleTriggerCommand(command as Extract<typeof command, { kind: "trigger" }>)
  assert.match(notices.join("\n"), /Schedule.*Not confirmed.*steer.*\/trigger resend firing-1/)
  assert.doesNotMatch(notices.join("\n"), /already-resent|other-agent|No triggers/)
})

test("protocol-567 kernels retain /wait aliases and reject trigger-only commands", async () => {
  const calls: Record<string, unknown>[] = []
  const errors: string[] = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    supportsAgentTriggers: async () => false,
    sendAgentTriggerRequest: async (request: Record<string, unknown>) => {
      calls.push(request)
      return { session: makeSession() }
    },
    flashFooter: (message: string, tone: string) => { if (tone === "error") errors.push(message) },
  }))
  for (const raw of ["/wait-in 1 Ping", "/wait-every 0.5 Check"]) {
    const command = parseSlashCommand(raw)
    assert.equal(command?.kind, "trigger")
    await handlers.handleTriggerCommand(command as Extract<typeof command, { kind: "trigger" }>)
  }
  assert.deepEqual(calls, [
    { CreateAgentPromptSchedule: { session_id: "session-1", agent_id: "agent-1", kind: "once", interval_seconds: 60, prompt: "Ping" } },
    { CreateAgentPromptSchedule: { session_id: "session-1", agent_id: "agent-1", kind: "recurring", interval_seconds: 30, prompt: "Check" } },
  ])
  for (const raw of ["/trigger", "/trigger every 1 Ping", "/trigger in 1 --steer Ping", "/trigger off t1", "/trigger remove t1"]) {
    const command = parseSlashCommand(raw)
    await handlers.handleTriggerCommand(command as Extract<typeof command, { kind: "trigger" }>)
  }
  assert.equal(calls.length, 2, "unsupported requests must never reach an old kernel")
  assert.equal(errors.length, 5)
  assert.ok(errors.every((message) => /kernel.*576.*upgrade/i.test(message)))
})

test("formatAgentListSummary renders aliases and pluralization", () => {
  const agents: AgentInstance[] = [
    {
      id: "agent-1",
      agent_ref: "agent-1",
      session_id: "session-1",
      alias: "planner",
      provider: "opencode",
      model: "openai/gpt-5",
      worktree_id: "worktree-1",
      state: "Idle",
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 0,
      last_activity_at_ms: 0,
    },
  ]

  assert.equal(formatAgentListSummary([]), "no agents in session")
  assert.equal(
    formatAgentListSummary(agents),
    "1 agent: agent-1 (planner) [Idle; opencode openai/gpt-5; worktree worktree-1; local; 0 grants]",
  )
})

test("formatAgentCapabilityGrants renders MCP and skill grants", () => {
  const agent = makeAgent({
    alias: "qa",
    extension_grants: [
      { kind: "mcp", name: "browser" },
      { kind: "mcp", name: "github" },
      { kind: "skill", name: "browser-qa" },
    ],
  })

  assert.equal(
    formatAgentCapabilityGrants(agent, "mcp"),
    "agent-1 (qa) MCP grants:\n- browser (worker-local)\n- github (worker-local)",
  )
  assert.equal(
    formatAgentCapabilityGrants(agent, "skill"),
    "agent-1 (qa) skill grants:\n- browser-qa (worker-local)",
  )
  assert.equal(
    formatAgentCapabilityGrants(makeAgent(), "skill"),
    "agent-1 has no skill grants.",
  )
  assert.equal(
    formatAgentCapabilityGrants(makeAgent({
      remote_execution: {
        worker_kernel_id: "worker-1",
        worker_machine_id: "machine-1",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
      remote_extension_manifest_sync: {
        state: "stale",
        last_error: "worker offline",
      },
      extension_grants: [{ kind: "script", name: "lookup", environment: "python" }],
    }), "script"),
    "agent-1 script grants:\n- lookup (active tools home-proxy, env=python)\n\nremote extension sync: stale, worker offline\nruntime: home-proxy: home owns definition, grants, credentials, and execution; worker receives projected tools\nauthority boundary: home validates every call; credentials never leave home\nplacement: remote (worker=machine-1, kernel=worker-1, lease=lease-1, leased_agent=leased-agent-1)\nnext: home keeps stale home-proxy calls blocked; run /extension sync-status agent-1; run /machine kernels machine-1; use /extension sync-retry agent-1 after worker connectivity is healthy",
  )
})

test("formatHomeExtensionAuditEvents renders diagnostic context without payload bodies", () => {
  assert.equal(
    formatHomeExtensionAuditEvents([
      {
        kind: "home_extension.grant.created",
        timestamp_ms: 0,
        payload: {
          home_user_id: "alice",
          caller_user_id: "alice",
          agent_id: "agent-1",
          lease_id: "lease-1",
          worker_kernel_id: "worker-1",
          active_worker_provider_run_id: "run-1",
          grant: {
            kind: "script",
            name: "home-only",
            environment: "python",
            credential_present: true,
            max_safety: "write",
          },
        },
      },
      {
        kind: "home_extension.invoke.completed",
        timestamp_ms: 1000,
        payload: {
          home_user_id: "alice",
          caller_user_id: "bob",
          agent_id: "agent-2",
          lease_id: "lease-2",
          worker_kernel_id: "worker-2",
          worker_provider_run_id: "run-2",
          duration_ms: 42,
          result_bytes: 128,
          ok: true,
          status: "completed",
          invocation: {
            invocation_id: "invoke-1",
            provider_tool_call_id: "call-1",
            attempt: 2,
            idempotency_key: "idem-1",
          },
          tool: {
            kind: "connector",
            name: "linear",
            tool_name: "linear_create_issue",
            safety: "write",
            timeout_sec: 30,
            version_hash: "hash-1",
          },
        },
      },
      {
        kind: "home_extension.manifest.failed",
        timestamp_ms: 1250,
        payload: {
          home_user_id: "alice",
          caller_user_id: "alice",
          agent_ref: "A1",
          worker_machine_id: "machine-1",
          status: "failed",
          manifest_hash: "manifest-hash-1",
          tool_count: 3,
          pending_revoke: true,
          error: "worker offline",
        },
      },
      {
        kind: "home_extension.manifest.retry_scheduled",
        timestamp_ms: 1300,
        payload: {
          agent_ref: "A1",
          status: "retry_scheduled",
          manifest_hash: "manifest-hash-1",
          tool_count: 3,
          pending_revoke: true,
          attempt: 2,
          delay_sec: 10,
          error: "worker offline",
        },
      },
      {
        kind: "home_extension.manifest.synced",
        timestamp_ms: 1400,
        payload: {
          agent_ref: "A1",
          status: "synced",
          manifest_hash: "manifest-hash-2",
          tool_count: 0,
          pending_revoke: true,
          revoke_acknowledged: true,
        },
      },
      {
        kind: "home_extension.invoke.replayed",
        timestamp_ms: 1500,
        payload: {
          status: "replayed",
          invocation: {
            invocation_id: "invoke-1-retry",
            attempt: 3,
            idempotency_key: "idem-1",
          },
          tool: {
            kind: "connector",
            name: "linear",
            tool_name: "linear_create_issue",
          },
        },
      },
      {
        kind: "home_extension.invoke.denied",
        timestamp_ms: 2000,
        payload: {
          status: "denied",
          error: "worker mismatch",
          args: { secret: "not rendered" },
          result: { body: "not rendered" },
        },
      },
    ]),
    [
      "1970-01-01T00:00:00.000Z home_extension.grant.created home-only",
      "  actor: home=alice caller=alice agent=agent-1 lease=lease-1 worker=worker-1 run=run-1",
      "  boundary: home grants, validates, and executes; worker receives projected tools only; credentials stay home",
      "  grant: script:home-only env=python credential=yes allow=write",
      "1970-01-01T00:00:01.000Z home_extension.invoke.completed linear_create_issue completed",
      "  actor: home=alice caller=bob agent=agent-2 lease=lease-2 worker=worker-2 run=run-2",
      "  boundary: home grants, validates, and executes; worker receives projected tools only; credentials stay home",
      "  tool: connector:linear as=linear_create_issue safety=write timeout=30s hash=hash-1",
      "  invocation: id=invoke-1 call=call-1 attempt=2 idempotency=idem-1",
      "  result: ok=true bytes=128 duration=42ms",
      "1970-01-01T00:00:01.250Z home_extension.manifest.failed failed",
      "  actor: home=alice caller=alice agent=A1 machine=machine-1",
      "  boundary: home grants, validates, and executes; worker receives projected tools only; credentials stay home",
      "  manifest: hash=manifest-hash-1 tools=3 pending_revoke=true",
      "  error: worker offline",
      "  next: keep the home revoke in place; run /extension sync-status A1; run /machine kernels machine-1 if the revoke stays pending; use /extension sync-retry A1 after the worker reconnects",
      "1970-01-01T00:00:01.300Z home_extension.manifest.retry_scheduled retry_scheduled",
      "  actor: agent=A1",
      "  boundary: home grants, validates, and executes; worker receives projected tools only; credentials stay home",
      "  manifest: hash=manifest-hash-1 tools=3 pending_revoke=true attempt=2 retry_in=10s",
      "  error: worker offline",
      "  next: keep the home revoke in place; run /extension sync-status A1; run /kernel remote-runtime to identify worker connectivity if the revoke stays pending; use /extension sync-retry A1 after the worker reconnects",
      "1970-01-01T00:00:01.400Z home_extension.manifest.synced synced",
      "  actor: agent=A1",
      "  boundary: home grants, validates, and executes; worker receives projected tools only; credentials stay home",
      "  manifest: hash=manifest-hash-2 tools=0 pending_revoke=true revoke_acknowledged=true",
      "  next: worker acknowledged the revoke; home will continue denying calls for the removed grant",
      "1970-01-01T00:00:01.500Z home_extension.invoke.replayed linear_create_issue replayed",
      "  boundary: home grants, validates, and executes; worker receives projected tools only; credentials stay home",
      "  tool: connector:linear as=linear_create_issue",
      "  invocation: id=invoke-1-retry attempt=3 idempotency=idem-1",
      "  next: cached idempotent result was returned; no retry needed",
      "1970-01-01T00:00:02.000Z home_extension.invoke.denied denied",
      "  boundary: home grants, validates, and executes; worker receives projected tools only; credentials stay home",
      "  redacted: args result",
      "  error: worker mismatch",
      "  next: identify the affected agent in /kernel remote-runtime or the home extension audit, then retry only after the worker lease and provider run match the current home grant",
    ].join("\n"),
  )
})

test("parseMcpInstallConfig supports stdio and streamable HTTP MCPs", () => {
  assert.deepEqual(
    parseMcpInstallConfig(["install", "browser", "--command", "npx", "--arg", "-y", "--arg", "@modelcontextprotocol/server-browser", "--env", "BROWSER_TOKEN"]),
    {
      name: "browser",
      transport: {
        type: "stdio",
        command: "npx",
        args: ["-y", "@modelcontextprotocol/server-browser"],
        env: {},
        env_vars: ["BROWSER_TOKEN"],
      },
      enabled: true,
      required: false,
    },
  )
  assert.deepEqual(
    parseMcpInstallConfig(["install", "remote", "--url", "https://example.test/mcp", "--bearer-token-env-var", "REMOTE_MCP_TOKEN"]),
    {
      name: "remote",
      transport: {
        type: "streamable_http",
        url: "https://example.test/mcp",
        bearer_token_env_var: "REMOTE_MCP_TOKEN",
        http_headers: {},
        env_http_headers: {},
      },
      enabled: true,
      required: false,
    },
  )
  assert.equal(parseMcpInstallConfig(["install", "bad", "--command", "npx", "--url", "https://example.test/mcp"]), null)
})

// MP-08 / MP-10 / MP-11: post-launch Environment/Adjust uses kernel authority.
test("MP-08 / MP-10 / MP-11 /env project shows saved decisions and Adjust opens the current session review", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    sessionState: () => makeSession({project_id: "project-1"}),
    focusedAgentId: () => "agent-1",
    sendRoomEnvironmentRequest: async <T>(request: unknown): Promise<T> => {
      requests.push(request)
      return {ProjectEnvironmentManifest: {manifest: {schema_version: 1, project_id: "project-1", evidence_digest: "a".repeat(64), entries: [], private_files: [{workspace_id: "web", path: "notes.md", bring: true, reason: "Project notes"}], toolchain_hints: [], package_hints: [], service_hints: []}}} as T
    },
    appendNotice: (message: string) => notices.push(message),
  }))
  await handlers.handleEnvCommand({kind: "env", raw: "/env project", args: ["project"]})
  assert.match(notices[0]!, /Bring notes.md — Project notes/)
  await handlers.handleEnvCommand({kind: "env", raw: "/env project adjust", args: ["project", "adjust"]})
  assert.deepEqual(requests, [{GetProjectEnvironmentManifest: {projectId: "project-1", agentId: "agent-1"}}, {AdjustProjectEnvironment: {sessionId: "session-1", agentId: "agent-1"}}])
})
