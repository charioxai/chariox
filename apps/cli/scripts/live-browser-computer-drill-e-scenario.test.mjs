import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, stat } from "node:fs/promises"
import { tmpdir } from "node:os"
import path from "node:path"
import test from "node:test"
import {
  parseArgs,
  runDrillEScenarioCli,
  runDrillEScenario,
  validateOptions,
} from "./live-browser-computer-drill-e-scenario.mjs"
import { verifyDrillE } from "./live-browser-computer-drill-e.mjs"

function options(overrides = {}) {
  return {
    kernelUrl: "ws://127.0.0.1:43118/kernel",
    sessionId: "session-e",
    agentA: "agent-a",
    agentB: "agent-b",
    agentC: "agent-c",
    sameTabId: "tab-same",
    otherTabId: "tab-other",
    output: "/tmp/drill-e-evidence.json",
    timeoutMs: 1_000,
    pollMs: 250,
    execute: true,
    ...overrides,
  }
}

function memoryOutput({ failFirstWrite = false } = {}) {
  const state = {
    bytes: Buffer.alloc(0),
    writes: 0,
    truncates: 0,
    syncs: 0,
    closed: false,
    failFirstWrite,
  }
  const handle = {
    async truncate(length) {
      state.truncates += 1
      state.bytes = state.bytes.subarray(0, length)
    },
    async write(buffer, offset, length, position) {
      state.writes += 1
      if (state.failFirstWrite && state.writes === 1) {
        const partial = buffer.subarray(offset, offset + Math.min(16, length))
        const end = position + partial.length
        if (end > state.bytes.length) {
          const expanded = Buffer.alloc(end)
          state.bytes.copy(expanded)
          state.bytes = expanded
        }
        partial.copy(state.bytes, position)
        throw new Error("injected output write failure with private payload")
      }
      const chunk = buffer.subarray(offset, offset + length)
      const end = position + chunk.length
      if (end > state.bytes.length) {
        const expanded = Buffer.alloc(end)
        state.bytes.copy(expanded)
        state.bytes = expanded
      }
      chunk.copy(state.bytes, position)
      return { bytesWritten: chunk.length }
    },
    async sync() { state.syncs += 1 },
    async close() { state.closed = true },
  }
  return { handle, outputPath: "/tmp/drill-e-evidence.json", state }
}

function environment({
  actions = [],
  pending = [],
  ownership = [],
  eventCursor = 10,
  environmentId = "environment-e",
  runtimeGeneration = 4,
} = {}) {
  return {
    session_id: "session-e",
    environment_id: environmentId,
    runtime_generation: runtimeGeneration,
    event_cursor: eventCursor,
    lifecycle: "ready",
    viewport: {
      css_width: 1280,
      css_height: 720,
      device_scale_factor: 1,
      desktop_pixel_width: 1280,
      desktop_pixel_height: 720,
      revision: 1,
      last_actor_id: null,
    },
    health: [],
    actors: [
      { actor_id: "agent:agent-a", kind: "agent", display_label: "A", presentation_color: "blue", presence: "present" },
      { actor_id: "agent:agent-b", kind: "agent", display_label: "B", presentation_color: "cyan", presence: "present" },
      { actor_id: "agent:agent-c", kind: "agent", display_label: "C", presentation_color: "green", presence: "present" },
      { actor_id: "user:operator", kind: "human", display_label: "Operator", presentation_color: "amber", presence: "present" },
    ],
    pointers: [],
    tabs: [
      { tab_id: "tab-same", url: "http://fixture.invalid/same", title: "Same", document_revision: 1, focused: true },
      { tab_id: "tab-other", url: "http://fixture.invalid/other", title: "Other", document_revision: 1, focused: false },
    ],
    focused_tab_id: "tab-same",
    actions,
    pending_input_takeovers: pending,
    input_ownership: ownership,
  }
}

function session({
  agentIds = ["agent-a", "agent-b", "agent-c"],
  promptStates = {},
} = {}) {
  return {
    id: "session-e",
    agents: agentIds.map((id) => ({ id })),
    active_prompt: null,
    queued_prompts: [],
    prompt_states: promptStates,
  }
}

function prompt(id, targetAgentId, attachmentId = "attachment-e") {
  return {
    id,
    target_agent_id: targetAgentId,
    source_attachment_id: attachmentId,
    status: "running",
  }
}

function action({
  id,
  sequence,
  actor,
  tab,
  kind,
  state,
  submitted,
  started = null,
  finished = null,
  cancellationRequested = false,
  outcome = null,
}) {
  return {
    action_id: id,
    sequence,
    idempotency_key: null,
    actor_id: actor,
    runtime_generation: 4,
    mode: "browser",
    kind,
    targets: [{ kind: "browser_tab", id: tab }],
    state,
    cancellation_requested: cancellationRequested,
    submitted_at_ms: submitted,
    started_at_ms: started,
    finished_at_ms: finished,
    outcome,
  }
}

