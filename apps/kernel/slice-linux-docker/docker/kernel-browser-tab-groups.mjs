// MP-08/MP-11 (Miguel 2026-10-14 19:54Z): a built-in component extension of the
// kernel's own Chromium groups each agent's Tabs in the native tab strip under the
// agent's name; the person's Tabs stay ungrouped. The kernel writes it, loads it
// only over its private CDP pipe and drives it there; it never reads page content
// (debugger is used only for getTargets, to map CDP target ids to tab ids, never attach).
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

// The id is derived from this public key (no private key exists for it).
const key = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAwYnjYNb6F8Ak7OPn0a85KRhnnRvl5QsdWIl3tFtHPubMMulPEXLM8cmUT8vgE3lmHYkwpPAowsy9nSQJIvGg72eSsOQDqcieYRlY6mrrHOQq8SGfdYN5R/a5iEzOivE4q9GaM/dIQ11NYMs2X45oL172k2rvsZ0QqbC/oEMhWAwFrHidbB3yx/O7txxTz6Ds0/o97sSGHGvdsM1dtLN0nvOJkYbX6PFEADq3wP/eQzfiPsda6rH4whZejgrFDyuOXMv7dfjHFLWseuVG7XCPMB0vBBULhhLrDoTezQAteqBOmsAuRTAhws60ucpF8J1b3vvu/HcZBCwygdTmV/5o3QIDAQAB";
export const TAB_GROUPS_EXTENSION_ID = "ecideajikgeiboaphimcgeahicjgcfea";
const manifest = { manifest_version: 3, name: "Chariox tab groups", version: "1", key, permissions: ["tabs", "tabGroups", "debugger"], background: { service_worker: "sw.js" } };
const worker = `// MP-08/MP-11: driven by the kernel over CDP.
const targetTabs = async () => new Map((await chrome.debugger.getTargets()).filter(t => t.tabId !== undefined).map(t => [t.id, t.tabId]));
// An extension API call every 20 s keeps this worker alive for the kernel.
setInterval(() => chrome.runtime.getPlatformInfo(), 20000);
// One browser window (coordinator 12:22Z): a Tab in a second tabbed window (New window,
// Move tab to new window, a Tab dragged off the strip) becomes the active Tab of the oldest one;
// its emptied window closes. Chrome refuses Tab edits while the person still drags: retry.
const fold = async (id, tries = 100) => {
  const tab = await chrome.tabs.get(id);
  const [main, ...others] = (await chrome.windows.getAll({ windowTypes: ["normal"] })).sort((a, b) => a.id - b.id);
  if (!others.some(window => window.id === tab.windowId)) return;
  try { await chrome.tabs.move(id, { windowId: main.id, index: -1 }); }
  catch (error) {
    if (!tries || !/dragging/.test(error?.message)) throw error;
    await new Promise(resolve => setTimeout(resolve, 100));
    return fold(id, tries - 1);
  }
  await chrome.tabs.update(id, { active: true });
  await chrome.windows.update(main.id, { focused: true });
};
chrome.tabs.onCreated.addListener(tab => fold(tab.id).catch(() => {}));
chrome.tabs.onAttached.addListener(id => fold(id).catch(() => {}));
// MP-08/MP-10/MP-11: one fresh native query supplies active identity and
// dimensions. Parallel read-only inventory never caches admission authority;
// a newly created Tab absent from this inventory is refused until the next read.
const activeViewport = async () => {
  const [[normal], ids] = await Promise.all([chrome.windows.getAll({ windowTypes: ["normal"] }), targetTabs()]);
  if (!normal) return null;
  const [tab] = await chrome.tabs.query({ active: true, windowId: normal.id });
  for (const [target, id] of ids) if (id === tab?.id) return {target_id: target, width: tab.width, height: tab.height};
  return null;
};
globalThis.charioxTabs = {
  async viewport(target) {
    if (target === undefined) return activeViewport();
    const id = (await targetTabs()).get(target);
    if (id === undefined) return null;
    const tab = await chrome.tabs.get(id);
    return {target_id: target, width: tab.width, height: tab.height};
  },
  async active() { return (await activeViewport())?.target_id ?? null; },
  async sync(groups) {
    const ids = await targetTabs(), grouped = new Set();
    for (const { target_ids, title, color } of groups) {
      const tabIds = target_ids.map(id => ids.get(id)).filter(id => id !== undefined);
      if (!tabIds.length) continue;
      const existing = (await chrome.tabGroups.query({ title }))[0];
      const groupId = await chrome.tabs.group({ tabIds, ...(existing ? { groupId: existing.id } : {}) });
      await chrome.tabGroups.update(groupId, { title, color, collapsed: false });
      tabIds.forEach(id => grouped.add(id));
    }
    const loose = (await chrome.tabs.query({})).filter(tab => tab.groupId !== -1 && !grouped.has(tab.id)).map(tab => tab.id);
    if (loose.length) await chrome.tabs.ungroup(loose);
    return true;
  },
};
`;

