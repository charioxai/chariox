import assert from "node:assert/strict"
import test from "node:test"
import { createSessionProcessController, sessionProcessLines } from "./session-process-controller.js"
import type { SessionProcess } from "@chariox/kernel-client/kernel-events"
const row = (pid: number, sampled = Date.now()): SessionProcess => ({ process_id: `process-${pid}`, owner: {kind: "agent", id: "agent-a"}, pid, command: "python3", resident_set_bytes: 1048576, cpu_millis: 250, disk_io_bytes: null, disk_written_bytes: null, deleted_open_bytes: 0, outlived: false, age_ms: 12000, sampled_at_ms: sampled, state: {kind: "running"} })

test("process rows expose owners and resources and exclude stale samples", () => {
  const lines = sessionProcessLines([row(123, 10000), row(456, 1)], 50000)
  assert.match(lines.join("\n"), /agent:agent-a.*123.*python3.*1.0 MiB.*25.0%.*52s/)
  assert.doesNotMatch(lines.join("\n"), /456/)
})

// The kernel sends a quiet process once and again after 30 s, not every sample: the age is read from the sample time.
test("a quiet process stays listed and its age keeps growing between samples", () => {
  const sampled = 100_000
  const early = sessionProcessLines([row(123, sampled)], sampled + 1000).join("\n")
  const later = sessionProcessLines([row(123, sampled)], sampled + 31_000).join("\n")
  assert.match(early, /123.*13s/)
  assert.match(later, /123.*43s/)
})

test("the live process panel refreshes samples, scrolls, and stops refreshing on close", () => {
  let rows = [row(123), row(456)]
  let renderCount = 0
  const timers: Array<() => void> = []
  const panel = createSessionProcessController({ processes: () => rows, ownerLabel: row => row.owner.id, render: () => { renderCount++ }, schedule: callback => timers.push(callback), pageSize: () => 2 })
  panel.open()
  assert.match(panel.visibleLines().join("\n"), /123/)
  panel.handleKey({name: "down"}); panel.handleKey({name: "down"})
  assert.match(panel.visibleLines().join("\n"), /456/)
  rows = [row(789), row(800)]
  timers.shift()!()
  assert.match(panel.visibleLines().join("\n"), /800/)
  panel.close()
  const afterClose = renderCount
  timers.shift()!()
  assert.equal(renderCount, afterClose)
})
