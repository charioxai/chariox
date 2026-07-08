import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeAttachment,
  RuntimeProviderRun,
  RuntimeSession,
  TerminalOutputRecord,
  WorkspaceLiveSyncStatus,
} from "./cli-types.js"
import { createCliPollingController } from "./cli-polling-controller.js"
import type { runPollingLoop } from "./polling-effects.js"

type PollLoopOptions = Parameters<typeof runPollingLoop>[0]

test("cli polling controller queues terminal output records and records activity", async () => {
  const record: TerminalOutputRecord = { timestamp_ms: 1, kind: "provider_output", bytes: [104, 105] }
  const harness = createHarness({
    pumpTerminalOutput: async () => [record],
  })

  await harness.controller.pollOutput()

  assert.deepEqual(harness.loopOperations, ["polling terminal output"])
  assert.deepEqual(harness.calls, [
    "pumpTerminalOutput:session-1:attachment-1",
    "activity:terminal_output",
    "queue:1",
  ])
  assert.deepEqual(harness.queuedRecords, [record])
})

test("cli polling controller clears stale provider run on benign terminal-output miss", async () => {
  const harness = createHarness({
    providerRun: providerRun("run-1"),
    pumpTerminalOutput: async () => {
      throw new Error("Session has no active provider run")
    },
  })

  await harness.controller.pollOutput()

  assert.equal(harness.providerRun, null)
  assert.deepEqual(harness.calls, [
    "pumpTerminalOutput:session-1:attachment-1",
    "setProviderRun:null",
    "updateSessionChrome",
  ])
})

test("cli polling controller keeps provider run on terminal-output miss while projected activity is busy", async () => {
  const run = providerRun("run-1")
  const harness = createHarness({
    providerRun: run,
    sessionState: session({
      agents: [agent()],
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
        },
      },
    }),
    pumpTerminalOutput: async () => {
      throw new Error("Session has no active provider run")
    },
  })

  await assert.rejects(harness.controller.pollOutput(), /Session has no active provider run/)

  assert.equal(harness.providerRun, run)
  assert.deepEqual(harness.calls, [
    "pumpTerminalOutput:session-1:attachment-1",
  ])
})

test("cli polling controller appends runtime notices", async () => {
  const harness = createHarness({
    pollRuntimeNotices: async () => [{ message: "heads up" }],
  })

  await harness.controller.pollNotices()

  assert.deepEqual(harness.calls, [
    "pollRuntimeNotices:session-1:attachment-1",
    "activity:runtime_notices",
    "notice:heads up",
  ])
})

test("cli polling controller suppresses queued prompt lifecycle notices", async () => {
  const harness = createHarness({
    pollRuntimeNotices: async () => [
      {
        message: "A queued message from attachment `attachment-4` was added to agent `agent-1` in session `session-1` as `prompt-7`. Queue depth is now 1.",
      },
      {
        message: "Attachment `attachment-1` steered queued prompt `prompt-4` to agent `agent-1`.",
      },
      {
        message: "Provider prompt dispatch failed: denied",
      },
    ],
  })

  await harness.controller.pollNotices()

  assert.deepEqual(harness.calls, [
    "pollRuntimeNotices:session-1:attachment-1",
    "activity:runtime_notices",
    "notice:Provider prompt dispatch failed: denied",
  ])
})

test("cli polling controller refreshes session state and provider run metadata", async () => {
  const refreshedRun = providerRun("run-2", { model: "next-model" })
  const nextSession = session({
    active_provider_run_id: refreshedRun.id,
  })
  const harness = createHarness({
    sessionState: session({
      prompt_states: {
        "agent-1": {
          active_prompt: activePrompt(),
          queued_prompts: [],
        },
      },
    }),
    nextSession,
    providerRun: providerRun("run-1"),
    shouldRefreshAgentPanesForSessionChange: () => false,
    tryGetProviderRun: async () => refreshedRun,
  })

  await harness.controller.pollSessionState()

  assert.equal(harness.providerRun?.id, refreshedRun.id)
  assert.deepEqual(harness.calls, [
    "getSessionState:session-1",
    "activity:session_state_poll",
    "applySessionState:session-1",
    "refreshAgentPanes:session-1",
    "tryGetProviderRun:run-2",
    "providerDebug:session poll refreshed provider run:run-2:run_changed",
    "setProviderRun:run-2",
    "applySessionState:session-1",
    "updateSessionChrome",
  ])
})

test("cli polling controller refreshes workspace live sync footer state after a turn completes", async () => {
  const nextSession = session()
  const status = workspaceLiveSyncStatus("conflict")
  const harness = createHarness({
    sessionState: session({
      prompt_states: {
        "agent-1": {
          active_prompt: activePrompt(),
          queued_prompts: [],
        },
      },
    }),
    nextSession,
    getWorkspaceLiveSyncStatus: async () => status,
  })

  await harness.controller.pollSessionState()

  assert.equal(harness.workspaceLiveSyncStatus?.footer_state, "conflict")
  assert.deepEqual(harness.calls, [
    "getSessionState:session-1",
    "activity:session_state_poll",
    "applySessionState:session-1",
    "refreshAgentPanes:session-1",
    "getWorkspaceLiveSyncStatus:session-1",
    "setWorkspaceLiveSyncStatus:conflict",
    "updateSessionChrome",
  ])
})

