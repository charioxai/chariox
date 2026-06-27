import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeNoticeRecord, TerminalOutputRecord, TranscriptEntry } from "./cli-types.js"
import { createKernelEventController } from "./kernel-event-controller.js"

function createDeps(overrides: Record<string, unknown> = {}) {
  const calls: string[] = []
  const notices: Array<{ message: string; tone: "default" | "warning" | undefined }> = []
  const paneEntries = new Map<string, TranscriptEntry[]>()

  const deps = {
    recordDaemonActivity: (source: string) => calls.push(`activity:${source}`),
    recordTurnActivity: (source: string) => calls.push(`turn-activity:${source}`),
    resolveTerminalRecordAgentId: (record: TerminalOutputRecord) => record.agent_id ?? null,
    setStreamingAgentId: (agentId: string) => calls.push(`streaming:${agentId}`),
    markAgentBusy: (agentId: string | null | undefined) => calls.push(`busy:${agentId ?? "null"}`),
    splitAgentResponseMode: () => false,
    visibleTranscriptAgentId: () => "agent-a",
    focusedAgentId: () => "agent-a",
    hasTrailingUserPrompt: () => false,
    currentAgentPaneEntries: (agentId: string) => paneEntries.get(agentId) ?? [],
    computeNextTurnId: () => 1,
    appendTranscriptEntryToAgentPane: (agentId: string, entry: Omit<TranscriptEntry, "id">) => {
      calls.push(`pane-entry:${agentId}:${entry.role}:${entry.text}`)
    },
    appendProviderChunkToAgentPane: (
      agentId: string,
      role: TranscriptEntry["role"],
      text: string,
      mergeKey?: string,
    ) => {
      calls.push(`pane-chunk:${agentId}:${role}:${text}:${mergeKey ?? ""}`)
    },
    appendToolUpdateToAgentPane: (agentId: string, text: string) => {
      calls.push(`pane-tool:${agentId}:${text}`)
    },
    setAgentActivityLabel: (agentId: string | null | undefined, label: string | null) => {
      calls.push(`agent-activity:${agentId ?? "null"}:${label ?? "null"}`)
    },
    agentActivityLabel: () => null,
    setProviderActivityLabel: (label: string | null) => calls.push(`provider-activity:${label ?? "null"}`),
    applyProviderActivity: (active: boolean) => calls.push(`provider-active:${active}`),
    syncVisibleActivityLabel: () => calls.push("sync-visible-activity"),
    getProviderActivityLabel: (text: string) => (text.includes("thinking") ? "Thinking" : null),
    shouldRenderProviderStatus: (text: string) => !text.includes("idle"),
    appendEntry: (entry: Omit<TranscriptEntry, "id">) => calls.push(`entry:${entry.role}:${entry.text}`),
    appendProviderChunk: (
      role: TranscriptEntry["role"],
      text: string,
      mergeKey?: string,
    ) => {
      calls.push(`chunk:${role}:${text}:${mergeKey ?? ""}`)
    },
    appendToolUpdate: (text: string) => calls.push(`tool:${text}`),
    appendProviderError: (text: string) => calls.push(`error:${text.trim()}`),
    syncVisibleTranscriptPreview: () => calls.push("sync-visible-preview"),
    appendAgentPanePreview: (agentId: string | null | undefined, line: string) => {
      calls.push(`preview:${agentId ?? "null"}:${line}`)
    },
    previewLineForTerminalRecord: (kind: TerminalOutputRecord["kind"], text: string) => `${kind}:${text.trim()}`,
    trimSingleTrailingNewline: (text: string) => text.replace(/\n$/, ""),
    setDaemonDisconnected: (value: boolean) => calls.push(`daemon-disconnected:${value}`),
    setStatusLine: (value: string) => calls.push(`status:${value}`),
    updateSessionChrome: () => calls.push("update-session-chrome"),
    appendNotice: (message: string, tone?: "default" | "warning") => {
      notices.push({ message, tone })
      calls.push(`notice:${message}:${tone ?? "default"}`)
    },
    connectedStatusLine: "Connected to the Arroba kernel.",
    markAssistantMessageCompleted: (agentId: string | null | undefined) => calls.push(`completed:${agentId ?? "null"}`),
    ...overrides,
  }

  return { deps, calls, notices, paneEntries }
}

