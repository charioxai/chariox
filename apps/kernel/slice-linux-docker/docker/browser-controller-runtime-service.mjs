#!/usr/bin/env node

import { timingSafeEqual } from "node:crypto";
import { AsyncLocalStorage } from "node:async_hooks";
import { chmod, mkdir, readFile, unlink, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { createServer, connect } from "node:net";
import { pathToFileURL } from "node:url";

import {
  BrowserController,
  ControllerError,
  ERROR_CODES,
} from "./browser-controller.mjs";
import {
  BrowserRuntimeMcpAdapter,
  RUNTIME_MCP_ERROR_CODES,
} from "./browser-controller-runtime-mcp.mjs";

const DEFAULT_SOCKET_PATH = "/opt/chariox-slice/private/browser-runtime-mcp.sock";
const DEFAULT_AUTH_FILE = "/opt/chariox-slice/private/browser-runtime-mcp.auth";
const MAX_REQUEST_BYTES = 64 * 1024;
const MAX_RESPONSE_BYTES = 1024 * 1024;
const MAX_ERROR_BYTES = 256;
const REQUEST_TIMEOUT_MS = 70_000;
const CONTROLLER_START_TIMEOUT_MS = 10_000;
const DEFAULT_WAIT_TIMEOUT_MS = 10_000;
const MIN_WAIT_TIMEOUT_MS = 100;
const MAX_WAIT_TIMEOUT_MS = 60_000;
const REQUEST_CONTEXT = new AsyncLocalStorage();

const RUNTIME_ERROR_CODES = Object.freeze({
  AUTH_REQUIRED: "RUNTIME_MCP_AUTH_REQUIRED",
  MALFORMED: "RUNTIME_MCP_MALFORMED",
  UNAUTHORIZED: "RUNTIME_MCP_UNAUTHORIZED",
  TIMEOUT: "RUNTIME_MCP_TIMEOUT",
  UNAVAILABLE: "RUNTIME_MCP_UNAVAILABLE",
  INTERNAL: "RUNTIME_MCP_INTERNAL",
});

const SAFE_ERROR_CODES = new Set([
  ...Object.values(RUNTIME_MCP_ERROR_CODES),
  ...Object.values(RUNTIME_ERROR_CODES),
  ...Object.values(ERROR_CODES),
]);

function isPlainObject(value) {
  if (value === null || typeof value !== "object") return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function exactKeys(value, allowed) {
  return isPlainObject(value) && Object.keys(value).every((key) => allowed.includes(key));
}

function requestId(value) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    Buffer.byteLength(value, "utf8") > 128 ||
    !/^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(value)
  ) {
    throw new Error(RUNTIME_ERROR_CODES.MALFORMED);
  }
  return value;
}

function boundedJson(value, maximum) {
  const encoded = JSON.stringify(value);
  if (typeof encoded !== "string" || Buffer.byteLength(encoded, "utf8") > maximum) {
    throw new Error(RUNTIME_ERROR_CODES.MALFORMED);
  }
  return encoded;
}

function safeCode(error, fallback = RUNTIME_ERROR_CODES.INTERNAL) {
  const code = typeof error?.code === "string"
    ? error.code
    : typeof error?.message === "string"
      ? error.message
      : fallback;
  return SAFE_ERROR_CODES.has(code) ? code : fallback;
}

function errorResponse(type, requestIdValue, error) {
  const code = safeCode(error);
  return {
    type,
    request_id: requestIdValue ?? null,
    ok: false,
    error: {
      code,
      message: code.slice(0, MAX_ERROR_BYTES),
    },
  };
}

function compareAuthToken(expected, received) {
  if (typeof received !== "string") return false;
  const expectedBytes = Buffer.from(expected, "utf8");
  const receivedBytes = Buffer.from(received, "utf8");
  if (expectedBytes.length !== receivedBytes.length) return false;
  return timingSafeEqual(expectedBytes, receivedBytes);
}

function attachedBrowserProcess() {
  const listeners = new Map();
  let exited = false;
  const emit = (event, ...args) => {
    for (const listener of listeners.get(event) ?? []) listener(...args);
  };
  return {
    stdout: { resume() {} },
    stderr: { resume() {} },
    on(event, listener) {
      const current = listeners.get(event) ?? [];
      current.push(listener);
      listeners.set(event, current);
      return this;
    },
    kill() {
      if (exited) return;
      exited = true;
      queueMicrotask(() => emit("exit", null, "SIGTERM"));
    },
  };
}

