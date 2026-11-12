import { captureBackgroundRenderer } from './kernel-browser-background.mjs';
import { AppTabs } from './browser-controller-apps.mjs';
import { HostDownloads } from "./kernel-browser-downloads.mjs";
import { BrowserHeaderState } from "./kernel-browser-header.mjs";
import {resolveDisplayTarget,nativeStateObservationOptions} from './kernel-browser-display-document.mjs';
import { rememberCardReferences, resolveCardRegion, forgetCardAnchor } from './kernel-browser-card-region.mjs';
import {capturePageScreenshot} from './kernel-browser-screenshot.mjs';
import {REGION_KEYS,regionScreenshot} from './kernel-browser-screenshot-region.mjs';
import {saveResource} from './kernel-browser-save.mjs';
import {waitForPage} from './kernel-browser-wait.mjs';
import { mirror2ObserverExpression } from './kernel-browser-mirror2-observer.mjs';
import {captureVisiblePage,pngFallbackCommand,subscribeVisiblePage,visiblePageInputSource,viewerPageInputActive,nativeInputDocumentTarget} from './kernel-browser-page-display.mjs';
import {reserveDisplay,maxDisplays} from './kernel-browser-display-admission.mjs';
import { appRestoreOrigin } from "./browser-app-restore.mjs";
import { DesktopDisplay } from './kernel-desktop-display.mjs';
import {displayGeometry as geometry,hostViewport,featureOn} from './kernel-browser-geometry.mjs';
// MD-2: sessionless host adapter over the shared controller/CDP implementation.
import { AgentToolRefusal, CaptureRefusal, PointerOutsideViewport, UserDomainRefusal } from "./kernel-browser-refusal.mjs";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { displayTiming, slowMark, timestamp, timedHostRequests } from './kernel-browser-timing.mjs';
import { hostErrorLabel, hostFailureMessage } from './kernel-browser-error-label.mjs';
// The kernel's daemon log carries the lines of the controller's stderr that start with this prefix (fixed labels only).
const lifecycleLog = text => process.stderr.write(`CHARIOX-LOG ${text}\n`);
import { BrowserCdpClient, isTrustedStaleReferenceError } from "./browser-controller-cdp.mjs";
import { BrowserControllerStdioServer, handleBrowserControllerRequest } from "./browser-controller.mjs";
import { NativeAccessibility } from './native-accessibility.mjs';
import { NativeComputer } from './native-computer.mjs';
import { HostChromium } from "./kernel-browser-process.mjs";
import { BrowserRecovery } from "./kernel-browser-recovery.mjs";
import { requireBrowserMemory } from './browser-memory.mjs';
import { admitComputerMutation, blockedUrl, extensionManagementUrl, EXTENSIONS_REFUSED } from "./kernel-browser-policy.mjs";
import { ChromeTabGroups, writeTabGroupsExtension } from "./kernel-browser-tab-groups.mjs";
import {nativeWindowInventory} from './kernel-browser-native-windows.mjs';
import { ChromeWindows, InputArbiter } from "./kernel-browser-chrome-window.mjs";
import { StateSignal, stateChangingEvent } from "./kernel-browser-state-signal.mjs";
import { redactObservation, redactSnapshot, redactBrowserState } from "./browser-controller-snapshot.mjs";
import {resizeBrowserPage} from './kernel-browser-resize.mjs';
import { TextInsertionRefused, inputHostTab } from "./kernel-browser-input.mjs";
import { publicUploadFailure } from "./kernel-browser-upload.mjs";
import { BusyInputQueue, lateInputAck, markDispatched } from "./kernel-browser-input-queue.mjs";
import { BrowserPopupEvidence, popupActionId } from "./kernel-browser-popup-evidence.mjs";
import { assertNotCancelled, BrowserActionError } from "./browser-controller-actions.mjs";
import { captureProtectedDisplay } from "./kernel-browser-region-protection.mjs";

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
  return ['browser.app.calls','browser.app.respond','browser.app.agent','host.browser.retire_input','host.browser.retire_observer'].includes(request.method) || request.method === 'host.browser' && request.params?.op === 'dialog' ||
    ['host.browser','host.computer'].includes(request.method) && (request.params?.op === 'screenshot' &&
    typeof request.params.display_subscription_id === 'string' || request.params?.op === 'mirror_next' && typeof request.params.subscription_id === 'string' ||
    (request.params?.op === 'state' || request.params?.op === 'display_subscribe' && (request.params.target || nativeDesktop && request.method === 'host.browser')) && browser?.canReconcileConcurrently?.(viewport) === true ||
    // An agent's save waits on the page session's fetch: it resolves only its own Tab and checks its authority before writing.
    request.params?.op === 'state_signal' || request.params?.op === 'wait' || request.params?.op === 'save' || request.params?.op === 'state' && browser?.dialogDefaults?.pending().length > 0 ||
    ['chrome_window_follow', 'note_selection', 'pointer_cursor', 'popup_region'].includes(request.params?.op) ||
    // Releasing a display's compositor takes seconds; the person's input and resize must not queue behind it.
    request.method === 'host.browser' && request.params?.op === 'unsubscribe')
    ? {kind:'bridge'} : {kind:'barrier'};
}
const TAB_LIMIT = 128;
// A Tab the person types or clicks in is theirs until they have been idle this long (an agent's input to it waits).
export const PERSON_IDLE_MS = 2000;
const SHOW_TAB_WAIT_MS = 1000;
// The answer to a Show tab carries the state read if it finishes in time; a busy renderer of another Tab never holds it.
const SHOW_TAB_OBSERVE_MS = 500;
// How often the focus emulation of the Tabs agents work in behind the shown one is asserted again.
const BACKGROUND_KEEP_MS = 1500;
// An agent Tab renders at full speed behind the shown one only this long after the agent's last action in it; an idle
// one stops scheduling frames, and the agent's next action wakes it before acting.
export const BACKGROUND_IDLE_MS = 30_000;
// The shown Tab renders into the owned window at full rate whether or not anyone watches it. With no viewer stream
// and no request for this long it is frozen (hidden: no frames, no page tasks) until the next request wakes it.
export const SHOWN_IDLE_MS = 30_000;
const SHOWN_IDLE_CHECK_MS = 5_000;
// A group's mark: a dot at rest, a spinner while the agent acts, and "needs you" while a card asks the person to act in one of its Tabs.
const SPINNER = ['◐', '◓', '◑', '◒'], IDLE_MARK = '●', NEEDS_MARK = '▲';
// How long a spinner stays after an agent's last action in a Tab, so a short action is still seen.
const SPINNER_LINGER_MS = 1500;
// Read-only host operations: they never dispatch input into a page.
const OBSERVATION_OPS = new Set(['chrome_window_follow', 'state', 'snapshot', 'screenshot', 'save', 'wait', 'unsubscribe', 'note_selection', 'mirror_subscribe', 'mirror_next', 'mirror_close', 'display_subscribe', 'display_attach', 'display_suspend', 'display_resume', 'display_input_target', 'pointer_cursor', 'popup_region']);
// MP-08/MP-10: opt-in timing names Tab lifecycle ops (fixed labels, never page values).
const SLOW_OP_MS = 1000;
const POST_INPUT_OBSERVE_MS = 500;
// A click or key arriving while the page is loading waits this long for its document to be identified, instead of being refused.
const INPUT_DOCUMENT_WAIT_MS = 1500;
// `promise`'s value, or null once `ms` have passed (the promise is left to settle on its own).
const withinMs = (promise, ms) => { let timer; return Promise.race([promise, new Promise(resolve => { timer = setTimeout(resolve, ms, null); })]).finally(() => clearTimeout(timer)); };
const TIMED_OPS = new Set(['start', 'state', 'open', 'close', 'navigate', 'mirror_subscribe', 'display_subscribe']);
const inputKinds = { click: 1, text: 2, key: 3, scroll: 4 };
const disablesWebBotAuth = value => value === null || (value && typeof value === "object" && value.disabled === true);
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
  constructor(root, { appStorageRoot, chromium = new HostChromium(root, { appStorageRoot }), browserFactory = connection => new BrowserCdpClient({ connectionFactory: () => connection, permissionRequests: true, focusWorldSource: mirror2ObserverExpression() }), personIdleMs = PERSON_IDLE_MS, backgroundIdleMs = BACKGROUND_IDLE_MS, shownIdleMs = SHOWN_IDLE_MS } = {}) {
    this.root = root;
    this.personIdleMs = personIdleMs; this.backgroundIdleMs = backgroundIdleMs;
    this.shownIdleMs = shownIdleMs; this.usedAt = Date.now(); this.frozen = null; this.freezing = null;
    this.shownTimer = setInterval(() => { this.freezing ??= this.freezeIdleShown().catch(() => {}).finally(() => { this.freezing = null; }); }, SHOWN_IDLE_CHECK_MS);
    this.shownTimer.unref?.();
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
    this.webBotAuthRestoreUrls = new Map();
    this.webBotAuthProjects = new Map();
    this.popupEvidence = new BrowserPopupEvidence(TAB_LIMIT);
    this.displays = new Map();
    this.displayTeardowns = new Set();
    this.displayTeardownErrors = [];
    this.displaySessions = new DisplaySessions(this);
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
    // The person's last deliberate input per Tab, and the background-run leases of Tabs agents work in.
    this.personInputAt = new Map(); this.backgroundHolds = new Map();
    this.agentBusy = new Map(); this.spin = 0; this.spinner = null; this.groupLabels = null; this.keepTimer = null;
    this.restoring = false;
    this.adoptedByInput = new WeakSet();
    this.keepaliveTarget = null;
    this.observedDocuments = new Map();
    this.cardReferences = new Map();
    this.cardAnchors = new Map();
    this.displayDocuments = new DisplayDocuments((stage, started, ended) => this.timing?.(stage, started, ended));
    this.stateSignal = new StateSignal();
    this.downloads = new HostDownloads({ ownerRoot: root, notify: () => this.stateSignal.notify() });
    this.header = new BrowserHeaderState(() => this.stateSignal.notify());
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
    if (policy.display_masked !== undefined && typeof policy.display_masked !== 'boolean') throw new Error('MD-5: invalid display audience');
    // The kernel's JSON has its own key order: compare the fields, or the first request would retire every bound stream.
    const fields = ({ values, targets, unknown, display_masked }) => JSON.stringify([values, targets, unknown, display_masked]);
    if (fields(policy) === fields(this.protection)) return {};
    const before = this.protection, changed = (a, b) => Number(JSON.stringify(a) !== JSON.stringify(b));
    // Counts only: which part of the policy replaced every bound stream.
    this.timing?.event?.('protection_replaced', { values_before: before.values.length, values_after: policy.values.length, values_changed: changed(before.values, policy.values),
      targets_before: before.targets.length, targets_after: policy.targets.length, targets_changed: changed(before.targets, policy.targets), unknown_before: Number(before.unknown === true), unknown_after: Number(policy.unknown === true) });
    this.protection = policy;
    this.stateSignal.notify();
    await this.desktopDisplay.retireSource();
    this.nativeAccessibility.clear();
    this.mirror.invalidate();
    await this.closeCompositors();
    for (const stream of this.displays.values()) stream.invalidate(true, 'protection');
    if (this.browser) this.browser.protectedValues = new Set(policy.values);
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
      tabs: [...this.tabs.values()].filter(tab => !this.browser?.appTabs?.apps || ![...this.browser.appTabs.apps.values()].some(app => app.targetId === tab.target_id)).slice(0, TAB_LIMIT).map(({ tab_id, url }) => {
        const deferred = this.webBotAuthRestoreUrls.get(tab_id);
        const projectId = this.webBotAuthProjects.get(tab_id) ?? deferred?.projectId;
        const restoreUrl = deferred?.url ?? url;
        const safeUrl = redactObservation(restoreUrl, this.protection.values) === restoreUrl ? restorationUrl(restoreUrl) : "about:blank";
        return { tab_id, url: safeUrl, ...(projectId ? { web_bot_auth: true, web_bot_auth_project_id: projectId } : {}) };
      }) };
    // App intent remains kernel-owned; only its origin selects the restored view.
    data.app_origins = [...new Set([
      ...[...(this.browser?.appTabs?.apps?.values() ?? [])].map(app => appRestoreOrigin(app.origin)).filter(Boolean),
      ...(this.pendingActive?.apps ?? []),
    ])].slice(0, TAB_LIMIT);
    data.active_tab_id = this.pendingActive?.tab_id ?? active?.tab_id ?? null;
    data.active_app_origin = this.pendingActive?.app_origin ?? appRestoreOrigin(active?.url) ?? null;
    data.window_layout = this.resizeSettled ? { width: this.resizeSettled.width, height: this.resizeSettled.height } : null;
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
    if (!allowStart && this.recovery.waiting()) throw new BrowserActionError("browser_unavailable", "MP-08: the browser was lost and restarts after a short wait");
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
    await Promise.all([...this.displays.values()].map(stream=>this.removeDisplay(stream)));
    await Promise.all(this.displayTeardowns);
    await this.closeCompositors();
    this.foreground.reset();
    this.sampleLanes.clear();this.inputChangedAt.clear();this.scrolling.clear();

    clearInterval(this.keepTimer); this.keepTimer = null; this.frozen = null;
    await Promise.all([...this.backgroundHolds.keys()].map(target => this.releaseBackground(target)));
    // MP-11: retire helpers on the old display before Chromium recovery can
    // replace it. A failed key release still requires owned desktop teardown.
    let preserveDesktop = this.chromium.desktop?.isLive?.() === true;
    try { await this.nativeComputer.close(); }
    catch { preserveDesktop = false; } // a failed physical release still resets the owned display
    finally { await this.chromium.stop(undefined, { preserveDesktop }); this.nativeAccessibility.clear(); }
    this.mirror.clear();
    this.displays.clear(); this.scales.clear(); this.inputEpochs.clear();
    this.tabGroups?.close?.();this.tabGroups=null;
    await this.browser?.close();
    await this.downloads.close();
    this.browser = null;
    if(this.displayTeardownErrors.length)throw new AggregateError(this.displayTeardownErrors.splice(0),'MD-DISPLAY: owned display teardown failed');
    this.backgroundHolds.clear();this.personInputAt.clear();
    clearInterval(this.spinner);this.spinner=null;this.agentBusy.clear();clearInterval(this.keepTimer);this.keepTimer=null;this.frozen=null;
    this.tabs.clear();this.header.clear();this.layouts.clear();this.layoutRevisions.clear();this.windowLayout=null;this.resizeSettled=null;
    this.popupEvidence.clear();
    this.observed = ++this.observations;
    this.observedDocuments.clear();
    this.cardReferences.clear();
    for (const id of this.cardAnchors.keys()) forgetCardAnchor(this, id);
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
    this.browser.appTabs = new AppTabs(this.browser, { windowMode: "tabbed" });
    this.browser.pageTiming = this.timing;
    this.browser.onPermissionChanged = () => this.stateSignal.notify();
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
      await this.downloads.start(connection, this.browser);
      // MP-08/MP-11: agent Tabs are grouped under the agent's name in the native strip.
      // Attribution UI only: a browser without the component extension still works.
      this.tabGroups = new ChromeTabGroups(connection, () => this.stateSignal.notify());
      this.browser.nativeViewport = target => this.tabGroups?.viewport(target) ?? null;
      try { await this.tabGroups.load(await writeTabGroupsExtension(this.chromium.filesRoot ?? this.root)); } catch { this.tabGroups?.close?.();this.tabGroups = null; this.timing('tab_groups_unavailable', timestamp()); }
      const { targetInfos = [] } = await connection.send("Target.getTargets");
      // Headed Chromium exits when its last page closes. Keep one internal blank
      // target so startup and closing the last user tab never stop the browser.
      const pages = targetInfos.filter(target => target.type === "page");
      this.keepaliveTarget = await this.openKeepalive(connection, pages);
      await this.tabGroups?.keepalive(this.keepaliveTarget);
      for (const target of (await connection.send("Target.getTargets")).targetInfos.filter(target => target.type === "page")) {
        if (target.targetId !== this.keepaliveTarget) {
          await connection.send("Target.closeTarget", { targetId: target.targetId });
        }
      }
      for (const tab of saved.tabs.slice(0, TAB_LIMIT)) {
        if (typeof tab.tab_id !== "string" || !tab.tab_id.startsWith("host-tab-")) throw new Error("MD-2: invalid saved tab identity");
        assertNotCancelled(signal);
        const url = restorationUrl(tab.url);
        const defer = tab.web_bot_auth === true && typeof tab.web_bot_auth_project_id === "string" && url !== "about:blank";
        if (defer) this.webBotAuthProjects.set(tab.tab_id, tab.web_bot_auth_project_id);
        await this.createTab(defer ? "about:blank" : url, tab.tab_id, {
          signal,
          background: true,
          ...(defer ? { deferredWebBotAuthRestoreUrl: url, webBotAuthProjectId: tab.web_bot_auth_project_id } : {}),
        });
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
      await this.restoreWindowLayout(saved.window_layout, signal);
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
    // A card asking the person to act in one of an agent's Tabs marks that agent's group (not saved: it follows the cards).
    this.groupNeeding = new Set(groups.filter(group => group?.needs_person === true && typeof group.label === 'string').map(group => group.label.slice(0, 64)));
    this.groupLabels = labels;
    await this.applyTabGroups();
    if (!this.restoring) await this.save();
  }
  // Each agent's group carries its status dot, and a spinner while the agent acts in one of its Tabs.
  async applyTabGroups() {
    const labels = this.groupLabels ?? new Map();
    const busy = [...labels].filter(([, targets]) => [...this.agentBusy.keys()].some(id => targets.includes(this.tabs.get(id)?.target_id)));
    await this.tabGroups?.sync([...labels].map(([label, target_ids]) => ({ label: label.slice(0, 64), target_ids, mark: this.groupNeeding?.has(label) ? NEEDS_MARK : busy.some(([name]) => name === label) ? SPINNER[this.spin % SPINNER.length] : IDLE_MARK })));
  }
  noteAgentBusy(tabId, delta) {
    const count = (this.agentBusy.get(tabId) ?? 0) + delta;
    if (count > 0) this.agentBusy.set(tabId, count); else this.agentBusy.delete(tabId);
    if (this.agentBusy.size && !this.spinner) {
      this.spinner = setInterval(() => {
        this.spin++;
        if (!this.agentBusy.size) { clearInterval(this.spinner); this.spinner = null; }
        void this.applyTabGroups().catch(() => {});
      }, 400);
      this.spinner.unref?.();
    }
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
  noteLayout(tabId, before) { const now = this.pageSize(tabId); if (now.width !== before.width || now.height !== before.height) { this.layoutRevisions.set(tabId, this.layoutRevision(tabId) + 1); this.timing?.event?.('page_layout_changed', { width_delta: now.width - before.width, height_delta: now.height - before.height }); } }
  layoutRevision(tabId) { return this.layoutRevisions.get(tabId) ?? 0; }
  async setLayout(tab, subscriptionId, layout) {
    if(subscriptionId!=='resize'&&this.layouts.get(tab.tab_id)?.subscriptionId==='resize')return;
    if(this.chromium.desktop) {
      const native=await this.browser.nativeViewport?.(tab.target_id);
      if(native?.target_id!==tab.target_id||![native?.width,native?.height].every(n=>Number.isSafeInteger(n)&&n>=2))throw new UserDomainRefusal('stale_reference');
      // The shared desktop includes Chrome's band and window decorations.
      // Emulation beyond its real page produces uncapturable canvas margins.
      const requested=layout??{width:this.viewport().css_width,height:this.viewport().css_height};
      layout={width:Math.min(requested.width,2*Math.floor(native.width/2)),height:Math.min(requested.height,2*Math.floor(native.height/2))};
    }
    const before = this.pageSize(tab.tab_id);
    await this.browser.setLayout?.(tab.target_id, layout);
    if (layout) this.layouts.set(tab.tab_id, { ...layout, subscriptionId }); else this.layouts.delete(tab.tab_id);
    this.noteLayout(tab.tab_id, before);
  }

  // A restarted browser opens at its default size. The viewer that sized it before would resize it again right after it
  // attached, and that re-snapshots the page it is typing into. Give the window that size again before anyone attaches.
  async restoreWindowLayout(layout, signal) {
    const tab = this.tabs.values().next().value;
    if (!tab || !Number.isSafeInteger(layout?.width) || !Number.isSafeInteger(layout?.height)) return;
    // Not restoring it changes nothing but speed: the next viewer resizes the window as it always did.
    try { await resizeBrowserPage(this, tab, { width: layout.width, height: layout.height, generation: this.generation }, signal); }
    catch { this.timing('window_layout_restore_failed', timestamp()); }
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
    // A viewer-sized window owns the viewport: a release that runs late (it no longer holds the backend lock) must not clear it.
    const tab=this.tabs.get(tabId);if(!tab||this.windowLayout)return;
    try{await this.browser.clearViewport(tab.target_id);}catch{}
  }
  async closeCompositors(tabId) {
    const entries=[];
    for(const [id,entry] of this.compositors)if(tabId===undefined||id===tabId){this.compositors.delete(id);entries.push(entry);}
    await Promise.all(entries.map(async entry=>{
      await entry.ready.catch(()=>null);
      await entry.source?.close().catch(error=>{this.displayTeardownErrors.push(error);this.timing('display_teardown_failed',timestamp());});
    }));
  }
  // MP-08/MP-10: one fixed-label diagnostic per change of native refusal scope.
  nativeScope(reason){if(this.nativeScopeReason!==reason){this.nativeScopeReason=reason;if(reason)this.timing('native_unavailable_scope '+reason,timestamp());}}
  async compositorFor(tab,stream) {
    if(stream.regionSource)return stream.regionSource;
    // MP-08/MP-10/MP-11: mirror capture/input never loads the video backend.
    const {LinuxCapture,selectNativeCapture} = await import('./kernel-browser-native.mjs');
    const {CompositorSource} = await import('./kernel-browser-compositor.mjs');
    const scope=stream.codec==='png'?'png_codec':this.protection.unknown?'protection_policy':null;
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
        const layoutRevision=this.layoutRevision(tab.tab_id),windowLayout=this.windowLayout;
        const source=new LinuxCapture({size:this.pageSize(tab.tab_id),pageWindow:this.layouts.get(tab.tab_id)?.subscriptionId==='resize'?async()=>{await this.chromeWindows.resolve();return this.chromeWindows.current().tabbed}:undefined,display:this.chromium.display,pid:this.chromium.browserPid,connection,sessionId,tab,scale:stream.device_scale_factor,hostScale:this.chromium.scale??1,policy,browser:this.browser,frames:frameId=>this.browser.frameSession?.(frameId,connection),screenshot:()=>this.displayScreenshot(tab,null,false),allowed:p=>this.windowLayout===windowLayout&&this.layoutRevision(tab.tab_id)===layoutRevision&&this.foreground.holds(tab.tab_id,claim)&&this.protection===p&&!p.unknown&&this.generation===generation,timing:this.timing});
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
      // A credit still in flight for a removed display starts no compositor: nothing would stop it.
      if(this.displays.get(stream.subscription_id)!==stream){if(source instanceof LinuxCapture)await source.close();return null;}
      const created={source,document:tab.document_id,policy};
      created.ready=source instanceof LinuxCapture?Promise.resolve(source):source.start().catch(()=>{created.refusals=refusals+1;created.retryAt=performance.now()+Math.min(30_000,1000*2**refusals);return null;});
      entry=created;this.compositors.set(tab.tab_id,entry);
    }
    return await entry.ready;
  }
  async stop({ recovering = false, preserveDesktop = false } = {}) {
    lifecycleLog(recovering ? 'browser_stop recovering' : 'browser_stop');
    if (!recovering) {
      this.recovery?.stop();
      await this.starting?.catch(() => {});
      this.recovery?.stop();
    }
    this.busyInputs.retireAll(); // a stopped browser is not relaunched by an input that waited for its renderer
    await this.desktopDisplay.close();
    await Promise.all([...this.displays.values()].map(stream=>this.removeDisplay(stream)));
    await Promise.all(this.displayTeardowns);
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
    this.mirror.clear();
    this.displays.clear(); this.scales.clear();
    this.tabGroups?.close?.();this.tabGroups=null;
    await this.browser?.close();
    await this.downloads.close();
    this.browser = null;
    this.timing.flush?.();
    const errors=this.displayTeardownErrors.splice(0);
    if(errors.length)throw new AggregateError(errors,'MD-DISPLAY: owned display teardown failed');
    return { state: "stopped", generation: this.generation, tabs: [] };
  }
  // MP-08/MP-10/MP-11: reconciles can overlap (a state poll beside input). They
  // apply in start order: an older observation reports the newer current tabs.
  tabState() {
    const active = this.pendingActive ? null : [...this.tabs.values()].find(tab => tab.target_id === this.activeTarget)?.tab_id ?? null;
    this.header.update([...this.tabs.values()], active, this.browser, this.tabGroups);
    // MP-08: private creation evidence for the kernel, stripped before client projection.
    const _tab_creation_actions = this.popupEvidence.inventory([...this.tabs.values()]);
    const dialogs = this.pendingDialogs();
    // A popup names the Tab whose page opened it (never the CDP target id), so a window that shares this browser
    // can tell its own page's popup from one another window's page opened.
    const tabIds = new Map([...this.tabs.values()].map(tab => [tab.target_id, tab.tab_id]));
    return redactBrowserState({ state: "ready", generation: this.generation, _observation: this.observed, _tab_creation_actions,
      active_tab_id: active, downloads: this.downloads.state(), tabs: [...this.tabs.values()].map(({ target_id, opener_target_id, ...tab }) => ({
        ...tab,
        ...(tabIds.has(opener_target_id) ? { opener_tab_id: tabIds.get(opener_target_id) } : {}),
        ...this.header.read(tab),
        ...(this.webBotAuthRestoreUrls.has(tab.tab_id) ? { title: "Web Bot Auth restore requires an agent to resume this tab" } : {}),
      })), dialogs, viewport:this.viewport(), chrome_window: this.chromeWindow(),
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
    for (const [id, tab] of this.tabs) {
      tab.tab_id = id;
      if (!this.downloads.bindings.has(tab.target_id)) this.downloads.bind(tab,
        this.downloads.bindings.get(tab.opener_target_id)?.session ?? "browser-viewer");
    }
    for (const target of [...this.backgroundHolds.keys()]) if (![...this.tabs.values()].some(tab => tab.target_id === target)) void this.releaseBackground(target);
    for (const id of [...this.personInputAt.keys()]) if (!this.tabs.has(id)) this.personInputAt.delete(id);
    // A surviving Tab keeps its committed page size; only retired Tabs drop theirs.
    for (const map of [this.layouts, this.layoutRevisions]) for (const id of [...map.keys()]) if (!this.tabs.has(id)) map.delete(id);
    // A Tab that opened in the resized window is laid out at the window's size, not the canonical one.
    // A Tab left at an older size by a resize that failed part-way is laid out again too.
    if (this.windowLayout) for (const tab of [...this.tabs.values()]) { const l = this.layouts.get(tab.tab_id); if (l?.subscriptionId !== 'resize' || l.width !== this.windowLayout.width || l.height !== this.windowLayout.height) await this.setLayout(tab, 'resize', this.windowLayout); }
    const previous = this.activeTarget;
    const intent = this.pendingActive;
    const boundOrigins = new Set([...(this.browser.appTabs?.apps?.values() ?? [])].filter(app => app.ready === true).map(app => app.origin));
    const restoreReady = intent && (intent.apps.every(origin => boundOrigins.has(origin)) || Date.now() >= intent.until);
    const nativeActive = await this.tabGroups?.active?.().catch(() => null) ?? null;
    // A newer observation (Show tab, a navigation commit, a later reconcile) owns the active identity from here on.
    if (observation < this.observed) return this.tabState();
    // MP-08/MP-10: a Tab the page opened after the human input (window.open, target=_blank) is what Chromium now shows;
    // the opener's input selection must not pull the foreground back over it.
    const openedByInput = intent?.known && nativeActive && !intent.known.has(nativeActive) && [...this.tabs.values()].some(tab => tab.target_id === nativeActive);
    if (openedByInput && this.pendingActive === intent) { this.pendingActive = null; this.adoptedByInput.add(intent); }
    const restored = !openedByInput && !this.restoring && restoreReady && [...this.tabs.values()].find(tab => intent.app_origin
      ? [...(this.browser.appTabs?.apps?.values() ?? [])].some(app => app.ready === true && app.targetId === tab.target_id && app.origin === intent.app_origin)
      : tab.tab_id === intent.tab_id);
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
    if (observation < this.observed) return this.tabState();
    this.activeTarget = restored?.target_id ?? nativeActive;
    if (this.activeTarget !== previous) await this.retireNativeForeground(this.activeTarget);
    const activeId = [...this.tabs.values()].find(tab => tab.target_id === this.activeTarget)?.tab_id ?? null;
    this.header.update([...this.tabs.values()], activeId, this.browser, this.tabGroups);
    const history = this.header.history(activeId);
    if (history) await withinMs(history, 10);
    const result = this.tabState();
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
  async open(url, tabId = `host-tab-${randomUUID()}`, { signal, downloadSession, background = false } = {}) {
    // A background Tab leaves the shown Tab, and the native claim on it, as they are.
    if (!background) { await this.closeCompositors();this.foreground.reset(); }
    await this.createTab(url, tabId, { signal, downloadSession, background });
    const state = await this.reconcile();
    return { ...state, tab_id: tabId };
  }
  async createTab(url, tabId, { signal, background = false, downloadSession = "browser-viewer", deferredWebBotAuthRestoreUrl, webBotAuthProjectId } = {}) {
    requireBrowserMemory();
    if (this.tabs.size >= TAB_LIMIT) throw new Error("MD-2: host tab limit reached");
    const connection = await this.browser.ensureConnection();
    const browser = this.browser;
    assertNotCancelled(signal);
    const deferredRestore = typeof deferredWebBotAuthRestoreUrl === "string" ? navigationUrl(deferredWebBotAuthRestoreUrl) : null;
    const targetUrl = deferredRestore ? "about:blank" : navigationUrl(url);
    const signFirstNavigation = browser.webBotAuth?.byTab.has(tabId) === true;
    const canAttachFetch = signFirstNavigation && typeof browser.ensureTargetSession === "function";
    const { targetId } = await connection.send("Target.createTarget", { url: canAttachFetch ? "about:blank" : targetUrl, ...(background ? { background: true } : {}) });
    if (browser !== this.browser) throw new BrowserActionError('browser_unavailable', 'MP-08: browser restarted; interrupted action was not replayed');
    this.tabs.set(tabId, { tab_id: tabId, target_id: targetId, url, title: "", document_id: "" });
    if (deferredRestore) this.webBotAuthRestoreUrls.set(tabId, { url: deferredRestore, projectId: webBotAuthProjectId });
    this.downloads.bind(this.tabs.get(tabId), downloadSession);
    this.observed = ++this.observations;
    try {
      if (canAttachFetch) {
        const sessionId = await browser.ensureTargetSession(connection, targetId);
        await browser.attachWebBotAuth(tabId, targetId, sessionId);
        if (targetUrl !== "about:blank") await connection.send("Page.navigate", { url: targetUrl }, sessionId);
      }
    } catch (error) {
      this.tabs.delete(tabId);
      if (browser.closePageTarget) await browser.closePageTarget(connection, targetId).catch(() => {});
      throw error;
    }
  }
  async resumeWebBotAuthRestore(tabId, { allowUnsigned = false } = {}) {
    const deferred = this.webBotAuthRestoreUrls.get(tabId);
    if (!deferred) return;
    if (!allowUnsigned && !this.browser?.webBotAuth?.byTab?.has(tabId)) return;
    const tab = this.tabs.get(tabId);
    if (!tab) {
      this.webBotAuthRestoreUrls.delete(tabId);
      this.webBotAuthProjects.delete(tabId);
      return;
    }
    const connection = await this.browser.ensureConnection();
    const sessionId = await this.browser.ensureTargetSession(connection, tab.target_id);
    await connection.send("Page.navigate", { url: deferred.url }, sessionId);
    this.webBotAuthRestoreUrls.delete(tabId);
    tab.url = deferred.url;
    this.stateSignal.notify();
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
  /** The Tab this host already owns under that generation: activating it needs no read of the other Tabs' renderers. */
  ownedTab(command) {
    if (command.generation !== this.generation) throw new UserDomainRefusal("stale_epoch");
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
  screenshot(tab, clip = null, protectedCapture = false, format = "png", optimizeForSpeed = true, mask = true) {
    return capturePageScreenshot(this, tab, clip, protectedCapture, format, optimizeForSpeed, mask);
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
    this.displays.delete(stream.subscription_id);
    const last=![...this.displays.values()].some(s=>s.tab_id===stream.tab_id);
    // Detachment ends delivery immediately; the host owns reaping until stop.
    // The Tab's compositor serves only its displays: its captures stop alongside the stream's slower close, never behind it.
    const closing=Promise.allSettled([last?this.closeCompositors(stream.tab_id):undefined,stream.close()]).then(results=>{
      for(const result of results)if(result.status==='rejected'){
        this.displayTeardownErrors.push(result.reason);this.timing('display_teardown_failed',timestamp());
      }
    }).finally(()=>this.displayTeardowns.delete(closing));
    this.displayTeardowns.add(closing);
    if(last)await this.releaseTabScale(stream.tab_id);
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
  async request(command, options) {
    const person = !command._agent_input && !command.focused_agent;
    if (person) this.notePersonInput(command);
    const acting = !person && typeof command.tab_id === 'string' && ['input', 'navigate', 'snapshot', 'screenshot', 'save', 'wait', 'resize'].includes(command.op) ? command.tab_id : null;
    if (acting) this.noteAgentBusy(acting, 1);
    try {
      return await (command.op === "input" && person
        ? this.busyInputs.submit(command.observed_by ?? "adapter", command, options) : this.requestNow(command, options));
    } finally {
      if (acting) {
        setTimeout(() => this.noteAgentBusy(acting, -1), SPINNER_LINGER_MS).unref?.();
        // Reconcile the background lease after navigation or capture changes its session.
        const target = this.tabs.get(acting)?.target_id;
        if (target !== undefined) void this.assertBackground(target).catch(() => {});
      }
    }
  }
  // The person's typing, click or scroll in a Tab keeps that Tab theirs until they have been idle for PERSON_IDLE_MS.
  notePersonInput(command) {
    if (!['input', 'mirror_input', 'display_input'].includes(command.op) || typeof command.tab_id !== 'string') return;
    if (command.action?.kind === 'selection') return;
    this.personInputAt.set(command.tab_id, Date.now());
  }
  async personQuiet(tabId, signal) {
    for (;;) {
      assertNotCancelled(signal);
      const wait = (this.personInputAt.get(tabId) ?? 0) + this.personIdleMs - Date.now();
      if (wait <= 0) { this.personInputAt.delete(tabId); return; }
      await delay(Math.min(wait, 250));
    }
  }
  // MP-08/MP-10/MP-11: background agent Tabs share the capture owner's wake-up lease.
  // An independent CDP enable is undone when a display releases its last capture.
  async holdBackground(tab) {
    if (!this.backgroundHolds.has(tab.target_id)) {
      this.backgroundHolds.set(tab.target_id, { pending: null, lease: null, tabId: tab.tab_id, usedAt: 0 });
      if (!this.keepTimer) { this.keepTimer = setInterval(() => void this.keepBackground(), BACKGROUND_KEEP_MS); this.keepTimer.unref?.(); }
    }
    this.backgroundHolds.get(tab.target_id).usedAt = Date.now();
    await this.assertBackground(tab.target_id);
  }
  // Releases the holds of agent Tabs idle past the linger and asserts the rest; no timer runs without a hold.
  async keepBackground() {
    for (const [target, hold] of this.backgroundHolds) {
      if (this.agentBusy.has(hold.tabId)) hold.usedAt = Date.now();
      else if (Date.now() - hold.usedAt > this.backgroundIdleMs) await this.releaseBackground(target);
    }
    if (!this.backgroundHolds.size) { clearInterval(this.keepTimer); this.keepTimer = null; }
    await this.assertBackground();
  }
  async assertBackground(only) {
    for (const [target, hold] of this.backgroundHolds) {
      if (only !== undefined && target !== only) continue;
      if (!hold.pending) hold.pending = (async () => {
        if (target === this.activeTarget) {
          const lease = hold.lease; hold.lease = null;
          await lease?.release();
          return;
        }
        const { connection, sessionId } = await this.browser.resolvePageTarget(target);
        if (this.backgroundHolds.get(target) !== hold) return;
        if (hold.lease?.connection !== connection || hold.lease?.sessionId !== sessionId) {
          const previous = hold.lease; hold.lease = null;
          await previous?.release().catch(() => {});
          const releaseFocus = await this.browser.inputCapture.hold(connection, sessionId);
          let releaseFrames;
          try { releaseFrames = await captureBackgroundRenderer(connection, target); }
          catch (error) { await releaseFocus().catch(() => {}); throw error; }
          const release = async () => { try { await releaseFrames(); } finally { await releaseFocus(); } };
          hold.lease = { connection, sessionId, release };
        }
        // A physical-focus observation may have suspended a shared display lease.
        await this.browser.inputCapture.run(connection, sessionId, async () => {});
      })().catch(() => {
        // A busy or detached renderer retries while its Tab remains registered.
      }).finally(() => { hold.pending = null; });
      await hold.pending;
    }
  }
  // A viewer stream that is not parked watches the owned window.
  watched() {
    return this.mirror.streams.size > 0 || [...this.displays.values()].some(stream => !stream.parked);
  }
  async freezeIdleShown() {
    const target = this.activeTarget;
    if (this.frozen || !this.browser || !target || this.watched() || Date.now() - this.usedAt < this.shownIdleMs) return;
    const { connection, sessionId } = await this.browser.resolvePageTarget(target);
    await connection.send('Page.setWebLifecycleState', { state: 'frozen' }, sessionId);
    this.frozen = { connection, sessionId };
  }
  // Every request but a parked state read or an unwatched park is a use: it wakes the shown Tab before it runs.
  async wakeShown(request) {
    if (request.method === 'health' || request.method === 'host.browser' && (request.params?.op === 'state_signal' || request.params?.op === 'display_park' && request.params.parked === true)) return;
    this.usedAt = Date.now();
    await this.freezing;
    const frozen = this.frozen;
    if (!frozen) return;
    this.frozen = null;
    await frozen.connection.send('Page.setWebLifecycleState', { state: 'active' }, frozen.sessionId).catch(() => {});
  }
  async releaseBackground(targetId) {
    const hold = this.backgroundHolds.get(targetId);
    if (!hold) return;
    this.backgroundHolds.delete(targetId);
    await hold.pending;
    await hold.lease?.release().catch(() => {});
  }
  async requestNow(command, { signal, inputGuard, nativeDisplay } = {}) {
    const scope = command.observed_by ?? (command.focused_agent ? "focused-agent" : "adapter");
    assertNotCancelled(signal);
    if (command.op.startsWith("mirror_") && !featureOn("CHARIOX_KERNEL_BROWSER_MIRROR")) throw new Error("MP-08: DOM mirror disabled");
    if (command.op.startsWith("display_") && !featureOn("CHARIOX_KERNEL_BROWSER_DISPLAY")) throw new Error("MD-DISPLAY: display disabled");
    if (command.op === "stop") return this.stop();
    if (command.op === "configureWebBotAuth") {
      if (typeof command.tab_id !== "string" || !command.tab_id) throw new Error("invalid Web Bot Auth tab id");
      await this.start({ signal });
      const tab = this.tabs.get(command.tab_id);
      const result = await this.browser.configureWebBotAuth(command.tab_id, command.signer, tab?.target_id);
      if (disablesWebBotAuth(command.signer)) this.webBotAuthProjects.delete(command.tab_id);
      else if (this.browser.webBotAuth?.byTab.has(command.tab_id)) this.webBotAuthProjects.set(command.tab_id, command.signer.project_id);
      await this.resumeWebBotAuthRestore(command.tab_id, { allowUnsigned: disablesWebBotAuth(command.signer) });
      await this.save();
      return result;
    }
    // MP-11: observation never launches/relaunches the browser.
    await this.start({ signal, allowStart: !["cancel_download", "dialog", "state", "snapshot", "screenshot", "save", "wait", "unsubscribe", "display_subscribe", "display_attach", "display_suspend", "display_resume", "display_park", "display_input_target", "resize", "note_selection", "pointer_cursor", "mirror_subscribe", "mirror_next", "mirror_close", "chrome_window_follow", "popup_region"].includes(command.op) });
    assertNotCancelled(signal);
    const browserAtStart = this.browser;
    if (this.protection.unknown) throw new Error("MD-5: observation registry unavailable");
    // A modal blocks renderer evaluation. Read and answer from the controller's
    // CDP event state, never from a reconciliation that waits on that renderer.
    if (command.op === "cancel_download") {
      const tab = this.tabs.get(command.tab_id);
      if (!tab || command.generation !== this.generation) throw new UserDomainRefusal("stale_reference");
      await this.downloads.cancel(command.download_id, tab.tab_id);
      return this.observe(this.tabState(), null, scope);
    }
    if (command.op === "dialog") {
      const tab = this.tabs.get(command.tab_id);
      if (command.generation !== this.generation || !tab || tab.document_id !== command.document_id ||
          typeof command.dialog_id !== "string" || !this.browser.dialogDefaults?.current(tab.target_id, command.dialog_id, command.document_id)) {
        throw new BrowserActionError("browser_dialog_invalid", "browser dialog is no longer pending; read the current browser state");
      }
      this.appForInput(tab, command, true);
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
    if (command.op === "popup_region") return resolveCardRegion(this, command, scope, signal);
    // Protocol 556: the signal count is taken before the read, so a change during it wakes the next wait.
    const stateSignal = command.state_wait === true ? this.stateSignal.count : null;
    // Protocol 540: a viewer's state read right after an address-bar key sees the list Chrome opened.
    if (["start", "state"].includes(command.op)) {
      const state = this.observe(await (command.op === "state" ? this.readState({ signal }) : this.reconcile()), null, scope);
      return command.state_wait === true ? { ...state, _state_signal: stateSignal } : state;
    }
    const agentRequest = command.focused_agent || command._agent_input;
    if (agentRequest && ["open", "navigate"].includes(command.op) && extensionManagementUrl(command.url)) throw new Error(EXTENSIONS_REFUSED);
    // Only the person changes the visible Tab: an agent's new Tab opens behind it.
    if (command.op === "open") {
      const opened = await this.open(command.url, command.tab_id, { signal, downloadSession: command._download_session, background: Boolean(agentRequest) });
      await this.rebindWindows();
      const tab = agentRequest ? this.tabs.get(opened.tab_id) : null;
      if (tab) await this.holdBackground(tab).catch(() => {});
      return this.observe(opened, null, scope);
    }
    for (const stream of this.displays.values()) if (Date.now() > stream.expires) await this.removeDisplay(stream);
    if(['display_suspend','display_resume'].includes(command.op)){
      if(command.focused_agent||command._agent_input)throw new UserDomainRefusal('not_granted');
      const binding=stream=>this.displayResumeBinding(stream,signal);
      return command.op==='display_resume'
        ?this.displaySessions.resume(command.subscription_id,scope,command.generation,binding)
        :this.displaySessions.suspend(command.subscription_id,scope,command.generation,binding);
    }
    // P1 (protocol 720): the viewer parks its Tab display while the DOM mirror paints; a commit unparks it.
    if(command.op==='display_park'){
      if(command.focused_agent||command._agent_input)throw new UserDomainRefusal('not_granted');
      const stream=this.displaySessions.require(command.subscription_id,scope,command.generation);
      if(typeof command.parked!=='boolean')throw new UserDomainRefusal('stale_reference','display_retired');
      // The kernel pump parks an unwatched display (`watch`): no capture, source paused, and each park renews the host
      // binding's 60 s expiry. A viewer's own park (DOM mirror painting) is the Tab's next commit to clear.
      if(stream.source_kind==='desktop')await this.desktopDisplay.park(stream,command.parked);
      else stream.park(command.parked,command.watch===true);
      stream.lastCreditAt=Date.now();stream.expires=Date.now()+60_000;this.armDisplayExpiry(stream);
      return {generation:this.generation,parked:Boolean(stream.parked)};
    }
    // MP-08/MP-10/MP-11: allocated536 default fallback never resizes the owned desktop.
    // The visible-page adapter attests H.264; an offer without it (no decoder, VP8/VP9 only) takes the CDP display below.
    if(command.op==='display_subscribe'&&!command.target&&command.codecs?.includes?.('avc1.420033')&&this.chromium.desktop?.binding?.()){
      try{return this.initialPush(command,await subscribeVisiblePage(this,command,scope,signal));}
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
    if (command.op === "unsubscribe") throw new UserDomainRefusal("stale_reference", "display_retired");
    // Explicit physical input already names an observed tab/document. Read
    // that live target directly; whole-tab discovery still runs afterward.
    // Per-dispatch document/cancellation and secret-focus checks remain below.
    // MP-08/MP-10: a navigation or a selection note reads only its own Tab's live document: a full
    // all-tab reconcile waits for busy (loading) pages and held the mutation lane.
    // The wait re-reads its Tab on every look, so it follows a navigation instead of binding to one document.
    if (command.op === "wait") {
      if (!agentRequest) throw new UserDomainRefusal("not_granted");
      return waitForPage(this, command, scope, signal);
    }
    let tab = command.op === 'input' && typeof command.document_id === 'string' || ['navigate', 'resize', 'note_selection', 'pointer_cursor', 'save'].includes(command.op)
      ? await slowMark(this.timing, 'input_wait display_target', this.displayTarget(command,nativeInputDocumentTarget(this,command,scope,nativeDisplay),INPUT_DOCUMENT_WAIT_MS))
      : command.op === 'show_tab' ? this.ownedTab(command) : await this.target(command);
    assertNotCancelled(signal);
    if (agentRequest && ["input", "navigate"].includes(command.op) && extensionManagementUrl(tab.url)) throw new Error(EXTENSIONS_REFUSED);
    // An agent's Tab runs at full speed behind the shown one; the person's own input to it goes first.
    if (agentRequest) await this.holdBackground(tab).catch(() => {});
    if (agentRequest && command.op === "input" && !command._person_idle_checked) await this.personQuiet(tab.tab_id, signal);
    const app = this.appForInput(tab, command, ["input", "navigate", "close", "resize"].includes(command.op));
    // The page stays on its origin.
    if (app && command.op === "navigate" && new URL(navigationUrl(command.url)).origin !== app.origin) {
      throw new UserDomainRefusal("not_granted");
    }
    if(command.op==='resize')return resizeBrowserPage(this,tab,command,signal);
    if (["navigate", "input"].includes(command.op)) this.downloads.bind(tab, command._download_session);
    const binding = { target_id: tab.target_id, document_id: tab.document_id };
    if (command.op === "display_subscribe") return this.initialPush(command, await reserveDisplay(this,async()=>{
      if (!featureOn("CHARIOX_KERNEL_BROWSER_DISPLAY")) throw new Error("MD-DISPLAY: display disabled");
      if (!Array.isArray(command.codecs) || !command.codecs.includes("png") || command.codecs.length > 8 ||
        !Number.isInteger(command.bitrate) || command.bitrate < 500_000 || command.bitrate > 64_000_000 ||
        ![1, 2].includes(command.device_scale_factor) || this.displays.size >= maxDisplays) throw new Error("MD-DISPLAY: invalid display negotiation");
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
      this.personInputAt.delete(tab.tab_id);
      this.releaseBackground(tab.target_id);
      await this.browser.manageTab({ ...binding, action: "close" }, { signal });
      await this.browser.clearWebBotAuth?.(tab.tab_id);
      this.webBotAuthProjects.delete(tab.tab_id);
      this.webBotAuthRestoreUrls.delete(tab.tab_id);
      for (const observed of this.observedDocuments.values()) observed.delete(tab.tab_id);
      for (const references of this.cardReferences.values()) references.delete(tab.tab_id);
      for (const [id, anchor] of this.cardAnchors) if (anchor.tab_id === tab.tab_id) forgetCardAnchor(this, id);
      const state = await this.reconcile();
      await this.rebindWindows();
      return state;
    }
    // Protocol 986: a person's "Show tab" (a card's button, Follow agent) makes Chrome itself show the Tab. An agent never may.
    if (command.op === "show_tab") {
      if (agentRequest) throw new UserDomainRefusal("not_granted");
      await this.retireNativeForeground(tab.target_id);
      await this.tabGroups?.personSelection?.();
      try { await (await this.browser.ensureConnection()).send("Target.activateTarget", { targetId: tab.target_id }); }
      catch (error) { throw /no target/i.test(error?.message) ? new UserDomainRefusal("stale_reference", "display_retired") : error; }
      this.pendingActive = null;
      // The answer names the Tab Chrome now shows: wait (bounded) until its own report agrees.
      let confirmed = false;
      for (const end = performance.now() + (this.tabGroups ? SHOW_TAB_WAIT_MS : 0); performance.now() < end;) {
        if (confirmed = await this.nativeFront(tab).catch(() => true)) break;
        await delay(20);
      }
      // Chrome's own report is the active identity, whether or not the page read below finishes; a reconcile begun earlier cannot undo it.
      if (confirmed) { this.activeTarget = tab.target_id; this.observed = ++this.observations; }
      return this.observe(await withinMs(this.reconcile(), SHOW_TAB_OBSERVE_MS) ?? this.tabState(), null, scope);
    }
    if (command.op === "navigate") {
      if (app) this.browser.appTabs.noteActivity(tab.target_id);
      // MP-08/MP-10/MP-11: a human navigation selects this owned desktop tab.
      // Retire native claims before physical focus can show a different page.
      if(this.chromium.desktop&&!command._agent_input&&!command.focused_agent){
        this.foreground.reset();await this.closeCompositors();
        await this.browser.manageTab({...binding,action:'activate'},{signal});
      }
      const navigated = await this.browser.navigate({ ...binding, url: navigationUrl(command.url) }, { signal });
      this.webBotAuthRestoreUrls.delete(tab.tab_id);
      // MP-08/MP-10: answer at commit, like a browser's address bar; the page loads
      // into attached views and later state reads reconcile its title.
      if (navigated?.download_started) return this.observe(this.tabState(), null, scope);
      const stored = this.tabs.get(tab.tab_id);
      if (!stored || typeof navigated?.document_id !== 'string' || !navigated.document_id) return this.observe(await this.reconcile(), null, scope);
      // A redirect commits another address: read the committed target, fenced to this document.
      const committed = await this.displayTarget({ generation: this.generation, tab_id: tab.tab_id });
      if (committed.document_id !== navigated.document_id) return this.observe(await this.reconcile(), null, scope);
      this.observed = ++this.observations; // an older in-flight reconcile cannot restore the previous document
      return this.observe(this.tabState(), null, scope);
    }
    if (command.op === "note_selection") return this.observe({ generation:this.generation, observation:await this.browser.observeNote({ ...binding, ...(command.quote ? {quote:command.quote} : {}) }) },tab,scope);
    if (command.op === "pointer_cursor") {
      const page = this.pageSize(tab.tab_id);
      if (!Number.isSafeInteger(command.x) || !Number.isSafeInteger(command.y) || command.x < 0 || command.y < 0 || command.x >= page.width || command.y >= page.height) throw new Error("MP-08: pointer cursor point outside the page");
      return { generation: this.generation, cursor: await this.browser.pointerCursor({ ...binding, x: command.x, y: command.y }) };
    }
    if (command.op === "save") {
      if (!agentRequest) throw new UserDomainRefusal("not_granted");
      return this.observe(await saveResource(this, tab, command, signal), tab, scope);
    }
    if (command.op === "snapshot") {
      const snapshot = await this.browser.snapshot(binding);
      rememberCardReferences(this, scope, tab, snapshot);
      return this.observe({ generation: this.generation, snapshot }, tab, scope);
    }
    if (command.op === "input") {
      if (app) this.browser.appTabs.noteActivity(tab.target_id);
      const observed = command.document_id ?? (command.focused_agent ? null : this.observedDocuments.get(scope)?.get(tab.tab_id));
      if (!observed || observed !== tab.document_id) throw new UserDomainRefusal("stale_reference");
      const at = timestamp();
      let dispatched = false, uploadResult;
      // MD-DISPLAY-02/04: input retires exact verification through its epoch.
      // MP-08/MP-10: all physical input keeps bounded video dependencies.
      // Retiring an encoded row on each key forces an IDR and can delay echo
      // behind its byte budget. Epochs still retire exact work; policy/document
      // retirement still closes the source/producer and resets every row.
      // Review #1072 P2: the Tab inventory is taken once, at dispatch. Reading it again after the
      // acknowledgement would count a popup adopted meanwhile as known and give the opener its focus back.
      let selection = null;
      const rememberSelection = () => {
        // MP-08/MP-10/MP-11: CDP can type into a background window. Its
        // admitted human selection supersedes older native restore intent.
        if (agentRequest || command._agent_input || command.focused_agent) return;
        // A newer observation that adopted a popup retired this intent for good.
        if (selection && (this.pendingActive === selection || this.adoptedByInput.has(selection))) return;
        const intent = this.pendingActive, origin = appRestoreOrigin(tab.url);
        // The cached target can still match while a late native window fold
        // changed Chrome's selection. Reconcile checks the live native target
        // before activating, so ordinary keys do not refocus an active tab.
        selection = {tab_id:tab.tab_id,app_origin:origin,apps:intent?.apps ?? [],until:intent?.until ?? Date.now(),known:selection?.known ?? new Set([...this.tabs.values()].map(known => known.target_id))};
        if (!agentRequest) this.pendingActive = selection;
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
          const dispatch = async (inputTimestamp, inputToken) => {
            if (command.input.kind !== "scroll") await this.tabGroups?.openingInput?.(tab.target_id, Boolean(agentRequest));
            return this.popupEvidence.capture(this.browser, tab, popupActionId(command), beginDispatch => inputHostTab(this.browser, tab, command.input, { uploadRoots: command._upload_roots, signal, inputTimestamp, inputToken, inputGuard, documentCurrent, chromeFocused: () => Boolean(this.arbiter?.chromeFocus), slow: (name, operation) => slowMark(this.timing, `input_wait ${name}`, operation), onDispatch: () => { onDispatch(); return beginDispatch(); }, viewport:{css_width:this.pageSize(tab.tab_id).width,css_height:this.pageSize(tab.tab_id).height}, asyncScroll: ()=>viewerActive()&&(handoverOrdered||owned?.attested&&typeof owned.valid==='function'&&owned.valid()), nativeWheel, nativeClick, nativeKey, resolveMirror: input => this.mirror.resolveInput(tab,input,scope,signal) }));
          };
          const result = app && agentRequest
            ? await this.browser.appTabs.input(app, scope.startsWith('agent:') ? scope.slice(6) : null, dispatch)
            : await dispatch();
          if (command.input.kind === 'upload') uploadResult = result;
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
        // Late CDP acknowledgements and definitive text refusals leave nothing held; neither needs a browser reset.
        if ((dispatched && !lateInputAck(error) && !(error instanceof TextInsertionRefused) || ["browser_action_cancelled", "stale_document_reference"].includes(error?.code)) && this.isBrowserLive(browserAtStart)) {
          // Clear any dispatched key/button state before another actor can use
          // the browser. Recovery rotates generation; cancelled input never replays. A reset is a loss the
          // browser recovers from with its Tabs: only the owner's stop keeps it down.
          lifecycleLog(`browser_reset after ${hostErrorLabel(error)}`);
          await this.stop({ recovering: true });
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
      if (uploadResult) result.upload = { file_count: uploadResult.file_count, total_bytes: uploadResult.total_bytes };
      return result;
    }
    if (command.op === "screenshot") {
      if (REGION_KEYS.some(key => command[key] !== undefined)) {
        if (!agentRequest) throw new UserDomainRefusal("not_granted");
        return this.observe(await regionScreenshot(this, tab, command), tab, scope);
      }
      // A frame laid out at the viewer's size (protocol 560/168) is the whole page; any other size is a crop of it.
      const hint=command._capture_viewport,scale=this.scales.get(tab.tab_id)??this.viewport().device_scale_factor,page=this.layouts.get(tab.tab_id)??geometry;
      const crop=hint&&(hint.width!==page.width*scale||hint.height!==page.height*scale);
      const frame = command._capture_protection===true||command._agent_input===true?(crop?await captureVisiblePage(this,tab,hint,scope):await this.screenshot(tab,null,true)):await this.displayScreenshot(tab);
      // MD-3: explicit binding is an internal display/MCP seam. Legacy 417
      // still emits its existing frame shape until the coordinator adapter lands.
      if (!command.focused_agent && !command.bound_frames) delete frame.document_id;
      return this.observe(frame, tab, scope);
    }
    throw new Error("MD-2: unsupported browser operation");
  }
  // MP-08/MP-11: an agent drives an App Tab as itself, only after the kernel
  // claimed that App view in the agent's own grant.
  appForInput(tab, command, drives) {
    const app = [...(this.browser.appTabs?.apps?.values() ?? [])].find(open => open.targetId === tab.target_id);
    if (app && drives && (command.focused_agent || command._agent_input) && command._app_view_tab !== tab.tab_id) throw new UserDomainRefusal("not_granted");
    return app;
  }
  // VAUDIT S3: reveal and read in one call; the value goes to the kernel only.
  async captureSecret(params, { signal }) {
    await this.start({ signal });
    if (this.protection.unknown) throw new Error("MD-5: observation registry unavailable");
    const tab = await this.target(params);
    if (tab.document_id !== params.document_id) throw new UserDomainRefusal("stale_reference");
    const app = this.appForInput(tab, params, true);
    if (app) this.browser.appTabs.noteActivity(tab.target_id);
    return this.sampleLane(tab).run("input", () => this.browser.captureValue({ target_id: tab.target_id, document_id: tab.document_id,
      node_ref: params.node_ref, reveal_node_ref: params.reveal_node_ref ?? null, read: params.read ?? "node",
      expected_document_url: params.expected_document_url }, { signal }));
  }
  async handle(request, { signal } = {}) {
    if (request.method === "host.downloads.cleanup") {
      if (!this.browser) await this.downloads.close();
      else await this.downloads.cleanupSession(request.params.session_id);
      return { id: request.id, ok: true, result: {} };
    }
    const browserAtEntry = this.browser;
    try { assertNotCancelled(signal); } catch { return { id: request.id, ok: false, error: { code: "browser_action_cancelled", message: "MD-3: browser action cancelled" } }; }
    try {
      await this.wakeShown(request);
      if (request.method === "health") return { id: request.id, ok: true, result: { state: "ready", process_id: process.pid, diagnostic_code: null } };
      if (request.method === "shutdown") return { id: request.id, ok: true, result: await this.stop() };
      if (request.method === "host.protect") return { id: request.id, ok: true, result: await this.protect(request.params) };
      if (request.method === "host.revoke_subscriptions") {
        const owners = new Set(request.params.subscription_owners ?? []);
        for(const owner of owners){await this.nativeComputer.retire(owner);for(const observer of this.nativeAccessibility.observers.keys())if(observer.startsWith(owner+':'))this.nativeAccessibility.retire(observer);}
        return { id: request.id, ok: true, result: { revoked: true } };
      }
      if (request.method === "host.web_bot_auth_policy") {
        const params = request.params ?? {};
        this.browser?.webBotAuth?.applyProjectPolicy(params.projects, params.reset, params.revision ?? 0);
        if (!params.reset) {
          for (const [tabId, projectId] of this.webBotAuthProjects) {
            if (typeof projectId === "string" && params.projects.includes(projectId)) continue;
            this.webBotAuthProjects.delete(tabId);
            await this.resumeWebBotAuthRestore(tabId, { allowUnsigned: true });
          }
        }
        if (this.browser) await this.save();
        return { id: request.id, ok: true, result: { updated: true } };
      }
      if (request.method === "host.secret" && request.params.op === "capture") {
        return { id: request.id, ok: true, result: { value: await this.captureSecret(request.params, { signal }) } };
      }
      if (request.method === "host.secret") {
        const { _web_bot_auth: webBotAuth, ...params } = request.params ?? {};
        // MP-08 / MP-10 / MP-11: this internal owner-only flag does not
        // loosen ordinary Vault secret insertion into password fields.
        const codeInput = params.mask_code_input === true;
        const factorInput = params.vault_factor_input === true;
        if (codeInput && (!String(params.observed_by ?? "").startsWith("terminal:") ||
            params.action?.kind !== "fill" || typeof params.action?.expected_document_url !== "string")) {
          throw new UserDomainRefusal("not_granted");
        }
        if (factorInput && (request.params.action?.kind !== "fill" || typeof request.params.action?.expected_document_url !== "string")) {
          throw new UserDomainRefusal("not_granted");
        }
        await this.start({ signal });
        if (this.protection.unknown) throw new Error("MD-5: observation registry unavailable");
        const tab = await this.target(params);
        if (tab.document_id !== params.document_id) throw new UserDomainRefusal("stale_reference");
        if (webBotAuth !== undefined) {
          await this.browser.configureWebBotAuth(params.tab_id, webBotAuth, tab.target_id);
          if (disablesWebBotAuth(webBotAuth)) this.webBotAuthProjects.delete(params.tab_id);
          else if (this.browser.webBotAuth?.byTab.has(params.tab_id)) this.webBotAuthProjects.set(params.tab_id, webBotAuth.project_id);
          await this.resumeWebBotAuthRestore(params.tab_id, { allowUnsigned: disablesWebBotAuth(webBotAuth) });
        }
        // MP-08/MP-11: a Vault fill is the agent's input to an App Tab too,
        // recorded before the field's input/change handlers can run.
        const app = this.appForInput(tab, params, true);
        if (app) this.browser.appTabs.noteActivity(tab.target_id);
        const action = { ...params.action, mask_code_input: codeInput, vault_factor_input: factorInput, plain_target: params.plain_target === true };
        const performed = await this.sampleLane(tab).run("input", () => this.browser.performAction({ target_id: tab.target_id, document_id: tab.document_id,
          node_ref: params.node_ref, action, timeout_ms: 10_000 }, { signal }));
        await this.save();
        return { id: request.id, ok: true, result: { inserted: true, ...(performed?.submitted ? { submitted: performed.submitted } : {}) } };
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
        this.cardReferences.delete(request.params.observer);
        for (const [id, anchor] of this.cardAnchors) if (anchor.scope === request.params.observer) forgetCardAnchor(this, id);
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
        const { _tab_groups: groups, _web_bot_auth: webBotAuth, ...params } = request.params ?? {};
        if (webBotAuth !== undefined) {
          await this.start({ signal });
          if (params.op === "open") params.tab_id ??= `host-tab-${randomUUID()}`;
          if (typeof params.tab_id === "string") {
            const tab = this.tabs.get(params.tab_id);
            await this.browser.configureWebBotAuth(params.tab_id, webBotAuth, tab?.target_id);
            if (disablesWebBotAuth(webBotAuth)) this.webBotAuthProjects.delete(params.tab_id);
            else if (this.browser.webBotAuth?.byTab.has(params.tab_id)) this.webBotAuthProjects.set(params.tab_id, webBotAuth.project_id);
            if (params.op !== "navigate") {
              await this.resumeWebBotAuthRestore(params.tab_id, { allowUnsigned: disablesWebBotAuth(webBotAuth) });
            }
          }
        }
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
        if (request.params?.op === "snapshot") {
          return { id: request.id, ok: true, result: { ...result, snapshot: redactSnapshot(result.snapshot, this.protection.values) } };
        }
        if (Array.isArray(result?.tabs) && Number.isSafeInteger(result.generation)
            || ["pointer_cursor", "resize"].includes(request.params?.op)) {
          return { id: request.id, ok: true, result: redactBrowserState(result, this.protection.values) };
        }
        // Card regions contain only the fixed protocol, observed origin and
        // verification receipt. None of these fields are page text.
        if (request.params?.op === 'popup_region') return { id: request.id, ok: true, result };
        if (!["screenshot", "mirror_next", "display_subscribe", "display_input_target"].includes(request.params?.op)) {
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
            await this.rebindWindows();
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
      if (error instanceof AgentToolRefusal) return {id:request.id,ok:false,error:{code:error.code,message:error.message}};
      const uploadFailure = request.params?.input?.kind === 'upload' ? publicUploadFailure(error) : null;
      if (uploadFailure) return { id: request.id, ok: false, error: uploadFailure };
      if (isTrustedStaleReferenceError(error)) return {id:request.id,ok:false,error:{code:"user_domain_stale_reference",message:"User-domain request refused"}};
      // A cancelled request that may have dispatched input stops the browser (no held keys/buttons). A
      // cancelled read-only observation (a viewer's credit on a huge page) must not stop every Tab (MP-08/MP-10).
      if (error?.code === "browser_action_cancelled" && request.method !== "host.computer" && !request.params?.display_subscription_id
        && this.browser === browserAtEntry && !(request.method === "host.browser" && OBSERVATION_OPS.has(request.params?.op))) { lifecycleLog('browser_reset after cancelled request'); await this.stop({ recovering: true }); }
      if (["browser_unavailable"].includes(error?.code)) {
        return { id: request.id, ok: false, error: { code: error.code, message: error.message } };
      }
      if (error?.code === "browser_dialog_invalid") return { id: request.id, ok: false, error: {
        code: "browser_dialog_invalid", message: "Browser dialog answer is invalid or no longer pending; read the current browser state." } };
      return { id: request.id, ok: false, error: { code: error?.code === "browser_action_cancelled" ? "browser_action_cancelled" : "kernel_browser_failed", message: error instanceof MirrorInputEpochRefusal ? "MP-11: stale mirror input epoch" : error instanceof PointerOutsideViewport ? error.message : hostFailureMessage(error) } }; }
  }
}

if (process.argv[2] === "stdio" && process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const host = new KernelBrowserHost(process.argv[3], { appStorageRoot: process.argv[4] });
  let closing = false;
  const stop = async () => { if (closing) return; closing = true; await host.stop(); process.exit(0); };
  process.on("SIGTERM", stop);
  process.on("SIGINT", stop);
  await new BrowserControllerStdioServer(timedHostRequests(host, request => scheduleHostRequest(request, host.browser, host.viewport(), Boolean(host.chromium.desktop?.binding?.())))).run().finally(() => host.stop());
  // The kernel waits for this process after "shutdown": a leftover helper handle must not keep it alive.
  process.exit(0);
}
