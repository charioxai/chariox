import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, RuntimeSession } from "./cli-types.js"
import { createInitialShellContext, defaultKernelEndpoint, executeShellScriptLines, parseShellCliArgs, shellUsage } from "./shell.js"

test("parseShellCliArgs parses kernel and context options", () => {
  assert.deepEqual(parseShellCliArgs([
    "--kernel-url", "ws://127.0.0.1:9999/kernel",
    "--workspace", "/repo",
    "--worktree", "/repo/wt",
    "--provider", "codex",
    "--model", "gpt-5.2",
    "--effort", "low",
  ]), {
    kernelUrl: "ws://127.0.0.1:9999/kernel",
    workspace: "/repo",
    worktree: "/repo/wt",
    provider: "codex",
    model: "gpt-5.2",
    effort: "low",
  })
})

test("parseShellCliArgs rejects conflicting endpoints", () => {
  assert.throws(() => parseShellCliArgs(["--kernel-url", "ws://x", "--socket", "/tmp/k.sock"]), /cannot be used together/)
})

test("parseShellCliArgs parses script-run mode", () => {
  assert.deepEqual(parseShellCliArgs(["run", "setup.arroba", "--workspace", "/repo"]), {
    mode: "run",
    scriptPath: "setup.arroba",
    workspace: "/repo",
  })
})

test("createInitialShellContext defaults worktree to workspace", () => {
  const context = createInitialShellContext({ workspace: "/repo", provider: "codex" })
  assert.equal(context.workspace, "/repo")
  assert.equal(context.worktree, "/repo")
  assert.equal(context.provider, "codex")
})

test("defaultKernelEndpoint honors env overrides", () => {
  const previousUrl = process.env.ARROBA_KERNEL_URL
  process.env.ARROBA_KERNEL_URL = "ws://example/kernel"
  try {
    assert.equal(defaultKernelEndpoint(), "ws://example/kernel")
  } finally {
    if (previousUrl === undefined) {
      delete process.env.ARROBA_KERNEL_URL
    } else {
      process.env.ARROBA_KERNEL_URL = previousUrl
    }
  }
})

test("shellUsage documents prompt commands without slash prefix", () => {
  const usage = shellUsage()
  assert.match(usage, /arroba-shell/)
  assert.match(usage, /@ session list/)
})

test("executeShellScriptLines runs comments, variables, and stops on success", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const agent = makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })
  const seen: Record<string, unknown>[] = []
  const output: string[] = []
  const code = await executeShellScriptLines([
    "# setup",
    "",
    "session new --dir qa as s",
    "agent spawn reviewer gpt-5.2 as reviewer",
  ], createInitialShellContext({ workspace: "/repo", worktree: "/repo" }), {
    client: {
      send: async (request) => {
        seen.push(request)
        if ("CreateSession" in request) {
          return { SessionCreated: { session } }
        }
        if ("SpawnAgent" in request) {
          assert.equal((request.SpawnAgent as { session_id?: string }).session_id, "session-2")
          return { AgentSpawned: { agent } }
        }
        return {}
      },
    },
    resolveExistingDirectory: async () => "/repo/qa",
  }, (line) => output.push(line))
  assert.equal(code, 0)
  assert.equal(seen.length, 2)
  assert.match(output.join(""), /bound \$s = session-2/)
  assert.match(output.join(""), /bound \$reviewer = agent-2/)
})

test("executeShellScriptLines stops on first command error", async () => {
  const output: string[] = []
  const code = await executeShellScriptLines([
    "agent list",
    "session list",
  ], createInitialShellContext({ workspace: "/repo", worktree: "/repo" }), {
    client: { send: async () => ({}) },
  }, (line) => output.push(line))
  assert.equal(code, 1)
  assert.match(output.join(""), /no current session/)
  assert.match(output.join(""), /stopped at line 1/)
})

function makeAgent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "gpt-5.2",
    worktree_id: "/repo",
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

function makeSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    workspace_id: "/repo",
    worktree_id: "/repo",
    created_at_ms: 0,
    status: "Running",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 6,
    agents: [makeAgent()],
    config_state: { version: 0, values: {} },
    ...overrides,
  }
}
