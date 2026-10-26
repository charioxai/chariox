// MP-08 / MP-10, protocol 508: recover persisted row boundaries from a
// coalesced transport record. Each row keeps its own replay identity, including
// when an outline has already supplied only part of the coalesced range.
export function splitTerminalOutputSequences<T extends {
  readonly bytes?: unknown
  readonly sequence?: unknown
  readonly sequence_end?: unknown
  readonly sequence_byte_lengths?: unknown
}>(record: T): T[] {
  if (record.sequence_byte_lengths === undefined) return [record]
  const { bytes, sequence, sequence_end: end, sequence_byte_lengths: lengths } = record
  if (!Array.isArray(bytes) || !Array.isArray(lengths)
    || !Number.isSafeInteger(sequence) || !Number.isSafeInteger(end)
    || (sequence as number) < 0 || (end as number) < (sequence as number)
    || lengths.length !== (end as number) - (sequence as number) + 1
    || lengths.some((length) => !Number.isSafeInteger(length) || length < 0)
    || lengths.reduce((sum, length) => sum + length, 0) !== bytes.length) {
    throw new Error("malformed terminal output sequence range")
  }
  let offset = 0
  return lengths.map((length, index) => {
    const fragment = {
      ...record,
      sequence: (sequence as number) + index,
      sequence_end: undefined,
      sequence_byte_lengths: undefined,
      sequence_fragment: true,
      bytes: bytes.slice(offset, offset + length),
    }
    offset += length
    return fragment
  })
}
