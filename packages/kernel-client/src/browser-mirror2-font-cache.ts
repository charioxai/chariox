// MIRROR-P2: subscription-local font retention. A font the viewer already holds is named by its digest (the first 24
// hex characters of the resource id) when a later document uses it, so the bytes of a site's fonts cross the link once.
// Like the sheet dictionary an advertised entry never leaves it before close; later fonts past the bounds use ordinary delivery.
const MAX_BYTES = 16 * 1024 * 1024, MAX_FONTS = 64
export class Mirror2FontCache {
  private readonly blobs = new Map<string, Blob>()
  private bytes = 0
  remember(resourceId: string, blob: Blob): void {
    const digest = resourceId.slice(0, 24)
    if (this.blobs.has(digest) || this.blobs.size >= MAX_FONTS || blob.size > MAX_BYTES - this.bytes) return
    this.blobs.set(digest, blob); this.bytes += blob.size
  }
  take(resourceId: string): Blob | undefined { return this.blobs.get(resourceId.slice(0, 24)) }
  digests(): string[] { return [...this.blobs.keys()] }
  // Before a new subscription of the same view lists the cache, the oldest fonts make room for the next page's.
  makeRoom(): void {
    for (const [digest, blob] of this.blobs) {
      if (this.blobs.size <= MAX_FONTS / 2 && this.bytes <= MAX_BYTES / 2) break
      this.blobs.delete(digest); this.bytes -= blob.size
    }
  }
  clear(): void { this.blobs.clear(); this.bytes = 0 }
}
