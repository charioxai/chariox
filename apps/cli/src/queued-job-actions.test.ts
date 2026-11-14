import assert from "node:assert/strict"
import test from "node:test"
import { performQueuedJobAction } from "./queued-job-actions.js"

test("OF-22 VM and Stop actions address the current Room and opaque process id", async () => {
  const sent: unknown[] = []
  const client = { send: async <T>(request: unknown): Promise<T> => { sent.push(request); return { SessionProcessRemoteStarted: {}, SessionProcessesStopped: {} } as T } }
  const job = { processId: "registered-job", command: "cargo build", wait: "waiting for memory", detail: null, remoteAvailable: true }
  await performQueuedJobAction(client, "room-now", job, "remote")
  await performQueuedJobAction(client, "room-now", job, "stop")
  assert.deepEqual(sent, [
    { RunSessionProcessRemotely: { session_id: "room-now", process_id: "registered-job" } },
    { StopSessionProcesses: { session_id: "room-now", target: { kind: "process", process_id: "registered-job" } } },
  ])
  await assert.rejects(performQueuedJobAction(client, "room-now", { ...job, remoteAvailable: false }, "remote"), /cannot run remotely/)
  assert.equal(sent.length, 2)
  await assert.rejects(performQueuedJobAction({ send: async <T>() => ({ Error: {} }) as T }, "room-now", job, "remote"), /did not confirm/)
})
