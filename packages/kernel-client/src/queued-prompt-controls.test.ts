import assert from "node:assert/strict"
import test from "node:test"

import {
  QUEUED_PROMPT_CONTROLS_UNAVAILABLE_REASON,
  QUEUED_PROMPT_STALE_REASON,
  normalizeQueuedPromptStatus,
  projectedQueuedPromptListsMatch,
  projectQueuedPrompt,
  queuedPromptActionability,
  queuedPromptActionabilityMatches,
  queuedPromptActionLabel,
  queuedPromptActionState,
  queuedPromptControlForPrompt,
  queuedPromptControlForPromptIds,
  queuedPromptMetaLabel,
  queuedPromptProjectionForAgent,
  queuedPromptStatusLabel,
  queuedPromptsForAgent,
  queuedPromptStatusIsQueued,
  queuedPromptTitleLabel,
  sortProjectedQueuedPrompts,
} from "./queued-prompt-controls.js"
import type { ProjectedQueuedPrompt } from "./queued-prompt-controls.js"
import type { PromptQueueItem, RuntimeSession } from "./kernel-types.js"

test("queued prompt actionability disables queued actions without kernel controls", () => {
  assert.deepEqual(queuedPromptActionability(undefined), {
    status: "queued",
    steerDisabled: true,
    canSteer: false,
    canCancel: false,
    steerDisabledReason: QUEUED_PROMPT_CONTROLS_UNAVAILABLE_REASON,
    cancelDisabledReason: QUEUED_PROMPT_CONTROLS_UNAVAILABLE_REASON,
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

test("queued prompt projection does not infer steering controls from external prompt origin", () => {
  const projected = projectQueuedPrompt({
    ...prompt("prompt-external"),
    prompt_origin: "external",
    status: "queued",
  })

  assert.equal(projected?.promptOrigin, "external")
  assert.equal(projected?.canSteer, false)
  assert.equal(projected?.steerDisabled, true)
  assert.equal(projected?.steerDisabledReason, QUEUED_PROMPT_CONTROLS_UNAVAILABLE_REASON)
})

test("queued prompt projection uses kernel controls to disable external steering", () => {
  const projected = projectQueuedPrompt({
    ...prompt("prompt-external"),
    prompt_origin: "external",
    status: "queued",
  }, {
    control: {
      prompt_id: "prompt-external",
      status: "queued",
      can_steer: false,
      can_cancel: true,
      steer_disabled_reason: "Kernel projected external turn reason.",
    },
  })

  assert.equal(projected?.promptOrigin, "external")
  assert.equal(projected?.canSteer, false)
  assert.equal(projected?.steerDisabled, true)
  assert.equal(projected?.steerDisabledReason, "Kernel projected external turn reason.")
  assert.equal(projected?.canCancel, true)
})

test("queued prompt actionability comparison includes status and controls", () => {
  const current = queuedPromptActionability("queued", enabledControl())
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

test("queued prompt action state follows projected prompt ownership controls", () => {
  const kernelProjectedExternalReason =
    "Steering is unavailable while the active provider turn was started outside Chariox."
  const externalBlocked = queuedPromptActionability("queued", {
    can_steer: false,
    can_cancel: true,
    steer_disabled_reason: kernelProjectedExternalReason,
    cancel_disabled_reason: null,
  })

  assert.deepEqual(queuedPromptActionState(externalBlocked, "steer"), {
    action: "steer",
    enabled: false,
    disabled: true,
    disabledReason: kernelProjectedExternalReason,
  })
  assert.deepEqual(queuedPromptActionState(externalBlocked, "cancel"), {
    action: "cancel",
    enabled: true,
    disabled: false,
    disabledReason: null,
  })
})

test("queued prompt action state treats stale prompts as disabled actions", () => {
  const stale = queuedPromptActionability("dispatching")

  assert.deepEqual(queuedPromptActionState(stale, "steer"), {
    action: "steer",
    enabled: false,
    disabled: true,
    disabledReason: QUEUED_PROMPT_STALE_REASON,
  })
  assert.deepEqual(queuedPromptActionState(stale, "cancel"), {
    action: "cancel",
    enabled: false,
    disabled: true,
    disabledReason: QUEUED_PROMPT_STALE_REASON,
  })
})

test("projected queued prompt comparison includes identity and actionability", () => {
  const current = projectedPrompt({
    id: "queued-1",
    pendingPromptId: "pending-1",
    sourceAttachmentId: "attachment-1",
    targetAgentId: "agent-1",
    prompt: "queued",
    promptOrigin: null,
    attachmentCount: 1,
  })

  assert.equal(projectedQueuedPromptListsMatch([current], [{ ...current }]), true)
  assert.equal(projectedQueuedPromptListsMatch([current], [{
    ...current,
    canSteer: false,
  }]), false)
  assert.equal(projectedQueuedPromptListsMatch([current], [{
    ...current,
    prompt: "edited queued prompt",
  }]), false)
  assert.equal(projectedQueuedPromptListsMatch([current], [{
    ...current,
    promptOrigin: "external",
  }]), false)
  assert.equal(projectedQueuedPromptListsMatch([current], [{
    ...current,
    createdAtMs: 2_000,
  }]), false)
  assert.equal(projectedQueuedPromptListsMatch([current], []), false)
})

test("projected queued prompt sorting uses creation time and stable id order", () => {
  assert.deepEqual(sortProjectedQueuedPrompts([
    projectedPrompt({ id: "queued-c", prompt: "c", createdAtMs: 1_000 }),
    projectedPrompt({ id: "queued-missing", prompt: "missing" }),
    projectedPrompt({ id: "queued-invalid", prompt: "invalid", createdAtMs: Number.NaN }),
    projectedPrompt({ id: "queued-a", prompt: "a", createdAtMs: 1_000 }),
    projectedPrompt({ id: "queued-oldest", prompt: "oldest", createdAtMs: 500 }),
  ]).map((prompt) => prompt.id), [
    "queued-oldest",
    "queued-a",
    "queued-c",
    "queued-invalid",
    "queued-missing",
  ])
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
  assert.equal(queuedPromptControlForPrompt(controls, " "), null)
  assert.equal(queuedPromptControlForPrompt(null, "prompt-1"), null)
})

test("queued prompt control lookup accepts pending and materialized identities", () => {
  assert.deepEqual(queuedPromptControlForPromptIds({
    "materialized-1": {
      prompt_id: "materialized-1",
      status: "dispatching",
    },
  }, ["pending-1", "materialized-1"]), {
    prompt_id: "materialized-1",
    status: "dispatching",
  })

  assert.deepEqual(queuedPromptControlForPromptIds({
    "pending-1": {
      prompt_id: "materialized-1",
      status: "dispatching",
    },
  }, ["pending-1", "materialized-1"]), {
    prompt_id: "materialized-1",
    status: "dispatching",
  })

  assert.equal(queuedPromptControlForPromptIds({
    "pending-1": {
      prompt_id: "other-prompt",
      status: "dispatching",
    },
  }, ["pending-1", "materialized-1"]), null)
})

test("queued prompt actionability marks unavailable steering disabled without reason", () => {
  assert.deepEqual(queuedPromptActionability("queued", {
    can_steer: false,
    can_cancel: false,
  }), {
    status: "queued",
    steerDisabled: true,
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
  assert.equal(queuedPromptStatusLabel("dispatching_prompt"), "dispatching prompt")
  assert.equal(queuedPromptStatusLabel("queued-prompt"), "queued prompt")
  assert.equal(queuedPromptMetaLabel({ status: "Queued", attachmentCount: 0 }), "queued")
  assert.equal(queuedPromptMetaLabel({ status: "dispatching", attachmentCount: 1 }), "dispatching · 1 file")
  assert.equal(queuedPromptMetaLabel({ status: "queued", attachmentCount: 2 }), "queued · 2 files")
  assert.equal(queuedPromptMetaLabel({ status: null, attachmentCount: -1 }), "queued")
})

test("queued prompt strip labels use compact focused controls", () => {
  assert.equal(queuedPromptTitleLabel(2, true), "QUEUE • 2 prompts • J/K select • S steer • C cancel")
  assert.equal(queuedPromptActionLabel("steer", true), "S")
  assert.equal(queuedPromptActionLabel("cancel", true), "C")
})

test("queued prompt strip labels keep unfocused mouse labels descriptive", () => {
  assert.equal(queuedPromptTitleLabel(1, false), "QUEUE • 1 prompt")
  assert.equal(queuedPromptActionLabel("steer", false), "steer")
  assert.equal(queuedPromptActionLabel("cancel", false), "cancel")
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

test("queued prompts for agent clear stale top-level queues when prompt state is empty", () => {
  const staleTopLevel = prompt("stale-top-level")
  const session = sessionWith({
    queued_prompts: [staleTopLevel],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
  })

  assert.deepEqual(queuedPromptsForAgent(session, "agent-1"), [])
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
      promptOrigin: null,
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

test("queued prompt projection does not infer steering policy from external active turn", () => {
  const session = sessionWith({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          ...prompt("external-active"),
          status: "Running",
        },
        queued_prompts: [prompt("queued-1")],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_prompt_count: 1,
        queued_prompt_count: 1,
        active_turn: {
          prompt_id: "external-active",
          prompt_origin: "external",
          external_provider: "codex",
          external_provider_session_id: "provider-session-1",
          external_provider_turn_id: "provider-turn-1",
          status: "running",
          phase: "streaming",
        },
        queued_prompt_controls: {
          "queued-1": enabledControl("queued-1"),
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
      promptOrigin: null,
      attachmentCount: 0,
      status: "queued",
      steerDisabled: false,
      canSteer: true,
      canCancel: true,
      steerDisabledReason: null,
      cancelDisabledReason: null,
    }],
  })
})

test("queued prompt projection returns prompts oldest first", () => {
  const session = sessionWith({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [
          prompt("queued-newer", "agent-1", 3_000),
          prompt("queued-oldest", "agent-1", 1_000),
          prompt("queued-middle", "agent-1", 2_000),
        ],
      },
    },
  })

  const projection = queuedPromptProjectionForAgent(session, "agent-1")

  assert.equal(projection.action, "replace")
  assert.deepEqual(projection.prompts.map((prompt) => prompt.id), [
    "queued-oldest",
    "queued-middle",
    "queued-newer",
  ])
  assert.deepEqual(projection.prompts.map((prompt) => prompt.createdAtMs), [
    1_000,
    2_000,
    3_000,
  ])
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
    created_at_ms: 1_234,
    status: "Queued",
  }, {
    fallbackTargetAgentId: "agent-fallback",
    control: enabledControl("queued-1"),
  }), {
    id: "queued-1",
    pendingPromptId: null,
    sourceAttachmentId: "attachment-1",
    targetAgentId: "agent-fallback",
    prompt: "queued",
    promptOrigin: null,
    createdAtMs: 1_234,
    attachmentCount: 1,
    status: "queued",
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  })
})

test("project queued prompt ignores non-finite creation timestamps", () => {
  const nonFinite = projectQueuedPrompt({
    id: "queued-nan",
    source_attachment_id: "attachment-1",
    prompt: "queued",
    created_at_ms: Number.NaN,
    status: "Queued",
  })
  const malformed = projectQueuedPrompt({
    id: "queued-malformed",
    source_attachment_id: "attachment-1",
    prompt: "queued",
    created_at_ms: "not-a-number",
    status: "Queued",
  } as unknown as PromptQueueItem)

  assert.equal(nonFinite?.createdAtMs, undefined)
  assert.equal(malformed?.createdAtMs, undefined)
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
  }, {
    control: enabledControl("pending-prompt-1"),
  }), {
    id: "pending-prompt-1",
    pendingPromptId: "pending-prompt-1",
    sourceAttachmentId: "attachment-1",
    targetAgentId: "agent-1",
    prompt: "queued",
    promptOrigin: null,
    attachmentCount: 0,
    status: "queued",
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  })
})

test("queued prompt projection applies controls keyed by materialized id for pending prompts", () => {
  const session = sessionWith({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "queued-materialized",
          pending_prompt_id: "queued-pending",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "queued",
          attachments: [],
          status: "queued",
        }],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "queued",
        busy: true,
        queued_prompt_count: 1,
        unread_idle_output: false,
        queued_prompt_controls: {
          "queued-materialized": {
            prompt_id: "queued-materialized",
            status: "dispatching",
            can_steer: false,
            can_cancel: true,
            steer_disabled_reason: "Kernel projected dispatch.",
            cancel_disabled_reason: null,
          },
        },
      },
    },
  })

  assert.deepEqual(queuedPromptProjectionForAgent(session, "agent-1"), {
    action: "replace",
    prompts: [{
      id: "queued-pending",
      pendingPromptId: "queued-pending",
      sourceAttachmentId: "attachment-1",
      targetAgentId: "agent-1",
      prompt: "queued",
      promptOrigin: null,
      attachmentCount: 0,
      status: "dispatching",
      steerDisabled: true,
      canSteer: false,
      canCancel: true,
      steerDisabledReason: "Kernel projected dispatch.",
      cancelDisabledReason: null,
    }],
  })
})

