import { DesktopDisplay } from './kernel-desktop-display.mjs';
import {displayMaskRegions} from './kernel-browser-pixels.mjs';
import {displayGeometry as geometry,displayDeviceMetrics} from './kernel-browser-geometry.mjs';
// MD-2: sessionless host adapter over the shared controller/CDP implementation.
import { UserDomainRefusal } from "./kernel-browser-refusal.mjs";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { displayTiming, timestamp } from './kernel-browser-timing.mjs';
import { hostErrorLabel } from './kernel-browser-error-label.mjs';
import { BrowserCdpClient, isTrustedStaleReferenceError } from "./browser-controller-cdp.mjs";
import { BrowserControllerStdioServer, handleBrowserControllerRequest } from "./browser-controller.mjs";
import { NativeAccessibility } from './native-accessibility.mjs';
import { NativeComputer } from './native-computer.mjs';
import { HostChromium } from "./kernel-browser-process.mjs";
import { redactObservation } from "./browser-controller-snapshot.mjs";
import { inputHostTab } from "./kernel-browser-input.mjs";
import { assertNotCancelled, assertCurrentDocument, BrowserActionError } from "./browser-controller-actions.mjs";
import { captureRegionMasks, captureProtectedDisplay, protectionDeclared, regionProtectionChanged } from "./kernel-browser-region-protection.mjs";
import { captureProtectedPage, wholeFrameMask } from "./kernel-browser-pixels.mjs";

import { MirrorService, MirrorInputEpochRefusal } from "./kernel-browser-mirror.mjs";
import {LinuxCapture,selectNativeCapture} from './kernel-browser-native.mjs';
import { CompositorSource } from './kernel-browser-compositor.mjs';
import { SampleLane } from './kernel-browser-sample-lane.mjs';
import { BrowserEncoder } from './kernel-browser-webcodecs.mjs';
import { exactPatchLimit, DisplayStream } from "./kernel-browser-display.mjs";
import {nativeDamageTiles} from './kernel-browser-tiles.mjs';
import {MotionEncoder} from './kernel-browser-motion.mjs';
import {NativeRefiner} from './kernel-browser-refiner.mjs';
import {identicalToDelivered,nativeCreditEmpty,nativeRegionBaseCurrent,nativeRegionPending} from './kernel-browser-native-credit.mjs';
import { DisplayCapture } from './kernel-browser-display-capture.mjs';

// Private display operations may overlap input; lifecycle still settles all
// owned work before closing. Public admission/Vault barriers stay in Rust.
export function scheduleHostRequest(request) {
  return request.method === 'host.browser' && request.params?.op === 'screenshot' &&
    typeof request.params.display_subscription_id === 'string'
    ? {kind:'bridge'} : {kind:'barrier'};
}
const TAB_LIMIT = 128;
function restorationUrl(url) {
  try { return navigationUrl(url); } catch { return "about:blank"; }
}

const viewport = { css_width: geometry.width, css_height: geometry.height, device_scale_factor: 1,
  desktop_pixel_width: geometry.width, desktop_pixel_height: geometry.height };
export function navigationUrl(raw) {
  if (typeof raw !== "string" || raw.length > 8192) throw new Error("invalid browser URL");
  const url = new URL(raw);
  if (raw !== "about:blank" && !["http:", "https:"].includes(url.protocol)) throw new Error("browser URL must be HTTP(S) or about:blank");
  if (url.username || url.password) throw new Error("browser URL credentials are forbidden");
  return url.href;
}

