// App views: one tab per installation on its own https origin. The controller
// serves the kernel-verified UI assets through CDP Fetch interception, so no
// port is opened and the page is a secure context. Every other request from
// the tab is blocked at the browser boundary, and popups it opens are closed.
// `window.chariox.call` is a CDP binding; the kernel answers each call.
import { BrowserControllerError } from "./browser-controller-cdp.mjs";

export const APP_ORIGIN_SUFFIX = ".app.chariox.internal";
const MAX_ASSETS = 256;
const MAX_ASSET_BYTES = 8 * 1024 * 1024;
const MAX_PENDING_CALLS = 64;
const MAX_CALL_BYTES = 256 * 1024;
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
  return `https://${label}${APP_ORIGIN_SUFFIX}`;
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
  constructor(browser) {
    this.browser = browser;
    // Every new CDP connection sweeps App Tabs, even when no App command
    // arrives: interception ends with the old connection.
    browser.onConnected = () => { this.reconcile().catch(() => {}); };
    this.apps = new Map(); // sessionId -> app
    this.calls = [];
    this.connection = null;
    this.unsubscribe = null;
  }

  async open(params) {
    const origin = appOrigin(params?.origin_label);
    if (typeof params?.installation_id !== "string" || !params.installation_id) throw invalid("missing installation");
    const entry = params.entry ?? "index.html";
    const app = { installation: params.installation_id, origin, entry, assets: assets(params.assets, entry), page: params.page ?? null };
    const connection = await this.browser.ensureConnection();
    this.listen(connection);
    // One shared App Tab per Room and installation: opening it again (another
    // terminal, or a kernel that restarted) shows that Tab with the current
    // assets instead of a second one.
    const shown = [...this.apps.entries()].find(([, open]) => open.installation === app.installation && open.origin === origin);
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
    const { targetId } = await connection.send("Target.createTarget", { url: "about:blank", newWindow: true });
    const sessionId = await this.browser.ensureTargetSession(connection, targetId);
    this.apps.set(sessionId, { ...app, targetId });
    try {
      await this.fullscreen(connection, targetId);
      const metrics = this.browser.appMetrics?.(app.page);
      if (metrics) await connection.send("Emulation.setDeviceMetricsOverride", metrics, sessionId);
      await connection.send("Fetch.enable", { patterns: [{ urlPattern: "*", requestStage: "Request" }] }, sessionId);
      await connection.send("Runtime.addBinding", { name: BINDING }, sessionId);
      await connection.send("Page.addScriptToEvaluateOnNewDocument", { source: BRIDGE_SOURCE }, sessionId);
      await connection.send("Page.navigate", { url: `${origin}/` }, sessionId);
    } catch (error) {
      this.apps.delete(sessionId);
      await connection.send("Target.closeTarget", { targetId }).catch(() => {});
      throw error;
    }
    return { target_id: targetId, origin };
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
    this.apps.clear();
    this.calls = [];
    this.swept = false;
    this.connection = connection;
    this.unsubscribe = connection.subscribe((message) => {
      void this.handle(connection, message).catch(() => {});
    });
  }

  async handle(connection, message) {
    if (message?.method === "Target.targetCreated") {
      const opener = message.params?.targetInfo?.openerId;
      if (opener && [...this.apps.values()].some((app) => app.targetId === opener)) {
        await connection.send("Target.closeTarget", { targetId: message.params.targetInfo.targetId });
      }
      return;
    }
    if (message?.method === "Target.detachedFromTarget") {
      this.apps.delete(message.params?.sessionId);
      return;
    }
    const app = this.apps.get(message?.sessionId);
    if (!app) return;
    if (message.method === "Fetch.requestPaused") {
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
    if (typeof payload !== "string" || payload.length > MAX_CALL_BYTES || this.calls.length >= MAX_PENDING_CALLS) return;
    let call;
    try { call = JSON.parse(payload); } catch { return; }
    if (typeof call?.id !== "string" || typeof call.method !== "string" || call.method.length > 128) return;
    this.calls.push({ installation_id: app.installation, target_id: app.targetId, call_id: call.id,
      method: call.method, params: call.params ?? {} });
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

  // A lost CDP connection ends Fetch interception and the bridge for every
  // App Tab. Forget those Tabs and close any App-origin Tab this controller
  // does not own (including ones left by an earlier controller process).
  async reconcile() {
    const connection = await this.browser.ensureConnection();
    this.listen(connection);
    if (this.swept) return;
    const { targetInfos = [] } = await connection.send("Target.getTargets", {});
    const owned = new Set([...this.apps.values()].map((app) => app.targetId));
    for (const target of targetInfos) {
      let host = "";
      try { host = new URL(target.url).hostname; } catch {}
      if (host.endsWith(APP_ORIGIN_SUFFIX) && !owned.has(target.targetId)) {
        await connection.send("Target.closeTarget", { targetId: target.targetId }).catch(() => {});
      }
    }
    this.swept = true;
  }

  /** Drain pending view calls; the kernel answers each with `respond`. */
  async takeCalls() {
    await this.reconcile();
    const calls = this.calls;
    this.calls = [];
    // Keep App windows fullscreen (someone may press Esc), so the panel beside
    // each page covers exactly the desktop the page leaves free.
    for (const app of this.apps.values()) {
      await this.fullscreen(this.connection, app.targetId).catch(() => false);
    }
    // Open App targets let the kernel drop views that closed or crashed.
    return {
      calls,
      open_targets: [...this.apps.values()].map((app) => app.targetId),
      app_panels: Boolean(this.browser.appMetrics?.()),
    };
  }

  async respond(params) {
    await this.reconcile();
    const entry = [...this.apps.entries()].find(([, app]) => app.targetId === params?.target_id);
    if (!entry || typeof params.call_id !== "string") throw invalid("unknown App tab or call");
    const [sessionId] = entry;
    const ok = params.error == null;
    const value = ok ? params.result ?? null : { code: String(params.error.code ?? "APP_ERROR"), message: String(params.error.message ?? "App call failed") };
    await this.resolve(sessionId, params.call_id, ok, value);
    return { delivered: true };
  }
}
