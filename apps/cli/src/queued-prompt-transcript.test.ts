import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession, TranscriptEntry } from "./cli-types.js"
import {
  syncQueuedPromptEntriesByAgent,
  syncQueuedPromptEntriesForAgent,
} from "./queued-prompt-transcript.js"

test("syncQueuedPromptEntriesForAgent appends queued prompts and removes settled queued entries", () => {
  const existing: TranscriptEntry[] = [
    { id: 1, role: "assistant", text: "ready" },
    {
      id: 2,
      role: "user",
      text: "old queued",
      queuedPrompt: queuedPrompt("agent-1", "old-prompt"),
    },
  ]

  const synced = syncQueuedPromptEntriesForAgent(existing, sessionWithQueuedPrompt(), "agent-1")

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.entries.map((entry) => entry.queuedPrompt?.promptId).filter(Boolean), [
    "prompt-1",
  ])
  assert.equal(synced.entries.at(-1)?.text, "new queued")
})

test("syncQueuedPromptEntriesForAgent does not infer steering controls from external active prompts", () => {
  const synced = syncQueuedPromptEntriesForAgent(
    [],
    sessionWithQueuedPrompt({
      active_prompt: {
        id: "prompt-external",
        source_attachment_id: "attachment-external",
        target_agent_id: "agent-1",
        prompt: "external running",
        status: "Running",
        prompt_origin: "external",
      },
    }),
    "agent-1",
  )

  assert.equal(synced.changed, true)
  assert.equal(synced.entries[0]?.queuedPrompt?.steerDisabled, false)
  assert.equal(synced.entries[0]?.queuedPrompt?.canSteer, true)
})

test("syncQueuedPromptEntriesForAgent ignores legacy external active prompt origins for steering", () => {
  const synced = syncQueuedPromptEntriesForAgent(
    [],
    sessionWithQueuedPrompt({
      active_prompt: {
        id: "prompt-external",
        source_attachment_id: "attachment-external",
        target_agent_id: "agent-1",
        prompt: "external running",
        status: "Running",
        prompt_origin: " External ",
      },
    }),
    "agent-1",
  )

  assert.equal(synced.changed, true)
  assert.equal(synced.entries[0]?.queuedPrompt?.steerDisabled, false)
  assert.equal(synced.entries[0]?.queuedPrompt?.canSteer, true)
})

test("syncQueuedPromptEntriesForAgent does not infer steering controls from external active turns", () => {
  const synced = syncQueuedPromptEntriesForAgent(
    [],
    sessionWithQueuedPrompt({}, {
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          active_turn: {
            prompt_id: "prompt-external",
            provider_run_id: "run-external",
            prompt_origin: "external",
            status: "running",
            phase: "streaming",
            started_at_ms: 2,
          },
        },
      },
    }),
    "agent-1",
  )

  assert.equal(synced.changed, true)
  assert.equal(synced.entries[0]?.queuedPrompt?.steerDisabled, false)
  assert.equal(synced.entries[0]?.queuedPrompt?.canSteer, true)
})

test("syncQueuedPromptEntriesForAgent ignores projected external active turn origins for steering", () => {
  const synced = syncQueuedPromptEntriesForAgent(
    [],
    sessionWithQueuedPrompt({}, {
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          active_turn: {
            prompt_id: "prompt-external",
            provider_run_id: "run-external",
            prompt_origin: " External ",
            status: "running",
            phase: "streaming",
            started_at_ms: 2,
          },
        },
      },
    }),
    "agent-1",
  )

  assert.equal(synced.changed, true)
  assert.equal(synced.entries[0]?.queuedPrompt?.steerDisabled, false)
  assert.equal(synced.entries[0]?.queuedPrompt?.canSteer, true)
})

test("syncQueuedPromptEntriesForAgent ignores active turn external metadata for steering", () => {
  const synced = syncQueuedPromptEntriesForAgent(
    [],
    sessionWithQueuedPrompt({}, {
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          active_turn: {
            prompt_id: "prompt-external",
            provider_run_id: "run-external",
            external_provider: "codex",
            external_provider_session_id: "thread-1",
            status: "running",
            phase: "streaming",
            started_at_ms: 2,
          },
        },
      },
    }),
    "agent-1",
  )

  assert.equal(synced.changed, true)
  assert.equal(synced.entries[0]?.queuedPrompt?.steerDisabled, false)
  assert.equal(synced.entries[0]?.queuedPrompt?.canSteer, true)
})