test("off-focus agent output updates the agent pane and preview without mutating the visible transcript", () => {
  const { deps, calls } = createDeps()
  const controller = createKernelEventController(deps as never)

  controller.processTerminalOutputRecord({
    agent_id: "agent-b",
    kind: "provider_output",
    merge_key: "reply-1",
    bytes: [...Buffer.from("hello from b\n", "utf8")],
  })

  assert.deepEqual(calls, [
    "activity:terminal_record",
    "turn-activity:terminal_record",
    "streaming:agent-b",
    "busy:agent-b",
    "pane-chunk:agent-b:assistant:hello from b\n:reply-1",
    "preview:agent-b:provider_output:hello from b",
  ])
})

test("unscoped terminal output records do not render into the visible transcript", () => {
  const { deps, calls } = createDeps()
  const controller = createKernelEventController(deps as never)

  controller.processTerminalOutputRecord({
    agent_id: null,
    kind: "provider_output",
    merge_key: "reply-1",
    bytes: [...Buffer.from("ambiguous\n", "utf8")],
  })

  assert.deepEqual(calls, [
    "activity:terminal_record",
    "turn-activity:terminal_record",
  ])
})

test("visible provider status updates activity and appends renderable status chunks", () => {
  const { deps, calls } = createDeps({
    resolveTerminalRecordAgentId: () => "agent-a",
  })
  const controller = createKernelEventController(deps as never)

  controller.processTerminalOutputRecord({
    agent_id: "agent-a",
    kind: "provider_status",
    bytes: [...Buffer.from("OpenCode is thinking...", "utf8")],
  })

  assert.deepEqual(calls, [
    "activity:terminal_record",
    "turn-activity:terminal_record",
    "streaming:agent-a",
    "busy:agent-a",
    "agent-activity:agent-a:Thinking",
    "provider-activity:Thinking",
    "provider-active:true",
    "sync-visible-activity",
    "chunk:status:OpenCode is thinking...:__provider_status__",
    "sync-visible-preview",
  ])
})

test("external provider history update status triggers pane refresh hook", () => {
  const { deps, calls } = createDeps({
    resolveTerminalRecordAgentId: () => "agent-a",
    handleExternalProviderHistoryUpdated: (agentId: string | null) => {
      calls.push(`external-history:${agentId ?? "null"}`)
    },
  })
  const controller = createKernelEventController(deps as never)

  controller.processTerminalOutputRecord({
    agent_id: "agent-a",
    kind: "provider_status",
    source: "external_provider_observed",
    bytes: [...Buffer.from("external_provider_history_updated", "utf8")],
  })

  assert.deepEqual(calls, [
    "activity:terminal_record",
    "turn-activity:terminal_record",
    "external-history:agent-a",
  ])
})

test("external provider history update status requires observed source", () => {
  const { deps, calls } = createDeps({
    resolveTerminalRecordAgentId: () => "agent-a",
    handleExternalProviderHistoryUpdated: (agentId: string | null) => {
      calls.push(`external-history:${agentId ?? "null"}`)
    },
  })
  const controller = createKernelEventController(deps as never)

  controller.processTerminalOutputRecord({
    agent_id: "agent-a",
    kind: "provider_status",
    bytes: [...Buffer.from("external_provider_history_updated", "utf8")],
  })

  assert.deepEqual(calls, [
    "activity:terminal_record",
    "turn-activity:terminal_record",
    "streaming:agent-a",
    "busy:agent-a",
    "agent-activity:agent-a:null",
    "provider-activity:null",
    "provider-active:false",
    "chunk:status:external_provider_history_updated:__provider_status__",
    "sync-visible-preview",
  ])
})

test("idle provider status is ignored so it cannot demote a live turn to idle", () => {
  const { deps, calls } = createDeps({
    resolveTerminalRecordAgentId: () => "agent-a",
    agentActivityLabel: () => "Thinking",
  })
  const controller = createKernelEventController(deps as never)

  controller.processTerminalOutputRecord({
    agent_id: "agent-a",
    kind: "provider_status",
    bytes: [...Buffer.from("OpenCode is idle.", "utf8")],
  })

  assert.deepEqual(calls, [
    "activity:terminal_record",
    "turn-activity:terminal_record",
    "streaming:agent-a",
  ])
})

