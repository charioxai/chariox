import assert from "node:assert/strict"

import {
  assertNoDuplicateSessionAgents,
  createPublicRequestLedger,
  durableAgentFingerprint,
  durableSessionFingerprint,
} from "./staged-kernel-restart-persistence-drill-helpers.mjs"

export const ATTACHMENT_RECOVERY_PUBLIC_REQUEST_LABELS = Object.freeze([
  "first daemon health",
  "initial session list",
  "session create",
  "attachment A attach",
  "attachment A state",
  "attachment B attach",
  "two-attachment authoritative state",
  "sibling pump while both attached",
  "attachment A detach",
  "controller recovery authoritative state",
  "controller recovery replacement attach",
  "post-recovery authoritative state",
  "post-recovery session list",
  "sibling pump after replacement",
  "replacement attachment detach",
  "sibling attachment detach",
  "final authoritative state",
])

export const ATTACHMENT_RECOVERY_PUBLIC_REQUEST_COUNT = ATTACHMENT_RECOVERY_PUBLIC_REQUEST_LABELS.length

export function createAttachmentRecoveryLedger() {
  const ledger = createPublicRequestLedger(ATTACHMENT_RECOVERY_PUBLIC_REQUEST_COUNT)
  const recordedLabels = []
  return {
    record(label) {
      const expected = ATTACHMENT_RECOVERY_PUBLIC_REQUEST_LABELS[recordedLabels.length]
      if (label !== expected) {
        throw new Error(
          `attachment recovery public request order mismatch: expected ${expected ?? "<complete>"}, got ${label}`,
        )
      }
      recordedLabels.push(label)
      return ledger.record(label)
    },
    get count() {
      return ledger.count
    },
    get labels() {
      return [...recordedLabels]
    },
    assertComplete() {
      ledger.assertComplete()
      assert.deepEqual(recordedLabels, ATTACHMENT_RECOVERY_PUBLIC_REQUEST_LABELS)
      return ledger.count
    },
  }
}

export function assertNoPromptReplay(session) {
  assert.equal(session?.active_provider_run_id ?? null, null, "attachment recovery must not have a provider run")
  assert.equal(session?.active_prompt ?? null, null, "attachment recovery must not have an active prompt")
  const promptStates = session?.prompt_states && typeof session.prompt_states === "object"
    ? Object.values(session.prompt_states)
    : []
  assert.equal(
    promptStates.some((state) => state?.active_prompt != null),
    false,
    "attachment recovery must not replay a prompt",
  )
}

export function assertAttachmentIds(session, expectedSessionId, expectedAttachmentIds) {
  assert.equal(session?.id, expectedSessionId, "attachment recovery returned a different session")
  const actual = Array.isArray(session?.attachment_ids) ? [...session.attachment_ids].sort() : null
  const expected = [...expectedAttachmentIds].sort()
  assert.deepEqual(actual, expected, `expected session attachments ${expected.join(", ") || "<none>"}`)
  assert.equal(new Set(actual ?? []).size, actual?.length ?? 0, "session contains duplicate attachment identities")
  return true
}

export function assertUnchangedDurableState({ session, expectedSession, expectedAgent }) {
  assertNoPromptReplay(session)
  assert.deepEqual(
    durableSessionFingerprint(session),
    expectedSession,
    `session ${expectedSession?.id ?? "<missing>"} changed during attachment recovery`,
  )
  assertNoDuplicateSessionAgents(session, expectedAgent?.id)
  const agents = Array.isArray(session?.agents) ? session.agents : []
  const agent = agents.find((candidate) => candidate?.id === expectedAgent?.id)
  assert.ok(agent, `agent ${expectedAgent?.id ?? "<missing>"} was lost during attachment recovery`)
  assert.deepEqual(durableAgentFingerprint(agent), expectedAgent, "agent identity changed during attachment recovery")
  return agent
}

export function assertAttachmentRecoveryState({
  session,
  expectedSessionId,
  expectedAttachmentIds,
  expectedSession,
  expectedAgent,
}) {
  assertAttachmentIds(session, expectedSessionId, expectedAttachmentIds)
  return assertUnchangedDurableState({ session, expectedSession, expectedAgent })
}

export function assertStaleAttachmentRejected(error) {
  if (error?.code !== "attachment_not_in_session") {
    throw new Error(
      `stale attachment must be rejected as attachment_not_in_session, got ${error?.code ?? "<missing>"}: ${error?.message ?? error}`,
    )
  }
  return true
}

export function assertNoSubmitPromptRequest(request, label = "outgoing request") {
  const keys = request && typeof request === "object" ? Object.keys(request) : []
  const forbidden = keys.filter((key) => key === "SubmitPrompt" || key === "SubmitPrompts")
  if (forbidden.length > 0) {
    throw new Error(`${label} must not send ${forbidden.join(", ")}`)
  }
  return true
}
