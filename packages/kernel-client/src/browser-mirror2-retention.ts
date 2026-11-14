// MIRROR-P2: what a Browser view keeps across the Mirror subscriptions of its navigations. The page runtime ends a
// subscription when its document is retired and attaches a new one, so a cache owned by the renderer died with each page.
import { Mirror2SheetCache } from './browser-mirror2-sheets.js'
import { Mirror2FontCache } from './browser-mirror2-font-cache.js'
export class Mirror2Retention {
  readonly sheets = new Mirror2SheetCache()
  readonly fonts = new Mirror2FontCache()
  /** A new subscription is about to list the caches: free room for what its pages teach. */
  begin(): void { this.sheets.makeRoom(); this.fonts.makeRoom() }
  clear(): void { this.sheets.clear(); this.fonts.clear() }
}