function createHarness(options: {
  attachment?: RuntimeAttachment | null
  sessionState?: RuntimeSession
  nextSession?: RuntimeSession
  providerRun?: RuntimeProviderRun | null
  pumpTerminalOutput?: () => Promise<TerminalOutputRecord[]>
  pollRuntimeNotices?: () => Promise<{ message: string }[]>
  shouldRefreshAgentPanesForSessionChange?: (session: RuntimeSession) => boolean
  tryGetProviderRun?: (providerRunId: string) => Promise<RuntimeProviderRun | null>
  getWorkspaceLiveSyncStatus?: (sessionId: string) => Promise<WorkspaceLiveSyncStatus>
} = {}) {
  const calls: string[] = []
  const loopOperations: string[] = []
  const queuedRecords: TerminalOutputRecord[] = []
  const harness = {
    calls,
    loopOperations,
    queuedRecords,
    attachment: options.attachment === undefined ? { id: "attachment-1", session_id: "session-1" } : options.attachment,
    sessionState: options.sessionState ?? session(),
    providerRun: options.providerRun ?? null,
    workspaceLiveSyncStatus: null as WorkspaceLiveSyncStatus | null,
    controller: null as ReturnType<typeof createCliPollingController> | null,
  }
  harness.controller = createCliPollingController({
    runPollingLoop: async (loopOptions: PollLoopOptions) => {
      loopOperations.push(loopOptions.operation)
      await loopOptions.task()
    },
    isClosing: () => false,
    formatError: (error) => error instanceof Error ? error.message : String(error),
    isSessionUnavailableError: () => false,
    getPollRecoveryDecision: () => ({ retry: false, delayMs: 0, message: "fatal" }),
    onSessionUnavailable: () => calls.push("sessionUnavailable"),
    onMarkRecovered: () => {},
    onMarkDegraded: () => {},
    onFatalError: () => calls.push("fatal"),
    sleep: async () => {},
    isAttached: () => harness.attachment !== null,
    getAttachment: () => harness.attachment,
    getSession: () => harness.sessionState,
    getProviderRun: () => harness.providerRun,
    setProviderRun: (run) => {
      calls.push(`setProviderRun:${run?.id ?? "null"}`)
      harness.providerRun = run
    },
    updateSessionChrome: () => calls.push("updateSessionChrome"),
    recordDaemonActivity: (activityType) => calls.push(`activity:${activityType}`),
    queueTerminalOutputRecords: (records) => {
      calls.push(`queue:${records.length}`)
      queuedRecords.push(...records)
    },
    pumpTerminalOutput: async (sessionId, attachmentId) => {
      calls.push(`pumpTerminalOutput:${sessionId}:${attachmentId}`)
      return options.pumpTerminalOutput?.() ?? []
    },
    pollRuntimeNotices: async (sessionId, attachmentId) => {
      calls.push(`pollRuntimeNotices:${sessionId}:${attachmentId}`)
      return options.pollRuntimeNotices?.() ?? []
    },
    appendNotice: (message) => calls.push(`notice:${message}`),
    getSessionState: async (sessionId) => {
      calls.push(`getSessionState:${sessionId}`)
      return options.nextSession ?? harness.sessionState
    },
    ...(options.getWorkspaceLiveSyncStatus
      ? {
        getWorkspaceLiveSyncStatus: async (sessionId: string) => {
          calls.push(`getWorkspaceLiveSyncStatus:${sessionId}`)
          return options.getWorkspaceLiveSyncStatus!(sessionId)
        },
        setWorkspaceLiveSyncStatus: (status: WorkspaceLiveSyncStatus | null) => {
          calls.push(`setWorkspaceLiveSyncStatus:${status?.footer_state ?? "null"}`)
          harness.workspaceLiveSyncStatus = status
        },
      }
      : {}),
    projectSession: (nextSession) => nextSession,
    shouldRefreshAgentPanesForSessionChange: options.shouldRefreshAgentPanesForSessionChange ?? (() => false),
    applySessionState: (nextSession) => {
      calls.push(`applySessionState:${nextSession.id}`)
      harness.sessionState = nextSession
    },
    refreshAgentPanes: async (nextSession) => {
      calls.push(`refreshAgentPanes:${nextSession.id}`)
    },
    tryGetProviderRun: async (providerRunId) => {
      calls.push(`tryGetProviderRun:${providerRunId}`)
      return options.tryGetProviderRun?.(providerRunId) ?? null
    },
    sameProviderRun: (left, right) => left.id === right.id,
    logProviderRunDebug: (message, run, fields) => {
      calls.push(`providerDebug:${message}:${run?.id ?? "null"}:${fields?.refresh_reason ?? ""}`)
    },
    recoverProviderRun: (reason) => calls.push(`recoverProviderRun:${reason}`),
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createCliPollingController>
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
    agents: [agent()],
    config_state: { values: {} } as RuntimeSession["config_state"],
    ...overrides,
  }
}

function activePrompt() {
  return {
    id: "prompt-1",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "build",
    status: "running",
  }
}

function providerRun(
  id: string,
  overrides: Partial<RuntimeProviderRun> = {},
): RuntimeProviderRun {
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
    ...overrides,
  }
}

function agent(
  overrides: Partial<RuntimeSession["agents"][number]> = {},
): RuntimeSession["agents"][number] {
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "model",
    worktree_id: "/workspace",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
    ...overrides,
  }
}

function workspaceLiveSyncStatus(
  footerState: WorkspaceLiveSyncStatus["footer_state"],
): WorkspaceLiveSyncStatus {
  return {
    session_id: "session-1",
    mode: footerState === "off" ? "unrestricted" : "managed",
    footer_state: footerState,
    sync_groups: [],
    targets: [],
    conflicts: [],
    ignore: {
      rules: [],
      force_excludes: [],
    },
  }
}