function domScript() {
  return String.raw`
    window.__charioxRuntimeElementIds = window.__charioxRuntimeElementIds || new WeakMap();
    window.__charioxRuntimeNextElementId = window.__charioxRuntimeNextElementId || 1;
    const visible = (element) => {
      const style = window.getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return style.visibility !== "hidden" && style.display !== "none" && rect.width > 0 && rect.height > 0;
    };
    const bounded = (value, maximum = 160) => String(value || "").trim().slice(0, maximum);
    const cssString = (value) => CSS.escape(String(value));
    const selectorFor = (element) => {
      if (element.id) return "#" + cssString(element.id);
      if (element.name) {
        const named = element.tagName.toLowerCase() + "[name='" + cssString(element.name) + "']";
        if (document.querySelectorAll(named).length === 1) return named;
      }
      const parts = [];
      let current = element;
      while (current && current.nodeType === Node.ELEMENT_NODE && current !== document.body) {
        const tag = current.tagName.toLowerCase();
        const siblings = Array.from(current.parentElement?.children ?? []).filter((item) => item.tagName === current.tagName);
        const index = siblings.indexOf(current) + 1;
        parts.unshift(siblings.length > 1 ? tag + ":nth-of-type(" + index + ")" : tag);
        current = current.parentElement;
      }
      return parts.length ? "body > " + parts.join(" > ") : element.tagName.toLowerCase();
    };
    const fieldIdFor = (element, kind) => {
      const existing = window.__charioxRuntimeElementIds.get(element);
      if (existing) return existing;
      const id = kind + ":" + String(window.__charioxRuntimeNextElementId++);
      window.__charioxRuntimeElementIds.set(element, id);
      return id;
    };
    const labelFor = (element) => {
      if (element.id) {
        const label = document.querySelector("label[for='" + cssString(element.id) + "']");
        if (label?.innerText?.trim()) return bounded(label.innerText);
      }
      const wrapper = element.closest("label");
      return wrapper?.innerText?.trim() ? bounded(wrapper.innerText) : "";
    };
    const summaryTextFor = (element) => {
      const type = String(element.getAttribute("type") || "").toLowerCase();
      if (type === "password") return bounded(element.getAttribute("placeholder") || element.getAttribute("aria-label") || "Password");
      return bounded(element.innerText || element.value || element.getAttribute("aria-label") || "");
    };
    const summarize = (element, kind) => ({
      kind,
      selector: bounded(selectorFor(element), 512),
      field_id: fieldIdFor(element, kind),
      tag: bounded(element.tagName, 64).toLowerCase(),
      type: bounded(element.getAttribute("type"), 64),
      name: bounded(element.getAttribute("name"), 128),
      id: bounded(element.id, 128),
      role: bounded(element.getAttribute("role"), 64),
      label: labelFor(element),
      placeholder: bounded(element.getAttribute("placeholder"), 160),
      text: summaryTextFor(element),
      disabled: Boolean(element.disabled),
      readOnly: Boolean(element.readOnly),
    });
    const fields = Array.from(document.querySelectorAll("input, textarea, select, [contenteditable=true]")).filter(visible).slice(0, 32).map((element) => summarize(element, "field"));
    const buttons = Array.from(document.querySelectorAll("button, input[type=button], input[type=submit], [role=button]")).filter(visible).slice(0, 32).map((element) => summarize(element, "button"));
    const links = Array.from(document.querySelectorAll("a[href], [role=link]")).filter(visible).slice(0, 32).map((element) => summarize(element, "link"));
    const resolveTarget = (target, kinds = ["field", "button", "link"]) => {
      if (!target) return null;
      try {
        const bySelector = document.querySelector(target);
        if (bySelector) return bySelector;
      } catch (_) {
        // Opaque field_id values are not CSS selectors.
      }
      const selectorByKind = {
        field: "input, textarea, select, [contenteditable=true]",
        button: "button, input[type=button], input[type=submit], [role=button]",
        link: "a[href], [role=link]",
      };
      const selectors = kinds.map((kind) => selectorByKind[kind]).filter(Boolean).join(",");
      for (const element of Array.from(document.querySelectorAll(selectors))) {
        for (const kind of kinds) if (fieldIdFor(element, kind) === target) return element;
      }
      return null;
    };
  `;
}

