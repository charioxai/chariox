// MD-2: sessionless host adapter over the shared controller/CDP implementation.
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";
import { BrowserControllerStdioServer, handleBrowserControllerRequest } from "./browser-controller.mjs";
import { HostChromium } from "./kernel-browser-process.mjs";
import { redactObservation } from "./browser-controller-snapshot.mjs";
import { captureProtectedPage, wholeFrameMask } from "./kernel-browser-pixels.mjs";

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
  constructor(root, { chromium = new HostChromium(root), browserFactory = endpoint => new BrowserCdpClient({ debuggerEndpoint: endpoint }) } = {}) {
    this.root = root;
    this.chromium = chromium;
    this.browserFactory = browserFactory;
    this.browser = null;
    this.generation = 0;
    this.tabs = new Map();
    this.streams = new Map();
    this.restoring = false;
    this.keepaliveTarget = null;
    this.protection = { values: [], targets: [], unknown: false };
  }
  async protect(policy) {
    if (!Array.isArray(policy.values) || policy.values.length > 256 || policy.values.some(value => typeof value !== "string" || !value) || !Array.isArray(policy.targets)) throw new Error("MD-5: invalid protection policy");
    if (JSON.stringify(policy) === JSON.stringify(this.protection)) return {};
    this.protection = policy;
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
  async start() {
    if (this.browser && this.chromium.child?.exitCode === null && this.chromium.child?.signalCode === null) return;
    await this.browser?.close();
    this.browser = null;
    for (const stream of this.streams.values()) { stream.off(); clearTimeout(stream.timer); }
    this.streams.clear();
    this.tabs.clear();
    await mkdir(this.root, { recursive: true, mode: 0o700 });
    let saved = { generation: 0, tabs: [] };
    try { saved = JSON.parse(await readFile(path.join(this.root, "tabs.json"), "utf8")); }
    catch (error) { if (error.code !== "ENOENT") throw new Error("MD-2: browser tab registry is unreadable; preserve it for recovery"); }
    if (!Number.isSafeInteger(saved.generation) || saved.generation < 0 || !Array.isArray(saved.tabs)) {
      throw new Error("MD-2: invalid browser tab registry");
    }
    const endpoint = await this.chromium.start();
    this.browser = this.browserFactory(endpoint);
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
        await this.open(restorationUrl(tab.url), tab.tab_id);
      }
      await this.save();
    } catch (error) { await this.stop(); throw error; }
    finally { this.restoring = false; }
  }
  async stop() {
    await this.chromium.stop(this.browser?.connection);
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
    if (!this.restoring) await this.save();
    return redactObservation({ state: "ready", generation: this.generation,
      tabs: [...this.tabs.values()].map(({ target_id, ...tab }) => tab), viewport }, this.protection.values);
  }
  async open(url, tabId = `host-tab-${randomUUID()}`) {
    if (this.tabs.size >= TAB_LIMIT) throw new Error("MD-2: host tab limit reached");
    const connection = await this.browser.ensureConnection();
    const { targetId } = await connection.send("Target.createTarget", { url: navigationUrl(url) });
    this.tabs.set(tabId, { tab_id: tabId, target_id: targetId, url, title: "", document_id: "" });
    const state = await this.reconcile();
    return { ...state, tab_id: tabId };
  }
  async target(command) {
    if (command.generation !== this.generation) throw new Error("MD-2: stale browser generation; refresh state");
    await this.reconcile();
    const tab = this.tabs.get(command.tab_id);
    if (!tab) throw new Error("MD-2: host tab does not exist");
    return tab;
  }
  async input(tab, input) {
    const { connection, sessionId } = await this.browser.resolvePageTarget(tab.target_id);
    return this.browser.inputCapture.run(connection, sessionId, async () => {
      if (input.kind === "text") {
        if (typeof input.text !== "string" || input.text.length > 16384) throw new Error("MD-2: input text exceeds limit");
        const { frameTree } = await connection.send("Page.getFrameTree", {}, sessionId);
        const { executionContextId } = await connection.send("Page.createIsolatedWorld", {
          frameId: frameTree.frame.id, worldName: "chariox-host-input", grantUniveralAccess: false,
        }, sessionId);
        const { result } = await connection.send("Runtime.evaluate", {
          contextId: executionContextId,
          expression: "(() => { let e = document.activeElement; while(e?.shadowRoot?.activeElement) e = e.shadowRoot.activeElement; return !!e && (e.type === 'password' || e.tagName === 'IFRAME' || /password|one-time-code/.test(e.autocomplete || '')); })()",
          returnByValue: true,
        }, sessionId);
        if (result?.value !== false) throw new Error("MD-2: secret field input requires the Vault path");
        await connection.send("Input.insertText", { text: input.text }, sessionId);
      } else if (input.kind === "key") {
        if (!["Tab", "Enter", "Escape", "Backspace", "Delete", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(input.key)) throw new Error("MD-2: unsupported key");
        await connection.send("Input.dispatchKeyEvent", { type: "keyDown", key: input.key }, sessionId);
        await connection.send("Input.dispatchKeyEvent", { type: "keyUp", key: input.key }, sessionId);
      } else {
        if (!Number.isInteger(input.x) || input.x < 0 || input.x >= viewport.css_width || !Number.isInteger(input.y) || input.y < 0 || input.y >= viewport.css_height) throw new Error("MD-2: pointer outside viewport");
        if (input.kind === "click") {
          await connection.send("Input.dispatchMouseEvent", { type: "mousePressed", x: input.x, y: input.y, button: "left", clickCount: 1 }, sessionId);
          await connection.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: input.x, y: input.y, button: "left", clickCount: 1 }, sessionId);
        } else if (input.kind === "scroll" && Number.isInteger(input.delta_x) && Number.isInteger(input.delta_y) && Math.abs(input.delta_x) <= 10000 && Math.abs(input.delta_y) <= 10000) {
          await connection.send("Input.dispatchMouseEvent", { type: "mouseWheel", x: input.x, y: input.y, deltaX: input.delta_x, deltaY: input.delta_y }, sessionId);
        } else throw new Error("MD-2: unsupported input");
      }
    });
  }
  async screenshot(tab) {
    const { connection, sessionId } = await this.browser.resolvePageTarget(tab.target_id);
    const data = await captureProtectedPage(this.browser, tab, this.protection.values,
      this.protection.targets.filter(target => target.kind === "browser"), async () => {
        const { data } = await connection.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false }, sessionId);
        return data;
      });
    if (typeof data !== "string" || data.length > 4 * 1024 * 1024) throw new Error("MD-2: frame exceeds limit");
    return { generation: this.generation, tab_id: tab.tab_id, mime_type: "image/png", data_base64: data, width: 1280, height: 800 };
  }
  async subscribe(tab) {
    if (this.streams.size >= 16) throw new Error("MD-2: frame subscription limit reached");
    const { connection, sessionId } = await this.browser.resolvePageTarget(tab.target_id);
    const id = `host-stream-${randomUUID()}`;
    const stream = { sessionId, tabId: tab.tab_id, latest: null, sequence: 0, expires: Date.now() + 60_000 };
    const captureProtected = () => {
      stream.latest ??= this.maskedStreamFrame(stream);
      if (stream.capturing || this.protection.unknown || Date.now() < (stream.nextCapture ?? 0)) return;
      stream.capturing = true;
      stream.nextCapture = Date.now() + 200;
      const policy = this.protection, generation = this.generation;
      void this.screenshot(tab).then(frame => {
        if (this.protection === policy && this.generation === generation && this.streams.get(id) === stream) {
          stream.latest = { ...frame, sequence: ++stream.sequence };
        }
      }).catch(() => {}).finally(() => { stream.capturing = false; });
    };
    stream.off = connection.subscribe(message => {
      if (message.method !== "Page.screencastFrame" || message.sessionId !== sessionId) return;
      const data = message.params?.data;
      if (typeof data === "string" && data.length <= 4 * 1024 * 1024 && Date.now() <= stream.expires) {
        const protectedPixels = this.protection.unknown || this.protection.values.length > 0;
        if (!protectedPixels) {
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
    if (this.protection.unknown || this.protection.values.length) captureProtected();
    return { generation: this.generation, subscription_id: id };
  }
  armExpiry(id, stream) {
    clearTimeout(stream.timer);
    stream.timer = setTimeout(() => { void this.removeStream(id); }, Math.max(1, stream.expires - Date.now()));
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
  async request(command) {
    if (command.op === "stop") return this.stop();
    await this.start();
    if (this.protection.unknown) throw new Error("MD-5: observation registry unavailable");
    for (const [id, stream] of this.streams) if (Date.now() > stream.expires) await this.removeStream(id);
    if (["start", "state"].includes(command.op)) return this.reconcile();
    if (command.op === "open") return this.open(command.url);
    if (["poll", "unsubscribe"].includes(command.op)) {
      if (command.generation !== this.generation || !this.streams.has(command.subscription_id)) throw new Error("MD-2: stale frame subscription");
      const stream = this.streams.get(command.subscription_id);
      stream.expires = Date.now() + 60_000;
      this.armExpiry(command.subscription_id, stream);
      if (command.op === "unsubscribe") { await this.removeStream(command.subscription_id); return { generation: this.generation, unsubscribed: true }; }
      return { generation: this.generation, frame: stream.latest };
    }
    const tab = await this.target(command);
    const binding = { target_id: tab.target_id, document_id: tab.document_id };
    if (command.op === "close") {
      for (const [id, stream] of this.streams) {
        if (stream.tabId === tab.tab_id) await this.removeStream(id);
      }
      await this.browser.manageTab({ ...binding, action: "close" });
      return this.reconcile();
    }
    if (command.op === "navigate") {
      await this.browser.navigate({ ...binding, url: navigationUrl(command.url) });
      return this.reconcile();
    }
    if (command.op === "snapshot") return { generation: this.generation, snapshot: await this.browser.snapshot(binding) };
    if (command.op === "input") { await this.input(tab, command.input); return this.reconcile(); }
    if (command.op === "screenshot") return this.screenshot(tab);
    if (command.op === "subscribe") return this.subscribe(tab);
    throw new Error("MD-2: unsupported browser operation");
  }
  async handle(request) {
    try {
      if (request.method === "health") return { id: request.id, ok: true, result: { state: "ready", process_id: process.pid, diagnostic_code: null } };
      if (request.method === "shutdown") return { id: request.id, ok: true, result: await this.stop() };
      if (request.method === "host.protect") return { id: request.id, ok: true, result: await this.protect(request.params) };
      if (request.method === "host.secret") {
        await this.start();
        if (this.protection.unknown) throw new Error("MD-5: observation registry unavailable");
        const tab = await this.target(request.params);
        if (tab.document_id !== request.params.document_id) throw new Error("MD-5: stale secret document");
        await this.browser.performAction({ target_id: tab.target_id, document_id: tab.document_id,
          node_ref: request.params.node_ref, action: request.params.action, timeout_ms: 10_000 });
        await this.save();
        return { id: request.id, ok: true, result: { inserted: true } };
      }
      if (request.method === "host.browser") {
        const result = await this.request(request.params);
        // Structured controller observations scrub before compaction; metadata
        // and other host replies receive the same protection at this boundary.
        if (!["screenshot", "poll"].includes(request.params?.op)) {
          return { id: request.id, ok: true, result: redactObservation(result, this.protection.values) };
        }
        return { id: request.id, ok: true, result };
      }
      // Kernel-internal App adapter; no public raw CDP dispatch.
      if (request.method.startsWith("browser.app.")) {
        await this.start();
        const result = await handleBrowserControllerRequest(request, { browser: this.browser });
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
    } catch { return { id: request.id, ok: false, error: { code: "kernel_browser_failed", message: "MD-2: host browser operation failed; refresh state or check host browser readiness" } }; }
  }
}

if (process.argv[2] === "stdio") {
  const host = new KernelBrowserHost(process.argv[3]);
  let closing = false;
  const stop = async () => { if (closing) return; closing = true; await host.stop(); process.exit(0); };
  process.on("SIGTERM", stop);
  process.on("SIGINT", stop);
  await new BrowserControllerStdioServer({ handleRequest: request => host.handle(request) }).run().finally(() => host.stop());
}
