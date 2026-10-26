import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { promisify } from "node:util"
import { fileURLToPath } from "node:url"
import test from "node:test"

const exec = promisify(execFile)
const script = fileURLToPath(new URL("./live-cloud-relay-drill.mjs", import.meta.url))

test("MP-08 / MP-10 / MP-11: clean-built live relay drill imports its runtime without starting services", async () => {
  const result = await exec(process.execPath, [script, "--check"], { timeout: 15_000 })
  assert.match(result.stdout, /runtime imports passed/)
  assert.doesNotMatch(result.stdout, /start-cloud|start-relay|kernel:stdout|token=/)
})