function terminal(actionValue, { state = "completed", finished, reason = null } = {}) {
  return {
    ...actionValue,
    state,
    finished_at_ms: finished,
    outcome: reason
      ? { status: "cancelled", reason }
      : { status: "completed" },
  }
}

function intervalsOverlap(left, right) {
  return Math.max(left.started_at_ms, right.started_at_ms)
    < Math.min(left.finished_at_ms, right.finished_at_ms)
}

function fullSequence() {
  const readA = action({
    id: "action-read-a",
    sequence: 1,
    actor: "agent:agent-a",
    tab: "tab-same",
    kind: "browser_status",
    state: "running",
    submitted: 1_000,
    started: 1_010,
  })
  const readB = action({
    id: "action-read-b",
    sequence: 2,
    actor: "agent:agent-b",
    tab: "tab-same",
    kind: "browser_find",
    state: "running",
    submitted: 1_020,
    started: 1_030,
  })
  const readC = action({
    id: "action-read-c",
    sequence: 3,
    actor: "agent:agent-c",
    tab: "tab-other",
    kind: "browser_history_reload",
    state: "running",
    submitted: 1_040,
    started: 1_050,
  })
  const mutationA = action({
    id: "action-mutation-a",
    sequence: 4,
    actor: "agent:agent-a",
    tab: "tab-same",
    kind: "browser_history_reload",
    state: "running",
    submitted: 2_000,
    started: 2_010,
  })
  const mutationC = action({
    id: "action-mutation-c",
    sequence: 5,
    actor: "agent:agent-c",
    tab: "tab-other",
    kind: "browser_history_reload",
    state: "running",
    submitted: 2_040,
    started: 2_050,
  })
  const mutationB = action({
    id: "action-mutation-b",
    sequence: 6,
    actor: "agent:agent-b",
    tab: "tab-same",
    kind: "browser_history_reload",
    state: "queued",
    submitted: 2_200,
  })
  const completedReads = [
    terminal(readA, { finished: 1_200 }),
    terminal(readB, { finished: 1_210 }),
    terminal(readC, { finished: 1_190 }),
  ]
  const firstCancelled = terminal(mutationA, {
    state: "cancelled",
    finished: 2_300,
    reason: "human_takeover",
  })
  firstCancelled.cancellation_requested = true
  const secondCancelled = terminal(mutationB, {
    state: "cancelled",
    finished: 2_400,
    reason: "human_takeover",
  })
  const independentCompleted = terminal(mutationC, { finished: 2_800 })
  const pending = {
    target: { kind: "browser_tab", id: "tab-same" },
    human_actor_id: "user:operator",
    blocking_action_ids: ["action-mutation-a"],
  }
  const owner = { target: { kind: "browser_tab", id: "tab-same" }, actor_id: "user:operator" }
  const baseline = environment({ eventCursor: 10 })
  const readOverlap = environment({
    eventCursor: 11,
    actions: [readA, readB, readC],
  })
  const firstMutation = environment({
    eventCursor: 12,
    actions: [...completedReads, mutationA, mutationC],
  })
  const queuedMutation = environment({
    eventCursor: 13,
    actions: [...completedReads, mutationA, mutationC, mutationB],
  })
  const waitingForTakeover = environment({
    eventCursor: 14,
    actions: [...completedReads, { ...mutationA, cancellation_requested: true }, mutationC, mutationB],
    pending: [pending],
  })
  const takeoverGranted = environment({
    eventCursor: 15,
    actions: [...completedReads, firstCancelled, independentCompleted, secondCancelled],
    ownership: [owner],
  })
  const final = environment({
    eventCursor: 16,
    actions: [...completedReads, firstCancelled, independentCompleted, secondCancelled],
    ownership: [owner],
  })
  const afterRelease = environment({
    eventCursor: 17,
    actions: [...completedReads, firstCancelled, independentCompleted, secondCancelled],
  })
  return {
    baseline,
    readOverlap,
    firstMutation,
    queuedMutation,
    waitingForTakeover,
    takeoverGranted,
    final,
    afterRelease,
    history: [...completedReads, firstCancelled, independentCompleted, secondCancelled],
  }
}

function emptyPromptStates() {
  return {
    "agent-a": { active_prompt: null, queued_prompts: [] },
    "agent-b": { active_prompt: null, queued_prompts: [] },
    "agent-c": { active_prompt: null, queued_prompts: [] },
  }
}

