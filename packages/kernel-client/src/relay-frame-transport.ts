// MP-08 / MP-10 / MP-11: framing is socket-local and precedes decryption/correlation.
export const relayChunkBytes = 16 * 1024
export const relayBinaryHeaderBytes = 16
export const relayBinaryPayloadBytes = relayChunkBytes - relayBinaryHeaderBytes
export const relayMaxMessageBytes = 64 * 1024 * 1024
export function relayTransportUrl(value: string): string {
  const url = new URL(value)
  url.searchParams.set("chariox_transport", "chunks-v2")
  return url.href
}
export const relayMaxTransfers = 4
export const relayMaxAssemblyBytes = 128 * 1024 * 1024
// Socket-local only: no payload inspection or request authority. A stalled
// transfer stops suppressing recovery after one second without valid bytes.
const receivers = new WeakMap<object, RelayFrameReceiver>()
export function relayResponseInProgress(socket: object | null): boolean { return !!socket && receivers.get(socket)?.receiving() === true }
export class RelayFrameReceiver {
  private progressAt = -Infinity
  constructor(socket?: object) { if (socket) receivers.set(socket, this) }
  receiving(): boolean { return (this.active.size + this.binaryActive.size) > 0 && performance.now() - this.progressAt < 1000 }
  private active = new Map<number, { total: number; bytes: number; parts: string[] }>()
  private binaryActive = new Map<number, { total: number; bytes: number; parts: Uint8Array[] }>()
  private reserved = 0
  private lastId = 0
  receiveBinary(data: ArrayBuffer, receipt: (frame: string) => void): ArrayBuffer | null {
    const bytes = new Uint8Array(data)
    if (bytes.length < 4 || bytes[0] !== 67 || bytes[1] !== 88 || bytes[2] !== 66 || bytes[3] !== 49) return data
    if (bytes.length <= relayBinaryHeaderBytes || bytes.length > relayChunkBytes) throw Error("invalid binary transport chunk bounds")
    const header = new DataView(data), id = header.getUint32(4), offset = header.getUint32(8), total = header.getUint32(12)
    const payload = bytes.subarray(relayBinaryHeaderBytes)
    // Full non-final chunks bound fragment storage and receipt pressure.
    if (!id || !total || total > relayMaxMessageBytes || offset > total || payload.length !== Math.min(relayBinaryPayloadBytes, total - offset)) throw Error("invalid binary transport chunk bounds")
    if (this.active.has(id)) throw Error("transport payload type changed")
    if (!this.binaryActive.has(id)) {
      if (offset !== 0 || id <= this.lastId || this.active.size + this.binaryActive.size >= relayMaxTransfers || total > relayMaxAssemblyBytes - this.reserved) throw Error("invalid transport transfer start")
      this.lastId = id; this.reserved += total
      this.binaryActive.set(id, {total, bytes: 0, parts: []})
    }
    const active = this.binaryActive.get(id)!
    if (active.total !== total || active.bytes !== offset) throw Error("non-contiguous transport chunk")
    active.parts.push(payload); active.bytes += payload.length
    this.progressAt = performance.now()
    let completed: ArrayBuffer | null = null
    if (active.bytes === total) {
      const assembled = new Uint8Array(total); let cursor = 0
      for (const part of active.parts) { assembled.set(part, cursor); cursor += part.length }
      if (assembled[0] === 67 && assembled[1] === 88 && assembled[2] === 66 && assembled[3] === 49) throw Error("nested transport frame")
      this.binaryActive.delete(id); this.reserved -= total
      completed = assembled.buffer
    }
    receipt(JSON.stringify({kind:"transport_ack",transfer_id:id,offset:active.bytes}))
    return completed
  }
  receive(text: string, receipt: (frame: string) => void): string | null {
    const frame: unknown = JSON.parse(text)
    if (!frame || typeof frame !== "object" || Array.isArray(frame)) throw Error("invalid relay frame")
    const value = frame as Record<string, unknown>
    if (value.kind === "transport_ack") throw Error("unexpected transport receipt")
    if (value.kind !== "transport_chunk") return text
    const { transfer_id: id, offset, total_bytes: total, payload } = value
    const integer = (n: unknown): n is number => typeof n === "number" && Number.isSafeInteger(n) && n >= 0 && n <= 0xffffffff
    if (!integer(id) || id === 0 || !integer(offset) || !integer(total)
        || total > relayMaxMessageBytes || typeof payload !== "string"
        || Object.keys(value).sort().join(",") !== "kind,offset,payload,total_bytes,transfer_id") throw Error("invalid transport chunk")
    const bytes = new TextEncoder().encode(payload).length
    if (bytes > relayChunkBytes) throw Error("invalid transport chunk bounds")
    if (total === 0) {
      if (offset !== 0 || payload !== "" || id <= this.lastId) throw Error("invalid transport checkpoint")
      this.lastId = id
      receipt(JSON.stringify({kind:"transport_ack",transfer_id:id,offset:0}))
      return null
    }
    if (bytes === 0) throw Error("invalid transport chunk bounds")
    if (this.binaryActive.has(id)) throw Error("transport payload type changed")
    if (!this.active.has(id)) {
      if (offset !== 0 || id <= this.lastId || this.active.size + this.binaryActive.size >= relayMaxTransfers
          || total > relayMaxAssemblyBytes - this.reserved) throw Error("invalid transport transfer start")
      this.lastId = id; this.reserved += total
      this.active.set(id, { total, bytes: 0, parts: [] })
    }
    const active = this.active.get(id)!
    if (active.total !== total || active.bytes !== offset || bytes > total - active.bytes) throw Error("non-contiguous transport chunk")
    // Coalesce tiny fragments so an admitted byte budget also bounds array entries.
    const last = active.parts.length - 1
    if (last >= 0 && active.parts[last]!.length < relayChunkBytes) active.parts[last] += payload
    else active.parts.push(payload)
    active.bytes += bytes
    this.progressAt = performance.now()
    let completed: string | null = null
    if (active.bytes === total) {
      completed = active.parts.join("")
      const parsed: unknown = JSON.parse(completed)
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)
          || (typeof (parsed as Record<string,unknown>).kind === "string" && String((parsed as Record<string,unknown>).kind).startsWith("transport_"))) throw Error("invalid assembled relay frame")
      this.active.delete(id); this.reserved -= total
    }
    receipt(JSON.stringify({ kind: "transport_ack", transfer_id: id, offset: active.bytes }))
    return completed
  }
}
