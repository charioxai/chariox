import assert from "node:assert/strict"
import { mkdtemp, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { createCommandActionHandlers, formatAgentCapabilityGrants, formatAgentListSummary, formatHomeExtensionAuditEvents, parseMcpInstallConfig, parseRequestedViewLayout } from "./command-actions.js"
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
    "1 agent: agent-1 (planner) [Idle; opencode openai/gpt-5; worktree worktree-1; home-local; 0 grants]",
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
    "agent-1 (qa) MCP grants:\n- browser (home-local, source=home, runtime=home-local, kernel=home)\n- github (home-local, source=home, runtime=home-local, kernel=home)",
  )
  assert.equal(
    formatAgentCapabilityGrants(agent, "skill"),
    "agent-1 (qa) skill grants:\n- browser-qa (home-local, source=home, runtime=home-local, kernel=home)",
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
    "agent-1 script grants:\n- lookup (active tools home-proxy, source=home, runtime=home-proxy, kernel=home, env=python)\n\nhome extension sync: stale, worker offline\nruntime: home-proxy: home owns definition, grants, credentials, and execution; worker receives projected tools\nauthority boundary: home validates every call; credentials never leave home\nplacement: remote (worker=machine-1, kernel=worker-1, lease=lease-1, leased_agent=leased-agent-1)\nhome extension next: home keeps stale home-proxy calls blocked; run /extension sync-status agent-1; run /machine kernels machine-1; use /extension sync-retry agent-1 after worker connectivity is healthy",
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
