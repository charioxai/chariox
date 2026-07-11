import assert from "node:assert/strict"
import test from "node:test"

import { writeClaudeRemoteRenderedTerminalRecords } from "./claude-remote-rendered-io.js"

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
