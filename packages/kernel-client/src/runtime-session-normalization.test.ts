import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "./kernel-types.js"
import {
  normalizeAgentPromptState,
  normalizeRuntimeSession,
  normalizeRuntimeSessions,
  normalizeRuntimeSessionWithAgentActivity,
} from "./runtime-session-normalization.js"

test("runtime session normalization fills missing runtime arrays and prompt states", () => {
  const normalized = normalizeRuntimeSession({
    ...session(),
    agents: [{ id: "agent-1" }, { id: "agent-2" }] as never,
    queued_prompts: null as never,
    active_interactions: null as never,
    metaagent_tasks: null as never,
    workflows: null as never,
    workflow_publications: null as never,
    workflow_runs: null as never,
    workflow_prompt_queues: null as never,
    workflow_queued_prompts: null as never,
    workflow_schedules: null as never,
    workflow_consoles: null as never,
    workspace_links: null as never,
    external_provider_imports: null as never,
    prompt_states: {
      "agent-1": {
        active_prompt: { id: "prompt-1" } as never,
      } as never,
      "agent-2": null as never,
      "agent-ghost": {
        active_prompt: { id: "prompt-ghost" } as never,
        queued_prompts: [{ id: "queued-ghost" }] as never,
      } as never,
    },
  })

  assert.deepEqual(normalized.queued_prompts, [])
  assert.deepEqual(normalized.active_interactions, [])
  assert.deepEqual(normalized.metaagent_tasks, [])
  assert.deepEqual(normalized.workflows, [])
  assert.deepEqual(normalized.workflow_publications, [])
  assert.deepEqual(normalized.workflow_runs, [])
  assert.deepEqual(normalized.workflow_prompt_queues, [])
  assert.deepEqual(normalized.workflow_queued_prompts, [])
  assert.deepEqual(normalized.workflow_schedules, [])
  assert.deepEqual(normalized.workflow_consoles, [])
  assert.deepEqual(normalized.workspace_links, [])
  assert.deepEqual(normalized.external_provider_imports, [])
  assert.deepEqual(normalized.prompt_states?.["agent-1"], {
    active_prompt: { id: "prompt-1" },
    queued_prompts: [],
  })
  assert.deepEqual(normalized.prompt_states?.["agent-2"], {
    active_prompt: null,
    queued_prompts: [],
  })
  assert.equal(Object.hasOwn(normalized.prompt_states ?? {}, "agent-ghost"), false)
})