test("syncQueuedPromptEntriesForAgent ignores prompt state external metadata for steering", () => {
  const synced = syncQueuedPromptEntriesForAgent(
    [],
    sessionWithQueuedPrompt({
      active_prompt: activePromptWithExternalMetadata(),
    }),
    "agent-1",
  )

  assert.equal(synced.changed, true)
  assert.equal(synced.entries[0]?.queuedPrompt?.steerDisabled, false)
  assert.equal(synced.entries[0]?.queuedPrompt?.canSteer, true)
})

test("syncQueuedPromptEntriesForAgent ignores top-level active prompt external metadata for steering", () => {
  const session = sessionWithoutPromptStates({
    active_prompt: activePromptWithExternalMetadata(),
    queued_prompts: [{
      id: "prompt-queued",
      source_attachment_id: "attachment-queued",
      target_agent_id: "agent-1",
      prompt: "queued after external",
      status: "Queued",
    }],
  })
  const synced = syncQueuedPromptEntriesForAgent([], session, "agent-1")

  assert.equal(synced.changed, true)
  assert.equal(synced.entries[0]?.queuedPrompt?.steerDisabled, false)
  assert.equal(synced.entries[0]?.queuedPrompt?.canSteer, true)
})

test("syncQueuedPromptEntriesForAgent ignores stale active prompt origin when projected activity exists", () => {
  const synced = syncQueuedPromptEntriesForAgent(
    [],
    sessionWithQueuedPrompt({
      active_prompt: {
        id: "prompt-stale",
        source_attachment_id: "attachment-stale",
        target_agent_id: "agent-1",
        prompt: "stale external running",
        status: "Running",
        prompt_origin: "external",
      },
    }, {
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          active_turn: {
            prompt_id: "prompt-arroba",
            provider_run_id: "run-arroba",
            prompt_origin: "arroba",
            status: "running",
            phase: "streaming",
            started_at_ms: 2,
          },
        },
      },
    }),
    "agent-1",
  )

  assert.equal(synced.changed, true)
  assert.equal(synced.entries[0]?.queuedPrompt?.steerDisabled, false)
})

test("syncQueuedPromptEntriesForAgent prefers projected queued prompt controls", () => {
  const synced = syncQueuedPromptEntriesForAgent(
    [],
    sessionWithQueuedPrompt({
      active_prompt: {
        id: "prompt-stale",
        source_attachment_id: "attachment-stale",
        target_agent_id: "agent-1",
        prompt: "stale arroba running",
        status: "Running",
        prompt_origin: "arroba",
      },
      queued_prompts: [{
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "new queued",
        status: "Queued",
      }],
    }, {
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          queued_prompt_controls: {
            "prompt-1": {
              prompt_id: "prompt-1",
              status: "dispatching",
              can_steer: false,
              can_cancel: false,
              steer_disabled_reason: "This prompt is no longer waiting in the queue.",
              cancel_disabled_reason: "This prompt is no longer waiting in the queue.",
            },
          },
          active_turn: {
            prompt_id: "prompt-external",
            provider_run_id: "run-external",
            prompt_origin: "external",
            status: "running",
            phase: "streaming",
            started_at_ms: 2,
          },
        },
      },
    }),
    "agent-1",
  )

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.entries[0]?.queuedPrompt, {
    agentId: "agent-1",
    promptId: "prompt-1",
    status: "dispatching",
    steerDisabled: true,
    canSteer: false,
    canCancel: false,
    steerDisabledReason: "This prompt is no longer waiting in the queue.",
    cancelDisabledReason: "This prompt is no longer waiting in the queue.",
  })
})

