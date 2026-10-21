import {NativeTabState,nativeWindowChangedBinding,nativeBoundsPayload} from "./kernel-browser-native-tab-state.mjs";
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
const invalidateNativeState = payload => () => globalThis[${JSON.stringify(nativeWindowChangedBinding)}]?.(payload);
// The payload only names the event kind (diagnostics); every kind but bounds retires the guards.
for (const [kind, event] of [['activated', chrome.tabs.onActivated], ['removed', chrome.tabs.onRemoved], ['windowRemoved', chrome.windows.onRemoved]]) event?.addListener?.(invalidateNativeState(kind));
// Chrome also reports the zoom it applies to a newly committed page; only a changed factor can change the layout.
chrome.tabs.onZoomChange?.addListener?.(info => invalidateNativeState(typeof info?.oldZoomFactor === 'number' && info.oldZoomFactor === info.newZoomFactor ? 'zoomsame' : 'zoom')());
chrome.windows.onBoundsChanged?.addListener?.(invalidateNativeState(${JSON.stringify(nativeBoundsPayload)}));
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
  revision: 2,
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
      const windowId = (await chrome.tabs.get(tabIds[0])).windowId;
      const existing = (await chrome.tabGroups.query({ title, windowId }))[0];
      const groupId = await chrome.tabs.group({ tabIds, ...(existing ? { groupId: existing.id } : { createProperties: { windowId } }) });
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
  constructor(connection, onChange) { this.connection = connection; this.session = null; this.signature = "[]"; this.nativeState=new NativeTabState(connection,()=>{this.session=null;},onChange); }
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
  async ensureSession() {
    if(this.nativeState.closed)throw Error('MP-11: native state observer closed');
    if(this.session)return this.session;
    return this.connecting??=(async()=>{
      // Removal/disablement never revives a cached native owner snapshot.
      let worker=await this.worker(10);
      if(!worker&&this.directory){
        await this.connection.send("Extensions.uninstall",{id:TAB_GROUPS_EXTENSION_ID}).catch(()=>{});
        await this.load(this.directory);this.signature="[]";worker=await this.worker(40);
      }
      if(!worker)throw Error("MP-11: kernel tab-groups extension unavailable");
      // MP-08/MP-10: an existing profile can run cached component code even
      // after its files change. Bind first, reload only an older worker, and
      // attach the replacement before exposing native state to callers.
      for(let attempt=0;attempt<2;attempt++){
        const {sessionId}=await this.connection.send("Target.attachToTarget",{targetId:worker.targetId,flatten:true});
        await this.nativeState.attach(sessionId);
        const {result}=await this.connection.send("Runtime.evaluate",{expression:"charioxTabs.revision === 2",returnByValue:true},sessionId);
        if(result?.value===true){this.session=sessionId;return sessionId;}
        this.nativeState.detach();this.signature="[]";
        if(attempt)throw Error("MP-11: kernel tab-groups extension unavailable");
        await this.connection.send("Runtime.evaluate",{expression:"chrome.runtime.reload()"},sessionId).catch(()=>{});
        await delay(500);worker=await this.worker(40);
        if(!worker)throw Error("MP-11: kernel tab-groups extension unavailable");
      }
    })().finally(()=>{this.connecting=null;});
  }
  async evaluate(expression,kind) {
    for(let attempt=0;attempt<2;attempt++){
      try{
        const session=await this.ensureSession(),epoch=this.nativeState.epoch;
        const {result,exceptionDetails}=await this.connection.send("Runtime.evaluate",{expression,awaitPromise:true,returnByValue:true},session);
        if(exceptionDetails)throw Error("MP-11: kernel tab-groups extension failed");
        this.nativeState.remember(kind,result?.value,epoch);return result?.value;
      }catch(error){this.nativeState.detach();if(attempt)throw error;}
    }
  }
  /** CDP target id of the active Tab in the tabbed window, or null. */
  active(){return this.evaluate("charioxTabs.active()",'active');}
  /** A target-specific read is geometry only, never active-Tab authority. */
  viewport(target){return this.evaluate(`charioxTabs.viewport(${JSON.stringify(target)})`,target===undefined?'viewport':undefined);}
  nativeStateSupported(){return this.nativeState.supported;}
  nativeStateGuard(){return this.nativeState.guard();}
  cachedActive(){return this.nativeState.cachedActive();}
  cachedViewport(){return this.nativeState.cachedViewport();}
  close(){this.nativeState.close();}
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
