import assert from "node:assert/strict"
import test from "node:test"

import {
  createClaudeRemoteRenderedComposerState,
  planClaudeRemoteRenderedInput,
  writeClaudeRemoteRenderedTerminalRecords,
} from "./claude-remote-rendered-io.js"

test("remote Claude stdin keeps a terminal response in one transport write", () => {
  const composer = createClaudeRemoteRenderedComposerState()

  assert.deepEqual(
    planClaudeRemoteRenderedInput(composer, Buffer.from("\u001b[?1;2c")),
    [{ type: "input", data: "\u001b[?1;2c" }],
  )
  assert.equal(composer.text, "")
})

test("remote Claude stdin coalesces a terminal response split across chunks", () => {
  const composer = createClaudeRemoteRenderedComposerState()

  assert.deepEqual(
    planClaudeRemoteRenderedInput(composer, Buffer.from("\u001b[?1;")),
    [],
  )
  assert.deepEqual(
    planClaudeRemoteRenderedInput(composer, Buffer.from("2cReply")),
    [{ type: "input", data: "\u001b[?1;2cReply" }],
  )
  assert.equal(composer.text, "Reply")
})

test("remote Claude stdin does not treat SS3 navigation as composer text", () => {
  const composer = createClaudeRemoteRenderedComposerState()

  assert.deepEqual(
    planClaudeRemoteRenderedInput(composer, Buffer.from("\u001bOA")),
    [{ type: "input", data: "\u001bOA" }],
  )
  assert.equal(composer.text, "")
})

test("remote Claude stdin batches text and preserves one CRLF submit", () => {
  const composer = createClaudeRemoteRenderedComposerState()

  assert.deepEqual(
    planClaudeRemoteRenderedInput(composer, Buffer.from("hello\r\n")),
    [
      { type: "input", data: "hello" },
      { type: "enter", prompt: "hello" },
    ],
  )
  assert.equal(composer.text, "")
})

test("remote Claude stdin snapshots composer text at each submit boundary", () => {
  const composer = createClaudeRemoteRenderedComposerState()

  assert.deepEqual(
    planClaudeRemoteRenderedInput(composer, Buffer.from("first\rsecond\r")),
    [
      { type: "input", data: "first" },
      { type: "enter", prompt: "first" },
      { type: "input", data: "second" },
      { type: "enter", prompt: "second" },
    ],
  )
  assert.equal(composer.text, "")
})

test("remote Claude rendering selects raw worker output by home agent identity", () => {
  const output = captureStdout(() => {
    writeClaudeRemoteRenderedTerminalRecords([
      {
        provider_run_id: "provider-run-2",
        agent_id: "agent-other",
        bytes: [...Buffer.from("WRONG_AGENT")],
      },
      {
        provider_run_id: "provider-run-2",
        agent_id: "agent-home-b",
        bytes: [...Buffer.from("CLAUDEDELTA")],
      },
    ], "leased:leased-agent-b:provider-run-2", "agent-home-b")
  })

  assert.equal(output, "CLAUDEDELTA")
})

function captureStdout(run: () => void): string {
  const previousWrite = process.stdout.write
  const chunks: Buffer[] = []
  ;(process.stdout.write as unknown as (chunk: string | Uint8Array) => boolean) = (chunk) => {
    chunks.push(Buffer.from(chunk))
    return true
  }
  try {
    run()
  } finally {
    process.stdout.write = previousWrite
  }
  return Buffer.concat(chunks).toString("utf8")
}
