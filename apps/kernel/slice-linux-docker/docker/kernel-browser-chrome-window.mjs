// MP-08/MP-10/MP-11 (INTEGRATION.md §2b): kernel-private binding of the kernel
// Chromium's own windows. A viewer never names a window, rect, PID or role; the
// kernel resolves them from CDP window bounds and the owned display's numeric
// inventory, and every consumer re-checks isCurrent() before readback or input.
import {nativePageRect} from './kernel-browser-page-geometry.mjs';
import {ownsDisplay} from './kernel-browser-owned-display.mjs';
import {UserDomainRefusal} from './kernel-browser-refusal.mjs';
import {keysyms, notches, sensitiveFocusExpression} from './kernel-browser-input.mjs';
import {assertCurrentDocument} from './browser-controller-actions.mjs';

// The owned display's numeric window inventory comes from the native producer
// (vidlat nativeWindowInventory, wired with the 519 region step); without it no binding.
const noInventory = async () => { throw new UserDomainRefusal('stale_reference'); };

export class ChromeWindows {
  constructor(host, {inventory = noInventory} = {}) {
    this.host = host; this.inventory = inventory;
    this.windowGeneration = 0; this.bound = null; this.listeners = new Set();
    this.auxiliaryIds = new Map(); this.auxiliarySequence = 0;
  }
  onRetire(listener) { this.listeners.add(listener); return () => this.listeners.delete(listener); }
  retire(reason) {
    if (!this.bound) return;
    const {window_generation} = this.bound.tabbed; this.bound = null;
    for (const listener of this.listeners) listener({window_generation, reason});
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
    const physical = {x: bounds.left * scale, y: bounds.top * scale, width: bounds.width * scale, height: bounds.height * scale};
    const tabbed = rows.filter(r => r.visible && !r.override_redirect && r.transient_for === 0 &&
      r.x === physical.x && r.y === physical.y && r.width === physical.width && r.height === physical.height);
    // Exactly one match or no binding: never the "closest" window.
    if (tabbed.length !== 1 || !live()) throw new UserDomainRefusal('stale_reference');
    const row = tabbed[0];
    const measure = async () => {
      const next = await host.tabGroups?.viewport();
      if (!next || ![...host.tabs.values()].some(tab => tab.target_id === next.target_id)) throw new UserDomainRefusal('stale_reference');
      return nativePageRect(row, next, scale);
    };
    const pageRect = nativePageRect(row, viewport, scale);
    // The page origin remains exact, including odd rows and GTK side insets.
    // Only the disjoint Chrome video band rounds down to even codec rows.
    const contentTop = Math.floor(pageRect.top / 2) * 2;
    if (!(contentTop > 0 && contentTop < row.height)) throw new UserDomainRefusal('stale_reference');
    const previous = this.bound?.tabbed;
    const same = previous && previous.xid === row.id && previous.bounds.width === row.width && previous.bounds.height === row.height && previous.content_top === contentTop && previous.browser_generation === browserGeneration && JSON.stringify(previous.page_rect) === JSON.stringify(pageRect);
    if (!same) { if (previous) this.retire('window_changed'); this.windowGeneration++; }
    const windowGeneration = this.windowGeneration;
    const isCurrent = () => live() && this.windowGeneration === windowGeneration && this.bound?.tabbed.xid === row.id;
    const isPageCurrent = async () => {
      if (!isCurrent()) return false;
      try { if (JSON.stringify(await measure()) === JSON.stringify(pageRect)) return isCurrent(); } catch {}
      if (isCurrent()) this.retire('page_geometry_changed');
      return false;
    };
    const binding = role => r => Object.freeze({browser_generation: browserGeneration, window_generation: windowGeneration, role, xid: r.id,
      parent_xid: role === 'auxiliary' ? row.id : null, bounds: Object.freeze({x: r.x, y: r.y, width: r.width, height: r.height}),
      content_top: role === 'tabbed' ? contentTop : null, page_rect: role === 'tabbed' ? pageRect : null,
      scale, pid, isCurrent, isPageCurrent});
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
    this.auxiliaryIds = ids;
    return bound;
  }
  /** Protocol 530: refresh only the auxiliary windows of the current tabbed binding (a live
   * band polls its own worker's inventory); a changed tabbed window waits for resolve(). */
  refreshAuxiliary(rows) {
    const bound = this.bound, tabbed = bound?.tabbed;
    if (!tabbed?.isCurrent() || !Array.isArray(rows)) return;
    const row = rows.find(r => r.id === tabbed.xid);
    if (!row || row.x !== tabbed.bounds.x || row.y !== tabbed.bounds.y || row.width !== tabbed.bounds.width || row.height !== tabbed.bounds.height) return;
    const bind = r => Object.freeze({...tabbed, role: 'auxiliary', xid: r.id, parent_xid: tabbed.xid, content_top: null,
      bounds: Object.freeze({x: r.x, y: r.y, width: r.width, height: r.height})});
    this.bound = Object.freeze({tabbed, auxiliary: this.auxiliaryOf(tabbed, rows, bind)});
  }
  /** While a band is watched, follow its popups (one poller per host, shared by viewers). */
  watch(read, interval = 150) {
    const watcher = this.watcher ??= {reads: new Set(), timer: null};
    watcher.reads.add(read);
    const tick = async () => {
      const [first] = watcher.reads;
      try { if (first) this.refreshAuxiliary(await first()); } catch {}
        if (watcher.reads.size) watcher.timer = setTimeout(tick, interval).unref();
    };
    if (!watcher.timer) watcher.timer = setTimeout(tick, interval).unref();
    return () => {
      watcher.reads.delete(read);
      if (!watcher.reads.size) { clearTimeout(watcher.timer); this.watcher = null; }
    };
  }
  /** Protocol 530 state: popups of the band's window, CSS px from the window's top-left. */
  auxiliaryState() {
    const tabbed = this.bound?.tabbed;
    if (!tabbed?.isCurrent()) return [];
    return this.bound.auxiliary.map(({auxiliary_id, bounds}) => ({auxiliary_id,
      x: Math.round((bounds.x - tabbed.bounds.x) / tabbed.scale), y: Math.round((bounds.y - tabbed.bounds.y) / tabbed.scale),
      width: (bounds.width - bounds.width % 2) / tabbed.scale, height: (bounds.height - bounds.height % 2) / tabbed.scale}));
  }
  auxiliary(auxiliaryId) {
    const binding = this.current().auxiliary.find(a => a.auxiliary_id === auxiliaryId);
    if (!binding?.isCurrent()) throw new UserDomainRefusal('stale_reference');
    return binding;
  }
  /** Constructor arguments of vidlat's NativeWindowRegionCapture for the band. */
  band(binding = this.current().tabbed) {
    return {surface: 'browser_chrome', window: {id: binding.xid, width: binding.bounds.width, height: binding.bounds.height},
      rect: {left: 0, top: 0, width: binding.bounds.width, height: binding.content_top},
      contentHeight: binding.bounds.height - binding.content_top, isCurrent: binding.isCurrent};
  }
  auxiliaryRegion(binding) {
    if (binding?.role !== 'auxiliary' || binding.parent_xid !== this.current().tabbed.xid) throw new UserDomainRefusal('stale_reference');
    return {surface: 'browser_auxiliary', window: {id: binding.xid, width: binding.bounds.width, height: binding.bounds.height},
      rect: {left: 0, top: 0, width: binding.bounds.width - binding.bounds.width % 2, height: binding.bounds.height - binding.bounds.height % 2}, isCurrent: binding.isCurrent};
  }
}

