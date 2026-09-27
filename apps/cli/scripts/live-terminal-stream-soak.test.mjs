import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import test from "node:test"
import { promisify } from "node:util"
import { fileURLToPath } from "node:url"

const run = promisify(execFile)
const script = fileURLToPath(new URL("./live-terminal-stream-soak.mjs", import.meta.url))

test("stream soak plans 30 minutes at one MiB per second", async () => {
  const { stdout } = await run(process.execPath, [script, "--dry-run", "--build-profile", "debug", "--output", "/tmp/stream-soak.json"], {
    env: { ...process.env, CARGO_TARGET_DIR: "/tmp/chariox-shared-target" },
  })
  const plan = JSON.parse(stdout)
  assert.equal(plan.durationSeconds, 1_800)
  assert.equal(plan.bytesPerSecond, 1_048_576)
  assert.equal(plan.totalBytes, 1_887_436_800)
  assert.equal(plan.maxRssMb, 1_024)
  assert.equal(plan.maxCpuPercent, 150)
  assert.equal(plan.buildProfile, "debug")
  assert.equal(plan.cargoTargetDir, "/tmp/chariox-shared-target")
  assert.equal(plan.release, false)
})

test("stream soak enforces kernel resource budgets", async () => {
  const source = await import("node:fs/promises").then(({ readFile }) => readFile(script, "utf8"))
  assert.match(source, /peakRssMb <= maxRssMb/)
  assert.match(source, /cpuP95Percent <= maxCpuPercent/)
})