function evaluateExpression(source, args = {}) {
  const encoded = boundedJson(args, 48 * 1024);
  return `(() => { const __charioxArgs = ${encoded}; ${source} })()`;
}

async function evaluate(cdp, source, args, timeoutMs) {
  const result = await cdp.send(
    "Runtime.evaluate",
    {
      expression: evaluateExpression(source, args),
      awaitPromise: false,
      returnByValue: true,
    },
    timeoutMs,
  );
  return result?.result?.value;
}

function targetFor(args) {
  return args.selector ?? args.field_id ?? null;
}

class BrowserRuntimeSemanticPort {
  constructor(controller, ownerId, requestContext = REQUEST_CONTEXT) {
    this.controller = controller;
    this.ownerId = ownerId;
    this.requestContext = requestContext;
    this.startPromise = null;
    this.nextActionId = 1;
    this.targetSelectors = new Map();
    this.activeMutations = new Map();
  }

  async ensureReady() {
    if (this.controller.state === "ready") return;
    if (this.startPromise) return this.startPromise;
    const start = this.controller.state === "fatal"
      ? this.controller.restart(this.ownerId, this.controller.generation)
      : this.controller.start(this.ownerId);
    this.startPromise = Promise.resolve(start).finally(() => {
      this.startPromise = null;
    });
    return this.startPromise;
  }

  async page(operation, selection = null) {
    await this.ensureReady();
    const selected = selection ?? await this.tab();
    const generation = selected.generation;
    return this.controller._enqueue(generation, async () => {
      if (this.controller.state !== "ready" || !this.controller._cdp) {
        throw new ControllerError(ERROR_CODES.CONTROLLER_NOT_READY);
      }
      const { connection } = await this.targetConnection(selected);
      return operation(connection, this.controller.cdpCommandTimeoutMs);
    });
  }

  rememberTargets(value) {
    const entries = [
      ...(Array.isArray(value?.fields) ? value.fields : []),
      ...(Array.isArray(value?.buttons) ? value.buttons : []),
      ...(Array.isArray(value?.links) ? value.links : []),
      ...(value?.focusedElement ? [value.focusedElement] : []),
    ];
    for (const entry of entries) {
      if (typeof entry?.field_id !== "string" || typeof entry?.selector !== "string") continue;
      if (entry.selector.length === 0 || entry.selector.length > 8 * 1024) continue;
      this.targetSelectors.set(entry.field_id, entry.selector);
    }
    while (this.targetSelectors.size > 256) {
      const oldest = this.targetSelectors.keys().next().value;
      this.targetSelectors.delete(oldest);
    }
  }

  semanticTarget(args) {
    if (args.selector !== undefined) return args.selector;
    return this.targetSelectors.get(args.field_id) ?? args.field_id;
  }

  requestId() {
    return this.requestContext?.getStore()?.requestId ?? null;
  }

  mutationActionId() {
    const actionId = `runtime-${process.pid}-${this.nextActionId++}`;
    return actionId.slice(0, 128);
  }

  async withMutation(run) {
    const actionId = this.mutationActionId();
    const actorId = this.ownerId;
    const requestId = this.requestId();
    const record = { actionId, actorId };
    if (requestId) {
      const records = this.activeMutations.get(requestId) ?? new Set();
      records.add(record);
      this.activeMutations.set(requestId, records);
    }
    try {
      return await run({ action_id: actionId, actor_id: actorId });
    } finally {
      if (requestId) {
        const records = this.activeMutations.get(requestId);
        records?.delete(record);
        if (records?.size === 0) this.activeMutations.delete(requestId);
      }
    }
  }

  cancelMutationForRequest(requestId) {
    const records = this.activeMutations.get(requestId);
    if (!records || records.size === 0) return { accepted: false, state: "unknown" };
    let result = { accepted: false, state: "unknown" };
    for (const record of records) {
      result = this.controller.cancelMutation({
        action_id: record.actionId,
        actor_id: record.actorId,
      });
    }
    return result;
  }

