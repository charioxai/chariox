import assert from "node:assert/strict"
import test from "node:test"
import { splitTerminalOutputSequences } from "./terminal-output-sequences.js"

test("MP-08 / MP-10 preserves UTF-8 row boundaries and consecutive history sequences", () => {
  const record = { sequence: 10, sequence_end: 11, sequence_byte_lengths: [2, 3], bytes: [...new TextEncoder().encode("é中")] }
  const fragments = splitTerminalOutputSequences(record)
  assert.deepEqual(fragments.map((r) => r.sequence), [10, 11])
  assert.deepEqual(fragments.map((r) => new TextDecoder().decode(new Uint8Array(r.bytes))), ["é", "中"])
  assert.deepEqual(record.sequence_byte_lengths, [2, 3])
})

test("MP-08 / MP-10 rejects malformed ranges without silently losing live text", () => {
  for (const change of [{ sequence_end: 12 }, { sequence: -1 }, { sequence_byte_lengths: [1, 1] }, { sequence_byte_lengths: [2, -1] }]) {
    assert.throws(() => splitTerminalOutputSequences({ sequence: 10, sequence_end: 11, sequence_byte_lengths: [2, 3], bytes: [1, 2, 3, 4, 5], ...change }), /malformed/)
  }
  const legacy = { bytes: [1, 2] }
  assert.deepEqual(splitTerminalOutputSequences(legacy), [legacy])
})
