import assert from "node:assert/strict"
import test from "node:test"

import {
  browserStateFixtureSidecarArgs,
  startBrowserStateFixtureSidecar,
} from "./browser-state-drill-fixture-sidecar.mjs"

const containerId = "a".repeat(64)
const imageId = `sha256:${"b".repeat(64)}`
const runId = "m20-docker-state-123-20260923T050000Z"

function harness({ replace = false, failCopy = false } = {}) {
  const calls = []
  let removed = false
  let copied = 0
  const docker = async (args) => {
    calls.push({ method: "docker", args })
    if (args[0] === "cp" && ++copied === 1 && failCopy) throw new Error("copy failed")
    if (args[0] === "rm") removed = true
    return { stdout: "" }
  }
  const dockerText = async (args, options = {}) => {
    calls.push({ method: "dockerText", args, options })
    if (args[0] === "image") return `${JSON.stringify(imageId)}\n`
    if (args[0] === "run") return `${containerId}\n`
    if (args.some(value => value.endsWith("/messages"))) return JSON.stringify([{ subject: "one" }])
    if (args.some(value => value.endsWith("/invalidate"))) return JSON.stringify({ count: 1 })
    if (args.some(value => value.endsWith("/close"))) return JSON.stringify({ closed: true })
    return ""
  }
  const runCommand = async (_command, args) => {
    calls.push({ method: "runCommand", args })
    if (args[0] === "container" && args[1] === "inspect") {
      if (removed) return { code: 1, stdout: "" }
      return {
        code: 0,
        stdout: JSON.stringify([{
          Id: replace ? "c".repeat(64) : containerId,
          Name: calls.find(({ args: candidate }) => candidate[0] === "run")?.args[4] &&
            `/${calls.find(({ args: candidate }) => candidate[0] === "run").args[4]}`,
          Image: imageId,
          Config: { Labels: { "io.chariox.drill-run": runId } },
        }]),
      }
    }
    return { code: 0, stdout: "" }
  }
  return { calls, docker, dockerText, runCommand }
}

test("rootless fixture arguments are bounded and do not publish a host port", () => {
  const args = browserStateFixtureSidecarArgs({ name: "m20-fixture-test", image: "pinned:image", runId })
  assert.ok(args.includes("--network") && args.includes("host"))
  assert.ok(args.includes("--memory") && args.includes("256m"))
  assert.ok(args.includes("--pids-limit") && args.includes("64"))
  assert.ok(args.includes("--cap-drop") && args.includes("ALL"))
  assert.ok(!args.includes("-p") && !args.includes("--publish"))
  assert.ok(args.includes(`io.chariox.drill-run=${runId}`))
})

test("sidecar control stays on container loopback and cleanup removes exact identity", async () => {
  const fake = harness()
  const fixture = await startBrowserStateFixtureSidecar({
    ...fake, image: "pinned:image", runId, port: 4321,
    account: "agent@chariox.test", password: "synthetic-password",
  })
  assert.equal(fixture.containerId, containerId)
  assert.deepEqual(await fixture.readMessages(), [{ subject: "one" }])
  assert.equal(await fixture.invalidateSessions(), 1)
  await fixture.close()
  await fixture.close()
  await fixture.cleanup()
  const submittedConfig = fake.calls.find(({ options }) => options?.stdin)?.options.stdin
  assert.equal(JSON.parse(submittedConfig).password, "synthetic-password")
  assert.ok(!fake.calls.some(({ args }) => args.some(value => value.includes("synthetic-password"))))
  assert.ok(fake.calls.some(({ args }) => args.includes("http://127.0.0.1:4322/messages")))
  assert.ok(fake.calls.some(({ args }) => args[0] === "rm" && args[2] === containerId))
  assert.equal(fake.calls.filter(({ args }) => args[0] === "cp").length, 3)
})

test("sidecar cleanup refuses a replaced container", async () => {
  const fake = harness({ replace: true })
  const fixture = await startBrowserStateFixtureSidecar({
    ...fake, image: "pinned:image", runId, port: 4321,
    account: "agent@chariox.test", password: "synthetic-password",
  })
  await assert.rejects(fixture.cleanup(), /identity changed/)
  assert.ok(!fake.calls.some(({ args }) => args[0] === "rm"))
})

test("sidecar startup failure removes only the created container", async () => {
  const fake = harness({ failCopy: true })
  await assert.rejects(startBrowserStateFixtureSidecar({
    ...fake, image: "pinned:image", runId, port: 4321,
    account: "agent@chariox.test", password: "synthetic-password",
  }), /copy failed/)
  assert.ok(fake.calls.some(({ args }) => args[0] === "rm" && args[2] === containerId))
})
