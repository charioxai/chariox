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
    "agent-1 script grants:\n- lookup (home-proxy, env=python)\n\nremote extension sync: stale, worker offline\nnext: check worker connectivity, then run /extension sync-retry for this agent",
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
      "  grant: script:home-only env=python credential=yes allow=write",
      "1970-01-01T00:00:01.000Z home_extension.invoke.completed linear_create_issue completed",
      "  actor: home=alice caller=bob agent=agent-2 lease=lease-2 worker=worker-2 run=run-2",
      "  tool: connector:linear as=linear_create_issue safety=write timeout=30s hash=hash-1",
      "  invocation: id=invoke-1 call=call-1 attempt=2 idempotency=idem-1",
      "  result: ok=true bytes=128 duration=42ms",
      "1970-01-01T00:00:01.500Z home_extension.invoke.replayed linear_create_issue replayed",
      "  tool: connector:linear as=linear_create_issue",
      "  invocation: id=invoke-1-retry attempt=3 idempotency=idem-1",
      "  next: cached idempotent result was returned; no retry needed",
      "1970-01-01T00:00:02.000Z home_extension.invoke.denied denied",
      "  error: worker mismatch",
      "  next: refresh remote extension sync and verify the worker/provider run is still current before retrying",
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
