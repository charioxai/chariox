// MP-08/MP-10/MP-11 (INTEGRATION.md §2b): kernel-private binding of the kernel
// Chromium's own windows. A viewer never names a window, rect, PID or role; the
// kernel resolves them from CDP window bounds and the owned display's numeric
// inventory, and every consumer re-checks isCurrent() before readback or input.
import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
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
  /** Resolve (or re-resolve) the tabbed window and its auxiliary windows. */
  async resolve() {
    const host = this.host, chromium = host.chromium, display = chromium.display, pid = chromium.browserPid ?? chromium.child?.pid, scale = chromium.scale;
    const browserGeneration = host.generation;
    const live = () => host.generation === browserGeneration && ownsDisplay(display) && (chromium.browserPid ?? chromium.child?.pid) === pid;
    if (!live() || !Number.isSafeInteger(pid) || pid <= 1) throw new UserDomainRefusal('stale_reference');
    const connection = host.browser.connection;
    const page = [...host.tabs.values()][0];
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
    // The page area is pinned by the kernel (geometry); the band is the rest of the
    // real window. An infobar only shrinks the page, so the band never includes page pixels.
    const contentTop = Math.floor((row.height - geometry.height * scale) / 2) * 2;
    if (!(contentTop > 0 && contentTop < row.height)) throw new UserDomainRefusal('stale_reference');
    const previous = this.bound?.tabbed;
    const same = previous && previous.xid === row.id && previous.bounds.width === row.width && previous.bounds.height === row.height && previous.content_top === contentTop && previous.browser_generation === browserGeneration;
    if (!same) { if (previous) this.retire('window_changed'); this.windowGeneration++; }
    const windowGeneration = this.windowGeneration;
    const isCurrent = () => live() && this.windowGeneration === windowGeneration && this.bound?.tabbed.xid === row.id;
    const binding = role => r => Object.freeze({browser_generation: browserGeneration, window_generation: windowGeneration, role, xid: r.id,
      parent_xid: role === 'auxiliary' ? row.id : null, bounds: Object.freeze({x: r.x, y: r.y, width: r.width, height: r.height}),
      content_top: role === 'tabbed' ? contentTop : null, scale, pid, isCurrent});
    // Omnibox dropdowns and menus (the dropdown overlays the omnibox itself): same PID
    // (the inventory's filter), visible, transient for or anchored inside the tabbed window.
    const auxiliary = rows.filter(r => r.id !== row.id && r.visible && (r.transient_for === row.id ||
      r.override_redirect && r.x >= row.x && r.x < row.x + row.width && r.y >= row.y && r.y < row.y + row.height));
    this.bound = Object.freeze({tabbed: binding('tabbed')(row), auxiliary: auxiliary.map(binding('auxiliary'))});
    return this.bound;
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
    const current = async () => {
      if (signal?.aborted || source?.surface !== 'browser_chrome' || !source.isCurrent?.()) throw new UserDomainRefusal('stale_reference');
      const tab = await this.activeTab();
      if (!tab || tab.tab_id !== command.tab_id || tab.document_id !== command.document_id || command.generation !== this.host.generation) throw new UserDomainRefusal('stale_reference');
      const {connection, sessionId} = await this.host.browser.resolvePageTarget(tab.target_id);
      await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
      return {connection, sessionId, tab};
    };
    // Isolated world: the page cannot redefine what it reports; focus emulation does not change it.
    const textTarget = async () => {
      const {connection, sessionId, tab} = await current();
      const {frameTree} = await connection.send('Page.getFrameTree', {}, sessionId);
      const world = await this.host.browser.ensureFocusWorld(connection, sessionId, tab.target_id, frameTree.frame);
      const {result} = await connection.send('Runtime.evaluate', {contextId: world.contextId, expression: sensitiveFocusExpression(false), returnByValue: true}, sessionId);
      if (result?.value !== false) throw new UserDomainRefusal('sensitive_requires_focus');
    };
    // At most once: a native action without a reply may have run; never replayed.
    const dispatched = async action => {
      let done;
      try { done = await action(); } catch (error) { if (error?.code === 'native_input_uncertain') return {dispatched: true}; throw error; }
      if (!done) throw new UserDomainRefusal('stale_reference');
      return {dispatched: true};
    };
    const chord = async (keysym, modifiers) => {
      if (!Number.isSafeInteger(keysym) || !Number.isSafeInteger(modifiers) || refusedChord(keysym, modifiers)) throw new UserDomainRefusal('not_granted');
      if (bandChord(keysym, modifiers)) await current(); else await textTarget();
      return dispatched(() => source.chord(keysym, modifiers));
    };
    if (input.kind === 'click') { await current(); return dispatched(() => source.click(input.x, input.y)); }
    if (input.kind === 'scroll') {
      const nx = notches(input.delta_x), ny = notches(input.delta_y);
      if (nx === null || ny === null || Math.abs(nx) > 10 || Math.abs(ny) > 10) throw new Error('MD-2: unsupported input');
      await current(); return dispatched(() => source.wheel(input.x, input.y, nx, ny));
    }
    if (input.kind === 'key') {
      const key = typeof input.key === 'string' ? input.key : '';
      const keysym = /^[\x20-\x7e]$/.test(key) ? key.charCodeAt(0) : keysyms[key];
      if (keysym === undefined) throw new Error('MD-2: unsupported key');
      return chord(keysym, input.modifiers ?? 0);
    }
    if (input.kind === 'text') {
      if (typeof input.text !== 'string' || input.text.length > 2048 || !/^[\x20-\x7e]*$/.test(input.text)) throw new Error('MD-2: unsupported text');
      for (const c of input.text) await chord(c.charCodeAt(0), 0);
      return {dispatched: true};
    }
    throw new Error('MD-2: unsupported input');
  }
}
