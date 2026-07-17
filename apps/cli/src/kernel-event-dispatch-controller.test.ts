import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeProviderRun,
  RuntimeSession,
  TerminalOutputRecord,
} from "./cli-types.js"
import { createKernelEventDispatchController } from "./kernel-event-dispatch-controller.js"
import type { SliceRecord } from "@arroba/kernel-client/kernel-types"
import type { ProviderCatalog } from "./provider-catalog.js"

test("kernel event dispatch applies normalized session snapshots with agent activity", async () => {
  const run = providerRun("run-1")
  const nextSession = session({
    agents: [{ id: "agent-1" } as RuntimeSession["agents"][number]],
  })
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "session_snapshot",
    session: nextSession as unknown as Record<string, unknown>,
    provider_run: run as unknown as Record<string, unknown>,
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

  assert.equal(harness.snapshots.length, 1)
  assert.equal(harness.snapshots[0]?.session.id, "session-1")
  assert.deepEqual(harness.snapshots[0]?.session.agent_activity, {
    "agent-1": {
      status: "working",
      prompt_status: "running",
      busy: true,
      unread_idle_output: false,
    },
  })
  assert.equal(harness.snapshots[0]?.session.agent_activity_revision, 7)
  assert.equal(harness.snapshots[0]?.providerRun?.id, "run-1")
  assert.deepEqual(harness.calls, [
    "activity:kernel_session_snapshot",
    "refresh-prompt-input-history",
    "apply-session-snapshot:session-1:run-1",
  ])
})

test("kernel event dispatch clears stale embedded activity when snapshot activity is absent", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "session_snapshot",
    session: {
      ...session(),
      agent_activity: { "agent-stale": { status: "working", busy: true } },
      agent_activity_revision: 4,
    } as unknown as Record<string, unknown>,
    provider_run: null,
    agent_activity: null as unknown as Record<string, unknown>,
    agent_activity_revision: 7,
  })

  assert.equal(harness.snapshots.length, 1)
  assert.equal(harness.snapshots[0]?.session.agent_activity, undefined)
  assert.equal(harness.snapshots[0]?.session.agent_activity_revision, undefined)
  assert.deepEqual(harness.calls, [
    "activity:kernel_session_snapshot",
    "refresh-prompt-input-history",
    "apply-session-snapshot:session-1:null",
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

test("kernel event dispatch applies waiting-room row patches without refreshing inventory", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "waiting_room_rows_changed",
    inventory_version: "v2",
    schema_version: 1,
    generated_at_ms: 2,
    sessions: [{
      id: "session-1",
      workspace_id: "/workspace",
      worktree_id: "/workspace/tree",
      status: "Created",
      created_at_ms: 1,
      connected_cli_count: 0,
    }],
    removed_session_ids: ["session-2"],
  })

  assert.deepEqual(harness.calls, [
    "apply-waiting-room-rows:v2:session-1:session-2",
  ])
})

test("kernel event dispatch applies provider catalog and slice-list cache patches", async () => {
  const harness = createHarness()
  const catalog: ProviderCatalog = {
    all: [],
    default: {},
    connected: ["opencode"],
    source: "daemon",
  }
  const slice = sliceRecord("slice-1")

  await harness.controller.handleKernelEvent({
    event: "provider_catalog_changed",
    generated_at_ms: 2,
    catalog,
  })
  await harness.controller.handleKernelEvent({
    event: "slices_changed",
    generated_at_ms: 3,
    slices: [slice],
  })

  assert.deepEqual(harness.calls, [
    "activity:kernel_provider_catalog_changed",
    "apply-provider-catalog:daemon:opencode",
    "activity:kernel_slices_changed",
    "apply-slices:slice-1",
  ])
})

test("kernel event dispatch applies relay and remote-machine patches without refreshing inventory", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "relay_status_changed",
    status: {
      configured: true,
      connected: true,
      relay_token_configured: true,
      daemon_id: "kernel-1",
      machine_id: "machine-1",
      machine_alias: "laptop",
    },
  })
  await harness.controller.handleKernelEvent({
    event: "remote_machines_changed",
    machines: [{
      machine_id: "machine-2",
      machine_alias: "worker",
      registry_alias: null,
      display_name: "worker",
      trust_status: "approved",
      online: true,
      pending: false,
      kernel_count: 1,
      available_providers: ["opencode"],
    }],
  })

  assert.deepEqual(harness.calls, [
    "activity:kernel_relay_status_changed",
    "apply-relay-status:kernel-1:connected",
    "activity:kernel_remote_machines_changed",
    "apply-remote-machines:machine-2",
  ])
})

test("kernel event dispatch applies workflow design ops without resyncing", async () => {
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
    "apply-workflow-design-op:session-1:workflow_create",
  ])
})

test("kernel event dispatch applies workflow run updates without resyncing", async () => {
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
    "apply-workflow-run:session-1:run-1",
  ])
})

test("kernel event dispatch applies agent activity deltas without resyncing", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "agent_activity_changed",
    session_id: "session-1",
    agent_activity: { "agent-1": { status: "working", busy: true } },
    agent_activity_revision: 9,
  })

  assert.deepEqual(harness.calls, [
    "activity:kernel_agent_activity_changed",
    "apply-agent-activity:session-1:agent-1:9",
  ])
})

