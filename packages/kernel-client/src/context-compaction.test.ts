import assert from "node:assert/strict"
import test from "node:test"

import { parseContextCompaction } from "./context-compaction.js"

test("a compaction is running, done with its summary, or failed with its reason", () => {
  assert.deepEqual(parseContextCompaction('{"state":"running","started_at_ms":5}'), { state: "running", started_at_ms: 5 })
  assert.deepEqual(parseContextCompaction('{"state":"done","summary":""}'), { state: "done", summary: "" })
  assert.deepEqual(parseContextCompaction('{"state":"failed","reason":"too long"}'), { state: "failed", reason: "too long" })
})

test("a compaction state this client does not know is refused, not drawn as an empty row", () => {
  assert.throws(() => parseContextCompaction('{"state":"paused"}'), /does not know/)
  assert.throws(() => parseContextCompaction("not json"))
})
