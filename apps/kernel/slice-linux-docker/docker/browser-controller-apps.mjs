// App views: one tab per installation on its own https origin. The controller
// serves the kernel-verified UI assets through CDP Fetch interception, so no
// port is opened and the page is a secure context. Every other request from
// the tab is blocked at the browser boundary, and popups it opens are closed.
// `window.chariox.call` is a CDP binding; the kernel answers each call.
// Each call carries the document (top-level loader) that made it: when the Tab
// reloads, navigates or closes, the kernel cancels that document's calls and
// an answer for it is never delivered to the next one.
import { BrowserControllerError } from "./browser-controller-cdp.mjs";
import { placeholderOrigin } from "./browser-app-restore.mjs";

// Each installation is a separate registrable domain. A shared parent such as
// app.chariox.internal lets one App set Domain cookies readable by another.
export const APP_ORIGIN_SUFFIX = ".invalid";
const LEGACY_APP_ORIGIN_SUFFIX = ".app.chariox.internal";
const APP_HOST = /^app\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.invalid$/;
const MAX_ASSETS = 256;
const MAX_ASSET_BYTES = 8 * 1024 * 1024;
const MAX_PENDING_CALLS = 64;
// The bounded host text plus its JSON envelope must fit a view call.
const MAX_CALL_BYTES = 512 * 1024;
const MAX_ANSWERABLE_CALLS = 1024;
const FULLSCREEN_CHECK_INTERVAL_MS = 250;
const CALL_WAIT_MS = 250;
const BINDING = "__charioxAppCall";
export const APP_CSP = [
  "default-src 'self'", "script-src 'self'", "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: blob:", "font-src 'self'", "connect-src 'none'", "frame-src 'none'",
  "child-src 'none'", "worker-src 'none'", "object-src 'none'", "form-action 'none'",
  "base-uri 'none'", "frame-ancestors 'none'",
  // No allow-popups/top-navigation/downloads/modals: a popup or new window
  // would be a new, unintercepted target that can reach the network.
  "sandbox allow-scripts allow-same-origin allow-forms",
].join("; ");

// Powerful features an App page must never reach. Screen capture would let
// the page ask for (and list) the Room's other Tabs; fullscreen could cover
// the private panel; devices, sensors and credentials are not App capabilities.
// The kernel bridge is the App's only way out of its page.
export const APP_PERMISSIONS_POLICY = [
  "display-capture", "fullscreen", "camera", "microphone", "geolocation", "clipboard-read",
  "usb", "serial", "hid", "bluetooth", "midi", "payment", "publickey-credentials-get",
  "publickey-credentials-create", "identity-credentials-get", "otp-credentials",
  "screen-wake-lock", "idle-detection", "local-fonts", "window-management",
  "accelerometer", "gyroscope", "magnetometer", "ambient-light-sensor",
  "xr-spatial-tracking", "storage-access", "browsing-topics", "attribution-reporting",
].map((feature) => `${feature}=()`).join(", ");

