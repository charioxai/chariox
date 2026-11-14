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
  const payload = bytes.subarray(8 + size)
  if (Object.keys(output).length === 1 && output.RoomTerminal) {
    const room = output.RoomTerminal as { result?: { result?: string; output?: Record<string, unknown>; snapshot?: Record<string, unknown> } }
    const result = room.result
    const body = result?.result === "output" ? result.output : result?.result === "snapshot" ? result.snapshot : null
    if (!body || typeof body !== "object" || Array.isArray(body)) throw invalid()
    const fields: readonly (readonly [string, "base64" | "text"])[] = result!.result === "output"
      ? [["data_base64", "base64"]]
      : [["screen_ansi_base64", "base64"], ["screen_text", "text"], ["scrollback_text", "text"]]
    let cursor = 0
    for (const [key, encoding] of fields) {
      const range = body[key]
      if (!Array.isArray(range) || range.length !== 2) throw invalid()
      const [offset, length] = range
      if (offset !== cursor || !Number.isSafeInteger(length) || length < 0 || length > payload.length - cursor) throw invalid()
      const data = payload.subarray(cursor, cursor + length)
      if (encoding === "text") body[key] = new TextDecoder("utf-8", { fatal: true }).decode(data)
      else {
        let binary = ""
        for (let index = 0; index < data.length; index += 16384) binary += String.fromCharCode(...data.subarray(index, index + 16384))
        body[key] = btoa(binary)
      }
      cursor += length
    }
    if (cursor !== payload.length) throw invalid()
    return output
  }
  // A Room terminal stream page keeps its bytes raw: its one reader draws them as they are.
  if (output.event === "room_terminal_output") {
    const range = output.bytes
    if (!Array.isArray(range) || range.length !== 2 || range[0] !== 0 || range[1] !== payload.length) throw invalid()
    output.bytes = new Uint8Array(payload)
    return output
  }
  const records: unknown = output.event === "terminal_output" ? output.records
    : Object.keys(output).length === 1 && output.TerminalOutput
      && typeof output.TerminalOutput === "object" && !Array.isArray(output.TerminalOutput)
      ? (output.TerminalOutput as Record<string, unknown>).records : null
  if (!Array.isArray(records)) throw invalid()
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

/** The JSON text of a decoded event: the raw bytes of a pushed Room terminal page become the number array that terminal output records use. */
export function terminalOutputJson(event: Record<string, unknown>): string {
  return JSON.stringify(event, (_key, value) => (value instanceof Uint8Array ? Array.from(value) : value))
}
