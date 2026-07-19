import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "../ipc.js"
import {
  createClaudeRemoteRenderedReadiness,
  createClaudeRemoteRenderedComposerState,
  installClaudeRemoteRenderedResizeForwarder,
  planClaudeRemoteRenderedInput,
  submitClaudeRemoteRenderedInitialPrompt,
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

test("remote Claude readiness waits for its fragmented input surface", async () => {
  const readiness = createClaudeRemoteRenderedReadiness()
  let resolved = false
  const waiting = readiness.wait(250).then(() => {
    resolved = true
  })

  readiness.observe([{
    provider_run_id: "provider-run-other",
    agent_id: "agent-other",
    bytes: [...Buffer.from("Claude Code\u001b[?2004h")],
  }], "leased:agent-b:provider-run-2", "agent-home-b")
  await new Promise((resolve) => setTimeout(resolve, 0))
  assert.equal(resolved, false)

  readiness.observe([{
    provider_run_id: "provider-run-2",
    agent_id: "agent-home-b",
    bytes: [...Buffer.from("\u001b]0;Claude ")],
  }], "leased:agent-b:provider-run-2", "agent-home-b")
  readiness.observe([{
    provider_run_id: "provider-run-2",
    agent_id: "agent-home-b",
    bytes: [...Buffer.from("Code\u0007\u001b[?2004h")],
  }], "leased:agent-b:provider-run-2", "agent-home-b")

  await waiting
  assert.equal(resolved, true)
})

test("remote Claude initial prompt waits for readiness then submits Enter separately", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      return {}
    },
  } as unknown as LocalIpcClient
  const readiness = createClaudeRemoteRenderedReadiness()
  readiness.observe([{
    provider_run_id: "provider-run-2",
    agent_id: "agent-home-b",
    bytes: [...Buffer.from("\u001b]0;Claude Code\u0007\u001b[?2004h")],
  }], "leased:agent-b:provider-run-2", "agent-home-b")

  await submitClaudeRemoteRenderedInitialPrompt({
    client,
    sessionId: "session-1",
    attachmentId: "attachment-1",
    providerRunId: "leased:agent-b:provider-run-2",
    prompt: "Reply exactly",
    readiness,
  })

  assert.deepEqual(requests, [
    {
      SendTerminalInput: {
        session_id: "session-1",
        attachment_id: "attachment-1",
        provider_run_id: "leased:agent-b:provider-run-2",
        data_base64: Buffer.from("Reply exactly").toString("base64"),
      },
    },
    {
      SendTerminalInput: {
        session_id: "session-1",
        attachment_id: "attachment-1",
        provider_run_id: "leased:agent-b:provider-run-2",
        data_base64: Buffer.from("\r").toString("base64"),
      },
    },
  ])
})

test("remote Claude resize targets its provider run and retries a transport outage", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if (requests.length === 1) throw new Error("relay unavailable")
      return {}
    },
  } as unknown as LocalIpcClient

  await withStdoutDimensions(async () => {
    const dispose = installClaudeRemoteRenderedResizeForwarder(
      client,
      "session-1",
      "provider-run-2",
    )
    try {
      await waitFor(() => requests.length >= 2)
    } finally {
      dispose()
    }
  })

  assert.deepEqual(requests, [
    {
      ResizeTerminal: {
        session_id: "session-1",
        provider_run_id: "provider-run-2",
        cols: 80,
        rows: 24,
      },
    },
    {
      ResizeTerminal: {
        session_id: "session-1",
        provider_run_id: "provider-run-2",
        cols: 80,
        rows: 24,
      },
    },
  ])
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

async function withStdoutDimensions(run: () => Promise<void>): Promise<void> {
  const properties = ["columns", "rows"] as const
  const descriptors = Object.fromEntries(properties.map((property) => [
    property,
    Object.getOwnPropertyDescriptor(process.stdout, property),
  ]))
  Object.defineProperties(process.stdout, {
    columns: { configurable: true, value: 80 },
    rows: { configurable: true, value: 24 },
  })
  try {
    await run()
  } finally {
    for (const property of properties) {
      const descriptor = descriptors[property]
      if (descriptor) Object.defineProperty(process.stdout, property, descriptor)
      else delete (process.stdout as unknown as Record<string, unknown>)[property]
    }
  }
}

async function waitFor(predicate: () => boolean): Promise<void> {
  const deadline = Date.now() + 2_000
  while (Date.now() < deadline) {
    if (predicate()) return
    await new Promise((resolve) => setTimeout(resolve, 10))
  }
  throw new Error("timed out waiting for resize retry")
}
