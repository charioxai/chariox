// MP-08/MP-10: presentation hints only; these reads never admit input or pixels.
const empty = () => ({ can_go_back: null, can_go_forward: null, favicon_png_base64: null });
export class BrowserHeaderState {
  constructor(changed) { this.changed = changed; this.tabs = new Map(); }
  clear() { this.tabs.clear(); }
  read(tab) { return this.tabs.get(tab.tab_id)?.value ?? empty(); }
  history(tabId) { return this.tabs.get(tabId)?.history; }
  commit(id, entry, patch) {
    if (this.tabs.get(id) !== entry) return;
    const changed = Object.entries(patch).some(([key, value]) => entry.value[key] !== value);
    Object.assign(entry.value, patch); if (changed) this.changed();
  }
  update(tabs, active, browser, groups) {
    const live = new Set(tabs.map(tab => tab.tab_id));
    for (const id of this.tabs.keys()) if (!live.has(id)) this.tabs.delete(id);
    if (!browser) return;
    for (const tab of tabs) {
      const key = JSON.stringify([tab.target_id, tab.document_id, tab.url]);
      let entry = this.tabs.get(tab.tab_id);
      if (entry?.key !== key) { entry = { key, checked: -Infinity, pending: false, value: empty() }; this.tabs.set(tab.tab_id, entry); }
      if (entry.pending || performance.now() - entry.checked < (tab.tab_id === active ? 250 : 5000)) continue;
      entry.pending = true; entry.checked = performance.now();
      // History is independent of a slow icon. Retired documents discard late
      // metadata; the page and its input never wait for an icon download.
      entry.history = Promise.resolve().then(() => browser.resolvePageTarget(tab.target_id)).then(({ connection, sessionId }) => connection.send('Page.getNavigationHistory', {}, sessionId)).then(h => {
        const known = Array.isArray(h?.entries) && Number.isInteger(h.currentIndex) && h.currentIndex >= 0 && h.currentIndex < h.entries.length;
        this.commit(tab.tab_id, entry, { can_go_back: known ? h.currentIndex > 0 : null, can_go_forward: known ? h.currentIndex + 1 < h.entries.length : null });
      }, () => this.commit(tab.tab_id, entry, { can_go_back: null, can_go_forward: null }));
      const icon = Promise.resolve().then(() => groups?.favicon?.(tab.target_id) ?? null).then(value => this.commit(tab.tab_id, entry, { favicon_png_base64: value }), () => {});
      void Promise.allSettled([entry.history, icon]).finally(() => { entry.pending = false; });
    }
  }
}
