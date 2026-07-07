import assert from "node:assert/strict"
import test from "node:test"

import { createAttachedSessionPrimeController } from "./attached-session-prime-controller.js"
import type {
  AgentInstance,
  RuntimeSession,
  SessionHistoryCursorState,
  SessionHistoryOutline,
  TranscriptEntry,
} from "./cli-types.js"
import {
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
} from "@arroba/kernel-client/external-provider-observation"

test("attached session prime clears the transcript when no agent is visible", async () => {
  const harness = primeHarness()

  await harness.controller.prime(session({ agents: [], focused_agent_id: null }))

  assert.deepEqual(harness.historyCalls, [])
  assert.deepEqual(harness.promptHistoryCalls, [{ sessionId: "session-1", generation: 1 }])
  assert.deepEqual(harness.replaceCalls, [{ agentId: null, entries: [] }])
  assert.equal(harness.nextHistoryCursor, null)
})

test("attached session prime seeds the visible agent transcript and pane preview", async () => {
  const harness = primeHarness({
    outline: outlineForAgents(["agent-1"], "hello\n", "world"),
  })

  await harness.controller.prime(session())

  assert.deepEqual(harness.historyCalls, [{ sessionId: "session-1", agentIds: ["agent-1"] }])
  assert.deepEqual(harness.promptHistoryCalls, [{ sessionId: "session-1", generation: 1 }])
  assert.deepEqual(harness.agentPaneEntries["agent-1"]?.map((entry) => entry.text), ["hello", "world"])
  assert.equal(harness.agentPanePreviews["agent-1"], "You: hello\nAsst: world")
  assert.deepEqual(harness.replaceCalls, [{
    agentId: "agent-1",
    entries: [
      { id: 1, role: "user", text: "hello" },
      { id: 2, role: "assistant", text: "world" },
    ],
  }])
  assert.equal(harness.nextHistoryCursor, null)
  assert.notEqual(harness.agentPaneEntries["agent-1"]?.[0], harness.replacedEntries[0]?.[0])
})

test("attached session prime preserves the visible agent history cursor", async () => {
  const harness = primeHarness({
    outline: outlineForAgents(["agent-1"], "hello\n", "world", { before_sequence: 12 }),
  })

  await harness.controller.prime(session())

  assert.deepEqual(harness.nextHistoryCursor, {
    agentId: "agent-1",
    cursor: { before_sequence: 12 },
  })
})

test("attached session prime selects the visible split-pane screen", async () => {
  const harness = primeHarness({ split: true, maxAgentsPerScreen: 2 })

  await harness.controller.prime(session({
    focused_agent_id: "agent-3",
    agents: [agent("agent-1"), agent("agent-2"), agent("agent-3"), agent("agent-4")],
  }))

  assert.deepEqual(harness.historyCalls, [{ sessionId: "session-1", agentIds: ["agent-1", "agent-2", "agent-3", "agent-4"] }])
  assert.deepEqual(harness.replaceCalls[0]?.agentId, "agent-3")
})

test("attached session prime projects kernel-scoped history without local import repair", async () => {
  const harness = primeHarness({
    outline: {
      agents: [{
        agent_id: "agent-1",
        turns: [{
          turn_id: "turn-1",
          prompt_id: "external:codex:thread-1:user-1",
          prompt_origin: "external",
          external_provider: "codex",
          external_provider_session_id: "thread-1",
          external_provider_turn_id: "user-1",
          started_at_ms: 1,
          lifecycle: "completed",
          completed_at_ms: 2,
          user_prompt: historyEntry(1, "user_prompt", "codex prompt\n", "agent-1", externalObserved("codex", "thread-1", "user-1")),
          entries: [
            historyEntry(2, "provider_output", "codex output", "agent-1", externalObserved("codex", "thread-1", "assistant-1")),
            historyEntry(3, "provider_output", "opencode output", "agent-1", externalObserved("opencode", "thread-2", "assistant-1")),
          ],
          summary: historyEntry(4, "provider_output", "codex summary", "agent-1", externalObserved("codex", "thread-1", "summary-1")),
          blobs: [],
        }],
        next_cursor: null,
      }],
    },
  })

  await harness.controller.prime(session({
    agents: [agent("agent-1", {
      external_provider_import: externalProviderImport("codex", "codex:thread-1", "thread-1"),
    })],
  }))

  assert.deepEqual(renderableTexts(harness.agentPaneEntries["agent-1"] ?? []), [
    "codex prompt",
    "codex output",
    "opencode output",
    "codex summary",
  ])
  assert.deepEqual(renderableReplaceEntries(harness.replaceCalls.at(-1)?.entries ?? []), [
    { id: 1, role: "user", text: "codex prompt" },
    { id: 3, role: "assistant", text: "codex output" },
    { id: 4, role: "assistant", text: "opencode output" },
    { id: 5, role: "assistant", text: "codex summary" },
  ])
  assert.equal(harness.replaceCalls.at(-1)?.agentId, "agent-1")
})

