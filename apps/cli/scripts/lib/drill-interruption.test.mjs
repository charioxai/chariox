import assert from "node:assert/strict"
import { EventEmitter } from "node:events"
import { spawn } from "node:child_process"
import test from "node:test"
import { createDrillInterruption } from "./drill-interruption.mjs"

for (const signal of ["SIGINT", "SIGTERM"]) {
  test(`${signal} stops polling and cleans once, repeated signals do not abort cleanup`, async () => {
    const signals = new EventEmitter()
    const guard = createDrillInterruption(signals)
    const events = []
    await guard.run(async () => {
      signals.emit(signal)
      await guard.sleep(1)
      events.push("unexpected-work")
    }, async () => {
      events.push("cleanup")
      signals.emit("SIGINT")
      signals.emit("SIGTERM")
      guard.check()
      await guard.sleep(1)
      events.push("cleaned")
    }, (error) => events.push(error.message))
    assert.deepEqual(events, [`drill interrupted by ${signal}`, "cleanup", "cleaned"])
    assert.equal(signals.listenerCount("SIGINT"), 0)
    assert.equal(signals.listenerCount("SIGTERM"), 0)
  })
}

test("in-flight provisioning settles before cleanup, and no next request starts", async () => {
  const signals = new EventEmitter()
  const guard = createDrillInterruption(signals)
  const events = []
  const client = guard.guardClient({ async send() {
    events.push("start")
    signals.emit("SIGINT")
    await new Promise((resolve) => setTimeout(resolve, 1))
    events.push("settled")
    return "created-resource"
  } })
  let resource
  await guard.run(async () => {
    resource = await client.send()
    await client.send()
  }, async () => {
    assert.equal(resource, "created-resource")
    events.push("cleanup")
  }, (error) => events.push(error.message))
  assert.deepEqual(events, ["start", "settled", "drill interrupted by SIGINT", "cleanup"])
})

test("a signal during a finishing operation cannot publish success", async () => {
  const signals = new EventEmitter()
  const guard = createDrillInterruption(signals)
  let failure
  let cleaned = 0
  await guard.run(async () => { signals.emit("SIGTERM") }, async () => { cleaned++ }, (error) => { failure = error })
  assert.equal(failure?.message, "drill interrupted by SIGTERM")
  assert.equal(cleaned, 1)
})

test("ordinary failure cleans once and cleanup failure still restores signal handlers", async () => {
  const signals = new EventEmitter()
  const guard = createDrillInterruption(signals)
  let original
  let cleaned = 0
  await assert.rejects(guard.run(async () => { throw new Error("original") }, async () => {
    cleaned++
    throw new Error("cleanup-failure")
  }, (error) => { original = error.message }), /cleanup-failure/)
  assert.equal(original, "original")
  assert.equal(cleaned, 1)
  assert.equal(signals.listenerCount("SIGINT"), 0)
})

for (const signal of ["SIGINT", "SIGTERM"]) {
  test(`real process handles ${signal} and repeated signals through cleanup`, async () => {
    const moduleUrl = new URL("./drill-interruption.mjs", import.meta.url).href
    const child = spawn(process.execPath, ["--input-type=module", "-e", `
      import { createDrillInterruption } from ${JSON.stringify(moduleUrl)};
      const guard = createDrillInterruption();
      await guard.run(async () => {
        process.send("ready");
        await guard.sleep(30000);
        process.send("unexpected-work");
      }, async () => {
        process.send("cleanup");
        await guard.sleep(50);
        process.send("cleaned");
      }, () => { process.exitCode = 1; });
      process.disconnect();
    `], { stdio: ["ignore", "ignore", "pipe", "ipc"] })
    const messages = []
    let stderr = ""
    child.stderr.on("data", (data) => { stderr += data })
    child.on("message", (message) => {
      messages.push(message)
      if (message === "ready") child.kill(signal)
      if (message === "cleanup") { child.kill("SIGINT"); child.kill("SIGTERM") }
    })
    const watchdog = setTimeout(() => child.kill("SIGKILL"), 3000)
    try {
      const result = await new Promise((resolve, reject) => {
        child.once("error", reject)
        child.once("exit", (code, signal) => resolve({ code, signal }))
      })
      assert.equal(stderr, "")
      assert.deepEqual(result, { code: 1, signal: null })
      assert.deepEqual(messages, ["ready", "cleanup", "cleaned"])
    } finally {
      clearTimeout(watchdog)
      if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL")
    }
  })
}
