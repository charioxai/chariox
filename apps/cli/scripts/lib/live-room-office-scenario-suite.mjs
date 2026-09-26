import { createHash } from "node:crypto"

export const ROOM_OFFICE_SCENARIO_SUITE_CONTRACT = "chariox.room-office-scenario-suite.v1"
export const ROOM_OFFICE_SCENARIO_IDS = Object.freeze([
  "email-gated-onboarding",
  "vendor-research-crm",
  "document-intake-follow-up",
  "public-api-extension",
  "support-ticket-lifecycle",
])

const MAX_HISTORY_PAGES = 20
const HISTORY_PAGE_SIZE = 100
const MAX_SCREENSHOT_BYTES = 16 * 1024 * 1024
const PNG_SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])
const safeId = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/
const safeActionKind = /^[A-Za-z][A-Za-z0-9._:_-]{0,127}$/
const definitions = Object.freeze([
  {
    id: ROOM_OFFICE_SCENARIO_IDS[0], title: "Email-gated SaaS onboarding",
    phases: ["credential-ready", "service-registered", "email-confirmed", "onboarding-complete"],
    authorities: ["browser", "computer", "vault"],
    evidence: ["screenshot", "screenshot", "screenshot", "transcript_scan", "vault_metadata"],
  },
  {
    id: ROOM_OFFICE_SCENARIO_IDS[1], title: "Vendor research to CRM-like entry",
    phases: ["research-complete", "extension-registered", "comparison-recorded", "request-submitted"],
    authorities: ["browser", "computer", "extension", "artifact"],
    evidence: ["screenshot", "extension_registry", "artifact", "screenshot"],
  },
  {
    id: ROOM_OFFICE_SCENARIO_IDS[2], title: "Document intake and follow-up email",
    phases: ["document-intake", "draft-ready", "mail-sent"],
    authorities: ["browser", "computer", "artifact"],
    evidence: ["screenshot", "screenshot", "mail_sent", "transcript_scan"],
  },
  {
    id: ROOM_OFFICE_SCENARIO_IDS[3], title: "Public API workflow with agent-created extension",
    phases: ["api-selected", "extension-registered", "extension-granted", "result-recorded"],
    authorities: ["browser", "extension", "artifact"],
    evidence: ["extension_registry", "tool_output", "screenshot", "artifact"],
  },
  {
    id: ROOM_OFFICE_SCENARIO_IDS[4], title: "Support ticket lifecycle",
    phases: ["portal-registered", "ticket-created", "email-confirmed", "ticket-updated"],
    authorities: ["browser", "computer", "vault", "artifact"],
    evidence: ["screenshot", "screenshot", "screenshot", "artifact", "transcript_scan", "vault_metadata"],
  },
])

const requiredOperatorActions = Object.freeze([
  "review the exact scenario task and target service",
  "provide only already-authorized account and credential access through Chariox",
  "approve external account, email, form, extension, or ticket changes for this scenario",
])
const postRunActions = Object.freeze([
  "review each external result with the operator and target service",
  "collect the remaining scenario evidence listed in the capture",
  "resolve external side effects and cleanup with the operator; no provider account is deleted automatically",
])

export function listRoomOfficeScenarios() {
  return definitions.map(({ id, title, phases, authorities, evidence }) => ({
    id, title, phases: [...phases], authorities: [...authorities], requiredEvidence: [...evidence],
  }))
}

