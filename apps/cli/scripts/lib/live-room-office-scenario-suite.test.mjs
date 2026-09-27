import assert from "node:assert/strict"
import test from "node:test"
import {
  listRoomOfficeScenarios,
  preflightRoomOfficeScenarioSuite,
  runRoomOfficeScenarioSuite,
} from "./live-room-office-scenario-suite.mjs"

const scenarioId = "document-intake-follow-up"

function requestBuilders() {
  return {
    getSessionStateRequest: (sessionId) => ({ GetSessionState: { session_id: sessionId } }),
    listAgentsRequest: (sessionId) => ({ ListAgents: { session_id: sessionId } }),
    listRoomEnvironmentActionHistoryRequest: (sessionId, beforeSequence, limit) => ({
      ListRoomEnvironmentActionHistory: { session_id: sessionId, before_sequence: beforeSequence, limit },
    }),
    attachToSessionRequest: (sessionId, clientId) => ({ AttachToSession: { session_id: sessionId, client_id: clientId } }),
    detachFromSessionRequest: (attachmentId) => ({ DetachFromSession: { attachment_id: attachmentId } }),
    submitPromptRequest: (sessionId, attachmentId, agentId, prompt, attachments) => ({
      SubmitPrompt: { session_id: sessionId, attachment_id: attachmentId, agent_id: agentId, prompt, attachments },
    }),
    cancelQueuedPromptRequest: (sessionId, attachmentId, agentId, promptId) => ({
      CancelQueuedPrompt: { session_id: sessionId, attachment_id: attachmentId, target_agent_id: agentId, prompt_id: promptId },
    }),
    getSessionHistoryOutlineRequest: (sessionId, agentIds, latestPromptCount) => ({
      GetSessionHistoryOutline: { session_id: sessionId, agent_ids: agentIds, latest_prompt_count: latestPromptCount },
    }),
    captureRoomEnvironmentScreenshotRequest: (sessionId, attachmentId) => ({
      CaptureRoomEnvironmentScreenshot: { session_id: sessionId, attachment_id: attachmentId },
    }),
    readRoomEnvironmentScreenshotChunkRequest: (sessionId, attachmentId, artifactId, offset, maxBytes) => ({
      ReadRoomEnvironmentScreenshotChunk: { session_id: sessionId, attachment_id: attachmentId, artifact_id: artifactId, offset, max_bytes: maxBytes },
    }),
  }
}

function pngFixture() {
  return Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADUlEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC",
    "base64",
  )
}

function suiteInput(client, overrides = {}) {
  const cleanup = overrides.cleanup ?? []
  return {
    scenarioIds: [scenarioId],
    tasks: { [scenarioId]: "Extract the action items and draft the requested follow-up." },
    confirmedScenarios: [scenarioId],
    execute: true,
    kernelUrl: "ws://127.0.0.1:43118",
    sessionId: "session-1",
    agentId: "agent-1",
    timeoutMs: 1_000,
    pollMs: 100,
    sleep: async () => {},
    writeScreenshot: async ({ fileName, bytes }) => { overrides.screenshot = { fileName, bytes } },
    recordCleanup: (entry) => cleanup.push(entry),
    client,
    requests: requestBuilders(),
    ...overrides,
    cleanup,
  }
}

test("catalog retains the canonical five office scenarios and evidence needs", () => {
  const manifest = listRoomOfficeScenarios()
  assert.equal(manifest.length, 5)
  assert.equal(manifest[0].id, "email-gated-onboarding")
  assert.ok(manifest.every((item) => item.phases.length > 0 && item.requiredEvidence.length > 0))
  assert.equal(JSON.stringify(manifest).includes("fixture-"), false)
})

test("preflight distinguishes missing operator decisions from runtime availability", () => {
  const plan = preflightRoomOfficeScenarioSuite({ scenarioIds: [scenarioId] })
  assert.equal(plan.status, "required_user_action")
  assert.ok(plan.requiredUserActions.includes("explicit_execute_flag"))
  assert.ok(plan.requiredUserActions.includes(`explicit_external_action_approval:${scenarioId}`))
})

