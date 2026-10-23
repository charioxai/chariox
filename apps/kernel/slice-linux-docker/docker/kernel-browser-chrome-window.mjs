// MP-08/MP-10/MP-11 (INTEGRATION.md §2b): kernel-private binding of the kernel
// Chromium's own windows. A viewer never names a window, rect, PID or role; the
// kernel resolves them from CDP window bounds and the owned display's numeric
// inventory, and every consumer re-checks isCurrent() before readback or input.
import {nativePageRect} from './kernel-browser-page-geometry.mjs';
import {ownsDisplay} from './kernel-browser-owned-display.mjs';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
import {keysyms, notches} from './kernel-browser-input.mjs';
import {markDispatched} from './kernel-browser-input-queue.mjs';
import {slowMark} from './kernel-browser-timing.mjs';
import {retireNativeStates} from './kernel-browser-native-worker.mjs';

// MP-08/MP-11 (coordinator 12:22Z): Chrome 155 draws the omnibox dropdown INSIDE its main window over
// the page (no X window). Its WebUI reports the popup widget (screen DIP) and its list's size. On the
// pinned Chrome the list body is the widget less a 16 DIP side and 28 DIP bottom shadow, with 16 DIP
// rounded bottom corners (measured at DPR 1 and 2). The part below the band streams as one more popup;
// any other layout is not offered (the band still shows the top of it).
export const OMNIBOX_POPUP_URL = 'chrome://omnibox-popup.top-chrome/';
const POPUP_SIDE = 16, POPUP_BOTTOM = 28;
export const POPUP_CORNER = 16;
/** Chrome 155's suggestion list holds at most 8 rows of 40 DIP in an 8 DIP frame (observed 48..288). */
export const OMNIBOX_LIST_HEIGHT = 344;
/** Protocol 540: an address bar without the person's input for this long no longer keeps its list stream warm. */
const OMNIBOX_WARM_MS = 30_000;
const omniboxSeenKey = t => `${t.browser_generation}:${t.xid}:${t.bounds.width}x${t.bounds.height}@${t.scale}:${JSON.stringify(t.page_rect)}`;
/** A state read right after an address-bar key waits this long for the list that key opens. */
const OMNIBOX_LIST_WAIT_MS = 250;
/** A listed popup is trusted this long after the last inventory read (a live band reads every 150 ms). */
const STALE_ROWS_MS = 500;
/** Raster rects outside the list's two rounded bottom corners (radius r px), one per row. */
export function omniboxPopupRect(state, windowLeft, windowTop, bandHeight, windowWidth, windowHeight) {
  if (!Array.isArray(state) || state[0] !== 'visible') return null;
  const [, screenX, screenY, width, height, outerWidth, outerHeight] = state;
  if (![screenX, screenY, width, height, outerWidth, outerHeight].every(Number.isSafeInteger) || outerWidth - width !== 2 * POPUP_SIDE || height < 2) return null;
  const x = screenX - windowLeft + POPUP_SIDE, bottom = screenY - windowTop + outerHeight - POPUP_BOTTOM;
  if (x < 0 || x + width > windowWidth || bottom > windowHeight || bottom - bandHeight < POPUP_CORNER + 2 || screenY - windowTop >= bandHeight) return null;
  return {x, y: bandHeight, width, height: bottom - bandHeight};
}

// MP-08/MP-11 (coordinator 12:22Z): a browser-modal Chrome dialog (the Web Store's 'Add "…"?') draws a
// scrim over the whole window, band included, and hangs from the toolbar: no X window, no CDP event.
// The band tells: its tab strip row and its toolbar row just above the dialog, each a colour learned
// while clear, both dimmed (real Chrome 155: 255 -> 102; the omnibox dropdown never covers the strip).
// While it holds, the page area streams natively as one more popup (Vault-masked, attested).
export const SCRIM_ROWS = [2, 81]; // DIP from the window top: the strip above its Tabs, the toolbar's bottom
export function rowMode(bgr0) {
  const counts = new Map();
  for (let i = 0; i + 3 < bgr0.length; i += 4) { const key = bgr0[i] | bgr0[i + 1] << 8 | bgr0[i + 2] << 16; counts.set(key, (counts.get(key) ?? 0) + 1); }
  let mode = 0, top = 0; for (const [key, n] of counts) if (n > top) { mode = key; top = n; }
  return {luminance: (mode & 255) + (mode >> 8 & 255) + (mode >> 16 & 255), share: top / Math.max(1, bgr0.length / 4)};
}

// The owned display's numeric window inventory comes from the native producer
// (vidlat nativeWindowInventory, wired with the 519 region step); without it no binding.
const slowStep = (host, step, operation) => slowMark(host.timing, `band_key_wait ${step}`, operation);
const noInventory = async () => { throw new UserDomainRefusal('stale_reference'); };

