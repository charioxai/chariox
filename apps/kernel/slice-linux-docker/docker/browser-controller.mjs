import { spawn as nodeSpawn } from "node:child_process";
import { createInterface } from "node:readline";
import { randomUUID } from "node:crypto";
import { createRequire } from "node:module";
import { performance } from "node:perf_hooks";
import { pathToFileURL } from "node:url";

const browserControllerRequire = createRequire(
  process.env.CHARIOX_BROWSER_CONTROLLER_PACKAGE_JSON ??
    new URL("../toolchain/package.json", import.meta.url),
);
const NodeWebSocket = browserControllerRequire("ws");

import { BrowserTabRegistry } from "./browser-tab-registry.mjs";
import {
  ACTION_ERROR_CODES,
  BrowserActionError,
  performBrowserAction,
} from "./browser-controller-actions.mjs";
import {
  BrowserObservationError,
  BrowserObservationStore,
  OBSERVATION_ERROR_CODES,
  OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES,
} from "./browser-controller-observations.mjs";
import {
  BrowserPageFeatureError,
  BrowserPageFeatures,
  PAGE_FEATURE_ERROR_CODES,
} from "./browser-controller-page-features.mjs";
import {
  BrowserMutationCoordinator,
  BrowserMutationError,
  MUTATION_ERROR_CODES,
} from "./browser-controller-mutation-coordinator.mjs";

export const CONTROLLER_STATES = Object.freeze([
  "idle",
  "starting",
  "ready",
  "stopping",
  "stopped",
  "fatal",
]);

export const ERROR_CODES = Object.freeze({
  SCHEMA_INVALID: "SCHEMA_INVALID",
  REQUEST_TOO_LARGE: "REQUEST_TOO_LARGE",
  DUPLICATE_REQUEST_ID: "DUPLICATE_REQUEST_ID",
  OWNER_MISMATCH: "OWNER_MISMATCH",
  OWNER_REQUIRED: "OWNER_REQUIRED",
  ALREADY_RUNNING: "ALREADY_RUNNING",
  CONTROLLER_BUSY: "CONTROLLER_BUSY",
  RESTART_REQUIRED: "RESTART_REQUIRED",
  STALE_GENERATION: "STALE_GENERATION",
  CONTROLLER_NOT_READY: "CONTROLLER_NOT_READY",
  QUEUE_SATURATED: "QUEUE_SATURATED",
  STARTUP_TIMEOUT: "STARTUP_TIMEOUT",
  PROCESS_SPAWN_FAILED: "PROCESS_SPAWN_FAILED",
  CONTROLLER_CRASHED: "CONTROLLER_CRASHED",
  PROCESS_ERROR: "PROCESS_ERROR",
  CDP_UNAVAILABLE: "CDP_UNAVAILABLE",
  CDP_PROTOCOL_INVALID: "CDP_PROTOCOL_INVALID",
  CDP_CONNECT_TIMEOUT: "CDP_CONNECT_TIMEOUT",
  CDP_CONNECT_FAILED: "CDP_CONNECT_FAILED",
  CDP_DISCONNECTED: "CDP_DISCONNECTED",
  CDP_COMMAND_TIMEOUT: "CDP_COMMAND_TIMEOUT",
  CDP_COMMAND_FAILED: "CDP_COMMAND_FAILED",
  STALE_DOCUMENT: "STALE_DOCUMENT",
  ELEMENT_REFERENCE_INVALIDATED: "ELEMENT_REFERENCE_INVALIDATED",
  FRAME_UNSUPPORTED: "FRAME_UNSUPPORTED",
  SNAPSHOT_TOO_LARGE: "SNAPSHOT_TOO_LARGE",
  ACTION_INVALID: "ACTION_INVALID",
  ACTION_TIMEOUT: "ACTION_TIMEOUT",
  ACTION_FAILED: "ACTION_FAILED",
  PAGE_FEATURE_INVALID: "PAGE_FEATURE_INVALID",
  PAGE_FEATURE_TIMEOUT: "PAGE_FEATURE_TIMEOUT",
  PAGE_FEATURE_FAILED: "PAGE_FEATURE_FAILED",
  PAGE_FEATURE_PATH_DENIED: "PAGE_FEATURE_PATH_DENIED",
  ACTION_POST_ACTION_UNCERTAIN: "ACTION_POST_ACTION_UNCERTAIN",
  REQUEST_CANCELLED: "REQUEST_CANCELLED",
  OUTPUT_TOO_LARGE: "OUTPUT_TOO_LARGE",
  INTERNAL_ERROR: "INTERNAL_ERROR",
});

const ERROR_MESSAGES = Object.freeze({
  [ERROR_CODES.SCHEMA_INVALID]: "request schema is invalid",
  [ERROR_CODES.REQUEST_TOO_LARGE]: "request exceeds the byte limit",
  [ERROR_CODES.DUPLICATE_REQUEST_ID]: "request_id has already been used",
  [ERROR_CODES.OWNER_MISMATCH]: "controller is owned by another owner",
  [ERROR_CODES.OWNER_REQUIRED]: "owner_id is required",
  [ERROR_CODES.ALREADY_RUNNING]: "controller is already running",
  [ERROR_CODES.CONTROLLER_BUSY]: "controller is busy",
  [ERROR_CODES.RESTART_REQUIRED]: "explicit restart is required",
  [ERROR_CODES.STALE_GENERATION]: "request generation is stale",
  [ERROR_CODES.CONTROLLER_NOT_READY]: "controller is not ready",
  [ERROR_CODES.QUEUE_SATURATED]: "controller request queue is full",
  [ERROR_CODES.STARTUP_TIMEOUT]: "browser startup timed out",
  [ERROR_CODES.PROCESS_SPAWN_FAILED]: "browser process could not start",
  [ERROR_CODES.CONTROLLER_CRASHED]: "browser controller entered fatal state",
  [ERROR_CODES.PROCESS_ERROR]: "browser process failed",
  [ERROR_CODES.CDP_UNAVAILABLE]: "CDP is unavailable",
  [ERROR_CODES.CDP_PROTOCOL_INVALID]: "CDP response was invalid",
  [ERROR_CODES.CDP_CONNECT_TIMEOUT]: "CDP connection timed out",
  [ERROR_CODES.CDP_CONNECT_FAILED]: "CDP connection failed",
  [ERROR_CODES.CDP_DISCONNECTED]: "CDP disconnected",
  [ERROR_CODES.CDP_COMMAND_TIMEOUT]: "CDP command timed out",
  [ERROR_CODES.CDP_COMMAND_FAILED]: "CDP command failed",
  [ERROR_CODES.STALE_DOCUMENT]: "browser document changed during observation",
  [ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED]: "element reference is no longer valid",
  [ERROR_CODES.FRAME_UNSUPPORTED]: "element actions inside child frames are not supported yet",
  [ERROR_CODES.SNAPSHOT_TOO_LARGE]: "browser observation exceeds the byte limit",
  [ERROR_CODES.ACTION_INVALID]: "browser action is invalid",
  [ERROR_CODES.ACTION_TIMEOUT]: "browser action timed out",
  [ERROR_CODES.ACTION_FAILED]: "browser action failed",
  [ERROR_CODES.PAGE_FEATURE_INVALID]: "browser page feature request is invalid",
  [ERROR_CODES.PAGE_FEATURE_TIMEOUT]: "browser page feature timed out",
  [ERROR_CODES.PAGE_FEATURE_FAILED]: "browser page feature failed",
  [ERROR_CODES.PAGE_FEATURE_PATH_DENIED]: "browser file path is not allowed",
  [ERROR_CODES.ACTION_POST_ACTION_UNCERTAIN]: "browser action outcome is uncertain after a post-action failure",
  [ERROR_CODES.REQUEST_CANCELLED]: "request was cancelled",
  [ERROR_CODES.OUTPUT_TOO_LARGE]: "response exceeds the byte limit",
  [ERROR_CODES.INTERNAL_ERROR]: "internal controller error",
});

const CDP_LIST_ENDPOINT = "http://127.0.0.1:9222/json/list";
const DEFAULT_EXECUTABLE = "chromium";
const DEFAULT_ARGS = Object.freeze([
  "--remote-debugging-address=127.0.0.1",
  "--remote-debugging-port=9222",
  "--no-first-run",
  "--no-default-browser-check",
  "--disable-background-networking",
  "about:blank",
]);

