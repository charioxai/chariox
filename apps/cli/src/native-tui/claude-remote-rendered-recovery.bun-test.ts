import assert from "node:assert/strict"
import test from "node:test"
import { setTimeout as sleep } from "node:timers/promises"

import { startClaudeRemoteRenderedPumpLoop } from "./claude-remote-rendered-io.js"
import { isSessionUnavailableError } from "../session-errors.js"

type PumpOutcome = "recovered" | "transient" | "fatal"

// RR-07: a remote native-TUI attachment carried through the gateway lapses after
// a >30s relay outage (the target sweeps it). The pump then polls a dead
// attachment id forever and the terminal silently freezes. The pump must re-attach
// on a definite attachment_not_found, adopt the new attachment id, and resume.
test("native remote pump re-attaches after attachment_not_found and resumes output", async () => {
  const attachment = { id: "att-1" }
  const delivered: unknown[] = []
  let reattachCalls = 0

  const client = {
    async send(request: Record<string, { attachment_id?: string }>) {
      const id = request?.PumpTerminalOutput?.attachment_id
      if (id === "att-2") return { TerminalOutput: { records: [{ marker: "after-recovery" }] } }
      // att-1 (the swept attachment) and every unknown id are refused.
      throw new Error(`attachment \`${String(id)}\` was not found`)
    },
  } as unknown as Parameters<typeof startClaudeRemoteRenderedPumpLoop>[0]

  const recover = async (error: unknown): Promise<PumpOutcome> => {
    if (!isSessionUnavailableError(error)) return "transient"
    reattachCalls += 1
    attachment.id = "att-2"
    return "recovered"
  }

  const pump = startClaudeRemoteRenderedPumpLoop(
    client,
    "sess-1",
    attachment,
    "run-1",
    "agent-1",
    recover,
    (records) => {
      if (Array.isArray(records) && records.length) delivered.push(...records)
    },
  )

  const start = Date.now()
  while (delivered.length === 0 && Date.now() - start < 4_000) await sleep(25)
  pump.stop()

  assert.equal(reattachCalls, 1, "re-attached exactly once on attachment_not_found")
  assert.equal(attachment.id, "att-2", "pump adopted the recovered attachment id")
  assert.ok(delivered.length >= 1, "terminal output resumed after recovery")
})

// A definite grant refusal / session-gone must stop the pump: no silent resurrection,
// the user's explicit detach or a revoked grant is honoured.
test("native remote pump stops when recovery is fatal", async () => {
  const attachment = { id: "att-1" }
  let sends = 0
  const client = {
    async send(_request: unknown) {
      sends += 1
      throw new Error("attachment `att-1` was not found")
    },
  } as unknown as Parameters<typeof startClaudeRemoteRenderedPumpLoop>[0]

  const recover = async (): Promise<PumpOutcome> => "fatal"

  const pump = startClaudeRemoteRenderedPumpLoop(client, "sess-1", attachment, "run-1", "agent-1", recover)
  await sleep(600)
  const sendsAtFatal = sends
  await sleep(600)
  pump.stop()
  assert.equal(sends, sendsAtFatal, "the pump stopped sending after a fatal recovery outcome")
})
