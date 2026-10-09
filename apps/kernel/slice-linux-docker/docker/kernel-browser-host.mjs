// MD-2: sessionless host adapter over the shared controller/CDP implementation.
import { UserDomainRefusal } from "./kernel-browser-refusal.mjs";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { displayTiming, timestamp } from './kernel-browser-timing.mjs';
import { BrowserCdpClient, isTrustedStaleReferenceError } from "./browser-controller-cdp.mjs";
import { BrowserControllerStdioServer, handleBrowserControllerRequest } from "./browser-controller.mjs";
import { NativeAccessibility } from './native-accessibility.mjs';
import { NativeComputer } from './native-computer.mjs';
import { HostChromium } from "./kernel-browser-process.mjs";
import { redactObservation } from "./browser-controller-snapshot.mjs";
import { inputHostTab } from "./kernel-browser-input.mjs";
import { assertNotCancelled, assertCurrentDocument, BrowserActionError } from "./browser-controller-actions.mjs";
import { captureRegionMasks } from "./kernel-browser-region-protection.mjs";
import { captureProtectedPage, wholeFrameMask } from "./kernel-browser-pixels.mjs";

import { MirrorService, MirrorInputEpochRefusal } from "./kernel-browser-mirror.mjs";
import { DisplayStream } from "./kernel-browser-display.mjs";
import { DisplayCapture } from './kernel-browser-display-capture.mjs';

const TAB_LIMIT = 128;
function restorationUrl(url) {
  try { return navigationUrl(url); } catch { return "about:blank"; }
}