const MAX_REQUEST_BYTES = 64 * 1024;
const MAX_OUTPUT_BYTES = 64 * 1024;
const MAX_CDP_BODY_BYTES = 64 * 1024;
const MAX_CDP_FRAME_BYTES = 64 * 1024;
const CDP_WEBSOCKET_OPTIONS = Object.freeze({
  // The ws receiver applies maxPayload while consuming continuation frames,
  // before it assembles and emits a message. Native WebSocket implementations
  // ignore the extra constructor arguments and retain the application check.
  maxPayload: OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES,
});
const LARGE_CDP_RESPONSE_METHODS = new Set([
  "Accessibility.getFullAXTree",
  "DOMSnapshot.captureSnapshot",
]);
const MAX_IDENTIFIER_BYTES = 128;
const MAX_PENDING_CDP = 32;
const MAX_SEEN_REQUEST_IDS = 1024;
const SAFE_OPERATION = "health_probe";
const DEFAULT_ACTION_TIMEOUT_MS = 5_000;
const MIN_ACTION_TIMEOUT_MS = 100;
const MAX_ACTION_TIMEOUT_MS = 5_000;

export class ControllerError extends Error {
  constructor(code, message = ERROR_MESSAGES[code] || ERROR_MESSAGES[ERROR_CODES.INTERNAL_ERROR]) {
    super(message);
    this.name = "ControllerError";
    this.code = code;
  }
}

export function redactDiagnostic(value) {
  const input = String(value ?? "");
  const redacted = input.replace(
    /((?:authorization|cookie|credential|password|passwd|secret|token|api[_-]?key|access[_-]?key)\s*[:=]\s*)(?:"[^"]*"|'[^']*'|[^\s,;]+)/gi,
    "$1[REDACTED]",
  );
  const bounded = redacted.length > 256 ? redacted.slice(0, 256) + "…" : redacted;
  return bounded.replace(/[\u0000-\u001f\u007f]/g, " ");
}

function controllerError(code) {
  return new ControllerError(code);
}

function isPlainObject(value) {
  if (value === null || typeof value !== "object") {
    return false;
  }
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function schemaError() {
  throw controllerError(ERROR_CODES.SCHEMA_INVALID);
}

function assertPlainObject(value) {
  if (!isPlainObject(value)) {
    schemaError();
  }
}

function assertExactKeys(value, allowed, required = []) {
  assertPlainObject(value);
  const allowedSet = new Set(allowed);
  for (const key of Object.keys(value)) {
    if (!allowedSet.has(key)) {
      schemaError();
    }
  }
  for (const key of required) {
    if (!Object.prototype.hasOwnProperty.call(value, key)) {
      schemaError();
    }
  }
}

function validateIdentifier(value, field) {
  if (typeof value !== "string" || value.length === 0 || Buffer.byteLength(value, "utf8") > MAX_IDENTIFIER_BYTES) {
    if (field === "owner_id") {
      throw controllerError(ERROR_CODES.OWNER_REQUIRED);
    }
    schemaError();
  }
  if (!/^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(value)) {
    schemaError();
  }
  return value;
}

function isValidTabTargetId(value) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    Buffer.byteLength(value, "utf8") <= MAX_IDENTIFIER_BYTES &&
    /^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(value)
  );
}

function validateGeneration(value) {
  if (!Number.isSafeInteger(value) || value < 1) {
    schemaError();
  }
  return value;
}

function normalizeActionTimeout(value) {
  const requested = value ?? DEFAULT_ACTION_TIMEOUT_MS;
  return Math.max(MIN_ACTION_TIMEOUT_MS, Math.min(requested, MAX_ACTION_TIMEOUT_MS));
}

function boundedJson(value, limit) {
  let encoded;
  try {
    encoded = JSON.stringify(value);
  } catch {
    throw controllerError(ERROR_CODES.SCHEMA_INVALID);
  }
  if (typeof encoded !== "string" || Buffer.byteLength(encoded, "utf8") > limit) {
    throw controllerError(ERROR_CODES.REQUEST_TOO_LARGE);
  }
  return encoded;
}

function defaultClock() {
  let last = 0;
  return {
    now() {
      const current = Number.isFinite(performance.now()) ? performance.now() : last;
      last = Math.max(last, current);
      return last;
    },
  };
}

function defaultTimers() {
  return {
    setTimeout,
    clearTimeout,
    setInterval,
    clearInterval,
    sleep(milliseconds) {
      return new Promise((resolve) => setTimeout(resolve, milliseconds));
    },
  };
}

function addEventListener(target, eventName, listener) {
  if (typeof target?.addEventListener === "function") {
    target.addEventListener(eventName, listener);
  } else if (typeof target?.on === "function") {
    target.on(eventName, listener);
  }
}

function removeEventListener(target, eventName, listener) {
  if (typeof target?.removeEventListener === "function") {
    target.removeEventListener(eventName, listener);
  } else if (typeof target?.off === "function") {
    target.off(eventName, listener);
  } else if (typeof target?.removeListener === "function") {
    target.removeListener(eventName, listener);
  }
}

