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
globalThis.charioxTabs = {
  async active() {
    const [normal] = await chrome.windows.getAll({ windowTypes: ["normal"] });
    if (!normal) return null;
    const [tab] = await chrome.tabs.query({ active: true, windowId: normal.id });
    for (const [target, id] of await targetTabs()) if (id === tab?.id) return target;
    return null;
  },
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
  }
  async evaluate(expression) {
    for (let attempt = 0; attempt < 2; attempt++) {
      if (!this.session) {
        let worker;
        for (let n = 0; n < 40 && !worker; n++) {
          worker = (await this.connection.send("Target.getTargets")).targetInfos
            .find(target => target.type === "service_worker" && target.url.startsWith(`chrome-extension://${TAB_GROUPS_EXTENSION_ID}/`));
          if (!worker) await delay(50);
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