test("project queued prompt ignores blank ids and pending ids", () => {
  assert.equal(projectQueuedPrompt({
    id: " ",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "queued",
    status: "queued",
  }), null)

  assert.deepEqual(projectQueuedPrompt({
    id: " queued-1 ",
    pending_prompt_id: " ",
    source_attachment_id: " ",
    target_agent_id: " ",
    prompt: "queued",
    status: "queued",
  }, {
    fallbackTargetAgentId: " fallback-agent ",
    control: enabledControl("queued-1"),
  }), {
    id: "queued-1",
    pendingPromptId: null,
    sourceAttachmentId: "",
    targetAgentId: "fallback-agent",
    prompt: "queued",
    promptOrigin: null,
    attachmentCount: 0,
    status: "queued",
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  })
})

test("project queued prompt normalizes prompt ownership", () => {
  assert.deepEqual(projectQueuedPrompt({
    id: "queued-external",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "queued",
    status: "queued",
    prompt_origin: " External ",
  }, {
    control: enabledControl("queued-external"),
  }), {
    id: "queued-external",
    pendingPromptId: null,
    sourceAttachmentId: "attachment-1",
    targetAgentId: "agent-1",
    prompt: "queued",
    promptOrigin: "external",
    attachmentCount: 0,
    status: "queued",
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  })

  assert.equal(projectQueuedPrompt({
    id: "external:codex:thread-1:turn-1",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "external queued",
    status: "queued",
  })?.promptOrigin, null)

  assert.deepEqual(projectQueuedPrompt({
    id: "external:codex:thread-1:turn-1",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "external queued",
    status: "queued",
  }, {
    control: enabledControl("external:codex:thread-1:turn-1"),
  }), {
    id: "external:codex:thread-1:turn-1",
    pendingPromptId: null,
    sourceAttachmentId: "attachment-1",
    targetAgentId: "agent-1",
    prompt: "external queued",
    promptOrigin: null,
    attachmentCount: 0,
    status: "queued",
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  })

  assert.deepEqual(projectQueuedPrompt({
    id: "draft-1",
    pending_prompt_id: "external:codex:thread-1:turn-2",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "external queued",
    status: "queued",
  })?.externalProviderTurnId, undefined)

  assert.equal(projectQueuedPrompt({
    id: "queued-external-provider",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "external queued",
    status: "queued",
    external_provider: "codex",
    external_provider_session_id: "thread-1",
  })?.promptOrigin, null)
  assert.equal(projectQueuedPrompt({
    id: "external:codex:thread-1:turn-from-id",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "external queued",
    status: "queued",
    external_provider: "opencode",
    external_provider_session_id: "thread-2",
    external_provider_turn_id: "turn-explicit",
  })?.externalProvider, "opencode")

  assert.equal(projectQueuedPrompt({
    id: "external:codex:thread-1:turn-1",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "chariox-owned queued",
    status: "queued",
    prompt_origin: "chariox",
  })?.promptOrigin, "chariox")
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

test("queued prompts for agent clear stale queues when projected busy has explicit zero queued count", () => {
  const session = sessionWith({
    queued_prompts: [prompt("queued-stale")],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        queued_prompt_count: 0,
      },
    },
  })

  assert.deepEqual(queuedPromptsForAgent(session, "agent-1"), [])
  assert.deepEqual(queuedPromptProjectionForAgent(session, "agent-1"), {
    action: "replace",
    prompts: [],
  })
})