// Installed before any App script runs. The binding carries only JSON strings;
// the kernel binds every call to this tab's installation, never to page data.
const BRIDGE_SOURCE = `(() => {
  // WebRTC is outside CSP and request interception; remove it before App code.
  for (const name of ["RTCPeerConnection", "webkitRTCPeerConnection", "RTCDataChannel",
    "RTCSessionDescription", "RTCIceCandidate"]) {
    try { Object.defineProperty(globalThis, name, { value: undefined, writable: false, configurable: false }) } catch {}
  }
  const call = globalThis.${BINDING};
  if (typeof call !== "function") return;
  delete globalThis.${BINDING};
  const pending = new Map();
  // Ids are unique per document: a reopened Tab loads a new one, and an
  // answer still in flight for the old document must not match a new call.
  const documentNonce = crypto.randomUUID();
  let next = 0;
  const request = (method, params) => new Promise((resolve, reject) => {
    const id = documentNonce + ":" + ++next;
    pending.set(id, { resolve, reject });
    call(JSON.stringify({ id, method, params }));
  });
  Object.defineProperty(globalThis, "__charioxAppResolve", { value(id, ok, value) {
    const entry = pending.get(id);
    if (!entry) return;
    pending.delete(id);
    ok ? entry.resolve(value) : entry.reject(Object.assign(new Error(value?.message ?? "App call failed"), { code: value?.code }));
  } });
  Object.defineProperty(globalThis, "chariox", { value: Object.freeze({
    call(method, params = {}) { return request(method, params); },
    host: Object.freeze({
      openLink(url) { return request("host.open_link", { url }); },
      writeClipboard(text) { return request("host.clipboard_write", { text }); },
    }),
    // Where Chariox draws the private agent panel beside this page: set
    // {placement: "right" | "bottom" | "none", size?} or get the current
    // layout. The user's own choice wins; the page never sees the panel.
    panel: Object.freeze({
      set(layout) { return request("chariox.panel", layout ?? {}); },
      get() { return request("chariox.panel", {}); },
    }),
  }) });
})();`;

function invalid(message) {
  return new BrowserControllerError("invalid_request", message);
}

export function appOrigin(label) {
  if (typeof label !== "string" || !/^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(label)) {
    throw invalid("App origin label must be a lowercase DNS label");
  }
  return `https://app.${label}${APP_ORIGIN_SUFFIX}`;
}

function assets(raw, entry) {
  if (!Array.isArray(raw) || raw.length === 0 || raw.length > MAX_ASSETS) throw invalid("invalid App assets");
  let total = 0;
  const map = new Map();
  for (const asset of raw) {
    if (typeof asset?.path !== "string" || !/^[A-Za-z0-9._/-]{1,512}$/.test(asset.path)
      || asset.path.split("/").some((part) => part === "" || part === "." || part === "..")
      || typeof asset.content_type !== "string" || typeof asset.body_base64 !== "string") {
      throw invalid("invalid App asset");
    }
    total += Buffer.byteLength(asset.body_base64, "base64");
    if (total > MAX_ASSET_BYTES) throw invalid("App assets exceed their size limit");
    map.set(asset.path, { contentType: asset.content_type, body: asset.body_base64 });
  }
  if (!map.has(entry)) throw invalid("App entry is not an asset");
  return map;
}

export class AppTabs {
  constructor(browser, { restoreGraceMs = 30_000 } = {}) {
    this.browser = browser;
    // Every new CDP connection sweeps App Tabs, even when no App command
    // arrives: interception ends with the old connection.
    browser.onConnected = (connection) => this.reconcile(connection).catch(() => {});
    this.apps = new Map(); // sessionId -> app
    this.calls = [];
    this.callWaiters = new Set();
    this.connection = null;
    this.unsubscribe = null;
    this.fullscreenMaintenance = null;
    this.nextFullscreenCheck = 0;
    this.restoreGraceMs = restoreGraceMs;
    this.orphanRestores = new Map();
  }

