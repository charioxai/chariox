import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowWatchdogDefinition,
} from "./cli-types.js"
import {
  handleWorkflowWatchdogCommand,
  type WorkflowWatchdogCommandContext,
  type WorkflowWatchdogCommandDeps,
} from "./workflow-watchdog-command-handlers.js"

test("workflow watchdog add parses explicit workflow options and prompt", async () => {
  const harness = createHarness({
    createWorkflowWatchdog: async (workflowRef, endpointRef, intervalSeconds, prompt, policy, maxWakeups) => {
      harness.calls.push(`create:${workflowRef}:${endpointRef}:${intervalSeconds}:${policy}:${maxWakeups ?? "null"}:${prompt}`)
      return {
        watchdog: watchdog(watchdogCreateOverrides(workflowRef, endpointRef, policy, maxWakeups)),
        workflow: workflow({ id: workflowRef }),
        session: session(),
      }
    },
  })

  await handleWorkflowWatchdogCommand(harness.deps, harness.context, [
    "watchdog",
    "add",
    "workflow-1",
    "entry",
    "every",
    "5m",
    "queue",
    "max-wakeups",
    "3",
    "run",
    "summary",
  ])

  assert.deepEqual(harness.calls, [
    "create:workflow-1:entry:300:queue:3:run summary",
    "apply:session-1",
    "select:workflow-1",
    "footer:info:created workflow watchdog watchdog-1",
  ])
})

test("workflow watchdog add defaults selected workflow, prompt, policy, and max wakeups", async () => {
  const harness = createHarness({ selectedWorkflowRef: "workflow-selected" })

  await handleWorkflowWatchdogCommand(harness.deps, harness.context, ["watchdog", "add", "entry", "every", "1h", "max-wakeups", "null"])

  assert.deepEqual(harness.calls, [
    "create:workflow-selected:entry:3600:skip:null:Run the workflow exactly as instructed.",
    "apply:session-1",
    "select:workflow-selected",
    "footer:info:created workflow watchdog watchdog-1",
  ])
})

test("workflow watchdog list renders configured watchdogs", async () => {
  const harness = createHarness({
    listWorkflowWatchdogs: async (workflowRef) => {
      harness.calls.push(`list:${workflowRef ?? "all"}`)
      return {
        watchdogs: [watchdog({ pending_run: true, max_wakeups: null, next_run_at_ms: 0 })],
      }
    },
  })

  await handleWorkflowWatchdogCommand(harness.deps, harness.context, ["watchdog", "list", "workflow-1"])

  assert.deepEqual(harness.calls, [
    "list:workflow-1",
    "notice:watchdog-1 workflow=workflow-1 endpoint=entry every=60s policy=skip enabled=true wakeups=0/unbounded next=1970-01-01T00:00:00.000Z pending=true",
    "footer:info:listed 1 workflow watchdog(s)",
  ])
})

test("workflow watchdog toggle and remove apply returned sessions", async () => {
  const harness = createHarness()

  await handleWorkflowWatchdogCommand(harness.deps, harness.context, ["watchdog", "disable", "watchdog-1"])
  await handleWorkflowWatchdogCommand(harness.deps, harness.context, ["watchdog", "remove", "watchdog-1"])

  assert.deepEqual(harness.calls, [
    "toggle:watchdog-1:false",
    "apply:session-1",
    "footer:info:disabled workflow watchdog watchdog-1",
    "remove:watchdog-1",
    "apply:session-1",
    "footer:info:removed workflow watchdog watchdog-1",
  ])
})

test("workflow watchdog command validates add options and unavailable runtime support", async () => {
  const missingRuntime = createHarness({ createWorkflowWatchdog: undefined })
  await handleWorkflowWatchdogCommand(missingRuntime.deps, missingRuntime.context, ["watchdog", "add", "entry", "every", "5m"])

  const invalidInterval = createHarness()
  await handleWorkflowWatchdogCommand(invalidInterval.deps, invalidInterval.context, ["watchdog", "add", "entry", "every", "0m"])

  const invalidMaxWakeups = createHarness()
  await handleWorkflowWatchdogCommand(invalidMaxWakeups.deps, invalidMaxWakeups.context, ["watchdog", "add", "entry", "every", "5m", "max-wakeups", "0"])

  assert.deepEqual(missingRuntime.calls, [
    "footer:error:workflow watchdogs are unavailable in this build",
  ])
  assert.deepEqual(invalidInterval.calls, [
    "footer:error:watchdog interval must be like 30s, 5m, 1h, or 1d",
  ])
  assert.deepEqual(invalidMaxWakeups.calls, [
    "footer:error:max-wakeups must be a positive integer or `null`",
  ])
})

