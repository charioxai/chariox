// MP-08 / MP-10 / MP-11: native exit projects only its kernel-owned outcome.
import assert from "node:assert/strict"
import test from "node:test"
import type { LocalIpcClient } from "../ipc.js"
import { waitForClaudeRemoteRenderedRunExit } from "./claude-provider-run.js"

const context = { sessionId: "session", agentId: "agent" }
function fixture(runId: string, status: string, category = "runtime_failure", lease?: string): LocalIpcClient {
  return { send: async (request: Record<string, unknown>) => "GetProviderRun" in request
    ? { ProviderRun: { provider_run: { id: "current-run", state: "Ended" } } }
    : { SessionState: { session: { agents: [{ id: "agent", remote_execution: lease ? { leased_agent_id: lease } : null }] }, agent_activity: { agent: { last_completed_turn: {
      provider_run_id: runId, settlement_status: status,
      provider_termination: { category, reason: "Claude stopped responding to cancel; the run was ended." },
    } } } } },
  } as unknown as LocalIpcClient
}

test("native rendered exit reports the exact run's definitive Error", async () => {
  await assert.rejects(waitForClaudeRemoteRenderedRunExit(fixture("current-run", "failed"), "current-run", context), /stopped responding to cancel/)
})

test("native rendered exit does not attribute another run's Error to this run", async () => {
  await waitForClaudeRemoteRenderedRunExit(fixture("foreign-run", "failed"), "current-run", context)
  await waitForClaudeRemoteRenderedRunExit(fixture("current-run", "cancelled"), "current-run", context)
})

test("normal native exit does not surface an earlier provider turn failure", async () => {
  await waitForClaudeRemoteRenderedRunExit(fixture("current-run", "failed", "explicit_provider_error"), "current-run", context)
})

// MP-08 / MP-10 / MP-11: home projection preserves the worker run identity.
test("leased native exit reports the exact lease and worker run Error", async () => {
  await assert.rejects(waitForClaudeRemoteRenderedRunExit(fixture("worker-run", "failed", "runtime_failure", "lease-current"), "leased:lease-current:worker-run", context), /stopped responding to cancel/)
})

test("leased native exit rejects another lease's otherwise identical worker run", async () => {
  await waitForClaudeRemoteRenderedRunExit(fixture("worker-run", "failed", "runtime_failure", "lease-other"), "leased:lease-current:worker-run", context)
})
