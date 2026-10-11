// MP-08 / MP-10 / MP-11: generated hook processes supplement live provider drills.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { setTimeout as sleep } from "node:timers/promises"
import test from "node:test"

for (const cancel of [true, false]) {
  test(`MP-08 / MP-10 preloaded context awaits current hook ${cancel ? "cancellation" : "acknowledgement"}`, async () => {
    const root = await mkdtemp(join(tmpdir(), "integ2-hook-ack-"))
    const children = []
    const responses = join(root, "responses")
    const events = join(root, "events")
    try {
      await mkdir(responses)
      await writeFile(join(root, "context"), "Previous turn context".repeat(4000))
      const outputs = []
      for (let index = 0; index < 8; index++) {
        const child = spawn(process.execPath, [fileURLToPath(new URL("./hook-handler.mjs", import.meta.url))], {
          env: { ...process.env, CHARIOX_CLAUDE_NATIVE_EVENTS: events,
            CHARIOX_CLAUDE_NATIVE_CONTEXT: join(root, "context"),
            CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES: responses,
            CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK: String(index), CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNKS: "8" },
          stdio: ["pipe", "pipe", "ignore"],
        })
        children.push(child)
        outputs.push(new Promise((resolve, reject) => {
          let output = ""
          child.stdout.on("data", chunk => { output += chunk.toString() })
          child.on("error", reject)
          child.on("close", code => {
            try { assert.equal(code, 0); resolve(JSON.parse(output)) } catch (error) { reject(error) }
          })
        }))
        child.stdin.end(JSON.stringify({ hook_event_name: "UserPromptSubmit", prompt: "Read the public documentation" }))
      }
      let request
      const deadline = Date.now() + 3000
      while (Date.now() < deadline && !request) {
        try { request = JSON.parse((await readFile(events, "utf8")).split("\n")[0]) } catch {}
        if (!request) await sleep(5)
      }
      assert(request?.hook_context_request_id)
      await sleep(750)
      const exitedBeforeAcknowledgement = children[0].exitCode !== null
      await writeFile(join(responses, `${cancel ? request.hook_context_request_id : "stale-request"}.cancelled`), "")
      if (!cancel) await writeFile(join(responses, `${request.hook_context_request_id}.txt`), "Current approved context")
      const results = await Promise.all(outputs)
      assert.equal(exitedBeforeAcknowledgement, false, "preloaded context cannot bypass the pending bridge response")
      assert.equal(results[0].decision === "block", cancel)
      assert(results.slice(1).every(result => !result.decision), "context helpers cannot decide admission")
      if (cancel) assert(results.slice(1).every(result => result.hookSpecificOutput.additionalContext === ""), "declined turns cannot carry previous context through helpers")
      if (!cancel) assert.equal(results.map(result => result.hookSpecificOutput.additionalContext).join(""), "Current approved context")
    } finally {
      for (const child of children) {
        if (child.exitCode === null && child.signalCode === null) {
          assert(Number.isSafeInteger(child.pid) && child.pid > 1)
          child.kill("SIGTERM")
          await new Promise(resolve => child.once("close", resolve))
        }
      }
      await rm(root, { recursive: true, force: true })
    }
  })
}
