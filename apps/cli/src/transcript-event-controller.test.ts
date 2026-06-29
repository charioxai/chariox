import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, TranscriptEntry } from "./cli-types.js"
import { createTranscriptEventController } from "./transcript-event-controller.js"

test("transcript event controller appends primary user prompts", () => {
  const harness = eventHarness({
    focusedAgentId: "agent-1",
    responsePrimaryAgentId: "agent-1",
    nextTurnId: 4,
  })

  harness.controller.appendUserPrompt("hello\n")

  assert.deepEqual(harness.recordedActivities, ["prompt_submit"])
  assert.equal(harness.resetTurns, 1)
  assert.equal(harness.submittingAgentId, "agent-1")
  assert.deepEqual(harness.streamingAgentIds, ["agent-1"])
  assert.deepEqual(harness.busyMarked, ["agent-1"])
  assert.equal(harness.nextTurnId, 5)
  assert.equal(harness.currentTurnId, 4)
  assert.deepEqual(harness.entries, [
    { role: "user", text: "hello", turnId: 4 },
  ])
  assert.equal(harness.previewSynced, 1)
  assert.equal(harness.scrolledToBottom, 1)
  assert.deepEqual(harness.submitting, [true])
  assert.deepEqual(harness.working, [true])
})

test("transcript event controller routes split-pane user prompts to the target pane", () => {
  const harness = eventHarness({
    split: true,
    focusedAgentId: "agent-1",
    responsePrimaryAgentId: "agent-1",
    paneEntries: {
      "agent-2": [{ id: 7, role: "assistant", text: "done", turnId: 3 }],
    },
  })

  harness.controller.appendUserPrompt("side quest", "agent-2")

  assert.equal(harness.currentTurnId, null)
  assert.equal(harness.nextTurnId, 1)
  assert.equal(harness.entries.length, 0)
  assert.deepEqual(harness.paneAppends, [{
    agentId: "agent-2",
    entry: { role: "user", text: "side quest", turnId: 4 },
    turnIds: [3],
  }])
  assert.equal(harness.scrolledToBottom, 0)
  assert.deepEqual(harness.submitting, [true])
  assert.deepEqual(harness.working, [true])
})

test("transcript event controller appends steered prompts without starting a new turn", () => {
  const harness = eventHarness({
    focusedAgentId: "agent-1",
    responsePrimaryAgentId: "agent-1",
    nextTurnId: 4,
  })

  harness.controller.appendSteeredPrompt("steer this\n", "agent-1", {
    promptId: "prompt-2",
    sourceAttachmentId: "attachment-2",
  })

  assert.deepEqual(harness.recordedActivities, ["queued_prompt_steer"])
  assert.equal(harness.resetTurns, 0)
  assert.equal(harness.nextTurnId, 4)
  assert.equal(harness.currentTurnId, null)
  assert.deepEqual(harness.streamingAgentIds, ["agent-1"])
  assert.deepEqual(harness.busyMarked, ["agent-1"])
  assert.deepEqual(harness.entries, [
    {
      role: "user",
      text: "steer this",
      turnTracking: "none",
      promptId: "prompt-2",
      sourceAttachmentId: "attachment-2",
    },
  ])
  assert.equal(harness.previewSynced, 1)
  assert.equal(harness.sessionChromeUpdates, 1)
  assert.equal(harness.scrolledToBottom, 1)
  assert.deepEqual(harness.submitting, [])
  assert.deepEqual(harness.working, [])
})

test("transcript event controller routes split-pane steered prompts to the target pane", () => {
  const harness = eventHarness({
    split: true,
    focusedAgentId: "agent-1",
    responsePrimaryAgentId: "agent-1",
  })

  harness.controller.appendSteeredPrompt("side steer", "agent-2", {
    promptId: "prompt-2",
  })

  assert.deepEqual(harness.paneAppends, [{
    agentId: "agent-2",
    entry: {
      role: "user",
      text: "side steer",
      turnTracking: "none",
      promptId: "prompt-2",
    },
    turnIds: undefined,
  }])
  assert.equal(harness.previewSynced, 0)
  assert.equal(harness.sessionChromeUpdates, 1)
  assert.equal(harness.scrolledToBottom, 0)
})

test("transcript event controller handles attached and detached cloud notices", () => {
  const attached = eventHarness({ attached: true })
  attached.controller.appendCloudNotice("linked")

  assert.deepEqual(attached.entries, [
    { role: "notice", text: "linked", emphasis: "muted" },
  ])
  assert.equal(attached.rebuiltTranscript, 0)
  assert.equal(attached.sessionChromeUpdates, 1)

  const detached = eventHarness({ attached: false })
  detached.controller.appendCloudNotice("waiting")

  assert.equal(detached.waitingRoomCloudNotice, "waiting")
  assert.equal(detached.entries.length, 0)
  assert.equal(detached.rebuiltTranscript, 1)
  assert.equal(detached.sessionChromeUpdates, 1)
})

