// Smoke-tests a compiled `chariox` release executable with neither Node nor Bun
// on its PATH, the way a release installation runs it:
//
//   node apps/cli/scripts/release-smoke.mjs EXECUTABLE VERSION
//
// It checks `--version` and `--help`, then runs native-Claude hooks exactly as
// Claude Code would (the shell command from claudeHookSettings): the hidden
// prompt context of UserPromptSubmit and the shared permission contract.
import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

import { claudeHookShellCommand, writeClaudeHookHandler } from "../dist/native-tui/claude-hook-handler.js"

const [executableArg, version] = process.argv.slice(2)
if (!executableArg || !version) throw new Error("usage: release-smoke.mjs EXECUTABLE VERSION")
const executable = await realpath(executableArg)
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..")
const scratch = await mkdtemp(path.join(await realpath(tmpdir()), "chariox-release-smoke-"))
// Only system tools; the release must not need Node or Bun.
const env = { PATH: "/usr/bin:/bin", HOME: scratch, LANG: "C.UTF-8" }
const run = (command, input = "", extra = {}) =>
  execFileSync("/bin/sh", ["-c", command], { encoding: "utf8", input, env: { ...env, ...extra }, timeout: 20_000 })

try {
  assert.equal(run("command -v node bun || true").trim(), "", "node or bun is on the smoke PATH")
  assert.equal(run(`${JSON.stringify(executable)} --version`).trim(), `chariox ${version}`)
  assert.match(run(`${JSON.stringify(executable)} --help`), /^usage: /)

  const handler = path.join(scratch, "hook handler.mjs")
  await writeClaudeHookHandler(handler)
  const hook = claudeHookShellCommand(handler, { version, executable })
  const hookEnv = {
    CHARIOX_CLAUDE_NATIVE_EVENTS: path.join(scratch, "events.jsonl"),
    CHARIOX_CLAUDE_NATIVE_CONTEXT: path.join(scratch, "context.txt"),
    CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES: path.join(scratch, "context-responses"),
    CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES: path.join(scratch, "permission-responses"),
  }
  await mkdir(hookEnv.CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES)
  await mkdir(hookEnv.CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES)
  await writeFile(hookEnv.CHARIOX_CLAUDE_NATIVE_CONTEXT, "hidden context from Chariox")
  const prompt = JSON.parse(run(hook, JSON.stringify({ hook_event_name: "UserPromptSubmit", prompt: "hi" }), hookEnv))
  assert.deepEqual(prompt.hookSpecificOutput, { hookEventName: "UserPromptSubmit", additionalContext: "hidden context from Chariox" })
  const contract = JSON.parse(await readFile(path.join(repoRoot, "fixtures/claude-permission-hook-contract.json"), "utf8"))
  for (const contractCase of contract) {
    const response = JSON.parse(run(hook, JSON.stringify(contractCase.input), hookEnv))
    assert.equal(response.hookSpecificOutput.decision.behavior, contractCase.behavior, contractCase.name)
  }
  assert.match(await readFile(hookEnv.CHARIOX_CLAUDE_NATIVE_EVENTS, "utf8"), /"hook_event_name":"PermissionRequest"/)
  process.stdout.write(`${JSON.stringify({ ok: true, executable, version, hook, contractCases: contract.length })}\n`)
} finally {
  await rm(scratch, { recursive: true, force: true })
}