function requestBuilders() {
  return {
    attachToSessionRequest: (sessionId, clientId) => ({
      AttachToSession: { session_id: sessionId, client_id: clientId, capability_level: "FullTerminal" },
    }),
    detachFromSessionRequest: (attachmentId) => ({ DetachFromSession: { attachment_id: attachmentId } }),
    getSessionStateRequest: (sessionId) => ({ GetSessionState: { session_id: sessionId } }),
    getRoomEnvironmentStateRequest: (sessionId) => ({ GetRoomEnvironmentState: { session_id: sessionId } }),
    listRoomEnvironmentActionHistoryRequest: (sessionId, beforeSequence = null, limit = null) => ({
      ListRoomEnvironmentActionHistory: {
        session_id: sessionId,
        before_sequence: beforeSequence,
        limit,
      },
    }),
    submitPromptRequest: (sessionId, attachmentId, targetAgentId, promptText, attachments) => ({
      SubmitPrompt: {
        session_id: sessionId,
        attachment_id: attachmentId,
        target_agent_id: targetAgentId,
        prompt: promptText,
        attachments,
      },
    }),
    requestRoomEnvironmentInputTakeoverRequest: (sessionId, target) => ({
      RequestRoomEnvironmentInputTakeover: { session_id: sessionId, target },
    }),
    cancelQueuedPromptRequest: (sessionId, attachmentId, targetAgentId, promptId) => ({
      CancelQueuedPrompt: {
        session_id: sessionId,
        attachment_id: attachmentId,
        target_agent_id: targetAgentId,
        prompt_id: promptId,
      },
    }),
    releaseRoomEnvironmentInputRequest: (sessionId, target) => ({
      ReleaseRoomEnvironmentInput: { session_id: sessionId, target },
    }),
  }
}

function idleCleanupSession(agentIds = ["agent-a", "agent-b", "agent-c"]) {
  return session({ agentIds, promptStates: emptyPromptStates() })
}

function fakeKernel({
  sessionAgentIds = ["agent-a", "agent-b", "agent-c"],
  sequence = null,
  expireAtEnvironmentRead = null,
  environmentOverrides = {},
  clock = null,
  stuckPromptIds = [],
  failDetach = false,
  failRelease = false,
  failAttachAfterAdmission = false,
} = {}) {
  const sent = []
  const submitted = []
  const cancellations = []
  let environmentReads = 0
  let sessionReads = 0
  let historyReads = 0
  let detachAttempts = 0
  let releaseAttempts = 0
  let takeoverRequests = 0
  const canceledIds = new Set()
  const promptIds = [
    "prompt-read-a",
    "prompt-read-b",
    "prompt-read-c",
    "prompt-mutation-a",
    "prompt-mutation-c",
    "prompt-mutation-b",
  ]
  const defaultEnvironment = environment()
  const snapshots = sequence
    ? [
        sequence.baseline,
        sequence.readOverlap,
        sequence.firstMutation,
        sequence.queuedMutation,
        sequence.takeoverGranted,
        sequence.final,
        sequence.final,
      ]
    : [defaultEnvironment]

  function cleanupPromptStates() {
    const states = emptyPromptStates()
    for (const id of stuckPromptIds) {
      const index = promptIds.indexOf(id)
      const agentId = ["agent-a", "agent-b", "agent-c"][index] ?? "agent-a"
      const ownedPrompt = prompt(id, agentId)
      const queued = (states[agentId].queued_prompts ?? []).filter(Boolean)
      if (agentId === "agent-a" && id === "prompt-read-a") {
        states[agentId].active_prompt = ownedPrompt
      } else {
        states[agentId].queued_prompts = [...queued, ownedPrompt]
      }
    }
    if (stuckPromptIds.length > 0) {
      states["agent-c"].queued_prompts.push(prompt("prompt-unrelated", "agent-c", "attachment-other"))
    }
    for (const id of canceledIds) {
      for (const state of Object.values(states)) {
        state.queued_prompts = state.queued_prompts.filter((entry) => entry.id !== id)
      }
    }
    return states
  }

  function sessionForRead(readIndex) {
    if (readIndex === 1) return session({ agentIds: sessionAgentIds })
    return idleCleanupSession(sessionAgentIds)
  }

  const client = {
    async send(request) {
      sent.push(request)
      if (request.AttachToSession) {
        if (failAttachAfterAdmission) throw new Error("fixture attach acknowledgment timeout")
        return { SessionAttached: { attachment: { id: "attachment-e" } } }
      }
      if (request.GetSessionState) {
        sessionReads += 1
        if (sessionReads === 1) return { SessionState: { session: sessionForRead(sessionReads) } }
        return {
          SessionState: {
            session: {
              ...session({ agentIds: sessionAgentIds, promptStates: cleanupPromptStates() }),
              active_prompt: null,
              queued_prompts: [],
            },
          },
        }
      }
      if (request.GetRoomEnvironmentState) {
        environmentReads += 1
        if (expireAtEnvironmentRead === environmentReads && clock) {
          clock.value = clock.runDeadline + 1
        }
        const index = Math.min(environmentReads - 1, snapshots.length - 1)
        const override = environmentOverrides[environmentReads]
        return { RoomEnvironmentState: { environment: override ?? snapshots[index] } }
      }
      if (request.ListRoomEnvironmentActionHistory) {
        historyReads += 1
        const actions = sequence && historyReads > 1 ? sequence.history : []
        return {
          RoomEnvironmentActionHistoryListed: {
            page: { actions, next_before_sequence: null },
          },
        }
      }
      if (request.SubmitPrompt) {
        const submission = request.SubmitPrompt
        const promptId = promptIds[submitted.length] ?? "prompt-extra-" + submitted.length
        submitted.push({ ...submission, promptId })
        return {
          PromptSubmitted: {
            outcome: {
              Started: {
                prompt: prompt(promptId, submission.target_agent_id),
              },
            },
          },
        }
      }
      if (request.RequestRoomEnvironmentInputTakeover) {
        takeoverRequests += 1
        const waiting = sequence?.waitingForTakeover ?? defaultEnvironment
        return {
          RoomEnvironmentTakeoverUpdated: {
            outcome: { state: "cancellation_required", action_ids: ["action-mutation-a"] },
            environment: waiting,
          },
        }
      }
      if (request.CancelQueuedPrompt) {
        const item = request.CancelQueuedPrompt
        cancellations.push(item)
        canceledIds.add(item.prompt_id)
        return {
          QueuedPromptCancelled: {
            prompt: prompt(item.prompt_id, item.target_agent_id),
          },
        }
      }
      if (request.ReleaseRoomEnvironmentInput) {
        releaseAttempts += 1
        if (failRelease) throw new Error("fixture release failure")
        return { RoomEnvironmentInputReleased: { environment: sequence?.afterRelease ?? defaultEnvironment } }
      }
      if (request.DetachFromSession) {
        detachAttempts += 1
        if (failDetach) throw new Error("fixture detach failure")
        return { SessionDetached: {} }
      }
      if (request.CancelActivePrompt) throw new Error("active prompt cancellation must not be requested")
      throw new Error("unexpected request " + Object.keys(request).join(","))
    },
  }
  return {
    client,
    sent,
    submitted,
    cancellations,
    takeoverRequests: () => takeoverRequests,
    detachAttempts: () => detachAttempts,
    releaseAttempts: () => releaseAttempts,
  }
}

