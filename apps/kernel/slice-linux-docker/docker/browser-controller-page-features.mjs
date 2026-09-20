import { realpath as nodeRealpath, stat as nodeStat } from "node:fs/promises";
import path from "node:path";

const DEFAULT_TIMEOUT_MS = 5_000;
const MIN_TIMEOUT_MS = 100;
const MAX_TIMEOUT_MS = 5_000;
const POLL_INTERVAL_MS = 50;
const MAX_IDENTIFIER_BYTES = 512;
const MAX_PROMPT_BYTES = 8 * 1024;
const MAX_PATH_BYTES = 4 * 1024;
const MAX_UPLOAD_FILES = 16;
const MAX_KNOWN_TABS = 256;
const MAX_UPLOAD_FILE_BYTES = 64 * 1024 * 1024;
const MAX_UPLOAD_TOTAL_BYTES = 256 * 1024 * 1024;

const PERMISSIONS = new Set([
  "accessibilityEvents",
  "audioCapture",
  "backgroundSync",
  "clipboardReadWrite",
  "clipboardSanitizedWrite",
  "displayCapture",
  "geolocation",
  "idleDetection",
  "midi",
  "midiSysex",
  "notifications",
  "paymentHandler",
  "periodicBackgroundSync",
  "protectedMediaIdentifier",
  "sensors",
  "speakerSelection",
  "videoCapture",
  "videoCapturePanTiltZoom",
]);

export const PAGE_FEATURE_ERROR_CODES = Object.freeze({
  INVALID_ARGUMENT: "PAGE_FEATURE_INVALID",
  TIMEOUT: "PAGE_FEATURE_TIMEOUT",
  FAILED: "PAGE_FEATURE_FAILED",
  PATH_DENIED: "PAGE_FEATURE_PATH_DENIED",
});

export class BrowserPageFeatureError extends Error {
  constructor(code, details = {}) {
    super(code);
    this.name = "BrowserPageFeatureError";
    this.code = code;
    this.details = details;
  }
}

function fail(code, details) {
  throw new BrowserPageFeatureError(code, details);
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requireConnection(connection) {
  if (typeof connection?.send !== "function") {
    fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
  }
  return connection;
}

function requireIdentifier(value) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    Buffer.byteLength(value, "utf8") > MAX_IDENTIFIER_BYTES ||
    /[\u0000-\u001f\u007f]/.test(value)
  ) {
    fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
  }
  return value;
}

function requirePositiveInteger(value) {
  if (!Number.isSafeInteger(value) || value < 1) {
    fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
  }
  return value;
}

function requireExactKeys(value, allowed, required = []) {
  if (!isPlainObject(value)) fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
  const allowedKeys = new Set(allowed);
  if (Object.keys(value).some((key) => !allowedKeys.has(key))) {
    fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
  }
  if (required.some((key) => !Object.prototype.hasOwnProperty.call(value, key))) {
    fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
  }
}

function boundedTimeout(value) {
  if (value === undefined) return DEFAULT_TIMEOUT_MS;
  if (!Number.isSafeInteger(value) || value < 1) {
    fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
  }
  return Math.max(MIN_TIMEOUT_MS, Math.min(MAX_TIMEOUT_MS, value));
}

function readNow(now) {
  const value = Number(now());
  if (!Number.isFinite(value)) fail(PAGE_FEATURE_ERROR_CODES.FAILED);
  return value;
}

function normalizeElement(raw) {
  requireExactKeys(
    raw,
    [
      "tab_id",
      "browser_generation",
      "target_generation",
      "document_id",
      "frame_id",
      "main_frame_id",
      "snapshot_revision",
      "backend_node_id",
    ],
    ["tab_id", "document_id", "snapshot_revision", "backend_node_id"],
  );
  return {
    tabId: requireIdentifier(raw.tab_id),
    documentId: requireIdentifier(raw.document_id),
    snapshotRevision: requirePositiveInteger(raw.snapshot_revision),
    backendNodeId: requirePositiveInteger(raw.backend_node_id),
  };
}

function normalizeDialog(raw) {
  requireExactKeys(raw, ["action", "prompt_text"], ["action"]);
  if (!new Set(["accept", "dismiss"]).has(raw.action)) {
    fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
  }
  if (raw.prompt_text !== undefined) {
    if (
      raw.action !== "accept" ||
      typeof raw.prompt_text !== "string" ||
      Buffer.byteLength(raw.prompt_text, "utf8") > MAX_PROMPT_BYTES
    ) {
      fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
    }
  }
  return {
    action: raw.action,
    promptText: raw.prompt_text,
  };
}

