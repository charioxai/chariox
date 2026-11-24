import assert from "node:assert/strict"
import test from "node:test"

import {
  roomListActivityBadgeLabel,
  roomListItemActivityBadgeState,
  roomListItemActivityHasUnreadIdleOutput,
  roomListItemActivityHasWork,
  roomListItemActivityWorkLabel,
  roomListLifecycleStatusLabel,
  roomListSessionActivityBadgeState,
  roomListSessionActivityHasUnreadIdleOutput,
  roomListSessionActivityHasWork,
  roomListSessionActivityNextAction,
  roomListSessionRemoteActivityLabel,
  roomListSessionActivityWorkLabel,
  roomListSessionRecencyMs,
  roomListSessionStatusLabel,
  roomListTimestampLabel,
} from "./room-list-activity.js"

test("Room list timestamp label formats explicit IANA zones with stable missing fallback", () => {
  const timestamp = Date.UTC(2026, 0, 2, 10, 30)
  assert.equal(roomListTimestampLabel(timestamp, { timeZone: "UTC" }), "2026-01-02 10:30 UTC")
  assert.equal(roomListTimestampLabel(timestamp, { timeZone: "Europe/Helsinki" }), "2026-01-02 12:30 GMT+2")
  assert.equal(roomListTimestampLabel(timestamp, { timeZone: "UTC", includeTimeZone: false }), "2026-01-02 10:30")
  assert.equal(roomListTimestampLabel(null), "-")
  assert.equal(roomListTimestampLabel(Number.NaN, { missingLabel: "missing" }), "missing")
})

test("Room list timestamp label follows IANA daylight-saving transitions", () => {
  assert.equal(
    roomListTimestampLabel(Date.UTC(2026, 2, 8, 6, 30), { timeZone: "America/New_York" }),
    "2026-03-08 01:30 EST",
  )
  assert.equal(
    roomListTimestampLabel(Date.UTC(2026, 2, 8, 7, 30), { timeZone: "America/New_York" }),
    "2026-03-08 03:30 EDT",
  )
})

test("MP-08 session recency reads semantic activity and ignores prompt and last-used fallbacks", () => {
  assert.equal(roomListSessionRecencyMs({
    last_prompt_sent_at_ms: 40,
    last_activity_at_ms: 30,
    last_used_at_ms: 10,
    created_at_ms: 20,
  }), 30)
  assert.equal(roomListSessionRecencyMs({
    last_prompt_sent_at_ms: null,
    last_activity_at_ms: 30,
    last_used_at_ms: 10,
    created_at_ms: 20,
  }), 30)
  assert.equal(roomListSessionRecencyMs({
    last_prompt_sent_at_ms: null,
    last_activity_at_ms: null,
    last_used_at_ms: 10,
    created_at_ms: 20,
  }), 20)
  assert.equal(roomListSessionRecencyMs({
    last_prompt_sent_at_ms: null,
    last_activity_at_ms: null,
    last_used_at_ms: null,
    created_at_ms: 20,
  }), 20)
  assert.equal(roomListSessionRecencyMs({
    last_prompt_sent_at_ms: Number.NaN,
    last_activity_at_ms: null,
    last_used_at_ms: null,
    created_at_ms: 20,
  }), 20)
})

test("Room list session activity predicates derive work and unread output", () => {
  assert.equal(roomListSessionActivityHasWork(null), false)
  assert.equal(roomListSessionActivityHasWork({
    working_agent_count: 0,
    active_prompt_count: 0,
    queued_prompt_count: 0,
  }), false)
  assert.equal(roomListSessionActivityHasWork({
    working_agent_count: 1,
    active_prompt_count: 0,
    queued_prompt_count: 0,
  }), true)
  assert.equal(roomListSessionActivityHasWork({
    working_agent_count: 0,
    active_prompt_count: 0,
    queued_prompt_count: 1,
  }), true)
  assert.equal(roomListSessionActivityHasUnreadIdleOutput(null), false)
  assert.equal(roomListSessionActivityHasUnreadIdleOutput({ unread_idle_agent_count: 0 }), false)
  assert.equal(roomListSessionActivityHasUnreadIdleOutput({ unread_idle_agent_count: 1 }), true)
  assert.equal(roomListSessionActivityBadgeState(null), "none")
  assert.equal(roomListSessionActivityBadgeState({
    working_agent_count: 0,
    active_prompt_count: 0,
    queued_prompt_count: 0,
    unread_idle_agent_count: 1,
  }), "done")
  assert.equal(roomListSessionActivityBadgeState({
    working_agent_count: 1,
    active_prompt_count: 0,
    queued_prompt_count: 0,
    unread_idle_agent_count: 0,
  }), "working")
  assert.equal(roomListSessionActivityBadgeState({
    working_agent_count: 0,
    active_prompt_count: 0,
    queued_prompt_count: 1,
    unread_idle_agent_count: 0,
  }), "queued")
  assert.equal(roomListSessionActivityBadgeState({
    working_agent_count: 0,
    active_prompt_count: 0,
    queued_prompt_count: 1,
    unread_idle_agent_count: 1,
  }), "mixedQueuedDone")
  assert.equal(roomListSessionActivityBadgeState({
    working_agent_count: 1,
    active_prompt_count: 0,
    queued_prompt_count: 0,
    unread_idle_agent_count: 1,
  }), "mixedWorkingDone")
  assert.equal(roomListSessionActivityWorkLabel(null), "-")
  assert.equal(roomListSessionActivityWorkLabel({
    working_agent_count: 1,
    active_prompt_count: 1,
    queued_prompt_count: 2,
  }), "1 top-level agent working, 1 active prompt, 2 queued prompts")
})

