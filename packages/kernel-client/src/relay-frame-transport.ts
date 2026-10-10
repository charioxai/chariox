// MP-08 / MP-10 / MP-11: framing is socket-local and precedes decryption/correlation.
export const relayChunkBytes = 16 * 1024
export const relayMaxMessageBytes = 64 * 1024 * 1024
export function relayTransportUrl(value: string): string {
  const url = new URL(value)
  url.searchParams.set("chariox_transport", "chunks-v1")
  return url.href
}
export class RelayFrameReceiver {
  private active: { id: number; total: number; bytes: number; parts: string[] } | null = null
  private lastId = 0
  receive(text: string, receipt: (frame: string) => void): string | null {
    const frame: unknown = JSON.parse(text)
    if (!frame || typeof frame !== "object" || Array.isArray(frame)) throw Error("invalid relay frame")
    const value = frame as Record<string, unknown>
    if (value.kind === "transport_ack") throw Error("unexpected transport receipt")
    if (value.kind !== "transport_chunk") return text
    const { transfer_id: id, offset, total_bytes: total, payload } = value
    const integer = (n: unknown): n is number => typeof n === "number" && Number.isSafeInteger(n) && n >= 0 && n <= 0xffffffff
    if (!integer(id) || id === 0 || !integer(offset) || !integer(total) || total <= relayChunkBytes
        || total > relayMaxMessageBytes || typeof payload !== "string" || payload.length === 0
        || Object.keys(value).sort().join(",") !== "kind,offset,payload,total_bytes,transfer_id") throw Error("invalid transport chunk")
    const bytes = new TextEncoder().encode(payload).length
    if (bytes > relayChunkBytes) throw Error("invalid transport chunk bounds")
    if (!this.active) {
      if (offset !== 0 || id <= this.lastId) throw Error("invalid transport transfer start")
      this.active = { id, total, bytes: 0, parts: [] }
    }
    const active = this.active
    if (active.id !== id || active.total !== total || active.bytes !== offset || bytes > total - active.bytes) throw Error("non-contiguous transport chunk")
    active.parts.push(payload); active.bytes += bytes
    let completed: string | null = null
    if (active.bytes === total) {
      completed = active.parts.join("")
      const parsed: unknown = JSON.parse(completed)
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)
          || (typeof (parsed as Record<string,unknown>).kind === "string" && String((parsed as Record<string,unknown>).kind).startsWith("transport_"))) throw Error("invalid assembled relay frame")
      this.lastId = id; this.active = null
    }
    receipt(JSON.stringify({ kind: "transport_ack", transfer_id: id, offset: active.bytes }))
    return completed
  }
}