test("CLI parsing requires explicit execution and rejects ambiguous agent or tab scope", async () => {
  const parsed = parseArgs([
    "--kernel-url", "ws://127.0.0.1:43118/kernel",
    "--session", "session-e",
    "--agent-a", "agent-a",
    "--agent-b", "agent-b",
    "--agent-c", "agent-c",
    "--same-tab", "tab-same",
    "--other-tab", "tab-other",
    "--output", "/tmp/drill-e.json",
  ])
  assert.equal(parsed.execute, false)
  await assert.rejects(runDrillEScenario({
    client: { send() {} }, requests: requestBuilders(), options: parsed,
  }), /explicit --execute opt-in/)
  assert.throws(() => validateOptions(options({ agentC: "agent-a" })), /three distinct/)
  assert.throws(() => validateOptions(options({ otherTabId: "tab-same" })), /two different/)
})

test("prompt acknowledgments alone remain incomplete and are cleaned up", async () => {
  const kernel = fakeKernel()
  let clock = 1_000
  const capture = await runDrillEScenario({
    client: kernel.client,
    requests: requestBuilders(),
    options: options(),
    now: () => clock,
    sleep: async (ms) => { clock += ms },
  })

  assert.equal(capture.report.status, "incomplete")
  assert.ok(Object.values(capture.report.checks).every((check) => check.status === "incomplete"))
  assert.equal(capture.report.historyActionCount, 0)
  assert.deepEqual(kernel.submitted.map((promptValue) => promptValue.target_agent_id), [
    "agent-a", "agent-b", "agent-c", "agent-a", "agent-c",
  ])
  assert.ok(kernel.submitted[0].prompt.includes("slice_browser_status"))
  assert.ok(kernel.submitted[0].prompt.includes("slice_browser_find"))
  assert.ok(kernel.submitted[2].prompt.includes("tab-other"))
  assert.ok(kernel.submitted[3].prompt.includes("tab-same"))
  assert.deepEqual(capture.cleanup.submittedPrompts.map((entry) => entry.promptId), [
    "prompt-read-a", "prompt-read-b", "prompt-read-c", "prompt-mutation-a", "prompt-mutation-c",
  ])
  assert.equal(capture.cleanup.status, "passed")
  assert.equal(capture.cleanup.detached, true)
  assert.equal(kernel.takeoverRequests(), 0)
  assert.equal(kernel.detachAttempts(), 1)
  assert.ok(kernel.sent.some((request) => request.AttachToSession?.session_id === "session-e"))
  assert.ok(kernel.sent.some((request) => request.SubmitPrompt?.attachment_id === "attachment-e"))
  assert.ok(kernel.sent.some((request) => request.ListRoomEnvironmentActionHistory?.session_id === "session-e"))
})

test("missing named agent fails preflight before prompts and still detaches", async () => {
  const kernel = fakeKernel({ sessionAgentIds: ["agent-a", "agent-b"] })
  await assert.rejects(runDrillEScenario({
    client: kernel.client,
    requests: requestBuilders(),
    options: options(),
  }), /does not contain requested agent agent-c/)
  assert.equal(kernel.submitted.length, 0)
  assert.equal(kernel.takeoverRequests(), 0)
  assert.equal(kernel.detachAttempts(), 1)
})

