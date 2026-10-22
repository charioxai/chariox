// MP-08/MP-10/MP-11: the owned Chromium window shows one tab at a time.
// A native source may read it only while this host last brought its tab to
// front. Every host foreground change (new tab, closed tab, restart) retires
// the claim before Chromium can show another tab; popups fence by CDP event.
export class WindowForeground {
  constructor() { this.tab = null; this.epoch = 0; }
  // Call before Page.bringToFront. Returns the claim bound into the source.
  claim(tabId) { this.epoch++; this.tab = tabId; return this.epoch; }
  holds(tabId, claim) { return this.tab === tabId && this.epoch === claim; }
  reset() { this.epoch++; this.tab = null; }
}