  async tab() {
    await this.ensureReady();
    const generation = this.controller.generation;
    const snapshot = this.controller.getTabRegistrySnapshot(this.ownerId, generation);
    const tab = snapshot.tabs?.[0];
    if (!tab) throw new ControllerError(ERROR_CODES.CDP_UNAVAILABLE);
    return { generation, tab };
  }

  async targetConnection(selection) {
    const registry = this.controller.tabRegistry;
    if (
      !registry ||
      typeof registry.resolveTarget !== "function" ||
      typeof this.controller._ensureTargetConnection !== "function"
    ) {
      throw new ControllerError(ERROR_CODES.CDP_UNAVAILABLE);
    }
    const target = registry.resolveTarget(selection.tab.tab_id, {
      generation: selection.generation,
      target_generation: selection.tab.target_generation,
    });
    const connection = await this.controller._ensureTargetConnection(target);
    if (!connection || connection.closed) {
      throw new ControllerError(ERROR_CODES.CDP_UNAVAILABLE);
    }
    return { target, connection };
  }

  async backendNodeIdForSelector(selector, selection = null) {
    if (typeof selector !== "string" || selector.trim().length === 0) return null;
    return this.page(async (cdp, timeoutMs) => {
      const document = await cdp.send("DOM.getDocument", { depth: 1, pierce: true }, timeoutMs);
      const rootNodeId = document?.root?.nodeId;
      if (!Number.isSafeInteger(rootNodeId)) return null;
      let match;
      try {
        match = await cdp.send("DOM.querySelector", { nodeId: rootNodeId, selector }, timeoutMs);
      } catch {
        return null;
      }
      if (!Number.isSafeInteger(match?.nodeId) || match.nodeId < 1) return null;
      const described = await cdp.send("DOM.describeNode", { nodeId: match.nodeId }, timeoutMs);
      const backendNodeId = described?.node?.backendNodeId;
      return Number.isSafeInteger(backendNodeId) && backendNodeId > 0 ? backendNodeId : null;
    }, selection);
  }

  async elementReferenceForSelector(selector, selection = null) {
    selection = selection ?? await this.tab();
    const { generation, tab } = selection;
    const snapshot = await this.controller.captureTabSnapshot(this.ownerId, generation, {
      tab_id: tab.tab_id,
      target_generation: tab.target_generation,
    });
    const backendNodeId = await this.backendNodeIdForSelector(selector, selection);
    if (backendNodeId === null) throw new ControllerError(ERROR_CODES.ACTION_FAILED);
    const candidates = [
      ...(snapshot.accessibility_nodes ?? []),
      ...(snapshot.dom_nodes ?? []),
    ];
    const seen = new Set();
    for (const candidate of candidates) {
      const elementRef = candidate?.element_ref;
      if (typeof elementRef !== "string" || seen.has(elementRef)) continue;
      seen.add(elementRef);
      try {
        const element = this.controller.resolveElementReference(this.ownerId, generation, {
          tab_id: tab.tab_id,
          target_generation: tab.target_generation,
          element_ref: elementRef,
        });
        if (element.backend_node_id === backendNodeId) {
          return {
            generation,
            request: {
              tab_id: tab.tab_id,
              target_generation: tab.target_generation,
              element_ref: elementRef,
            },
          };
        }
      } catch {
        // The observation was invalidated while resolving the selector.
      }
    }
    throw new ControllerError(ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED);
  }

  async submitSelector(args, selection = null) {
    const target = this.semanticTarget(args);
    return this.page((cdp, timeoutMs) => evaluate(cdp, `
      ${domScript()}
      const target = __charioxArgs.target
        ? resolveTarget(__charioxArgs.target, ["field", "button", "link"])
        : document.activeElement;
      if (!target) return null;
      const form = target.closest?.("form");
      if (!form) return null;
      const submit = target.matches?.("button, input[type=submit], [role=button]")
        ? target
        : form.querySelector("button[type=submit], input[type=submit], button, [role=button]");
      return submit ? selectorFor(submit) : null;
    `, { target: target ?? null }, timeoutMs), selection);
  }

