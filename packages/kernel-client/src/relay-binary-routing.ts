// MP-08 / MP-09 / MP-10 / MP-11: routing precedes existing authentication.
export function readCompactRoute(bytes: Uint8Array, offset: number, size: number): { id: string; end: number } {
  const length = size === 0 ? 16 : size
  if (size > 1024 || offset + length > bytes.length) throw Error("invalid compact routing bounds")
  const raw = bytes.subarray(offset, offset + length)
  const hex = size === 0 ? Array.from(raw, b => b.toString(16).padStart(2, "0")).join("") : ""
  const id = size === 0 ? `${hex.slice(0,8)}-${hex.slice(8,12)}-${hex.slice(12,16)}-${hex.slice(16,20)}-${hex.slice(20)}`
    : new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(raw)
  return { id, end: offset + length }
}

export function decodeCompactRelayRouting<T extends ArrayBufferLike>(bytes: Uint8Array<T>, event: boolean): {
  id: string; eventId: number; sender_public_key: string; nonce: string; ciphertext: Uint8Array<T>
} {
  const invalid = () => Error("invalid binary relay routing bounds")
  const start = event ? 92 : 84, keyStart = event ? 15 : 7
  if (bytes.length < start || bytes[0] !== 67 || bytes[1] !== 88 || bytes[2] !== 82 || bytes[3] !== (event ? 49 : 50)
      || bytes[4] !== 1 || bytes[keyStart] !== 4 || bytes.length > 64 * 1024 * 1024) throw invalid()
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const sequence = event ? view.getBigUint64(7) : 0n
  if (sequence > BigInt(Number.MAX_SAFE_INTEGER)) throw invalid()
  const { id, end } = readCompactRoute(bytes, start, view.getUint16(5))
  if (bytes.length - end < 16 || (event && bytes.length - end > 1024 * 1024)) throw invalid()
  const base64 = (raw: Uint8Array) => btoa(String.fromCharCode(...raw))
  return { id, eventId: Number(sequence), sender_public_key: base64(bytes.subarray(keyStart, keyStart + 65)),
    nonce: base64(bytes.subarray(keyStart + 65, start)), ciphertext: bytes.subarray(end) }
}