  async open(params) {
    const origin = appOrigin(params?.origin_label);
    if (typeof params?.installation_id !== "string" || !params.installation_id) throw invalid("missing installation");
    if (params.instance_id != null && (typeof params.instance_id !== "string"
      || !/^[a-zA-Z0-9_-]{1,128}$/.test(params.instance_id))) throw invalid("invalid App instance");
    const instanceId = params.instance_id ?? null;
    const entry = params.entry ?? "index.html";
    const app = { installation: params.installation_id, instanceId, origin, entry, assets: assets(params.assets, entry), page: params.page ?? null };
    const connection = await this.browser.ensureConnection();
    this.listen(connection);
    // One shared App Tab per Room and installation: opening it again (another
    // terminal, or a kernel that restarted) shows that Tab with the current
    // assets instead of a second one. User-domain instance keys keep distinct tabs.
    const shown = [...this.apps.entries()].find(([, open]) => open.installation === app.installation && open.origin === origin && open.instanceId === instanceId);
    if (shown) {
      const [sessionId, open] = shown;
      Object.assign(open, { entry, assets: app.assets, page: app.page });
      const metrics = this.browser.appMetrics?.(open.page);
      if (metrics) await connection.send("Emulation.setDeviceMetricsOverride", metrics, sessionId).catch(() => {});
      await connection.send("Target.activateTarget", { targetId: open.targetId });
      await this.fullscreen(connection, open.targetId).catch(() => false);
      // Navigate (not reload): it returns once the document commits, so the
      // Room projects the Tab's URL rather than the empty one of a reload.
      await connection.send("Page.navigate", { url: `${origin}/` }, sessionId);
      return { target_id: open.targetId, origin };
    }
    // Each App view gets its own fullscreen window, so page and desktop
    // coordinates agree. Its page lays out left of the trusted conversation
    // panel, which the terminal draws over the rest of the desktop.
    // A restored placeholder has no App script or authority. Only an Open
    // with kernel-verified assets can adopt it, matching the exact origin.
    const { targetInfos = [] } = await connection.send("Target.getTargets", {});
    const restored = instanceId === null
      ? targetInfos.find(target => target.type === "page" && placeholderOrigin(target.url) === origin)
      : undefined;
    const { targetId } = restored ?? await connection.send("Target.createTarget", { url: "about:blank", newWindow: true });
    clearTimeout(this.orphanRestores.get(targetId)?.timer);
    this.orphanRestores.delete(targetId);
    const sessionId = await this.browser.ensureTargetSession(connection, targetId);
    this.apps.set(sessionId, { ...app, targetId, document: null, pending: new Set() });
    try {
      if (restored) await connection.send("Target.activateTarget", { targetId });
      await this.fullscreen(connection, targetId);
      const metrics = this.browser.appMetrics?.(app.page);
      if (metrics) await connection.send("Emulation.setDeviceMetricsOverride", metrics, sessionId);
      await connection.send("Fetch.enable", { patterns: [{ urlPattern: "*", requestStage: "Request" }] }, sessionId);
      await connection.send("Runtime.addBinding", { name: BINDING }, sessionId);
      await connection.send("Page.addScriptToEvaluateOnNewDocument", { source: BRIDGE_SOURCE }, sessionId);
      await connection.send("Page.navigate", { url: `${origin}/` }, sessionId);
    } catch (error) {
      this.apps.delete(sessionId);
      await this.browser.closePageTarget(connection, targetId).catch(() => {});
      throw error;
    }
    return { target_id: targetId, origin };
  }

  async close(params) {
    await this.reconcile();
    const entry = [...this.apps.entries()].find(([, app]) => app.targetId === params?.target_id);
    if (!entry) throw invalid("unknown App tab");
    const [sessionId, app] = entry;
    await this.browser.closePageTarget(this.connection, app.targetId);
    this.apps.delete(sessionId);
    this.calls = this.calls.filter(call => call.target_id !== app.targetId);
    this.wakeCalls();
    return { closed: true };
  }

  // An update (or a kernel restart) rebinds an open view: serve the current
  // generation's assets and reload the page. A normal reload lets the App keep
  // drafts in its own web storage; the Tab and window stay.
  async reload(params) {
    await this.reconcile();
    const entry = [...this.apps.entries()].find(([, app]) => app.targetId === params?.target_id);
    if (!entry) throw invalid("unknown App tab");
    const [sessionId, app] = entry;
    const next = params.entry ?? "index.html";
    app.assets = assets(params.assets, next);
    app.entry = next;
    // Calls the old page queued are not the new page's: its promises go with it.
    this.calls = this.calls.filter((call) => call.target_id !== app.targetId);
    await this.connection.send("Page.reload", { ignoreCache: true }, sessionId);
    return { target_id: app.targetId };
  }

