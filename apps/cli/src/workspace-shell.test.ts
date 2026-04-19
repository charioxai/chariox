import assert from "node:assert/strict"
import test from "node:test"

import {
  appendWorkspaceShellEntry,
  isWorkspaceShellCommand,
  renderWorkspaceShellTranscript,
  workspaceShellCommandText,
} from "./workspace-shell.js"

test("workspace shell command parsing uses @ as the pane prompt marker", () => {
  assert.equal(isWorkspaceShellCommand("@ session list"), true)
  assert.equal(isWorkspaceShellCommand("  @ vars"), true)
  assert.equal(isWorkspaceShellCommand("session list"), false)
  assert.equal(workspaceShellCommandText("  @ session list"), "session list")
})

test("workspace shell transcript renders input separately from output", () => {
  const transcript = renderWorkspaceShellTranscript([
    { id: 1, command: "session list", output: "Sessions\n- abc", ok: true },
    { id: 2, command: "agent list", output: "no current session", ok: false },
  ])

  assert.match(transcript, /^@ session list/m)
  assert.match(transcript, /Sessions\n- abc/)
  assert.match(transcript, /@ agent list\nerror: no current session/)
})

test("workspace shell transcript is bounded", () => {
  const entries = appendWorkspaceShellEntry([
    { id: 1, command: "one", output: "1", ok: true },
    { id: 2, command: "two", output: "2", ok: true },
  ], { id: 3, command: "three", output: "3", ok: true }, 2)

  assert.deepEqual(entries.map((entry) => entry.command), ["two", "three"])
})