test("runtime notices and transport lifecycle update the kernel connection state", () => {
  const { deps, calls, notices } = createDeps()
  const controller = createKernelEventController(deps as never)

  controller.applyRuntimeNotices([
    { message: "provider resumed" },
    { message: "worker switched" },
  ] satisfies RuntimeNoticeRecord[])
  controller.applyTransportClosed("connection lost")
  controller.applyTransportResumed()

  assert.deepEqual(calls, [
    "activity:kernel_runtime_notices",
    "notice:provider resumed:default",
    "notice:worker switched:default",
    "daemon-disconnected:true",
    "status:Lost connection to the Arroba kernel.",
    "update-session-chrome",
    "notice:connection lost:warning",
    "activity:kernel_transport_resumed",
    "daemon-disconnected:false",
    "status:Connected to the Arroba kernel.",
    "update-session-chrome",
    "notice:Reconnected to the Arroba kernel.:default",
  ])
  assert.deepEqual(notices, [
    { message: "provider resumed", tone: undefined },
    { message: "worker switched", tone: undefined },
    { message: "connection lost", tone: "warning" },
    { message: "Reconnected to the Arroba kernel.", tone: undefined },
  ])
})

test("assistant completion clears the agent completion state without relying on idle status", () => {
  const { deps, calls } = createDeps()
  const controller = createKernelEventController(deps as never)

  controller.applyAssistantMessageCompleted({
    agent_id: "agent-a",
  })

  assert.deepEqual(calls, [
    "activity:assistant_message_completed",
    "completed:agent-a",
  ])
})

test("duplicate trailing prompt echo is ignored for split-agent panes", () => {
  const { deps, calls } = createDeps({
    splitAgentResponseMode: () => true,
    resolveTerminalRecordAgentId: () => "agent-b",
    hasTrailingUserPrompt: () => true,
  })
  const controller = createKernelEventController(deps as never)

  controller.processTerminalOutputRecord({
    agent_id: "agent-b",
    kind: "prompt_echo",
    bytes: [...Buffer.from("ship it\n", "utf8")],
  })

  assert.deepEqual(calls, [
    "activity:terminal_record",
    "turn-activity:terminal_record",
  ])
})

test("visible prompt echo carries kernel prompt identity into transcript entry", () => {
  const appended: Array<Omit<TranscriptEntry, "id">> = []
  const { deps } = createDeps({
    resolveTerminalRecordAgentId: () => "agent-a",
    appendEntry: (entry: Omit<TranscriptEntry, "id">) => {
      appended.push(entry)
    },
  })
  const controller = createKernelEventController(deps as never)

  controller.processTerminalOutputRecord({
    agent_id: "agent-a",
    prompt_id: "prompt-1",
    source_attachment_id: "attachment-1",
    kind: "prompt_echo",
    bytes: [...Buffer.from("ship it\n", "utf8")],
  })

  assert.equal(appended.length, 1)
  assert.equal(appended[0]?.role, "user")
  assert.equal(appended[0]?.text, "ship it")
  assert.equal(appended[0]?.promptId, "prompt-1")
  assert.equal(appended[0]?.sourceAttachmentId, "attachment-1")
})

test("visible external observed output carries kernel observation metadata into transcript entries", () => {
  const chunks: Array<{
    role: TranscriptEntry["role"]
    text: string
    metadata?: Partial<TranscriptEntry>
  }> = []
  const { deps } = createDeps({
    resolveTerminalRecordAgentId: () => "agent-a",
    appendProviderChunk: (
      role: TranscriptEntry["role"],
      text: string,
      _mergeKey?: string,
      _sourceText?: string,
      metadata?: Partial<TranscriptEntry>,
    ) => {
      chunks.push({ role, text, metadata })
    },
  })
  const controller = createKernelEventController(deps as never)

  controller.processTerminalOutputRecord({
    agent_id: "agent-a",
    kind: "provider_output",
    merge_key: "external:codex:thread-1:item-1",
    source: "external_provider_observed",
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "item-1",
    observed_at_ms: 1_234,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
    bytes: [...Buffer.from("observed reply\n", "utf8")],
  })

  assert.equal(chunks.length, 1)
  assert.equal(chunks[0]?.role, "assistant")
  assert.equal(chunks[0]?.text, "observed reply\n")
  assert.deepEqual(chunks[0]?.metadata, {
    source: "external_provider_observed",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "item-1",
    observedAtMs: 1_234,
    externalObservation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  })
})