test("kernel-shaped fixture responses produce allowlisted action observations and a bounded screenshot", async () => {
  const png = pngFixture()
  const sha256 = (await import("node:crypto")).createHash("sha256").update(png).digest("hex")
  const sent = []
  const requests = requestBuilders()
  const client = {
    async send(request) {
      sent.push(request)
      if (request.GetSessionState) return { SessionStateLoaded: { session: { id: "session-1", agent_activity: {} } } }
      if (request.ListAgents) return { AgentsListed: { agents: [{ id: "agent-1" }] } }
      if (request.ListRoomEnvironmentActionHistory) {
        const query = request.ListRoomEnvironmentActionHistory
        if (query.limit === 1) return { RoomEnvironmentActionHistoryListed: { page: { actions: [{ sequence: 4 }], next_before_sequence: null } } }
        return { RoomEnvironmentActionHistoryListed: { page: { actions: [
          { action_id: "action-5", sequence: 5, actor_id: "agent:agent-1", mode: "browser", kind: "browser_click", state: "completed", submitted_at_ms: 10, started_at_ms: 11, finished_at_ms: 12, targets: [{ kind: "browser_tab", id: "tab-1" }], arguments: { text: "private" } },
          { action_id: "other-6", sequence: 6, actor_id: "agent:agent-2", mode: "browser", kind: "submit", state: "completed", targets: [] },
        ], next_before_sequence: null } } }
      }
      if (request.AttachToSession) return { SessionAttached: { attachment: { id: "attachment-1" } } }
      if (request.SubmitPrompt) return { PromptSubmitted: { outcome: { Started: { prompt: { id: "prompt-1" } } } } }
      if (request.GetSessionHistoryOutline) return { SessionHistoryOutline: { agents: [{ agent_id: "agent-1", turns: [{ prompt_id: "prompt-1", lifecycle: "completed" }] }] } }
      if (request.CaptureRoomEnvironmentScreenshot) return { RoomEnvironmentScreenshotCaptured: { artifact: { artifact_id: "screenshot-1", media_type: "image/png", size_bytes: png.length, sha256 } } }
      if (request.ReadRoomEnvironmentScreenshotChunk) return { RoomEnvironmentScreenshotChunk: { chunk: { artifact_id: "screenshot-1", offset: 0, data_base64: png.toString("base64"), eof: true } } }
      if (request.DetachFromSession) return { SessionDetached: {} }
      throw new Error("unexpected kernel request")
    },
  }
  const input = suiteInput(client)
  const report = await runRoomOfficeScenarioSuite(input)
  assert.equal(report.status, "observed")
  assert.equal(report.acceptanceAssessment, "not_performed")
  assert.equal(report.scenarios[0].providerTurnLifecycle, "completed")
  assert.equal(report.scenarios[0].actionAttribution, "selected_agent_and_sequence_window_only")
  assert.deepEqual(report.scenarios[0].roomActions.map((action) => action.actionId), ["action-5"])
  assert.equal("arguments" in report.scenarios[0].roomActions[0], false)
  assert.equal(report.scenarios[0].screenshot.sha256, sha256)
  assert.ok(report.scenarios[0].evidenceStillRequired.includes("mail_sent"))
  assert.equal(JSON.stringify(report).includes("Extract the action items"), false)
  assert.ok(sent.some((request) => request.SubmitPrompt))
  assert.ok(sent.some((request) => request.ListRoomEnvironmentActionHistory))
  assert.deepEqual(input.cleanup.map((entry) => entry.status), ["detached"])
})

test("a concurrently queued prompt is cancelled and requires operator review", async () => {
  const sent = []
  const client = {
    async send(request) {
      sent.push(request)
      if (request.GetSessionState) return { SessionStateLoaded: { session: { id: "session-1", agent_activity: {} } } }
      if (request.ListAgents) return { AgentsListed: { agents: [{ id: "agent-1" }] } }
      if (request.ListRoomEnvironmentActionHistory) return { RoomEnvironmentActionHistoryListed: { page: { actions: [], next_before_sequence: null } } }
      if (request.AttachToSession) return { SessionAttached: { attachment: { id: "attachment-1" } } }
      if (request.SubmitPrompt) return { PromptSubmitted: { outcome: { Queued: { prompt: { id: "prompt-queued" } } } } }
      if (request.CancelQueuedPrompt) return { QueuedPromptCancelled: { prompt: { id: "prompt-queued", status: "cancelled" } } }
      if (request.DetachFromSession) return { SessionDetached: {} }
      throw new Error("unexpected kernel request")
    },
  }
  const report = await runRoomOfficeScenarioSuite(suiteInput(client))
  assert.equal(report.status, "required_user_action")
  assert.equal(report.scenarios[0].queuedPromptCancellation, "cancelled")
  assert.ok(sent.some((request) => request.CancelQueuedPrompt))
  assert.equal(sent.some((request) => request.CaptureRoomEnvironmentScreenshot), false)
})

test("an ambiguous prompt submission is surfaced for review and never retried", async () => {
  const sent = []
  const client = {
    async send(request) {
      sent.push(request)
      if (request.GetSessionState) return { SessionStateLoaded: { session: { id: "session-1", agent_activity: {} } } }
      if (request.ListAgents) return { AgentsListed: { agents: [{ id: "agent-1" }] } }
      if (request.ListRoomEnvironmentActionHistory) return { RoomEnvironmentActionHistoryListed: { page: { actions: [], next_before_sequence: null } } }
      if (request.AttachToSession) return { SessionAttached: { attachment: { id: "attachment-1" } } }
      if (request.SubmitPrompt) throw new Error("fixture transport closes after submission")
      if (request.DetachFromSession) return { SessionDetached: {} }
      throw new Error("unexpected kernel request")
    },
  }
  const report = await runRoomOfficeScenarioSuite(suiteInput(client))
  assert.equal(report.status, "required_user_action")
  assert.equal(report.scenarios[0].reason, "prompt_submission_result_ambiguous_do_not_retry")
  assert.equal(sent.filter((request) => request.SubmitPrompt).length, 1)
  assert.deepEqual(report.cleanup.observerAttachments.map((entry) => entry.status), ["detached"])
})
