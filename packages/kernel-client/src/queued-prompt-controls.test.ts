import assert from "node:assert/strict"
import test from "node:test"

import {
  QUEUED_PROMPT_STALE_REASON,
  normalizeQueuedPromptStatus,
  projectQueuedPrompt,
  queuedPromptActionability,
  queuedPromptActionabilityMatches,
  queuedPromptControlForPrompt,
  queuedPromptProjectionForAgent,
  queuedPromptsForAgent,
  queuedPromptStatusIsQueued,
} from "./queued-prompt-controls.js"
import type { PromptQueueItem, RuntimeSession } from "./kernel-types.js"

test("queued prompt actionability defaults queued prompts to both actions", () => {
  assert.deepEqual(queuedPromptActionability(undefined), {
    status: "queued",
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  })
})

test("queued prompt actionability marks non-queued prompts stale", () => {
  assert.deepEqual(queuedPromptActionability(" Cancelled "), {
    status: "cancelled",
    steerDisabled: true,
    canSteer: false,
    canCancel: false,
    steerDisabledReason: QUEUED_PROMPT_STALE_REASON,
    cancelDisabledReason: QUEUED_PROMPT_STALE_REASON,
  })
})

test("queued prompt actionability prefers kernel projected controls", () => {
  assert.deepEqual(queuedPromptActionability("queued", {
    prompt_id: "prompt-1",
    status: "dispatching",
    can_steer: false,
    can_cancel: true,
    steer_disabled_reason: "kernel says external turn",
    cancel_disabled_reason: null,
  }), {
    status: "dispatching",
    steerDisabled: true,
    canSteer: false,
    canCancel: true,
    steerDisabledReason: "kernel says external turn",
    cancelDisabledReason: null,
  })
})

test("queued prompt actionability comparison includes status and controls", () => {
  const current = queuedPromptActionability("queued")
  assert.equal(queuedPromptActionabilityMatches(current, {
    status: "queued",
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  }), true)

  assert.equal(queuedPromptActionabilityMatches(current, {
    ...current,
    canSteer: false,
  }), false)
  assert.equal(queuedPromptActionabilityMatches(current, {
    ...current,
    steerDisabledReason: "kernel reason",
  }), false)
})

test("queued prompt control lookup requires matching projected prompt identity", () => {
  const controls = {
    "prompt-1": {
      prompt_id: "prompt-1",
      status: "dispatching",
    },
    "prompt-2": {
      prompt_id: "other-prompt",
      status: "dispatching",
    },
    "prompt-3": {
      status: "dispatching",
    },
    "prompt-4": null,
  }
  assert.deepEqual(queuedPromptControlForPrompt(controls, "prompt-1"), {
    prompt_id: "prompt-1",
    status: "dispatching",
  })
  assert.equal(queuedPromptControlForPrompt(controls, "prompt-2"), null)
  assert.deepEqual(queuedPromptControlForPrompt(controls, "prompt-3"), {
    status: "dispatching",
  })
  assert.equal(queuedPromptControlForPrompt(controls, "missing"), null)
  assert.equal(queuedPromptControlForPrompt(controls, null), null)
  assert.equal(queuedPromptControlForPrompt(null, "prompt-1"), null)
})

test("queued prompt actionability does not mark unavailable action disabled without reason", () => {
  assert.deepEqual(queuedPromptActionability("queued", {
    can_steer: false,
    can_cancel: false,
  }), {
    status: "queued",
    steerDisabled: false,
    canSteer: false,
    canCancel: false,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  })
})

test("queued prompt status helpers normalize queue vocabulary", () => {
  assert.equal(normalizeQueuedPromptStatus(" Queued "), "queued")
  assert.equal(normalizeQueuedPromptStatus(""), "queued")
  assert.equal(queuedPromptStatusIsQueued(" queued "), true)
  assert.equal(queuedPromptStatusIsQueued("running"), false)
})

test("queued prompts for agent prefer authoritative prompt states", () => {
  const staleTopLevel = prompt("stale-top-level")
  const promptStatePrompt = prompt("prompt-state")
  const session = sessionWith({
    queued_prompts: [staleTopLevel],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [promptStatePrompt],
      },
    },
  })

  assert.deepEqual(queuedPromptsForAgent(session, "agent-1"), [promptStatePrompt])
})

