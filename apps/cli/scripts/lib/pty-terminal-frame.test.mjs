import assert from "node:assert/strict"
import test from "node:test"

import { replayPtyFrame, stripPtyControlSequences } from "./pty-terminal-frame.mjs"

test("replayPtyFrame reconstructs cursor-addressed terminal output", () => {
  const raw = "\u001b[2J\u001b[Hhome:mcp:browser\u001b[2;1Hworker:mcp:browser\u001b[2;8Hworker-local"
  const frame = replayPtyFrame(raw, 40, 4)
  assert.equal(frame.lines[0], "home:mcp:browser")
  assert.equal(frame.lines[1], "worker:worker-local")
})

test("stripPtyControlSequences preserves actual text while removing terminal controls", () => {
  assert.equal(stripPtyControlSequences("\u001b[31mworker\u001b[0m\r\n"), "worker\n")
})