test("attach acknowledgment timeout reports an unknown attachment instead of claiming detach", async () => {
  const kernel = fakeKernel({ failAttachAfterAdmission: true })
  await assert.rejects(runDrillEScenario({
    client: kernel.client,
    requests: requestBuilders(),
    options: options(),
  }), (error) => {
    assert.equal(error.message, "fixture attach acknowledgment timeout")
    assert.equal(error.cleanup.detached, false)
    assert.equal(error.cleanup.status, "failed")
    assert.ok(error.cleanup.failures.some((item) =>
      item.code === "session_attachment_identity_unavailable"))
    return true
  })
  assert.equal(kernel.detachAttempts(), 0)
})

test("real-shaped kernel sequence proves concurrent reads, serialized mutations, takeover, and another tab", async () => {
  const sequence = fullSequence()
  const kernel = fakeKernel({ sequence })
  let clock = 10_000
  const capture = await runDrillEScenario({
    client: kernel.client,
    requests: requestBuilders(),
    options: options(),
    now: () => clock,
    sleep: async (ms) => { clock += ms },
  })

  assert.equal(capture.report.status, "passed")
  assert.ok(Object.values(capture.report.checks).every((check) => check.status === "passed"))
  assert.deepEqual(capture.report.checks.twoAgentTabReads.readActionIds, [
    "action-read-a", "action-read-b",
  ])
  assert.equal(capture.report.checks.sameTabSerialization.firstActionId, "action-mutation-a")
  assert.equal(capture.report.checks.sameTabSerialization.queuedActionId, "action-mutation-b")
  assert.equal(capture.report.checks.independentTabConcurrency.secondTabActionId, "action-mutation-c")
  assert.equal(capture.report.checks.humanTakeover.cancelledActionId, "action-mutation-b")
  const observedReadOverlap = capture.snapshots.find((sample) => {
    const runningIds = new Set(sample.environment.actions
      .filter((item) => item.state === "running")
      .map((item) => item.action_id))
    return ["action-read-a", "action-read-b", "action-read-c"].every((id) => runningIds.has(id))
  })
  assert.ok(observedReadOverlap, "kernel snapshot did not show all three read-phase actions running together")
  const actionById = new Map(capture.actions.map((item) => [item.action_id, item]))
  assert.ok(intervalsOverlap(actionById.get("action-read-a"), actionById.get("action-read-b")),
    "the two same-tab read actions did not overlap in their kernel action intervals")
  const observedMutationOverlap = capture.snapshots.find((sample) => {
    const runningIds = new Set(sample.environment.actions
      .filter((item) => item.state === "running")
      .map((item) => item.action_id))
    return runningIds.has("action-mutation-a") && runningIds.has("action-mutation-c")
  })
  assert.ok(observedMutationOverlap, "kernel snapshot did not show mutations on separate tabs running together")
  assert.ok(intervalsOverlap(actionById.get("action-mutation-a"), actionById.get("action-mutation-c")),
    "mutations on separate tabs did not overlap in their kernel action intervals")
  assert.ok(capture.snapshots.some((sample) => sample.environment.actions.some((item) =>
    item.action_id === "action-mutation-a" && item.state === "running")
      && sample.environment.actions.some((item) =>
        item.action_id === "action-mutation-b" && item.state === "queued")),
  "kernel snapshot did not show the same-tab mutation serialized in the queue")
  assert.ok(capture.snapshots.every((sample) => {
    const sameTabMutationsRunning = sample.environment.actions.filter((item) =>
      item.actor_id.startsWith("agent:") && item.kind === "browser_history_reload"
        && item.targets?.some((target) => target.kind === "browser_tab" && target.id === "tab-same")
        && item.state === "running")
    return sameTabMutationsRunning.length <= 1
  }), "kernel snapshot showed two same-tab mutations running at once")
  assert.deepEqual(kernel.submitted.map((promptValue) => promptValue.target_agent_id), [
    "agent-a", "agent-b", "agent-c", "agent-a", "agent-c", "agent-b",
  ])
  assert.deepEqual(capture.cleanup.submittedPrompts.map((entry) => entry.promptId), [
    "prompt-read-a", "prompt-read-b", "prompt-read-c",
    "prompt-mutation-a", "prompt-mutation-c", "prompt-mutation-b",
  ])
  assert.equal(capture.cleanup.status, "passed")
  assert.equal(capture.cleanup.detached, true)
  assert.equal(capture.cleanup.takeover.status, "released")
  assert.equal(kernel.releaseAttempts(), 1)
  assert.equal(kernel.detachAttempts(), 1)
})