test("syncQueuedPromptEntriesForAgent uses projected queue controls to disable steering behind external turns", () => {
  const synced = syncQueuedPromptEntriesForAgent(
    [],
    sessionWithQueuedPrompt({}, {
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          queued_prompt_controls: {
            "prompt-1": {
              prompt_id: "prompt-1",
              status: "queued",
              can_steer: false,
              can_cancel: true,
              steer_disabled_reason: "Kernel projected external turn reason.",
              cancel_disabled_reason: null,
            },
          },
          active_turn: {
            prompt_id: "prompt-external",
            provider_run_id: "run-external",
            prompt_origin: "external",
            status: "running",
            phase: "streaming",
            started_at_ms: 2,
          },
        },
      },
    }),
    "agent-1",
  )

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.entries[0]?.queuedPrompt, {
    agentId: "agent-1",
    promptId: "prompt-1",
    status: "queued",
    steerDisabled: true,
    canSteer: false,
    canCancel: true,
    steerDisabledReason: "Kernel projected external turn reason.",
    cancelDisabledReason: null,
  })
})

test("syncQueuedPromptEntriesForAgent ignores prompt state origin when projected activity is busy without active turn", () => {
  const synced = syncQueuedPromptEntriesForAgent(
    [],
    sessionWithQueuedPrompt({
      active_prompt: {
        id: "prompt-external",
        source_attachment_id: "attachment-external",
        target_agent_id: "agent-1",
        prompt: "external running",
        status: "Running",
        prompt_origin: "external",
      },
    }, {
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
        },
      },
    }),
    "agent-1",
  )

  assert.equal(synced.changed, true)
  assert.equal(synced.entries[0]?.queuedPrompt?.steerDisabled, false)
  assert.equal(synced.entries[0]?.queuedPrompt?.canSteer, true)
})

test("syncQueuedPromptEntriesForAgent removes stale queued prompts when projected activity is idle", () => {
  const existing: TranscriptEntry[] = [{
    id: 1,
    role: "user",
    text: "new queued",
    queuedPrompt: queuedPrompt("agent-1", "prompt-1"),
  }]

  const synced = syncQueuedPromptEntriesForAgent(
    existing,
    sessionWithQueuedPrompt({}, {
      agent_activity: {
        "agent-1": {
          status: "idle",
          prompt_status: "none",
          busy: false,
        },
      },
    }),
    "agent-1",
  )

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.entries, [])
})

test("syncQueuedPromptEntriesForAgent preserves queued prompts when projected busy omits queue state", () => {
  const existing: TranscriptEntry[] = [{
    id: 1,
    role: "user",
    text: "new queued",
    queuedPrompt: queuedPrompt("agent-1", "prompt-1"),
  }]

  const session = sessionWithoutPromptStates({
    queued_prompts: [],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
      },
    },
  })
  const synced = syncQueuedPromptEntriesForAgent(existing, session, "agent-1")

  assert.equal(synced.changed, false)
  assert.deepEqual(synced.entries, existing)
})

test("syncQueuedPromptEntriesForAgent ignores top-level queued prompts when projected busy omits queue state", () => {
  const session = sessionWithoutPromptStates({
    queued_prompts: [{
      id: "prompt-stale",
      source_attachment_id: "attachment-stale",
      target_agent_id: "agent-1",
      prompt: "stale top-level queued",
      status: "Queued",
    }],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
      },
    },
  })
  const synced = syncQueuedPromptEntriesForAgent([], session, "agent-1")

  assert.equal(synced.changed, false)
  assert.deepEqual(synced.entries, [])
})

test("syncQueuedPromptEntriesForAgent clears stale queued prompts when projected queue state is empty", () => {
  const existing: TranscriptEntry[] = [{
    id: 1,
    role: "user",
    text: "new queued",
    queuedPrompt: queuedPrompt("agent-1", "prompt-1"),
  }]

  const session = sessionWithQueuedPrompt({}, {
    prompt_states: {},
    queued_prompts: [],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "none",
        busy: false,
      },
    },
  })
  const synced = syncQueuedPromptEntriesForAgent(existing, session, "agent-1")

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.entries, [])
})

test("syncQueuedPromptEntriesForAgent preserves queued prompts when projected status is working", () => {
  const existing: TranscriptEntry[] = [{
    id: 1,
    role: "user",
    text: "new queued",
    queuedPrompt: queuedPrompt("agent-1", "prompt-1"),
  }]

  const session = sessionWithoutPromptStates({
    queued_prompts: [],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "none",
        busy: false,
      },
    },
  })
  const synced = syncQueuedPromptEntriesForAgent(existing, session, "agent-1")

  assert.equal(synced.changed, false)
  assert.deepEqual(synced.entries, existing)
})

