import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import test from "node:test"
import { promisify } from "node:util"
import { fileURLToPath } from "node:url"

const run = promisify(execFile)
const script = fileURLToPath(new URL("./live-reconnect-storm-drill.mjs", import.meta.url))

test("reconnect storm drill plans concurrent recovery and slow-subscriber pressure", async () => {
  const { stdout } = await run(process.execPath, [script, "--dry-run", "--output", "/tmp/reconnect-storm.json"])
  const plan = JSON.parse(stdout)
  assert.equal(plan.clientCount, 32)
  assert.equal(plan.cycles, 5)
  assert.equal(plan.slowEvents, 4_096)
  assert.equal(plan.release, true)
})

test("reconnect storm drill requires isolated slow-lane closure", async () => {
  const source = await import("node:fs/promises").then(({ readFile }) => readFile(script, "utf8"))
  assert.match(source, /slow_subscription_close_count >= 1/)
  assert.match(source, /seen\.slice\(1\)\.every/)
  assert.match(source, /restartKernelEventStream/)
  assert.match(source, /resumeCounts\.every/)
  assert.match(source, /event streams to resume/)
  assert.match(source, /contexts\[index\]\.attachmentId/)
  assert.match(source, /independent attachment cursors/)
  assert.match(source, /peakKernelRssMb <= 1_024/)
  assert.match(source, /\["SIGINT", "SIGTERM"\]/)
  assert.match(source, /appendNativeProviderOutputBatchRequest/)
  assert.match(source, /CHARIOX_RELAY_OUTGOING_QUEUE_CAPACITY: "32"/)
  assert.match(source, /terminateOwnedTree/)
})