test("Room list status labels normalize lifecycle state and activity overrides", () => {
  assert.equal(roomListLifecycleStatusLabel("active"), "Active")
  assert.equal(roomListLifecycleStatusLabel("remote_active"), "Remote Active")
  assert.equal(roomListLifecycleStatusLabel(""), "-")
  assert.equal(roomListLifecycleStatusLabel(null, "unknown"), "unknown")
  assert.equal(roomListActivityBadgeLabel("none"), null)
  assert.equal(roomListActivityBadgeLabel("queued"), "Queued")
  assert.equal(roomListActivityBadgeLabel("working"), "Working")
  assert.equal(roomListActivityBadgeLabel("done"), "Done")
  assert.equal(roomListActivityBadgeLabel("mixedQueuedDone"), "Queued+Done")
  assert.equal(roomListActivityBadgeLabel("mixedWorkingDone"), "Working+Done")
  assert.equal(roomListSessionStatusLabel({ status: "active", activity: null }), "Active")
  assert.equal(roomListSessionStatusLabel({
    status: "active",
    activity: {
      working_agent_count: 0,
      active_prompt_count: 0,
      queued_prompt_count: 1,
      unread_idle_agent_count: 0,
    },
  }), "Queued")
  assert.equal(roomListSessionStatusLabel({
    status: "active",
    activity: {
      working_agent_count: 1,
      active_prompt_count: 0,
      queued_prompt_count: 0,
      unread_idle_agent_count: 1,
    },
  }), "Working+Done")
})

test("Room list session activity next action surfaces recovery policy", () => {
  assert.equal(roomListSessionActivityNextAction(null), null)
  assert.equal(roomListSessionActivityNextAction({ activity: {
    missing_worker_provider_run_count: 1,
    remote_extension_sync_issue_count: 1,
    remote_extension_pending_revoke_count: 0,
  } }), "run /kernel remote-runtime; identify the affected remote/slice agent and worker before sending prompts to that agent")
  assert.equal(roomListSessionActivityNextAction({ activity: {
    missing_worker_provider_run_count: 0,
    remote_extension_sync_issue_count: 1,
    remote_extension_pending_revoke_count: 1,
  } }), "keep the home revoke in place; run /kernel remote-runtime to identify affected agents, then use /extension sync-status and /extension sync-retry after the worker reconnects")
})

test("Room list session remote activity label summarizes remote runtime gaps", () => {
  assert.equal(roomListSessionRemoteActivityLabel(null), "")
  assert.equal(roomListSessionRemoteActivityLabel(null, "-"), "-")
  assert.equal(roomListSessionRemoteActivityLabel({
    remote_agent_count: 1,
    missing_worker_provider_run_count: 2,
    home_proxy_agent_count: 1,
    remote_extension_sync_issue_count: 1,
    remote_extension_pending_revoke_count: 2,
  }), "1 remote/slice agent, 2 worker run gaps, 1 home-proxy agent, 1 extension sync issue, 2 pending revokes")
})

test("Room list item activity predicates derive work and unread output", () => {
  assert.equal(roomListItemActivityHasWork(null), false)
  assert.equal(roomListItemActivityHasWork({
    working: false,
    active_prompt_count: 0,
    queued_prompt_count: 0,
  }), false)
  assert.equal(roomListItemActivityHasWork({
    working: true,
    active_prompt_count: 0,
    queued_prompt_count: 0,
  }), true)
  assert.equal(roomListItemActivityHasWork({
    working: false,
    active_prompt_count: 1,
    queued_prompt_count: 0,
  }), true)
  assert.equal(roomListItemActivityHasUnreadIdleOutput(null), false)
  assert.equal(roomListItemActivityHasUnreadIdleOutput({ unread_idle_output: false }), false)
  assert.equal(roomListItemActivityHasUnreadIdleOutput({ unread_idle_output: true }), true)
  assert.equal(roomListItemActivityBadgeState(null), "none")
  assert.equal(roomListItemActivityBadgeState({
    working: false,
    active_prompt_count: 0,
    queued_prompt_count: 0,
    unread_idle_output: true,
  }), "done")
  assert.equal(roomListItemActivityBadgeState({
    working: true,
    active_prompt_count: 0,
    queued_prompt_count: 0,
    unread_idle_output: true,
  }), "mixedWorkingDone")
  assert.equal(roomListItemActivityBadgeState({
    working: false,
    active_prompt_count: 0,
    queued_prompt_count: 1,
    unread_idle_output: false,
  }), "queued")
  assert.equal(roomListItemActivityBadgeState({
    working: false,
    active_prompt_count: 0,
    queued_prompt_count: 1,
    unread_idle_output: true,
  }), "mixedQueuedDone")
  assert.equal(roomListItemActivityWorkLabel({
    working: true,
    active_prompt_count: 1,
    queued_prompt_count: 2,
  }), "working, 1 active prompt, 2 queued prompts")
  assert.equal(roomListItemActivityWorkLabel(undefined), "active work")
})

// MP-08 / MP-10 / MP-11: Unattached utility reviews remain discoverable.
test("pending review names the existing attachment path without declaring provider work", () => {
  const session = {status: "Ready", activity: {pending_interaction_count: 1, active_prompt_count: 0, queued_prompt_count: 0, working_agent_count: 0}};
  assert.equal(roomListSessionStatusLabel(session), "Needs you");
  assert.equal(roomListSessionActivityHasWork(session.activity), false);
  assert.equal(roomListSessionActivityNextAction(session), "Attach to this session to review the pending request");
});
