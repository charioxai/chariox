import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import { createKernelSessionSnapshotController } from "./kernel-session-snapshot-controller.js"

test("kernel session snapshot refreshes changed provider run and panes after prompt completion", async () => {
  const nextRun = providerRun("run-2")
  const harness = createHarness({
    session: session({ active_prompt: activePrompt() }),
    providerRun: providerRun("run-1"),
    shouldRefreshAgentPanesForSessionChange: () => false,
  })

  await harness.controller.apply(session(), nextRun)

  assert.equal(harness.providerRun?.id, "run-2")
  assert.deepEqual(harness.calls, [
    "applySessionState:session-1",
    "providerDebug:kernel event refreshed provider run:run-2:run-1",
    "setProviderRun:run-2",
    "updateSessionChrome",
    "refreshAgentPanes:session-1",
  ])
})

test("kernel session snapshot detects prompt completion from projected prompt states", async () => {
  const harness = createHarness({
    session: session({
      agents: [agent()],
      active_prompt: {
        ...activePrompt(),
        id: "prompt-stale",
        target_agent_id: "agent-1",
      },
      prompt_states: {
        "agent-1": {
          active_prompt: {
            ...activePrompt(),
            target_agent_id: "agent-1",
          },
          queued_prompts: [],
        },
      },
    }),
    shouldRefreshAgentPanesForSessionChange: () => false,
  })

  await harness.controller.apply(session({
    agents: [agent()],
    active_prompt: {
      ...activePrompt(),
      id: "prompt-stale",
      target_agent_id: "agent-1",
    },
    prompt_states: {},
  }), null)

  assert.deepEqual(harness.calls, [
    "applySessionState:session-1",
    "refreshAgentPanes:session-1",
  ])
})

test("kernel session snapshot clears missing provider run and recovers polling transport", async () => {
  const harness = createHarness({
    providerRun: providerRun("run-1"),
    supportsKernelEventStream: false,
  })

  await harness.controller.apply(session({ active_prompt: activePrompt() }), null)

  assert.equal(harness.providerRun, null)
  assert.deepEqual(harness.calls, [
    "applySessionState:session-1",
    "providerDebug:kernel event cleared provider run:run-1:",
    "setProviderRun:null",
    "updateSessionChrome",
    "recoverProviderRun:missing active provider run",
  ])
})

test("kernel session snapshot refreshes panes when session shape changes", async () => {
  const harness = createHarness({
    shouldRefreshAgentPanesForSessionChange: () => true,
  })

  await harness.controller.apply(session(), null)

  assert.deepEqual(harness.calls, [
    "applySessionState:session-1",
    "refreshAgentPanes:session-1",
  ])
})

function createHarness(options: {
  session?: RuntimeSession
  providerRun?: RuntimeProviderRun | null
  supportsKernelEventStream?: boolean
  shouldRefreshAgentPanesForSessionChange?: (session: RuntimeSession) => boolean
} = {}) {
  const calls: string[] = []
  const harness = {
    calls,
    session: options.session ?? session(),
    providerRun: options.providerRun ?? null,
    controller: null as ReturnType<typeof createKernelSessionSnapshotController> | null,
  }
  harness.controller = createKernelSessionSnapshotController({
    getSession: () => harness.session,
    getProviderRun: () => harness.providerRun,
    projectSession: (nextSession) => nextSession,
    shouldRefreshAgentPanesForSessionChange: options.shouldRefreshAgentPanesForSessionChange ?? (() => false),
    applySessionState: (nextSession) => {
      calls.push(`applySessionState:${nextSession.id}`)
      harness.session = nextSession
    },
    sameProviderRun: (left, right) => left.id === right.id,
    logProviderRunDebug: (message, run, fields) => {
      calls.push(`providerDebug:${message}:${run?.id ?? "null"}:${fields?.previous_provider_run_id ?? ""}`)
    },
    setProviderRun: (run) => {
      calls.push(`setProviderRun:${run?.id ?? "null"}`)
      harness.providerRun = run
    },
    updateSessionChrome: () => {
      calls.push("updateSessionChrome")
    },
    supportsKernelEventStream: () => options.supportsKernelEventStream ?? true,
    recoverProviderRun: (reason) => {
      calls.push(`recoverProviderRun:${reason}`)
    },
    refreshAgentPanes: async (nextSession) => {
      calls.push(`refreshAgentPanes:${nextSession.id}`)
    },
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createKernelSessionSnapshotController>
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Created",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 1,
    agents: [],
    config_state: { values: {} } as RuntimeSession["config_state"],
    ...overrides,
  }
}

function agent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "model",
    effort: null,
    worktree_id: "/workspace",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
    ...overrides,
  }
}

function activePrompt() {
  return {
    id: "prompt-1",
    source_attachment_id: "attachment-1",
    prompt: "build",
    status: "running",
  }
}

function providerRun(id: string): RuntimeProviderRun {
  return {
    id,
    session_id: "session-1",
    agent_instance_id: null,
    adapter_key: "opencode",
    provider: "opencode",
    account_profile: "default",
    model: "model",
    variant: null,
    usage_tokens_total: null,
    state: "running",
  }
}