test("strict observer rejects wrong actors, environments, generations, identities, and unchanged ownership", async () => {
  const sequence = fullSequence()
  const kernel = fakeKernel({ sequence })
  let clock = 10_000
  const capture = await runDrillEScenario({
    client: kernel.client,
    requests: requestBuilders(),
    options: options(),
    now: () => clock,
    sleep: async (ms) => { clock += ms },
  })
  const baseline = {
    sessionId: "session-e",
    snapshots: capture.snapshots,
    finalEnvironment: capture.finalEnvironment,
    actions: capture.actions,
    baselineActionSequence: capture.report.baselineActionSequence,
  }

  const wrongActor = structuredClone(baseline)
  wrongActor.actions.find((item) => item.action_id === "action-read-b").actor_id = "agent:agent-c"
  assert.equal(verifyDrillE(wrongActor).checks.twoAgentTabReads.status, "incomplete")

  const wrongTakeoverActor = structuredClone(baseline)
  wrongTakeoverActor.snapshots.find((sample) =>
    sample.environment.pending_input_takeovers.length > 0)
    .environment.pending_input_takeovers[0].human_actor_id = "agent:agent-c"
  assert.equal(verifyDrillE(wrongTakeoverActor).checks.humanTakeover.status, "incomplete")

  const wrongEnvironment = structuredClone(baseline)
  wrongEnvironment.snapshots[1].environment.environment_id = "another-environment"
  assert.throws(() => verifyDrillE(wrongEnvironment), /identity changed/)

  const wrongEnvironmentGeneration = structuredClone(baseline)
  wrongEnvironmentGeneration.snapshots[1].environment.runtime_generation = 5
  assert.throws(() => verifyDrillE(wrongEnvironmentGeneration), /runtime generation changed/)

  const wrongActionGeneration = structuredClone(baseline)
  wrongActionGeneration.actions.find((item) => item.action_id === "action-read-a").runtime_generation = 3
  assert.equal(verifyDrillE(wrongActionGeneration).checks.twoAgentTabReads.status, "incomplete")

  const wrongActionIdentity = structuredClone(baseline)
  wrongActionIdentity.actions.find((item) => item.action_id === "action-read-b").action_id = "action-substituted"
  assert.equal(verifyDrillE(wrongActionIdentity).checks.twoAgentTabReads.status, "incomplete")

  const unchangedOwnership = structuredClone(baseline)
  unchangedOwnership.finalEnvironment.input_ownership = []
  assert.equal(verifyDrillE(unchangedOwnership).checks.humanTakeover.status, "incomplete")
})

test("timeout gets an independent cleanup budget, cancels only exact queued prompts, and attempts detach", async () => {
  const clock = { value: 1_000, runDeadline: 2_000 }
  const kernel = fakeKernel({
    expireAtEnvironmentRead: 2,
    clock,
    stuckPromptIds: ["prompt-read-a", "prompt-read-b", "prompt-read-c"],
  })
  await assert.rejects(runDrillEScenario({
    client: kernel.client,
    requests: requestBuilders(),
    options: options(),
    now: () => clock.value,
    sleep: async (ms) => { clock.value += ms },
  }), (error) => {
    assert.equal(error.message, "Drill E request deadline expired")
    assert.equal(error.cleanup.status, "failed")
    assert.equal(error.cleanup.detached, true)
    assert.ok(error.cleanup.failures.some((item) =>
      item.code === "owned_active_prompt_cancel_by_id_unavailable"
        && item.promptId === "prompt-read-a"))
    return true
  })

  assert.deepEqual(kernel.cancellations.map((item) => item.prompt_id), [
    "prompt-read-b", "prompt-read-c",
  ])
  assert.ok(kernel.cancellations.every((item) => item.attachment_id === "attachment-e"))
  assert.equal(kernel.detachAttempts(), 1)
  assert.ok(clock.value > clock.runDeadline)
  assert.ok(!kernel.sent.some((request) => request.CancelActivePrompt))
})

test("detach and takeover release failures make an otherwise passing capture fail", async () => {
  const sequence = fullSequence()
  const kernel = fakeKernel({ sequence, failDetach: true, failRelease: true })
  let clock = 10_000
  const capture = await runDrillEScenario({
    client: kernel.client,
    requests: requestBuilders(),
    options: options(),
    now: () => clock,
    sleep: async (ms) => { clock += ms },
  })

  assert.equal(capture.report.status, "failed")
  assert.equal(capture.cleanup.status, "failed")
  assert.equal(capture.cleanup.detached, false)
  assert.ok(capture.cleanup.failures.some((item) => item.code === "session_detach_failed"))
  assert.ok(capture.cleanup.failures.some((item) => item.code === "owned_takeover_release_failed"))
})