export class KernelBrowserHost {
  constructor(root, { chromium = new HostChromium(root), browserFactory = connection => new BrowserCdpClient({ connectionFactory: () => connection }) } = {}) {
    this.root = root;
    this.timing = displayTiming(root);
    this.chromium = chromium;
    this.browserFactory = browserFactory;
    this.browser = null;
    this.generation = 0;
    this.tabs = new Map();
    this.streams = new Map();
    this.displays = new Map();
    this.scales = new Map();
    this.inputEpochs = new Map();this.inputChangedAt=new Map();
    this.scrolling = new Map();
    this.sampleLanes = new Map();
    this.compositors = new Map();
    this.restoring = false;
    this.keepaliveTarget = null;
    this.observedDocuments = new Map();
    this.mirror = new MirrorService(this);
    this.protection = { values: [], targets: [], unknown: false };
    this.desktopDisplay = new DesktopDisplay(this);
    this.onNativeInput = () => this.desktopDisplay.wake();
    this.nativeAccessibility = new NativeAccessibility({binding:()=>this.chromium.desktop?.binding()});
    this.nativeComputer = new NativeComputer({ placement: 'host',
      binding: () => this.chromium.desktop?.binding(),
      wakeCapture: event => this.onNativeInput?.(event),
    });
  }
  async protect(policy) {
    if (!Array.isArray(policy.values) || policy.values.length > 256 || policy.values.some(value => typeof value !== "string" || !value) || !Array.isArray(policy.targets)) throw new Error("MD-5: invalid protection policy");
    if (JSON.stringify(policy) === JSON.stringify(this.protection)) return {};
    this.protection = policy;
    await this.desktopDisplay.retireSource();
    this.nativeAccessibility.clear();
    this.mirror.invalidate();
    await this.closeCompositors();
    for (const stream of this.displays.values()) stream.invalidate();
    if (this.browser) this.browser.protectedValues = new Set(policy.values);
    // No frame captured before insertion/retirement can be returned afterward.
    for (const stream of this.streams.values()) {
      stream.latest = policy.unknown || policy.values.length ? this.maskedStreamFrame(stream) : null;
    }
    return {};
  }
  maskedStreamFrame(stream) {
    return { generation: this.generation, tab_id: stream.tabId, mime_type: "image/png", data_base64: wholeFrameMask(),
      width: viewport.css_width, height: viewport.css_height, sequence: ++stream.sequence };
  }
  save() {
    // MP-11: display reads overlap barrier operations; one tabs.json.new writer.
    return this.saving = (this.saving ?? Promise.resolve()).catch(() => {}).then(() => this.write());
  }
  async write() {
    const name = path.join(this.root, "tabs.json");
    const data = { generation: this.generation, tabs: [...this.tabs.values()].filter(tab => !this.browser?.appTabs?.apps || ![...this.browser.appTabs.apps.values()].some(app => app.targetId === tab.target_id)).slice(0, TAB_LIMIT).map(({ tab_id, url }) => ({ tab_id, url: redactObservation(url, this.protection.values) === url ? restorationUrl(url) : "about:blank" })) };
    const serialized = JSON.stringify(data);
    if (serialized === this.lastSaved) return;
    await writeFile(`${name}.new`, serialized, { mode: 0o600 });
    await rename(`${name}.new`, name);
    this.lastSaved = serialized;
  }
  async start({ signal, allowStart = true } = {}) {
    assertNotCancelled(signal);
    if (this.browser && this.chromium.child?.exitCode === null && this.chromium.child?.signalCode === null
      && this.chromium.connection?.isOpen() !== false) return;
    if (!allowStart) throw new BrowserActionError("browser_unavailable", "MP-11: user browser is stopped or unavailable; explicitly start/open the browser");
    await this.desktopDisplay.close();
    await this.closeCompositors();
    this.sampleLanes.clear();this.inputChangedAt.clear();this.scrolling.clear();

    // MP-11: retire helpers on the old display before Chromium recovery can
    // replace it. A failed key release still requires owned desktop teardown.
    try { await this.nativeComputer.close(); }
    finally { await this.chromium.stop(); this.nativeAccessibility.clear(); }
    for (const stream of this.displays.values()) await stream.close();
    this.mirror.clear();
    this.displays.clear(); this.scales.clear(); this.inputEpochs.clear();
    await this.browser?.close();
    this.browser = null;
    for (const stream of this.streams.values()) { stream.off(); clearTimeout(stream.timer); }
    this.streams.clear();
    this.tabs.clear();
    this.observedDocuments.clear();
    await mkdir(this.root, { recursive: true, mode: 0o700 });
    let saved = { generation: 0, tabs: [] };
    try { saved = JSON.parse(await readFile(path.join(this.root, "tabs.json"), "utf8")); }
    catch (error) { if (error.code !== "ENOENT") throw new Error("MD-2: browser tab registry is unreadable; preserve it for recovery"); }
    if (!Number.isSafeInteger(saved.generation) || saved.generation < 0 || !Array.isArray(saved.tabs)) {
      throw new Error("MD-2: invalid browser tab registry");
    }
    assertNotCancelled(signal);
    const connection = await this.chromium.start();
    try{await this.nativeComputer.primeKeyboard();}catch(error){await this.chromium.stop(connection);throw error;}
    assertNotCancelled(signal);
    this.browser = this.browserFactory(connection);
    this.browser.protectedValues = new Set(this.protection.values);
    this.generation = saved.generation + 1;
    // Publish the new generation before any restoration, so old references never revive.
    this.restoring = true;
    try {
      const connection = await this.browser.ensureConnection();
      const { targetInfos = [] } = await connection.send("Target.getTargets");
      // Headed Chromium exits when its last page closes. Keep one internal blank
      // target so startup and closing the last user tab never stop the browser.
      const pages = targetInfos.filter(target => target.type === "page");
      this.keepaliveTarget = pages.find(target => target.url === "about:blank")?.targetId
        ?? (await connection.send("Target.createTarget", { url: "about:blank" })).targetId;
      for (const target of pages) {
        if (target.targetId !== this.keepaliveTarget) {
          await connection.send("Target.closeTarget", { targetId: target.targetId });
        }
      }
      for (const tab of saved.tabs.slice(0, TAB_LIMIT)) {
        if (typeof tab.tab_id !== "string" || !tab.tab_id.startsWith("host-tab-")) throw new Error("MD-2: invalid saved tab identity");
        assertNotCancelled(signal);
        await this.open(restorationUrl(tab.url), tab.tab_id, { signal });
      }
      await this.save();
    } catch (error) { await this.stop(); throw error; }
    finally { this.restoring = false; }
  }
  async closeCompositors(tabId) {
    for(const [id,entry] of this.compositors) if(tabId===undefined||id===tabId){
      this.compositors.delete(id);await entry.ready.catch(()=>null);await entry.source?.close().catch(()=>{});
    }
  }
  // MP-08/MP-10: one fixed-label diagnostic per change of native refusal scope.
  nativeScope(reason){if(this.nativeScopeReason!==reason){this.nativeScopeReason=reason;if(reason)this.timing('native_unavailable_scope '+reason,timestamp());}}
  async compositorFor(tab,stream) {
    const scope=stream.codec==='png'?'png_codec':this.protection.unknown||this.protection.values.length||this.protection.targets.length?'protection_policy':
      [...this.streams.values()].some(s=>s.tabId===tab.tab_id)?'legacy_stream':null;
    if(scope){this.nativeScope(scope);return null;}
    let entry=this.compositors.get(tab.tab_id);
    // MP-11: a refused attestation (animated page, caret) costs a lease and two
    // protected captures. Retry it per document/policy after a doubling backoff.
    const refusals=entry?.document===tab.document_id&&entry.policy===this.protection?entry.refusals??0:0;
    if(refusals&&performance.now()<entry.retryAt)return null;
    if(entry&&(entry.document!==tab.document_id||entry.source?.closed)){await this.closeCompositors(tab.tab_id);entry=null;}
    if(!entry){
      const {connection,sessionId}=await this.browser.resolvePageTarget(tab.target_id);
      const policy=this.protection,generation=this.generation;
      let source=await selectNativeCapture({display:this.chromium.display,refused:reason=>this.nativeScope(reason),create:async()=>{
        if(this.tabs.size!==1){this.nativeScope('tab_count');throw Error('native tab scope');}
        this.nativeScope(null);
        const source=new LinuxCapture({display:this.chromium.display,pid:this.chromium.child?.pid,connection,sessionId,tab,scale:stream.device_scale_factor,policy,screenshot:()=>this.displayScreenshot(tab,null,false),allowed:p=>this.tabs.size===1&&this.protection===p&&!p.unknown&&!p.values.length&&!p.targets.length&&this.generation===generation,timing:this.timing});
        return await source.start();
      }});
      source??=new CompositorSource({connection,sessionId,tab,scale:stream.device_scale_factor,policy,timing:this.timing,width:geometry.width*stream.device_scale_factor,height:geometry.height*stream.device_scale_factor,format:'jpeg',acquire:()=>this.sampleLane(tab).run('input',()=>this.browser.inputCapture.hold(connection,sessionId)),
        screenshot:clip=>this.displayScreenshot(tab,clip),protect:()=>this.displayScreenshot(tab),allowed:p=>this.protection===p&&!p.unknown&&!p.values.length&&!p.targets.length&&this.generation===generation});
      const created={source,document:tab.document_id,policy};
      created.ready=source instanceof LinuxCapture?Promise.resolve(source):source.start().catch(()=>{created.refusals=refusals+1;created.retryAt=performance.now()+Math.min(30_000,1000*2**refusals);return null;});
      entry=created;this.compositors.set(tab.tab_id,entry);
    }
    return await entry.ready;
  }
  async stop() {
    await this.desktopDisplay.close();
    await this.closeCompositors();
    this.sampleLanes.clear();

    this.nativeAccessibility.clear();
    // A dead X server can make release fail; retirement still destroys the
    // owned desktop before acknowledging stop.
    await this.nativeComputer.close().catch(() => {});
    await this.chromium.stop(this.browser?.connection);
    for (const stream of this.displays.values()) await stream.close();
    this.mirror.clear();
    this.displays.clear(); this.scales.clear();
    await this.browser?.close();
    this.browser = null;
    for (const stream of this.streams.values()) { stream.off(); clearTimeout(stream.timer); }
    this.streams.clear();
    this.timing.flush?.();
    return { state: "stopped", generation: this.generation, tabs: [] };
  }
  async reconcile() {
    let state;
    // Restored pages can finish navigation between CDP document reads. Retry
    // only this observation, never the mutation that led to reconciliation.
    for (let attempt = 0; attempt < 3; attempt++) {
      try { state = await this.browser.reconcile(viewport, { browserBarVisible: false }); break; }
      catch (error) {
        if (error.code !== "stale_document_reference" || attempt === 2) throw error;
        await delay(25);
      }
    }
    // Native address-bar navigation turns the initial blank target into a user
    // tab. Replace the reserve without taking focus before adopting that tab.
    const keepalive = state.tabs.find(tab => tab.target_id === this.keepaliveTarget);
    if (keepalive && keepalive.url !== "about:blank") {
      const connection = await this.browser.ensureConnection();
      this.keepaliveTarget = (await connection.send("Target.createTarget", {
        url: "about:blank", background: true,
      })).targetId;
    }
    const byTarget = new Map([...this.tabs.values()].map(tab => [tab.target_id, tab]));
    this.tabs.clear();
    const discovered = state.tabs.filter(tab => tab.target_id !== this.keepaliveTarget);
    // Preserve already-adopted identities before accepting native/popup tabs.
    discovered.sort((a, b) => Number(byTarget.has(b.target_id)) - Number(byTarget.has(a.target_id)));
    for (const tab of discovered) {
      if (this.tabs.size >= TAB_LIMIT) {
        const connection = await this.browser.ensureConnection();
        await connection.send("Target.closeTarget", { targetId: tab.target_id });
        continue;
      }
      const old = byTarget.get(tab.target_id);
      this.tabs.set(old?.tab_id ?? `host-tab-${randomUUID()}`, { ...tab, tab_id: old?.tab_id ?? null });
    }
    for (const [id, tab] of this.tabs) tab.tab_id = id;
    for (const [id, stream] of this.streams) if (!this.tabs.has(stream.tabId)) await this.removeStream(id);
    if (!this.restoring) await this.save();
    return redactObservation({ state: "ready", generation: this.generation,
      tabs: [...this.tabs.values()].map(({ target_id, ...tab }) => tab), viewport }, this.protection.values);
  }
  async open(url, tabId = `host-tab-${randomUUID()}`, { signal } = {}) {
    await this.closeCompositors();
    if (this.tabs.size >= TAB_LIMIT) throw new Error("MD-2: host tab limit reached");
    const connection = await this.browser.ensureConnection();
    assertNotCancelled(signal);
    const { targetId } = await connection.send("Target.createTarget", { url: navigationUrl(url) });
    this.tabs.set(tabId, { tab_id: tabId, target_id: targetId, url, title: "", document_id: "" });
    const state = await this.reconcile();
    return { ...state, tab_id: tabId };
  }
  async target(command) {
    const started = timestamp();
    if (command.generation !== this.generation) throw new UserDomainRefusal("stale_epoch");
    await this.reconcile();
    this.timing('target_reconcile', started);
    const tab = this.tabs.get(command.tab_id);
    if (!tab) throw new UserDomainRefusal("not_granted");
    return tab;
  }
  async displayTarget(command) {
    if (command.generation !== this.generation) throw new UserDomainRefusal("stale_epoch");
    const stored = this.tabs.get(command.tab_id);
    if (!stored) throw new UserDomainRefusal("not_granted");
    const { connection, sessionId } = await this.browser.resolvePageTarget(stored.target_id);
    const { frameTree } = await connection.send("Page.getFrameTree", {}, sessionId);
    const document_id = frameTree?.frame?.loaderId;
    if (typeof document_id !== "string" || !document_id) throw new Error("MD-DISPLAY: document unavailable");
    // Observe this same native target without building an entire accessibility snapshot.
    if (stored.document_id !== document_id) {
      const { targetInfo } = await connection.send("Target.getTargetInfo", { targetId: stored.target_id });
      Object.assign(stored, { document_id, url: targetInfo.url, title: targetInfo.title });
      await this.save();
    }
    return { ...stored, document_id };
  }
  sampleLane(tab) {
    let lane=this.sampleLanes.get(tab.tab_id);
    if(!lane){lane=new SampleLane();this.sampleLanes.set(tab.tab_id,lane)}
    return lane;
  }
  // MP-08/MP-11: all display routes consume already-masked PNGs. Use the full
  // viewport so protected DOM bounds cannot shift with a page/crop origin.
  async displayScreenshot(tab,clip=null,optimizeForSpeed=true){
    return captureProtectedDisplay(this,tab,clip,optimizeForSpeed);
  }
  async screenshot(tab, clip = null, protectedCapture = false, format = "png", optimizeForSpeed = true) {
    const started = timestamp();
    const scale = this.scales.get(tab.tab_id) ?? 1;
    const { connection, sessionId } = await this.browser.resolvePageTarget(tab.target_id);
    await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
    const regionMasks = protectedCapture ? await captureRegionMasks(connection, sessionId) : null;
    const data = await captureProtectedPage(this.browser, tab, this.protection.values,
      this.protection.targets.filter(target => target.kind === "browser"), async () => {
        const at = timestamp();
        const sample = async () => {
          const compositor=this.compositors.get(tab.tab_id)?.source;
          if(clip)compositor?.pause();
          try{return await connection.send("Page.captureScreenshot", { format, ...(format === "jpeg" ? {quality:95} : {}), captureBeyondViewport: false, optimizeForSpeed, ...(clip ? { clip } : {}) }, sessionId)}
          finally{if(clip)compositor?.resume()}
        };
        const policy=this.protection;
        const {data}=await (!protectedCapture&&!clip&&!policy.unknown&&!policy.values.length&&!policy.targets.length ? sample() : this.sampleLane(tab).run("capture",sample));
        this.timing(clip?.scale < 1 ? 'cdp_preview' : clip ? 'cdp_crop' : 'cdp_capture', at);
        return data;
      }, scale, clip);
    await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
    const width = Math.round((clip?.width ?? geometry.width) * scale * (clip?.scale ?? 1));
    const height = Math.round((clip?.height ?? geometry.height) * scale * (clip?.scale ?? 1));
    const protected_regions = protectedCapture ? await regionMasks.afterCapture({ width, height }) : undefined;
    this.timing('protected_capture', started);
    if (typeof data !== "string" || data.length > 4 * 1024 * 1024) throw new Error("MD-2: frame exceeds limit");
    return { generation: this.generation, tab_id: tab.tab_id, document_id: tab.document_id, mime_type: `image/${format}`, data_base64: data, width, height, ...(protectedCapture ? { protected_regions } : {}) };
  }
  async subscribe(tab, boundFrames = false, owner = null) {
    await this.closeCompositors(tab.tab_id);
    if (this.streams.size >= 16) throw new Error("MD-2: frame subscription limit reached");
    const { connection, sessionId } = await this.browser.resolvePageTarget(tab.target_id);
    const id = `host-stream-${randomUUID()}`;
    const stream = { sessionId, tabId: tab.tab_id, boundFrames, owner, latest: null, sequence: 0, expires: Date.now() + 60_000 };
    const captureProtected = () => {
      stream.latest ??= this.maskedStreamFrame(stream);
      if (stream.capturing || this.protection.unknown || Date.now() < (stream.nextCapture ?? 0)) return;
      stream.capturing = true;
      stream.nextCapture = Date.now() + 200;
      const policy = this.protection, generation = this.generation,regionEpoch=stream.regionEpoch??0;
      void this.displayScreenshot(tab).then(frame => {
        if (this.protection === policy && this.generation === generation && this.streams.get(id) === stream&&(stream.regionEpoch??0)===regionEpoch) {
          if (!stream.boundFrames) delete frame.document_id;
          stream.latest = { ...frame, sequence: ++stream.sequence };
        }
      }).catch(() => {}).finally(() => { stream.capturing = false; });
    };
    stream.off = connection.subscribe(message => {
      if(regionProtectionChanged(message,sessionId,stream.latest?.[displayMaskRegions]?.length!==0)){
        if(protectionDeclared(message)){stream.regionEpoch=(stream.regionEpoch??0)+1;stream.latest=this.maskedStreamFrame(stream);}
        captureProtected();return;
      }
      if (message.method !== "Page.screencastFrame" || message.sessionId !== sessionId) return;
      const data = message.params?.data;
      if (typeof data === "string" && data.length <= 4 * 1024 * 1024 && Date.now() <= stream.expires) {
        // MP-11: raw screencast timestamps cannot bind protected DOM layout.
        // They wake the same masked capture path; no raw page bytes escape.
        captureProtected();
      }
      // One CDP source per page; subscriber fan-out must not duplicate ACKs.
      if ([...this.streams].find(([, current]) => current.sessionId === sessionId)?.[0] === id) {
        void connection.send("Page.screencastFrameAck", { sessionId: message.params?.sessionId }, sessionId).catch(() => {});
      }
    });
    const alreadyStreaming = [...this.streams.values()].some(current => current.sessionId === sessionId);
    this.streams.set(id, stream);
    this.armExpiry(id, stream);
    try { if (!alreadyStreaming) await connection.send("Page.startScreencast", { format: "jpeg", quality: 80, maxWidth: geometry.width, maxHeight: geometry.height, everyNthFrame: 1 }, sessionId); }
    catch (error) { clearTimeout(stream.timer); stream.off(); this.streams.delete(id); throw error; }
    captureProtected();
    return { generation: this.generation, subscription_id: id };
  }
  armExpiry(id, stream) {
    clearTimeout(stream.timer);
    stream.timer = setTimeout(() => { void this.removeStream(id); }, Math.max(1, stream.expires - Date.now()));
    stream.timer.unref?.();
  }
  armDisplayExpiry(stream) {
    clearTimeout(stream.timer);
    stream.timer = setTimeout(() => {
      if (this.displays.get(stream.subscription_id) !== stream) return;
      if(stream.source_kind==='desktop'){void this.desktopDisplay.remove(stream).catch(()=>{});return;}
      this.displays.delete(stream.subscription_id);
      void stream.close().then(() => {if(![...this.displays.values()].some(s=>s.tab_id===stream.tab_id))return this.closeCompositors(stream.tab_id)}).catch(() => {});
    }, Math.max(1, stream.expires - Date.now()));
    stream.timer.unref?.();
  }
  async removeStream(id) {
    const stream = this.streams.get(id);
    if (!stream) return;
    this.streams.delete(id);
    clearTimeout(stream.timer);
    stream.off();
    if (![...this.streams.values()].some(other => other.sessionId === stream.sessionId)) {
      await this.browser.connection?.send("Page.stopScreencast", {}, stream.sessionId).catch(() => {});
    }
  }
  observe(result, tab, scope) {
    let observed = this.observedDocuments.get(scope);
    if (!observed) {
      if (this.observedDocuments.size >= 64) this.observedDocuments.delete(this.observedDocuments.keys().next().value);
      observed = new Map(); this.observedDocuments.set(scope, observed);
    }
    const tabs = tab ? [tab] : [...this.tabs.values()];
    for (const current of tabs) observed.set(current.tab_id, current.document_id);
    for (const id of observed.keys()) if (!this.tabs.has(id)) observed.delete(id);
    return result;
  }
  async request(command, { signal } = {}) {
    const scope = command.observed_by ?? (command.focused_agent ? "focused-agent" : "adapter");
    assertNotCancelled(signal);
    if (command.op.startsWith("mirror_") && process.env.CHARIOX_KERNEL_BROWSER_MIRROR !== "1") throw new Error("MP-08: DOM mirror disabled");
    if (command.op.startsWith("display_") && process.env.CHARIOX_KERNEL_BROWSER_DISPLAY !== "1") throw new Error("MD-DISPLAY: experimental display disabled");
    if (command.op === "stop") return this.stop();
    // MP-11: observation never launches/relaunches the browser.
    await this.start({ signal, allowStart: !["state", "snapshot", "screenshot", "subscribe", "poll", "unsubscribe", "display_subscribe", "display_attach", "note_selection", "note_reanchor", "mirror_subscribe", "mirror_next", "mirror_close"].includes(command.op) });
    assertNotCancelled(signal);
    if (this.protection.unknown) throw new Error("MD-5: observation registry unavailable");
    for (const [id, stream] of this.streams) if (Date.now() > stream.expires) await this.removeStream(id);
    if (["start", "state"].includes(command.op)) return this.observe(await this.reconcile(), null, scope);
    if (command.op === "open") return this.observe(await this.open(command.url, undefined, { signal }), null, scope);
    for (const [id, stream] of this.displays) if (Date.now() > stream.expires) { if(stream.source_kind==='desktop')await this.desktopDisplay.remove(stream);else {await stream.close();this.displays.delete(id);} }
    if(command.op==='mirror_subscribe') return this.mirror.subscribe(command,scope);
    if(command.op==='mirror_next') return this.mirror.next(command,scope,{signal});
    if(command.op==='mirror_close') {this.mirror.require(command.subscription_id,scope,command.generation);this.mirror.streams.delete(command.subscription_id);return {closed:true};}
    const encodedCapture = command.op === "screenshot" && typeof command.display_subscription_id === "string";
    if (encodedCapture || command.op === "display_attach" || (command.op === "unsubscribe" && this.displays.has(command.subscription_id))) {
      const stream = this.displays.get(command.display_subscription_id ?? command.subscription_id);
      if (!stream || stream.observed_by !== scope || command.generation !== this.generation) throw new UserDomainRefusal("not_granted");
      if(stream.source_kind==='desktop')return this.desktopDisplay.request(command,scope,{signal});
      if (command.op === "unsubscribe") { await stream.close(); this.displays.delete(command.subscription_id); if(![...this.displays.values()].some(s=>s.tab_id===stream.tab_id))await this.closeCompositors(stream.tab_id); return { generation: this.generation, unsubscribed: true }; }
      stream.expires = Date.now() + 60_000;
      this.armDisplayExpiry(stream);
      if (command.op === "display_attach") return { attached: true, generation: this.generation };
      const cachedSource=this.compositors.get(stream.tab_id)?.source;
      const empty=()=>nativeCreditEmpty(stream,cachedSource,this.protection,this.inputEpochs.get(stream.tab_id)??0,
          this.inputChangedAt.get(stream.tab_id)??-Infinity,command.after_sequence);
      if(empty()){
        // MP-08/MP-10/MP-11: park one serial capture credit on source/codec
        // readiness during motion instead of exchanging hundreds of empty RPCs.
        // This is negative-only scheduling; every pixel still takes full fences.
        // Input's sparse source offer also wakes this bounded wait. Idle exact
        // credits must not poll the kernel/controller thousands of times/sec.
        await stream.producer.waitReady(20,signal);
        assertNotCancelled(signal);
        if(empty())return {generation:this.generation,frame_sent:false,display_frame:null};
      }
      const tab = await this.displayTarget({ tab_id: stream.tab_id, generation: command.generation });
      // Recreate the closure when navigation changes the loader binding. Pixels
      // and pending repairs are then invalidated by the new document as usual.
      if (!stream.capture || stream.capture.document !== tab.document_id)
        stream.capture = new DisplayCapture((clip) => {
          return this.displayScreenshot(tab,clip);
        }, stream.device_scale_factor, this.timing);
      const { connection, sessionId } = await this.browser.resolvePageTarget(tab.target_id);
      const layoutAt = timestamp();
      // MP-08/MP-10: an attested window supplies the complete viewport. Layout
      // is needed only to select safe CDP crops; a retired native source falls
      // back to full protected capture when no layout was read.
      const viewport = cachedSource?.attested ? null : (await connection.send("Page.getLayoutMetrics", {}, sessionId)).cssVisualViewport;
      this.timing('capture_layout_metrics', layoutAt);
      // Native CDP clips are page rectangles. Our private damage hints are
      // viewport rectangles; use full protected capture for scroll/zoom/unknown
      // origins until that coordinate transform has separate mask/race proof.
      const nativeCropSafe = viewport?.pageX === 0 && viewport?.pageY === 0 && viewport?.scale === 1;
      const capturePolicy=this.protection;
      const epoch = this.inputEpochs.get(tab.tab_id) ?? 0;
      const compositor=await this.compositorFor(tab,stream);
      const regionRevision=compositor?.regionRevision;
      if(stream.compositorRegionRevision!==regionRevision){
        // MP-11: no pixels may leave during the metadata fence. Keep only the
        // local canvas bookkeeping until a fresh capture decides whether its
        // masks/base are identical; pending codecs/refinements always retire.
        if(nativeRegionPending(stream,compositor,tab.document_id,this.protection)){
          stream.producer?.retireUnsent();stream.refiner?.invalidate();
          return {generation:this.generation,frame_sent:false,display_frame:null};
        }
        // MP-08/MP-10/MP-11: keep only a COMPLETE exact canvas after a fresh
        // post-fence capture proves the same masks and an exact source base.
        // Unsent old frames still retire; unknown/new geometry takes a key.
        const stable=nativeRegionBaseCurrent(stream,compositor,tab.document_id,this.protection);
        if(stable){stream.producer.retireUnsent();stream.refiner?.invalidate();}else stream.invalidate();
        stream.compositorRegionRevision=regionRevision;
      }
      const sample=compositor?.sample();
      let source;
      // MP-08/MP-10/MP-11: an admitted native source refreshing its masks
      // must not put an exact CDP capture in front of the next input credit.
      if(compositor&&!sample&&stream.codec!=='png')return {generation:this.generation,frame_sent:false,display_frame:null};
      if(compositor&&sample&&stream.codec!=='png'){
        // Serials are source-local. Retiring a source also retires its exact
        // base, even when a newly navigated document starts at the same serial.
        if(stream.producer?.source!==compositor){stream.invalidate();stream.document_id=null;await stream.producer?.close();stream.producer=new MotionEncoder(compositor,stream.encoder,{bitrate:stream.bitrate,codec:stream.codec,independent:!stream.dependencies,stripes:stream.stripes,shouldEncode:sample=>{const encode=!stream.canPatchNative(sample)&&!stream.shiftCandidate(sample)&&!(stream.exact&&identicalToDelivered(stream,sample));if(encode&&stream.exact)this.timing('shift_skipped_exact',timestamp());return encode;},valid:()=>!compositor.closed&&compositor.allowed(compositor.policy),timing:this.timing});}
        // Admit recovery before selecting a native patch or taking an encoded
        // packet. A lost canvas base also reoffers any skipped patchable source.
        // Once retired, empty credits must let the pending recovery key finish.
        if(stream.previous&&(stream.document_id!==tab.document_id||!stream.acceptsCredit(command.after_sequence)))stream.invalidate();
        if(!stream.refiner||stream.refinerDocument!==tab.document_id){await stream.refiner?.close();stream.refiner=new NativeRefiner(binding=>binding.native?binding.sample:this.displayScreenshot(tab,null,false),{now:()=>performance.now(),prepareTiles:true,timing:this.timing});stream.refinerDocument=tab.document_id;}
        const policy=this.protection;
        const binding={source:compositor,document:tab.document_id,policy,epoch,serial:sample.serial,scale:stream.device_scale_factor,native:compositor.attested===true&&Boolean(sample.raw),sample,repairLimit:exactPatchLimit(stream.bitrate),encoder:stream.encoder.nativeSession,nativeDelivered:stream.encoder.nativeDeliveredRevision};
        // Always run the deadline/epoch-aware verifier before unchanged reuse.
        // A lossy JPEG fingerprint cannot rule out fine native RGB damage.
        const nativeExact=binding.native&&stream.exact&&(sample.serial===stream.compositorSerial||stream.canPatchNative(sample));
        const exact=nativeExact?null:stream.refiner.request(binding,Math.max(compositor.changedAt,this.inputChangedAt.get(tab.tab_id)??-Infinity),()=>this.protection===policy&&compositor.sample()?.serial===sample.serial&&(this.inputEpochs.get(tab.tab_id)??0)===epoch);
        stream.creditEpoch=epoch;
        stream.producer.feedback(Math.max(0,stream.sequence-command.after_sequence));
        // MP-08/MP-10: an identical readback (protection refresh) of the
        // delivered exact canvas with the same masks needs no frame.
        if(stream.exact&&identicalToDelivered(stream,sample)&&JSON.stringify(sample.raw[displayMaskRegions]??[])===stream.compositorMasks){
          stream.compositorSerial=sample.serial;
          return {generation:this.generation,frame_sent:false,display_frame:null};
        }
        const patchable=stream.canPatchNative(sample);
        const shift=patchable?null:stream.shiftKind(sample);
        const encoded=patchable||shift ? null : stream.producer.take();
        if(shift){
          // MP-08/MP-10: lossless scroll frame: proved moves plus WebP residuals.
          stream.producer.retireUnsent();
          const reply=await sample.raw.nativeExact({encoder:stream.encoder.nativeSession,regions:[],patch:true,shift,effort:stream.shiftEffort(),...(shift==='overlay'?{base:stream.compositorCommittedSerial??stream.compositorSerial}:{})});
          if(reply.shift_refused===true){
            this.timing(`shift_refused_${shift} ${/^MP-1[01]: [a-z ]{1,40}$/.test(reply.reason)?reply.reason:''}`,timestamp());
            // A refused plan holds lossless frames until exactness returns
            // and re-offers the latest sample to the video encoder.
            stream.shiftHold=true;stream.producer.retry(compositor.sample()??sample);
            return {generation:this.generation,frame_sent:false,display_frame:null};
          }
          if(reply.native_packet)stream.encoder.adoptPacket(reply.native_packet);
          if(!Array.isArray(reply.native_tiles)||!Array.isArray(reply.moves)||reply.moves.length>64||reply.native_tiles.length>64||!(reply.moves.length||reply.native_tiles.length)){stream.encoder.discard?.({packet:reply.native_packet});throw Error('MP-11: native shift reply');}
          source={...sample,...reply,...(reply.moves.length?{}:{moves:undefined}),motion:false,generation:this.generation};
        }else if(patchable){
          stream.producer.retireUnsent();
          const adjacent=sample.serial===(stream.exact?(stream.compositorCommittedSerial??stream.compositorSerial):stream.compositorSerial)+1&&stream.compositorMasks===JSON.stringify(sample.raw[displayMaskRegions]??[])&&nativeDamageTiles(sample.raw,true,true)!==null;
          const patch=sample.raw.nativeExact?await sample.raw.nativeExact({encoder:stream.encoder.nativeSession,regions:sample.raw[displayMaskRegions]??[],patch:true,adjacent}):{native_tiles:nativeDamageTiles(sample.raw)};
          source={...sample,...patch,motion:false,generation:this.generation};
        }
        else if(encoded){source={...encoded,generation:this.generation};}
        else if(exact&&stream.previous)source={...exact,generation:this.generation};
        // The viewer already backs off empty credits. A second delay while
        // holding the capture gate adds latency to every pipelined slot.
        else return {generation:this.generation,frame_sent:false,display_frame:null};
      }else{
        source=await stream.capture.next({...tab,input_epoch:epoch},this.protection,stream.previous&&stream.acceptsCredit(command.after_sequence),!stream.exact||!nativeCropSafe,
          stream.codec!=='png'&&this.protection.values.length===0&&viewport?.scale===1&&Number.isFinite(viewport.pageX)&&Number.isFinite(viewport.pageY)?{x:viewport.pageX,y:viewport.pageY,width:geometry.width,height:geometry.height,scale:1/stream.device_scale_factor,display_motion:true}:null,
          this.scrolling.get(tab.tab_id)?.document_id===tab.document_id&&this.scrolling.get(tab.tab_id).until>performance.now());
      }
      // MP-10: only a post-dispatch native capture can claim the input burst.
      const inputAt=this.inputChangedAt.get(tab.tab_id)??-Infinity;
      source.input_triggered=Number.isFinite(inputAt)&&epoch!==stream.deliveredInputEpoch&&Number.isFinite(source.captured_ms)&&source.captured_ms>=performance.timeOrigin+inputAt;
      try { await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id); }
      catch (error) { stream.encoder.discard?.(source.encoded);stream.invalidate(); throw error; }
      assertNotCancelled(signal);
      const frame = await stream.frame(source, source.document_id, command.after_sequence, async () => {
        assertNotCancelled(signal);
        await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
        return !compositor?.closed&&compositor?.regionRevision===regionRevision&&(source.motion || ((this.inputEpochs.get(tab.tab_id) ?? 0) === epoch&&((source.refinement_serial===undefined||compositor?.sample()?.serial===source.refinement_serial)&&(source.native_revision===undefined||source.native_revision===stream.encoder.nativeRevision))));
      },()=>!compositor?.closed&&compositor?.regionRevision===regionRevision&&this.protection===capturePolicy&&(source.motion||((this.inputEpochs.get(tab.tab_id)??0)===epoch&&((source.refinement_serial===undefined||compositor?.sample()?.serial===source.refinement_serial)&&(source.native_revision===undefined||source.native_revision===stream.encoder.nativeRevision)))));
      if(frame){stream.compositorMasks=JSON.stringify(source.raw?.[displayMaskRegions]??[]);stream.compositorSerial=source.refinement_serial ?? source.serial;if(source.input_triggered)stream.deliveredInputEpoch=epoch;if(stream.exact&&source.raw?.nativeCommit){source.raw.nativeCommit(stream.encoder.nativeSession,!source.moves);stream.compositorCommittedSerial=stream.compositorSerial;}}
      compositor?.plans?.(stream.exact&&!stream.shiftHold&&stream.compositorMasks==='[]');
      return { generation: this.generation, frame_sent: frame !== null, display_frame: frame };
    }
    if (["poll", "unsubscribe"].includes(command.op)) {
      if (command.generation !== this.generation || !this.streams.has(command.subscription_id)) throw new UserDomainRefusal("stale_reference");
      const stream = this.streams.get(command.subscription_id);
      stream.expires = Date.now() + 60_000;
      this.armExpiry(command.subscription_id, stream);
      if (command.op === "unsubscribe") { await this.removeStream(command.subscription_id); return { generation: this.generation, unsubscribed: true }; }
      return { generation: this.generation, frame: stream.latest };
    }
    // Explicit physical input already names an observed tab/document. Read
    // that live target directly; whole-tab discovery still runs afterward.
    // Per-dispatch document/cancellation and secret-focus checks remain below.
    const tab = command.op === 'input' && typeof command.document_id === 'string'
      ? await this.displayTarget(command) : await this.target(command);
    assertNotCancelled(signal);
    if (["input", "navigate", "close"].includes(command.op) && (command.focused_agent || command._agent_input)
      && [...(this.browser.appTabs?.apps?.values() ?? [])].some(app => app.targetId === tab.target_id)) {
      throw new UserDomainRefusal("not_granted");
    }
    const binding = { target_id: tab.target_id, document_id: tab.document_id };
    if (command.op === "display_subscribe") {
      if (process.env.CHARIOX_KERNEL_BROWSER_DISPLAY !== "1") throw new Error("MD-DISPLAY: experimental display disabled");
      if (!Array.isArray(command.codecs) || !command.codecs.includes("png") || command.codecs.length > 8 ||
        !Number.isInteger(command.bitrate) || command.bitrate < 500_000 || command.bitrate > 64_000_000 ||
        ![1, 2].includes(command.device_scale_factor) || (geometry.width===1920&&command.device_scale_factor!==1) || this.displays.size >= 8) throw new Error("MD-DISPLAY: invalid display negotiation");
      const scale = this.scales.get(tab.tab_id);
      if (scale && scale !== command.device_scale_factor) throw new Error("MD-DISPLAY: canonical tab geometry is already selected");
      const { connection, sessionId } = await this.browser.resolvePageTarget(tab.target_id);
      await connection.send("Emulation.setDeviceMetricsOverride", displayDeviceMetrics(geometry.width,geometry.height,command.device_scale_factor), sessionId);
      this.scales.set(tab.tab_id, command.device_scale_factor);
      const id = `host-display-${randomUUID()}`;
      const codec=process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER&&command.codecs.includes('avc1.420033')?'avc1.420033':command.codecs.find(c=>['vp8','vp09.00.50.08','vp09.00.40.08','vp09.00.10.08','avc1.420033'].includes(c))??'png';
      const stream = new DisplayStream({ relay_binary:command.codecs.includes('chariox-relay-binary-v96'), subscription_id: id, tab_id: tab.tab_id, observed_by: scope, bitrate: command.bitrate, device_scale_factor: command.device_scale_factor, codec, css_width:geometry.width, css_height:geometry.height, dependencies:command.codecs.includes('chariox-video-dependencies-v1'),stripes:command.codecs.includes('chariox-stripes-v1')&&['avc1.420033','vp8'].includes(codec) }, { timing:this.timing,encoder:new BrowserEncoder(this.browser,tab.target_id) });
      this.displays.set(id, stream);
      this.armDisplayExpiry(stream);
      return { generation: this.generation, subscription_id: id, codec, bitrate: command.bitrate, device_scale_factor: command.device_scale_factor };
    }
    if (command.op === "close") {
      this.mirror.removeTab(tab.tab_id);
      for (const [id, stream] of this.displays) if (stream.tab_id === tab.tab_id) { await stream.close(); this.displays.delete(id); }
      await this.closeCompositors(tab.tab_id);
      this.sampleLanes.delete(tab.tab_id);
      this.scales.delete(tab.tab_id);
      this.inputEpochs.delete(tab.tab_id);
      this.scrolling.delete(tab.tab_id);
      for (const [id, stream] of this.streams) {
        if (stream.tabId === tab.tab_id) await this.removeStream(id);
      }
      await this.browser.manageTab({ ...binding, action: "close" }, { signal });
      for (const observed of this.observedDocuments.values()) observed.delete(tab.tab_id);
      return this.reconcile();
    }
    if (command.op === "navigate") {
      await this.browser.navigate({ ...binding, url: navigationUrl(command.url) }, { signal });
      return this.observe(await this.reconcile(), null, scope);
    }
    if (command.op === "note_selection" || command.op === "note_reanchor") return this.observe({ generation:this.generation, observation:await this.browser.observeNote({ ...binding, ...(command.quote ? {quote:command.quote} : {}) }) },tab,scope);
    if (command.op === "snapshot") return this.observe({ generation: this.generation, snapshot: await this.browser.snapshot(binding) }, tab, scope);
    if (command.op === "input") {
      const observed = command.document_id ?? (command.focused_agent ? null : this.observedDocuments.get(scope)?.get(tab.tab_id));
      if (!observed || observed !== tab.document_id) throw new UserDomainRefusal("stale_reference");
      const at = timestamp();
      let dispatched = false;
      // MD-DISPLAY-02/04: input retires exact verification through its epoch.
      // MP-08/MP-10: all physical input keeps bounded video dependencies.
      // Retiring an encoded row on each key forces an IDR and can delay echo
      // behind its byte budget. Epochs still retire exact work; policy/document
      // retirement still closes the source/producer and resets every row.
      const onDispatch = () => {
        if (dispatched) return;
        dispatched = true;
        this.inputChangedAt.set(tab.tab_id, performance.now());
        this.inputEpochs.set(tab.tab_id, (this.inputEpochs.get(tab.tab_id) ?? 0) + 1);
        this.compositors.get(tab.tab_id)?.source?.wake?.();
      };
      try {
        // MP-08/MP-10/MP-11: agent and ordinary input must await CDP's ack,
        // including the lifetime of hidden-target focus emulation. Only an
        // admitted human viewer with a live display lease owns the fast path.
        const viewerActive=()=>command._display_input===true&&!command._agent_input&&!command.focused_agent&&
          [...this.displays.values()].some(s=>s.tab_id===tab.tab_id&&s.observed_by===scope&&s.expires>Date.now());
        const owned=this.compositors.get(tab.tab_id)?.source;
        const nativeWheel=viewerActive()&&owned?.attested&&typeof owned.wheel==='function'?(x,y,dx,dy)=>viewerActive()&&owned.wheel(x,y,dx,dy):null;
        const deferred=await this.sampleLane(tab).run("input", () => inputHostTab(this.browser, tab, command.input, { signal, onDispatch, asyncScroll: ()=>viewerActive()&&owned?.attested&&typeof owned.valid==='function'&&owned.valid(), nativeWheel, resolveMirror: input => this.mirror.resolveInput(tab,input,scope,signal) }));
        // MP-08/MP-10: wheel input is asynchronous, as in a native browser. The
        // fenced, ledgered dispatch is ordered by CDP; the renderer's
        // frame-aligned ack would otherwise serialize kernel input admission.
        deferred?.ack?.catch(()=>{});
        if(dispatched)this.compositors.get(tab.tab_id)?.source?.wake?.();
        this.timing('cdp_input', at);
      }
      catch (error) {
        if (dispatched || ["browser_action_cancelled", "stale_document_reference"].includes(error?.code)) {
          // Clear any dispatched key/button state before another actor can use
          // the browser. Recovery rotates generation; cancelled input never replays.
          await this.stop();
        }
        throw error;
      }
      const post = timestamp();
      if(command.input.kind === 'scroll')this.scrolling.set(tab.tab_id,{document_id:tab.document_id,until:performance.now()+400});
      const result = this.observe(await this.reconcile(), null, scope);
      this.timing('input_post_reconcile', post);
      return result;
    }
    if (command.op === "screenshot") {
      const frame = command._capture_protection===true?await this.screenshot(tab,null,true):await this.displayScreenshot(tab);
      // MD-3: explicit binding is an internal display/MCP seam. Legacy 417
      // still emits its existing frame shape until the coordinator adapter lands.
      if (!command.focused_agent && !command.bound_frames) delete frame.document_id;
      return this.observe(frame, tab, scope);
    }
    if (command.op === "subscribe") return this.subscribe(tab, command.bound_frames === true, command._subscription_owner ?? null);
    throw new Error("MD-2: unsupported browser operation");
  }
  async handle(request, { signal } = {}) {
    try { assertNotCancelled(signal); } catch { return { id: request.id, ok: false, error: { code: "browser_action_cancelled", message: "MD-3: browser action cancelled" } }; }
    try {
      if (request.method === "health") return { id: request.id, ok: true, result: { state: "ready", process_id: process.pid, diagnostic_code: null } };
      if (request.method === "shutdown") return { id: request.id, ok: true, result: await this.stop() };
      if (request.method === "host.protect") return { id: request.id, ok: true, result: await this.protect(request.params) };
      if (request.method === "host.revoke_subscriptions") {
        const ids = new Set(request.params.subscription_ids ?? []);
        const owners = new Set(request.params.subscription_owners ?? []);
        for(const owner of owners){await this.nativeComputer.retire(owner);for(const observer of this.nativeAccessibility.observers.keys())if(observer.startsWith(owner+':'))this.nativeAccessibility.retire(observer);}
        for (const [id, stream] of this.streams) if (ids.has(id) || owners.has(stream.owner)) await this.removeStream(id);
        return { id: request.id, ok: true, result: { revoked: true } };
      }
      if (request.method === "host.secret") {
        await this.start({ signal });
        if (this.protection.unknown) throw new Error("MD-5: observation registry unavailable");
        const tab = await this.target(request.params);
        if (tab.document_id !== request.params.document_id) throw new UserDomainRefusal("stale_reference");
        await this.sampleLane(tab).run("input", () => this.browser.performAction({ target_id: tab.target_id, document_id: tab.document_id,
          node_ref: request.params.node_ref, action: request.params.action, timeout_ms: 10_000 }, { signal }));
        await this.save();
        return { id: request.id, ok: true, result: { inserted: true } };
      }
      if (request.method === "host.computer") {
        const observer=(request.params._subscription_owner ? request.params._subscription_owner+':' : '')+(request.params.observed_by??'terminal');
        if(request.params._agent_input && this.browser?.appTabs?.apps?.size)throw new UserDomainRefusal('not_granted');
        const nativeParams={...request.params,observed_by:observer};
        if (request.params.op === 'display_subscribe') {
          if(process.env.CHARIOX_KERNEL_BROWSER_DISPLAY!=='1')throw new Error('MP-08: experimental display disabled');
          return {id:request.id,ok:true,result:await this.desktopDisplay.subscribe(nativeParams,request.params.observed_by??'terminal')};
        }
        if (request.params.op === 'snapshot') {
          const binding=this.chromium.desktop?.binding();
          if(!binding || request.params.surface_id!==binding.surface_id || request.params.generation!==binding.generation)throw new Error('MP-11: stale native snapshot surface');
          return {id:request.id,ok:true,result:await this.nativeAccessibility.snapshot(observer,this.protection,{signal})};
        }
        if (request.params.op === 'target_action') {
          const binding=this.chromium.desktop?.binding();
          if(!binding || request.params.surface_id!==binding.surface_id || request.params.generation!==binding.generation)throw new Error('MP-11: stale native target surface');
          return {id:request.id,ok:true,result:await this.nativeAccessibility.action(observer,request.params,this.protection,{signal})};
        }
        if (request.params.op === 'start') {
          if(!this.chromium.desktop || this.chromium.environment?.CHARIOX_KERNEL_BROWSER_HEADLESS==='1')throw new Error('MP-08: owned Computer desktop requires headed Linux');
          await this.start({ signal });
          return { id: request.id, ok: true, result: await this.nativeComputer.request({op:'state'}, this.protection, {signal}) };
        }
        return { id: request.id, ok: true, result: await this.nativeComputer.request(nativeParams, this.protection, {signal}) };
      }
      if (request.method === 'host.computer.retire') {
        await this.desktopDisplay.retire(request.params.observer);
        await this.nativeComputer.retire(request.params.observer);
        this.nativeAccessibility.retire(request.params.observer);
        return {id:request.id,ok:true,result:{retired:true}};
      }
      if (request.method === "host.computer.reset") {
        await this.nativeComputer.reset();
        return { id: request.id, ok: true, result: { released: true } };
      }
      if (request.method === "host.browser") {
        if(this.nativeComputer.held.size && ['input','open','close','navigate'].includes(request.params.op))throw new Error('MP-11: native keys held; release desktop input first');
        const at = timestamp();
        const result = await this.request(request.params, { signal });
        this.timing(request.params.op === 'input' ? 'host_input' : 'host_capture_or_control', at);
        // Structured controller observations scrub before compaction; metadata
        // and other host replies receive the same protection at this boundary.
        if (!["screenshot", "poll", "mirror_next"].includes(request.params?.op)) {
          return { id: request.id, ok: true, result: redactObservation(result, this.protection.values) };
        }
        return { id: request.id, ok: true, result };
      }
      // Kernel-internal App adapter; no public raw CDP dispatch.
      if (request.method.startsWith("browser.app.")) {
        const { _host_generation: generation, ...params } = request.params ?? {};
        // Bound ephemeral instances cannot start/restore a stopped or crashed
        // browser. Only an explicit Open or ordinary tab request may start it.
        if (generation !== undefined && (!this.browser || this.chromium.child?.exitCode !== null
          || this.chromium.child?.signalCode !== null)) throw new Error("App host is no longer live");
        await this.start({ signal });
        if (generation !== undefined && generation !== this.generation) throw new UserDomainRefusal("stale_epoch");
        const result = await handleBrowserControllerRequest({ ...request, params }, { browser: this.browser, signal });
        if (result.ok) {
          await this.reconcile();
          if (request.method === "browser.app.open") {
            result.result.tab_id = [...this.tabs.values()].find(tab => tab.target_id === result.result.target_id)?.tab_id;
            result.result.generation = this.generation;
          }
        }
        return result;
      }
      throw new Error("MD-2: unsupported host method");
    } catch (error) {
      // MP-11: only fixed error classes/codes and public source positions.
      this.timing(hostErrorLabel(error),timestamp());
      if (error instanceof UserDomainRefusal) return {id:request.id,ok:false,error:{code:error.code,message:error.message}};
      if (isTrustedStaleReferenceError(error)) return {id:request.id,ok:false,error:{code:"user_domain_stale_reference",message:"User-domain request refused"}};
      if (error?.code === "browser_action_cancelled" && request.method !== "host.computer" && !request.params?.display_subscription_id) await this.stop();      if (["browser_unavailable"].includes(error?.code)) {
        return { id: request.id, ok: false, error: { code: error.code, message: error.message } };
      }
      return { id: request.id, ok: false, error: { code: error?.code === "browser_action_cancelled" ? "browser_action_cancelled" : "kernel_browser_failed", message: error instanceof MirrorInputEpochRefusal ? "MP-11: stale mirror input epoch" : "MD-2: host browser operation failed; refresh state or check host browser readiness" } }; }
  }
}

if (process.argv[2] === "stdio" && process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const host = new KernelBrowserHost(process.argv[3]);
  let closing = false;
  const stop = async () => { if (closing) return; closing = true; await host.stop(); process.exit(0); };
  process.on("SIGTERM", stop);
  process.on("SIGINT", stop);
  await new BrowserControllerStdioServer({ scheduleRequest: scheduleHostRequest, handleRequest: (request, options) => host.handle(request, options) }).run().finally(() => host.stop());
}
