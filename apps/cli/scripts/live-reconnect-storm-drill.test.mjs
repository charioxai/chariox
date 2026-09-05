import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import test from "node:test"
import { promisify } from "node:util"
import { fileURLToPath } from "node:url"
import { mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"

const run = promisify(execFile)
const script = fileURLToPath(new URL("./live-reconnect-storm-drill.mjs", import.meta.url))

test("reconnect storm drill plans concurrent recovery and slow-subscriber pressure", async () => {
  const { stdout } = await run(process.execPath, [script, "--dry-run", "--output", "/tmp/reconnect-storm.json"])
  const plan = JSON.parse(stdout)
  assert.equal(plan.clientCount, 32)
  assert.equal(plan.cycles, 5)
  assert.equal(plan.slowEvents, 4_096)
  assert.equal(plan.release, true)
  assert.equal(path.isAbsolute(plan.cargoTargetDir), true)
  assert.equal(plan.buildProfile, "release")
  assert.equal(plan.kernelBinary, path.join(plan.cargoTargetDir, "release", "chariox-kernel"))
  assert.equal(plan.relayBinary, path.join(plan.cargoTargetDir, "release", "chariox-relay"))
})

test("reconnect storm drill defaults evidence outside the repository", async () => {
  const evidenceRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-reconnect-storm-evidence-"))
  try {
    const { stdout } = await run(process.execPath, [script, "--dry-run"], {
      env: {
        ...process.env,
        CHARIOX_RECONNECT_STORM_EVIDENCE_ROOT: evidenceRoot,
      },
    })
    const plan = JSON.parse(stdout)
    assert.equal(path.dirname(path.dirname(plan.output)), evidenceRoot)
    assert.equal(path.basename(plan.output), "report.json")
  } finally {
    await rm(evidenceRoot, { recursive: true, force: true })
  }
})

test("reconnect storm drill rejects repository-owned evidence paths", async () => {
  await assert.rejects(
    run(process.execPath, [script, "--dry-run", "--output", path.join(path.dirname(script), "report.json")]),
    /evidence must stay outside repositories/,
  )
})

test("reconnect storm drill requires separate slow and healthy viewers", async () => {
  await assert.rejects(
    run(process.execPath, [script, "--dry-run", "--clients", "1", "--output", "/tmp/reconnect-storm.json"]),
    /--clients must be at least 2/,
  )
})

test("reconnect storm drill requires isolated slow-lane closure", async () => {
  const source = await import("node:fs/promises").then(({ readFile }) => readFile(script, "utf8"))
  assert.match(source, /slow_subscription_close_count <= pressureBaselineHealth\.backpressure\.slow_subscription_close_count/)
  assert.match(source, /seen\.slice\(1\)\.every/)
  assert.match(source, /submitPromptRequest/)
  assert.match(source, /provider prompt during slow-subscriber pressure/)
  assert.match(source, /provider output from dev-stub during slow-client pressure/)
  assert.match(source, /provider turn completion during slow-subscriber pressure/)
  assert.match(source, /dev-stub turn did not complete during slow-client pressure/)
  assert.match(source, /completePromptRequest/)
  assert.doesNotMatch(source, /native: \{ nativeTui: true \}/)
  assert.match(source, /record\.kind === "provider_output"/)
  assert.match(source, /kernel control during slow-subscriber pressure/)
  assert.match(source, /withDeadline/)
  assert.match(source, /slowEventsSubmittedAtHealthyProbe/)
  assert.match(source, /slowEventsSubmittedAtHealthyCompletion/)
  assert.match(source, /slowSubscriptionActiveThroughoutHealthyProbe/)
  assert.match(source, /subscription_queue_max_depth >= pressureQueueDepth/)
  assert.match(source, /pressured_subscription_count >= 1/)
  assert.match(source, /slow subscription closed before healthy work completed/)
  assert.match(source, /slow flood did not advance during the healthy probe/)
  assert.match(source, /await healthyProbeStarted/)
  assert.match(source, /slow flood progress during the healthy probe/)
  assert.match(source, /restartKernelEventStream/)
  assert.match(source, /resumeCounts\.every/)
  assert.match(source, /event streams to resume/)
  assert.match(source, /contexts\[index\]\.attachmentId/)
  assert.match(source, /independent attachment cursors/)
  assert.match(source, /peakKernelRssMb <= 1_024/)
  assert.match(source, /\["SIGINT", "SIGTERM"\]/)
  assert.match(source, /appendNativeProviderOutputBatchRequest/)
  assert.match(source, /CHARIOX_RELAY_OUTGOING_QUEUE_CAPACITY: "64"/)
  assert.match(source, /terminateOwnedTree/)
  assert.match(source, /requireExecutable\(kernelBinary/)
  assert.match(source, /requireExecutable\(relayBinary/)
})
