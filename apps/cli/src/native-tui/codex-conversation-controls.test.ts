import assert from "node:assert/strict"
import test from "node:test"
import type { RuntimeProviderRun } from "../cli-types.js"
import type { LocalIpcClient } from "../ipc.js"
import { mapCodexNativeCompaction } from "./codex-conversation-controls.js"

const boundRun = {
  id: "run", session_id: "session", agent_instance_id: "agent", provider: "codex",
  state: "Running", structured_endpoint: "ws://fixture", provider_session_id: "old-thread",
} as RuntimeProviderRun
const message = { id: "compact", method: "thread/compact/start", params: { threadId: "display-thread" } }
function options(send: (request: unknown) => Promise<unknown>) {
  return {
    client: { send } as LocalIpcClient, sessionId: "session", agentId: "agent",
    bindState: { run: boundRun, promise: Promise.resolve(boundRun) },
  }
}

test("MP-08 MP-10 native compaction refreshes the managed conversation for every request", async () => {
  let calls = 0
  const opts = options(async (request) => {
    assert.deepEqual(request, { GetProviderRun: { provider_run_id: "run" } })
    return { ProviderRun: { provider_run: { ...boundRun, provider_session_id: `thread-${++calls}` } } }
  })
  for (const threadId of ["thread-1", "thread-2"]) {
    assert.deepEqual(await mapCodexNativeCompaction(message, opts), { ...message, params: { threadId } })
  }
  assert.equal(opts.bindState.run.provider_session_id, "old-thread", "launch metadata must not become an identity cache")
})

test("MP-08 MP-10 native compaction rejects ended or foreign run projections", async () => {
  for (const change of [
    { id: "replacement" }, { session_id: "other" }, { agent_instance_id: "other" },
    { provider: "other" }, { state: "Ended" }, { structured_endpoint: "ws://other" },
  ]) {
    await assert.rejects(mapCodexNativeCompaction(message, options(async () => ({ ProviderRun: { provider_run: { ...boundRun, ...change } } }))),
      /changed or ended/)
  }
})

test("MP-08 MP-10 native compaction waits for asynchronous first-turn identity publication", async () => {
  let calls = 0
  const mapped = await mapCodexNativeCompaction(message, options(async () => ({ ProviderRun: { provider_run: {
    ...boundRun, provider_session_id: ++calls === 1 ? null : "managed-thread",
  } } })), 500)
  assert.equal(mapped.params?.threadId, "managed-thread")
  assert.equal(calls, 2)
})

test("MP-08 MP-10 native compaction bounds missing identity and stalled kernel reads", async () => {
  for (const send of [
    async () => ({ ProviderRun: { provider_run: { ...boundRun, provider_session_id: null } } }),
    async () => new Promise(() => {}),
  ]) await assert.rejects(mapCodexNativeCompaction(message, options(send), 20), /timed out.*managed Codex conversation/)
})
