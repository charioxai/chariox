import assert from "node:assert/strict"
import { mkdtemp, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { createCommandActionHandlers, formatAgentCapabilityGrants, formatAgentListSummary, parseMcpInstallConfig, parseRequestedViewLayout } from "./command-actions.js"
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
    "1 agent: agent-1 (planner) [Idle]",
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
