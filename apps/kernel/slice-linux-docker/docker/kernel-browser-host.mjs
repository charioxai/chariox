import {resolveDisplayTarget,nativeStateObservationOptions} from './kernel-browser-display-document.mjs';
import {capturePageScreenshot} from './kernel-browser-screenshot.mjs';
import { mirror2ObserverExpression } from './kernel-browser-mirror2-observer.mjs';
import {NavigationDisplays} from './kernel-browser-navigation-display.mjs';
import {captureVisiblePage,pngFallbackCommand,subscribeVisiblePage,visiblePageInputSource,viewerPageInputActive,nativeInputDocumentTarget} from './kernel-browser-page-display.mjs';
import {reserveDisplay} from './kernel-browser-display-admission.mjs';
import { appRestoreOrigin } from "./browser-app-restore.mjs";
import { DesktopDisplay } from './kernel-desktop-display.mjs';
import {displayMaskRegions} from './kernel-browser-pixels.mjs';
import {displayGeometry as geometry,hostViewport,featureOn} from './kernel-browser-geometry.mjs';
// MD-2: sessionless host adapter over the shared controller/CDP implementation.
import { CaptureRefusal, PointerOutsideViewport, UserDomainRefusal } from "./kernel-browser-refusal.mjs";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { displayTiming, slowMark, timestamp, timedHostRequests } from './kernel-browser-timing.mjs';
import { hostErrorLabel, hostFailureMessage } from './kernel-browser-error-label.mjs';
import { BrowserCdpClient, isTrustedStaleReferenceError } from "./browser-controller-cdp.mjs";
import { BrowserControllerStdioServer, handleBrowserControllerRequest } from "./browser-controller.mjs";
import { NativeAccessibility } from './native-accessibility.mjs';
import { NativeComputer } from './native-computer.mjs';
import { HostChromium } from "./kernel-browser-process.mjs";
import { BrowserRecovery } from "./kernel-browser-recovery.mjs";
import { admitComputerMutation, blockedUrl, extensionManagementUrl, EXTENSIONS_REFUSED } from "./kernel-browser-policy.mjs";
import { ChromeTabGroups, writeTabGroupsExtension } from "./kernel-browser-tab-groups.mjs";
import {nativeWindowInventory} from './kernel-browser-native-windows.mjs';
import { ChromeWindows, InputArbiter } from "./kernel-browser-chrome-window.mjs";
import { StateSignal, stateChangingEvent } from "./kernel-browser-state-signal.mjs";
import { redactObservation } from "./browser-controller-snapshot.mjs";
import {resizeBrowserPage} from './kernel-browser-resize.mjs';
import { inputHostTab } from "./kernel-browser-input.mjs";
import { BusyInputQueue, lateInputAck, markDispatched } from "./kernel-browser-input-queue.mjs";
import { BrowserPopupEvidence, popupActionId } from "./kernel-browser-popup-evidence.mjs";
import { assertNotCancelled, BrowserActionError } from "./browser-controller-actions.mjs";
import { captureProtectedDisplay, protectionDeclared, regionProtectionChanged } from "./kernel-browser-region-protection.mjs";

import { MirrorService, MirrorInputEpochRefusal } from "./kernel-browser-mirror.mjs";
import { VisitedLinks } from "./kernel-browser-visited-links.mjs";
import { WindowForeground } from './kernel-browser-foreground.mjs';
import { SampleLane } from './kernel-browser-sample-lane.mjs';
import { DisplaySessions } from './kernel-display-resumption.mjs';
import { DisplayDocuments } from './kernel-browser-display-document.mjs';

// Private display operations may overlap input; lifecycle still settles all
// owned work before closing. Public admission/Vault barriers stay in Rust.
// MP-08/MP-10: a refused native start mostly clears within a second (the
// window resize landing, a transient overlay); persistent refusals back off
// to one attempt a minute.
export const nativeRetryDelayMs = attempts => Math.min(60_000, 1_000 * 2 ** (attempts - 1));

export function scheduleHostRequest(request, browser, viewport = hostViewport(), nativeDesktop = false) {
  // MP-08/MP-10: App drains wait for enqueue, so the shared bridge lane must
  // stay independent of input and lifecycle barriers on the host as in Rooms.
  // A native desktop's default page crop never resizes Chrome; its protected
  // first sample may wait without parking input. Legacy full-page setup still
  // applies viewport geometry and must retain the barrier.
  // A state poll waits for every page's reconcile: once the viewport is applied
  // it only observes, so input does not queue behind it.
  // Ending a person's parked input never waits behind a browser operation: its authority is already gone.
  return ['browser.app.calls','browser.app.respond','host.browser.retire_input','host.browser.retire_observer'].includes(request.method) || request.method === 'host.browser' && request.params?.op === 'dialog' ||
    ['host.browser','host.computer'].includes(request.method) && (request.params?.op === 'screenshot' &&
    typeof request.params.display_subscription_id === 'string' || request.params?.op === 'mirror_next' && typeof request.params.subscription_id === 'string' ||
    (request.params?.op === 'state' || request.params?.op === 'display_subscribe' && (request.params.target || nativeDesktop && request.method === 'host.browser')) && browser?.canReconcileConcurrently?.(viewport) === true ||
    request.params?.op === 'state_signal' || request.params?.op === 'state' && browser?.dialogDefaults?.pending().length > 0 ||
    ['chrome_window_follow', 'note_selection'].includes(request.params?.op))
    ? {kind:'bridge'} : {kind:'barrier'};
}
const TAB_LIMIT = 128;
// Read-only host operations: they never dispatch input into a page.
const OBSERVATION_OPS = new Set(['chrome_window_follow', 'state', 'snapshot', 'screenshot', 'subscribe', 'poll', 'unsubscribe', 'note_selection', 'mirror_subscribe', 'mirror_next', 'mirror_close', 'display_subscribe', 'display_attach', 'display_suspend', 'display_resume', 'display_input_target']);
// MP-08/MP-10: opt-in timing names Tab lifecycle ops (fixed labels, never page values).
const SLOW_OP_MS = 1000;
const POST_INPUT_OBSERVE_MS = 500;
// A click or key arriving while the page is loading waits this long for its document to be identified, instead of being refused.
const INPUT_DOCUMENT_WAIT_MS = 1500;
// `promise`'s value, or null once `ms` have passed (the promise is left to settle on its own).
const withinMs = (promise, ms) => { let timer; return Promise.race([promise, new Promise(resolve => { timer = setTimeout(resolve, ms, null); })]).finally(() => clearTimeout(timer)); };
const TIMED_OPS = new Set(['start', 'state', 'open', 'close', 'navigate', 'mirror_subscribe', 'display_subscribe']);
const inputKinds = { click: 1, text: 2, key: 3, scroll: 4 };
function restorationUrl(url) {
  try { return navigationUrl(url); } catch { return "about:blank"; }
}

export function navigationUrl(raw) {
  if (typeof raw !== "string" || raw.length > 8192) throw new Error("invalid browser URL");
  const url = new URL(raw);
  if (raw !== "about:blank" && !["http:", "https:"].includes(url.protocol)) throw new Error("browser URL must be HTTP(S) or about:blank");
  if (url.username || url.password) throw new Error("browser URL credentials are forbidden");
  return url.href;
}