export function preflightRoomOfficeScenarioSuite(input = {}) {
  const scenarioIds = Array.isArray(input.scenarioIds) ? input.scenarioIds : []
  const missing = []
  if (scenarioIds.length === 0) missing.push("select_at_least_one_scenario")
  if (scenarioIds.some((id) => !ROOM_OFFICE_SCENARIO_IDS.includes(id))
    || new Set(scenarioIds).size !== scenarioIds.length) missing.push("correct_scenario_selection")
  if (input.execute !== true) missing.push("explicit_execute_flag")
  if (!safeId.test(String(input.sessionId ?? ""))) missing.push("existing_session_id")
  if (!safeId.test(String(input.agentId ?? ""))) missing.push("existing_agent_id")
  if (!isSafeKernelUrl(input.kernelUrl)) missing.push("kernel_endpoint")
  if (typeof input.writeScreenshot !== "function") missing.push("private_external_evidence_directory")
  if (!Number.isInteger(input.timeoutMs) || input.timeoutMs < 1_000 || input.timeoutMs > 900_000) missing.push("bounded_timeout")
  if (!Number.isInteger(input.pollMs) || input.pollMs < 100 || input.pollMs > 5_000) missing.push("bounded_poll_interval")
  const tasks = input.tasks && typeof input.tasks === "object" ? input.tasks : {}
  const confirmations = new Set(input.confirmedScenarios ?? [])
  if (Object.keys(tasks).some((id) => !scenarioIds.includes(id))
    || [...confirmations].some((id) => !scenarioIds.includes(id))) missing.push("limit_tasks_and_approvals_to_selected_scenarios")
  for (const id of scenarioIds) {
    if (typeof tasks[id] !== "string" || tasks[id].trim().length < 8 || tasks[id].length > 20_000) {
      missing.push(`scenario_task:${id}`)
    }
    if (!confirmations.has(id)) missing.push(`explicit_external_action_approval:${id}`)
  }
  return {
    contract: ROOM_OFFICE_SCENARIO_SUITE_CONTRACT,
    status: missing.length ? "required_user_action" : "ready",
    scenarioIds: scenarioIds.filter((id) => ROOM_OFFICE_SCENARIO_IDS.includes(id)),
    requiredUserActions: [...new Set(missing)].length ? [...new Set(missing)] : [...requiredOperatorActions],
    ...(missing.length ? { reason: "operator_inputs_or_scenario_approval_missing" } : {}),
  }
}

export async function runRoomOfficeScenarioSuite(input = {}) {
  const preflight = preflightRoomOfficeScenarioSuite(input)
  if (preflight.status !== "ready") return preflight
  if (!input.client || typeof input.client.send !== "function" || !input.requests) {
    return unavailable(preflight.scenarioIds, "kernel_client_unavailable")
  }

  const report = {
    contract: ROOM_OFFICE_SCENARIO_SUITE_CONTRACT,
    source: "kernel-room-observation",
    status: "observed",
    acceptanceAssessment: "not_performed",
    sessionId: input.sessionId,
    agentId: input.agentId,
    startedAt: new Date().toISOString(),
    scenarios: [],
    cleanup: { observerAttachments: [], externalSideEffects: "not_automatically_reversible" },
    requiredUserActions: [...postRunActions],
  }
  const cleanupEntries = []
  const scenarioInput = {
    ...input,
    recordCleanup(entry) {
      cleanupEntries.push({
        scenarioId: entry.scenarioId,
        attachmentId: safeId.test(String(entry.attachmentId ?? "")) ? entry.attachmentId : "omitted",
        status: ["detached", "detach_unconfirmed"].includes(entry.status) ? entry.status : "unavailable",
      })
      try { input.recordCleanup?.(entry) } catch {}
    },
  }

  for (const scenarioId of preflight.scenarioIds) {
    let result
    try {
      result = await runScenario(scenarioInput, definitions.find((item) => item.id === scenarioId))
    } catch {
      result = { scenarioId, status: "unavailable", reason: "kernel_protocol_or_transport_unavailable" }
    }
    report.scenarios.push(result)
    if (result.status !== "observed") {
      for (const remaining of preflight.scenarioIds.slice(report.scenarios.length)) {
        report.scenarios.push({ scenarioId: remaining, status: "not_started", reason: "prior_scenario_requires_operator_review" })
      }
      report.status = result.status
      break
    }
  }
  report.cleanup.observerAttachments = cleanupEntries
  if (cleanupEntries.some((entry) => entry.status !== "detached") && report.status === "observed") {
    report.status = "required_user_action"
  }
  report.finishedAt = new Date().toISOString()
  return report
}

