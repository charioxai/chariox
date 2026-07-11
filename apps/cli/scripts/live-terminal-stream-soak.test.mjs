import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import test from "node:test"
import { promisify } from "node:util"
import { fileURLToPath } from "node:url"

const run = promisify(execFile)
const script = fileURLToPath(new URL("./live-terminal-stream-soak.mjs", import.meta.url))

test("stream soak plans 30 minutes at one MiB per second", async () => {
  const { stdout } = await run(process.execPath, [script, "--dry-run", "--output", "/tmp/stream-soak.json"])
  const plan = JSON.parse(stdout)
  assert.equal(plan.durationSeconds, 1_800)
  assert.equal(plan.bytesPerSecond, 1_048_576)
  assert.equal(plan.totalBytes, 1_887_436_800)
  assert.equal(plan.release, true)
})
