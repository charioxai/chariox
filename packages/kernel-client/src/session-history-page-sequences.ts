import type { SessionHistoryPageEntry } from "./kernel-types-history.js"
import { splitTerminalOutputSequences } from "./terminal-output-sequences.js"

// MP-08 / MP-10: expand compact wire rows before fragment/replay reconciliation.
export function expandSessionHistoryPageSequences(page: SessionHistoryPageEntry): SessionHistoryPageEntry[] {
  if (page.sequence_byte_lengths === undefined) return [page]
  if (page.fragment_start !== 0 || page.fragment_end !== page.total_chars || [...page.entry.text].length !== page.total_chars) {
    throw new Error("malformed complete history sequence range")
  }
  const indices = page.sequence_indices ?? page.sequence_byte_lengths.map((_, i) => page.entry_index + i)
  if (indices.length !== page.sequence_byte_lengths.length || indices[0] !== page.entry_index || indices.at(-1) !== page.sequence_end
      || indices.some((n, i) => !Number.isSafeInteger(n) || n < 0 || (i > 0 && n <= indices[i - 1]!))) {
    throw new Error("malformed history row identities")
  }
  const bytes = [...new TextEncoder().encode(page.entry.text)]
  return splitTerminalOutputSequences({
    bytes,
    sequence: 0,
    sequence_end: page.sequence_byte_lengths.length - 1,
    sequence_byte_lengths: page.sequence_byte_lengths,
  }).map(row => {
    const text = new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(row.bytes))
    const chars = [...text].length
    const { sequence_end: _end, sequence_byte_lengths: _lengths, sequence_indices: _indices, ...single } = page
    return { ...single, entry_index: indices[row.sequence]!, fragment_start: 0, fragment_end: chars, total_chars: chars, entry: { ...page.entry, text } }
  })
}