  status() {
    return this.page((cdp, timeoutMs) => evaluate(cdp, `
      ${domScript()}
      const active = document.activeElement && document.activeElement !== document.body
        ? summarize(document.activeElement, document.activeElement.matches("input, textarea, select, [contenteditable=true]") ? "field" : "element")
        : null;
      return {
        url: String(location.href || "").slice(0, 2048),
        host: String(location.host || "").slice(0, 512),
        title: String(document.title || "").slice(0, 512),
        readyState: document.readyState,
        focusedElement: active,
        fields,
        buttons,
        links,
      };
    `, {}, timeoutMs)).then((result) => {
      this.rememberTargets(result);
      return result;
    });
  }

  find(args) {
    return this.page((cdp, timeoutMs) => evaluate(cdp, `
      ${domScript()}
      const query = String(__charioxArgs.query || "").toLowerCase();
      const kind = __charioxArgs.kind || "any";
      const pool = [
        ...(kind === "field" || kind === "any" ? fields : []),
        ...(kind === "button" || kind === "any" ? buttons : []),
        ...(kind === "link" || kind === "any" ? links : []),
      ];
      const matches = pool.filter((entry) => [
        entry.selector, entry.id, entry.name, entry.role, entry.label,
        entry.placeholder, entry.text, entry.type,
      ].some((value) => String(value || "").toLowerCase().includes(query)));
      return { query: __charioxArgs.query, kind, matches };
    `, args, timeoutMs)).then((result) => {
      this.rememberTargets({ fields: result?.matches, buttons: result?.matches, links: result?.matches });
      return result;
    });
  }

  async fill(args) {
    const target = await this.elementReferenceForSelector(this.semanticTarget(args));
    await this.withMutation((mutation) => this.controller.performElementAction(
      this.ownerId,
      target.generation,
      { ...target.request, action: { kind: "fill", text: args.text, append: false } },
      mutation,
    ));
    return { ok: true, selector: targetFor(args) };
  }

  async click(args) {
    const target = await this.elementReferenceForSelector(this.semanticTarget(args));
    await this.withMutation((mutation) => this.controller.performElementAction(
      this.ownerId,
      target.generation,
      { ...target.request, action: { kind: "click" } },
      mutation,
    ));
    return { ok: true, selector: targetFor(args) };
  }

  async submit(args) {
    const selection = await this.tab();
    const selector = await this.submitSelector(args, selection);
    if (!selector) throw new ControllerError(ERROR_CODES.ACTION_FAILED);
    const target = await this.elementReferenceForSelector(selector, selection);
    await this.withMutation((mutation) => this.controller.performElementAction(
      this.ownerId,
      target.generation,
      { ...target.request, action: { kind: "click" } },
      mutation,
    ));
    return { ok: true, selector: targetFor(args) };
  }

  async dialog(args) {
    const { generation, tab } = await this.tab();
    await this.withMutation((mutation) => this.controller.handleDialog(
      this.ownerId,
      generation,
      {
        tab_id: tab.tab_id,
        target_generation: tab.target_generation,
        dialog: {
          action: args.action,
          ...(args.prompt_text === undefined ? {} : { prompt_text: args.prompt_text }),
        },
      },
      mutation,
    ));
    return { ok: true, action: args.action };
  }

  text() {
    return this.page((cdp, timeoutMs) => evaluate(cdp, `
      return String(document.body?.innerText || "").slice(0, 48 * 1024);
    `, {}, timeoutMs));
  }

  waitForText(args) {
    return this.wait(args.timeout_ms ?? DEFAULT_WAIT_TIMEOUT_MS, (cdp, timeoutMs) => evaluate(cdp, `
      const text = String(__charioxArgs.text || "");
      return { ok: (document.body?.innerText || "").includes(text), text, readyState: document.readyState };
    `, { text: args.text }, timeoutMs));
  }

  waitForSelector(args) {
    return this.wait(args.timeout_ms ?? DEFAULT_WAIT_TIMEOUT_MS, (cdp, timeoutMs) => evaluate(cdp, `
      const selector = String(__charioxArgs.selector || "");
      let element;
      try { element = selector ? document.querySelector(selector) : null; } catch (_) { return { ok: false, selector, reason: "invalid" }; }
      if (!element) return { ok: false, selector, reason: "missing" };
      const style = window.getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return { ok: style.visibility !== "hidden" && style.display !== "none" && rect.width > 0 && rect.height > 0, selector, reason: "not_visible" };
    `, { selector: args.selector }, timeoutMs));
  }

