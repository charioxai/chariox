// MP-08 / MP-10 / MP-11: reassemble one bounded, receipted official frame.
import { gunzipSync } from "node:zlib"
import type { NativeProviderTransportResult } from "@chariox/kernel-client/kernel-types"

const frameBytes = 16 * 1024 * 1024
const pageBytes = 64 * 1024
const encodedPageBytes = Math.ceil(pageBytes / 3) * 4

export function createCodexNativeOutputAssembler() {
  let pending: { total: number; decoded: number; offset: number; pages: Buffer[] } | null = null
  return (result: NativeProviderTransportResult): readonly string[] => {
    const frames = [...result.frames]
    const fragment = result.gzip_fragment
    if (fragment) {
      if (![fragment.offset, fragment.total_bytes, fragment.decoded_bytes].every(Number.isSafeInteger)
        || fragment.offset < 0 || fragment.total_bytes <= 0 || fragment.total_bytes > frameBytes + pageBytes
        || fragment.decoded_bytes <= 0 || fragment.decoded_bytes > frameBytes
        || typeof fragment.data !== "string" || fragment.data.length > encodedPageBytes) {
        throw new Error("native output fragment exceeds its byte bound")
      }
      const page = Buffer.from(fragment.data, "base64")
      if (!page.length || page.length > pageBytes || page.toString("base64") !== fragment.data
        || fragment.offset + page.length > fragment.total_bytes) {
        throw new Error("native output fragment has invalid encoding or bounds")
      }
      if (!pending) pending = { total: fragment.total_bytes, decoded: fragment.decoded_bytes, offset: 0, pages: [] }
      if (fragment.offset !== pending.offset || fragment.total_bytes !== pending.total || fragment.decoded_bytes !== pending.decoded) {
        throw new Error("native output fragments are not contiguous")
      }
      pending.pages.push(page)
      pending.offset += page.length
      if (pending.offset === pending.total) {
        let decoded: Buffer
        try { decoded = gunzipSync(Buffer.concat(pending.pages), { maxOutputLength: pending.decoded }) }
        catch { throw new Error("native output gzip frame is invalid or exceeds its decoded bound") }
        if (decoded.length !== pending.decoded) throw new Error("native output decoded size changed")
        try { frames.push(new TextDecoder("utf-8", { fatal: true }).decode(decoded)) }
        catch { throw new Error("native output frame is not UTF-8") }
        pending = null
      }
    }
    if (result.closed && pending) throw new Error("native output closed before its last fragment")
    return frames
  }
}