test("queued prompts for agent preserve stale rows when projected busy has queued count without details", () => {
  const session = sessionWith({
    queued_prompts: [],
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        queued_prompt_count: 2,
      },
    },
  })

  assert.equal(queuedPromptsForAgent(session, "agent-1"), null)
})

test("queued prompts for agent preserve existing transcript when explicit queue depth is positive", () => {
  const session = sessionWith({
    queued_prompts: [],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
        queued_prompt_count: 2,
      },
    },
  })

  assert.equal(queuedPromptsForAgent(session, "agent-1"), null)
  assert.deepEqual(queuedPromptProjectionForAgent(session, "agent-1"), {
    action: "preserve",
  })
})

test("queued prompts for agent ignores legacy top-level queues without projections", () => {
  const queuedForAgent = prompt("queued-agent")
  const queuedForOther = prompt("queued-other", "agent-2")
  const session = sessionWith({
    queued_prompts: [queuedForOther, queuedForAgent],
  })

  assert.deepEqual(queuedPromptsForAgent(session, "agent-1"), [])
  assert.deepEqual(queuedPromptProjectionForAgent(session, "agent-1"), {
    action: "ignore",
  })
})

test("queued prompts for agent ignore all legacy top-level queues", () => {
  const queuedForAgent = prompt("queued-agent")
  const queuedTargetless = {
    id: "queued-targetless",
    source_attachment_id: "attachment-1",
    prompt: "queued-targetless",
    status: "Queued",
  }
  const session = sessionWith({
    queued_prompts: [queuedTargetless, queuedForAgent],
  })

  assert.deepEqual(queuedPromptsForAgent(session, "agent-1"), [])
  assert.deepEqual(queuedPromptsForAgent(session, "agent-2"), [])
  assert.deepEqual(queuedPromptProjectionForAgent(session, "agent-1"), {
    action: "ignore",
  })
})