test("cleanup will not release a takeover after the Environment runtime changes", async () => {
  const sequence = fullSequence()
  const restartedOwner = environment({
    eventCursor: 17,
    environmentId: "environment-after-restart",
    runtimeGeneration: 5,
    ownership: [{ target: { kind: "browser_tab", id: "tab-same" }, actor_id: "user:operator" }],
  })
  const kernel = fakeKernel({
    sequence,
    environmentOverrides: { 7: restartedOwner },
  })
  let clock = 10_000
  const capture = await runDrillEScenario({
    client: kernel.client,
    requests: requestBuilders(),
    options: options(),
    now: () => clock,
    sleep: async (ms) => { clock += ms },
  })

  assert.equal(capture.report.status, "failed")
  assert.equal(capture.cleanup.takeover.status, "environment_changed")
  assert.ok(capture.cleanup.failures.some((item) => item.code === "takeover_environment_changed"))
  assert.equal(kernel.releaseAttempts(), 0)
  assert.equal(kernel.detachAttempts(), 1)
})

function acceptedCapture() {
  const sequence = fullSequence()
  return {
    report: {
      schema: "chariox.browser_computer.drill_e.kernel_evidence.v1",
      status: "passed",
      sessionId: "session-e",
      environmentId: "environment-e",
      runtimeGeneration: 4,
      baselineActionSequence: 0,
      checks: Object.fromEntries([
        "twoAgentTabReads", "sameTabSerialization", "independentTabConcurrency", "humanTakeover",
      ].map((name) => [name, { status: "passed" }])),
    },
    snapshots: [{ observed_at_ms: 1_000, environment: sequence.baseline }],
    finalEnvironment: sequence.final,
    actions: sequence.history,
    kernelStatePollCount: 1,
    cleanup: {
      status: "passed",
      stage: "finished",
      budgetMs: 5_000,
      detachReserveMs: 1_000,
      detached: true,
      attachmentId: "attachment-e",
      environmentId: "environment-e",
      runtimeGeneration: 4,
      submittedPrompts: [],
      takeover: { requested: false, targetTabId: "tab-same", status: "not_requested" },
      failures: [],
    },
  }
}

async function runInjectedCli({
  output = memoryOutput(),
  cliOptions = options(),
  runScenario,
  client = null,
  requests = null,
} = {}) {
  const messages = []
  const result = await runDrillEScenarioCli({
    options: cliOptions,
    client,
    requests,
    runScenario,
    reserveOutput: async (outputPath) => {
      assert.equal(outputPath, cliOptions.output)
      return output
    },
    logger: {
      log: (message) => messages.push(message),
      error: (message) => messages.push(message),
    },
    capturedAt: () => "2026-09-26T00:00:00.000Z",
  })
  return { output, result, messages }
}

test("CLI write path fsyncs a successful capture without turning it into a diagnostic", async () => {
  const { output, result } = await runInjectedCli({ runScenario: async () => acceptedCapture() })
  const document = JSON.parse(output.state.bytes.toString("utf8"))

  assert.equal(result.status, "passed")
  assert.equal(result.exitCode, 0)
  assert.equal(document.status, "passed")
  assert.equal(document.diagnostic, undefined)
  assert.equal(output.state.truncates, 1)
  assert.equal(output.state.syncs, 1)
  assert.equal(output.state.closed, true)
  assert.ok(output.state.bytes.byteLength <= 4 * 1024 * 1024)
})

test("CLI persists redacted timeout diagnostics and cleanup failures", async () => {
  const clock = { value: 1_000, runDeadline: 2_000 }
  const kernel = fakeKernel({
    expireAtEnvironmentRead: 2,
    clock,
    stuckPromptIds: ["prompt-read-a", "prompt-read-b", "prompt-read-c"],
  })
  const cliOptions = options()
  const { output, result } = await runInjectedCli({
    cliOptions,
    client: kernel.client,
    requests: requestBuilders(),
    runScenario: (input) => runDrillEScenario({
      ...input,
      now: () => clock.value,
      sleep: async (ms) => { clock.value += ms },
    }),
  })
  const document = JSON.parse(output.state.bytes.toString("utf8"))

  assert.equal(result.status, "failed")
  assert.equal(result.diagnostic, true)
  assert.equal(result.exitCode, 1)
  assert.equal(document.status, "failed")
  assert.equal(document.acceptanceClaimed, false)
  assert.equal(document.failure.stage, "final_environment_observation")
  assert.equal(document.failure.code, "scenario_timed_out")
  assert.deepEqual(document.unexecutedChecks, [
    "twoAgentTabReads", "sameTabSerialization", "independentTabConcurrency", "humanTakeover",
  ])
  assert.deepEqual(document.identities, {
    sessionId: "session-e",
    agentIds: ["agent-a", "agent-b", "agent-c"],
    tabIds: ["tab-same", "tab-other"],
    attachmentId: "attachment-e",
    environmentId: "environment-e",
    runtimeGeneration: 4,
  })
  assert.equal(document.cleanup.status, "failed")
  assert.equal(document.cleanup.detached, true)
  assert.ok(document.cleanup.failures.some((failure) =>
    failure.code === "owned_active_prompt_cancel_by_id_unavailable"
      && failure.promptId === "prompt-read-a"))
  assert.equal(document.cleanup.submittedPrompts[0].promptId, "prompt-read-a")
  assert.equal(output.state.syncs, 1)
  assert.equal(output.state.closed, true)
  assert.ok(!output.state.bytes.toString("utf8").includes("request deadline expired"))
})