function defaultSpawnBrowser(executable, args) {
  const child = nodeSpawn(executable, args, {
    shell: false,
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout?.resume();
  child.stderr?.resume();
  return child;
}

function normalizeError(error, fallback = ERROR_CODES.INTERNAL_ERROR) {
  if (error instanceof ControllerError) {
    return error;
  }
  if (error instanceof BrowserActionError) {
    return controllerError(
      Object.values(ACTION_ERROR_CODES).includes(error.code)
        ? error.code
        : ERROR_CODES.ACTION_FAILED,
    );
  }
  if (error instanceof BrowserObservationError && error.code in ERROR_MESSAGES) {
    return controllerError(error.code);
  }
  if (error instanceof BrowserObservationError) {
    return controllerError(
      error.code === OBSERVATION_ERROR_CODES.INVALID_ARGUMENT
        ? ERROR_CODES.SCHEMA_INVALID
        : ERROR_CODES.CDP_PROTOCOL_INVALID,
    );
  }
  if (error instanceof BrowserPageFeatureError) {
    return controllerError(
      Object.values(PAGE_FEATURE_ERROR_CODES).includes(error.code)
        ? error.code
        : ERROR_CODES.PAGE_FEATURE_FAILED,
    );
  }
  return controllerError(fallback);
}

function isOpenState(socket) {
  return socket?.readyState === 1 || socket?.readyState === socket?.OPEN;
}

function createCdpWebSocket(WebSocketImpl, url) {
  try {
    // ws accepts its connection options as the second argument and enforces
    // maxPayload while it consumes fragmented frames.
    return new WebSocketImpl(url, CDP_WEBSOCKET_OPTIONS);
  } catch {
    // The WHATWG constructor treats the second argument as protocols. Keep
    // the same option available to implementations that accept a third
    // options argument, while its message boundary remains guarded below.
    return new WebSocketImpl(url, [], CDP_WEBSOCKET_OPTIONS);
  }
}

function validateCdpResultMessage(value) {
  if (!isPlainObject(value) || !Number.isSafeInteger(value.id) || value.id < 1) {
    throw controllerError(ERROR_CODES.CDP_PROTOCOL_INVALID);
  }
  if (Object.prototype.hasOwnProperty.call(value, "error") && !isPlainObject(value.error)) {
    throw controllerError(ERROR_CODES.CDP_PROTOCOL_INVALID);
  }
  if (
    Object.prototype.hasOwnProperty.call(value, "result") &&
    !isPlainObject(value.result)
  ) {
    throw controllerError(ERROR_CODES.CDP_PROTOCOL_INVALID);
  }
}

class CdpConnection {
  constructor({ socket, timers, commandTimeoutMs, onDisconnect }) {
    this.socket = socket;
    this.timers = timers;
    this.commandTimeoutMs = commandTimeoutMs;
    this.onDisconnect = onDisconnect;
    this.nextId = 1;
    this.pending = new Map();
    this.opened = isOpenState(socket);
    this.closed = false;
    this.expectedClose = false;
    this.openResolve = null;
    this.openReject = null;
    this.openTimer = null;
    this.openSettled = this.opened;
    this.boundOpen = () => this._onOpen();
    this.boundError = () => this._onError();
    this.boundClose = () => this._onClose();
    this.boundMessage = (event) => this._onMessage(event);

    addEventListener(socket, "open", this.boundOpen);
    addEventListener(socket, "error", this.boundError);
    addEventListener(socket, "close", this.boundClose);
    addEventListener(socket, "message", this.boundMessage);
  }

  async open(timeoutMs) {
    if (this.opened) {
      return;
    }
    if (this.closed) {
      throw controllerError(ERROR_CODES.CDP_CONNECT_FAILED);
    }
    await new Promise((resolve, reject) => {
      this.openResolve = resolve;
      this.openReject = reject;
      this.openTimer = this.timers.setTimeout(() => {
        this.openTimer = null;
        this.openSettled = true;
        this.openReject = null;
        reject(controllerError(ERROR_CODES.CDP_CONNECT_TIMEOUT));
        this._closeSocket();
      }, timeoutMs);
    });
  }

  async send(method, params = {}, timeoutMs = this.commandTimeoutMs, options = {}) {
    if (this.closed || !this.opened) {
      throw controllerError(ERROR_CODES.CDP_DISCONNECTED);
    }
    if (this.pending.size >= MAX_PENDING_CDP) {
      throw controllerError(ERROR_CODES.QUEUE_SATURATED);
    }
    if (typeof method !== "string" || !/^[A-Za-z][A-Za-z0-9_.-]{1,127}$/.test(method)) {
      throw controllerError(ERROR_CODES.SCHEMA_INVALID);
    }
    if (!isPlainObject(params)) {
      throw controllerError(ERROR_CODES.SCHEMA_INVALID);
    }
    if (!isPlainObject(options)) {
      throw controllerError(ERROR_CODES.SCHEMA_INVALID);
    }
    const maxResponseBytes = options.maxResponseBytes ?? MAX_CDP_FRAME_BYTES;
    if (
      !Number.isSafeInteger(maxResponseBytes) ||
      maxResponseBytes < MAX_CDP_FRAME_BYTES ||
      maxResponseBytes > OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES ||
      (maxResponseBytes > MAX_CDP_FRAME_BYTES && !LARGE_CDP_RESPONSE_METHODS.has(method))
    ) {
      throw controllerError(ERROR_CODES.SCHEMA_INVALID);
    }
    const id = this.nextId++;
    const message = { id, method, params };
    const encoded = boundedJson(message, MAX_CDP_FRAME_BYTES);
    return new Promise((resolve, reject) => {
      const timer = this.timers.setTimeout(() => {
        this.pending.delete(id);
        reject(controllerError(ERROR_CODES.CDP_COMMAND_TIMEOUT));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer, maxResponseBytes });
      try {
        this.socket.send(encoded);
      } catch {
        this.pending.delete(id);
        this.timers.clearTimeout(timer);
        reject(controllerError(ERROR_CODES.CDP_DISCONNECTED));
      }
    });
  }

  close() {
    if (this.closed) {
      return;
    }
    this.expectedClose = true;
    this._closeSocket();
    this._failPending(ERROR_CODES.CDP_DISCONNECTED);
  }

  _onOpen() {
    if (this.closed) {
      return;
    }
    this.opened = true;
    if (!this.openSettled) {
      this.openSettled = true;
      if (this.openTimer !== null) {
        this.timers.clearTimeout(this.openTimer);
        this.openTimer = null;
      }
      const resolve = this.openResolve;
      this.openResolve = null;
      this.openReject = null;
      resolve?.();
    }
  }

  _onError() {
    if (!this.opened && !this.openSettled) {
      this.openSettled = true;
      if (this.openTimer !== null) {
        this.timers.clearTimeout(this.openTimer);
        this.openTimer = null;
      }
      const reject = this.openReject;
      this.openResolve = null;
      this.openReject = null;
      reject?.(controllerError(ERROR_CODES.CDP_CONNECT_FAILED));
      this._closeSocket();
      return;
    }
    this._failConnection(ERROR_CODES.CDP_DISCONNECTED);
  }

  _onClose() {
    const wasExpected = this.expectedClose;
    this.closed = true;
    this.opened = false;
    if (!this.openSettled) {
      this.openSettled = true;
      if (this.openTimer !== null) {
        this.timers.clearTimeout(this.openTimer);
        this.openTimer = null;
      }
      const reject = this.openReject;
      this.openResolve = null;
      this.openReject = null;
      reject?.(controllerError(ERROR_CODES.CDP_CONNECT_FAILED));
    }
    this._failPending(ERROR_CODES.CDP_DISCONNECTED);
    if (!wasExpected) {
      this.onDisconnect?.();
    }
  }

  _onMessage(event) {
    const data = event?.data;
    const encodedBytes = typeof data === "string"
      ? Buffer.byteLength(data, "utf8")
      : data instanceof Uint8Array
        ? data.byteLength
        : null;
    if (encodedBytes === null || encodedBytes > OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES) {
      this._failConnection(ERROR_CODES.CDP_PROTOCOL_INVALID);
      return;
    }

    // Check the raw payload before any binary-to-text conversion or JSON parse.
    const encoded = typeof data === "string"
      ? data
      : Buffer.from(data.buffer, data.byteOffset, data.byteLength).toString("utf8");
    let message;
    try {
      message = JSON.parse(encoded);
      validateCdpResultMessage(message);
    } catch (error) {
      const normalized = normalizeError(error, ERROR_CODES.CDP_PROTOCOL_INVALID);
      this._failConnection(normalized.code);
      return;
    }
    const pending = this.pending.get(message.id);
    if (encodedBytes > (pending?.maxResponseBytes ?? MAX_CDP_FRAME_BYTES)) {
      this._failConnection(ERROR_CODES.CDP_PROTOCOL_INVALID);
      return;
    }
    if (!pending) {
      return;
    }
    this.pending.delete(message.id);
    this.timers.clearTimeout(pending.timer);
    if (message.error) {
      pending.reject(controllerError(ERROR_CODES.CDP_COMMAND_FAILED));
    } else {
      pending.resolve(message.result || {});
    }
  }

  _failPending(code) {
    const error = controllerError(code);
    for (const pending of this.pending.values()) {
      this.timers.clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
  }

  _failConnection(code) {
    const expected = this.expectedClose;
    this._failPending(code);
    this._closeSocket();
    if (!expected) {
      this.onDisconnect?.();
    }
  }

  _closeSocket() {
    if (this.closed) {
      return;
    }
    this.expectedClose = true;
    try {
      this.socket.close?.();
    } catch {
      // The socket is already unusable; pending calls are failed below.
    }
    this.closed = true;
    this.opened = false;
    removeEventListener(this.socket, "open", this.boundOpen);
    removeEventListener(this.socket, "error", this.boundError);
    removeEventListener(this.socket, "close", this.boundClose);
    removeEventListener(this.socket, "message", this.boundMessage);
  }
}

export class BrowserController {
  constructor(options = {}) {
    this.controllerId = validateIdentifier(
      options.controllerId || "controller-" + randomUUID(),
      "controller_id",
    );
    this.clock = options.clock || defaultClock();
    this.timers = options.timers || defaultTimers();
    this.fetchImpl = options.fetchImpl || globalThis.fetch;
    this.WebSocketImpl = options.WebSocketImpl || NodeWebSocket;
    this.spawnBrowser = options.spawnBrowser || defaultSpawnBrowser;
    this.signalProcess = options.signalProcess || ((child, signal) => child.kill?.(signal));
    this.executable = options.executable || DEFAULT_EXECUTABLE;
    this.args = Object.freeze([...(options.args || DEFAULT_ARGS)]);
    this.startupTimeoutMs = options.startupTimeoutMs ?? 5000;
    this.pollIntervalMs = options.pollIntervalMs ?? 100;
    this.cdpConnectTimeoutMs = options.cdpConnectTimeoutMs ?? 1000;
    this.cdpCommandTimeoutMs = options.cdpCommandTimeoutMs ?? 1000;
    this.shutdownGraceMs = options.shutdownGraceMs ?? 1000;
    this.terminateGraceMs = options.terminateGraceMs ?? 500;
    this.killGraceMs = options.killGraceMs ?? 250;
    this.heartbeatIntervalMs = options.heartbeatIntervalMs ?? 1000;
    this.queueLimit = options.queueLimit ?? 8;
    this.state = "idle";
    this.ownerId = null;
    this.generation = 0;
    this.fatalCode = null;
    this._listeners = new Set();
    this._process = null;
    this._cdp = null;
    this.tabRegistry = options.tabRegistry ?? new BrowserTabRegistry();
    this.mutationCoordinator = options.mutationCoordinator ?? new BrowserMutationCoordinator({
      maxTabs: options.mutationMaxTabs,
      maxQueuedPerTab: options.mutationMaxQueuedPerTab,
      maxCompleted: options.mutationMaxCompleted,
      maxInvalidatedTabs: options.mutationMaxInvalidatedTabs,
    });
    if (
      typeof this.mutationCoordinator.mutate !== "function" ||
      typeof this.mutationCoordinator.cancelAction !== "function" ||
      typeof this.mutationCoordinator.invalidateTab !== "function" ||
      typeof this.mutationCoordinator.advanceBrowserGeneration !== "function" ||
      typeof this.mutationCoordinator.snapshot !== "function"
    ) {
      throw new TypeError("mutationCoordinator must expose the controller mutation seam");
    }
    this.observationStore = options.observationStore ?? new BrowserObservationStore();
    this.pageFeatures = options.pageFeatures ?? new BrowserPageFeatures({
      uploadRoots: options.uploadRoots ?? [],
      downloadRoot: options.downloadRoot ?? null,
      uploadArtifactBroker: options.uploadArtifactBroker ?? null,
      now: () => this._now(),
      sleep: (milliseconds) => this.timers.sleep(milliseconds),
    });
    this._targetConnections = new Map();
    this._queue = [];
    this._queueActive = false;
    this._heartbeatTimer = null;
    this._heartbeatSeq = 0;
    this._lastHeartbeatMs = this._now();
    this._assertOptions();
  }

  onEvent(listener) {
    if (typeof listener !== "function") {
      throw new TypeError("listener must be a function");
    }
    this._listeners.add(listener);
    return () => this._listeners.delete(listener);
  }

  health(ownerId, expectedGeneration) {
    if (ownerId !== undefined) {
      this._admitOwner(ownerId);
    }
    if (expectedGeneration !== undefined) {
      this._requireGeneration(expectedGeneration);
    }
    this._touchHeartbeat();
    return {
      controller_id: this.controllerId,
      owner_id: this.ownerId,
      generation: this.generation,
      state: this.state,
      heartbeat_seq: this._heartbeatSeq,
      last_heartbeat_ms: this._lastHeartbeatMs,
      queue_depth: this._queue.length + (this._queueActive ? 1 : 0),
      cdp_connected: Boolean(this._cdp && !this._cdp.closed),
      fatal_code: this.fatalCode,
    };
  }

  async start(ownerId) {
    this._admitOwner(ownerId);
    if (this.state === "ready") {
      throw controllerError(ERROR_CODES.ALREADY_RUNNING);
    }
    if (this.state === "starting" || this.state === "stopping") {
      throw controllerError(ERROR_CODES.CONTROLLER_BUSY);
    }
    if (this.state === "fatal") {
      throw controllerError(ERROR_CODES.RESTART_REQUIRED);
    }

    if (typeof this.pageFeatures.prepare === "function") {
      try {
        await this.pageFeatures.prepare();
      } catch (error) {
        throw normalizeError(error, ERROR_CODES.PAGE_FEATURE_PATH_DENIED);
      }
    }

    this.generation += 1;
    this._advanceMutationBrowserGeneration(this.generation);
    this.state = "starting";
    this.fatalCode = null;
    let record;
    try {
      record = await this._spawnProcess();
    } catch (error) {
      const normalized = normalizeError(error, ERROR_CODES.PROCESS_SPAWN_FAILED);
      this._setFatal(normalized.code);
      throw normalized;
    }
    this._process = record;

    try {
      const connected = await this._connectUntilReady(record);
      if (
        this._process?.token !== record.token ||
        record.exited ||
        this.state !== "starting"
      ) {
        connected.connection.close();
        throw controllerError(ERROR_CODES.CONTROLLER_CRASHED);
      }
      this._cdp = connected.connection;
      await this._reconcileTabRegistry(this.generation, connected.targets);
      this.state = "ready";
      this._startHeartbeat();
      this._emit("ready", {
        generation: this.generation,
        heartbeat_seq: this._heartbeatSeq,
      });
      return this.health();
    } catch (error) {
      const normalized = normalizeError(error, ERROR_CODES.STARTUP_TIMEOUT);
      await this._abortStart(record);
      if (this.state !== "fatal") {
        this._setFatal(normalized.code);
      }
      throw controllerError(this.fatalCode || normalized.code);
    }
  }

  async shutdown(ownerId, expectedGeneration) {
    this._admitOwner(ownerId);
    if (expectedGeneration !== undefined) {
      this._requireGeneration(expectedGeneration);
    }
    return this._shutdownInternal("shutdown");
  }

  async restart(ownerId, expectedGeneration) {
    this._admitOwner(ownerId);
    if (expectedGeneration !== undefined) {
      this._requireGeneration(expectedGeneration);
    }
    if (this.state === "starting" || this.state === "stopping") {
      throw controllerError(ERROR_CODES.CONTROLLER_BUSY);
    }
    await this._shutdownInternal("restart");
    return this.start(ownerId);
  }

  async shutdownForSignal() {
    return this._shutdownInternal("signal");
  }

  async execute(ownerId, expectedGeneration, operation) {
    this._admitOwner(ownerId);
    this._requireGeneration(expectedGeneration);
    if (operation !== SAFE_OPERATION) {
      throw controllerError(ERROR_CODES.SCHEMA_INVALID);
    }
    if (this.state !== "ready" || !this._cdp) {
      throw controllerError(ERROR_CODES.CONTROLLER_NOT_READY);
    }
    const generation = this.generation;
    const cdp = this._cdp;
    return this._enqueue(generation, async () => {
      if (this.state !== "ready" || this.generation !== generation || this._cdp !== cdp) {
        throw controllerError(ERROR_CODES.STALE_GENERATION);
      }
      await cdp.send("Browser.getVersion", {}, this.cdpCommandTimeoutMs);
      return {
        operation: SAFE_OPERATION,
        generation,
        cdp_connected: true,
      };
    });
  }

  async refreshTabs(ownerId, expectedGeneration, options = {}) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    if (!isPlainObject(options) || Object.keys(options).some((key) => key !== "reconnect")) {
      schemaError();
    }
    if (options.reconnect !== undefined && typeof options.reconnect !== "boolean") {
      schemaError();
    }
    const generation = this.generation;
    const cdp = this._cdp;
    return this._enqueue(generation, async () => {
      const targets = await this._discoverTargets(this.cdpCommandTimeoutMs);
      if (targets === null) {
        throw controllerError(ERROR_CODES.CDP_UNAVAILABLE);
      }
      if (
        this.state !== "ready" ||
        this.generation !== generation ||
        this._cdp !== cdp ||
        cdp.closed
      ) {
        throw controllerError(ERROR_CODES.STALE_GENERATION);
      }
      return this._reconcileTabRegistry(generation, targets, options);
    });
  }

  getTabRegistrySnapshot(ownerId, expectedGeneration) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    return this.tabRegistry.snapshot();
  }

  // Internal runtime seam. The serialized controller protocol intentionally
  // does not expose mutation cancellation or attribution.
  cancelMutation(request) {
    assertExactKeys(request, ["action_id", "actor_id"], ["action_id", "actor_id"]);
    return this.mutationCoordinator.cancelAction({
      action_id: validateIdentifier(request.action_id, "action_id"),
      actor_id: validateIdentifier(request.actor_id, "actor_id"),
    });
  }

  claimViewport(ownerId, expectedGeneration, request) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    return this.tabRegistry.claimViewport(request);
  }

  resizeViewport(ownerId, expectedGeneration, request) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    return this.tabRegistry.resizeViewport(request);
  }

  async captureTabSnapshot(ownerId, expectedGeneration, request) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    assertExactKeys(request, ["tab_id", "target_generation", "limits"], [
      "tab_id",
      "target_generation",
    ]);
    const tabId = validateIdentifier(request.tab_id, "tab_id");
    const targetGeneration = validateGeneration(request.target_generation);
    if (request.limits !== undefined && !isPlainObject(request.limits)) {
      schemaError();
    }
    const generation = this.generation;
    return this._enqueue(generation, async () => {
      const target = this.tabRegistry.resolveTarget(tabId, {
        generation,
        target_generation: targetGeneration,
      });
      const connection = await this._ensureTargetConnection(target);
      try {
        const snapshot = await this.observationStore.capture({
          connection,
          tab: target,
          limits: request.limits,
        });
        if (typeof this.pageFeatures.observeDocument === "function") {
          await this.pageFeatures.observeDocument({
            tab_id: snapshot.tab_id,
            document_id: snapshot.document_id,
          });
        }
        return snapshot;
      } catch (error) {
        throw normalizeError(error);
      }
    });
  }

  resolveElementReference(ownerId, expectedGeneration, request) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    assertExactKeys(request, ["tab_id", "target_generation", "element_ref"], [
      "tab_id",
      "target_generation",
      "element_ref",
    ]);
    const tabId = validateIdentifier(request.tab_id, "tab_id");
    const targetGeneration = validateGeneration(request.target_generation);
    const elementRef = validateIdentifier(request.element_ref, "element_ref");
    const generation = this.generation;
    return this._enqueue(generation, async () => {
      const target = this.tabRegistry.resolveTarget(tabId, {
        generation,
        target_generation: targetGeneration,
      });
      const connection = await this._ensureTargetConnection(target);
      try {
        return await this.observationStore.resolve(target, elementRef, { connection });
      } catch (error) {
        throw normalizeError(error);
      }
    });
  }

  async performElementAction(ownerId, expectedGeneration, request, mutation) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    assertExactKeys(
      request,
      ["tab_id", "target_generation", "element_ref", "action", "timeout_ms"],
      ["tab_id", "target_generation", "element_ref", "action"],
    );
    const tabId = validateIdentifier(request.tab_id, "tab_id");
    const targetGeneration = validateGeneration(request.target_generation);
    const elementRef = validateIdentifier(request.element_ref, "element_ref");
    if (!isPlainObject(request.action)) schemaError();
    if (
      request.timeout_ms !== undefined &&
      (!Number.isSafeInteger(request.timeout_ms) || request.timeout_ms < 1)
    ) {
      schemaError();
    }
    const generation = this.generation;
    const actionTimeoutMs = normalizeActionTimeout(request.timeout_ms);
    const actionDeadline = this._now() + actionTimeoutMs;
    const attribution = this._createTabMutationAttribution(
      mutation,
      "perform_element_action",
      tabId,
      targetGeneration,
      generation,
    );
    return this._enqueueTabMutation(attribution, async ({ signal }) => {
      if (signal.aborted) {
        throw new BrowserMutationError(MUTATION_ERROR_CODES.INDETERMINATE);
      }
      if (actionDeadline - this._now() <= 0) {
        throw controllerError(ERROR_CODES.ACTION_TIMEOUT);
      }
      const target = this.tabRegistry.resolveTarget(tabId, {
        generation,
        target_generation: targetGeneration,
      });
      let element;
      try {
        element = this.observationStore.resolve(target, elementRef);
      } catch (error) {
        throw normalizeError(error);
      }
      const connection = await this._ensureTargetConnection(target);
      if (actionDeadline - this._now() <= 0) {
        throw controllerError(ERROR_CODES.ACTION_TIMEOUT);
      }
      try {
        const result = await performBrowserAction({
          connection,
          element,
          action: request.action,
          timeoutMs: actionTimeoutMs,
          deadline: actionDeadline,
          now: () => this._now(),
          sleep: (milliseconds) => this.timers.sleep(milliseconds),
        });
        if (signal.aborted) {
          throw new BrowserMutationError(MUTATION_ERROR_CODES.INDETERMINATE);
        }
        return result;
      } catch (error) {
        throw normalizeError(error);
      }
    }, {
      target_id: null,
      page_id: null,
      document_id: null,
      arguments: {
        element_ref: elementRef,
        action: request.action,
        timeout_ms: request.timeout_ms ?? null,
      },
      payload: null,
    });
  }

  async describeElementContext(ownerId, expectedGeneration, request) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    assertExactKeys(request, ["tab_id", "target_generation", "element_ref"], [
      "tab_id",
      "target_generation",
      "element_ref",
    ]);
    const tabId = validateIdentifier(request.tab_id, "tab_id");
    const targetGeneration = validateGeneration(request.target_generation);
    const elementRef = validateIdentifier(request.element_ref, "element_ref");
    const generation = this.generation;
    return this._enqueue(generation, async () => {
      const target = this.tabRegistry.resolveTarget(tabId, {
        generation,
        target_generation: targetGeneration,
      });
      let element;
      try {
        element = this.observationStore.resolve(target, elementRef);
      } catch (error) {
        throw normalizeError(error);
      }
      const connection = await this._ensureTargetConnection(target);
      try {
        return await this.pageFeatures.describeElement({ connection, element });
      } catch (error) {
        throw normalizeError(error);
      }
    });
  }

  async handleDialog(ownerId, expectedGeneration, request, mutation) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    assertExactKeys(request, ["tab_id", "target_generation", "dialog"], [
      "tab_id",
      "target_generation",
      "dialog",
    ]);
    const tabId = validateIdentifier(request.tab_id, "tab_id");
    const targetGeneration = validateGeneration(request.target_generation);
    if (!isPlainObject(request.dialog)) schemaError();
    const generation = this.generation;
    const attribution = this._createTabMutationAttribution(
      mutation,
      "handle_dialog",
      tabId,
      targetGeneration,
      generation,
    );
    return this._enqueueTabMutation(attribution, async ({ signal }) => {
      if (signal.aborted) {
        throw new BrowserMutationError(MUTATION_ERROR_CODES.INDETERMINATE);
      }
      const target = this.tabRegistry.resolveTarget(tabId, {
        generation,
        target_generation: targetGeneration,
      });
      const connection = await this._ensureTargetConnection(target);
      try {
        const result = await this.pageFeatures.handleDialog({ connection, dialog: request.dialog });
        if (signal.aborted) {
          throw new BrowserMutationError(MUTATION_ERROR_CODES.INDETERMINATE);
        }
        return result;
      } catch (error) {
        throw normalizeError(error);
      }
    }, {
      target_id: null,
      page_id: null,
      document_id: null,
      arguments: { dialog: request.dialog },
      payload: null,
    });
  }

  async waitForPopup(ownerId, expectedGeneration, request) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    assertExactKeys(request, ["known_tab_ids", "timeout_ms"], ["known_tab_ids"]);
    if (!Array.isArray(request.known_tab_ids)) schemaError();
    const knownTabIds = request.known_tab_ids.map((tabId) => validateIdentifier(tabId, "tab_id"));
    if (
      request.timeout_ms !== undefined &&
      (!Number.isSafeInteger(request.timeout_ms) || request.timeout_ms < 1)
    ) {
      schemaError();
    }
    const generation = this.generation;
    return this._enqueue(generation, async () => {
      try {
        return await this.pageFeatures.waitForPopup({
          known_tab_ids: knownTabIds,
          timeout_ms: request.timeout_ms,
          listTabs: async () => {
            const targets = await this._discoverTargets(this.cdpCommandTimeoutMs);
            if (targets === null) throw controllerError(ERROR_CODES.CDP_UNAVAILABLE);
            if (this.state !== "ready" || this.generation !== generation) {
              throw controllerError(ERROR_CODES.STALE_GENERATION);
            }
            return await this._reconcileTabRegistry(generation, targets);
          },
        });
      } catch (error) {
        throw normalizeError(error);
      }
    });
  }

  async configureDownloads(ownerId, expectedGeneration, request = {}) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    assertExactKeys(request, ["browser_context_id"]);
    const generation = this.generation;
    const connection = this._cdp;
    return this._enqueue(generation, async () => {
      try {
        return await this.pageFeatures.configureDownloads({
          connection,
          browser_context_id: request.browser_context_id,
        });
      } catch (error) {
        throw normalizeError(error);
      }
    });
  }

  async uploadFiles(ownerId, expectedGeneration, request, mutation) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    assertExactKeys(request, ["tab_id", "target_generation", "element_ref", "paths"], [
      "tab_id",
      "target_generation",
      "element_ref",
      "paths",
    ]);
    const tabId = validateIdentifier(request.tab_id, "tab_id");
    const targetGeneration = validateGeneration(request.target_generation);
    const elementRef = validateIdentifier(request.element_ref, "element_ref");
    if (!Array.isArray(request.paths)) schemaError();
    const generation = this.generation;
    const attribution = this._createTabMutationAttribution(
      mutation,
      "upload_files",
      tabId,
      targetGeneration,
      generation,
    );
    return this._enqueueTabMutation(attribution, async ({ signal }) => {
      if (signal.aborted) {
        throw new BrowserMutationError(MUTATION_ERROR_CODES.INDETERMINATE);
      }
      const target = this.tabRegistry.resolveTarget(tabId, {
        generation,
        target_generation: targetGeneration,
      });
      let element;
      try {
        element = this.observationStore.resolve(target, elementRef);
      } catch (error) {
        throw normalizeError(error);
      }
      const connection = await this._ensureTargetConnection(target);
      try {
        const result = await this.pageFeatures.uploadFiles({ connection, element, paths: request.paths });
        if (signal.aborted) {
          throw new BrowserMutationError(MUTATION_ERROR_CODES.INDETERMINATE);
        }
        return result;
      } catch (error) {
        throw normalizeError(error);
      }
    }, {
      target_id: null,
      page_id: null,
      document_id: null,
      arguments: { element_ref: elementRef },
      payload: { paths: request.paths },
    });
  }

  async grantPermissions(ownerId, expectedGeneration, request) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    assertExactKeys(request, ["origin", "permissions", "browser_context_id"], [
      "origin",
      "permissions",
    ]);
    const generation = this.generation;
    const connection = this._cdp;
    return this._enqueue(generation, async () => {
      try {
        return await this.pageFeatures.grantPermissions({ connection, ...request });
      } catch (error) {
        throw normalizeError(error);
      }
    });
  }

  async resetPermissions(ownerId, expectedGeneration, request = {}) {
    this._assertTabRegistryAccess(ownerId, expectedGeneration);
    assertExactKeys(request, ["browser_context_id"]);
    const generation = this.generation;
    const connection = this._cdp;
    return this._enqueue(generation, async () => {
      try {
        return await this.pageFeatures.resetPermissions({ connection, ...request });
      } catch (error) {
        throw normalizeError(error);
      }
    });
  }

  _assertOptions() {
    const positiveOptions = [
      this.startupTimeoutMs,
      this.pollIntervalMs,
      this.cdpConnectTimeoutMs,
      this.cdpCommandTimeoutMs,
      this.shutdownGraceMs,
      this.terminateGraceMs,
      this.killGraceMs,
      this.heartbeatIntervalMs,
      this.queueLimit,
    ];
    if (positiveOptions.some((value) => !Number.isSafeInteger(value) || value < 1)) {
      throw new TypeError("controller timing and queue options must be positive integers");
    }
    if (typeof this.fetchImpl !== "function" || typeof this.WebSocketImpl !== "function") {
      // The dependency is checked again on start so health can still report idle state.
      return;
    }
  }

  _now() {
    const value = Number(this.clock.now());
    if (!Number.isFinite(value)) {
      return this._lastHeartbeatMs || 0;
    }
    this._lastHeartbeatMs = Math.max(this._lastHeartbeatMs || 0, value);
    return this._lastHeartbeatMs;
  }

  _touchHeartbeat() {
    this._lastHeartbeatMs = Math.max(this._lastHeartbeatMs, this._now());
    this._heartbeatSeq += 1;
  }

  _startHeartbeat() {
    this._stopHeartbeat();
    this._heartbeatTimer = this.timers.setInterval(() => {
      if (this.state === "ready" || this.state === "starting") {
        this._touchHeartbeat();
        this._emit("heartbeat", {
          heartbeat_seq: this._heartbeatSeq,
          last_heartbeat_ms: this._lastHeartbeatMs,
        });
      }
    }, this.heartbeatIntervalMs);
  }

  _stopHeartbeat() {
    if (this._heartbeatTimer !== null) {
      this.timers.clearInterval(this._heartbeatTimer);
      this._heartbeatTimer = null;
    }
  }

  _emit(event, payload = {}) {
    const message = {
      type: "event",
      event,
      controller_id: this.controllerId,
      generation: this.generation,
      state: this.state,
      ...payload,
    };
    for (const listener of this._listeners) {
      try {
        listener(message);
      } catch {
        // Event consumers cannot change controller state.
      }
    }
  }

  _admitOwner(ownerId) {
    const validated = validateIdentifier(ownerId, "owner_id");
    if (this.ownerId === null) {
      this.ownerId = validated;
      return;
    }
    if (this.ownerId !== validated) {
      throw controllerError(ERROR_CODES.OWNER_MISMATCH);
    }
  }

  _requireGeneration(expectedGeneration) {
    if (!Number.isSafeInteger(expectedGeneration) || expectedGeneration < 1) {
      schemaError();
    }
    if (expectedGeneration !== this.generation) {
      throw controllerError(ERROR_CODES.STALE_GENERATION);
    }
  }

  _assertTabRegistryAccess(ownerId, expectedGeneration) {
    this._admitOwner(ownerId);
    this._requireGeneration(expectedGeneration);
    if (this.state !== "ready" || !this._cdp || this._cdp.closed) {
      throw controllerError(ERROR_CODES.CONTROLLER_NOT_READY);
    }
  }

  _advanceMutationBrowserGeneration(generation) {
    const current = this.mutationCoordinator.snapshot().browser_generation;
    if (current === null || current < generation) {
      this.mutationCoordinator.advanceBrowserGeneration(generation);
      return;
    }
    if (current !== generation) {
      throw controllerError(ERROR_CODES.STALE_GENERATION);
    }
  }

  _createTabMutationAttribution(mutation, operation, tabId, targetGeneration, generation) {
    assertExactKeys(mutation, ["action_id", "actor_id"], ["action_id", "actor_id"]);
    return {
      action_id: validateIdentifier(mutation.action_id, "action_id"),
      actor_id: validateIdentifier(mutation.actor_id, "actor_id"),
      browser_generation: generation,
      operation,
      tab_id: tabId,
      target_generation: targetGeneration,
    };
  }

  _enqueueTabMutation(attribution, run, identity) {
    return this.mutationCoordinator.mutate(attribution, async (context) => {
      if (context.signal.aborted) {
        throw new BrowserMutationError(MUTATION_ERROR_CODES.INDETERMINATE);
      }
      return run(context);
    }, identity);
  }

  _invalidateAllMutations() {
    const snapshot = this.mutationCoordinator.snapshot();
    const invalidated = new Set(snapshot.active.map((record) => record.tab_id));
    for (const queued of snapshot.queued) {
      invalidated.add(queued.tab_id);
    }
    for (const tabId of invalidated) {
      this.mutationCoordinator.invalidateTab(tabId, Number.MAX_SAFE_INTEGER);
    }
  }

  async _spawnProcess() {
    if (typeof this.spawnBrowser !== "function") {
      throw controllerError(ERROR_CODES.PROCESS_SPAWN_FAILED);
    }
    let child;
    try {
      child = await this.spawnBrowser(this.executable, [...this.args], {
        shell: false,
        stdio: ["ignore", "pipe", "pipe"],
      });
    } catch {
      throw controllerError(ERROR_CODES.PROCESS_SPAWN_FAILED);
    }
    if (!child || typeof child !== "object") {
      throw controllerError(ERROR_CODES.PROCESS_SPAWN_FAILED);
    }
    child.stdout?.resume?.();
    child.stderr?.resume?.();
    const token = Symbol("browser-process");
    let resolveExit;
    const exitPromise = new Promise((resolve) => {
      resolveExit = resolve;
    });
    const record = {
      child,
      token,
      expected: false,
      exited: false,
      exitPromise,
      resolveExit,
      exitInfo: null,
    };
    addEventListener(child, "exit", (code, signal) => {
      if (record.exited) {
        return;
      }
      record.exited = true;
      record.exitInfo = { code, signal };
      record.resolveExit(record.exitInfo);
      if (this._process?.token !== record.token || record.expected) {
        return;
      }
      this._setFatal(ERROR_CODES.CONTROLLER_CRASHED);
    });
    addEventListener(child, "error", () => {
      if (record.expected || record.exited || this._process?.token !== record.token) {
        return;
      }
      this._setFatal(ERROR_CODES.PROCESS_ERROR);
      record.expected = true;
      this._signal(record, "SIGKILL");
    });
    return record;
  }

  async _connectUntilReady(record) {
    if (typeof this.fetchImpl !== "function" || typeof this.WebSocketImpl !== "function") {
      throw controllerError(ERROR_CODES.CDP_UNAVAILABLE);
    }
    const deadline = this._now() + this.startupTimeoutMs;
    while (this._now() <= deadline) {
      if (
        record.exited ||
        this._process?.token !== record.token ||
        this.state !== "starting"
      ) {
        throw controllerError(ERROR_CODES.CONTROLLER_CRASHED);
      }
      let targets;
      try {
        targets = await this._discoverTargets(Math.max(1, deadline - this._now()));
      } catch (error) {
        const normalized = normalizeError(error, ERROR_CODES.CDP_PROTOCOL_INVALID);
        if (normalized.code === ERROR_CODES.CDP_PROTOCOL_INVALID) {
          throw normalized;
        }
        targets = null;
      }
      const target = targets?.[0] ?? null;
      if (target) {
        let connection;
        try {
          const socket = createCdpWebSocket(this.WebSocketImpl, target.webSocketDebuggerUrl);
          connection = new CdpConnection({
            socket,
            timers: this.timers,
            commandTimeoutMs: this.cdpCommandTimeoutMs,
            onDisconnect: () => this._onCdpDisconnect(),
          });
          await connection.open(this.cdpConnectTimeoutMs);
          await connection.send("Browser.getVersion", {}, this.cdpCommandTimeoutMs);
          return { connection, targets };
        } catch (error) {
          connection?.close();
          const normalized = normalizeError(error, ERROR_CODES.CDP_CONNECT_FAILED);
          if (this._now() >= deadline) {
            throw controllerError(ERROR_CODES.STARTUP_TIMEOUT);
          }
        }
      }
      const remaining = deadline - this._now();
      if (remaining <= 0) {
        break;
      }
      await this.timers.sleep(Math.min(this.pollIntervalMs, remaining));
    }
    throw controllerError(ERROR_CODES.STARTUP_TIMEOUT);
  }

  async _discoverTarget(timeoutMs = this.cdpCommandTimeoutMs) {
    const targets = await this._discoverTargets(timeoutMs);
    return targets?.[0]?.webSocketDebuggerUrl ?? null;
  }

  async _discoverTargets(timeoutMs = this.cdpCommandTimeoutMs) {
    const abortController = typeof AbortController === "function" ? new AbortController() : null;
    const abortTimer = abortController
      ? this.timers.setTimeout(() => abortController.abort(), Math.max(1, timeoutMs))
      : null;
    try {
      const response = await this.fetchImpl(CDP_LIST_ENDPOINT, {
        method: "GET",
        ...(abortController ? { signal: abortController.signal } : {}),
      });
      if (response?.ok === false) {
        return null;
      }
      let body;
      if (typeof response?.text === "function") {
        body = await response.text();
      } else if (typeof response?.json === "function") {
        body = JSON.stringify(await response.json());
      } else {
        throw controllerError(ERROR_CODES.CDP_PROTOCOL_INVALID);
      }
      if (typeof body !== "string" || Buffer.byteLength(body, "utf8") > MAX_CDP_BODY_BYTES) {
        throw controllerError(ERROR_CODES.CDP_PROTOCOL_INVALID);
      }
      let targets;
      try {
        targets = JSON.parse(body);
      } catch {
        throw controllerError(ERROR_CODES.CDP_PROTOCOL_INVALID);
      }
      if (!Array.isArray(targets)) {
        throw controllerError(ERROR_CODES.CDP_PROTOCOL_INVALID);
      }
      const validTargets = [];
      for (const target of targets) {
        if (!isPlainObject(target) || target.type !== "page" || typeof target.webSocketDebuggerUrl !== "string") {
          continue;
        }
        if (Buffer.byteLength(target.webSocketDebuggerUrl, "utf8") > 2048) {
          continue;
        }
        try {
          const url = new URL(target.webSocketDebuggerUrl);
          if (url.protocol !== "ws:" && url.protocol !== "wss:") {
            continue;
          }
        } catch {
          continue;
        }
        validTargets.push(target);
      }
      return validTargets;
    } catch (error) {
      if (error instanceof ControllerError) {
        throw error;
      }
      return null;
    } finally {
      if (abortTimer !== null) {
        this.timers.clearTimeout(abortTimer);
      }
    }
  }

  async _reconcileTabRegistry(generation, targets, options = {}) {
    const previousTabs = this.tabRegistry.listTabs();
    const registryTargets = targets.map((target) => {
      if (!isValidTabTargetId(target.id)) {
        throw controllerError(ERROR_CODES.CDP_PROTOCOL_INVALID);
      }
      return {
        target_id: target.id,
        target_type: target.type,
        websocket_url: target.webSocketDebuggerUrl,
      };
    });
    const result = this.tabRegistry.reconcile(generation, registryTargets, options);
    for (const detached of result.detached) {
      this.mutationCoordinator.invalidateTab(detached.tab_id, detached.target_generation);
    }
    this.observationStore.reconcile(result.tabs);
    this._reconcileTargetConnections(result.tabs);
    if (typeof this.pageFeatures.releaseUploadsForTab === "function") {
      const activeTabIds = new Set(result.tabs.map((tab) => tab.tab_id));
      for (const tab of previousTabs) {
        if (!activeTabIds.has(tab.tab_id)) {
          await this.pageFeatures.releaseUploadsForTab(tab.tab_id);
        }
      }
    }
    return result;
  }

  async _ensureTargetConnection(target) {
    const existing = this._targetConnections.get(target.tab_id);
    if (
      existing &&
      existing.generation === target.generation &&
      existing.targetGeneration === target.target_generation &&
      existing.websocketUrl === target.websocket_url &&
      !existing.connection.closed
    ) {
      return existing.connection;
    }
    existing?.connection.close();
    if (typeof target.websocket_url !== "string" || target.websocket_url.length === 0) {
      throw controllerError(ERROR_CODES.CDP_UNAVAILABLE);
    }
    let record;
    try {
      const connection = new CdpConnection({
        socket: createCdpWebSocket(this.WebSocketImpl, target.websocket_url),
        timers: this.timers,
        commandTimeoutMs: this.cdpCommandTimeoutMs,
        onDisconnect: () => {
          if (this._targetConnections.get(target.tab_id) === record) {
            this._targetConnections.delete(target.tab_id);
            this.observationStore.invalidate(target.tab_id);
            this.mutationCoordinator.invalidateTab(target.tab_id, target.target_generation);
            void Promise.resolve(this.pageFeatures.releaseUploadsForTab?.(target.tab_id)).catch(() => {});
          }
        },
      });
      record = {
        generation: target.generation,
        targetGeneration: target.target_generation,
        websocketUrl: target.websocket_url,
        connection,
      };
      this._targetConnections.set(target.tab_id, record);
      await connection.open(this.cdpConnectTimeoutMs);
      return connection;
    } catch (error) {
      record?.connection.close();
      if (this._targetConnections.get(target.tab_id) === record) {
        this._targetConnections.delete(target.tab_id);
      }
      throw normalizeError(error, ERROR_CODES.CDP_CONNECT_FAILED);
    }
  }

  _reconcileTargetConnections(activeTabs) {
    const activeByTabId = new Map(activeTabs.map((tab) => [tab.tab_id, tab]));
    for (const [tabId, record] of this._targetConnections) {
      const tab = activeByTabId.get(tabId);
      if (
        !tab ||
        tab.generation !== record.generation ||
        tab.target_generation !== record.targetGeneration
      ) {
        this.mutationCoordinator.invalidateTab(tabId, record.targetGeneration);
        record.connection.close();
        this._targetConnections.delete(tabId);
      }
    }
  }

  _closeTargetConnections() {
    for (const record of this._targetConnections.values()) {
      record.connection.close();
    }
    this._targetConnections.clear();
    this.observationStore.clear();
  }

  async _shutdownPageFeatures() {
    if (typeof this.pageFeatures.shutdown === "function") {
      await this.pageFeatures.shutdown();
    }
  }

  async _abortStart(record) {
    this._stopHeartbeat();
    this._closeTargetConnections();
    await this._shutdownPageFeatures();
    if (this._cdp) {
      this._cdp.close();
      this._cdp = null;
    }
    if (record && !record.exited) {
      record.expected = true;
      this._signal(record, "SIGKILL");
      await this._waitForExit(record, this.killGraceMs);
    }
    if (this._process?.token === record?.token) {
      this._process = null;
    }
  }

  async _shutdownInternal(reason) {
    this.state = "stopping";
    this._stopHeartbeat();
    this._invalidateAllMutations();
    this._closeTargetConnections();
    await this._shutdownPageFeatures();
    this._rejectQueued(controllerError(
      reason === "restart" ? ERROR_CODES.STALE_GENERATION : ERROR_CODES.REQUEST_CANCELLED,
    ));
    const record = this._process;
    if (record && !record.exited) {
      record.expected = true;
    }
    const connection = this._cdp;
    let browserCloseSent = false;
    if (connection && !connection.closed) {
      try {
        await connection.send("Browser.close", {}, this.cdpCommandTimeoutMs);
        browserCloseSent = true;
      } catch {
        // A crashed or already-closing browser is handled by the process fallback.
      }
      connection.close();
      this._cdp = null;
    }

    let forcedKill = false;
    if (record && !record.exited) {
      record.expected = true;
      let exited = await this._waitForExit(record, this.shutdownGraceMs);
      if (!exited) {
        this._signal(record, "SIGTERM");
        exited = await this._waitForExit(record, this.terminateGraceMs);
      }
      if (!exited) {
        forcedKill = true;
        this._signal(record, "SIGKILL");
        await this._waitForExit(record, this.killGraceMs);
      }
    }
    if (this._process?.token === record?.token) {
      this._process = null;
    }
    this.state = "stopped";
    this.fatalCode = null;
    const result = {
      ...this.health(),
      forced_kill: forcedKill,
      browser_close_sent: browserCloseSent,
    };
    this._emit("shutdown", {
      forced_kill: forcedKill,
      browser_close_sent: browserCloseSent,
    });
    return result;
  }

  async _waitForExit(record, milliseconds) {
    if (!record || record.exited) {
      return true;
    }
    const exited = record.exitPromise.then(() => true);
    const timeout = this.timers.sleep(milliseconds).then(() => false);
    return Promise.race([exited, timeout]);
  }

  _signal(record, signal) {
    try {
      this.signalProcess(record.child, signal);
    } catch {
      // A process that cannot accept a signal is treated as already detached.
    }
  }

  _onCdpDisconnect() {
    if (this.state === "stopping" || this.state === "stopped" || this.state === "idle") {
      return;
    }
    const record = this._process;
    this._cdp = null;
    this._setFatal(ERROR_CODES.CONTROLLER_CRASHED);
    if (record && !record.exited) {
      record.expected = true;
      this._signal(record, "SIGKILL");
    }
  }

  _setFatal(code) {
    if (this.state === "fatal") {
      return;
    }
    this.state = "fatal";
    this.fatalCode = code;
    this._stopHeartbeat();
    this._invalidateAllMutations();
    this._closeTargetConnections();
    this._rejectQueued(controllerError(code));
    void Promise.resolve(this.pageFeatures.shutdown?.()).catch(() => {});
    const connection = this._cdp;
    this._cdp = null;
    connection?.close();
    this._emit("fatal", { fatal_code: code });
  }

  _enqueue(generation, run) {
    const inFlight = this._queue.length + (this._queueActive ? 1 : 0);
    if (inFlight >= this.queueLimit) {
      return Promise.reject(controllerError(ERROR_CODES.QUEUE_SATURATED));
    }
    return new Promise((resolve, reject) => {
      this._queue.push({ generation, run, resolve, reject });
      this._pumpQueue();
    });
  }

  _pumpQueue() {
    if (this._queueActive || this._queue.length === 0) {
      return;
    }
    const item = this._queue.shift();
    this._queueActive = true;
    Promise.resolve()
      .then(() => {
        if (this.state !== "ready" || this.generation !== item.generation) {
          throw controllerError(ERROR_CODES.STALE_GENERATION);
        }
        return item.run();
      })
      .then(item.resolve, item.reject)
      .finally(() => {
        this._queueActive = false;
        this._pumpQueue();
      });
  }

  _rejectQueued(error) {
    while (this._queue.length > 0) {
      this._queue.shift().reject(error);
    }
  }
}