export class KernelBrowserHost {
  constructor(root, { chromium = new HostChromium(root), browserFactory = connection => new BrowserCdpClient({ connectionFactory: () => connection, focusWorldSource: mirror2ObserverExpression() }) } = {}) {
    this.root = root;
    this.timing = displayTiming(root);
    this.visitedLinks = new VisitedLinks();
    // r55: a Node event loop that stalls delays every request behind it without any step to name: mark the stall itself.
    let tick = timestamp();
    setInterval(() => { const now = timestamp(), late = now - tick - 100; tick = now; if (late > 400) this.timing('node_loop_stall', now - late); }, 100).unref();
    this.busyInputs = new BusyInputQueue(this.requestNow.bind(this), command => this.parkedState(command.observed_by ?? "adapter"), { lane: command => this.inputLane(command), sameDocument: command => command.generation === this.generation && typeof command.document_id === 'string' && this.tabs.get(command.tab_id)?.document_id === command.document_id, event: (name, cause) => this.timing(cause ? hostErrorLabel(cause).replace('host_error', `busy_${name} by`) : `busy_${name}`, timestamp()) });
    this.retiredEpochs = new Map();
    this.chromium = chromium;
    // MP-08/MP-10/MP-11 (§2b): the native Chrome window binding and its input arbiter.
    this.chromeWindows = new ChromeWindows(this,{inventory:(display,pid,isCurrent)=>nativeWindowInventory({display,pid,isCurrent})});
    this.arbiter = new InputArbiter(this, this.chromeWindows);
    this.browserFactory = browserFactory;
    this.browser = null;
    this.recovery = new BrowserRecovery(this);
    this.savedGroups = [];
    this.generation = 0;
    this.observations = 0; this.observed = 0;
    this.tabs = new Map();
    this.popupEvidence = new BrowserPopupEvidence(TAB_LIMIT);
    this.streams = new Map();
    this.displays = new Map();
    this.displaySessions = new DisplaySessions(this);
    this.navigationDisplays = new NavigationDisplays(this);
    this.scales = new Map();
    this.layouts = new Map();
    this.layoutRevisions = new Map();
    // The size the person's view gave the one Chrome window (protocol 629); null keeps the canonical page.
    this.windowLayout = null;
    // The windowLayout whose every step (all Tab layouts, stream geometry, window re-bind) completed.
    this.resizeSettled = null;
    this.inputEpochs = new Map();this.inputChangedAt=new Map();
    this.scrolling = new Map();
    this.sampleLanes = new Map();
    this.compositors = new Map();
    this.foreground = new WindowForeground();
    this.restoring = false;
    this.keepaliveTarget = null;
    this.observedDocuments = new Map();
    this.displayDocuments = new DisplayDocuments((stage, started, ended) => this.timing?.(stage, started, ended));
    this.stateSignal = new StateSignal();
    this.mirror = new MirrorService(this);
    this.protection = { values: [], targets: [], unknown: false };
    // MP-08/MP-11: desktop consumers place CDP protection regions on X11 pixels.
    if (this.chromium.desktop) this.chromium.desktop.browser = () => this.browser;
    this.desktopDisplay = new DesktopDisplay(this);
    this.onNativeInput = () => this.desktopDisplay.wake();
    this.nativeAccessibility = new NativeAccessibility({binding:()=>this.chromium.desktop?.binding()});
    this.nativeComputer = new NativeComputer({ placement: 'host',
      timing: this.timing,
      binding: () => this.chromium.desktop?.binding(),
      wakeCapture: event => this.onNativeInput?.(event),
    });
  }
  async protect(policy) {
    if (!Array.isArray(policy.values) || policy.values.length > 256 || policy.values.some(value => typeof value !== "string" || !value) || !Array.isArray(policy.targets)) throw new Error("MD-5: invalid protection policy");
    if (JSON.stringify(policy) === JSON.stringify(this.protection)) return {};
    this.protection = policy;
    this.stateSignal.notify();
    await this.desktopDisplay.retireSource();
    this.nativeAccessibility.clear();
    this.mirror.invalidate();
    await this.closeCompositors();
    for (const stream of this.displays.values()) stream.invalidate();
    if (this.browser) this.browser.protectedValues = new Set(policy.values);
    // No frame captured before insertion/retirement can be returned afterward.
    for (const stream of this.streams.values()) {
      stream.latest = null;
    }
    return {};
  }
  // MP-08/MP-10: concurrent reconciles (e.g. a native click that navigates)
  // share one writer; parallel renames of one temporary file raced (ENOENT).
  save() {
    const run = (this.saving ?? Promise.resolve()).catch(() => {}).then(() => this.write());
    this.saving = run;
    return run;
  }
  async write() {
    const name = path.join(this.root, "tabs.json");
    const active = [...this.tabs.values()].find(tab => tab.target_id === this.activeTarget);
    const data = { generation: this.generation, active_tab_id: active?.tab_id ?? null, groups: this.savedGroups,
      tabs: [...this.tabs.values()].filter(tab => !this.browser?.appTabs?.apps || ![...this.browser.appTabs.apps.values()].some(app => app.targetId === tab.target_id)).slice(0, TAB_LIMIT).map(({ tab_id, url }) => ({ tab_id, url: redactObservation(url, this.protection.values) === url ? restorationUrl(url) : "about:blank" })) };
    // App intent remains kernel-owned; only its origin selects the restored view.
    data.app_origins = [...new Set([
      ...[...(this.browser?.appTabs?.apps?.values() ?? [])].map(app => appRestoreOrigin(app.origin)).filter(Boolean),
      ...(this.pendingActive?.apps ?? []),
    ])].slice(0, TAB_LIMIT);
    data.active_tab_id = this.pendingActive?.tab_id ?? active?.tab_id ?? null;
    data.active_app_origin = this.pendingActive?.app_origin ?? appRestoreOrigin(active?.url) ?? null;
    const serialized = JSON.stringify(data);
    if (serialized === this.lastSaved) return;
    await writeFile(`${name}.new`, serialized, { mode: 0o600 });
    await rename(`${name}.new`, name);
    this.lastSaved = serialized;
  }
  isBrowserLive(browser = this.browser) {
    return !!browser && browser === this.browser
      && this.chromium?.child?.exitCode === null && this.chromium?.child?.signalCode === null
      && this.chromium?.connection?.isOpen?.() !== false && browser.connection?.isOpen?.() !== false;
  }
  async start({ signal, allowStart = true } = {}) {
    assertNotCancelled(signal);
    if (this.starting) return this.starting;
    if (this.isBrowserLive()) return;
    if (!allowStart && !this.recovery.enabled) throw new BrowserActionError("browser_unavailable", "MP-11: user browser is stopped or unavailable; explicitly start/open the browser");
    const recovering = this.recovery.enabled;
    this.recovery.enabled = true;
    this.recovery.disarm();
    // MP-08/MP-10: attribution/state reads can arrive during teardown. Keep
    // their empty live inventory from replacing the durable recovery snapshot.
    this.restoring = true;
    // Recovery belongs to the shared browser, not the viewer whose read first
    // notices the loss. Its caller still checks cancellation after this wait.
    this.starting = this.startNow({ signal: recovering ? undefined : signal });
    try { return await this.starting; }
    catch (error) { this.recovery.schedule(); throw error; }
    finally { this.starting = null; this.restoring = false; }
  }
  async startNow({ signal } = {}) {
    // MP-08/MP-10: settle the last durable tab snapshot before replacing CDP.
    await this.saving?.catch(() => {});
    await this.desktopDisplay.close();
    await this.closeCompositors();
    this.foreground.reset();
    this.sampleLanes.clear();this.inputChangedAt.clear();this.scrolling.clear();

    // MP-11: retire helpers on the old display before Chromium recovery can
    // replace it. A failed key release still requires owned desktop teardown.
    let preserveDesktop = this.chromium.desktop?.isLive?.() === true;
    try { await this.nativeComputer.close(); }
    catch { preserveDesktop = false; } // a failed physical release still resets the owned display
    finally { await this.chromium.stop(undefined, { preserveDesktop }); this.nativeAccessibility.clear(); }
    for (const stream of this.displays.values()) await stream.close();
    this.mirror.clear();
    this.displays.clear(); this.scales.clear(); this.inputEpochs.clear();
    this.tabGroups?.close?.();this.tabGroups=null;
    await this.browser?.close();
    this.browser = null;
    for (const stream of this.streams.values()) { stream.off(); clearTimeout(stream.timer); }
    this.streams.clear();
    this.tabs.clear();this.layouts.clear();this.layoutRevisions.clear();this.windowLayout=null;this.resizeSettled=null;this.navigationDisplays.clear();
    this.popupEvidence.clear();
    this.observed = ++this.observations;
    this.observedDocuments.clear();
    this.displayDocuments.clear();
    await mkdir(this.root, { recursive: true, mode: 0o700 });
    let saved = { generation: 0, tabs: [] };
    try { saved = JSON.parse(await readFile(path.join(this.root, "tabs.json"), "utf8")); }
    catch (error) { if (error.code !== "ENOENT") throw new Error("MD-2: browser tab registry is unreadable; preserve it for recovery"); }
    if (!Number.isSafeInteger(saved.generation) || saved.generation < 0 || !Array.isArray(saved.tabs)) {
      throw new Error("MD-2: invalid browser tab registry");
    }
    this.savedGroups = Array.isArray(saved.groups) ? saved.groups.slice(0, TAB_LIMIT).filter(group =>
      typeof group?.tab_id === 'string' && typeof group.label === 'string' && group.label.length <= 64) : [];
    assertNotCancelled(signal);
    // MP-08/MP-10: relaunch at the device scale of the last display viewer.
    const connection = await this.chromium.start({ preserveDesktop });
    try{await this.nativeComputer.primeKeyboard();}catch(error){await this.chromium.stop(connection,{preserveDesktop});throw error;}
    assertNotCancelled(signal);
    this.browser = this.browserFactory(connection);
    this.browser.pageTiming = this.timing;
    this.browser.visitedLinks = this.visitedLinks;
    connection.presentationFence?.bindOwner?.(this.browser, this.timing);
    this.browser.protectedValues = new Set(this.protection.values);
    this.pendingActive = { tab_id: typeof saved.active_tab_id === 'string' ? saved.active_tab_id : null,
      app_origin: appRestoreOrigin(saved.active_app_origin),
      apps: Array.isArray(saved.app_origins) ? saved.app_origins.map(appRestoreOrigin).filter(Boolean).slice(0, TAB_LIMIT) : [],
      until: Date.now() + 30_000 };
    if (!this.pendingActive.tab_id && !this.pendingActive.app_origin) this.pendingActive = null;
    this.generation = saved.generation + 1;
    this.chromeWindows.retire('browser_generation');
    // Publish the new generation before any restoration, so old references never revive.
    try {
      const connection = await this.browser.ensureConnection();
      await this.lockdown(connection);
      // MP-08/MP-11: agent Tabs are grouped under the agent's name in the native strip.
      // Attribution UI only: a browser without the component extension still works.
      this.tabGroups = new ChromeTabGroups(connection, () => this.stateSignal.notify());
      this.browser.nativeViewport = target => this.tabGroups?.viewport(target) ?? null;
      try { await this.tabGroups.load(await writeTabGroupsExtension(this.root)); } catch { this.tabGroups?.close?.();this.tabGroups = null; this.timing('tab_groups_unavailable', timestamp()); }
      const { targetInfos = [] } = await connection.send("Target.getTargets");
      // Headed Chromium exits when its last page closes. Keep one internal blank
      // target so startup and closing the last user tab never stop the browser.
      const pages = targetInfos.filter(target => target.type === "page");
      this.keepaliveTarget = await this.openKeepalive(connection, pages);
      for (const target of (await connection.send("Target.getTargets")).targetInfos.filter(target => target.type === "page")) {
        if (target.targetId !== this.keepaliveTarget) {
          await connection.send("Target.closeTarget", { targetId: target.targetId });
        }
      }
      for (const tab of saved.tabs.slice(0, TAB_LIMIT)) {
        if (typeof tab.tab_id !== "string" || !tab.tab_id.startsWith("host-tab-")) throw new Error("MD-2: invalid saved tab identity");
        assertNotCancelled(signal);
        await this.createTab(restorationUrl(tab.url), tab.tab_id, { signal, background: true });
      }
      // MP-08/MP-10: restore the registry in one inventory pass. A loading
      // background page must not delay every subsequent Tab's creation.
      // Every target now exists. Apply the pending Tab/App selection inside
      // this same inventory pass, after the restoration fence is released.
      this.restoring = false;
      await this.reconcile({ readOnly: true });
      await this.syncTabGroups(this.savedGroups,{restore:true});
      await this.save();
      await this.rebindWindows();
      this.recovery.watch(connection);
      this.stateSignal.notify();
    } catch (error) { await this.stop({ recovering: true, preserveDesktop }); throw error; }
  }
  // MP-08/MP-10/MP-11: native binding changes stay on the host mutation lane.
  async rebindWindows() {
    if (!this.chromium.display) return;
    try { await this.chromeWindows.resolve(); } catch { this.chromeWindows.retire('window_changed'); }
  }
  async syncTabGroups(groups,{restore=false}={}) {
    this.savedGroups = groups.slice(0, TAB_LIMIT).filter(group => typeof group?.tab_id === 'string' && typeof group.label === 'string' && group.label)
      .map(({ tab_id, label }) => ({ tab_id, label: label.slice(0, 64) }));
    if(this.restoring&&!restore)return;
    const labels = new Map();
    for (const { tab_id, label } of this.savedGroups) {
      const target = this.tabs.get(tab_id)?.target_id;
      if (target && typeof label === "string" && label) labels.set(label, [...(labels.get(label) ?? []), target]);
    }
    await this.tabGroups?.sync([...labels].map(([label, target_ids]) => ({ label: label.slice(0, 64), target_ids })));
    if (!this.restoring) await this.save();
  }
  // MP-11: fail-closed second layer behind the kernel's managed policies: a Tab that
  // still reaches a blocked scheme is closed unless it shows the policy's own refusal
  // (Chrome's error document). Downloads are allowed (Miguel 2026-10-15).
  async lockdown(connection) {
    if (this.chromium.policy === "unavailable" && !this.policyWarned) {
      this.policyWarned = true;
      process.stderr.write("MP-11: kernel browser managed policies unavailable (no private namespace: run the Setup system step install-root.sh, which installs the kernel browser launcher and its AppArmor profile); only the CDP lockdown applies\n");
    }
    this.lockdownOff?.();
    this.lockdownOff = connection.subscribe(message => {
      // Protocol 556: the same subscription wakes parked state reads (a Tab or document changed).
      if (stateChangingEvent(message)) this.stateSignal.notify();
      const info = message?.params?.targetInfo;
      if (!["Target.targetCreated", "Target.targetInfoChanged"].includes(message?.method) || info?.type !== "page") return;
      if (blockedUrl(info.url)) void this.closeBlocked(connection, info.targetId);
    });
  }
  async closeBlocked(connection, targetId) {
    try {
      const { sessionId } = await connection.send("Target.attachToTarget", { targetId, flatten: true });
      try { if (/^chrome-error:/.test((await connection.send("DOM.getDocument", { depth: 0 }, sessionId)).root?.documentURL ?? "")) return; }
      finally { await connection.send("Target.detachFromTarget", { sessionId }).catch(() => {}); }
    } catch {}
    await connection.send("Target.closeTarget", { targetId }).catch(() => {});
  }
  // MP-08/MP-10 (BROWSER-UX-14): the keepalive is a popup window (no tab strip; Chrome never adds Tabs
  // to it), so the native tab strip shows only the person's Tabs, in their own tabbed window.
  async openKeepalive(connection, pages) {
    const opener = pages.find(target => target.url === "about:blank")?.targetId
      ?? (await connection.send("Target.createTarget", { url: "about:blank" })).targetId;
    const known = new Set([...pages.map(target => target.targetId), opener]);
    const { sessionId } = await connection.send("Target.attachToTarget", { targetId: opener, flatten: true });
    await connection.send("Runtime.evaluate", { expression: "!!window.open('about:blank', 'chariox-keepalive', 'popup,width=64,height=64,left=0,top=0')", userGesture: true }, sessionId);
    for (let attempt = 0; attempt < 40; attempt++) {
      const popup = (await connection.send("Target.getTargets")).targetInfos.find(target => target.type === "page" && !known.has(target.targetId));
      if (popup) return popup.targetId;
      await delay(25);
    }
    throw new Error("MD-2: browser keepalive window unavailable");
  }
  // MP-08/MP-10/MP-11: native page density applies to the private display
  // only; the owned Computer desktop stays at its 1x X11 density.
  viewport() { return hostViewport(!this.chromium.desktop && (featureOn('CHARIOX_KERNEL_BROWSER_DISPLAY') || featureOn('CHARIOX_KERNEL_BROWSER_MIRROR'))); }
  // Protocol 560/168: a viewer narrower than the canonical desktop gets the Tab laid out at its own size.
  // The newest subscription owns the layout until an explicit shared-window resize takes precedence; null restores the canonical page.
  pageSize(tabId) { const l = this.layouts.get(tabId); return l ? { width: l.width, height: l.height } : { width: geometry.width, height: geometry.height }; }
  noteLayout(tabId, before) { const now = this.pageSize(tabId); if (now.width !== before.width || now.height !== before.height) this.layoutRevisions.set(tabId, this.layoutRevision(tabId) + 1); }
  layoutRevision(tabId) { return this.layoutRevisions.get(tabId) ?? 0; }
  async setLayout(tab, subscriptionId, layout) {
    if(subscriptionId!=='resize'&&this.layouts.get(tab.tab_id)?.subscriptionId==='resize')return;
    const before = this.pageSize(tab.tab_id);
    await this.browser.setLayout?.(tab.target_id, layout);
    if (layout) this.layouts.set(tab.tab_id, { ...layout, subscriptionId }); else this.layouts.delete(tab.tab_id);
    this.noteLayout(tab.tab_id, before);
  }