export async function writeTabGroupsExtension(root) {
  const directory = path.join(root, "tab-groups-extension");
  await mkdir(directory, { recursive: true, mode: 0o755 });
  await writeFile(path.join(directory, "manifest.json"), JSON.stringify(manifest), { mode: 0o644 });
  await writeFile(path.join(directory, "sw.js"), worker, { mode: 0o644 });
  return directory;
}

const colors = ["blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange"];
const colorOf = label => colors[[...label].reduce((sum, c) => (sum * 31 + c.codePointAt(0)) >>> 0, 7) % colors.length];

export class ChromeTabGroups {
  constructor(connection) { this.connection = connection; this.session = null; this.signature = "[]"; }
  async load(directory) {
    const { id } = await this.connection.send("Extensions.loadUnpacked", { path: directory });
    if (id !== TAB_GROUPS_EXTENSION_ID) throw new Error("MP-11: unexpected kernel tab-groups extension id");
    this.directory = directory;
  }
  async worker(polls) {
    for (let n = 0; n < polls; n++) {
      const worker = (await this.connection.send("Target.getTargets")).targetInfos
        .find(target => target.type === "service_worker" && target.url.startsWith(`chrome-extension://${TAB_GROUPS_EXTENSION_ID}/`));
      if (worker) return worker;
      await delay(50);
    }
    return null;
  }
  async evaluate(expression) {
    for (let attempt = 0; attempt < 2; attempt++) {
      if (!this.session) {
        // The person can remove or disable it in chrome://extensions (no policy locks an unpacked
        // extension): the kernel installs it again (loading alone does not re-enable a disabled
        // one) and re-sends the groups; until it is back, band input fails closed.
        let worker = await this.worker(10);
        if (!worker && this.directory) {
          await this.connection.send("Extensions.uninstall", { id: TAB_GROUPS_EXTENSION_ID }).catch(() => {});
          await this.load(this.directory); this.signature = "[]"; worker = await this.worker(40);
        }
        if (!worker) throw new Error("MP-11: kernel tab-groups extension unavailable");
        this.session = (await this.connection.send("Target.attachToTarget", { targetId: worker.targetId, flatten: true })).sessionId;
      }
      try {
        const { result, exceptionDetails } = await this.connection.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }, this.session);
        if (exceptionDetails) throw new Error("MP-11: kernel tab-groups extension failed");
        return result?.value;
      } catch (error) { this.session = null; if (attempt) throw error; }
    }
  }
  /** CDP target id of the active Tab in the tabbed window, or null. */
  active() { return this.evaluate("charioxTabs.active()"); }
  /** Native Tab area in host pixels/DIPs; page emulation cannot supply it. */
  viewport(target) { return this.evaluate(`charioxTabs.viewport(${JSON.stringify(target)})`); }
  /** groups: [{target_ids, label}] — one native group per agent label. */
  async sync(groups) {
    const spec = groups.map(({ target_ids, label }) => ({ target_ids: [...target_ids].sort(), title: label, color: colorOf(label) }))
      .sort((a, b) => a.title.localeCompare(b.title));
    const signature = JSON.stringify(spec);
    if (signature === this.signature) return false;
    await this.evaluate(`charioxTabs.sync(${signature})`);
    this.signature = signature;
    return true;
  }
}