  // The kernel resized the page beside its panel (moved, minimized, hidden).
  async layout(params) {
    await this.reconcile();
    const entry = [...this.apps.entries()].find(([, app]) => app.targetId === params?.target_id);
    if (!entry) throw invalid("unknown App tab");
    const [sessionId, app] = entry;
    app.page = params.page ?? null;
    const metrics = this.browser.appMetrics?.(app.page);
    if (metrics) await this.connection.send("Emulation.setDeviceMetricsOverride", metrics, sessionId);
    return { target_id: app.targetId };
  }

  listen(connection) {
    if (this.connection === connection) return;
    this.unsubscribe?.();
    for (const restore of this.orphanRestores.values()) clearTimeout(restore.timer);
    this.apps.clear();
    this.calls = [];
    this.wakeCalls();
    this.swept = false;
    this.nextFullscreenCheck = 0;
    this.connection = connection;
    this.unsubscribe = connection.subscribe((message) => {
      void this.handle(connection, message).catch(() => {});
    });
  }

  async handle(connection, message) {
    if (this.connection !== connection) return;
    if (["Target.targetCreated", "Target.targetInfoChanged"].includes(message?.method)
      && placeholderOrigin(message.params?.targetInfo?.url)) {
      // Chromium can finish restoring targets after the initial CDP sweep.
      // Sweep this event's connection: it may still be the opening one.
      this.swept = false;
      await this.reconcile(connection);
    }
    if (message?.method === "Target.targetCreated") {
      const opener = message.params?.targetInfo?.openerId;
      if (opener && [...this.apps.values()].some((app) => app.targetId === opener)) {
        await this.browser.closePageTarget(connection, message.params.targetInfo.targetId);
      }
      return;
    }
    if (message?.method === "Target.detachedFromTarget") {
      this.apps.delete(message.params?.sessionId);
      if (!this.apps.size) this.wakeCalls();
      return;
    }
    const app = this.apps.get(message?.sessionId);
    if (!app) return;
    if (message.method === "Page.frameNavigated" && !message.params?.frame?.parentId) {
      app.document = message.params?.frame?.loaderId ?? null;
      app.pending.clear();
    } else if (message.method === "Fetch.requestPaused") {
      await this.fulfill(connection, message.sessionId, app, message.params);
    } else if (message.method === "Runtime.bindingCalled" && message.params?.name === BINDING) {
      await this.enqueueCall(app, message.params.payload, message.sessionId);
    }
  }

  async fulfill(connection, sessionId, app, { requestId, request }) {
    let url;
    try { url = new URL(request?.url); } catch { url = null; }
    if (!url || url.origin !== app.origin || request.method !== "GET") {
      await connection.send("Fetch.failRequest", { requestId, errorReason: "BlockedByClient" }, sessionId);
      return;
    }
    let path = null;
    try {
      path = url.pathname === "/" ? app.entry : decodeURIComponent(url.pathname.slice(1));
    } catch {}

    const asset = path === null ? undefined : app.assets.get(path);
    const headers = [
      { name: "Content-Security-Policy", value: APP_CSP },
      { name: "Permissions-Policy", value: APP_PERMISSIONS_POLICY },
      { name: "X-Content-Type-Options", value: "nosniff" },
      { name: "Cache-Control", value: "no-store" },
      { name: "Referrer-Policy", value: "no-referrer" },
      { name: "X-DNS-Prefetch-Control", value: "off" },
    ];
    await connection.send("Fetch.fulfillRequest", asset
      ? { requestId, responseCode: 200, responseHeaders: [{ name: "Content-Type", value: asset.contentType }, ...headers], body: asset.body }
      : { requestId, responseCode: 404, responseHeaders: headers, body: "" }, sessionId);
  }