function normalizeOrigin(value) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    Buffer.byteLength(value, "utf8") > 2048 ||
    /[\u0000-\u001f\u007f]/.test(value)
  ) {
    fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
  }
  let url;
  try {
    url = new URL(value);
  } catch {
    fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
  }
  if (
    !["http:", "https:"].includes(url.protocol) ||
    url.username ||
    url.password ||
    url.origin !== value
  ) {
    fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
  }
  return value;
}

function normalizePermissions(raw) {
  if (!Array.isArray(raw) || raw.length === 0 || raw.length > PERMISSIONS.size) {
    fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
  }
  const permissions = [...new Set(raw.map((permission) => {
    if (typeof permission !== "string" || !PERMISSIONS.has(permission)) {
      fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
    }
    return permission;
  }))].sort();
  return permissions;
}

function normalizeBrowserContextId(value) {
  return value === undefined ? undefined : requireIdentifier(value);
}

function normalizePath(value) {
  if (
    typeof value !== "string" ||
    !path.isAbsolute(value) ||
    Buffer.byteLength(value, "utf8") > MAX_PATH_BYTES ||
    /[\u0000-\u001f\u007f]/.test(value)
  ) {
    fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
  }
  return path.normalize(value);
}

function isWithin(root, candidate) {
  const relative = path.relative(root, candidate);
  return relative === "" || (!relative.startsWith(".." + path.sep) && relative !== "..");
}

function normalizeTabSnapshot(raw) {
  if (!isPlainObject(raw) || !Array.isArray(raw.tabs)) {
    fail(PAGE_FEATURE_ERROR_CODES.FAILED);
  }
  return raw.tabs.map((tab) => {
    if (!isPlainObject(tab)) fail(PAGE_FEATURE_ERROR_CODES.FAILED);
    return {
      tab_id: requireIdentifier(tab.tab_id),
      target_generation: requirePositiveInteger(tab.target_generation),
    };
  });
}

function safeNodeName(value) {
  if (typeof value !== "string") return "";
  return value.slice(0, 128).replace(/[\u0000-\u001f\u007f]/g, "");
}

export class BrowserPageFeatures {
  constructor({
    uploadRoots = [],
    downloadRoot = null,
    realpath = nodeRealpath,
    stat = nodeStat,
    now = Date.now,
    sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
  } = {}) {
    if (
      !Array.isArray(uploadRoots) ||
      typeof realpath !== "function" ||
      typeof stat !== "function" ||
      typeof now !== "function" ||
      typeof sleep !== "function"
    ) {
      throw new TypeError("invalid browser page feature dependencies");
    }
    this.uploadRoots = uploadRoots.map(normalizePath);
    this.downloadRoot = downloadRoot === null ? null : normalizePath(downloadRoot);
    this.realpath = realpath;
    this.stat = stat;
    this.now = now;
    this.sleep = sleep;
  }

  async describeElement({ connection, element: rawElement } = {}) {
    requireConnection(connection);
    const element = normalizeElement(rawElement);
    let response;
    try {
      response = await connection.send("DOM.describeNode", {
        backendNodeId: element.backendNodeId,
        depth: 0,
        pierce: true,
      });
    } catch {
      fail(PAGE_FEATURE_ERROR_CODES.FAILED);
    }
    if (!isPlainObject(response?.node)) fail(PAGE_FEATURE_ERROR_CODES.FAILED);
    const node = response.node;
    return {
      tab_id: element.tabId,
      document_id: element.documentId,
      snapshot_revision: element.snapshotRevision,
      frame_id: typeof node.frameId === "string" ? requireIdentifier(node.frameId) : null,
      shadow_root_type: typeof node.shadowRootType === "string"
        ? safeNodeName(node.shadowRootType)
        : null,
      node_name: safeNodeName(node.nodeName),
      local_name: safeNodeName(node.localName),
    };
  }

  async handleDialog({ connection, dialog } = {}) {
    requireConnection(connection);
    const normalized = normalizeDialog(dialog);
    try {
      await connection.send("Page.handleJavaScriptDialog", {
        accept: normalized.action === "accept",
        ...(normalized.promptText === undefined ? {} : { promptText: normalized.promptText }),
      });
    } catch {
      fail(PAGE_FEATURE_ERROR_CODES.FAILED);
    }
    return { action: normalized.action };
  }