test("syncQueuedPromptEntriesByAgent prunes stale queued prompt panes from authoritative prompt states", () => {
  const synced = syncQueuedPromptEntriesByAgent({
    "agent-stale": [{
      id: 1,
      role: "user",
      text: "stale queued",
      queuedPrompt: queuedPrompt("agent-stale", "queued-stale"),
    }],
  }, sessionWithQueuedPrompt({}, {
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    agents: [{
      ...sessionWithQueuedPrompt().agents[0]!,
      id: "agent-1",
    }],
  }))

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.entriesByAgent["agent-stale"], [])
  assert.deepEqual(synced.previews["agent-stale"], "")
})

test("syncQueuedPromptEntriesByAgent projects activity-only agents", () => {
  const session = sessionWithoutPromptStates({
    agents: [],
    agent_activity: {
      "agent-stale": {
        status: "idle",
        prompt_status: "none",
        busy: false,
      },
    },
  })
  const synced = syncQueuedPromptEntriesByAgent({
    "agent-stale": [{
      id: 1,
      role: "user",
      text: "stale queued",
      queuedPrompt: queuedPrompt("agent-stale", "queued-stale"),
    }],
  }, session)

  assert.equal(synced.changed, true)
  assert.deepEqual(synced.entriesByAgent["agent-stale"], [])
  assert.deepEqual(synced.previews["agent-stale"], "")
})

test("syncQueuedPromptEntriesByAgent preserves stale queued prompt panes without authoritative projection", () => {
  const session = sessionWithoutPromptStates({
    agents: [{
      ...sessionWithQueuedPrompt().agents[0]!,
      id: "agent-1",
    }],
    queued_prompts: [],
  })
  const existing: TranscriptEntry[] = [{
    id: 1,
    role: "user",
    text: "stale queued",
    queuedPrompt: queuedPrompt("agent-stale", "queued-stale"),
  }]
  const synced = syncQueuedPromptEntriesByAgent({
    "agent-stale": existing,
  }, session)

  assert.equal(synced.changed, false)
  assert.deepEqual(synced.entriesByAgent["agent-stale"], existing)
})

function sessionWithoutPromptStates(sessionOverrides: Partial<RuntimeSession> = {}): RuntimeSession {
  const session = sessionWithQueuedPrompt({}, sessionOverrides)
  delete session.prompt_states
  return session
}

function queuedPrompt(agentId: string, promptId: string, steerDisabled = false): NonNullable<TranscriptEntry["queuedPrompt"]> {
  return {
    agentId,
    promptId,
    status: "queued",
    steerDisabled,
    canSteer: !steerDisabled,
    canCancel: true,
    steerDisabledReason: steerDisabled ? "disabled by test" : null,
    cancelDisabledReason: null,
  }
}

function sessionWithQueuedPrompt(
  overrides: Partial<NonNullable<RuntimeSession["prompt_states"]>[string]> = {},
  sessionOverrides: Partial<RuntimeSession> = {},
): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "prompt-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "new queued",
          status: "Queued",
        }],
        ...overrides,
      },
    },
    focused_agent_id: "agent-1",
    max_agents: 1,
    agents: [{
      id: "agent-1",
      agent_ref: "agent-1",
      session_id: "session-1",
      alias: null,
      provider: "codex",
      model: "gpt-5",
      worktree_id: "/workspace/tree",
      state: "Idle",
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 1,
      last_activity_at_ms: 1,
    }],
    config_state: {
      version: 1,
      values: {},
    },
    ...sessionOverrides,
  }
}

function activePromptWithExternalMetadata(): NonNullable<RuntimeSession["active_prompt"]> & {
  external_provider: string
  external_provider_session_id: string
} {
  return {
    id: "prompt-external",
    source_attachment_id: "attachment-external",
    target_agent_id: "agent-1",
    prompt: "external running",
    status: "Running",
    external_provider: "codex",
    external_provider_session_id: "thread-1",
  }
}