  async enqueueCall(app, payload, sessionId) {
    if (typeof payload !== "string" || payload.length > MAX_CALL_BYTES) return;
    let call;
    try { call = JSON.parse(payload); } catch { return; }
    if (typeof call?.id !== "string" || typeof call.method !== "string" || call.method.length > 128) return;
    // Resolve the document before admission; the bounded queue check and push
    // then run without an await, even when a page sends one concurrent burst.
    const documentId = app.document ?? await this.document(sessionId, app);
    // Refusing admission must settle the page's promise. Silently dropping
    // overflow leaves Promise.all backlogs waiting forever, even after drain.
    if (this.calls.length >= MAX_PENDING_CALLS) {
      await this.resolve(sessionId, call.id, false, { code: "APP_BUSY",
        message: `App view call queue is full (${MAX_PENDING_CALLS} pending calls); retry after 500 ms` });
      return;
    }
    // Calls the kernel may answer for this document; a new one clears them.
    app.pending.add(call.id);
    if (app.pending.size > MAX_ANSWERABLE_CALLS) app.pending.delete(app.pending.values().next().value);
    this.calls.push({ installation_id: app.installation, target_id: app.targetId,
      document_id: documentId, call_id: call.id,
      method: call.method, params: call.params ?? {} });
    this.wakeCalls();
  }

  // The Tab's current top-level document, when no navigation was seen yet
  // (Page events are enabled with the target's session).
  async document(sessionId, app) {
    const tree = await this.connection.send("Page.getFrameTree", {}, sessionId).catch(() => null);
    app.document ??= tree?.frameTree?.frame?.loaderId ?? null;
    return app.document;
  }

  async resolve(sessionId, callId, ok, value) {
    await this.connection.send("Runtime.evaluate", {
      expression: `globalThis.__charioxAppResolve(${JSON.stringify(callId)}, ${ok}, ${JSON.stringify(value)})`,
    }, sessionId);
  }

  // True when the App's window already is fullscreen; otherwise restores it
  // (someone pressed Esc) and reports false.
  async fullscreen(connection, targetId) {
    const { windowId } = await connection.send("Browser.getWindowForTarget", { targetId });
    const { bounds } = await connection.send("Browser.getWindowBounds", { windowId });
    if (bounds?.windowState === "fullscreen") return true;
    await connection.send("Browser.setWindowBounds", { windowId, bounds: { windowState: "fullscreen" } });
    return false;
  }

  // Window inspection is housekeeping, not part of dispatch. Awaiting it in
  // takeCalls holds both the stdio queue and the kernel controller lock: even
  // a reply for another App then waits behind every window's CDP round trips.
  // Keep one bounded pass, at the old cadence, while bridge polls can drain.
  maintainFullscreen() {
    const now = performance.now();
    if (this.fullscreenMaintenance || now < this.nextFullscreenCheck) return;
    this.nextFullscreenCheck = now + FULLSCREEN_CHECK_INTERVAL_MS;
    const connection = this.connection;
    const apps = [...this.apps.entries()];
    const pass = async () => {
      for (const [sessionId, app] of apps) {
        if (this.connection !== connection) return;
        if (this.apps.get(sessionId) !== app) continue;
        await this.fullscreen(connection, app.targetId).catch(() => false);
      }
    };
    const maintenance = pass().finally(() => {
      this.fullscreenMaintenance = null;
    });
    this.fullscreenMaintenance = maintenance;
  }