  releaseLayout(tabId, subscriptionId) {
    if (this.layouts.get(tabId)?.subscriptionId !== subscriptionId) return;
    const before = this.pageSize(tabId);
    this.layouts.delete(tabId);
    this.noteLayout(tabId, before);
    const tab = this.tabs.get(tabId);
    return tab ? this.browser.setLayout?.(tab.target_id, null).catch(() => {}) : undefined;
  }
  // MP-08/MP-11: on the owned Computer desktop the Browser window is the
  // visible one, and an emulated density leaves it unbindable (withheld):
  // drop a tab's viewer emulation with its last Browser viewer or mirror.
  async releaseTabScale(tabId) {
    if(!this.chromium.desktop||!this.scales.has(tabId))return;
    if([...this.displays.values()].some(s=>s.tab_id===tabId&&s.source_kind!=='desktop')||[...this.mirror.streams.values()].some(s=>s.tab_id===tabId))return;
    this.scales.delete(tabId);
    const tab=this.tabs.get(tabId);if(!tab)return;
    try{await this.browser.clearViewport(tab.target_id);}catch{}
  }
  async closeCompositors(tabId) {
    for(const [id,entry] of this.compositors) if(tabId===undefined||id===tabId){
      this.compositors.delete(id);await entry.ready.catch(()=>null);await entry.source?.close().catch(()=>{});
    }
  }
  // MP-08/MP-10: one fixed-label diagnostic per change of native refusal scope.
  nativeScope(reason){if(this.nativeScopeReason!==reason){this.nativeScopeReason=reason;if(reason)this.timing('native_unavailable_scope '+reason,timestamp());}}
  async compositorFor(tab,stream) {
    if(stream.regionSource)return stream.regionSource;
    // MP-08/MP-10/MP-11: mirror capture/input never loads the video backend.
    const {LinuxCapture,selectNativeCapture} = await import('./kernel-browser-native.mjs');
    const {CompositorSource} = await import('./kernel-browser-compositor.mjs');
    const scope=stream.codec==='png'?'png_codec':this.protection.unknown?'protection_policy':
      [...this.streams.values()].some(s=>s.tabId===tab.tab_id)?'legacy_stream':null;
    if(scope){this.nativeScope(scope);return null;}
    let entry=this.compositors.get(tab.tab_id);
    // MP-11: a refused attestation (animated page, caret) costs a lease and two
    // protected captures. Retry it per document/policy after a doubling backoff.
    const refusals=entry?.document===tab.document_id&&entry.policy===this.protection?entry.refusals??0:0;
    if(refusals&&performance.now()<entry.retryAt)return null;
    // MP-08/MP-10: a CDP fallback after a refused native start retries native
    // with backoff (nativeRetryDelayMs) for the same document.
    const retry=this.nativeRetryAt?.get(tab.tab_id),retryNative=entry&&!(entry.source instanceof LinuxCapture)&&retry?.document===tab.document_id&&performance.now()>=retry.at;
    if(entry&&(entry.document!==tab.document_id||entry.source?.closed||retryNative)){await this.closeCompositors(tab.tab_id);entry=null;}
    if(!entry){
      const {connection,sessionId}=await this.browser.resolvePageTarget(tab.target_id);
      const policy=this.protection,generation=this.generation;
      // MP-08/MP-10: a refused native start is retried on the backoff, not per credit.
      const retryAt=this.nativeRetryAt?.get(tab.tab_id);
      let source=retryAt?.document===tab.document_id&&performance.now()<retryAt.at?null:await selectNativeCapture({display:this.chromium.display,refused:reason=>this.nativeScope(reason),create:async()=>{
        // MP-08/MP-10/MP-11: one tab owns the window. A live native viewer on
        // another tab keeps it; otherwise retire other native sources and claim
        // the foreground before the source brings this tab to front.
        const holder=this.foreground.tab;
        if(holder&&holder!==tab.tab_id&&[...this.displays.values()].some(s=>s.tab_id===holder&&s.expires>Date.now())&&this.compositors.get(holder)?.source instanceof LinuxCapture){this.nativeScope('foreground_busy');throw Error('native foreground busy');}
        for(const id of [...this.compositors.keys()])if(id!==tab.tab_id&&this.compositors.get(id)?.source instanceof LinuxCapture)await this.closeCompositors(id);
        const claim=this.foreground.claim(tab.tab_id);
        this.nativeScope(null);
        const source=new LinuxCapture({size:this.pageSize(tab.tab_id),pageWindow:this.layouts.get(tab.tab_id)?.subscriptionId==='resize'?async()=>{await this.chromeWindows.resolve();return this.chromeWindows.current().tabbed}:undefined,display:this.chromium.display,pid:this.chromium.browserPid,connection,sessionId,tab,scale:stream.device_scale_factor,hostScale:this.chromium.scale??1,policy,browser:this.browser,frames:frameId=>this.browser.frameSession?.(frameId,connection),screenshot:()=>this.displayScreenshot(tab,null,false),allowed:p=>this.foreground.holds(tab.tab_id,claim)&&this.protection===p&&!p.unknown&&this.generation===generation,timing:this.timing});
        // Review #1016 P1: Tab switches outside the browser's own paths (the Computer surface's
        // Ctrl+Tab) do not reconcile. Every readback is re-admitted against the live active Tab.
        source.validateCurrent=()=>this.nativeFront(tab);
        try{return await source.start();}
        catch(error){
          const attempts=retryAt?.document===tab.document_id?retryAt.attempts+1:1;
          (this.nativeRetryAt??=new Map()).set(tab.tab_id,{document:tab.document_id,attempts,at:performance.now()+nativeRetryDelayMs(attempts)});throw error;
        }
        // The start fits Chrome's window to the page, also when it then refuses. The band's own capture
        // retires on that resize, so no live band may be left to notice it: rebind from the owner's inventory.
        finally{this.chromeWindows.resolve().catch(()=>{});}
      }});
      source??=new CompositorSource({connection,sessionId,tab,scale:stream.device_scale_factor,policy,timing:this.timing,width:this.pageSize(tab.tab_id).width*stream.device_scale_factor,height:this.pageSize(tab.tab_id).height*stream.device_scale_factor,format:'jpeg',acquire:()=>this.sampleLane(tab).run('input',()=>this.browser.inputCapture.hold(connection,sessionId)),
        screenshot:clip=>this.displayScreenshot(tab,clip),protect:()=>this.displayScreenshot(tab),allowed:p=>this.protection===p&&!p.unknown&&!p.values.length&&!p.targets.length&&this.generation===generation});
      const created={source,document:tab.document_id,policy};
      created.ready=source instanceof LinuxCapture?Promise.resolve(source):source.start().catch(()=>{created.refusals=refusals+1;created.retryAt=performance.now()+Math.min(30_000,1000*2**refusals);return null;});
      entry=created;this.compositors.set(tab.tab_id,entry);
    }
    return await entry.ready;
  }
  async stop({ recovering = false, preserveDesktop = false } = {}) {
    if (!recovering) {
      this.recovery?.stop();
      await this.starting?.catch(() => {});
      this.recovery?.stop();
    }
    this.busyInputs.retireAll(); // a stopped browser is not relaunched by an input that waited for its renderer
    await this.desktopDisplay.close();
    this.displayDocuments.clear();
    this.lockdownOff?.(); this.lockdownOff = null;
    this.stateSignal.notify();
    await this.closeCompositors();
    this.sampleLanes.clear();

    this.nativeAccessibility.clear();
    this.popupEvidence.clear();
    // A dead X server can make release fail; retirement still destroys the
    // owned desktop before acknowledging stop.
    await this.nativeComputer.close().catch(() => { preserveDesktop = false; });
    await this.chromium.stop(this.browser?.connection, { preserveDesktop });
    for (const stream of this.displays.values()) await stream.close();
    this.mirror.clear();
    this.displays.clear(); this.scales.clear();
    this.tabGroups?.close?.();this.tabGroups=null;
    await this.browser?.close();
    this.browser = null;
    for (const stream of this.streams.values()) { stream.off(); clearTimeout(stream.timer); }
    this.streams.clear();
    this.timing.flush?.();
    return { state: "stopped", generation: this.generation, tabs: [] };
  }
  // MP-08/MP-10/MP-11: reconciles can overlap (a state poll beside input). They
  // apply in start order: an older observation reports the newer current tabs.
  tabState() {
    // MP-08: private creation evidence for the kernel, stripped before client projection.
    const _tab_creation_actions = this.popupEvidence.inventory([...this.tabs.values()]);
    const dialogs = this.pendingDialogs();
    return redactObservation({ state: "ready", generation: this.generation, _observation: this.observed, _tab_creation_actions,
      tabs: [...this.tabs.values()].map(({ target_id, opener_target_id, ...tab }) => tab), dialogs, viewport:this.viewport(), chrome_window: this.chromeWindow(),
      // MP-11: whether Chromium's managed policies apply, or only the kernel's CDP lockdown layer.
      browser_lockdown: this.chromium?.policy ?? null }, this.protection.values);
  }
  pendingDialogs() {
    const byTarget = new Map([...this.tabs.values()].map(tab => [tab.target_id, tab]));
    return (this.browser?.dialogDefaults?.pending() ?? []).flatMap(({ target_id, ...dialog }) => {
      const tab = byTarget.get(target_id);
      if (!tab || typeof dialog.document_id !== "string" || !dialog.document_id) return [];
      // A navigation can open a dialog before its first renderer observation.
      // The controller bound this opening to CDP's committed document.
      tab.document_id = dialog.document_id;
      return [{ ...dialog, tab_id: tab.tab_id, generation: this.generation }];
    });
  }
  async readState({ signal } = {}) {
    if (this.pendingDialogs().length) return this.tabState();
    const waiting = new AbortController();
    const cancel = () => waiting.abort();
    signal?.addEventListener('abort', cancel, { once: true });
    const dialog = async () => {
      let after = this.stateSignal.count;
      while (!waiting.signal.aborted) {
        if (this.pendingDialogs().length) return this.tabState();
        after = await this.stateSignal.wait(after, 2000, waiting.signal);
      }
      // The observation won, or the request was cancelled.
      return new Promise(() => {});
    };
    const observation = async () => {
      if (this.chromium?.display && this.chromium.policy === 'managed') await this.chromeWindows?.heal?.();
      await this.chromeWindows?.refreshWarm?.();
      await this.chromeWindows?.refreshStale?.();
      return this.reconcile({ readOnly: true });
    };
    try { return await Promise.race([dialog(), observation()]); }
    finally { waiting.abort(); signal?.removeEventListener('abort', cancel); }
  }
  /** PERF_AUDIT 4.2.3 (protocol 540): a viewer that holds frames pushed before its registration
   * response asks for its first frame at registration; the kernel's relay pump starts there
   * instead of waiting for the viewer's first acknowledgement (one round trip per stream open). */
  initialPush(command, result) {
    const stream = command.initial_push === true && this.displays.get(result?.subscription_id);
    if (stream) stream.initial_push = true;
    return result;
  }
  // MP-08/MP-10/MP-11 (BROWSER-UX-14): the band's window epoch (for a 519 BrowserChrome
  // subscription) and the Tab Chrome's own strip shows, as last observed by the kernel.
  chromeWindow() {
    const tabbed = this.chromeWindows?.bound?.tabbed;
    // MP-11: only the managed lockdown refuses fullscreen; without it no band is offered.
    if (!featureOn("CHARIOX_KERNEL_BROWSER_DISPLAY") || this.chromium?.policy !== "managed" || !(tabbed?.isBandCurrent ?? tabbed?.isCurrent)?.()) return null;
    const active = [...this.tabs.values()].find(tab => tab.target_id === this.activeTarget);
    // Protocol 530: the window's own popups (menus, omnibox dropdown, extension popups, bubbles).
    // MP-08/MP-10: restoring Apps temporarily focus their own windows. The
    // existing nullable selection stays unknown until restore settles; physical
    // input still checks the actual native target through activeTarget.
    return { window_generation: tabbed.window_generation, active_tab_id: this.pendingActive ? null : active?.tab_id ?? null, auxiliary: this.chromeWindows.auxiliaryState() };
  }
  async reconcile({readOnly=false}={}) {
    const browser = this.browser, generation = this.generation;
    const observation = ++this.observations;
    const stateOptions = await nativeStateObservationOptions(this, readOnly);
    let state;
    // MP-08/MP-10/MP-11 (BROWSER-UX-14): the kernel's Tabs keep their window's native tab
    // strip and omnibox (the browser_chrome band); the Room bar's fullscreen policy is not applied.
    // Restored pages can finish navigation between CDP document reads. Retry
    // only this observation, never the mutation that led to reconciliation.
    for (let attempt = 0; attempt < 3; attempt++) {
      try { state = await browser.reconcile(this.viewport(), stateOptions); break; }
      catch (error) {
        if (error.code !== "stale_document_reference" || attempt === 2) throw error;
        await delay(25);
      }
    }
    if (browser !== this.browser || generation !== this.generation) throw new BrowserActionError('browser_unavailable', 'MP-08: browser restarted; interrupted action was not replayed');
    if (observation < this.observed) return this.tabState();
    this.observed = observation;
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
    // Preserve already-adopted identities before accepting native/popup tabs, in their existing order:
    // Chrome lists targets most recently active first, and a Tab strip must not reorder (MP-08/MP-10).
    const known = new Map([...byTarget.keys()].map((target, index) => [target, index]));
    discovered.sort((a, b) => (known.get(a.target_id) ?? known.size) - (known.get(b.target_id) ?? known.size));
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
    // A surviving Tab keeps its committed page size; only retired Tabs drop theirs.
    for (const map of [this.layouts, this.layoutRevisions]) for (const id of [...map.keys()]) if (!this.tabs.has(id)) map.delete(id);
    // A Tab that opened in the resized window is laid out at the window's size, not the canonical one.
    // A Tab left at an older size by a resize that failed part-way is laid out again too.
    if (this.windowLayout) for (const tab of [...this.tabs.values()]) { const l = this.layouts.get(tab.tab_id); if (l?.subscriptionId !== 'resize' || l.width !== this.windowLayout.width || l.height !== this.windowLayout.height) await this.setLayout(tab, 'resize', this.windowLayout); }
    const previous = this.activeTarget;
    const intent = this.pendingActive;
    const boundOrigins = new Set([...(this.browser.appTabs?.apps?.values() ?? [])].filter(app => app.ready === true).map(app => app.origin));
    const restoreReady = intent && (intent.apps.every(origin => boundOrigins.has(origin)) || Date.now() >= intent.until);
    const restored = !this.restoring && restoreReady && [...this.tabs.values()].find(tab => intent.app_origin
      ? [...(this.browser.appTabs?.apps?.values() ?? [])].some(app => app.ready === true && app.targetId === tab.target_id && app.origin === intent.app_origin)
      : tab.tab_id === intent.tab_id);
    const nativeActive = await this.tabGroups?.active?.().catch(() => null) ?? null;
    if (restored) {
      if (this.pendingActive !== intent) return this.reconcile();
      if (nativeActive !== restored.target_id) {
        await this.retireNativeForeground(restored.target_id);
        const connection = await this.browser.ensureConnection();
        if (this.pendingActive !== intent) return this.reconcile();
        await connection.send('Target.activateTarget', {targetId:restored.target_id});
      }
      if (this.pendingActive !== intent) return this.reconcile();
      this.pendingActive = null;
    } else if (intent && this.pendingActive === intent && !this.restoring && Date.now() >= intent.until) {
      // The saved selection cannot return (App uninstalled, Tab closed): the live native selection wins.
      this.pendingActive = null;
    }
    this.activeTarget = restored?.target_id ?? nativeActive;
    if (this.activeTarget !== previous) await this.retireNativeForeground(this.activeTarget);
    const result = this.tabState();
    for (const [id, stream] of this.streams) if (!this.tabs.has(stream.tabId)) await this.removeStream(id);
    if (!this.restoring) await this.save();
    return result;
  }
  /** Review #1016 P1: whether `tab` is Chrome's active Tab right now (live, not the last observation). */
  // `cached`: a key or wheel notch may use the owner snapshot until an activation/resize/detach event retires it (a miss reads live).
  async nativeFront(tab, { cached = false } = {}) {
    const active = (cached ? this.tabGroups?.cachedActive?.() : null) ?? await this.tabGroups?.active?.().catch(() => null);
    return !!active && active === tab.target_id;
  }
  // MP-08/MP-10/MP-11 (review #1016 P1): Chrome's own strip, wheel or shortcuts, or another actor,
  // can show another Tab in the one window. Whole-page native pixels and input of every other Tab
  // stop first (the claim is retired synchronously); `keep` is a Tab target known to be in front.
  retireNativeForeground(keep = null) {
    const kept = tabId => keep !== null && this.tabs.get(tabId)?.target_id === keep;
    if (this.foreground.tab && !kept(this.foreground.tab)) this.foreground.reset();
    return import('./kernel-browser-native.mjs').then(async ({ LinuxCapture }) => {
      for (const [id, entry] of [...this.compositors]) if (entry.source instanceof LinuxCapture && !kept(id)) await this.closeCompositors(id);
    });
  }
  async open(url, tabId = `host-tab-${randomUUID()}`, { signal } = {}) {
    await this.closeCompositors();this.foreground.reset();
    await this.createTab(url, tabId, { signal });
    const state = await this.reconcile();
    return { ...state, tab_id: tabId };
  }
  async createTab(url, tabId, { signal, background = false } = {}) {
    if (this.tabs.size >= TAB_LIMIT) throw new Error("MD-2: host tab limit reached");
    const connection = await this.browser.ensureConnection();
    const browser = this.browser;
    assertNotCancelled(signal);
    const { targetId } = await connection.send("Target.createTarget", { url: navigationUrl(url), ...(background ? { background: true } : {}) });
    if (browser !== this.browser) throw new BrowserActionError('browser_unavailable', 'MP-08: browser restarted; interrupted action was not replayed');
    this.tabs.set(tabId, { tab_id: tabId, target_id: targetId, url, title: "", document_id: "" });
    this.observed = ++this.observations;
  }
  async target(command) {
    const started = timestamp();
    if (command.generation !== this.generation) throw new UserDomainRefusal("stale_epoch");
    await this.reconcile();
    this.timing('target_reconcile', started);
    const tab = this.tabs.get(command.tab_id);
    if (!tab) throw new UserDomainRefusal("stale_reference", "display_retired");
    return tab;
  }
  async mirrorTarget(command) {
    if (command.generation !== this.generation) throw new UserDomainRefusal("stale_epoch");
    const stored = this.tabs.get(command.tab_id);
    if (!stored) throw new UserDomainRefusal("stale_reference", "display_retired");
    const current = this.browser.currentPageFrame?.(stored.target_id);
    return current ? { ...stored, document_id: current.frame.loaderId } : this.displayTarget(command);
  }
  displayTarget(command, native = false, wait = 0) { return resolveDisplayTarget(this, command, native, wait); }

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
  screenshot(tab, clip = null, protectedCapture = false, format = "png", optimizeForSpeed = true) {
    return capturePageScreenshot(this, tab, clip, protectedCapture, format, optimizeForSpeed);
  }
  async subscribe(tab, boundFrames = false, owner = null) {
    await this.closeCompositors(tab.tab_id);
    if (this.streams.size >= 16) throw new Error("MD-2: frame subscription limit reached");
    const { connection, sessionId } = await this.browser.resolvePageTarget(tab.target_id);
    const id = `host-stream-${randomUUID()}`;
    const stream = { sessionId, tabId: tab.tab_id, boundFrames, owner, latest: null, sequence: 0, expires: Date.now() + 60_000 };
    const captureProtected = () => {
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
        if(protectionDeclared(message)){stream.regionEpoch=(stream.regionEpoch??0)+1;stream.latest=null;}
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
      void this.removeDisplay(stream).catch(()=>{});
    }, Math.max(1, stream.expires - Date.now()));
    stream.timer.unref?.();
  }
  async removeDisplay(reference) {
    const stream=typeof reference==='string'?this.displays.get(reference):reference;
    if(!stream||this.displays.get(stream.subscription_id)!==stream)return;
    clearTimeout(stream.timer);
    if(stream.source_kind==='desktop')return this.desktopDisplay.remove(stream);
    this.displays.delete(stream.subscription_id);await stream.close();
    if(![...this.displays.values()].some(s=>s.tab_id===stream.tab_id)){
      await this.closeCompositors(stream.tab_id);await this.releaseTabScale(stream.tab_id);
    }
  }
  async displayResumeBinding(stream,signal) {
    assertNotCancelled(signal);
    if(stream.source_kind==='region'){
      const source=stream.regionSource;
      if(!source||typeof stream.regionCurrent!=='function'||!await stream.regionCurrent()||!stream.regionLease?.isAlive()||!stream.regionWindow?.isCurrent()||!source.valid()||source.closed||source.allowed&&!source.allowed(this.protection))throw new UserDomainRefusal('stale_reference','display_retired');
      assertNotCancelled(signal);
      return source;
    }
    if(stream.source_kind==='desktop'){
      const binding=this.chromium.desktop?.binding();
      if(!binding||binding.surface_id!==stream.tab_id||binding.generation!==stream.desktop_generation)throw new UserDomainRefusal('stale_reference','display_retired');
      return binding;
    }
    const tab=await this.displayTarget({tab_id:stream.tab_id,generation:this.generation});
    assertNotCancelled(signal);
    if(tab.document_id!==stream.document_id)throw new UserDomainRefusal('stale_reference','display_retired');
    const viewport=this.viewport();
    const source=this.compositors.get(tab.tab_id)?.source;
    if(source?.closed||source?.allowed&&!source.allowed(this.protection))throw new UserDomainRefusal('stale_reference','display_retired');
    const key=JSON.stringify([tab.target_id,tab.document_id,viewport.width,viewport.height,viewport.device_scale_factor,source?.regionRevision]);
    if(stream.resumeSourceIdentity?.key!==key||stream.resumeSourceIdentity?.source!==source)stream.resumeSourceIdentity={key,source};
    return stream.resumeSourceIdentity;
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
  // Protocol 552: input this observer has parked behind a busy renderer; the viewer holds its preview handover until it is admitted.
  parkedState(scope) {
    const state = this.observe(this.tabState(), null, scope), parked_input = this.busyInputs.parked(scope);
    return parked_input ? { ...state, parked_input } : state;
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
  // The surface a person's input acts on: Chrome's own band (no page renderer behind its fences) or the page.
  inputLane(command) {
    const stream = typeof command.binding_id === 'string' ? [...this.displays.values()].find(s => s.binding_id === command.binding_id) : null;
    if (['browser_chrome', 'browser_auxiliary'].includes(stream?.regionTarget?.kind)) return 'band';
    // A wheel parked behind a busy renderer is not what a key waits for: it scrolls, the key types where focus already is.
    return command.input?.kind === 'scroll' ? 'wheel' : 'page';
  }
  // A person's input queues in order while a busy renderer cannot answer the fences; agents keep awaiting their ack.
  request(command, options) {
    return command.op === "input" && !command._agent_input && !command.focused_agent
      ? this.busyInputs.submit(command.observed_by ?? "adapter", command, options) : this.requestNow(command, options);
  }
  async requestNow(command, { signal, inputGuard, nativeDisplay } = {}) {
    const scope = command.observed_by ?? (command.focused_agent ? "focused-agent" : "adapter");
    assertNotCancelled(signal);
    if (command.op.startsWith("mirror_") && !featureOn("CHARIOX_KERNEL_BROWSER_MIRROR")) throw new Error("MP-08: DOM mirror disabled");
    if (command.op.startsWith("display_") && !featureOn("CHARIOX_KERNEL_BROWSER_DISPLAY")) throw new Error("MD-DISPLAY: display disabled");
    if (command.op === "stop") return this.stop();
    // MP-11: observation never launches/relaunches the browser.
    await this.start({ signal, allowStart: !["dialog", "state", "snapshot", "screenshot", "subscribe", "poll", "unsubscribe", "display_subscribe", "display_attach", "display_suspend", "display_resume", "display_input_target", "resize", "note_selection", "mirror_subscribe", "mirror_next", "mirror_close", "chrome_window_follow"].includes(command.op) });
    assertNotCancelled(signal);
    const browserAtStart = this.browser;
    if (this.protection.unknown) throw new Error("MD-5: observation registry unavailable");
    for (const [id, stream] of this.streams) if (Date.now() > stream.expires) await this.removeStream(id);
    // A modal blocks renderer evaluation. Read and answer from the controller's
    // CDP event state, never from a reconciliation that waits on that renderer.
    if (command.op === "dialog") {
      const tab = this.tabs.get(command.tab_id);
      if (command.generation !== this.generation || !tab || tab.document_id !== command.document_id ||
          typeof command.dialog_id !== "string" || !this.browser.dialogDefaults?.current(tab.target_id, command.dialog_id, command.document_id)) {
        throw new BrowserActionError("browser_dialog_invalid", "browser dialog is no longer pending; read the current browser state");
      }
      this.appAgent(tab, command, true);
      await this.browser.handleDialog({ ...command, target_id: tab.target_id }, { signal });
      this.stateSignal.notify();
      return this.observe(this.tabState(), null, scope);
    }
    // MP-08/MP-10: the kernel's navigate/subscribe preflight reads only its own Tab
    // live; an all-tab reconcile waits for every busy (loading) page.
    if (command.op === "state" && typeof command.tab_id === "string") {
      const tab = await this.displayTarget(command);
      this.observed = ++this.observations; // newest observation, as at a navigation commit
      return this.observe(this.tabState(), tab, scope);
    }
    // The reconcile below would wait on the busy renderer that parked this observer's input.
    if (command.op === "state" && this.busyInputs.parked(scope)) return this.parkedState(scope);
    // PERF_AUDIT 4.2.2/4.2.6: the window a person's Chrome UI input left, read outside the input lane.
    if (command.op === "chrome_window_follow") return this.arbiter?.follow ? this.arbiter.follow(command) : {};
    // Protocol 556: the signal count is taken before the read, so a change during it wakes the next wait.
    const stateSignal = command.state_wait === true ? this.stateSignal.count : null;
    // Protocol 540: a viewer's state read right after an address-bar key sees the list Chrome opened.
    if (["start", "state"].includes(command.op)) {
      const state = this.observe(await (command.op === "state" ? this.readState({ signal }) : this.reconcile()), null, scope);
      return command.state_wait === true ? { ...state, _state_signal: stateSignal } : state;
    }
    const agentRequest = command.focused_agent || command._agent_input;
    if (agentRequest && ["open", "navigate"].includes(command.op) && extensionManagementUrl(command.url)) throw new Error(EXTENSIONS_REFUSED);
    if (command.op === "open") { const opened = await this.open(command.url, undefined, { signal }); await this.rebindWindows(); return this.observe(opened, null, scope); }
    for (const stream of this.displays.values()) if (Date.now() > stream.expires) await this.removeDisplay(stream);
    if(['display_suspend','display_resume'].includes(command.op)){
      if(command.focused_agent||command._agent_input)throw new UserDomainRefusal('not_granted');
      const binding=stream=>this.displayResumeBinding(stream,signal);
      return command.op==='display_resume'
        ?this.displaySessions.resume(command.subscription_id,scope,command.generation,binding)
        :this.displaySessions.suspend(command.subscription_id,scope,command.generation,binding);
    }
    // MP-08/MP-10/MP-11: allocated536 default fallback never resizes the owned desktop.
    if(command.op==='display_subscribe'&&!command.target&&this.chromium.desktop?.binding?.()){
      // A navigation the observer's mirror just reported already started this display (see NavigationDisplays).
      this.navigationDisplays.remember(command,scope);
      try{return this.initialPush(command,await this.navigationDisplays.adopt(command,scope)??await subscribeVisiblePage(this,command,scope,signal));}
      catch(error){const png=pngFallbackCommand(error,command);if(!png)throw error;this.timing('native_visible_page_png_fallback',timestamp());command=png;}
    }
    if(command.op==='display_subscribe'&&command.target) {
      const {subscribeRegion}=await import('./kernel-browser-display-regions.mjs');
      return this.initialPush(command,await subscribeRegion(this,command,scope,signal));
    }
    if(command.op==='display_input_target'||command.op==='input'&&command.binding_id) {
      const {regionInput,regionInputTarget}=await import('./kernel-browser-display-regions.mjs');
      return command.op==='display_input_target'?regionInputTarget(this,command,scope,signal):regionInput(this,command,scope,signal);
    }
    if(command.op==='mirror_subscribe') return this.mirror.subscribe(command,scope);
    if(command.op==='mirror_next') return this.mirror.next(command,scope,{signal});
    if(command.op==='mirror_close') {this.mirror.require(command.subscription_id,scope,command.generation);this.mirror.drop(command.subscription_id);return {closed:true};}
    const encodedCapture = command.op === "screenshot" && typeof command.display_subscription_id === "string";
    if (encodedCapture || command.op === "display_attach" || (command.op === "unsubscribe" && this.displays.has(command.subscription_id))) {
      const stream = this.displays.get(command.display_subscription_id ?? command.subscription_id);
      if (stream && stream.observed_by !== scope) throw new UserDomainRefusal("not_granted");
      if (!stream || command.generation !== this.generation) throw new UserDomainRefusal('stale_reference', 'display_retired');
      if(stream.source_kind==='desktop')return this.desktopDisplay.request(command,scope,{signal});
      if (command.op === "unsubscribe") { await this.removeDisplay(stream); return { generation: this.generation, unsubscribed: true }; }
      if(stream.suspended)throw new UserDomainRefusal('stale_reference','display_suspended');
      stream.lastCreditAt=Date.now();
      stream.expires = Date.now() + 60_000;
      this.armDisplayExpiry(stream);
      if (command.op === "display_attach") return { attached: true, generation: this.generation, ...(stream.initial_push ? { initial_push: true } : {}) };
      const {displayCredit, displayPushCredit} = await import('./kernel-browser-display-credit.mjs');
      return command.push ? displayPushCredit(this, stream, command, signal) : displayCredit(this, stream, command, signal);
    }
    if (["poll", "unsubscribe"].includes(command.op)) {
      if (command.generation !== this.generation || !this.streams.has(command.subscription_id)) throw new UserDomainRefusal("stale_reference", "display_retired");
      const stream = this.streams.get(command.subscription_id);
      stream.expires = Date.now() + 60_000;
      this.armExpiry(command.subscription_id, stream);
      if (command.op === "unsubscribe") { await this.removeStream(command.subscription_id); return { generation: this.generation, unsubscribed: true }; }
      return { generation: this.generation, frame: stream.latest };
    }
    // Explicit physical input already names an observed tab/document. Read
    // that live target directly; whole-tab discovery still runs afterward.
    // Per-dispatch document/cancellation and secret-focus checks remain below.
    // MP-08/MP-10: a navigation or a selection note reads only its own Tab's live document: a full
    // all-tab reconcile waits for busy (loading) pages and held the mutation lane.
    let tab = command.op === 'input' && typeof command.document_id === 'string' || ['navigate', 'resize', 'note_selection'].includes(command.op)
      ? await slowMark(this.timing, 'input_wait display_target', this.displayTarget(command,nativeInputDocumentTarget(this,command,scope,nativeDisplay),INPUT_DOCUMENT_WAIT_MS)) : await this.target(command);
    assertNotCancelled(signal);
    if (agentRequest && ["input", "navigate"].includes(command.op) && extensionManagementUrl(tab.url)) throw new Error(EXTENSIONS_REFUSED);
    const { app, agent } = this.appAgent(tab, command, ["input", "navigate", "close", "resize"].includes(command.op));
    // The page stays on its origin.
    if (app && command.op === "navigate" && new URL(navigationUrl(command.url)).origin !== app.origin) {
      throw new UserDomainRefusal("not_granted");
    }
    if(command.op==='resize')return resizeBrowserPage(this,tab,command,signal);
    const binding = { target_id: tab.target_id, document_id: tab.document_id };
    if (command.op === "display_subscribe") return this.initialPush(command, await reserveDisplay(this,async()=>{
      if (!featureOn("CHARIOX_KERNEL_BROWSER_DISPLAY")) throw new Error("MD-DISPLAY: display disabled");
      if (!Array.isArray(command.codecs) || !command.codecs.includes("png") || command.codecs.length > 8 ||
        !Number.isInteger(command.bitrate) || command.bitrate < 500_000 || command.bitrate > 64_000_000 ||
        ![1, 2].includes(command.device_scale_factor) || this.displays.size >= 8) throw new Error("MD-DISPLAY: invalid display negotiation");
      // MP-08/MP-10/MP-11: observation never changes the native page density.
      const scale = this.viewport().device_scale_factor;
      this.scales.set(tab.tab_id, scale);
      const id = `host-display-${randomUUID()}`;
      const codec=process.env.CHARIOX_BROWSER_DISPLAY_NATIVE_WORKER&&command.codecs.includes('avc1.420033')?'avc1.420033':command.codecs.find(c=>['vp8','vp09.00.50.08','vp09.00.40.08','vp09.00.10.08','avc1.420033'].includes(c))??'png';
      const {DisplayStream} = await import('./kernel-browser-display.mjs');
      const {BrowserEncoder} = await import('./kernel-browser-webcodecs.mjs');
      const stream = new DisplayStream({ relay_binary:command.codecs.includes('chariox-relay-binary-v96'), subscription_id: id, tab_id: tab.tab_id, observed_by: scope, bitrate: command.bitrate, device_scale_factor: scale, codec, css_width:this.pageSize(tab.tab_id).width, css_height:this.pageSize(tab.tab_id).height, dependencies:command.codecs.includes('chariox-video-dependencies-v1'),stripes:command.codecs.includes('chariox-stripes-v1')&&['avc1.420033','vp8'].includes(codec) }, { timing:this.timing,encoder:new BrowserEncoder(this.browser,tab.target_id) });
      this.displays.set(id, stream);
      this.armDisplayExpiry(stream);
      return { generation: this.generation, subscription_id: id, codec, bitrate: command.bitrate, device_scale_factor: scale };
    }));
    if (command.op === "close") {
      // The closed Tab can no longer be restored as the selection (MP-08/MP-10).
      const intent = this.pendingActive, origin = appRestoreOrigin(tab.url);
      if (intent && (intent.tab_id === tab.tab_id || (origin && intent.app_origin === origin))) this.pendingActive = null;
      this.mirror.removeTab(tab.tab_id);this.layouts.delete(tab.tab_id);this.layoutRevisions.delete(tab.tab_id);
      for (const [id, stream] of this.displays) if (stream.tab_id === tab.tab_id && !['browser_chrome', 'browser_auxiliary', 'browser_window'].includes(stream.regionTarget?.kind)) { await stream.close(); this.displays.delete(id); }
      await this.closeCompositors(tab.tab_id);if(this.foreground.tab===tab.tab_id)this.foreground.reset();
      this.sampleLanes.delete(tab.tab_id);
      this.scales.delete(tab.tab_id);
      this.inputEpochs.delete(tab.tab_id);
      this.scrolling.delete(tab.tab_id);
      for (const [id, stream] of this.streams) {
        if (stream.tabId === tab.tab_id) await this.removeStream(id);
      }
      await this.browser.manageTab({ ...binding, action: "close" }, { signal });
      for (const observed of this.observedDocuments.values()) observed.delete(tab.tab_id);
      const state = await this.reconcile();
      await this.rebindWindows();
      return state;
    }
    if (command.op === "navigate") {
      // MP-08/MP-11: an agent's navigation is its input to the App Tab too.
      if (app && agent) this.browser.appTabs.noteAgent(tab.target_id, agent);
      // MP-08/MP-10/MP-11: a human navigation selects this owned desktop tab.
      // Retire native claims before physical focus can show a different page.
      if(this.chromium.desktop&&!command._agent_input&&!command.focused_agent){
        this.foreground.reset();await this.closeCompositors();
        await this.browser.manageTab({...binding,action:'activate'},{signal});
      }
      const navigated = await this.browser.navigate({ ...binding, url: navigationUrl(command.url) }, { signal });
      // MP-08/MP-10: answer at commit, like a browser's address bar; the page loads
      // into attached views and later state reads reconcile its title.
      const stored = this.tabs.get(tab.tab_id);
      if (!stored || typeof navigated?.document_id !== 'string' || !navigated.document_id) return this.observe(await this.reconcile(), null, scope);
      // A redirect commits another address: read the committed target, fenced to this document.
      const committed = await this.displayTarget({ generation: this.generation, tab_id: tab.tab_id });
      if (committed.document_id !== navigated.document_id) return this.observe(await this.reconcile(), null, scope);
      this.observed = ++this.observations; // an older in-flight reconcile cannot restore the previous document
      return this.observe(this.tabState(), null, scope);
    }
    if (command.op === "note_selection") return this.observe({ generation:this.generation, observation:await this.browser.observeNote({ ...binding, ...(command.quote ? {quote:command.quote} : {}) }) },tab,scope);
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
      const rememberSelection = () => {
        // MP-08/MP-10/MP-11: CDP can type into a background window. Its
        // admitted human selection supersedes older native restore intent.
        const intent = this.pendingActive, origin = appRestoreOrigin(tab.url);
        // The cached target can still match while a late native window fold
        // changed Chrome's selection. Reconcile checks the live native target
        // before activating, so ordinary keys do not refocus an active tab.
        if (!agent && !command._agent_input && !command.focused_agent) {
          this.pendingActive = {tab_id:tab.tab_id,app_origin:origin,apps:intent?.apps ?? [],until:intent?.until ?? Date.now()};
        }
      };
      const onDispatch = () => {
        if (dispatched) return;
        dispatched = true;
        rememberSelection();
        this.inputChangedAt.set(tab.tab_id, performance.now());
        this.timing.event?.('input_dispatch',{at,input:this.inputCount=(this.inputCount??0)+1,kind:inputKinds[command.input?.kind]??0});
        this.inputEpochs.set(tab.tab_id, (this.inputEpochs.get(tab.tab_id) ?? 0) + 1);
        (nativeDisplay?.regionSource??this.compositors.get(tab.tab_id)?.source)?.wake?.();
      };
      // MP-08/MP-11: before the page can see it, so every call it causes runs as the agent.
      if (app && agent) this.browser.appTabs.noteAgent(tab.target_id, agent);
      try {
        // MP-08/MP-10/MP-11: agent and ordinary input must await CDP's ack,
        // including the lifetime of hidden-target focus emulation. Only an
        // admitted human viewer with a live display lease owns the fast path.
        const viewerActive=()=>viewerPageInputActive(this,command,scope);
        // MP-08/MP-10/MP-11: the input grant and document fence admit drags too; a DOM mirror needs no video lease.
        // Review #1016 P1: native whole-page input only while this Tab is Chrome's live active Tab.
        // MP-08/MP-10/MP-11: a preview handover flush must follow actual CDP input ACKs, not an asynchronous XTest receipt.
        const handoverOrdered=[...this.mirror.streams.values()].some(s=>s.tab_id===tab.tab_id&&s.scope===scope&&s.navigationPreview);
        const candidate=handoverOrdered?null:(nativeDisplay?visiblePageInputSource(this,nativeDisplay,tab,scope):this.compositors.get(tab.tab_id)?.source);
        const owned=candidate?.attested&&viewerActive()&&await slowMark(this.timing,'input_wait native_front',this.nativeFront(tab,{cached:['key','scroll'].includes(command.input?.kind)}))?candidate:null;
        const nativeWheel=viewerActive()&&owned?.attested&&typeof owned.wheel==='function'?(x,y,dx,dy)=>viewerActive()&&owned.wheel(x,y,dx,dy):null;
        const nativeClick=viewerActive()&&owned?.attested&&typeof owned.click==='function'?(x,y)=>{if(!viewerActive())return false;this.arbiter.pageFocused();return owned.click(x,y);}:null;
        const nativeKey=viewerActive()&&owned?.attested&&typeof owned.key==='function'?(keysym,shift)=>viewerActive()&&owned.key(keysym,shift):null;
        const documentCurrent=(nativeKey&&command.input?.kind==='key'||nativeWheel&&command.input?.kind==='scroll')?await slowMark(this.timing,'input_wait document_guard',this.displayDocuments.guard(this.browser,tab)):undefined;
        const laneAt=timestamp();
        const deferred=await this.sampleLane(tab).run("input", async () => {
          if(timestamp()-laneAt>250)this.timing('input_wait sample_lane',laneAt);
          const result=await this.popupEvidence.capture(this.browser, tab, popupActionId(command), beginDispatch => inputHostTab(this.browser, tab, command.input, { signal, inputGuard, documentCurrent, chromeFocused: () => Boolean(this.arbiter?.chromeFocus), slow: (name, operation) => slowMark(this.timing, `input_wait ${name}`, operation), onDispatch: () => { onDispatch(); return beginDispatch(); }, viewport:{css_width:this.pageSize(tab.tab_id).width,css_height:this.pageSize(tab.tab_id).height}, asyncScroll: ()=>viewerActive()&&(handoverOrdered||owned?.attested&&typeof owned.valid==='function'&&owned.valid()), nativeWheel, nativeClick, nativeKey, resolveMirror: input => this.mirror.resolveInput(tab,input,scope,signal) }));
          rememberSelection();
          return result;
        });
        // MP-08/MP-10: wheel input is asynchronous, as in a native browser. The
        // fenced, ledgered dispatch is ordered by CDP; the renderer's
        // frame-aligned ack would otherwise serialize kernel input admission.
        deferred?.ack?.catch(()=>{});
        if(dispatched)(nativeDisplay?.regionSource??this.compositors.get(tab.tab_id)?.source)?.wake?.();
        this.timing('cdp_input', at);
      }
      catch (error) {
        if (dispatched) markDispatched(error);
        // A busy renderer that is only late to acknowledge a CDP press/release (both already queued in order) leaves nothing held: it must not take the browser down.
        if ((dispatched && !lateInputAck(error) || ["browser_action_cancelled", "stale_document_reference"].includes(error?.code)) && this.isBrowserLive(browserAtStart)) {
          // Clear any dispatched key/button state before another actor can use
          // the browser. Recovery rotates generation; cancelled input never replays.
          await this.stop();
        }
        throw error;
      }
      const post = timestamp();
      if(command.input.kind === 'scroll')this.scrolling.set(tab.tab_id,{document_id:tab.document_id,until:performance.now()+400});
      // MP-08/MP-10: a person's wheel cannot add, close or retarget a Tab; the state events and the next state read do.
      // A viewer's key is the same: no input-lane reconcile (the lane serializes the next key behind it).
      if (['scroll', 'key'].includes(command.input.kind) && command._display_input === true) return this.observe(this.tabState(), null, scope);
      // The page acknowledged this input: a timeout of the observation that follows never makes it replayable.
      // A renderer that is busy cannot answer the observation either: the input was accepted, the last observed Tab state stands.
      // The observation is bounded too: it goes on in the background and a later state read has its result.
      const observation = this.reconcile().catch(error => { if (error?.code === 'browser_cdp_timeout') return null; throw markDispatched(error); });
      observation.catch(() => {});
      const result = this.observe(await withinMs(observation, POST_INPUT_OBSERVE_MS) ?? this.tabState(), null, scope);
      this.timing('input_post_reconcile', post);
      return result;
    }
    if (command.op === "screenshot") {
      // A frame laid out at the viewer's size (protocol 560/168) is the whole page; any other size is a crop of it.
      const hint=command._capture_viewport,scale=this.scales.get(tab.tab_id)??this.viewport().device_scale_factor,page=this.layouts.get(tab.tab_id)??geometry;
      const crop=hint&&(hint.width!==page.width*scale||hint.height!==page.height*scale);
      const frame = command._capture_protection===true?(crop?await captureVisiblePage(this,tab,hint,scope):await this.screenshot(tab,null,true)):await this.displayScreenshot(tab);
      // MD-3: explicit binding is an internal display/MCP seam. Legacy 417
      // still emits its existing frame shape until the coordinator adapter lands.
      if (!command.focused_agent && !command.bound_frames) delete frame.document_id;
      return this.observe(frame, tab, scope);
    }
    if (command.op === "subscribe") return this.subscribe(tab, command.bound_frames === true, command._subscription_owner ?? null);
    throw new Error("MD-2: unsupported browser operation");
  }
  // MP-08/MP-11: an agent drives an App Tab as itself, only after the kernel
  // claimed that App view in the agent's own grant.
  appAgent(tab, command, drives) {
    const app = [...(this.browser.appTabs?.apps?.values() ?? [])].find(open => open.targetId === tab.target_id);
    const agent = command.focused_agent || command._agent_input
      ? /^agent:(.+)$/.exec(String(command.observed_by ?? ""))?.[1] ?? "unattributed-agent" : null;
    if (app && drives && agent && command._app_view_tab !== tab.tab_id) throw new UserDomainRefusal("not_granted");
    return { app, agent };
  }
  async handle(request, { signal } = {}) {
    const browserAtEntry = this.browser;
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
        // MP-08 / MP-10 / MP-11: this internal owner-only flag does not
        // loosen ordinary Vault secret insertion into password fields.
        const codeInput = request.params.mask_code_input === true;
        if (codeInput && (!String(request.params.observed_by ?? "").startsWith("terminal:") ||
            request.params.action?.kind !== "fill" || typeof request.params.action?.expected_document_url !== "string")) {
          throw new UserDomainRefusal("not_granted");
        }
        await this.start({ signal });
        if (this.protection.unknown) throw new Error("MD-5: observation registry unavailable");
        const tab = await this.target(request.params);
        if (tab.document_id !== request.params.document_id) throw new UserDomainRefusal("stale_reference");
        // MP-08/MP-11: a Vault fill is the agent's input to an App Tab too,
        // recorded before the field's input/change handlers can run.
        const { app, agent } = this.appAgent(tab, request.params, true);
        if (app && agent) this.browser.appTabs.noteAgent(tab.target_id, agent);
        const action = codeInput ? { ...request.params.action, mask_code_input: true } : request.params.action;
        await this.sampleLane(tab).run("input", () => this.browser.performAction({ target_id: tab.target_id, document_id: tab.document_id,
          node_ref: request.params.node_ref, action, timeout_ms: 10_000 }, { signal }));
        await this.save();
        return { id: request.id, ok: true, result: { inserted: true } };
      }
      if (request.method === "host.computer") {
        const observer=(request.params._subscription_owner ? request.params._subscription_owner+':' : '')+(request.params.observed_by??'terminal');
        if(request.params._agent_input && this.browser?.appTabs?.apps?.size)throw new UserDomainRefusal('not_granted');
        await admitComputerMutation(this, request.params);
        const nativeParams={...request.params,observed_by:observer};
        if (request.params.op === 'display_subscribe') {
          if(!featureOn('CHARIOX_KERNEL_BROWSER_DISPLAY'))throw new Error('MP-08: display disabled');
          return {id:request.id,ok:true,result:await this.desktopDisplay.subscribe(nativeParams,request.params.observed_by??'terminal')};
        }
        if (['installed_applications','launch_application'].includes(request.params.op)) {
          const binding=this.chromium.desktop?.binding();
          if(!binding || request.params.surface_id!==binding.surface_id || request.params.generation!==binding.generation)throw new Error('MP-11: stale installed application surface');
          if(request.params.op==='installed_applications')return {id:request.id,ok:true,result:{surface_id:binding.surface_id,generation:binding.generation,...await this.chromium.desktop.installedApplications({signal})}};
          const launched=await this.chromium.desktop.launchApplication(request.params.application_id,{signal,observe:()=>this.nativeAccessibility.snapshot(observer,this.protection,{signal})});
          this.onNativeInput();
          return {id:request.id,ok:true,result:launched};
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
        const at=timestamp();
        const result=await this.nativeComputer.request(nativeParams, this.protection, {signal});
        // Fixed labels only: operation and input kind, never input values.
        if(request.params.op==='input')this.timing('computer_input '+(/^[a-z_]{1,16}$/.test(request.params.input?.kind)?request.params.input.kind:'other'),at);
        return { id: request.id, ok: true, result };
      }
      if (request.method === 'host.browser.retire_input') {
        // Retirements arrive from concurrent takeovers: an older ownership change of a tab must not retire that tab's newer owner's input.
        const {tab_id,except,epoch}=request.params;
        if(!(epoch<(this.retiredEpochs.get(tab_id)??0))){this.retiredEpochs.set(tab_id,epoch);this.busyInputs.retire((key, command) => command.tab_id === tab_id && key !== except)}
        return {id:request.id,ok:true,result:{retired:true}};
      }
      if (request.method === 'host.browser.retire_observer') {
        this.busyInputs.retire(key => key === request.params.observer);
        return {id:request.id,ok:true,result:{retired:true}};
      }
      if (request.method === 'host.computer.retire') {
        this.busyInputs.retire(key => key === request.params.observer);
        await this.desktopDisplay.retire(request.params.observer);
        await this.nativeComputer.retire(request.params.observer);
        this.nativeAccessibility.retire(request.params.observer);
        return {id:request.id,ok:true,result:{retired:true}};
      }
      if (request.method === "host.computer.reset") {
        await this.nativeComputer.reset();
        return { id: request.id, ok: true, result: { released: true } };
      }
      // Protocol 556: the kernel parks here, outside its protection barrier, until a state read could differ.
      if (request.method === "host.browser" && request.params?.op === "state_signal") {
        const {after, wait_ms} = request.params;
        const parked = await this.stateSignal.wait(Number.isSafeInteger(after) ? after : null, Math.min(Number(wait_ms) || 0, 5000), signal);
        return { id: request.id, ok: true, result: { signal: parked } };
      }
      if (request.method === "host.browser") {
        if(this.nativeComputer.held.size && ['input','open','close','navigate','resize'].includes(request.params.op))throw new Error('MP-11: native keys held; release desktop input first');
        const at = timestamp();
        // MP-08/MP-11: the kernel's opener ledger names the agent Tabs ([{tab_id, label}]).
        const { _tab_groups: groups, ...params } = request.params ?? {};
        let result;
        try { result = await this.request(params, { signal }); }
        // A person's keys and wheel never wake a state read; Tab changes and an agent acting do.
        finally { if (['open', 'close', 'navigate', 'stop'].includes(params.op) || params.op === 'input' && (params._agent_input || params.focused_agent)) this.stateSignal.notify(); }
        if (Array.isArray(groups)) void this.syncTabGroups(groups).catch(() => {});
        this.timing(request.params.op === 'input' ? 'host_input' : 'host_capture_or_control', at);
        if (TIMED_OPS.has(request.params.op)) this.timing(`host_op ${request.params.op}${typeof request.params.tab_id === 'string' && request.params.op === 'state' ? ' tab' : ''}`, at);
        else if (timestamp() - at > SLOW_OP_MS && /^[a-z_]{1,32}$/.test(request.params.op)) this.timing(`host_op_slow ${request.params.op}`, at);
        // Structured controller observations scrub before compaction; metadata
        // and other host replies receive the same protection at this boundary.
        if (!["screenshot", "poll", "mirror_next", "display_subscribe", "display_input_target"].includes(request.params?.op)) {
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
      if (browserAtEntry && this.recovery?.enabled && (browserAtEntry !== this.browser || browserAtEntry.connection?.isOpen?.() === false)) {
        return { id: request.id, ok: false, error: { code: 'browser_unavailable', message: 'MP-08: browser disconnected; interrupted action was not replayed' } };
      }
      if (error instanceof UserDomainRefusal || error instanceof CaptureRefusal) return {id:request.id,ok:false,error:{code:error.code,message:error.message}};
      if (isTrustedStaleReferenceError(error)) return {id:request.id,ok:false,error:{code:"user_domain_stale_reference",message:"User-domain request refused"}};
      // A cancelled request that may have dispatched input stops the browser (no held keys/buttons). A
      // cancelled read-only observation (a viewer's credit on a huge page) must not stop every Tab (MP-08/MP-10).
      if (error?.code === "browser_action_cancelled" && request.method !== "host.computer" && !request.params?.display_subscription_id
        && this.browser === browserAtEntry && !(request.method === "host.browser" && OBSERVATION_OPS.has(request.params?.op))) await this.stop();
      if (["browser_unavailable"].includes(error?.code)) {
        return { id: request.id, ok: false, error: { code: error.code, message: error.message } };
      }
      if (error?.code === "browser_dialog_invalid") return { id: request.id, ok: false, error: {
        code: "browser_dialog_invalid", message: "Browser dialog answer is invalid or no longer pending; read the current browser state." } };
      return { id: request.id, ok: false, error: { code: error?.code === "browser_action_cancelled" ? "browser_action_cancelled" : "kernel_browser_failed", message: error instanceof MirrorInputEpochRefusal ? "MP-11: stale mirror input epoch" : error instanceof PointerOutsideViewport ? error.message : hostFailureMessage(error) } }; }
  }
}

if (process.argv[2] === "stdio" && process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const host = new KernelBrowserHost(process.argv[3]);
  let closing = false;
  const stop = async () => { if (closing) return; closing = true; await host.stop(); process.exit(0); };
  process.on("SIGTERM", stop);
  process.on("SIGINT", stop);
  await new BrowserControllerStdioServer(timedHostRequests(host, request => scheduleHostRequest(request, host.browser, host.viewport(), Boolean(host.chromium.desktop?.binding?.())))).run().finally(() => host.stop());
}