  waitForIdle(args) {
    return this.wait(args.timeout_ms ?? DEFAULT_WAIT_TIMEOUT_MS, (cdp, timeoutMs) => evaluate(cdp, `
      return { ok: document.readyState === "complete", readyState: document.readyState, url: String(location.href || "").slice(0, 2048), title: String(document.title || "").slice(0, 512) };
    `, {}, timeoutMs));
  }

  async wait(timeoutMs, probe) {
    const bounded = Math.max(MIN_WAIT_TIMEOUT_MS, Math.min(MAX_WAIT_TIMEOUT_MS, timeoutMs));
    return this.page(async (cdp, commandTimeoutMs) => {
      const started = Date.now();
      let last = null;
      while (Date.now() - started <= bounded) {
        last = await probe(cdp, commandTimeoutMs);
        if (last?.ok) return { ok: true, waited_ms: Date.now() - started, ...last };
        await new Promise((resolve) => setTimeout(resolve, Math.min(50, bounded)));
      }
      return { ok: false, error: "timeout", timeout_ms: bounded, waited_ms: Date.now() - started, last };
    });
  }
}

class AttachedBrowserController extends BrowserController {
  // BrowserController normally sends Browser.close during shutdown because it
  // owns the child it spawned. The desktop lifecycle owns Chromium here, so
  // this controller only detaches its CDP connection and terminates its
  // in-process attachment record.
  async shutdownForSignal() {
    this._stopHeartbeat();
    this._invalidateAllMutations();
    this._closeTargetConnections();
    this._rejectQueued(new ControllerError(ERROR_CODES.REQUEST_CANCELLED));
    const connection = this._cdp;
    this._cdp = null;
    connection?.close();
    const record = this._process;
    this._process = null;
    if (record && !record.exited) {
      record.expected = true;
      this._signal(record, "SIGTERM");
    }
    this.state = "stopped";
    this.fatalCode = null;
    return this.health();
  }
}

function semanticPortFor(controller, ownerId, requestContext = REQUEST_CONTEXT) {
  const semantic = new BrowserRuntimeSemanticPort(controller, ownerId, requestContext);
  return {
    startController: () => semantic.ensureReady(),
    status: () => semantic.status(),
    find: (args) => semantic.find(args),
    fill: (args) => semantic.fill(args),
    click: (args) => semantic.click(args),
    submit: (args) => semantic.submit(args),
    dialog: (args) => semantic.dialog(args),
    text: () => semantic.text(),
    waitForText: (args) => semantic.waitForText(args),
    waitForSelector: (args) => semantic.waitForSelector(args),
    waitForIdle: (args) => semantic.waitForIdle(args),
    cancelMutationForRequest: (requestId) => semantic.cancelMutationForRequest(requestId),
  };
}

export class BrowserRuntimeMcpService {
  constructor({
    socketPath,
    authToken,
    authFile,
    adapter,
    controller = null,
    requestTimeoutMs = REQUEST_TIMEOUT_MS,
    requestContext = REQUEST_CONTEXT,
  } = {}) {
    if (typeof socketPath !== "string" || socketPath.length === 0) throw new TypeError("socketPath is required");
    if (!(adapter instanceof BrowserRuntimeMcpAdapter)) throw new TypeError("adapter is required");
    this.socketPath = socketPath;
    this.authToken = authToken;
    this.authFile = authFile;
    this.adapter = adapter;
    this.controller = controller;
    this.requestContext = requestContext;
    this.requestTimeoutMs = Math.max(100, Math.min(REQUEST_TIMEOUT_MS, requestTimeoutMs));
    this.server = null;
    this.connections = new Set();
    this.stopping = false;
  }