  // A lost CDP connection ends Fetch interception and the bridge for every
  // App Tab. Forget those Tabs and close any App-origin Tab this controller
  // does not own (including ones left by an earlier controller process).
  async reconcile(opened) {
    // The connection hook runs inside the browser's in-flight opening, which
    // `ensureConnection` would wait on: sweep the connection it hands over.
    const connection = opened ?? await this.browser.ensureConnection();
    this.listen(connection);
    if (this.swept) return;
    const { targetInfos = [] } = await connection.send("Target.getTargets", {});
    const present = new Set(targetInfos.map(target => target.targetId));
    for (const [targetId, restore] of this.orphanRestores) {
      if (!present.has(targetId)) {
        clearTimeout(restore.timer);
        this.orphanRestores.delete(targetId);
      }
    }
    const owned = new Set([...this.apps.values()].map((app) => app.targetId));
    for (const target of targetInfos) {
      if (!owned.has(target.targetId) && placeholderOrigin(target.url)) {
        const previous = this.orphanRestores.get(target.targetId);
        clearTimeout(previous?.timer);
        const deadline = previous?.deadline ?? Date.now() + this.restoreGraceMs;
        const timer = setTimeout(() => {
          void this.closeOrphanRestore(connection, target.targetId, target.url).catch(() => {});
        }, Math.max(0, deadline - Date.now()));
        timer.unref?.();
        this.orphanRestores.set(target.targetId, { deadline, timer });
      }
      let host = "";
      try { host = new URL(target.url).hostname; } catch {}
      if ((APP_HOST.test(host) || host.endsWith(LEGACY_APP_ORIGIN_SUFFIX)) && !owned.has(target.targetId)) {
        await this.browser.closePageTarget(connection, target.targetId).catch(() => {});
      }
    }
    this.swept = true;
  }

  async closeOrphanRestore(connection, targetId, url) {
    if (this.connection !== connection) return;
    if ([...this.apps.values()].some(app => app.targetId === targetId)) return;
    const { targetInfos = [] } = await connection.send("Target.getTargets", {});
    if (this.connection !== connection || !this.orphanRestores.has(targetId)
      || [...this.apps.values()].some(app => app.targetId === targetId)) return;
    if (targetInfos.some(target => target.targetId === targetId && target.url === url)) {
      await this.browser.closePageTarget(connection, targetId);
    }
    this.orphanRestores.delete(targetId);
  }

  /** Drain pending view calls; the kernel answers each with `respond`. */
  wakeCalls() {
    for (const wake of this.callWaiters) wake();
  }

  // Keep the existing RPC and batch shape. Enqueue wakes an outstanding drain;
  // the bounded timeout still projects closed targets and maintains windows.
  async waitForCalls() {
    if (this.calls.length || !this.apps.size) return;
    await new Promise((resolve) => {
      const wake = () => {
        clearTimeout(timer);
        this.callWaiters.delete(wake);
        resolve();
      };
      const timer = setTimeout(wake, CALL_WAIT_MS);
      this.callWaiters.add(wake);
    });
  }

  async takeCalls() {
    await this.reconcile();
    await this.waitForCalls();
    // A call whose Tab closed or moved to another document is not run.
    const current = new Map([...this.apps.values()].map((app) => [app.targetId, app.document]));
    const calls = this.calls.filter((call) => current.has(call.target_id)
      && (call.document_id == null || current.get(call.target_id) == null || call.document_id === current.get(call.target_id)));
    this.calls = [];
    // Keep App windows fullscreen (someone may press Esc), so the panel beside
    // each page covers exactly the desktop the page leaves free.
    this.maintainFullscreen();
    // Open App targets let the kernel drop views that closed or crashed, and
    // their documents let it cancel calls made by a document that is gone.
    const documents = Object.fromEntries([...current].filter(([, document]) => document != null));
    return { calls, open_targets: [...current.keys()], documents,
      app_panels: Boolean(this.browser.appMetrics?.()) };
  }

  async respond(params) {
    await this.reconcile();
    const entry = [...this.apps.entries()].find(([, app]) => app.targetId === params?.target_id);
    if (!entry || typeof params.call_id !== "string") throw invalid("unknown App tab or call");
    const [sessionId, app] = entry;
    // The calling document is gone: its answer must not reach the next one.
    if (!app.pending.delete(params.call_id)) return { delivered: false };
    const ok = params.error == null;
    const value = ok ? params.result ?? null : { code: String(params.error.code ?? "APP_ERROR"), message: String(params.error.message ?? "App call failed") };
    await this.resolve(sessionId, params.call_id, ok, value);
    return { delivered: true };
  }
}