  async waitForPopup({ listTabs, known_tab_ids: knownTabIds, timeout_ms: timeoutMs } = {}) {
    if (
      typeof listTabs !== "function" ||
      !Array.isArray(knownTabIds) ||
      knownTabIds.length > MAX_KNOWN_TABS
    ) {
      fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
    }
    const known = new Set(knownTabIds.map(requireIdentifier));
    const bounded = boundedTimeout(timeoutMs);
    const startedAt = readNow(this.now);
    let attempts = 0;
    while (true) {
      attempts += 1;
      const tabs = normalizeTabSnapshot(await listTabs());
      const popup = tabs.find((tab) => !known.has(tab.tab_id));
      if (popup) {
        return { ...popup, attempts, elapsed_ms: Math.max(0, readNow(this.now) - startedAt) };
      }
      const elapsed = Math.max(0, readNow(this.now) - startedAt);
      if (elapsed >= bounded) {
        fail(PAGE_FEATURE_ERROR_CODES.TIMEOUT, { attempts, timeout_ms: bounded });
      }
      await this.sleep(Math.min(POLL_INTERVAL_MS, bounded - elapsed));
    }
  }

  async configureDownloads({ connection, browser_context_id: browserContextId } = {}) {
    requireConnection(connection);
    if (this.downloadRoot === null) fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    const resolvedRoot = await this.#resolveAllowedRoot(this.downloadRoot);
    try {
      await connection.send("Browser.setDownloadBehavior", {
        behavior: "allowAndName",
        downloadPath: resolvedRoot,
        eventsEnabled: true,
        ...(browserContextId === undefined
          ? {}
          : { browserContextId: normalizeBrowserContextId(browserContextId) }),
      });
    } catch {
      fail(PAGE_FEATURE_ERROR_CODES.FAILED);
    }
    return { enabled: true };
  }

  async uploadFiles({ connection, element: rawElement, paths } = {}) {
    requireConnection(connection);
    const element = normalizeElement(rawElement);
    if (!Array.isArray(paths) || paths.length === 0 || paths.length > MAX_UPLOAD_FILES) {
      fail(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT);
    }
    const files = [];
    let totalBytes = 0;
    for (const candidate of paths) {
      const file = await this.#resolveUploadFile(candidate);
      totalBytes += file.size;
      if (totalBytes > MAX_UPLOAD_TOTAL_BYTES) {
        fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
      }
      files.push(file.path);
    }
    try {
      await connection.send("DOM.setFileInputFiles", {
        backendNodeId: element.backendNodeId,
        files,
      });
    } catch {
      fail(PAGE_FEATURE_ERROR_CODES.FAILED);
    }
    return {
      tab_id: element.tabId,
      document_id: element.documentId,
      snapshot_revision: element.snapshotRevision,
      file_count: files.length,
    };
  }

  async grantPermissions({ connection, origin, permissions, browser_context_id: browserContextId } = {}) {
    requireConnection(connection);
    const normalizedOrigin = normalizeOrigin(origin);
    const normalizedPermissions = normalizePermissions(permissions);
    try {
      await connection.send("Browser.grantPermissions", {
        origin: normalizedOrigin,
        permissions: normalizedPermissions,
        ...(browserContextId === undefined
          ? {}
          : { browserContextId: normalizeBrowserContextId(browserContextId) }),
      });
    } catch {
      fail(PAGE_FEATURE_ERROR_CODES.FAILED);
    }
    return { origin: normalizedOrigin, permissions: normalizedPermissions };
  }

  async resetPermissions({ connection, browser_context_id: browserContextId } = {}) {
    requireConnection(connection);
    try {
      await connection.send("Browser.resetPermissions", browserContextId === undefined
        ? {}
        : { browserContextId: normalizeBrowserContextId(browserContextId) });
    } catch {
      fail(PAGE_FEATURE_ERROR_CODES.FAILED);
    }
    return { reset: true };
  }

  async #resolveAllowedRoot(candidate) {
    const normalized = normalizePath(candidate);
    let resolved;
    try {
      resolved = normalizePath(await this.realpath(normalized));
      const metadata = await this.stat(resolved);
      if (!metadata?.isDirectory?.()) fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    } catch (error) {
      if (error instanceof BrowserPageFeatureError) throw error;
      fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    }
    return resolved;
  }

  async #resolveUploadFile(candidate) {
    const normalized = normalizePath(candidate);
    if (this.uploadRoots.length === 0) fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    let resolved;
    let roots;
    try {
      [resolved, roots] = await Promise.all([
        this.realpath(normalized),
        Promise.all(this.uploadRoots.map((root) => this.realpath(root))),
      ]);
      resolved = normalizePath(resolved);
      roots = roots.map(normalizePath);
      if (!roots.some((root) => isWithin(root, resolved))) {
        fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
      }
      const metadata = await this.stat(resolved);
      if (
        !metadata?.isFile?.() ||
        !Number.isSafeInteger(metadata.size) ||
        metadata.size < 0 ||
        metadata.size > MAX_UPLOAD_FILE_BYTES
      ) {
        fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
      }
      return { path: resolved, size: metadata.size };
    } catch (error) {
      if (error instanceof BrowserPageFeatureError) throw error;
      fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    }
  }
}