function validateRequest(value) {
  assertPlainObject(value);
  if (typeof value.type !== "string") {
    schemaError();
  }
  const type = value.type;
  if (type === "start") {
    assertExactKeys(value, ["type", "request_id", "owner_id"], ["type", "request_id", "owner_id"]);
    return {
      type,
      request_id: validateIdentifier(value.request_id, "request_id"),
      owner_id: validateIdentifier(value.owner_id, "owner_id"),
    };
  }
  if (type === "health") {
    assertExactKeys(value, ["type", "request_id", "owner_id", "generation"], [
      "type",
      "request_id",
      "owner_id",
    ]);
    return {
      type,
      request_id: validateIdentifier(value.request_id, "request_id"),
      owner_id: validateIdentifier(value.owner_id, "owner_id"),
      generation: value.generation === undefined ? undefined : validateGeneration(value.generation),
    };
  }
  if (type === "shutdown" || type === "restart") {
    assertExactKeys(value, ["type", "request_id", "owner_id", "generation"], [
      "type",
      "request_id",
      "owner_id",
      "generation",
    ]);
    return {
      type,
      request_id: validateIdentifier(value.request_id, "request_id"),
      owner_id: validateIdentifier(value.owner_id, "owner_id"),
      generation: validateGeneration(value.generation),
    };
  }
  if (type === "operation") {
    assertExactKeys(value, ["type", "request_id", "owner_id", "generation", "operation"], [
      "type",
      "request_id",
      "owner_id",
      "generation",
      "operation",
    ]);
    if (value.operation !== SAFE_OPERATION) {
      schemaError();
    }
    return {
      type,
      request_id: validateIdentifier(value.request_id, "request_id"),
      owner_id: validateIdentifier(value.owner_id, "owner_id"),
      generation: validateGeneration(value.generation),
      operation: value.operation,
    };
  }
  schemaError();
}