test("attached session prime filters passive external provider statuses through shared hydration", async () => {
  const harness = primeHarness({
    outline: {
      agents: [{
        agent_id: "agent-1",
        turns: [{
          turn_id: "turn-1",
          prompt_id: "external:codex:thread-1:user-1",
          prompt_origin: "external",
          external_provider: "codex",
          external_provider_session_id: "thread-1",
          external_provider_turn_id: "user-1",
          started_at_ms: 1,
          lifecycle: "open",
          completed_at_ms: null,
          user_prompt: historyEntry(1, "user_prompt", "external prompt\n", "agent-1", externalObserved("codex", "thread-1", "user-1")),
          entries: [
            historyEntry(2, "provider_status", "codex token_count\n{\"total\":42}", "agent-1", {
              ...externalObserved("codex", "thread-1", "token-count"),
              external_observation: {
                settles_active_prompt: false,
                passive_telemetry: true,
              },
            }),
            historyEntry(3, "provider_output", "external output", "agent-1", externalObserved("codex", "thread-1", "assistant-1")),
          ],
          summary: null,
          blobs: [],
        }],
        next_cursor: null,
      }],
    },
  })

  await harness.controller.prime(session())

  assert.deepEqual(renderableTexts(harness.agentPaneEntries["agent-1"] ?? []), [
    "external prompt",
    "external output",
  ])
  assert.deepEqual(renderableReplaceEntries(harness.replaceCalls.at(-1)?.entries ?? []), [
    { id: 1, role: "user", text: "external prompt" },
    { id: 2, role: "assistant", text: "external output" },
  ])
  assert.equal(harness.agentPanePreviews["agent-1"], "You: external prompt\nAsst: external output")
})

function primeHarness(options: {
  split?: boolean
  maxAgentsPerScreen?: number
  outline?: SessionHistoryOutline
} = {}) {
  const harness = {
    promptGeneration: 0,
    historyCalls: [] as Array<{ sessionId: string; agentIds: string[] }>,
    promptHistoryCalls: [] as Array<{ sessionId: string; generation: number }>,
    nextHistoryCursor: undefined as SessionHistoryCursorState | undefined,
    agentPaneEntries: {} as Record<string, TranscriptEntry[]>,
    agentPanePreviews: {} as Record<string, string>,
    replaceCalls: [] as Array<{
      agentId: string | null
      entries: Array<{ id: number; role: TranscriptEntry["role"]; text: string }>
    }>,
    replacedEntries: [] as TranscriptEntry[][],
    controller: null as ReturnType<typeof createAttachedSessionPrimeController> | null,
  }
  harness.controller = createAttachedSessionPrimeController({
    promptHistoryHydrationController: {
      begin: () => {
        harness.promptGeneration += 1
        return harness.promptGeneration
      },
      loadAndApply: async (sessionId, generation) => {
        harness.promptHistoryCalls.push({ sessionId, generation })
      },
    },
    splitAgentResponseMode: () => options.split ?? false,
    maxAgentsPerScreen: () => options.maxAgentsPerScreen ?? 3,
    loadSessionHistoryOutline: async (sessionId, agentIds) => {
      harness.historyCalls.push({ sessionId, agentIds: [...agentIds] })
      return options.outline ?? outlineForAgents([...agentIds])
    },
    setAgentPaneEntries: (agentId, entries) => {
      harness.agentPaneEntries[agentId] = entries
    },
    setAgentPanePreview: (agentId, preview) => {
      harness.agentPanePreviews[agentId] = preview
    },
    replaceTranscriptEntries: (entries, agentId) => {
      harness.replacedEntries.push(entries)
      harness.replaceCalls.push({
        agentId,
        entries: entries.map((entry) => ({
          id: entry.id,
          role: entry.role,
          text: entry.text,
        })),
      })
    },
    setNextHistoryCursor: (cursor) => {
      harness.nextHistoryCursor = cursor
    },
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createAttachedSessionPrimeController>
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 8,
    agents: [agent("agent-1")],
    config_state: { version: 1, values: {} },
    ...overrides,
  }
}

function renderableTexts(entries: readonly TranscriptEntry[]): string[] {
  return entries
    .filter((entry) => entry.text !== "click to collapse")
    .map((entry) => entry.text)
}

function renderableReplaceEntries(
  entries: readonly { id: number; role: TranscriptEntry["role"]; text: string }[],
): Array<{ id: number; role: TranscriptEntry["role"]; text: string }> {
  return entries.filter((entry) => entry.text !== "click to collapse")
}

function agent(id: string, overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "gpt-5.4",
    worktree_id: null,
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

function historyEntry(
  entryIndex: number,
  kind: "user_prompt" | "provider_output" | "provider_status",
  text: string,
  agentId: string,
  metadata: Record<string, unknown> = {},
) {
  return {
    entry_index: entryIndex,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: { kind, text, agent_id: agentId, ...metadata },
  }
}

function externalObserved(
  provider: string,
  sessionId: string,
  turnId: string,
): Record<string, unknown> {
  return {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_provider: provider,
    external_provider_session_id: sessionId,
    external_provider_turn_id: turnId,
  }
}

function outlineForAgents(
  agentIds: string[],
  prompt = "hello\n",
  summaryPrefix = "history for",
  nextCursor: SessionHistoryOutline["agents"][number]["next_cursor"] = null,
): SessionHistoryOutline {
  return {
    agents: agentIds.map((agentId, index) => ({
      agent_id: agentId,
      turns: [{
        turn_id: `${agentId}-turn-1`,
        prompt_id: `${agentId}-prompt-1`,
        started_at_ms: 1,
        lifecycle: "completed",
        completed_at_ms: 2,
        user_prompt: historyEntry(index * 2, "user_prompt", prompt, agentId),
        entries: [],
        summary: historyEntry(index * 2 + 1, "provider_output", summaryPrefix === "world" ? "world" : `${summaryPrefix} ${agentId}`, agentId),
        blobs: [],
      }],
      next_cursor: nextCursor,
    })),
  }
}

function externalProviderImport(
  provider: string,
  externalSessionId: string,
  providerSessionId: string,
) {
  return {
    external_provider: provider,
    external_provider_session_id: externalSessionId,
    external_provider_session_provider_id: providerSessionId,
    observed_cursor: {},
    imported_at_ms: 1,
  }
}