// vidlat chord modifier bits: Alt 1, Ctrl 2, Meta 4, Shift 8; keysyms are X keysyms.
const ALT = 1, CTRL = 2, SHIFT = 8, key = c => c.charCodeAt(0);
const lower = keysym => keysym >= 0x41 && keysym <= 0x5a ? keysym + 0x20 : keysym;
const matches = chords => (keysym, modifiers) => chords.some(([k, m]) => k === lower(keysym) && m === modifiers);
// Chords that open surfaces the lockdown blocks (DevTools, view-source, incognito,
// print, save, open file, downloads, history, clear data, task manager).
export const refusedChord = matches([[0xffc9, 0], [key('i'), CTRL | SHIFT], [key('j'), CTRL | SHIFT], [key('c'), CTRL | SHIFT],
  [key('u'), CTRL], [key('n'), CTRL | SHIFT], [0xffff, CTRL | SHIFT], [key('p'), CTRL], [key('s'), CTRL], [key('o'), CTRL],
  [key('j'), CTRL], [key('h'), CTRL], [0xff1b, SHIFT]]);
// Browser chords always go to the band, whatever the page focus.
export const bandChord = matches([[key('l'), CTRL], [key('t'), CTRL], [key('w'), CTRL], [0xff09, CTRL], [0xff09, CTRL | SHIFT],
  [key('t'), CTRL | SHIFT], [0xff51, ALT], [0xff53, ALT]]);

// The native tab strip's height in the band (DIP); the toolbar row starts below it.
const STRIP_HEIGHT = 41;