function extractRequestId(value) {
  if (!isPlainObject(value) || typeof value.request_id !== "string") {
    return null;
  }
  if (value.request_id.length === 0 || Buffer.byteLength(value.request_id, "utf8") > MAX_IDENTIFIER_BYTES) {
    return null;
  }
  return value.request_id;
}

function publicError(error) {
  const normalized = normalizeError(error);
  return {
    code: normalized.code,
    message: ERROR_MESSAGES[normalized.code] || ERROR_MESSAGES[ERROR_CODES.INTERNAL_ERROR],
  };
}

export class ControllerProtocol {
  constructor(controller, options = {}) {
    if (!(controller instanceof BrowserController)) {
      throw new TypeError("controller must be a BrowserController");
    }
    this.controller = controller;
    this.maxRequestBytes = options.maxRequestBytes ?? MAX_REQUEST_BYTES;
    this.maxOutputBytes = options.maxOutputBytes ?? MAX_OUTPUT_BYTES;
    this.seen = new Set();
    this.seenOrder = [];
  }

  async handleLine(line) {
    if (typeof line !== "string" || Buffer.byteLength(line, "utf8") > this.maxRequestBytes) {
      return this._errorResponse(null, controllerError(ERROR_CODES.REQUEST_TOO_LARGE));
    }
    let value;
    try {
      value = JSON.parse(line);
    } catch {
      return this._errorResponse(null, controllerError(ERROR_CODES.SCHEMA_INVALID));
    }
    return this.handle(value);
  }