const viewport = { css_width: 1280, css_height: 800, device_scale_factor: 1,
  desktop_pixel_width: 1280, desktop_pixel_height: 800 };
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
    this.inputEpochs = new Map();
    this.restoring = false;
    this.keepaliveTarget = null;
    this.observedDocuments = new Map();
    this.mirror = new MirrorService(this);
    this.protection = { values: [], targets: [], unknown: false };
    // MP-08/MP-11: desktop consumers place CDP protection regions on X11 pixels.
    if (this.chromium.desktop) this.chromium.desktop.browser = () => this.browser;
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
    this.nativeAccessibility.clear();
    this.mirror.invalidate();
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
  async save() {
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
  async stop() {
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
  async screenshot(tab, clip = null, protectedCapture = false) {
    const started = timestamp();
    const scale = this.scales.get(tab.tab_id) ?? 1;
    const { connection, sessionId } = await this.browser.resolvePageTarget(tab.target_id);
    await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
    const regionMasks = protectedCapture ? await captureRegionMasks(connection, sessionId) : null;
    const data = await captureProtectedPage(this.browser, tab, this.protection.values,
      this.protection.targets.filter(target => target.kind === "browser"), async () => {
        const at = timestamp();
        const { data } = await connection.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false, optimizeForSpeed: true, ...(clip ? { clip } : {}) }, sessionId);
        this.timing(clip?.scale < 1 ? 'cdp_preview' : clip ? 'cdp_crop' : 'cdp_capture', at);
        return data;
      }, scale, clip);
    await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
    const width = Math.round((clip?.width ?? 1280) * scale * (clip?.scale ?? 1));
    const height = Math.round((clip?.height ?? 800) * scale * (clip?.scale ?? 1));
    const protected_regions = protectedCapture ? await regionMasks.afterCapture({ width, height }) : undefined;
    this.timing('protected_capture', started);
    if (typeof data !== "string" || data.length > 4 * 1024 * 1024) throw new Error("MD-2: frame exceeds limit");
    return { generation: this.generation, tab_id: tab.tab_id, document_id: tab.document_id, mime_type: "image/png", data_base64: data, width, height, ...(protectedCapture ? { protected_regions } : {}) };
  }
  async subscribe(tab, boundFrames = false, owner = null) {
    if (this.streams.size >= 16) throw new Error("MD-2: frame subscription limit reached");
    const { connection, sessionId } = await this.browser.resolvePageTarget(tab.target_id);
    const id = `host-stream-${randomUUID()}`;
    const stream = { sessionId, tabId: tab.tab_id, boundFrames, owner, latest: null, sequence: 0, expires: Date.now() + 60_000 };
    const captureProtected = () => {
      stream.latest ??= this.maskedStreamFrame(stream);
      if (stream.capturing || this.protection.unknown || Date.now() < (stream.nextCapture ?? 0)) return;
      stream.capturing = true;
      stream.nextCapture = Date.now() + 200;
      const policy = this.protection, generation = this.generation;
      void this.screenshot(tab).then(frame => {
        if (this.protection === policy && this.generation === generation && this.streams.get(id) === stream) {
          if (!stream.boundFrames) delete frame.document_id;
          stream.latest = { ...frame, sequence: ++stream.sequence };
        }
      }).catch(() => {}).finally(() => { stream.capturing = false; });
    };
    stream.off = connection.subscribe(message => {
      if (message.method !== "Page.screencastFrame" || message.sessionId !== sessionId) return;
      const data = message.params?.data;
      if (typeof data === "string" && data.length <= 4 * 1024 * 1024 && Date.now() <= stream.expires) {
        const protectedPixels = this.protection.unknown || this.protection.values.length > 0;
        if (!protectedPixels && !stream.boundFrames) {
          stream.latest = { generation: this.generation, tab_id: tab.tab_id, mime_type: "image/jpeg", data_base64: data,
            width: viewport.css_width, height: viewport.css_height, sequence: ++stream.sequence };
        } else {
          // Raw screencast timestamps cannot bind a masking layout. Use the
          // same protected screenshot path, triggered by screencast activity.
          // One capture at a time, at most 5Hz; never queue page frames.
          captureProtected();
        }
      }
      // One CDP source per page; subscriber fan-out must not duplicate ACKs.
      if ([...this.streams].find(([, current]) => current.sessionId === sessionId)?.[0] === id) {
        void connection.send("Page.screencastFrameAck", { sessionId: message.params?.sessionId }, sessionId).catch(() => {});
      }
    });
    const alreadyStreaming = [...this.streams.values()].some(current => current.sessionId === sessionId);
    this.streams.set(id, stream);
    this.armExpiry(id, stream);
    try { if (!alreadyStreaming) await connection.send("Page.startScreencast", { format: "jpeg", quality: 80, maxWidth: 1280, maxHeight: 800, everyNthFrame: 1 }, sessionId); }
    catch (error) { clearTimeout(stream.timer); stream.off(); this.streams.delete(id); throw error; }
    if (this.protection.unknown || this.protection.values.length || boundFrames) captureProtected();
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
      this.displays.delete(stream.subscription_id);
      void stream.close().catch(() => {});
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
    for (const [id, stream] of this.displays) if (Date.now() > stream.expires) { await stream.close(); this.displays.delete(id); }
    if(command.op==='mirror_subscribe') return this.mirror.subscribe(command,scope);
    if(command.op==='mirror_next') return this.mirror.next(command,scope,{signal});
    if(command.op==='mirror_close') {this.mirror.require(command.subscription_id,scope,command.generation);this.mirror.streams.delete(command.subscription_id);return {closed:true};}
    const encodedCapture = command.op === "screenshot" && typeof command.display_subscription_id === "string";
    if (encodedCapture || command.op === "display_attach" || (command.op === "unsubscribe" && this.displays.has(command.subscription_id))) {
      const stream = this.displays.get(command.display_subscription_id ?? command.subscription_id);
      if (!stream || stream.observed_by !== scope || command.generation !== this.generation) throw new UserDomainRefusal("not_granted");
      if (command.op === "unsubscribe") { await stream.close(); this.displays.delete(command.subscription_id); return { generation: this.generation, unsubscribed: true }; }
      stream.expires = Date.now() + 60_000;
      this.armDisplayExpiry(stream);
      if (command.op === "display_attach") return { attached: true, generation: this.generation };
      const tab = await this.displayTarget({ tab_id: stream.tab_id, generation: command.generation });
      stream.capture ??= new DisplayCapture((clip, currentTab) => this.screenshot(currentTab, clip), stream.device_scale_factor, this.timing);
      const source = await stream.capture.next({ ...tab, input_epoch: this.inputEpochs.get(tab.tab_id) ?? 0 }, this.protection, stream.previous && command.after_sequence === stream.sequence, !stream.exact);
      const { connection, sessionId } = await this.browser.resolvePageTarget(tab.target_id);
      try { await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id); }
      catch (error) { stream.invalidate(); throw error; }
      const frame = await stream.frame(source, source.document_id, command.after_sequence);
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
    const tab = await this.target(command);
    assertNotCancelled(signal);
    if (["input", "navigate", "close"].includes(command.op) && (command.focused_agent || command._agent_input)
      && [...(this.browser.appTabs?.apps?.values() ?? [])].some(app => app.targetId === tab.target_id)) {
      throw new UserDomainRefusal("not_granted");
    }
    const binding = { target_id: tab.target_id, document_id: tab.document_id };
    if (command.op === "display_subscribe") {
      if (process.env.CHARIOX_KERNEL_BROWSER_DISPLAY !== "1") throw new Error("MD-DISPLAY: experimental display disabled");
      if (!Array.isArray(command.codecs) || !command.codecs.includes("png") || command.codecs.length > 8 ||
        !Number.isInteger(command.bitrate) || command.bitrate < 500_000 || command.bitrate > 8_000_000 ||
        ![1, 2].includes(command.device_scale_factor) || this.displays.size >= 8) throw new Error("MD-DISPLAY: invalid display negotiation");
      const scale = this.scales.get(tab.tab_id);
      if (scale && scale !== command.device_scale_factor) throw new Error("MD-DISPLAY: canonical tab geometry is already selected");
      const { connection, sessionId } = await this.browser.resolvePageTarget(tab.target_id);
      await connection.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 800, deviceScaleFactor: command.device_scale_factor, mobile: false }, sessionId);
      this.scales.set(tab.tab_id, command.device_scale_factor);
      const id = `host-display-${randomUUID()}`;
      const codec = command.codecs.includes("vp09.00.10.08") ? "vp09.00.10.08" : "png";
      const stream = new DisplayStream({ subscription_id: id, tab_id: tab.tab_id, observed_by: scope, bitrate: command.bitrate, device_scale_factor: command.device_scale_factor, codec }, { timing: this.timing });
      this.displays.set(id, stream);
      this.armDisplayExpiry(stream);
      return { generation: this.generation, subscription_id: id, codec, bitrate: command.bitrate, device_scale_factor: command.device_scale_factor };
    }
    if (command.op === "close") {
      this.mirror.removeTab(tab.tab_id);
      for (const [id, stream] of this.displays) if (stream.tab_id === tab.tab_id) { await stream.close(); this.displays.delete(id); }
      this.scales.delete(tab.tab_id);
      this.inputEpochs.delete(tab.tab_id);
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
      try { await inputHostTab(this.browser, tab, command.input, { signal, onDispatch: () => { dispatched = true; }, resolveMirror: input => this.mirror.resolveInput(tab,input,scope,signal) }); this.timing('cdp_input', at); }
      catch (error) {
        if (dispatched || ["browser_action_cancelled", "stale_document_reference"].includes(error?.code)) {
          // Clear any dispatched key/button state before another actor can use
          // the browser. Recovery rotates generation; cancelled input never replays.
          await this.stop();
        }
        throw error;
      }
      const post = timestamp();
      this.inputEpochs.set(tab.tab_id, (this.inputEpochs.get(tab.tab_id) ?? 0) + 1);
      const result = this.observe(await this.reconcile(), null, scope);
      this.timing('input_post_reconcile', post);
      return result;
    }
    if (command.op === "screenshot") {
      const frame = await this.screenshot(tab, null, command._capture_protection === true);
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
        await this.browser.performAction({ target_id: tab.target_id, document_id: tab.document_id,
          node_ref: request.params.node_ref, action: request.params.action, timeout_ms: 10_000 }, { signal });
        await this.save();
        return { id: request.id, ok: true, result: { inserted: true } };
      }
      if (request.method === "host.computer") {
        const observer=(request.params._subscription_owner ? request.params._subscription_owner+':' : '')+(request.params.observed_by??'terminal');
        if(request.params._agent_input && this.browser?.appTabs?.apps?.size)throw new UserDomainRefusal('not_granted');
        const nativeParams={...request.params,observed_by:observer};
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
      if (error instanceof UserDomainRefusal) return {id:request.id,ok:false,error:{code:error.code,message:error.message}};
      if (isTrustedStaleReferenceError(error)) return {id:request.id,ok:false,error:{code:"user_domain_stale_reference",message:"User-domain request refused"}};
      if (error?.code === "browser_action_cancelled" && request.method !== "host.computer") await this.stop();
      if (["browser_unavailable"].includes(error?.code)) {
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
  await new BrowserControllerStdioServer({ handleRequest: (request, options) => host.handle(request, options) }).run().finally(() => host.stop());
}