test("queued prompt projection returns display prompts with actionability", () => {
  const session = sessionWith({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [prompt("queued-1")],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        queued_prompt_controls: {
          "queued-1": {
            prompt_id: "queued-1",
            status: "dispatching",
            can_steer: false,
            can_cancel: true,
            steer_disabled_reason: "Kernel projected steer reason.",
            cancel_disabled_reason: null,
          },
        },
      },
    },
  })

  assert.deepEqual(queuedPromptProjectionForAgent(session, "agent-1"), {
    action: "replace",
    prompts: [{
      id: "queued-1",
      pendingPromptId: null,
      sourceAttachmentId: "attachment-1",
      targetAgentId: "agent-1",
      prompt: "queued-1",
      attachmentCount: 0,
      status: "dispatching",
      steerDisabled: true,
      canSteer: false,
      canCancel: true,
      steerDisabledReason: "Kernel projected steer reason.",
      cancelDisabledReason: null,
    }],
  })
})

test("queued prompt projection preserves transcript when projected busy omits queue detail", () => {
  const session = sessionWith({
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(queuedPromptProjectionForAgent(session, "agent-1"), { action: "preserve" })
})

test("project queued prompt records attachment count and fallback target", () => {
  assert.deepEqual(projectQueuedPrompt({
    id: "queued-1",
    source_attachment_id: "attachment-1",
    prompt: "queued",
    attachments: [{ url: "file:///tmp/a.txt", mime: "text/plain", filename: "a.txt" }],
    status: "Queued",
  }, {
    fallbackTargetAgentId: "agent-fallback",
  }), {
    id: "queued-1",
    pendingPromptId: null,
    sourceAttachmentId: "attachment-1",
    targetAgentId: "agent-fallback",
    prompt: "queued",
    attachmentCount: 1,
    status: "queued",
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  })
})

test("project queued prompt uses pending prompt id as action identity", () => {
  assert.deepEqual(projectQueuedPrompt({
    id: "draft-queued",
    pending_prompt_id: "pending-prompt-1",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "queued",
    attachments: [],
    status: "queued",
  }), {
    id: "pending-prompt-1",
    pendingPromptId: "pending-prompt-1",
    sourceAttachmentId: "attachment-1",
    targetAgentId: "agent-1",
    prompt: "queued",
    attachmentCount: 0,
    status: "queued",
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  })
})

test("queued prompts for agent clear stale queues when projected activity is idle", () => {
  const queued = prompt("queued-stale")
  const session = sessionWith({
    queued_prompts: [queued],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(queuedPromptsForAgent(session, "agent-1"), [])
})

test("queued prompts for agent preserve existing transcript when projected busy omits queue detail", () => {
  const session = sessionWith({
    queued_prompts: [],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(queuedPromptsForAgent(session, "agent-1"), null)
})

test("queued prompts for agent falls back to legacy top-level queues without projections", () => {
  const queuedForAgent = prompt("queued-agent")
  const queuedForOther = prompt("queued-other", "agent-2")
  const session = sessionWith({
    queued_prompts: [queuedForOther, queuedForAgent],
  })

  assert.deepEqual(queuedPromptsForAgent(session, "agent-1"), [queuedForAgent])
})

function sessionWith(overrides: Partial<RuntimeSession>): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    workspace_id: "/repo",
    worktree_id: "/repo",
    created_at_ms: 1,
    status: "Active",
    agent_defaults: {
      provider: "codex",
      model: null,
      effort: null,
      account_profile: null,
      execution_mode: "build",
      permission_level: "yolo",
    },
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 4,
    agents: [{
      id: "agent-1",
      agent_ref: "agent-1",
      session_id: "session-1",
      alias: "agent-1",
      provider: "codex",
      model: null,
      effort: null,
      account_profile: null,
      state: "Idle",
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 1,
      last_activity_at_ms: 1,
      worktree_id: "/repo",
    }],
    workflows: [],
    workflow_runs: [],
    workflow_watchdogs: [],
    workflow_consoles: [],
    config_state: {
      version: 1,
      values: {},
      updated_by_attachment_id: null,
    },
    ...overrides,
  }
}

function prompt(id: string, agentId = "agent-1"): PromptQueueItem {
  return {
    id,
    source_attachment_id: "attachment-1",
    target_agent_id: agentId,
    prompt: id,
    status: "Queued",
  }
}
