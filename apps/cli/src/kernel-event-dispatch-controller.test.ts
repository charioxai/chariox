import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeProviderRun,
  RuntimeSession,
  TerminalOutputRecord,
} from "./cli-types.js"
import { createKernelEventDispatchController } from "./kernel-event-dispatch-controller.js"

test("kernel event dispatch applies normalized session snapshots with agent activity", async () => {
  const run = providerRun("run-1")
  const nextSession = session()
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "session_snapshot",
    session: nextSession as unknown as Record<string, unknown>,
    provider_run: run as unknown as Record<string, unknown>,
    agent_activity: { "agent-1": { working: true } },
  })

  assert.equal(harness.snapshots.length, 1)
  assert.equal(harness.snapshots[0]?.session.id, "session-1")
  assert.deepEqual(harness.snapshots[0]?.session.agent_activity, { "agent-1": { working: true } })
  assert.equal(harness.snapshots[0]?.providerRun?.id, "run-1")
  assert.deepEqual(harness.calls, [
    "activity:kernel_session_snapshot",
    "refresh-prompt-input-history",
    "apply-session-snapshot:session-1:run-1",
  ])
})

test("kernel event dispatch refreshes waiting-room inventory asynchronously", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "waiting_room_inventory_changed",
    inventory_version: "v2",
  })

  assert.deepEqual(harness.calls, ["refresh-waiting-room"])
})

test("kernel event dispatch resyncs after workflow design events", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "workflow_design_op",
    design_op: {
      session_id: "session-1",
      origin_client_id: "client-1",
      op_id: "op-1",
      kernel_sequence: 1,
      op: {
        kind: "workflow_create",
        workflow: {
          id: "workflow-1",
          alias: "Review",
        },
      },
    },
  })

  assert.deepEqual(harness.calls, [
    "activity:kernel_workflow_design_op",
    "resync:workflow_design_op",
  ])
})

test("kernel event dispatch resyncs after workflow run updates", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "workflow_run_updated",
    session_id: "session-1",
    workflow_run: {
      id: "run-1",
      workflow_id: "workflow-1",
      endpoint_id: "endpoint-1",
      entry_node_id: "node-1",
      status: "Running",
      invocation_prompt: null,
      active_node_run_id: null,
      node_runs: [],
      messages: [],
      created_at_ms: 1,
      started_at_ms: null,
      completed_at_ms: null,
    },
  })

  assert.deepEqual(harness.calls, [
    "activity:kernel_workflow_run_updated",
    "resync:workflow_run_updated",
  ])
})

test("kernel event dispatch handles replay gaps through notice, footer, and resync", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "replay_gap",
    session_id: "session-1",
    requested_from_event_id: 4,
    first_retained_event_id: 9,
    latest_event_id: 12,
    message: "gap",
  })

  assert.deepEqual(harness.calls, [
    "activity:kernel_replay_gap",
    "notice:Missed retained kernel events, refreshed session state.:warning",
    "footer:Missed retained kernel events, refreshed session state.:info",
    "resync:replay_gap",
  ])
})

test("kernel event dispatch recovers after transport close", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "transport_closed",
    message: "connection lost",
  })

  assert.deepEqual(harness.calls, [
    "transport-closed:connection lost",
    "recover-after-restart",
  ])
})

test("kernel event dispatch routes terminal output records and heartbeats", async () => {
  const harness = createHarness()
  const record: TerminalOutputRecord = {
    agent_id: "agent-1",
    kind: "provider_output",
    bytes: [...Buffer.from("hello", "utf8")],
  }

  await harness.controller.handleKernelEvent({
    event: "terminal_output",
    records: [record as unknown as Record<string, unknown>],
  })
  await harness.controller.handleKernelEvent({
    event: "heartbeat",
    session_id: "session-1",
  })

  assert.deepEqual(harness.terminalRecords, [[record]])
  assert.deepEqual(harness.calls, [
    "activity:kernel_terminal_output",
    "queue-terminal-records:1",
    "activity:kernel_heartbeat",
    "refresh-prompt-input-history",
  ])
})

function createHarness() {
  const calls: string[] = []
  const snapshots: Array<{
    session: RuntimeSession
    providerRun: RuntimeProviderRun | null
  }> = []
  const terminalRecords: TerminalOutputRecord[][] = []
  const controller = createKernelEventDispatchController({
    recordDaemonActivity: (activityType) => {
      calls.push(`activity:${activityType}`)
    },
    queueTerminalOutputRecords: (records) => {
      terminalRecords.push(records)
      calls.push(`queue-terminal-records:${records.length}`)
    },
    applyRuntimeNotices: (notices) => {
      calls.push(`runtime-notices:${notices.length}`)
    },
    applyAssistantMessageCompleted: (event) => {
      calls.push(`assistant-completed:${event.agent_id ?? "null"}`)
    },
    applyKernelSessionSnapshot: (nextSession, nextProviderRun) => {
      snapshots.push({ session: nextSession, providerRun: nextProviderRun })
      calls.push(`apply-session-snapshot:${nextSession.id}:${nextProviderRun?.id ?? "null"}`)
    },
    scheduleSharedPromptInputHistoryRefresh: () => {
      calls.push("refresh-prompt-input-history")
    },
    handleKernelSessionUnavailable: (message) => {
      calls.push(`session-unavailable:${message}`)
    },
    refreshWaitingRoomData: () => {
      calls.push("refresh-waiting-room")
    },
    applyTransportResumed: () => {
      calls.push("transport-resumed")
    },
    resyncAttachedKernelState: (reason) => {
      calls.push(`resync:${reason}`)
    },
    appendNotice: (message, tone) => {
      calls.push(`notice:${message}:${tone ?? "muted"}`)
    },
    flashFooter: (message, tone) => {
      calls.push(`footer:${message}:${tone}`)
    },
    applyTransportClosed: (message) => {
      calls.push(`transport-closed:${message}`)
    },
    recoverAttachedSessionAfterKernelRestart: () => {
      calls.push("recover-after-restart")
    },
  })

  return {
    calls,
    controller,
    snapshots,
    terminalRecords,
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