  async start() {
    if (!this.authToken && this.authFile) this.authToken = (await readFile(this.authFile, "utf8")).trim();
    if (typeof this.authToken !== "string" || this.authToken.length === 0) throw new Error(RUNTIME_ERROR_CODES.AUTH_REQUIRED);
    if (typeof this.adapter.port.startController === "function") {
      await this.withTimeout(
        this.adapter.port.startController(),
        Math.min(this.requestTimeoutMs, CONTROLLER_START_TIMEOUT_MS),
        () => this.controller?.shutdownForSignal(),
      );
    }
    await mkdir(dirname(this.socketPath), { recursive: true, mode: 0o700 });
    await unlink(this.socketPath).catch(() => {});
    this.server = createServer((socket) => this.handleConnection(socket));
    await new Promise((resolve, reject) => {
      this.server.once("error", reject);
      this.server.listen(this.socketPath, () => {
        this.server.off("error", reject);
        resolve();
      });
    });
    await chmod(this.socketPath, 0o600);
    return this;
  }

  async handleLine(line) {
    if (!Buffer.isBuffer(line)) line = Buffer.from(String(line), "utf8");
    if (line.length > MAX_REQUEST_BYTES) return errorResponse("tool_result", null, new Error(RUNTIME_ERROR_CODES.MALFORMED));
    let value;
    try { value = JSON.parse(line.toString("utf8")); } catch { return errorResponse("tool_result", null, new Error(RUNTIME_ERROR_CODES.MALFORMED)); }
    if (!isPlainObject(value) || typeof value.type !== "string") return errorResponse("tool_result", null, new Error(RUNTIME_ERROR_CODES.MALFORMED));
    let id = null;
    try { id = requestId(value.request_id); } catch { return errorResponse(value.type === "health" ? "health_result" : "tool_result", null, new Error(RUNTIME_ERROR_CODES.MALFORMED)); }
    if (!compareAuthToken(this.authToken, value.auth_token)) return errorResponse(value.type === "health" ? "health_result" : "tool_result", id, new Error(RUNTIME_ERROR_CODES.UNAUTHORIZED));
    if (value.type === "health") {
      if (!exactKeys(value, ["type", "request_id", "auth_token"])) return errorResponse("health_result", id, new Error(RUNTIME_ERROR_CODES.MALFORMED));
      return { type: "health_result", request_id: id, ok: true, service: "browser-runtime-mcp", state: this.server ? "listening" : "stopped", controller_state: this.controller?.state ?? "injected" };
    }
    if (value.type !== "tool_call" || !exactKeys(value, ["type", "request_id", "auth_token", "tool_name", "arguments"]) || typeof value.tool_name !== "string" || !isPlainObject(value.arguments)) {
      return errorResponse("tool_result", id, new Error(RUNTIME_ERROR_CODES.MALFORMED));
    }
    try {
      const invocation = this.requestContext.run({ requestId: id }, () => (
        this.adapter.invoke(value.tool_name, value.arguments)
      ));
      const result = await this.withTimeout(
        invocation,
        this.requestTimeoutMs,
        () => this.adapter.port.cancelMutationForRequest?.(id),
      );
      return { type: "tool_result", request_id: id, ok: true, tool_name: result.tool_name, result: result.result };
    } catch (error) {
      return errorResponse("tool_result", id, error);
    }
  }

  async withTimeout(promise, timeoutMs, onTimeout) {
    let timer;
    const deadline = new Promise((_, reject) => {
      timer = setTimeout(() => {
        try { onTimeout?.(); } catch { /* Cancellation is best effort after the deadline. */ }
        reject(new Error(RUNTIME_ERROR_CODES.TIMEOUT));
      }, timeoutMs);
    });
    try { return await Promise.race([promise, deadline]); } finally { clearTimeout(timer); }
  }

  handleConnection(socket) {
    this.connections.add(socket);
    socket.setNoDelay(true);
    let pending = Buffer.alloc(0);
    let chain = Promise.resolve();
    const write = (response) => {
      let encoded;
      try { encoded = boundedJson(response, MAX_RESPONSE_BYTES); } catch { encoded = JSON.stringify(errorResponse("tool_result", null, new Error(RUNTIME_MCP_ERROR_CODES.RESULT_TOO_LARGE))); }
      if (!socket.destroyed) socket.write(encoded + "\n");
    };
    socket.on("data", (chunk) => {
      if (this.stopping) return;
      if (pending.length + chunk.length > MAX_REQUEST_BYTES + 1) {
        write(errorResponse("tool_result", null, new Error(RUNTIME_ERROR_CODES.MALFORMED)));
        socket.destroy();
        return;
      }
      pending = Buffer.concat([pending, chunk]);
      while (true) {
        const newline = pending.indexOf(0x0a);
        if (newline < 0) break;
        const line = pending.subarray(0, newline);
        pending = pending.subarray(newline + 1);
        chain = chain.then(() => this.handleLine(line)).then(write, (error) => write(errorResponse("tool_result", null, error)));
      }
    });
    socket.on("close", () => this.connections.delete(socket));
    socket.on("error", () => this.connections.delete(socket));
  }