async function runScenario(input, definition) {
  const { client, requests } = input
  const sessionResponse = await client.send(requests.getSessionStateRequest(input.sessionId))
  const session = sessionResponse?.SessionStateLoaded?.session ?? sessionResponse?.SessionState?.session
  if (!session || session.id !== input.sessionId) {
    return { scenarioId: definition.id, status: "required_user_action", reason: "existing_session_unavailable" }
  }
  const activity = session.agent_activity?.[input.agentId]
  if (activity?.busy === true || ["queued", "dispatching", "running", "cancelling", "settling"].includes(activity?.prompt_status)) {
    return { scenarioId: definition.id, status: "required_user_action", reason: "selected_agent_is_busy" }
  }
  const agentsResponse = await client.send(requests.listAgentsRequest(input.sessionId))
  const agents = agentsResponse?.AgentsListed?.agents ?? []
  if (!agents.some((agent) => agent.id === input.agentId)) {
    return { scenarioId: definition.id, status: "required_user_action", reason: "selected_agent_not_in_session" }
  }

  const baseline = await latestActionSequence(client, requests, input.sessionId)
  const attachmentResponse = await client.send(requests.attachToSessionRequest(
    input.sessionId, `office-suite-${process.pid}-${definition.id}`,
  ))
  const attachment = attachmentResponse?.SessionAttached?.attachment
  if (!safeId.test(String(attachment?.id ?? ""))) {
    return { scenarioId: definition.id, status: "unavailable", reason: "observer_attachment_unavailable" }
  }
  let cleanup = "detach_pending"
  try {
    const prompt = buildScenarioPrompt(definition.id, input.tasks[definition.id])
    let submitted
    try {
      submitted = await client.send(requests.submitPromptRequest(
        input.sessionId, attachment.id, input.agentId, prompt, [],
      ))
    } catch {
      return {
        scenarioId: definition.id, status: "required_user_action",
        reason: "prompt_submission_result_ambiguous_do_not_retry",
      }
    }
    const outcome = submitted?.PromptSubmitted?.outcome
    const promptId = outcome?.Started?.prompt?.id ?? outcome?.Queued?.prompt?.id
    if (!safeId.test(String(promptId ?? ""))) {
      return { scenarioId: definition.id, status: "required_user_action", reason: "prompt_submission_unconfirmed_do_not_retry" }
    }
    if (outcome?.Queued) {
      let cancellation = "unconfirmed"
      try {
        const cancelled = await client.send(requests.cancelQueuedPromptRequest(
          input.sessionId, attachment.id, input.agentId, promptId,
        ))
        const cancelledPrompt = cancelled?.QueuedPromptCancelled?.prompt
        cancellation = cancelledPrompt?.id === promptId && cancelledPrompt.status === "cancelled"
          ? "cancelled" : "unconfirmed"
      } catch {
        cancellation = "unconfirmed"
      }
      return {
        scenarioId: definition.id, status: "required_user_action", reason: "agent_busy_prompt_queued",
        promptId, queuedPromptCancellation: cancellation,
      }
    }

    let lifecycle
    try {
      lifecycle = await waitForPrompt(client, requests, input.sessionId, input.agentId, promptId,
        input.timeoutMs, input.pollMs, input.sleep)
    } catch {
      return {
        scenarioId: definition.id, status: "required_user_action",
        reason: "provider_turn_state_unavailable_after_submission_do_not_retry", promptId,
      }
    }
    if (lifecycle !== "completed") {
      return {
        scenarioId: definition.id, status: "required_user_action",
        reason: lifecycle === "open" ? "provider_turn_still_open" : `provider_turn_${lifecycle}`,
        promptId, lifecycle,
      }
    }
    let history
    try {
      history = await actionHistorySince(client, requests, input.sessionId, input.agentId, baseline)
    } catch {
      return {
        scenarioId: definition.id, status: "required_user_action",
        reason: "room_action_history_unavailable_after_turn_do_not_retry", promptId,
        providerTurnLifecycle: lifecycle,
      }
    }
    const screenshot = await captureScreenshot(client, requests, input.sessionId, attachment.id)
    let screenshotRecord = { status: "unavailable", reason: "room_screenshot_unavailable" }
    if (screenshot) {
      try {
        await input.writeScreenshot({ fileName: `${definition.id}.png`, bytes: screenshot.bytes })
        screenshotRecord = {
          status: "captured", fileName: `${definition.id}.png`, sha256: screenshot.sha256,
          sizeBytes: screenshot.sizeBytes,
        }
      } catch {
        screenshotRecord = { status: "unavailable", reason: "private_evidence_write_failed" }
      }
    }
    const followUpReasons = []
    if (!history.complete) followUpReasons.push("room_action_history_truncated")
    if (history.actions.some((action) => action.state !== "completed")) followUpReasons.push("room_action_not_completed")
    if (screenshotRecord.status !== "captured") followUpReasons.push("room_screenshot_unavailable")
    return {
      scenarioId: definition.id,
      status: followUpReasons.length ? "required_user_action" : "observed",
      ...(followUpReasons.length ? { reason: followUpReasons[0] } : {}),
      promptId,
      providerTurnLifecycle: lifecycle,
      roomActionHistoryComplete: history.complete,
      actionAttribution: "selected_agent_and_sequence_window_only",
      roomActions: history.actions,
      screenshot: screenshotRecord,
      evidenceStillRequired: [...definition.evidence],
    }
  } finally {
    try {
      const detached = await client.send(requests.detachFromSessionRequest(attachment.id))
      cleanup = detached?.SessionDetached ? "detached" : "detach_unconfirmed"
    } catch {
      cleanup = "detach_unconfirmed"
    }
    input.recordCleanup?.({ scenarioId: definition.id, attachmentId: attachment.id, status: cleanup })
  }
}