test("queued prompts for agent ignore queues outside session agents", () => {
  const session = sessionWith({
    queued_prompts: [prompt("queued-ghost", "agent-ghost")],
    prompt_states: {
      "agent-ghost": {
        active_prompt: null,
        queued_prompts: [prompt("state-queued-ghost", "agent-ghost")],
      },
    },
    agent_activity: {
      "agent-ghost": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        queued_prompt_controls: {
          "state-queued-ghost": {
            prompt_id: "state-queued-ghost",
            status: "dispatching",
            can_steer: false,
            can_cancel: false,
          },
        },
      },
    },
  })

  assert.deepEqual(queuedPromptsForAgent(session, "agent-ghost"), [])
  assert.deepEqual(queuedPromptProjectionForAgent(session, "agent-ghost"), {
    action: "replace",
    prompts: [],
  })
})

function sessionWith(overrides: Partial<RuntimeSession>): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
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

function prompt(id: string, agentId = "agent-1", createdAtMs?: number): PromptQueueItem {
  return {
    id,
    source_attachment_id: "attachment-1",
    target_agent_id: agentId,
    prompt: id,
    ...(createdAtMs !== undefined ? { created_at_ms: createdAtMs } : {}),
    status: "Queued",
  }
}

function enabledControl(promptId = "queued-1") {
  return {
    prompt_id: promptId,
    status: "queued",
    can_steer: true,
    can_cancel: true,
    steer_disabled_reason: null,
    cancel_disabled_reason: null,
  }
}

function projectedPrompt(
  overrides: Partial<ProjectedQueuedPrompt> = {},
): ProjectedQueuedPrompt {
  return {
    id: "queued-1",
    pendingPromptId: null,
    sourceAttachmentId: "attachment-1",
    targetAgentId: "agent-1",
    prompt: "queued",
    promptOrigin: null,
    attachmentCount: 0,
    status: "queued",
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
    ...overrides,
  }
}
