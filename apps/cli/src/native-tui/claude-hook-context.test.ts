// MP-08 / MP-10 / MP-11: real generated hook must fail a selected-body timeout.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import path from "node:path"
import test from "node:test"
import { writeClaudeHookHandler } from "./claude-hook-handler.js"

async function hook(prompt: string, kernel = false) {
  const dir = await mkdtemp(path.join(tmpdir(), "skillsp-hook-"))
  try {
    const handler = path.join(dir, "hook.mjs")
    if (kernel) {
      const source = await readFile(new URL("../../../kernel/src/provider/claude/native_tui.rs", import.meta.url), "utf8")
      const script = source.match(/fn claude_native_hook_handler\(\) -> &'static str \{\s*r#"([\s\S]*?)"#/)
      assert.ok(script, "extract the actual kernel hook asset")
      await writeFile(handler, script[1]!)
    } else await writeClaudeHookHandler(handler)
    await mkdir(path.join(dir, "responses"))
    await writeFile(path.join(dir, "context"), "")
    const child = spawn("node", [handler], {
      env: { ...process.env, CHARIOX_HOME: dir,
        CHARIOX_CLAUDE_NATIVE_CONTEXT_CHUNK: "0",
        CHARIOX_CLAUDE_NATIVE_EVENTS: path.join(dir, "events.jsonl"),
        CHARIOX_CLAUDE_NATIVE_CONTEXT: path.join(dir, "context"),
        CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES: path.join(dir, "responses") },
      stdio: ["pipe", "pipe", "pipe"],
    })
    const stdout: Buffer[] = []
    child.stdout.on("data", data => stdout.push(data))
    child.stdin.end(JSON.stringify({ hook_event_name: "UserPromptSubmit", prompt }))
    const code = await new Promise(resolve => child.once("exit", resolve))
    return { code, result: JSON.parse(Buffer.concat(stdout).toString()),
      events: (await readFile(path.join(dir, "events.jsonl"), "utf8")).trim().split("\n").map(line => JSON.parse(line)) }
  } finally { await rm(dir, { recursive: true, force: true }) }
}
test("selected Chariox skill cannot continue with empty context after timeout", { timeout: 10_000 }, async () => {
  const { result, events } = await hook("$review inspect source")
  assert.equal(events.some(e => e.hook_event_name === "UserPromptSubmit"), false)
  assert.equal(result.decision, "block")
  assert.match(result.reason, /skill.*context.*refresh|skill.*context.*retry/i)
})
test("ordinary prompt without skills still works when no skill bridge replies", { timeout: 10_000 }, async () => {
  const { code, result, events } = await hook("explain the source")
  assert.equal(events.filter(e => e.hook_event_name === "UserPromptSubmit").length, 1)
  assert.equal(code, 0)
  assert.equal(result.decision, undefined)
  assert.equal(result.hookSpecificOutput.additionalContext, "")
})

// Execute the production kernel's generated hook as well as the release utility.
test("kernel hook blocks a selected skill when its context delivery times out", { timeout: 10_000 }, async () => {
  const { result, events } = await hook("$review inspect source", true)
  assert.equal(events.some(e => e.hook_event_name === "UserPromptSubmit"), false)
  assert.equal(result.decision, "block")
  assert.match(result.reason, /skill.*context.*retry/i)
})
test("kernel hook still admits an ordinary prompt without selected skills", { timeout: 10_000 }, async () => {
  const { result } = await hook("explain source", true)
  assert.equal(result.decision, undefined)
  assert.equal(result.hookSpecificOutput.additionalContext, "")
})

for (const kernel of [false, true]) test(`quoted/fenced dollar examples remain ordinary prompts (${kernel ? "kernel" : "release"})`, { timeout: 10_000 }, async () => {
  const { result } = await hook("Explain this example:\n```sh\n$review\n```\nthen \"use $review\" or `$review` with $HOME", kernel)
  assert.equal(result.decision, undefined)
})