function buildScenarioPrompt(scenarioId, task) {
  return [
    "This is a Chariox office-work scenario. Use the selected provider through the normal kernel path and Chariox runtime tools.",
    "Perform only the specific actions stated in the operator task. Do not reveal credentials or put secret values in responses, files, or artifacts.",
    "If a required credential, service consent, or user decision is missing, stop and report the exact operator action needed. Do not retry an uncertain external mutation.",
    `Scenario: ${scenarioId}`,
    "Operator task:",
    task.trim(),
  ].join("\n")
}

async function waitForPrompt(client, requests, sessionId, agentId, promptId, timeoutMs, pollMs, injectedSleep) {
  const sleep = injectedSleep ?? ((ms) => new Promise((resolve) => setTimeout(resolve, ms)))
  const deadline = Date.now() + timeoutMs
  while (Date.now() <= deadline) {
    const response = await client.send(requests.getSessionHistoryOutlineRequest(sessionId, [agentId], 10))
    const outline = response?.SessionHistoryOutline
    const turns = outline?.agents?.find((item) => item.agent_id === agentId)?.turns ?? []
    const turn = turns.find((item) => item.prompt_id === promptId)
    if (["completed", "failed", "cancelled"].includes(turn?.lifecycle)) return turn.lifecycle
    await sleep(Math.min(pollMs, Math.max(1, deadline - Date.now())))
  }
  return "open"
}

async function latestActionSequence(client, requests, sessionId) {
  const response = await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, null, 1))
  const actions = response?.RoomEnvironmentActionHistoryListed?.page?.actions
  if (!Array.isArray(actions)) throw new Error("invalid room action history")
  return Math.max(0, ...actions.map((action) => Number(action.sequence)).filter(Number.isSafeInteger))
}

async function actionHistorySince(client, requests, sessionId, agentId, baseline) {
  let before = null
  let complete = true
  const actions = []
  for (let pageIndex = 0; pageIndex < MAX_HISTORY_PAGES; pageIndex += 1) {
    const response = await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, before, HISTORY_PAGE_SIZE))
    const page = response?.RoomEnvironmentActionHistoryListed?.page
    if (!Array.isArray(page?.actions)) throw new Error("invalid room action history")
    actions.push(...page.actions.filter((action) => action.sequence > baseline))
    if (page.next_before_sequence == null) break
    const oldest = Math.min(...page.actions.map((action) => action.sequence).filter(Number.isSafeInteger))
    if (oldest <= baseline) break
    before = page.next_before_sequence
    if (pageIndex === MAX_HISTORY_PAGES - 1) complete = false
  }
  const projected = actions
    .filter((action) => action.actor_id === `agent:${agentId}`)
    .sort((left, right) => left.sequence - right.sequence)
    .map(projectAction)
    .filter(Boolean)
  return { complete, actions: projected }
}

