import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

import {
  assertAttachmentRecoveryState,
  assertNoSubmitPromptRequest,
  assertStaleAttachmentRejected,
  ATTACHMENT_RECOVERY_PUBLIC_REQUEST_COUNT,
  ATTACHMENT_RECOVERY_PUBLIC_REQUEST_LABELS,
  createAttachmentRecoveryLedger,
} from "./candidate-kernel-attachment-recovery-helpers.mjs"
import {
  assertNoDuplicateSessionAgents,
  createPublicRequestLedger,
  durableAgentFingerprint,
  durableSessionFingerprint,
} from "./staged-kernel-restart-persistence-drill-helpers.mjs"

function agentFixture(overrides = {}) {
  return {
    id: "agent-1",
    agent_ref: "agent-ref-1",
    session_id: "session-1",
    provider: "none",
    model: null,
    alias: null,
    worktree_id: "worktree-1",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 100,
    ...overrides,
  }
}

function sessionFixture(overrides = {}) {
  const agent = agentFixture(overrides.agent)
  return {
    id: "session-1",
    project_id: "project-1",
    alias: "candidate-kernel-attachment-recovery",
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    host_machine_id: "machine-1",
    host_daemon_id: "daemon-1",
    created_at_ms: 100,
    status: "Active",
    agent_defaults: null,
    focused_agent_id: null,
    max_agents: 1,
    config_state: { version: 0, values: {} },
    workspace_live_sync_mode: null,
    attachment_ids: [],
    agents: [agent],
    ...overrides,
  }
}

test("attachment recovery ledger requires every real successful public request", async () => {
  const expected = [
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
  ]
  assert.deepEqual(ATTACHMENT_RECOVERY_PUBLIC_REQUEST_LABELS, expected)
  assert.equal(ATTACHMENT_RECOVERY_PUBLIC_REQUEST_COUNT, expected.length)

  const complete = createAttachmentRecoveryLedger()
  for (const label of expected) complete.record(label)
  assert.equal(complete.assertComplete(), expected.length)

  const missingReplacementState = createPublicRequestLedger(ATTACHMENT_RECOVERY_PUBLIC_REQUEST_COUNT)
  for (const label of expected.slice(0, -1)) {
    missingReplacementState.record(label)
  }
  assert.throws(
    () => missingReplacementState.assertComplete(),
    /exactly 17 public requests, got 16.*sibling attachment detach/,
  )

  const source = await readFile(new URL("./candidate-kernel-attachment-recovery-check.mjs", import.meta.url), "utf8")
  for (const label of expected) {
    const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
    assert.match(source, new RegExp(`ledger\\.record\\(\\"${escaped}\\"\\)`), `runnable check must count ${label}`)
  }
})

test("runnable check wires the production recovery controller to stale rejection", async () => {
  const source = await readFile(new URL("./candidate-kernel-attachment-recovery-check.mjs", import.meta.url), "utf8")
  assert.match(source, /createKernelRestartRecoveryController/)
  assert.match(source, /recoveryDisconnected = true/)
  assert.match(source, /recoveryController\.recover\(\)/)
  assert.match(source, /controller recovery authoritative state/)
  assert.match(source, /controller recovery replacement attach/)
  assert.doesNotMatch(source, /recoverAttachmentThroughControllerPath/)
})

test("outgoing request boundary rejects prompt commands", () => {
  assert.equal(assertNoSubmitPromptRequest({ PumpTerminalOutput: { session_id: "s", attachment_id: "a" } }), true)
  assert.throws(
    () => assertNoSubmitPromptRequest({ SubmitPrompt: { session_id: "s", prompt: "not run" } }, "candidate request"),
    /candidate request must not send SubmitPrompt/,
  )
  assert.throws(
    () => assertNoSubmitPromptRequest({ SubmitPrompts: { session_id: "s", prompts: [] } }),
    /outgoing request must not send SubmitPrompts/,
  )
})

test("stale attachment rejection is exact and does not accept a generic error", () => {
  assert.equal(assertStaleAttachmentRejected({ code: "attachment_not_in_session" }), true)
  assert.throws(
    () => assertStaleAttachmentRejected({ code: "attachment_not_found", message: "missing" }),
    /attachment_not_in_session/,
  )
})

test("sibling and replacement attachments preserve authoritative session and agent identity", () => {
  const session = sessionFixture({ attachment_ids: ["attachment-sibling", "attachment-replacement"] })
  const agent = session.agents[0]
  const expectedSession = durableSessionFingerprint(session)
  const expectedAgent = durableAgentFingerprint(agent)

  assertAttachmentRecoveryState({
    session,
    expectedSessionId: "session-1",
    expectedAttachmentIds: ["attachment-sibling", "attachment-replacement"],
    expectedSession,
    expectedAgent,
  })
  assert.throws(
    () => assertAttachmentRecoveryState({
      session: { ...session, id: "lost-session" },
      expectedSessionId: "session-1",
      expectedAttachmentIds: ["attachment-sibling", "attachment-replacement"],
      expectedSession,
      expectedAgent,
    }),
    /different session/,
  )
  assert.throws(
    () => assertNoDuplicateSessionAgents({ ...session, agents: [...session.agents, agent] }, "agent-1"),
    /duplicate agent identities/,
  )
})