test("kernel event dispatch applies provider run deltas without resyncing", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "provider_run_changed",
    session_id: "session-1",
    provider_run: { id: "run-2", session_id: "session-1", agent_instance_id: "agent-1" },
  })

  assert.deepEqual(harness.calls, [
    "activity:kernel_provider_run_changed",
    "apply-provider-run:session-1:run-2",
  ])
})

test("kernel event dispatch applies session metadata deltas without resyncing", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "session_metadata_changed",
    session_id: "session-1",
    metadata: { alias: "ops", focused_agent_id: "agent-1" },
  })

  assert.deepEqual(harness.calls, [
    "activity:kernel_session_metadata_changed",
    "apply-session-metadata:session-1:alias,focused_agent_id",
  ])
})

test("kernel event dispatch applies runtime interaction deltas without resyncing", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "runtime_interactions_changed",
    session_id: "session-1",
    active_interactions: [{
      id: "interaction-1",
      agent_id: "agent-1",
      kind: "permission",
      level: "warning",
      message: "Approve?",
      choices: [{ id: "approve", label: "Approve", reply: "approve" }],
    }],
  })

  assert.deepEqual(harness.calls, [
    "activity:kernel_runtime_interactions_changed",
    "apply-runtime-interactions:session-1:1",
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
    timestamp_ms: 1,
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
  ])
})

test("kernel event dispatch treats successful transport resume as local liveness state", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "transport_resumed",
    session_id: "session-1",
    resumed_from_event_id: 42,
  })

  assert.deepEqual(harness.calls, [
    "transport-resumed",
  ])
})

test("kernel event dispatch reconciles durable history after assistant completion", async () => {
  const harness = createHarness()

  await harness.controller.handleKernelEvent({
    event: "assistant_message_completed",
    session_id: "session-1",
    provider_run_id: "run-1",
    agent_id: "agent-1",
    message_id: "message-1",
    completed_at_ms: 1,
  })

  assert.deepEqual(harness.calls, [
    "drain-terminal-records",
    "assistant-completed:agent-1",
    "refresh-assistant-history:agent-1",
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
    drainTerminalOutputRecords: () => {
      calls.push("drain-terminal-records")
    },
    applyRuntimeNotices: (notices) => {
      calls.push(`runtime-notices:${notices.length}`)
    },
    applyAssistantMessageCompleted: (event) => {
      calls.push(`assistant-completed:${event.agent_id ?? "null"}`)
    },
    refreshAssistantMessageHistory: (agentId: string) => {
      calls.push(`refresh-assistant-history:${agentId}`)
    },
    applyKernelSessionSnapshot: (nextSession, nextProviderRun) => {
      snapshots.push({ session: nextSession, providerRun: nextProviderRun })
      calls.push(`apply-session-snapshot:${nextSession.id}:${nextProviderRun?.id ?? "null"}`)
    },
    applyAgentActivityChanged: (sessionId, agentActivity, agentActivityRevision) => {
      calls.push(`apply-agent-activity:${sessionId}:${Object.keys(agentActivity).sort().join(",")}:${agentActivityRevision ?? "none"}`)
    },
    applyProviderRunChanged: (sessionId, providerRun) => {
      calls.push(`apply-provider-run:${sessionId}:${providerRun?.id ?? "null"}`)
    },
    applySessionMetadataChanged: (sessionId, metadata) => {
      calls.push(`apply-session-metadata:${sessionId}:${Object.keys(metadata).sort().join(",")}`)
    },
    applyRuntimeInteractionsChanged: (sessionId, activeInteractions) => {
      calls.push(`apply-runtime-interactions:${sessionId}:${activeInteractions.length}`)
    },
    applyWorkflowRunUpdated: (sessionId, workflowRun) => {
      calls.push(`apply-workflow-run:${sessionId}:${workflowRun.id}`)
    },
    applyWorkflowDesignOp: (event) => {
      calls.push(`apply-workflow-design-op:${event.session_id}:${event.op.kind}`)
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
    applyWaitingRoomRowsChanged: (patch) => {
      calls.push(`apply-waiting-room-rows:${patch.inventoryVersion}:${patch.sessions.map((session) => session.id).join(",")}:${patch.removedSessionIds.join(",")}`)
    },
    applyRelayStatusChanged: (status) => {
      calls.push(`apply-relay-status:${status.daemon_id}:${status.connected ? "connected" : "disconnected"}`)
    },
    applyRemoteMachinesChanged: (machines) => {
      calls.push(`apply-remote-machines:${machines.map((machine) => machine.machine_id).join(",")}`)
    },
    applyProviderCatalogChanged: (catalog) => {
      calls.push(`apply-provider-catalog:${catalog.source ?? "daemon"}:${catalog.connected.join(",")}`)
    },
    applySlicesChanged: (slices) => {
      calls.push(`apply-slices:${slices.map((slice) => slice.id).join(",")}`)
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

function sliceRecord(id: string): SliceRecord {
  return {
    id,
    name: id,
    owner_kernel_id: "kernel-1",
    owner_machine_id: "machine-1",
    backend: "local_docker",
    os: "linux",
    status: "running",
    worker_kernel_ref: "kernel-1",
    created_at_ms: 1,
    updated_at_ms: 1,
  }
}