function projectAction(action) {
  if (!Number.isSafeInteger(action.sequence) || typeof action.action_id !== "string" || !safeId.test(action.action_id)
    || !["browser", "computer"].includes(action.mode) || !safeActionKind.test(String(action.kind ?? ""))) return null
  if (!["queued", "running", "completed", "failed", "cancelled"].includes(action.state)) return null
  const targets = Array.isArray(action.targets) ? action.targets.map((target) => target?.kind)
    .filter((kind) => ["desktop", "browser_tab"].includes(kind)) : []
  return {
    actionId: action.action_id,
    sequence: action.sequence,
    mode: action.mode,
    kind: action.kind,
    state: action.state,
    submittedAtMs: safeTimestamp(action.submitted_at_ms),
    startedAtMs: safeTimestamp(action.started_at_ms),
    finishedAtMs: safeTimestamp(action.finished_at_ms),
    targetKinds: [...new Set(targets)],
  }
}

function safeTimestamp(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : null
}

function isSafeKernelUrl(value) {
  if (typeof value !== "string") return false
  try {
    const endpoint = new URL(value)
    return ["ws:", "wss:"].includes(endpoint.protocol) && !endpoint.username && !endpoint.password
      && endpoint.search === "" && endpoint.hash === "" && Boolean(endpoint.hostname)
  } catch {
    return false
  }
}

async function captureScreenshot(client, requests, sessionId, attachmentId) {
  try {
    const response = await client.send(requests.captureRoomEnvironmentScreenshotRequest(sessionId, attachmentId))
    const artifact = response?.RoomEnvironmentScreenshotCaptured?.artifact
    if (!safeId.test(String(artifact?.artifact_id ?? "")) || artifact?.media_type !== "image/png"
      || !Number.isSafeInteger(artifact.size_bytes)
      || artifact.size_bytes <= PNG_SIGNATURE.length || artifact.size_bytes > MAX_SCREENSHOT_BYTES
      || !/^[a-f0-9]{64}$/.test(artifact.sha256 ?? "")) return null
    const chunks = []
    let offset = 0
    while (offset < artifact.size_bytes) {
      const chunkResponse = await client.send(requests.readRoomEnvironmentScreenshotChunkRequest(
        sessionId, attachmentId, artifact.artifact_id, offset, 128 * 1024,
      ))
      const chunk = chunkResponse?.RoomEnvironmentScreenshotChunk?.chunk
      if (chunk?.artifact_id !== artifact.artifact_id || chunk.offset !== offset
        || typeof chunk.data_base64 !== "string" || !/^[A-Za-z0-9+/]+={0,2}$/.test(chunk.data_base64)) return null
      const bytes = Buffer.from(chunk.data_base64, "base64")
      if (bytes.length === 0 || bytes.length > 128 * 1024 || offset + bytes.length > artifact.size_bytes) return null
      chunks.push(bytes)
      offset += bytes.length
      if (chunk.eof !== (offset === artifact.size_bytes)) return null
    }
    const bytes = Buffer.concat(chunks)
    const sha256 = createHash("sha256").update(bytes).digest("hex")
    if (bytes.length !== artifact.size_bytes || sha256 !== artifact.sha256
      || !bytes.subarray(0, PNG_SIGNATURE.length).equals(PNG_SIGNATURE) || bytes.length < 33
      || bytes.readUInt32BE(8) !== 13 || bytes.toString("ascii", 12, 16) !== "IHDR") return null
    const width = bytes.readUInt32BE(16)
    const height = bytes.readUInt32BE(20)
    if (width < 1 || height < 1 || width > 16_384 || height > 16_384 || width * height > 100_000_000) return null
    return { bytes, sha256, sizeBytes: bytes.length }
  } catch {
    return null
  }
}

function unavailable(scenarioIds, reason) {
  return {
    contract: ROOM_OFFICE_SCENARIO_SUITE_CONTRACT,
    source: "kernel-room-observation",
    status: "unavailable",
    acceptanceAssessment: "not_performed",
    scenarioIds,
    reason,
    requiredUserActions: [...requiredOperatorActions],
  }
}
