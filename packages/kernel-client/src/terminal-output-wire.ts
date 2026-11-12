/** CXT1: u32 JSON header length, record metadata with byte ranges, raw terminal bytes. */
export function decodeTerminalOutput(bytes: Uint8Array): Record<string, unknown> | null {
  if (bytes.length < 4 || bytes[0] !== 0x43 || bytes[1] !== 0x58 || bytes[2] !== 0x54 || bytes[3] !== 0x31) return null
  const invalid = () => new Error("invalid terminal output event")
  if (bytes.length < 8) throw invalid()
  const size = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(4)
  if (size < 2 || size > 1024 * 1024 || size > bytes.length - 8) throw invalid()
  const event: unknown = JSON.parse(new TextDecoder("utf-8", {fatal: true}).decode(bytes.subarray(8, 8 + size)))
  if (!event || typeof event !== "object" || Array.isArray(event)) throw invalid()
  const output = event as Record<string, unknown>
  const records: unknown = output.event === "terminal_output" ? output.records
    : Object.keys(output).length === 1 && output.TerminalOutput
      && typeof output.TerminalOutput === "object" && !Array.isArray(output.TerminalOutput)
      ? (output.TerminalOutput as Record<string, unknown>).records : null
  if (!Array.isArray(records)) throw invalid()
  const payload = bytes.subarray(8 + size)
  let cursor = 0
  for (const record of records) {
    if (!record || typeof record !== "object" || Array.isArray(record)) throw invalid()
    const range: unknown = record.bytes
    if (!Array.isArray(range) || range.length !== 2) throw invalid()
    const [offset, length] = range
    if (offset !== cursor || !Number.isSafeInteger(length) || length < 0 || length > payload.length - cursor) throw invalid()
    record.bytes = Array.from(payload.subarray(cursor, cursor + length))
    cursor += length
  }
  if (cursor !== payload.length) throw invalid()
  return output
}
