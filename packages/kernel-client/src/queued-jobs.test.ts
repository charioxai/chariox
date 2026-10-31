import assert from "node:assert/strict"
import test from "node:test"

import type { SessionProcess, SessionProcessState } from "./kernel-events.js"
import { queuedJobsForAgent } from "./queued-jobs.js"

const GIB = 2 ** 30
const row = (pid: number, agent: string, state: SessionProcessState): SessionProcess => ({
  process_id: `process-${pid}`, owner: { kind: "agent", id: agent }, pid, command: "sh",
  resident_set_bytes: 1, cpu_millis: 0, age_ms: 0, sampled_at_ms: 0, state,
})

test("an agent's waiting commands read as queued-job rows in line order", () => {
  const processes = [
    row(10, "a", { kind: "running" }),
    row(11, "a", { kind: "paused", reason: "memory", command_line: "pnpm test" }),
    row(12, "a", { kind: "held", reason: "memory", position: 3, need_bytes: 6 * GIB, free_bytes: GIB, command_line: "cargo build --release" }),
    row(13, "b", { kind: "held", reason: "memory", position: 1, need_bytes: 9 * GIB, free_bytes: 2 * GIB, command_line: "make" }),
    row(14, "a", { kind: "held", reason: "disk", position: 1, need_bytes: 15 * GIB, free_bytes: 0, command_line: "docker build ." }),
    row(15, "a", { kind: "held", reason: "memory", position: 2, need_bytes: 6 * GIB, free_bytes: GIB, command_line: "" }),
  ]
  assert.deepEqual(queuedJobsForAgent(processes, "a").map(job => [job.command, job.wait]), [
    ["sh", "waiting for memory, 2nd in line"],
    ["cargo build --release", "waiting for memory, 3rd in line"],
    ["docker build .", "waiting for disk space, needs 15 GB"],
    ["pnpm test", "paused at the memory limit"],
  ])
  assert.deepEqual(queuedJobsForAgent(processes, "b").map(job => [job.wait, job.detail]), [
    ["waiting for memory, needs 9 GB", "Needs about 9 GB; 2 GB free"],
  ])
  assert.deepEqual(queuedJobsForAgent(processes, null), [])
})
