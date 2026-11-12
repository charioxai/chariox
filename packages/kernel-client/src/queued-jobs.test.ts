import assert from "node:assert/strict"
import test from "node:test"

import type { SessionProcess, SessionProcessState } from "./kernel-events.js"
import { queuedJobsForAgent, remoteJobLinesForAgent } from "./queued-jobs.js"

const GIB = 2 ** 30
const row = (pid: number, agent: string, state: SessionProcessState): SessionProcess => ({
  process_id: `process-${pid}`, owner: { kind: "agent", id: agent }, pid, command: "sh",
  resident_set_bytes: 1, cpu_millis: 0, disk_io_bytes: null, disk_written_bytes: null, deleted_open_bytes: 0, outlived: false, age_ms: 0, sampled_at_ms: 0, state,
})

test("OF-22/23 remote eligibility and estimates are kernel state, hidden without eligibility", () => {
  const process = row(1, "a", { kind: "held", reason: "memory", position: 2, need_bytes: GIB, free_bytes: 0, command_line: "build", since_ms: 1 })
  const estimate = { amount_micros: 40000, currency: "USD", kind: "total" as const, memory_mib: 16384, minutes: 30 }
  const [job] = queuedJobsForAgent([{ ...process, remote_available: true, estimate }], "a")
  assert.equal(job!.rate, "≈ $0.04")
  assert.match(job!.rateDetail!, /16 GB, about 30 min/)
  assert.equal(job!.remoteAvailable, true)
  assert.equal(queuedJobsForAgent([{ ...process, remote_available: false, estimate }], "a")[0]!.rate, undefined)
})

test("OF-52 progress uses the kernel's placement, rate, result and confirmed deletion", () => {
  const job = { job_id: "j", agent_id: "a", command: "build", state: "running" as const, placement: "remote" as const, started_at_ms: 1000, machine_label: "offload-1234", address: "2001:db8::1", class_id: "small", rate_per_minute: { amount_micros: 1200, currency: "USD" }, estimate_micros: 40000, deleted: false }
  assert.deepEqual(remoteJobLinesForAgent([job], "a", 253000), ["Running on offload-1234 · 2001:db8::1 · build · 4m 12s · small · $0.0012/min · ≈ $0.04"])
  assert.deepEqual(remoteJobLinesForAgent([{ ...job, deleted: true }], "a", 253000).at(-1), "Deleted offload-1234")
  assert.deepEqual(remoteJobLinesForAgent([job], "b", 253000), [])
})

test("an agent's waiting commands read as queued-job rows in line order", () => {
  const processes = [
    row(10, "a", { kind: "running" }),
    row(11, "a", { kind: "paused", reason: "memory", position: 1, need_bytes: 3 * GIB, free_bytes: GIB, command_line: "pnpm test", since_ms: 1 }),
    row(12, "a", { kind: "held", reason: "memory", position: 3, need_bytes: 6 * GIB, free_bytes: GIB, command_line: "cargo build --release", since_ms: 1 }),
    row(13, "b", { kind: "held", reason: "memory", position: 1, need_bytes: 9 * GIB, free_bytes: 2 * GIB, command_line: "make", since_ms: 1 }),
    row(14, "a", { kind: "held", reason: "disk", position: 1, need_bytes: 15 * GIB, free_bytes: 0, command_line: "docker build .", since_ms: 1 }),
    row(15, "a", { kind: "held", reason: "memory", position: 2, need_bytes: 6 * GIB, free_bytes: GIB, command_line: "", since_ms: 1 }),
  ]
  assert.deepEqual(queuedJobsForAgent(processes, "a").map(job => [job.command, job.wait]), [
    ["docker build .", "waiting for disk space, needs 15 GB"],
    ["sh", "waiting for memory, 2nd in line"],
    ["cargo build --release", "waiting for memory, 3rd in line"],
    ["pnpm test", "paused at the memory limit"],
  ])
  assert.deepEqual(queuedJobsForAgent(processes, "b").map(job => [job.wait, job.detail]), [
    ["waiting for memory, needs 9 GB", "Needs about 9 GB; 2 GB free"],
  ])
  assert.deepEqual(queuedJobsForAgent(processes, null), [])
})
