// MP-07/MP-08/MP-11: lock recovery crosses the actual target, SSH and driver boundaries.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { mkdir } from "node:fs/promises"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { runSshMachine } from "./transport.mjs"
import { sshTarget, availablePort } from "./ssh-target-fixture.mjs"

test("MP-07/MP-11 actual SSH and driver preserve a path-free interrupted-install classification", async t => {
  const { home } = await sshTarget(t)
  await mkdir(join(home, ".local/share/chariox/ssh-machines/.byom-one.lock"), { recursive: true, mode: 0o700 })
  const request = { action: "inspect", installId: "byom-one", port: await availablePort(), releaseDigest: `sha256:${"a".repeat(64)}` }
  await assert.rejects(runSshMachine("byom-local", request), error => {
    assert.equal(error.exitCode, 75)
    assert.equal(error.message.includes(home), false)
    return true
  })
  const child = spawn("node", [fileURLToPath(new URL("./driver.mjs", import.meta.url))], { stdio: ["pipe", "pipe", "pipe"] })
  let stdout = "", stderr = ""
  child.stdout.on("data", bytes => { stdout += bytes })
  child.stderr.on("data", bytes => { stderr += bytes })
  child.stdin.end(JSON.stringify({ host: "byom-local", request }))
  assert.equal(await new Promise(resolve => child.once("close", resolve)), 75)
  assert.equal(stdout, "")
  assert.equal(stderr.includes(home), false)
  assert.ok(stderr.length < 256)
})
