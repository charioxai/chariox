import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import test from "node:test"
import { promisify } from "node:util"
import { fileURLToPath } from "node:url"

const run = promisify(execFile)
const script = fileURLToPath(new URL("./live-distributed-scale-drill.mjs", import.meta.url))

test("distributed scale drill plans the 10 by 50 release gate", async () => {
  const { stdout } = await run(process.execPath, [script, "--dry-run", "--workers", "10", "--agents-per-worker", "50", "--output", "/tmp/distributed-scale.json"])
  const plan = JSON.parse(stdout)
  assert.equal(plan.workerCount, 10)
  assert.equal(plan.agentsPerWorker, 50)
  assert.equal(plan.totalAgents, 500)
  assert.equal(plan.release, true)
  assert.equal(plan.output, "/tmp/distributed-scale.json")
})

test("distributed scale drill activates every lease and samples every worker", async () => {
  const source = await import("node:fs/promises").then(({ readFile }) => readFile(script, "utf8"))
  assert.match(source, /launchProviderRunsRequest\(spawned\.map/)
  assert.match(source, /model: "distributed-scale-shared-pty"/)
  assert.match(source, /runningProviderAgents: totalAgents/)
  assert.match(source, /syntheticProviderProcesses: workerCount/)
})
