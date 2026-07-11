import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "../ipc.js"
import {
  createClaudeRemoteRenderedComposerState,
  installClaudeRemoteRenderedResizeForwarder,
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