/** The inventory poll is a backstop: the worker announces its changes. */
const INVENTORY_BACKSTOP_MS = 1000;
export class ChromeWindows {
  constructor(host, {inventory = noInventory} = {}) {
    this.host = host; this.inventory = inventory;
    this.windowGeneration = 0; this.pageEpoch = 0; this.omniboxSeen = new Map(); this.bound = null; this.listeners = new Set();
    this.auxiliaryIds = new Map(); this.auxiliarySequence = 0;
  }
  onRetire(listener) { this.listeners.add(listener); return () => this.listeners.delete(listener); }
  /** Native UI input invalidates the observation before dispatch and after its acknowledgement. */
  uiInput() {
    this.occlusionRevision = (this.occlusionRevision ?? 0) + 1;
    this.uiInputs = (this.uiInputs ?? 0) + 1;
    let finished = false;
    return () => { if (!finished) { finished = true; this.uiInputs--; this.occlusionRevision++; } };
  }
  retire(reason) {
    if (!this.bound) return;
    retireNativeStates();
    const {window_generation} = this.bound.tabbed; this.bound = null;
    this.host.timing?.(`chrome_window_retired ${reason}`, performance.timeOrigin + performance.now());
    for (const listener of this.listeners) listener({window_generation, reason});
    this.host.stateSignal?.notify();
  }
  /** The page rect moved under an unchanged band: page-bound bindings go stale, the band keeps its generation. */
  retirePage() {
    if (!this.bound) return;
    retireNativeStates();
    const {window_generation} = this.bound.tabbed; this.pageEpoch++;
    this.host.timing?.('chrome_window_page_retired', performance.timeOrigin + performance.now());
    for (const listener of this.listeners) listener({window_generation, reason: 'page_geometry_changed'});
  }
  current() {
    if (!this.bound || !this.bound.tabbed.isCurrent()) throw new UserDomainRefusal('stale_reference');
    return this.bound;
  }
  /** Resolve the owned window set serially on the host mutation lane. */
  resolve() {
    const run = (this.resolving ?? Promise.resolve()).catch(() => {}).then(() => this.resolveNow());
    this.resolving = run;
    return run;
  }
  async resolveNow() {
    const host = this.host, chromium = host.chromium, display = chromium.display, pid = chromium.browserPid ?? chromium.child?.pid, scale = chromium.scale;
    const browserGeneration = host.generation;
    const live = () => host.generation === browserGeneration && ownsDisplay(display) && (chromium.browserPid ?? chromium.child?.pid) === pid;
    if (!live() || !Number.isSafeInteger(pid) || pid <= 1) throw new UserDomainRefusal('stale_reference');
    const connection = host.browser.connection;
    const viewport = await host.tabGroups?.viewport();
    const page = [...host.tabs.values()].find(tab => tab.target_id === viewport?.target_id);
    if (!page) { this.retire('last_tab_closed'); throw new UserDomainRefusal('stale_reference'); }
    const {bounds} = await connection.send('Browser.getWindowForTarget', {targetId: page.target_id});
    // Fullscreen/maximized windows hide or resize the band; only the launch geometry binds.
    if (bounds?.windowState !== 'normal') throw new UserDomainRefusal('stale_reference');
    const rows = await this.inventory(display, pid, live);
    this.rows = rows;
    const physical = {x: bounds.left * scale, y: bounds.top * scale, width: bounds.width * scale, height: bounds.height * scale};
    const tabbed = rows.filter(r => r.visible && !r.override_redirect && r.transient_for === 0 &&
      r.x === physical.x && r.y === physical.y && r.width === physical.width && r.height === physical.height);
    // Exactly one match or no binding: never the "closest" window.
    if (tabbed.length !== 1 || !live()) throw new UserDomainRefusal('stale_reference');
    const row = tabbed[0];
    const measure = async () => {
      const next = await host.tabGroups?.viewport();
      if (!next || ![...host.tabs.values()].some(tab => tab.target_id === next.target_id)) throw new UserDomainRefusal('stale_reference');
      return {rect:nativePageRect(row, next, scale),target_id:next.target_id};
    };
    const pageRect = nativePageRect(row, viewport, scale);
    // The page origin remains exact, including odd rows and GTK side insets.
    // Only the disjoint Chrome video band rounds down to even codec rows.
    const contentTop = Math.floor(pageRect.top / 2) * 2;
    if (!(contentTop > 0 && contentTop < row.height)) throw new UserDomainRefusal('stale_reference');
    const previous = this.bound?.tabbed;
    // The band is the window's top `content_top` rows: a page-only change (insets, width) leaves its identity
    // and generation alone and retires only what is bound to the page rect (r54).
    const sameBand = previous && previous.xid === row.id && previous.bounds.width === row.width && previous.bounds.height === row.height && previous.content_top === contentTop && previous.browser_generation === browserGeneration;
    if (!sameBand) { if (previous) this.retire('window_changed'); this.windowGeneration++; }
    else if (previous.isCurrent() && JSON.stringify(previous.page_rect) !== JSON.stringify(pageRect)) this.retirePage();
    const windowGeneration = this.windowGeneration, pageEpoch = this.pageEpoch;
    const isBandCurrent = () => live() && this.windowGeneration === windowGeneration && this.bound?.tabbed.xid === row.id;
    const isCurrent = () => isBandCurrent() && this.pageEpoch === pageEpoch;
    // MP-08/MP-10/MP-11: identity and geometry share one fresh native query.
    const isPageCurrent = async target_id => {
      if (!isCurrent()) return false;
      try { const next=await measure();if (JSON.stringify(next.rect) === JSON.stringify(pageRect)) return (target_id===undefined||next.target_id===target_id)&&isCurrent(); } catch {}
      // Rebind at the real geometry: no other path may notice (r13 union3: the band never came back).
      if (isCurrent()) { this.retirePage(); this.resolve().catch(() => {}); }
      return false;
    };
    // MP-11: source publication still calls the fresh check above. Delivery
    // can reuse its native snapshot until the extension/resize observer retires it.
    const isPageCurrentCached=host.tabGroups?.nativeStateSupported?.()===true?target_id=>{
      if(!isCurrent())return false;
      const cached=host.tabGroups.cachedViewport();
      if(!cached||(target_id!==undefined&&cached.target_id!==target_id))return false;
      try{return JSON.stringify(nativePageRect(row,cached,scale))===JSON.stringify(pageRect)&&isCurrent();}catch{return false;}
    }:undefined;
    // MP-11: source publication reuses the native owner snapshot only while no extension, resize or
    // detach event has retired it (same window generation, same Tab, same page rect); a miss reads fresh.
    const isPagePublishable=isPageCurrentCached?async target_id=>isPageCurrentCached(target_id)||isPageCurrent(target_id):isPageCurrent;
    const binding = role => r => Object.freeze({browser_generation: browserGeneration, window_generation: windowGeneration, role, xid: r.id,
      parent_xid: role === 'auxiliary' ? row.id : null, bounds: Object.freeze({x: r.x, y: r.y, width: r.width, height: r.height}),
      content_top: role === 'tabbed' ? contentTop : null, page_rect: role === 'tabbed' ? pageRect : null,
      scale, pid, isCurrent: role === 'tabbed' ? isCurrent : isBandCurrent, isBandCurrent, isPageCurrent,isPageCurrentCached,isPagePublishable});
    const tabbedBinding = binding('tabbed')(row);
    this.bound = Object.freeze({tabbed: tabbedBinding, auxiliary: this.auxiliaryOf(tabbedBinding, rows, binding('auxiliary'))});
    return this.bound;
  }
  // Menus, the omnibox dropdown, extension popups and bubbles: same PID (the inventory's
  // filter), visible, transient for or anchored inside the tabbed window. Protocol 530: each
  // gets a kernel-issued id, new whenever its window or geometry changes (fixed-size stream).
  auxiliaryOf(tabbed, rows, bind) {
    const row = {id: tabbed.xid, ...tabbed.bounds}, ids = new Map();
    const auxiliary = rows.filter(r => r.id !== row.id && r.visible && r.width >= 2 && r.height >= 2 && (r.transient_for === row.id ||
      r.override_redirect && r.x >= row.x && r.x < row.x + row.width && r.y >= row.y && r.y < row.y + row.height));
    const bound = auxiliary.slice(0, 8).map(r => {
      const key = `${tabbed.window_generation}:${r.id}:${r.x},${r.y},${r.width}x${r.height}`;
      const auxiliary_id = this.auxiliaryIds.get(key) ?? ++this.auxiliarySequence;
      ids.set(key, auxiliary_id);
      const binding = bind(r);
      return Object.freeze({...binding, auxiliary_id, isCurrent: () => binding.isCurrent() && this.bound?.auxiliary.some(a => a.auxiliary_id === auxiliary_id) === true});
    });
    // A browser-modal dialog over the page: the whole page area of the same X window.
    if (this.modal && bound.length < 8) {
      // Exactly the native page area (GTK frame insets and Chrome's own Tab size), even-sized.
      const page = tabbed.page_rect, r = {id: row.id, x: row.x + page.left, y: row.y + page.top, width: page.width - page.width % 2, height: page.height - page.height % 2};
      const key = `${tabbed.window_generation}:${r.id}:modal:${r.x},${r.y},${r.width}x${r.height}`;
      const auxiliary_id = this.auxiliaryIds.get(key) ?? ++this.auxiliarySequence;
      ids.set(key, auxiliary_id);
      const binding = bind(r);
      bound.push(Object.freeze({...binding, in_window: true, page_overlay: true, auxiliary_id, isCurrent: () => binding.isCurrent() && this.bound?.auxiliary.some(a => a.auxiliary_id === auxiliary_id) === true}));
    }
    // The omnibox dropdown drawn in the window: its rect of the same X window (physical px).
    const live = omniboxPopupRect(this.omnibox, tabbed.bounds.x / tabbed.scale, tabbed.bounds.y / tabbed.scale, tabbed.page_rect.top / tabbed.scale,
      tabbed.bounds.width / tabbed.scale, tabbed.bounds.height / tabbed.scale), last = this.omniboxRect(tabbed);
    // Chrome resizes the list on almost every key and each new stream left the person without a list
    // for 0.4-1.3 s (r10, r11 profile): the stream opens at the list's full height and keeps it while the
    // list stays open. Below the live list it shows the page (its masks apply); input there is refused.
    const full = live && {...live, height: Math.max(live.height, Math.min(OMNIBOX_LIST_HEIGHT, Math.floor((tabbed.bounds.height / tabbed.scale - live.y) / 2) * 2))};
    // Protocol 540: while the address bar has the person's keys the last list's stream stays (nothing
    // visible), so the next list paints from a running stream instead of a new one (r12: ~0.9 s).
    const popup = live ? (last && last.x === live.x && last.y === live.y && last.width === live.width && live.height <= last.height ? last : full)
      : this.omniboxWarm() ? last : null;
    if (popup) { this.omniboxLast = {...popup, window_generation: tabbed.window_generation}; this.noteOmnibox(tabbed, popup); }
    this.omniboxLive = live;
    if (popup && bound.length < 8) {
      const scale = tabbed.scale, r = {id: row.id, x: row.x + popup.x * scale, y: row.y + popup.y * scale, width: popup.width * scale, height: popup.height * scale};
      const key = `${tabbed.window_generation}:${r.id}:in-window:${r.x},${r.y},${r.width}x${r.height}`;
      const auxiliary_id = this.auxiliaryIds.get(key) ?? ++this.auxiliarySequence;
      ids.set(key, auxiliary_id);
      const binding = bind(r);
      bound.push(Object.freeze({...binding, in_window: true, omnibox: true, auxiliary_id, isCurrent: () => binding.isCurrent() && this.bound?.auxiliary.some(a => a.auxiliary_id === auxiliary_id) === true}));
    }
    this.auxiliaryIds = ids;
    return bound;
  }
  /** A state read with no binding retries one, at most once a second: a missed rebind never strands the band. */
  async heal() {
    if (this.bound) return;
    const now = performance.now();
    if (now - (this.healedAt ?? -Infinity) < 1000) return;
    this.healedAt = now;
    await this.resolve().catch(() => {});
  }
  /** The last list rect of this window (window CSS px), or null. A window that is rebound at the same X
   * window, size, scale, page boundary and Chrome process (a native capture fit and back) keeps the list rect Chrome showed
   * there: it is the same proven rect, so the first list after the rebind paints from a warm stream. */
  omniboxRect(tabbed = this.bound?.tabbed) {
    if (!tabbed) return null;
    return this.omniboxLast?.window_generation === tabbed.window_generation ? this.omniboxLast : this.omniboxSeen.get(omniboxSeenKey(tabbed)) ?? null;
  }
  noteOmnibox(tabbed, {x, y, width, height}) {
    const key = omniboxSeenKey(tabbed);
    this.omniboxSeen.delete(key); this.omniboxSeen.set(key, {x, y, width, height});
    if (this.omniboxSeen.size > 4) this.omniboxSeen.delete(this.omniboxSeen.keys().next().value);
  }
  /** Protocol 540: the address bar has (`on`) or lost the person's keys; `ago` backdates it (ms). */
  focusOmnibox(on, ago = 0) {
    const was = this.omniboxWarm();
    this.omniboxFocusAt = on ? performance.now() + ago : null;
    if (was !== this.omniboxWarm()) this.refreshAuxiliary(this.rows);
  }
  omniboxWarm() { return this.omniboxFocusAt != null && performance.now() - this.omniboxFocusAt < OMNIBOX_WARM_MS; }
  /** A state read while the address bar is warm sees a list Chrome opened since the last poll; right
   * after an address-bar key it waits briefly for the list that key opens (else the next 250 ms poll). */
  async refreshWarm() {
    if (!this.omniboxWarm() || !this.bound?.tabbed.isCurrent()) return;
    for (;;) {
      this.omnibox = await this.readOmnibox(); this.refreshAuxiliary(this.rows);
      if (this.omniboxLive || !this.omniboxWarm() || performance.now() - this.omniboxFocusAt > OMNIBOX_LIST_WAIT_MS) return;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
  }
  /** A band that cannot read (retired, re-subscribing) leaves a closed menu listed for seconds (r17, accbrowser r1): a state read
   * then re-reads the owner's own inventory, one read at a time, only while a popup is listed and no read is recent. */
  async refreshStale() {
    if (!this.bound?.auxiliary.length || !this.bound.tabbed.isCurrent() || performance.now() - (this.rowsReadAt ?? -Infinity) < STALE_ROWS_MS) return;
    this.staleRead ??= this.readOwnerRows().catch(() => {}).finally(() => { this.staleRead = null; });
    await this.staleRead;
  }
  async readOwnerRows() {
    const host = this.host, chromium = host.chromium, display = chromium?.display, pid = chromium?.browserPid ?? chromium?.child?.pid, generation = host.generation;
    const live = () => host.generation === generation && ownsDisplay(display) && (chromium.browserPid ?? chromium.child?.pid) === pid;
    if (!live() || !Number.isSafeInteger(pid) || pid <= 1) return;
    const rows = await this.inventory(display, pid, live), bound = this.bound;
    if (!live() || !bound?.tabbed.isCurrent()) return;
    this.rowsReadAt = performance.now();
    // Retire only: a popup whose own X window is gone leaves the list and the cached inventory (a later refresh must not
    // bring it back); nothing is rebound here; a parked state read is woken.
    const open = new Set(rows.filter(r => r.visible).map(r => r.id)), kept = bound.auxiliary.filter(a => a.in_window || open.has(a.xid));
    if (kept.length === bound.auxiliary.length) return;
    const gone = new Set(bound.auxiliary.filter(a => !kept.includes(a)).map(a => a.xid));
    this.rows = this.rows?.filter(r => !gone.has(r.id));
    this.bound = Object.freeze({tabbed: bound.tabbed, auxiliary: kept});
    this.auxiliaryCount = kept.length; this.host.timing?.(`aux_count ${kept.length}`, performance.timeOrigin + performance.now());
    this.notifyPopups();
  }
  /** Is Chrome's omnibox list shown right now (re-read, not the last poll)? */
  async omniboxShown() {
    this.omnibox = await this.readOmnibox();
    this.refreshAuxiliary(this.rows);
    return !!this.omniboxLive;
  }
  /** Is window CSS x over the address bar (the last list's span)? null before a first list. */
  onOmnibox(x) { const last = this.omniboxRect(); return last ? x >= last.x && x < last.x + last.width : null; }
  /** A band frame (Chrome's own UI): is the toolbar row under a browser-modal scrim? */
  observeBand(raw) {
    const tabbed = this.bound?.tabbed;
    if (!tabbed?.isCurrent() || !raw || raw.width !== tabbed.bounds.width || raw.height < SCRIM_ROWS[1] * tabbed.scale) return;
    const modes = [];
    for (const dip of SCRIM_ROWS) {
      const y = dip * tabbed.scale;
      try { modes.push(rowMode(typeof raw.readRegion === 'function' ? raw.readRegion(0, y, raw.width, 1) : raw.pixels.subarray(y * raw.width * 4, (y + 1) * raw.width * 4))); } catch { return; }
    }
    if (modes.some(mode => mode.share < 0.6)) return;
    this.clear ??= SCRIM_ROWS.map(() => 0);
    if (!this.modal) modes.forEach((mode, i) => { this.clear[i] = Math.max(this.clear[i], mode.luminance); });
    const modal = modes.every((mode, i) => this.clear[i] >= 384 && mode.luminance <= this.clear[i] * 0.75 && mode.luminance >= this.clear[i] * 0.2);
    if (modal !== !!this.modal) { this.modal = modal; this.refreshAuxiliary(this.rows); }
  }
  /** The page overlay's attestation found no scrim (e.g. a darker theme): learn the band again. */
  relearnToolbar() { this.clear = null; if (this.modal) { this.modal = false; this.refreshAuxiliary(this.rows); } }
  /** The omnibox dropdown's WebUI: visibility and geometry only (never its content). */
  async readOmnibox() {
    const connection = this.host.browser.connection;
    if (!connection) return null;
    try {
      if (!this.omniboxSession) {
        const target = (await connection.send('Target.getTargets')).targetInfos.find(t => t.type === 'browser_ui' && t.url === OMNIBOX_POPUP_URL);
        if (!target) return null;
        this.omniboxSession = (await connection.send('Target.attachToTarget', {targetId: target.targetId, flatten: true})).sessionId;
      }
      const {result} = await connection.send('Runtime.evaluate', {expression: 'JSON.stringify([document.visibilityState,screenX,screenY,innerWidth,innerHeight,outerWidth,outerHeight])', returnByValue: true}, this.omniboxSession);
      return JSON.parse(result.value);
    } catch { this.omniboxSession = null; return null; }
  }
  /** Protocol 530: refresh the auxiliary windows of the current tabbed binding (a live band polls
   * its own worker's inventory). A moved or resized tabbed window (e.g. fitted for a native page
   * capture) retires the band at once and is rebound at its real geometry. */
  refreshAuxiliary(rows) {
    const bound = this.bound, tabbed = bound?.tabbed;
    if (!tabbed?.isCurrent() || !Array.isArray(rows)) return;
    this.rows = rows;
    const row = rows.find(r => r.id === tabbed.xid);
    if (!row) return;
    if (row.x !== tabbed.bounds.x || row.y !== tabbed.bounds.y || row.width !== tabbed.bounds.width || row.height !== tabbed.bounds.height) {
      this.retire('window_changed'); this.resolve().catch(() => {}); return;
    }
    const bind = r => Object.freeze({...tabbed, role: 'auxiliary', xid: r.id, parent_xid: tabbed.xid, content_top: null,
      bounds: Object.freeze({x: r.x, y: r.y, width: r.width, height: r.height})});
    this.bound = Object.freeze({tabbed, auxiliary: this.auxiliaryOf(tabbed, rows, bind)});
    if (this.bound.auxiliary.length !== this.auxiliaryCount) { this.auxiliaryCount = this.bound.auxiliary.length; this.host.timing?.(`aux_count ${this.auxiliaryCount}`, performance.timeOrigin + performance.now()); }
    this.notifyPopups();
  }
  /** Protocol 556: a parked state read wakes when the popups it reports (menu, omnibox list) change. */
  notifyPopups() {
    const popups = JSON.stringify(this.auxiliaryState());
    if (popups !== this.popupsReported) { this.popupsReported = popups; this.host.stateSignal?.notify(); }
  }
  /** Review #1016 P1: an in-window popup closes without an X event, its rect then being the page
   * itself. Input on it re-reads Chrome's dropdown now; a closed or changed one retires at once. */
  async inWindowCurrent(source, point = null) {
    this.omnibox = await this.readOmnibox();
    this.refreshAuxiliary(this.rows);
    if (source.isCurrent() !== true) return false;
    // MP-11: keeping capture warm never grants keyboard admission after the list closes.
    if (!point) return !!this.omniboxLive;
    const live = this.omniboxLive, scale = this.bound.tabbed.scale, region = source.region.window_region;
    const x = region.left / scale + point.x, y = region.top / scale + point.y;
    return !!live && x >= live.x && x < live.x + live.width && y >= live.y && y < live.y + live.height;
  }
  /** r10: is Chrome UI drawn inside the tabbed window (the omnibox dropdown, a dialog's page overlay)
   * over this window rect (physical px from the window's top-left)? A page region then holds its frame. */
  inWindowOver(rect, live = this.omniboxLive) {
    const tabbed = this.bound?.tabbed, scale = tabbed?.scale;
    // The list's stream is taller than a short list: only the live list covers the page.
    const over = b => !!b && b.x < rect.left + rect.width && rect.left < b.x + b.width && b.y < rect.top + rect.height && rect.top < b.y + b.height;
    return !!tabbed?.isCurrent() && (over(live && {x: live.x * scale, y: live.y * scale, width: live.width * scale, height: live.height * scale}) ||
      this.bound.auxiliary.some(({in_window, omnibox, bounds: b}) => in_window && !omnibox &&
        over({x: b.x - tabbed.bounds.x, y: b.y - tabbed.bounds.y, width: b.width, height: b.height})));
  }
  /** The same, after re-reading Chrome's list (the poll may not have seen it open yet). */
  async inWindowOverNow(rect) {
    const binding = this.bound?.tabbed, focus = this.host.arbiter?.chromeFocus;
    if (!binding?.isCurrent()) return false;
    // Steady page frames use the shared watcher. Chrome-UI input is the narrow
    // interval in which a dropdown can paint before that watcher observes it.
    const revision = this.occlusionRevision ?? 0, cache = this.occlusionCache;
    // MP-11: native dispatch acknowledges queueing, not Chrome's paint. A
    // negative read cannot end an active omnibox interval; the list may open
    // before the next watcher tick without another key or focus change.
    const focused = focus && focus.generation === this.host.generation && focus.window_generation === binding.window_generation;
    if (!focused && !this.uiInputs && (revision === 0 || cache?.binding === binding && cache.revision === revision)) return this.inWindowOver(rect, cache?.binding === binding ? cache.live : this.omniboxLive);
    const read = this.occlusionRead?.binding === binding && this.occlusionRead.revision === revision ? this.occlusionRead :
      (this.occlusionRead = {binding, revision, promise: this.readOmnibox()});
    let state;
    try { state = await read.promise; } finally { if (this.occlusionRead === read) this.occlusionRead = null; }
    if (this.bound?.tabbed !== binding || !binding.isCurrent()) return false;
    if (revision !== (this.occlusionRevision ?? 0)) return true; // input crossed observation: hold pixels
    // A capture probe observes geometry; auxiliary publication/retirement stays
    // on the watcher/input paths rather than mutating bindings inside readback.
    const live = omniboxPopupRect(state, binding.bounds.x / binding.scale, binding.bounds.y / binding.scale,
      binding.page_rect.top / binding.scale, binding.bounds.width / binding.scale, binding.bounds.height / binding.scale);
    if (!this.uiInputs && revision === (this.occlusionRevision ?? 0)) this.occlusionCache = {binding, revision, focus, live};
    return this.inWindowOver(rect, live);
  }
  /** While a band is watched, follow its popups (one poller per host, shared by viewers). The newest
   * band that still reads wins: an older band's capture can retire before its viewer unsubscribes.
   * The worker reports inventory changes (`wakes(fn)` subscribes to them): a report reads at once,
   * and `interval` is only the backstop for a change no event announces. Exactness never lives
   * here: every consumer still proves its window with isCurrent(). */
  watch(read, interval = INVENTORY_BACKSTOP_MS, wakes = null) {
    const watcher = this.watcher ??= {reads: new Set(), timer: null, running: false, again: false, offs: new Set()};
    watcher.reads.add(read);
    const arm = () => { clearTimeout(watcher.timer); watcher.timer = setTimeout(tick, interval).unref(); };
    const tick = async () => {
      clearTimeout(watcher.timer); watcher.running = true;
      try { await this.observeRows(); } finally { watcher.running = false; }
      if (!watcher.reads.size) return;
      if (watcher.again) { watcher.again = false; void tick(); } else arm();
    };
    const wake = () => { if (watcher.running) watcher.again = true; else if (watcher.reads.size) void tick(); };
    const off = wakes?.(wake);
    if (off) watcher.offs.add(off);
    if (!watcher.timer && !watcher.running) arm();
    return () => {
      watcher.reads.delete(read);
      if (off) { watcher.offs.delete(off); off(); }
      if (!watcher.reads.size) { clearTimeout(watcher.timer); this.watcher = null; }
    };
  }
  /** One read of the watched inventory now (a band input's reply sees the menu it opened). */
  async observeRows() {
    let unreadable = 0;
    for (const next of [...this.watcher?.reads ?? []].reverse()) {
      let rows;
      try { rows = await next(); } catch { unreadable++; continue; }
      this.unreadableMarked = false;
      try {
        const binding = this.bound?.tabbed, revision = this.occlusionRevision ?? 0, focus = this.host.arbiter?.chromeFocus;
        this.rowsReadAt = performance.now();
        this.omnibox = await this.readOmnibox(); this.refreshAuxiliary(rows);
        if (binding === this.bound?.tabbed && !this.uiInputs && revision === (this.occlusionRevision ?? 0)) this.occlusionCache = {binding, revision, focus, live: this.omniboxLive};
      } catch {}
      return;
    }
    // Every watched band is unreadable: popups are no longer refreshed (a closed menu stays listed). Marked once per streak.
    if (unreadable && !this.unreadableMarked) { this.unreadableMarked = true; this.host.timing?.('aux_poll_unreadable', performance.timeOrigin + performance.now()); }
  }
  /** Protocol 530 state: popups of the band's window, CSS px from the window's top-left. */
  auxiliaryState() {
    const tabbed = this.bound?.tabbed;
    if (!tabbed?.isCurrent()) return [];
    // The omnibox list's stream keeps its grown size; the viewer shows only the live list (r10), none while warm (540).
    const live = this.omniboxLive;
    return this.bound.auxiliary.map(({auxiliary_id, bounds, omnibox}) => ({auxiliary_id,
      x: Math.round((bounds.x - tabbed.bounds.x) / tabbed.scale), y: Math.round((bounds.y - tabbed.bounds.y) / tabbed.scale),
      width: (bounds.width - bounds.width % 2) / tabbed.scale,
      height: Math.min((bounds.height - bounds.height % 2) / tabbed.scale, omnibox ? live ? Math.floor(live.height / 2) * 2 : 0 : Infinity)}));
  }
  auxiliary(auxiliaryId) {
    const binding = this.current().auxiliary.find(a => a.auxiliary_id === auxiliaryId);
    if (!binding?.isCurrent()) throw new UserDomainRefusal('stale_reference');
    return binding;
  }
  /** Constructor arguments of vidlat's NativeWindowRegionCapture for the band. */
  band(binding = this.current().tabbed) {
    return {surface: 'browser_chrome', window: {id: binding.xid, width: binding.bounds.width, height: binding.bounds.height},
      rect: {left: 0, top: 0, width: binding.bounds.width-binding.bounds.width%2, height: binding.content_top-binding.content_top%2},
      contentHeight: binding.bounds.height - binding.content_top, isCurrent: binding.isBandCurrent ?? binding.isCurrent};
  }
  auxiliaryRegion(binding) {
    const tabbed = this.current().tabbed;
    if (binding?.role !== 'auxiliary' || binding.parent_xid !== tabbed.xid) throw new UserDomainRefusal('stale_reference');
    if (binding.page_overlay) {
      if (binding.xid !== tabbed.xid) throw new UserDomainRefusal('stale_reference');
      return {surface: 'browser_auxiliary', inWindow: true, pageOverlay: true, window: {id: tabbed.xid, width: tabbed.bounds.width, height: tabbed.bounds.height},
        rect: {left: binding.bounds.x - tabbed.bounds.x, top: binding.bounds.y - tabbed.bounds.y, width: binding.bounds.width, height: binding.bounds.height},
        contentHeight: tabbed.bounds.height - (binding.bounds.y - tabbed.bounds.y), isCurrent: binding.isCurrent};
    }
    if (binding.in_window) {
      // Below the band only (the band streams the rest). Its rect lies over the page (around its rounded
      // corners, below a shrunk list): the active Tab's masks apply there on every readback (see the page rect).
      if (binding.xid !== tabbed.xid) throw new UserDomainRefusal('stale_reference');
      const width = binding.bounds.width - binding.bounds.width % 2, height = binding.bounds.height - binding.bounds.height % 2;
      return {surface: 'browser_auxiliary', inWindow: true, window: {id: tabbed.xid, width: tabbed.bounds.width, height: tabbed.bounds.height},
        rect: {left: binding.bounds.x - tabbed.bounds.x, top: binding.bounds.y - tabbed.bounds.y, width, height},
        contentHeight: tabbed.bounds.height - tabbed.content_top,
        pageRect: tabbed.page_rect, isCurrent: binding.isCurrent};
    }
    return {surface: 'browser_auxiliary', window: {id: binding.xid, width: binding.bounds.width, height: binding.bounds.height},
      rect: {left: 0, top: 0, width: binding.bounds.width - binding.bounds.width % 2, height: binding.bounds.height - binding.bounds.height % 2}, isCurrent: binding.isCurrent};
  }
}

// vidlat chord modifier bits: Alt 1, Ctrl 2, Meta 4, Shift 8; keysyms are X keysyms.
const ALT = 1, CTRL = 2, SHIFT = 8, key = c => c.charCodeAt(0);
const lower = keysym => keysym >= 0x41 && keysym <= 0x5a ? keysym + 0x20 : keysym;
const matches = chords => (keysym, modifiers) => chords.some(([k, m]) => k === lower(keysym) && m === modifiers);
// Chords that open surfaces the lockdown blocks (view-source, incognito, open file,
// task manager). DevTools, print, save, downloads, history and settings are allowed (Miguel 2026-10-15).
export const refusedChord = matches([[key('u'), CTRL], [key('n'), CTRL | SHIFT], [key('o'), CTRL], [0xff1b, SHIFT]]);
// Browser chords always go to the band, whatever the page focus.
export const bandChord = matches([[key('l'), CTRL], [key('t'), CTRL], [key('w'), CTRL], [0xff09, CTRL], [0xff09, CTRL | SHIFT],
  [key('t'), CTRL | SHIFT], [0xff51, ALT], [0xff53, ALT]]);

// The native tab strip's height in the band (DIP); the toolbar row starts below it.
const STRIP_HEIGHT = 41;
// PERF_AUDIT 4.2.6: how long a strip input's reply waits for Chrome to show another Tab.
const FOLLOW_MS = 150;
// The whole follow-up (Tab wait, inventory and omnibox reads) is bounded.
const FOLLOW_LIMIT_MS = 500;

export class InputArbiter {
  constructor(host, windows) { this.host = host; this.windows = windows; }
  static agent(command) { return command._agent_input === true || command.focused_agent === true; }
  /** A native click into the page puts the keys back in the page. */
  pageFocused() { this.chromeFocus = null; }
  /** The kernel-observed active Tab of the tabbed window, or null (fail closed). */
  async activeTab() {
    // The native observer's own record stands while no window event retired it: a busy browser process need not answer a key.
    const target = this.host.tabGroups?.cachedActive?.() ?? await this.host.tabGroups?.active().catch(() => null);
    return target ? [...this.host.tabs.values()].find(tab => tab.target_id === target) ?? null : null;
  }
  async isActiveTab(tab, binding) {
    if (!binding?.isCurrent?.()) return false;
    const active = await this.activeTab();
    return !!active && active.target_id === tab?.target_id && binding.isCurrent();
  }
  /** BrowserChrome input acts on the active Tab: the ledger admits it against that Tab. */
  async resolveDisplayInput(command) {
    if (command.window_generation === undefined) return {tab_id: command.tab_id, document_id: command.document_id, generation: command.generation};
    if (InputArbiter.agent(command)) throw new UserDomainRefusal('not_granted');
    const binding = this.windows.current().tabbed;
    if (command.window_generation !== binding.window_generation) throw new UserDomainRefusal('stale_reference');
    const generation = this.host.generation;
    let tab = await slowStep(this.host, 'admit_active', this.activeTab());
    // A native New Tab can precede the next inventory poll. Discover it here,
    // before ledger admission; no action has been dispatched or replayed.
    // Discovery is advisory: the displayTarget read below still proves this
    // active document before admission. Loading background pages need no proof.
    if (!tab) { await slowStep(this.host, 'admit_reconcile', this.host.reconcile({readOnly:true})); tab = await this.activeTab(); }
    if (!tab || !binding.isCurrent()) throw new UserDomainRefusal('stale_reference');
    // Native navigation commits independently of inventory polling. Admit the
    // live document instead of attributing a person's new input to cached state.
    tab = await slowStep(this.host, 'admit_target', this.host.displayTarget({tab_id: tab.tab_id, generation, fresh: true}));
    const active = await slowStep(this.host, 'admit_recheck', this.activeTab());
    if (!binding.isCurrent() || generation !== this.host.generation || !active || active.target_id !== tab.target_id) throw new UserDomainRefusal('stale_reference');
    return {tab_id: tab.tab_id, document_id: tab.document_id, generation};
  }
  /** After ledger admission: band input reaches Chrome's own UI through the band
   * source, for the Tab the ledger admitted (rechecked right before dispatch). Only
   * the browser shortcuts go to Chrome's UI whatever the focus; every other key, text
   * or chord may land in the page (X focus decides), so it passes the page's own
   * text-target fence first (Vault-only fields, cross-origin frames fail closed). */
  async input(args) {
    const follow = {switches: false};
    const reply = await this.dispatchInput(args, follow);
    // The kernel answers each person's input in order (one serial input lane): the window it left is
    // read by a separate observation (`follow`) after that lane is free, never while later keys wait.
    return this.host.chromeWindow ? {...reply, _follow: {tab_id: args.command.tab_id, switches: follow.switches}} : reply;
  }
  /** `chrome_window_follow`: the window an admitted Chrome UI input left (best effort, bounded; a
   * stalled Chrome read answers without it and the viewer reads the state). */
  async follow({tab_id, switches}) {
    if (typeof tab_id !== 'string' || typeof switches !== 'boolean') throw new Error('MD-2: invalid window follow');
    const bound = new Promise(resolve => setTimeout(resolve, FOLLOW_LIMIT_MS, {}).unref?.());
    try { return await Promise.race([this.followed({}, {tab_id}, switches), bound]); } catch { return {}; }
  }
  /** PERF_AUDIT 4.2.2/4.2.6 (protocol 540): a band input's reply carries the window it left: the Tab
   * the strip now shows and its popups (the omnibox list a key opened, a menu a click opened). The
   * viewer follows in this round trip instead of at its next state read. A Tab the kernel has not
   * adopted yet (a new Tab) is left to that state read. */
  async followed(reply, command, switches) {
    const before = this.host.tabs.get(command.tab_id)?.target_id;
    let active = before;
    if (switches) for (const end = performance.now() + FOLLOW_MS; ;) {
      active = await this.host.tabGroups?.active?.().catch(() => null) ?? null;
      if (active !== before || performance.now() >= end) break;
      await new Promise(resolve => setTimeout(resolve, 15));
    }
    if (switches) await this.windows.observeRows?.();
    await this.windows.refreshWarm?.();
    const window = this.host.chromeWindow?.(), tab = [...this.host.tabs.values()].find(t => t.target_id === active);
    return window && tab ? {...reply, chrome_window: {...window, active_tab_id: tab.tab_id}} : reply;
  }
  async dispatchInput({stream, command, signal, onDispatch}, follow) {
    if (InputArbiter.agent(command)) throw new UserDomainRefusal('not_granted');
    const source = stream?.regionSource, input = command.input ?? {};
    // Clicks, wheel and browser shortcuts reach only Chrome's UI (the band crop holds no page
    // pixels): they need the admitted active Tab, not its document, so a strip click while the
    // page loads is not dropped. The person's text follows that same native focus.
    const activeAdmitted = async () => {
      if (signal?.aborted || !['browser_chrome', 'browser_auxiliary'].includes(source?.surface) || !source.isCurrent?.()) throw new UserDomainRefusal('stale_reference');
      const tab = await this.activeTab();
      if (!tab || tab.tab_id !== command.tab_id || command.generation !== this.host.generation) throw new UserDomainRefusal('stale_reference');
      if (source.overPage && !await this.windows.inWindowCurrent(source, ['click', 'scroll'].includes(input.kind) ? {x: input.x, y: input.y} : null)) throw new UserDomainRefusal('stale_reference');
      return tab;
    };
    const uiFocus = tab => ({tab_id:tab.tab_id, generation:this.host.generation, window_generation:stream.regionWindow?.window_generation});
    // MP-08/MP-10: these are the person's physical keys into their own
    // Chrome window. Navigation can replace the page after ledger admission;
    // it does not replace the omnibox. Input is not a model observation and
    // must not wait for a renderer/focus/document proof. The owned source,
    // browser generation and active Tab still admit every physical dispatch.
    const textTarget = activeAdmitted;
    // At most once: a native action without a reply may have run; never replayed.
    // vidlat C10: `onDispatch` marks the native action after every fence, right before it runs.
    const dispatched = async action => {
      let done;
      const finishUi = ['browser_chrome', 'browser_auxiliary'].includes(source?.surface) ? this.windows.uiInput?.() : null;
      try { onDispatch?.(); done = await slowStep(this.host, 'native_action', action()); } catch (error) { if (error?.code === 'native_input_uncertain') return {dispatched: true}; throw markDispatched(error); }
      finally { finishUi?.(); }
      if (!done) throw new UserDomainRefusal('stale_reference');
      return {dispatched: true};
    };
    // Strip clicks, wheel over the strip and browser shortcuts can show another Tab (review #1016 P1).
    const switching = async () => { follow.switches = true; const tab = await activeAdmitted(); this.host.retireNativeForeground?.()?.catch(() => {}); return tab; };
    const popup = source?.surface === 'browser_auxiliary';
    // Protocol 540: the address bar has the person's keys; its list stream stays warm meanwhile.
    const omnibox = (on, ago) => { this.inOmnibox = on; this.windows.focusOmnibox?.(on, ago); };
    const chord = async (keysym, modifiers) => {
      if (!Number.isSafeInteger(keysym) || !Number.isSafeInteger(modifiers) || refusedChord(keysym, modifiers)) throw new UserDomainRefusal('not_granted');
      let tab;
      // Review #1016 P2: an admitted native popup (menu, dropdown, dialog) owns the keys while it is
      // current (rechecked here): Chrome's UI, not the page's remembered field.
      // Review #1016 @40979b97d P1: the warm omnibox entry (540) stays current while the list is closed;
      // it owns the keys only while Chrome shows the list (re-read now). Otherwise the page's own fences apply.
      if (bandChord(keysym, modifiers)) tab = await switching();
      else if (popup && (!source.overPage || await this.windows.omniboxShown?.() === true)) await activeAdmitted();
      else await textTarget();
      // Ctrl+L puts the keys in the omnibox; Enter and Tab hand them back to the page, other shortcuts may switch Tabs.
      if (tab) this.chromeFocus = lower(keysym) === key('l') ? uiFocus(tab) : null;
      else if (keysym === 0xff0d || keysym === 0xff09 || keysym === 0xff1b) this.chromeFocus = null;
      omnibox(tab ? !!this.chromeFocus : !!this.inOmnibox && !!this.chromeFocus);
      return dispatched(() => source.chord(keysym, modifiers));
    };
    const band = source?.surface === 'browser_chrome';
    if (input.kind === 'click') {
      // Protocol 530: middle (close a Tab) and right (Chrome's context menu) clicks reach the band only.
      const button = input.button === undefined ? 1 : band ? {middle: 2, right: 3}[input.button] : undefined;
      if (button === undefined) throw new Error('MD-2: unsupported input');
      // Below the tab strip is Chrome's toolbar (omnibox, buttons): the keys go to Chrome's UI.
      // A popup item (menu, dropdown) may navigate or switch Tabs: the keys return to the page.
      const tab = await switching(); this.chromeFocus = band && button === 1 && input.y >= STRIP_HEIGHT ? uiFocus(tab) : null;
      // Before the window's first list its span is unknown: a toolbar click may be the address bar, so the keys
      // after it wait for the list they open (the click itself does not wait).
      const over = this.windows.onOmnibox?.(input.x);
      omnibox(!!this.chromeFocus && (over === true || over === null), over === null ? -OMNIBOX_LIST_WAIT_MS - 1 : 0);
      return dispatched(() => source.click(input.x, input.y, button));
    }
    if (input.kind === 'drag') {
      // Protocol 530: reorder Tabs within the strip; leaving it would tear a Tab into a new window.
      if (!band || ![input.x, input.y, input.to_x, input.to_y].every(Number.isSafeInteger) || input.y >= STRIP_HEIGHT || input.to_y >= STRIP_HEIGHT) throw new Error('MD-2: unsupported input');
      await switching(); this.chromeFocus = null; omnibox(false);
      return dispatched(() => source.drag(input.x, input.y, input.to_x, input.to_y));
    }
    if (input.kind === 'scroll') {
      const nx = notches(input.delta_x), ny = notches(input.delta_y);
      if (nx === null || ny === null || Math.abs(nx) > 10 || Math.abs(ny) > 10) throw new Error('MD-2: unsupported input');
      await switching(); return dispatched(() => source.wheel(input.x, input.y, nx, ny));
    }
    if (input.kind === 'key') {
      const key = typeof input.key === 'string' ? input.key : '';
      const keysym = /^[\x20-\x7e]$/.test(key) ? key.charCodeAt(0) : keysyms[key];
      if (keysym === undefined) throw new Error('MD-2: unsupported key');
      // MP-08/MP-10/MP-11: an accelerator's key label may be upper-case
      // without Shift (Caps Lock). Do not let X11 infer an extra modifier.
      const modifiers = input.modifiers ?? 0;
      return chord(modifiers & 7 ? lower(keysym) : keysym, modifiers);
    }
    if (input.kind === 'text') {
      if (typeof input.text !== 'string' || input.text.length > 2048 || !/^[\x20-\x7e]*$/.test(input.text)) throw new Error('MD-2: unsupported text');
      for (const c of input.text) await chord(c.charCodeAt(0), 0);
      return {dispatched: true};
    }
    throw new Error('MD-2: unsupported input');
  }
}
