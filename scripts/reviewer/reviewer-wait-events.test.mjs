// The replacement wait client against a stand-in for the CLI: the stdout contract, the fallback listing and the
// unreachable kernel. The real kernel and CLI are exercised by scripts/drills/notif-external-agent.mjs --scenario workflow-runs.
import assert from "node:assert/strict"
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { formatLine, kernelUnreachable, main } from "./reviewer-wait-events.mjs"

const SOURCE = { source_id: "run_finished_a", kind: "workflow_run", display_name: "reviewer run finished", name: "reviewer", workflow_id: "wf-1", session_id: "room-1" }
const PROMPT = "Handle GitHub pull_request.synchronize for charioxai/chariox #1279 at head 0e8b5e6fd8c3a1b2."
const at = Date.UTC(2026, 10, 6, 9, 17)
const notification = (run_id, status, extra = {}) => ({ type: "notification", at_ms: at, value: { source_id: SOURCE.source_id, output: { message: JSON.stringify({ status, run_id, prompt: PROMPT, ...extra }) } } })

// A stand-in CLI: `script` is the body of a function (args, input) returning { code, stdout, stderr }.
function cli(dir, body) {
  const file = join(dir, "chariox")
  writeFileSync(file, `#!/usr/bin/env node\nimport { readFileSync, appendFileSync } from "node:fs"\nconst args = process.argv.slice(2)\nconst input = args[0] === "request" ? readFileSync(0, "utf8") : ""\nappendFileSync(${JSON.stringify(join(dir, "calls"))}, args.join(" ") + "\\n")\nconst answer = ((args, input) => { ${body} })(args, input)\nprocess.stdout.write(answer.stdout ?? "")\nprocess.stderr.write(answer.stderr ?? "")\nprocess.exit(answer.code ?? 0)\n`)
  chmodSync(file, 0o755)
  return file
}
const sources = `if (args[1] === "sources") return { stdout: JSON.stringify({ sources: [${JSON.stringify(SOURCE)}, { kind: "workflow_exit", source_id: "x", display_name: "exit" }] }) }
if (args[1] === "subscription") return { stdout: JSON.stringify({ subscription: { sources: [{ source_id: "run_finished_a" }] } }) }`
async function wait(dir, binary, ...flags) {
  const out = []
  const code = await main(["--chariox", binary, "--state", join(dir, "state.json"), ...flags], line => out.push(line))
  return { code, out }
}
const scratch = () => mkdtempSync(join(tmpdir(), "rwe-"))

test("a completed and a failed run read like the polling wait's lines", () => {
  assert.equal(formatLine({ at, status: "Completed", prompt: PROMPT, runId: "r1" }), "09:17Z review Completed charioxai/chariox #1279 @0e8b5e6fd8")
  assert.equal(formatLine({ at, status: "Failed", prompt: PROMPT, runId: "r1", failure: "agent stopped" }), "09:17Z review Failed charioxai/chariox #1279 @0e8b5e6fd8: agent stopped")
  assert.equal(formatLine({ at, status: "Completed", prompt: "something else", runId: "r9" }), "09:17Z review Completed run r9")
})

test("only a connection problem is an unreachable kernel", () => {
  assert.ok(kernelUnreachable({ code: 1, stderr: "connect ECONNREFUSED 127.0.0.1:50000" }))
  assert.ok(kernelUnreachable({ code: 3, stderr: "kernel is shutting down" }))
  assert.ok(!kernelUnreachable({ code: 5, stderr: "" }))
  assert.ok(!kernelUnreachable({ code: 1, stderr: "events_refused: source not available" }))
})

test("a notification ends the wait with one line per run, once, and is acknowledged", async () => {
  const dir = scratch()
  const binary = cli(dir, `${sources}
if (args[1] === "wait") return { stdout: JSON.stringify({ events: [${JSON.stringify(notification("r1", "Completed"))}, ${JSON.stringify(notification("r2", "Failed", { failure: "boom" }))}], next_cursor: 7 }) }
return { stdout: "{}" }`)
  const first = await wait(dir, binary, "--window-minutes", "1")
  assert.equal(first.code, 0)
  assert.deepEqual(first.out, ["09:17Z review Completed charioxai/chariox #1279 @0e8b5e6fd8", "09:17Z review Failed charioxai/chariox #1279 @0e8b5e6fd8: boom"])
  const calls = (await import("node:fs")).readFileSync(join(dir, "calls"), "utf8")
  assert.match(calls, /events ack 7/)
  // The same notifications again (a lost acknowledgement) print nothing new; the window then closes.
  const second = await wait(dir, binary, "--window-minutes", "0.02")
  assert.deepEqual(second.out, ["no review finished in the window"])
  rmSync(dir, { recursive: true })
})

test("without any notification the runs are listed once after the fallback and a finished run is not hidden", async () => {
  const dir = scratch()
  const runs = [{ id: "old", status: "Completed", completed_at_ms: 1, invocation_prompt: PROMPT }, { id: "new", status: "Failed", completed_at_ms: Date.now() + 60_000, invocation_prompt: PROMPT, failure_events: [{ message: "no verdict" }] }]
  const binary = cli(dir, `${sources}
if (args[1] === "wait") return { code: 5 }
if (args[0] === "request") return { stdout: JSON.stringify({ WorkflowRunsListed: { workflow_runs: ${JSON.stringify(runs)} } }) }
return { stdout: "{}" }`)
  const result = await wait(dir, binary, "--window-minutes", "1", "--fallback-minutes", "0.02")
  assert.equal(result.code, 0)
  assert.equal(result.out.length, 1)
  assert.match(result.out[0], /^\d\d:\d\dZ review Failed charioxai\/chariox #1279 @0e8b5e6fd8: no verdict$/)
  rmSync(dir, { recursive: true })
})

test("an unreachable kernel and a window without a run are reported, not hidden", async () => {
  const dir = scratch()
  const down = cli(dir, `return { code: 1, stderr: "connect ECONNREFUSED 127.0.0.1:50000" }`)
  assert.deepEqual((await wait(dir, down)).out, ["reviewer kernel unreachable"])
  const quiet = cli(dir, `${sources}
return { code: 5 }`)
  assert.deepEqual((await wait(dir, quiet, "--window-minutes", "0.02")).out, ["no review finished in the window"])
  rmSync(dir, { recursive: true })
})

test("a revoked grant says so instead of reporting an unreachable kernel", async () => {
  const dir = scratch()
  const ended = { events: [{ type: "subscription_ended", value: { reason: "grant revoked" } }] }
  const binary = cli(dir, `${sources}
if (args[1] === "wait") return { code: 3, stdout: ${JSON.stringify(JSON.stringify(ended))} }
return { stdout: "{}" }`)
  const result = await wait(dir, binary, "--window-minutes", "1")
  assert.equal(result.code, 0)
  assert.deepEqual(result.out, ['event follow ended: {"reason":"grant revoked"}'])
  rmSync(dir, { recursive: true })
})