test("workflow schedule add parses cron seconds options and prompt", async () => {
  const harness = createHarness()

  await handleWorkflowWatchdogCommand(harness.deps, harness.context, [
    "schedule",
    "add",
    "workflow-1",
    "entry",
    "--cron",
    "15 30 14 * * *",
    "--tz",
    "America/New_York",
    "--queue",
    "queue-1",
    "--overlap",
    "queue",
    "--max-runs",
    "2",
    "--prompt",
    "run",
    "summary",
  ])

  assert.deepEqual(harness.calls, [
    "schedule-create:workflow-1:entry:cron:15 30 14 * * *:America/New_York:queue:2:queue-1:run summary",
    "apply:session-1",
    "select:workflow-1",
    "footer:info:created workflow schedule watchdog-1",
  ])
})

test("workflow schedule list toggle and remove use schedule labels", async () => {
  const harness = createHarness({
    listWorkflowSchedules: async (workflowRef) => {
      harness.calls.push(`schedule-list:${workflowRef ?? "all"}`)
      return {
        schedules: [
          watchdog({
            id: "schedule-1",
            trigger: { kind: "cron", expression: "15 30 14 * * *", timezone: "UTC" },
            overlap_policy: "queue",
            max_runs: null,
            runs_started: 2,
            next_run_at_ms: 0,
            pending_run: true,
          }),
        ],
      }
    },
  })

  await handleWorkflowWatchdogCommand(harness.deps, harness.context, ["schedule", "list", "workflow-1"])
  await handleWorkflowWatchdogCommand(harness.deps, harness.context, ["schedule", "disable", "schedule-1"])
  await handleWorkflowWatchdogCommand(harness.deps, harness.context, ["schedule", "remove", "schedule-1"])

  assert.deepEqual(harness.calls, [
    "schedule-list:workflow-1",
    "notice:schedule-1 workflow=workflow-1 endpoint=entry trigger=cron:15 30 14 * * * tz=UTC overlap=queue enabled=true runs=2/unbounded next=1970-01-01T00:00:00.000Z pending=true",
    "footer:info:listed 1 workflow schedule(s)",
    "schedule-toggle:schedule-1:false",
    "apply:session-1",
    "footer:info:disabled workflow schedule schedule-1",
    "schedule-remove:schedule-1",
    "apply:session-1",
    "footer:info:removed workflow schedule schedule-1",
  ])
})

type HarnessOptions = Omit<Partial<WorkflowWatchdogCommandDeps>, "createWorkflowWatchdog"> & {
  createWorkflowWatchdog?: WorkflowWatchdogCommandDeps["createWorkflowWatchdog"] | undefined
  context?: Partial<WorkflowWatchdogCommandContext>
  selectedWorkflowRef?: string | null
}