test("runtime session normalization filters activity to session agents", () => {
  const normalized = normalizeRuntimeSession({
    ...session(),
    agents: [{ id: "agent-1" }] as never,
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
      "agent-ghost": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(Object.keys(normalized.agent_activity ?? {}), ["agent-1"])
})

test("runtime session normalization preserves empty authoritative projections after filtering", () => {
  const normalized = normalizeRuntimeSession({
    ...session(),
    agents: [{ id: "agent-1" }] as never,
    prompt_states: {
      "agent-ghost": {
        active_prompt: { id: "prompt-ghost" } as never,
        queued_prompts: [{ id: "queued-ghost" }] as never,
      } as never,
    },
    agent_activity: {
      "agent-ghost": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(normalized.prompt_states, {})
  assert.deepEqual(normalized.agent_activity, {})
})

test("runtime session normalization can apply projected agent activity payloads", () => {
  const normalized = normalizeRuntimeSessionWithAgentActivity({
    session: {
      ...session(),
      agents: [{ id: "agent-1" }] as never,
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    agent_activity_revision: 7,
  })

  assert.equal(normalized.agent_activity?.["agent-1"]?.busy, true)
  assert.equal(normalized.agent_activity_revision, 7)
})

test("runtime session normalization filters projected activity payloads to session agents", () => {
  const normalized = normalizeRuntimeSessionWithAgentActivity({
    session: {
      ...session(),
      agents: [{ id: "agent-1" }] as never,
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
      "agent-ghost": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    agent_activity_revision: 8,
  })

  assert.deepEqual(Object.keys(normalized.agent_activity ?? {}), ["agent-1"])
  assert.equal(normalized.agent_activity?.["agent-ghost"], undefined)
  assert.equal(normalized.agent_activity_revision, 8)
})

test("runtime session normalization preserves empty projected activity as authoritative", () => {
  const normalized = normalizeRuntimeSessionWithAgentActivity({
    session: {
      ...session(),
      agents: [{ id: "agent-1" }] as never,
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
        },
      },
      agent_activity_revision: 6,
    },
    agent_activity: {
      "agent-ghost": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    agent_activity_revision: 8,
  })

  assert.deepEqual(normalized.agent_activity, {})
  assert.equal(normalized.agent_activity_revision, 8)
})

test("runtime session normalization canonicalizes legacy workflow watchdog arrays", () => {
  const legacyOnly = normalizeRuntimeSession({
    ...session(),
    workflow_schedules: null as never,
    workflow_watchdogs: [{ id: "watchdog-legacy" }] as never,
  })

  assert.deepEqual(legacyOnly.workflow_schedules, [{ id: "watchdog-legacy" }])
  assert.equal(Object.hasOwn(legacyOnly, "workflow_watchdogs"), false)

  const canonicalWins = normalizeRuntimeSession({
    ...session(),
    workflow_schedules: [{ id: "schedule-1" }] as never,
    workflow_watchdogs: [{ id: "watchdog-legacy" }] as never,
  })

  assert.deepEqual(canonicalWins.workflow_schedules, [{ id: "schedule-1" }])
  assert.equal(Object.hasOwn(canonicalWins, "workflow_watchdogs"), false)
})

test("runtime session normalization clears stale activity revision when replacement has no revision", () => {
  const normalized = normalizeRuntimeSessionWithAgentActivity({
    session: {
      ...session(),
      agents: [{ id: "agent-1" }] as never,
      agent_activity: {
        "agent-stale": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
        },
      },
      agent_activity_revision: 6,
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(Object.keys(normalized.agent_activity ?? {}), ["agent-1"])
  assert.equal(normalized.agent_activity_revision, undefined)
})

test("runtime session normalization clears stale embedded activity when projection is explicitly absent", () => {
  const normalized = normalizeRuntimeSessionWithAgentActivity({
    session: {
      ...session(),
      agents: [{ id: "agent-1" }] as never,
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
        },
      },
      agent_activity_revision: 6,
    },
    agent_activity: null,
    agent_activity_revision: 7,
  })

  assert.equal(normalized.agent_activity, undefined)
  assert.equal(normalized.agent_activity_revision, undefined)
})

test("runtime session normalization preserves embedded activity without a top-level projection override", () => {
  const normalized = normalizeRuntimeSessionWithAgentActivity({
    session: {
      ...session(),
      agents: [{ id: "agent-1" }] as never,
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
        },
      },
      agent_activity_revision: 6,
    },
  })

  assert.equal(normalized.agent_activity?.["agent-1"]?.busy, true)
  assert.equal(normalized.agent_activity_revision, 6)
})

test("runtime session normalization handles prompt state lists and session lists", () => {
  assert.deepEqual(normalizeAgentPromptState({
    active_prompt: null,
    queued_prompts: null as never,
  }), {
    active_prompt: null,
    queued_prompts: [],
  })
  assert.deepEqual(normalizeRuntimeSessions([
    { ...session(), id: "session-1", queued_prompts: null as never },
    { ...session(), id: "session-2", queued_prompts: null as never },
  ]).map((item) => [item.id, item.queued_prompts]), [
    ["session-1", []],
    ["session-2", []],
  ])
})

function session(): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    created_at_ms: 1,
    status: "Created",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 4,
    agents: [],
    config_state: {
      version: 1,
      values: {},
      updated_by_attachment_id: null,
    },
  }
}