test("CLI persists unknown attachment acknowledgment diagnostics with all checks unexecuted", async () => {
  const kernel = fakeKernel({ failAttachAfterAdmission: true })
  const { output, result } = await runInjectedCli({
    client: kernel.client,
    requests: requestBuilders(),
    runScenario: (input) => runDrillEScenario(input),
  })
  const document = JSON.parse(output.state.bytes.toString("utf8"))

  assert.equal(result.status, "failed")
  assert.equal(document.failure.stage, "attach")
  assert.equal(document.failure.code, "attachment_acknowledgment_unknown")
  assert.equal(document.identities.attachmentId, null)
  assert.equal(document.identities.environmentId, null)
  assert.equal(document.identities.runtimeGeneration, null)
  assert.equal(document.cleanup.detached, false)
  assert.ok(document.cleanup.failures.some((failure) =>
    failure.code === "session_attachment_identity_unavailable"))
  assert.deepEqual(document.unexecutedChecks, [
    "twoAgentTabReads", "sameTabSerialization", "independentTabConcurrency", "humanTakeover",
  ])
  assert.equal(document.acceptanceClaimed, false)
})

test("CLI replaces a partial failed write with a bounded failed diagnostic", async () => {
  const output = memoryOutput({ failFirstWrite: true })
  const { result } = await runInjectedCli({ output, runScenario: async () => acceptedCapture() })
  const serialized = output.state.bytes.toString("utf8")
  const document = JSON.parse(serialized)

  assert.equal(result.status, "failed")
  assert.equal(result.diagnostic, true)
  assert.equal(result.exitCode, 1)
  assert.equal(document.status, "failed")
  assert.equal(document.acceptanceClaimed, false)
  assert.equal(document.failure.stage, "evidence_write")
  assert.equal(document.failure.code, "evidence_write_failed")
  assert.deepEqual(document.unexecutedChecks, [])
  assert.equal(document.cleanup.status, "passed")
  assert.equal(document.identities.attachmentId, "attachment-e")
  assert.equal(document.identities.environmentId, "environment-e")
  assert.equal(document.identities.runtimeGeneration, 4)
  assert.equal(document.report, undefined)
  assert.equal(document.checks, undefined)
  assert.equal(output.state.truncates, 2)
  assert.equal(output.state.syncs, 1)
  assert.equal(output.state.closed, true)
  assert.ok(output.state.bytes.byteLength <= 4 * 1024 * 1024)
  assert.ok(!serialized.includes("injected output write failure"))
})

test("CLI reserves external evidence exclusively with mode 600 and does not overwrite it", async () => {
  const temporaryDirectory = await mkdtemp(path.join(tmpdir(), "drill-e-cli-test-"))
  try {
    const outputPath = path.join(temporaryDirectory, "evidence.json")
    const cliOptions = options({ output: outputPath })
    const result = await runDrillEScenarioCli({
      options: cliOptions,
      runScenario: async () => acceptedCapture(),
      logger: { log() {}, error() {} },
      capturedAt: () => "2026-09-26T00:00:00.000Z",
    })
    const firstDocument = await readFile(outputPath, "utf8")
    const outputStat = await stat(outputPath)

    assert.equal(result.status, "passed")
    assert.equal(outputStat.mode & 0o777, 0o600)
    assert.ok(outputStat.size <= 4 * 1024 * 1024)
    assert.equal(JSON.parse(firstDocument).status, "passed")
    await assert.rejects(runDrillEScenarioCli({
      options: cliOptions,
      runScenario: async () => acceptedCapture(),
      logger: { log() {}, error() {} },
    }), { code: "EEXIST" })
    assert.equal(await readFile(outputPath, "utf8"), firstDocument)
  } finally {
    await rm(temporaryDirectory, { recursive: true, force: true })
  }
})

test("timeout with an unresolved takeover reports the missing pending release seam", async () => {
  const sequence = fullSequence()
  const clock = { value: 1_000, runDeadline: 2_000 }
  const kernel = fakeKernel({
    sequence,
    expireAtEnvironmentRead: 5,
    environmentOverrides: {
      5: sequence.waitingForTakeover,
      6: sequence.waitingForTakeover,
    },
    clock,
  })
  await assert.rejects(runDrillEScenario({
    client: kernel.client,
    requests: requestBuilders(),
    options: options(),
    now: () => clock.value,
    sleep: async (ms) => { clock.value += ms },
  }), (error) => {
    assert.equal(error.cleanup.detached, true)
    assert.ok(error.cleanup.failures.some((item) =>
      item.code === "pending_takeover_cannot_be_retracted"
        && item.targetTabId === "tab-same"))
    assert.equal(error.cleanup.takeover.status, "pending_cancel_seam_missing")
    return true
  })
  assert.equal(kernel.takeoverRequests(), 1)
  assert.equal(kernel.releaseAttempts(), 0)
  assert.equal(kernel.detachAttempts(), 1)
})