test("transcript event controller normalizes provider errors", () => {
  const harness = eventHarness({ visibleTranscriptAgentId: "agent-1" })

  harness.controller.appendProviderError("\r\nfailed\r")
  harness.controller.appendProviderError("  \n")

  assert.equal(harness.cancelledTurns, 1)
  assert.deepEqual(harness.working, [false])
  assert.deepEqual(harness.submitting, [false])
  assert.deepEqual(harness.busyCleared, ["agent-1"])
  assert.equal(harness.submittingAgentId, null)
  assert.deepEqual(harness.entries, [
    { role: "error", text: "failed", emphasis: "error" },
  ])
  assert.equal(harness.previewSynced, 1)
  assert.equal(harness.renderedSessionChrome, 1)
  assert.equal(harness.scrolledToBottom, 1)
})

function eventHarness(options: {
  attached?: boolean
  split?: boolean
  focusedAgentId?: string | null
  visibleTranscriptAgentId?: string | null
  responsePrimaryAgentId?: string | null
  entries?: TranscriptEntry[]
  paneEntries?: Record<string, TranscriptEntry[]>
  nextTurnId?: number
} = {}) {
  const harness = {
    attached: options.attached ?? true,
    split: options.split ?? false,
    focusedAgentId: options.focusedAgentId ?? "agent-1",
    visibleTranscriptAgentId: options.visibleTranscriptAgentId ?? "agent-1",
    responsePrimaryAgentId: options.responsePrimaryAgentId ?? "agent-1",
    entries: [...(options.entries ?? [])] as Array<Omit<TranscriptEntry, "id">>,
    paneEntries: options.paneEntries ?? {},
    nextTurnId: options.nextTurnId ?? 1,
    currentTurnId: null as number | null,
    submittingAgentId: undefined as string | null | undefined,
    recordedActivities: [] as string[],
    resetTurns: 0,
    cancelledTurns: 0,
    streamingAgentIds: [] as Array<string | null>,
    busyMarked: [] as Array<string | null | undefined>,
    busyCleared: [] as Array<string | null | undefined>,
    paneAppends: [] as Array<{
      agentId: string
      entry: Omit<TranscriptEntry, "id">
      turnIds: readonly number[] | undefined
    }>,
    submitting: [] as boolean[],
    working: [] as boolean[],
    renderedSessionChrome: 0,
    previewSynced: 0,
    scrolledToBottom: 0,
    sessionChromeUpdates: 0,
    waitingRoomCloudNotice: null as string | null,
    rebuiltTranscript: 0,
    controller: null as ReturnType<typeof createTranscriptEventController> | null,
  }
  harness.controller = createTranscriptEventController({
    recordTurnActivity: (activityType) => {
      harness.recordedActivities.push(activityType)
    },
    resetTurnCompletion: () => {
      harness.resetTurns += 1
    },
    cancelPendingTurnCompletion: () => {
      harness.cancelledTurns += 1
    },
    focusedAgentId: () => harness.focusedAgentId,
    visibleTranscriptAgentId: () => harness.visibleTranscriptAgentId,
    responsePrimaryAgent: () => harness.responsePrimaryAgentId
      ? ({ id: harness.responsePrimaryAgentId } as AgentInstance)
      : null,
    splitAgentResponseMode: () => harness.split,
    isAttached: () => harness.attached,
    entries: () => harness.entries as TranscriptEntry[],
    nextTurnId: () => harness.nextTurnId,
    setNextTurnId: (turnId) => {
      harness.nextTurnId = turnId
    },
    setCurrentTurnId: (turnId) => {
      harness.currentTurnId = turnId
    },
    setSubmittingAgentId: (agentId) => {
      harness.submittingAgentId = agentId
    },
    setStreamingAgentId: (agentId) => {
      harness.streamingAgentIds.push(agentId)
    },
    markAgentBusy: (agentId) => {
      harness.busyMarked.push(agentId)
    },
    clearAgentBusy: (agentId) => {
      harness.busyCleared.push(agentId)
    },
    currentAgentPaneEntries: (agentId) => harness.paneEntries[agentId] ?? [],
    collapseLatestTurnForAgent: (_agentId, paneEntries) => paneEntries
      .map((entry) => entry.turnId)
      .filter((turnId): turnId is number => typeof turnId === "number"),
    appendTranscriptEntryToAgentPane: (agentId, entry, turnIds) => {
      harness.paneAppends.push({ agentId, entry, turnIds })
    },
    appendEntry: (entry) => {
      harness.entries.push(entry)
    },
    setSubmitting: (value) => {
      harness.submitting.push(value)
    },
    setWorking: (value) => {
      harness.working.push(value)
    },
    renderSessionChromeBoundary: () => {
      harness.renderedSessionChrome += 1
    },
    syncVisibleTranscriptPreview: () => {
      harness.previewSynced += 1
    },
    scrollTranscriptToBottom: () => {
      harness.scrolledToBottom += 1
    },
    updateSessionChrome: () => {
      harness.sessionChromeUpdates += 1
    },
    setWaitingRoomCloudNotice: (text) => {
      harness.waitingRoomCloudNotice = text
    },
    rebuildTranscript: () => {
      harness.rebuiltTranscript += 1
    },
  })
  return harness as typeof harness & { controller: ReturnType<typeof createTranscriptEventController> }
}
