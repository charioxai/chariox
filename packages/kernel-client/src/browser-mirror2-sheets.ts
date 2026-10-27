// MP-08/MP-10/MP-11: subscription-local immutable sheet retention. Advertised
// digests never leave this dictionary before close, including across credits
// and resets in flight. New sheets fall back to ordinary per-document delivery.
const MAX_BYTES = 8 * 1024 * 1024, MAX_SHEETS = 128
export class Mirror2SheetCache {
  readonly texts = new Map<string, string>()
  private bytes = 0
  remember(sheets: Record<string, string> | undefined): void {
    for (const [digest, text] of Object.entries(sheets ?? {})) {
      if (this.texts.has(digest) || this.texts.size >= MAX_SHEETS) continue
      const bytes = new TextEncoder().encode(text).length
      if (bytes > MAX_BYTES - this.bytes) continue
      this.texts.set(digest, text); this.bytes += bytes
    }
  }
  digests(): string[] { return [...this.texts.keys()] }
  clear(): void { this.texts.clear(); this.bytes = 0 }
}