  async handle(value) {
    const requestId = extractRequestId(value);
    let request;
    try {
      request = validateRequest(value);
    } catch (error) {
      return this._errorResponse(requestId, error);
    }
    if (this.seen.has(request.request_id)) {
      return this._errorResponse(request.request_id, controllerError(ERROR_CODES.DUPLICATE_REQUEST_ID));
    }
    this._rememberRequestId(request.request_id);
    try {
      let result;
      if (request.type === "start") {
        result = await this.controller.start(request.owner_id);
      } else if (request.type === "health") {
        result = this.controller.health(request.owner_id, request.generation);
      } else if (request.type === "shutdown") {
        result = await this.controller.shutdown(request.owner_id, request.generation);
      } else if (request.type === "restart") {
        result = await this.controller.restart(request.owner_id, request.generation);
      } else {
        result = await this.controller.execute(request.owner_id, request.generation, request.operation);
      }
      const response = {
        type: "response",
        request_id: request.request_id,
        ok: true,
        result,
      };
      if (Buffer.byteLength(JSON.stringify(response), "utf8") > this.maxOutputBytes) {
        return this._errorResponse(request.request_id, controllerError(ERROR_CODES.OUTPUT_TOO_LARGE));
      }
      return response;
    } catch (error) {
      return this._errorResponse(request.request_id, error);
    }
  }

