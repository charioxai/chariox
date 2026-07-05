import assert from "node:assert/strict"
import test from "node:test"

import {
  waitingRoomItemActivityHasUnreadIdleOutput,
  waitingRoomItemActivityHasWork,
  waitingRoomSessionActivityHasUnreadIdleOutput,
  waitingRoomSessionActivityHasWork,
  waitingRoomSessionActivityWorkLabel,
} from "./waiting-room-activity.js"

test("waiting room session activity predicates derive work and unread output", () => {
  assert.equal(waitingRoomSessionActivityHasWork(null), false)
  assert.equal(waitingRoomSessionActivityHasWork({
    working_agent_count: 0,
    active_prompt_count: 0,
    queued_prompt_count: 0,
  }), false)
  assert.equal(waitingRoomSessionActivityHasWork({
    working_agent_count: 1,
    active_prompt_count: 0,
    queued_prompt_count: 0,
  }), true)
  assert.equal(waitingRoomSessionActivityHasWork({
    working_agent_count: 0,
    active_prompt_count: 0,
    queued_prompt_count: 1,
  }), true)
  assert.equal(waitingRoomSessionActivityHasUnreadIdleOutput(null), false)
  assert.equal(waitingRoomSessionActivityHasUnreadIdleOutput({ unread_idle_agent_count: 0 }), false)
  assert.equal(waitingRoomSessionActivityHasUnreadIdleOutput({ unread_idle_agent_count: 1 }), true)
  assert.equal(waitingRoomSessionActivityWorkLabel(null), "-")
  assert.equal(waitingRoomSessionActivityWorkLabel({
    working_agent_count: 1,
    active_prompt_count: 1,
    queued_prompt_count: 2,
  }), "1 working, 1 active prompt, 2 queued prompts")
})

test("waiting room item activity predicates derive work and unread output", () => {
  assert.equal(waitingRoomItemActivityHasWork(null), false)
  assert.equal(waitingRoomItemActivityHasWork({
    working: false,
    active_prompt_count: 0,
    queued_prompt_count: 0,
  }), false)
  assert.equal(waitingRoomItemActivityHasWork({
    working: true,
    active_prompt_count: 0,
    queued_prompt_count: 0,
  }), true)
  assert.equal(waitingRoomItemActivityHasWork({
    working: false,
    active_prompt_count: 1,
    queued_prompt_count: 0,
  }), true)
  assert.equal(waitingRoomItemActivityHasUnreadIdleOutput(null), false)
  assert.equal(waitingRoomItemActivityHasUnreadIdleOutput({ unread_idle_output: false }), false)
  assert.equal(waitingRoomItemActivityHasUnreadIdleOutput({ unread_idle_output: true }), true)
})