function createHarness(options: HarnessOptions = {}) {
  const {
    context: contextOverrides,
    selectedWorkflowRef = "workflow-1",
    ...depOverrides
  } = options
  const { createWorkflowWatchdog, ...plainDepOverrides } = depOverrides
  const calls: string[] = []
  const deps: WorkflowWatchdogCommandDeps = {
    appendNotice: (message) => {
      calls.push(`notice:${message}`)
    },
    createWorkflowSchedule: async (workflowRef, endpointRef, trigger, prompt, overlapPolicy, maxRuns, queueRef) => {
      const triggerLabel = trigger.kind === "interval"
        ? `interval:${trigger.every_seconds}`
        : `cron:${trigger.expression}:${trigger.timezone}`
      calls.push(`schedule-create:${workflowRef}:${endpointRef}:${triggerLabel}:${overlapPolicy}:${maxRuns ?? "null"}:${queueRef ?? "null"}:${prompt}`)
      return {
        schedule: watchdog({
          workflow_id: workflowRef,
          endpoint_id: endpointRef,
          trigger,
          overlap_policy: overlapPolicy,
          max_runs: maxRuns ?? null,
        }),
        workflow: workflow({ id: workflowRef }),
        session: session(),
      }
    },
    listWorkflowSchedules: async (workflowRef) => {
      calls.push(`schedule-list:${workflowRef ?? "all"}`)
      return { schedules: [] }
    },
    setWorkflowScheduleEnabled: async (scheduleRef, enabled) => {
      calls.push(`schedule-toggle:${scheduleRef}:${String(enabled)}`)
      return { schedule: watchdog({ id: scheduleRef, enabled }), session: session() }
    },
    removeWorkflowSchedule: async (scheduleRef) => {
      calls.push(`schedule-remove:${scheduleRef}`)
      return { schedule: watchdog({ id: scheduleRef }), session: session() }
    },
    listWorkflowWatchdogs: async (workflowRef) => {
      calls.push(`list:${workflowRef ?? "all"}`)
      return { watchdogs: [] }
    },
    setWorkflowWatchdogEnabled: async (watchdogRef, enabled) => {
      calls.push(`toggle:${watchdogRef}:${String(enabled)}`)
      return { watchdog: watchdog({ id: watchdogRef, enabled }), session: session() }
    },
    removeWorkflowWatchdog: async (watchdogRef) => {
      calls.push(`remove:${watchdogRef}`)
      return { watchdog: watchdog({ id: watchdogRef }), session: session() }
    },
    applySessionState: (nextSession) => {
      calls.push(`apply:${nextSession.id}`)
    },
    selectWorkflowCanvas: (workflowId) => {
      calls.push(`select:${workflowId}`)
    },
    flashFooter: (message, tone) => {
      calls.push(`footer:${tone}:${message}`)
    },
    ...plainDepOverrides,
  }
  if (createWorkflowWatchdog !== undefined || !("createWorkflowWatchdog" in depOverrides)) {
    deps.createWorkflowWatchdog = createWorkflowWatchdog ?? (async (workflowRef, endpointRef, intervalSeconds, prompt, policy, maxWakeups) => {
      calls.push(`create:${workflowRef}:${endpointRef}:${intervalSeconds}:${policy}:${maxWakeups ?? "null"}:${prompt}`)
      return {
        watchdog: watchdog(watchdogCreateOverrides(workflowRef, endpointRef, policy, maxWakeups)),
        workflow: workflow({ id: workflowRef }),
        session: session(),
      }
    })
  }
  const context: WorkflowWatchdogCommandContext = {
    workflowRefOrSelected: (workflowRef) => workflowRef ?? selectedWorkflowRef,
    ...contextOverrides,
  }
  return { calls, context, deps }
}

function watchdogCreateOverrides(
  workflowRef: string,
  endpointRef: string,
  policy: "skip" | "queue",
  maxWakeups: number | null | undefined,
): Partial<WorkflowWatchdogDefinition> {
  return {
    workflow_id: workflowRef,
    endpoint_id: endpointRef,
    trigger: { kind: "interval", every_seconds: 60 },
    overlap_policy: policy,
    policy,
    ...(maxWakeups !== undefined ? { max_wakeups: maxWakeups } : {}),
  }
}

function watchdog(overrides: Partial<WorkflowWatchdogDefinition> = {}): WorkflowWatchdogDefinition {
  return {
    id: "watchdog-1",
    workflow_id: "workflow-1",
    endpoint_id: "entry",
    enabled: true,
    trigger: { kind: "interval", every_seconds: 60 },
    interval_seconds: 60,
    invocation_prompt: "prompt",
    overlap_policy: "skip",
    policy: "skip",
    runs_started: 0,
    max_wakeups: 1,
    wakeups_executed: 0,
    next_run_at_ms: 1,
    created_at_ms: 1,
    updated_at_ms: 1,
    ...overrides,
  }
}

function workflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: null,
    nodes: [],
    edges: [],
    endpoints: [],
    ...overrides,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 1,
    status: "active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 4,
    agents: [],
    config_state: { version: 1, values: {} },
    ...overrides,
  }
}
