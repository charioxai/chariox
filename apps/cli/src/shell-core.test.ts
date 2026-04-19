import assert from "node:assert/strict"
import test from "node:test"

import {
  applyShellCommandResult,
  createDefaultShellContext,
  parseShellCommand,
  renderShellCommandResult,
  tokenizeShellLine,
} from "./shell-core.js"

test("tokenizeShellLine handles quotes and escaped spaces", () => {
  assert.deepEqual(
    tokenizeShellLine('workflow run qa default "Run QA" path\\ with\\ spaces'),
    ["workflow", "run", "qa", "default", "Run QA", "path with spaces"],
  )
})

test("parseShellCommand normalizes slash and shell commands", () => {
  assert.deepEqual(parseShellCommand("/agent spawn reviewer gpt-5.2 as reviewer"), {
    kind: "shared",
    raw: "/agent spawn reviewer gpt-5.2 as reviewer",
    normalized: "agent spawn reviewer gpt-5.2",
    command: "agent",
    args: ["spawn", "reviewer", "gpt-5.2"],
    assignment: "reviewer",
  })
  assert.equal(parseShellCommand("agent list").kind, "shared")
})

test("parseShellCommand substitutes shell variables", () => {
  const parsed = parseShellCommand("agent spawn reviewer --session $s as reviewer", {
    variables: { s: "session-1" },
  })
  assert.equal(parsed.kind, "shared")
  assert.deepEqual(parsed.args, ["spawn", "reviewer", "--session", "session-1"])
  assert.equal(parsed.assignment, "reviewer")
})

test("parseShellCommand classifies shell-local and tui-only commands", () => {
  assert.equal(parseShellCommand("set model gpt-5.2").kind, "shell-local")
  const waiting = parseShellCommand("/waiting")
  assert.equal(waiting.kind, "tui-only")
  assert.match(waiting.reason ?? "", /TUI/)
  assert.equal(parseShellCommand("/opencode status").kind, "tui-only")
})

test("parseShellCommand rejects invalid assignments and unknown variables", () => {
  assert.equal(parseShellCommand("agent list as 123").kind, "invalid")
  assert.equal(parseShellCommand("agent list --session $missing").kind, "invalid")
})

test("applyShellCommandResult updates context and bindings", () => {
  const context = createDefaultShellContext({ workspace: "/repo", provider: "codex" })
  const next = applyShellCommandResult(context, {
    ok: true,
    message: "created session session-1",
    bindings: { s: "session-1" },
    contextUpdates: { sessionId: "session-1" },
  })
  assert.equal(next.sessionId, "session-1")
  assert.equal(next.variables.s, "session-1")
  assert.equal(next.provider, "codex")
})

test("renderShellCommandResult prints messages and variable bindings", () => {
  assert.equal(
    renderShellCommandResult({ ok: true, message: "spawned agent agent-2", bindings: { reviewer: "agent-2" } }),
    "spawned agent agent-2\nbound $reviewer = agent-2",
  )
})
