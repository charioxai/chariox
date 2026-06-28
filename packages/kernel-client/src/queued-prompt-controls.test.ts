import assert from "node:assert/strict"
import test from "node:test"

import {
  QUEUED_PROMPT_STALE_REASON,
  normalizeQueuedPromptStatus,
  queuedPromptActionability,
  queuedPromptStatusIsQueued,
} from "./queued-prompt-controls.js"

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