  _rememberRequestId(requestId) {
    if (this.seen.size >= MAX_SEEN_REQUEST_IDS) {
      const oldest = this.seenOrder.shift();
      this.seen.delete(oldest);
    }
    this.seen.add(requestId);
    this.seenOrder.push(requestId);
  }

  _errorResponse(requestId, error) {
    return {
      type: "response",
      request_id: requestId,
      ok: false,
      error: publicError(error),
    };
  }
}

async function runCli() {
  const controller = new BrowserController();
  const protocol = new ControllerProtocol(controller);
  const write = (message) => {
    const encoded = JSON.stringify(message);
    if (Buffer.byteLength(encoded, "utf8") <= MAX_OUTPUT_BYTES) {
      process.stdout.write(encoded + "\n");
    }
  };
  controller.onEvent(write);
  let signalHandled = false;
  const handleSignal = () => {
    if (signalHandled) {
      return;
    }
    signalHandled = true;
    controller.shutdownForSignal().finally(() => {
      process.stdin.pause();
    });
  };
  process.once("SIGTERM", handleSignal);
  process.once("SIGINT", handleSignal);

  const input = createInterface({
    input: process.stdin,
    crlfDelay: Infinity,
  });
  for await (const line of input) {
    write(await protocol.handleLine(line));
  }
  await controller.shutdownForSignal();
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await runCli();
}