  async stop() {
    this.stopping = true;
    for (const socket of this.connections) socket.destroy();
    if (this.server) await new Promise((resolve) => this.server.close(() => resolve()));
    this.server = null;
    if (this.controller) await Promise.race([this.controller.shutdownForSignal(), new Promise((resolve) => setTimeout(resolve, 4_000))]).catch(() => {});
    await unlink(this.socketPath).catch(() => {});
  }
}

export function createProductionBrowserRuntimeMcpService({ socketPath, authFile, ownerId } = {}) {
  const requestContext = new AsyncLocalStorage();
  const controller = new AttachedBrowserController({
    // The desktop lifecycle owns Chromium. The semantic controller owns the sole
    // CDP connection and serializes all DOM operations over that existing page.
    spawnBrowser: async () => attachedBrowserProcess(),
    signalProcess: (process) => process.kill?.(),
  });
  const port = semanticPortFor(
    controller,
    ownerId || `kernel:${process.env.CHARIOX_SLICE_ID || "linux"}`,
    requestContext,
  );
  return new BrowserRuntimeMcpService({
    socketPath: socketPath || process.env.CHARIOX_BROWSER_RUNTIME_MCP_SOCKET || DEFAULT_SOCKET_PATH,
    authFile: authFile || process.env.CHARIOX_BROWSER_RUNTIME_MCP_AUTH_FILE || DEFAULT_AUTH_FILE,
    adapter: new BrowserRuntimeMcpAdapter(port),
    controller,
    requestContext,
  });
}

async function probe(socketPath, authFile) {
  const token = (await readFile(authFile, "utf8")).trim();
  return new Promise((resolve, reject) => {
    const socket = connect(socketPath);
    let data = "";
    const timer = setTimeout(() => { socket.destroy(); reject(new Error(RUNTIME_ERROR_CODES.UNAVAILABLE)); }, 2_000);
    socket.setEncoding("utf8");
    socket.on("connect", () => socket.write(JSON.stringify({ type: "health", request_id: "health-probe", auth_token: token }) + "\n"));
    socket.on("data", (chunk) => {
      data += chunk;
      const line = data.split("\n", 1)[0];
      if (!line) return;
      clearTimeout(timer);
      socket.destroy();
      try {
        const response = JSON.parse(line);
        if (
          response.ok !== true ||
          response.state !== "listening" ||
          response.controller_state !== "ready"
        ) {
          reject(new Error(RUNTIME_ERROR_CODES.UNAVAILABLE));
        }
        else resolve(response);
      } catch { reject(new Error(RUNTIME_ERROR_CODES.MALFORMED)); }
    });
    socket.on("error", (error) => { clearTimeout(timer); reject(error); });
  });
}

async function runCli() {
  const socketPath = process.env.CHARIOX_BROWSER_RUNTIME_MCP_SOCKET || DEFAULT_SOCKET_PATH;
  const authFile = process.env.CHARIOX_BROWSER_RUNTIME_MCP_AUTH_FILE || DEFAULT_AUTH_FILE;
  if (process.argv[2] === "health") {
    await probe(socketPath, authFile);
    return;
  }
  const service = createProductionBrowserRuntimeMcpService({ socketPath, authFile });
  await service.start();
  const readyFile = process.env.CHARIOX_BROWSER_RUNTIME_MCP_READY_FILE;
  if (readyFile) await writeFile(readyFile, JSON.stringify({ socket: socketPath }) + "\n", { mode: 0o600 });
  let stopping = false;
  const stop = () => {
    if (stopping) return;
    stopping = true;
    service.stop().finally(() => process.exit(0));
  };
  process.once("SIGTERM", stop);
  process.once("SIGINT", stop);
  process.once("SIGHUP", stop);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}

export { BrowserRuntimeSemanticPort, RUNTIME_ERROR_CODES, domScript };
