import test from "node:test"
import assert from "node:assert/strict"
import { mkdtempSync, existsSync, readFileSync, rmSync } from "node:fs"
import { join } from "node:path"
import { tmpdir } from "node:os"
import type { CliRenderer } from "@opentui/core"
import type { TranscriptEntry } from "./cli-types.js"
import { recordPaneFirstFrame } from "./pane-first-frame-timing.js"

test("MP-08 / MP-10 assistant timing follows the accepted prompt, not a provider echo", () => {
  const root = mkdtempSync(join(process.env.TMPDIR ?? tmpdir(), "pane-receipt-"))
  const path = join(root, "receipt.jsonl")
  const oldPath = process.env.CHARIOX_PANE_TIMING_FILE
  const oldPrefix = process.env.CHARIOX_PANE_TIMING_PREFIX
  process.env.CHARIOX_PANE_TIMING_FILE = path
  process.env.CHARIOX_PANE_TIMING_PREFIX = "MP1-test-"
  let sample = () => {}
  const renderer = { addPostProcessFn: (f: () => void) => { sample = f }, removePostProcessFn: () => {} } as unknown as CliRenderer
  const entries: TranscriptEntry[] = [
    { id: 1, role: "user", text: "Review source MP1-test-01", promptId: "prompt-1", turnId: 1 },
    { id: 2, role: "assistant", text: "An older reply quotes MP1-test-01", promptId: "prompt-0", turnId: 0 },
  ]
  let dispose = () => {}
  try {
    dispose = recordPaneFirstFrame(renderer, () => false, () => entries)
    sample()
    assert.equal(JSON.parse(readFileSync(path, "utf8").trim()).event, "pane_user_frame")
    entries.push({ id: 3, role: "assistant", text: "The actual public answer omits the requested marker.", promptId: "prompt-1", turnId: 1 })
    sample()
    sample()
    const rows = readFileSync(path, "utf8").trim().split("\n").map(line => JSON.parse(line)).filter(row => row.event === "pane_assistant_frame")
    assert.equal(rows.length, 1)
    assert.deepEqual({ event: rows[0].event, tag: rows[0].tag, entryId: rows[0].entryId, promptId: rows[0].promptId, turnId: rows[0].turnId }, { event: "pane_assistant_frame", tag: "MP1-test-01", entryId: 3, promptId: "prompt-1", turnId: 1 })
  } finally {
    dispose()
    if (oldPath === undefined) delete process.env.CHARIOX_PANE_TIMING_FILE
    else process.env.CHARIOX_PANE_TIMING_FILE = oldPath
    if (oldPrefix === undefined) delete process.env.CHARIOX_PANE_TIMING_PREFIX
    else process.env.CHARIOX_PANE_TIMING_PREFIX = oldPrefix
    rmSync(root, { recursive: true })
  }
})

test("MP-08 / MP-10 user frame timing does not wait for provider output", () => {
  const root = mkdtempSync(join(process.env.TMPDIR ?? tmpdir(), "pane-user-receipt-"))
  const path = join(root, "receipt.jsonl")
  const oldPath = process.env.CHARIOX_PANE_TIMING_FILE, oldPrefix = process.env.CHARIOX_PANE_TIMING_PREFIX
  process.env.CHARIOX_PANE_TIMING_FILE = path
  process.env.CHARIOX_PANE_TIMING_PREFIX = "MP1-test-"
  let sample = () => {}
  const renderer = { addPostProcessFn: (f: () => void) => { sample = f }, removePostProcessFn: () => {} } as unknown as CliRenderer
  try {
    const dispose = recordPaneFirstFrame(renderer, () => false, () => [{ id: 1, role: "user", text: "Review MP1-test-01", promptId: "prompt-1", turnId: 1 }])
    sample(); sample(); dispose()
    assert.equal(existsSync(path), true)
    const rows = readFileSync(path, "utf8").trim().split("\n").map(row => JSON.parse(row))
    assert.equal(rows.length, 1)
    assert.equal(rows[0].event, "pane_user_frame")
    assert.equal(rows[0].promptId, "prompt-1")
  } finally {
    if (oldPath === undefined) delete process.env.CHARIOX_PANE_TIMING_FILE; else process.env.CHARIOX_PANE_TIMING_FILE = oldPath
    if (oldPrefix === undefined) delete process.env.CHARIOX_PANE_TIMING_PREFIX; else process.env.CHARIOX_PANE_TIMING_PREFIX = oldPrefix
    rmSync(root, { recursive: true })
  }
})
