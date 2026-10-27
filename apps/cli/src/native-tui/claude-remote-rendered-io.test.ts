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
        kind: "provider_terminal",
        bytes: [...Buffer.from("WRONG_AGENT")],
      },
      {
        provider_run_id: "provider-run-2",
        agent_id: "agent-home-b",
        kind: "provider_output",
        bytes: [...Buffer.from("SEMANTIC_ANSWER")],
      },
      {
        provider_run_id: "provider-run-2",
        agent_id: "agent-home-b",
        kind: "provider_terminal",
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
    kind: "provider_terminal",
    bytes: [...Buffer.from("Claude Code\u001b[?2004h")],
  }], "leased:agent-b:provider-run-2", "agent-home-b")
  await new Promise((resolve) => setTimeout(resolve, 0))
  assert.equal(resolved, false)

  readiness.observe([{
    provider_run_id: "provider-run-2",
    agent_id: "agent-home-b",
    kind: "provider_terminal",
    bytes: [...Buffer.from("\u001b]0;Claude ")],
  }], "leased:agent-b:provider-run-2", "agent-home-b")
  readiness.observe([{
    provider_run_id: "provider-run-2",
    agent_id: "agent-home-b",
    kind: "provider_terminal",
    bytes: [...Buffer.from("Code\u0007\u001b[?2004hClaude\u001b[19GCode\r\n───\r\n❯ Try\"fix lint errors\"\r\n───")],
  }], "leased:agent-b:provider-run-2", "agent-home-b")

  await waiting
  assert.equal(resolved, true)
})

test("MP-08/MP-11 styled Claude onboarding is not a ready composer", async () => {
  const readiness = createClaudeRemoteRenderedReadiness()
  const observe = (data: string) => readiness.observe([{
    provider_run_id: "run-1", agent_id: "agent-1", kind: "provider_terminal", bytes: [...Buffer.from(data)],
  }], "run-1", "agent-1")
  observe("\u001b[?2004hWelcome\u001b[9Gto\u001b[12GClaude\u001b[19GCode\r\nChoose the text style\r\n❯Dark mode")
  await assert.rejects(readiness.wait(20), /timed out/)
  observe("\r\nAccessing workspace:\r\n❯No, exit\r\nYes, I trust this folder\r\nEnter to confirm")
  await assert.rejects(readiness.wait(20), /timed out/)
  // The real 2.1.292 PTY draws the empty prompt after normal trust approval.
  const painted = Buffer.from("\r\n────────────────────\r\n❯ \r\n────────────────────\r\n")
  const glyph = painted.indexOf(Buffer.from("❯"))
  for (const bytes of [painted.subarray(0, glyph + 1), painted.subarray(glyph + 1)]) {
    readiness.observe([{ provider_run_id: "run-1", agent_id: "agent-1", kind: "provider_terminal", bytes: [...bytes] }], "run-1", "agent-1")
  }
  await readiness.wait(100)
})

test("MP-08/MP-11 initial prompt allows more than sixty seconds for native onboarding", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] })
  const readiness = createClaudeRemoteRenderedReadiness()
  let outcome = "pending"
  const waiting = readiness.wait().then(() => { outcome = "ready" }, () => { outcome = "failed" })
  context.mock.timers.tick(61_000)
  await Promise.resolve()
  assert.equal(outcome, "pending")
  readiness.observe([{
    provider_run_id: "run-1", agent_id: "agent-1", kind: "provider_terminal",
    bytes: [...Buffer.from("\u001b[?2004hClaude Code\r\n❯ \r\n────────────────────\r\n")],
  }], "run-1", "agent-1")
  await waiting
  assert.equal(outcome, "ready")
})

test("MP-08/MP-11 provider exit releases an initial prompt readiness wait immediately", async () => {
  const readiness = createClaudeRemoteRenderedReadiness()
  const waiting = readiness.wait(50)
  const ended = new Error("Claude native TUI ended before its input composer was ready")
  readiness.fail(ended)
  await assert.rejects(waiting, (error) => error === ended)
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
    kind: "provider_terminal",
    bytes: [...Buffer.from("\u001b]0;Claude Code\u0007\u001b[?2004hClaude Code\r\n───\r\n❯ \r\n───")],
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