export class InputArbiter {
  constructor(host, windows) { this.host = host; this.windows = windows; }
  static agent(command) { return command._agent_input === true || command.focused_agent === true; }
  /** The kernel-observed active Tab of the tabbed window, or null (fail closed). */
  async activeTab() {
    const target = await this.host.tabGroups?.active().catch(() => null);
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
    const tab = await this.activeTab();
    if (!tab || !binding.isCurrent()) throw new UserDomainRefusal('stale_reference');
    return {tab_id: tab.tab_id, document_id: tab.document_id, generation: this.host.generation};
  }
  /** After ledger admission: band input reaches Chrome's own UI through the band
   * source, for the Tab the ledger admitted (rechecked right before dispatch). Only
   * the browser shortcuts go to Chrome's UI whatever the focus; every other key, text
   * or chord may land in the page (X focus decides), so it passes the page's own
   * text-target fence first (Vault-only fields, cross-origin frames fail closed). */
  async input({stream, command, signal}) {
    if (InputArbiter.agent(command)) throw new UserDomainRefusal('not_granted');
    const source = stream?.regionSource, input = command.input ?? {};
    // Clicks, wheel and browser shortcuts reach only Chrome's UI (the band crop holds no page
    // pixels): they need the admitted active Tab, not its document, so a strip click while the
    // page loads is not dropped. Text and other keys may reach the page: document fenced too.
    const activeAdmitted = async () => {
      if (signal?.aborted || !['browser_chrome', 'browser_auxiliary'].includes(source?.surface) || !source.isCurrent?.()) throw new UserDomainRefusal('stale_reference');
      const tab = await this.activeTab();
      if (!tab || tab.tab_id !== command.tab_id || command.generation !== this.host.generation) throw new UserDomainRefusal('stale_reference');
      return tab;
    };
    const current = async () => {
      const tab = await activeAdmitted();
      if (tab.document_id !== command.document_id) throw new UserDomainRefusal('stale_reference');
      const {connection, sessionId} = await this.host.browser.resolvePageTarget(tab.target_id);
      await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
      return {connection, sessionId, tab};
    };
    // Isolated world: the page cannot redefine what it reports. The shared
    // input owner suspends a mirror's focus emulation for the physical read.
    const textTarget = async () => {
      const {connection, sessionId, tab} = await current();
      const {frameTree} = await connection.send('Page.getFrameTree', {}, sessionId);
      const world = await this.host.browser.ensureFocusWorld(connection, sessionId, tab.target_id, frameTree.frame);
      const evaluate = expression => connection.send('Runtime.evaluate', {contextId: world.contextId, expression, returnByValue: true}, sessionId);
      // Review #1016 P2: Chromium keeps the page's activeElement while its own UI owns the keys.
      // Only when the person put focus there for this document and the page reports no focus.
      if (this.chromeFocus === `${tab.tab_id}:${tab.document_id}`) {
        const focused=await this.host.browser.inputCapture.physicalFocus(connection,sessionId,async()=>(await evaluate('document.hasFocus()')).result?.value);
        if(signal?.aborted||!source.isCurrent?.())throw new UserDomainRefusal('stale_reference');
        if(focused===false)return;
      }
      const {result} = await evaluate(sensitiveFocusExpression(false));
      if (result?.value !== false) throw new UserDomainRefusal('sensitive_requires_focus');
    };
    // At most once: a native action without a reply may have run; never replayed.
    const dispatched = async action => {
      let done;
      try { done = await action(); } catch (error) { if (error?.code === 'native_input_uncertain') return {dispatched: true}; throw error; }
      if (!done) throw new UserDomainRefusal('stale_reference');
      return {dispatched: true};
    };
    // Strip clicks, wheel over the strip and browser shortcuts can show another Tab (review #1016 P1).
    const switching = async () => { const tab = await activeAdmitted(); this.host.retireNativeForeground?.()?.catch(() => {}); return tab; };
    const chord = async (keysym, modifiers) => {
      if (!Number.isSafeInteger(keysym) || !Number.isSafeInteger(modifiers) || refusedChord(keysym, modifiers)) throw new UserDomainRefusal('not_granted');
      let tab;
      if (bandChord(keysym, modifiers)) tab = await switching(); else await textTarget();
      // Ctrl+L puts the keys in the omnibox; Enter and Tab hand them back to the page, other shortcuts may switch Tabs.
      if (tab) this.chromeFocus = lower(keysym) === key('l') ? `${tab.tab_id}:${tab.document_id}` : null;
      else if (keysym === 0xff0d || keysym === 0xff09) this.chromeFocus = null;
      return dispatched(() => source.chord(keysym, modifiers));
    };
    const band = source?.surface === 'browser_chrome';
    if (input.kind === 'click') {
      // Protocol 530: middle (close a Tab) and right (Chrome's context menu) clicks reach the band only.
      const button = input.button === undefined ? 1 : band ? {middle: 2, right: 3}[input.button] : undefined;
      if (button === undefined) throw new Error('MD-2: unsupported input');
      // Below the tab strip is Chrome's toolbar (omnibox, buttons): the keys go to Chrome's UI.
      // A popup item (menu, dropdown) may navigate or switch Tabs: the keys return to the page.
      const tab = await switching(); this.chromeFocus = band && button === 1 && input.y >= STRIP_HEIGHT ? `${tab.tab_id}:${tab.document_id}` : null;
      return dispatched(() => source.click(input.x, input.y, button));
    }
    if (input.kind === 'drag') {
      // Protocol 530: reorder Tabs within the strip; leaving it would tear a Tab into a new window.
      if (!band || ![input.x, input.y, input.to_x, input.to_y].every(Number.isSafeInteger) || input.y >= STRIP_HEIGHT || input.to_y >= STRIP_HEIGHT) throw new Error('MD-2: unsupported input');
      await switching(); this.chromeFocus = null;
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
