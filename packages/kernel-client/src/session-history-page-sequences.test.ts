import assert from "node:assert/strict"
import test from "node:test"
import { hydrateSessionHistoryTranscriptEntries } from "./session-history-transcript.js"
import type { SessionHistoryPageEntry } from "./kernel-types-history.js"

test("MP-08 / MP-10: compact history restores distinct rows before replay deduplication", () => {
 const wire = { entry_index: 10, sequence_end: 12, sequence_byte_lengths: [2,3,2], fragment_start: 0, fragment_end: 3, total_chars: 3, entry: { kind: "provider_output", agent_id: "a", merge_key: "m", text: "é中é" } } as SessionHistoryPageEntry
 const entries = hydrateSessionHistoryTranscriptEntries([wire])
 assert.equal(entries[0]?.text, "é中é")
 assert.deepEqual(entries[0]?.historySequenceRanges, [[10,12]])
})

test("MP-08 / MP-10: interleaved agents do not invent missing sequence identities", () => {
 const wire={entry_index:10,sequence_end:13,sequence_indices:[10,12,13],sequence_byte_lengths:[2,3,2],fragment_start:0,fragment_end:3,total_chars:3,entry:{kind:"provider_output",agent_id:"a",merge_key:"m",text:"é中é"}} as SessionHistoryPageEntry
 const entries=hydrateSessionHistoryTranscriptEntries([wire]);assert.equal(entries[0]?.text,"é中é");assert.deepEqual(entries[0]?.historySequenceRanges,[[10,10],[12,13]])
})
