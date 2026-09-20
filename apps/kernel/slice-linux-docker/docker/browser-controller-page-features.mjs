import {
  mkdtemp as nodeMkdtemp,
  open as nodeOpen,
  realpath as nodeRealpath,
  rm as nodeRm,
  stat as nodeStat,
} from "node:fs/promises";
import { constants as fsConstants } from "node:fs";
import { tmpdir as nodeTmpdir } from "node:os";
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
const UPLOAD_COPY_CHUNK_BYTES = 64 * 1024;

export const CHROMIUM_PERMISSION_TYPES = Object.freeze([
  "ar",
  "audioCapture",
  "automaticFullscreen",
  "backgroundFetch",
  "backgroundSync",
  "cameraPanTiltZoom",
  "capturedSurfaceControl",
  "clipboardReadWrite",
  "clipboardSanitizedWrite",
  "displayCapture",
  "durableStorage",
  "geolocation",
  "handTracking",
  "idleDetection",
  "keyboardLock",
  "localFonts",
  "localNetwork",
  "localNetworkAccess",
  "loopbackNetwork",
  "midi",
  "midiSysex",
  "nfc",
  "notifications",
  "paymentHandler",
  "periodicBackgroundSync",
  "pointerLock",
  "protectedMediaIdentifier",
  "sensors",
  "smartCard",
  "speakerSelection",
  "storageAccess",
  "topLevelStorageAccess",
  "videoCapture",
  "vr",
  "wakeLockScreen",
  "wakeLockSystem",
  "webAppInstallation",
  "webPrinting",
  "windowManagement",
]);

const PERMISSIONS = new Set(CHROMIUM_PERMISSION_TYPES);

export const PAGE_FEATURE_ERROR_CODES = Object.freeze({
  INVALID_ARGUMENT: "PAGE_FEATURE_INVALID",
  TIMEOUT: "PAGE_FEATURE_TIMEOUT",
  FAILED: "PAGE_FEATURE_FAILED",
  PATH_DENIED: "PAGE_FEATURE_PATH_DENIED",
  STALE_DOCUMENT: "STALE_DOCUMENT",
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
      "shadow_root_type",
    ],
    ["tab_id", "document_id", "snapshot_revision", "backend_node_id"],
  );
  return {
    tabId: requireIdentifier(raw.tab_id),
    documentId: requireIdentifier(raw.document_id),
    snapshotRevision: requirePositiveInteger(raw.snapshot_revision),
    backendNodeId: requirePositiveInteger(raw.backend_node_id),
    frameId: raw.frame_id === undefined || raw.frame_id === null
      ? null
      : requireIdentifier(raw.frame_id),
    mainFrameId: raw.main_frame_id === undefined || raw.main_frame_id === null
      ? null
      : requireIdentifier(raw.main_frame_id),
    shadowRootType: raw.shadow_root_type === undefined || raw.shadow_root_type === null
      ? null
      : safeNodeName(raw.shadow_root_type),
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

function fileIdentity(value) {
  if (
    !Number.isSafeInteger(value?.dev) ||
    !Number.isSafeInteger(value?.ino)
  ) {
    return null;
  }
  return { dev: value.dev, ino: value.ino };
}

function sameFileIdentity(left, right) {
  const leftIdentity = left?.identity ?? fileIdentity(left);
  const rightIdentity = right?.identity ?? fileIdentity(right);
  return leftIdentity !== null && rightIdentity !== null && (
    leftIdentity.dev === rightIdentity.dev &&
    leftIdentity.ino === rightIdentity.ino
  );
}

function validFileMetadata(metadata, maximumBytes) {
  return (
    metadata?.isFile?.() === true &&
    Number.isSafeInteger(metadata.size) &&
    metadata.size >= 0 &&
    metadata.size <= MAX_UPLOAD_FILE_BYTES &&
    metadata.size <= maximumBytes
  );
}

function validDirectoryMetadata(metadata) {
  return metadata?.isDirectory?.() === true;
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
  #uploadRoots;
  #downloadRoot;
  #uploadRootsSnapshot;
  #downloadRootSnapshot;

  constructor({
    uploadRoots = [],
    downloadRoot = null,
    realpath = nodeRealpath,
    stat = nodeStat,
    open = nodeOpen,
    mkdtemp = nodeMkdtemp,
    remove = nodeRm,
    stageUpload = null,
    now = Date.now,
    sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
  } = {}) {
    if (
      !Array.isArray(uploadRoots) ||
      typeof realpath !== "function" ||
      typeof stat !== "function" ||
      typeof open !== "function" ||
      typeof mkdtemp !== "function" ||
      typeof remove !== "function" ||
      (stageUpload !== null && typeof stageUpload !== "function") ||
      typeof now !== "function" ||
      typeof sleep !== "function"
    ) {
      throw new TypeError("invalid browser page feature dependencies");
    }
    this.#uploadRoots = Object.freeze(uploadRoots.map(normalizePath));
    this.#downloadRoot = downloadRoot === null ? null : normalizePath(downloadRoot);
    this.realpath = realpath;
    this.stat = stat;
    this.open = open;
    this.mkdtemp = mkdtemp;
    this.remove = remove;
    this.stageUpload = stageUpload;
    this.now = now;
    this.sleep = sleep;
    this.#uploadRootsSnapshot = this.#captureConfiguredRoots(this.#uploadRoots);
    this.#downloadRootSnapshot = this.#downloadRoot === null
      ? Promise.resolve(null)
      : this.#captureConfiguredDownloadRoot(this.#downloadRoot);
  }

  async prepare() {
    await this.#getUploadRoots();
    await this.#getDownloadRoot();
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
    const describedFrameId = typeof node.frameId === "string"
      ? requireIdentifier(node.frameId)
      : null;
    return {
      tab_id: element.tabId,
      document_id: element.documentId,
      snapshot_revision: element.snapshotRevision,
      frame_id: element.frameId ?? describedFrameId,
      shadow_root_type: element.shadowRootType,
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
    const resolvedRoot = await this.#getDownloadRoot();
    if (resolvedRoot === null) fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    await this.#assertConfiguredRoot(resolvedRoot);
    try {
      await connection.send("Browser.setDownloadBehavior", {
        behavior: "allowAndName",
        downloadPath: resolvedRoot.path,
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
    const inspectedFiles = [];
    let totalBytes = 0;
    for (const candidate of paths) {
      const file = await this.#resolveUploadFile(candidate);
      totalBytes += file.size;
      if (totalBytes > MAX_UPLOAD_TOTAL_BYTES) {
        fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
      }
      inspectedFiles.push(file);
    }

    const stageDirectory = await this.#createUploadStageDirectory();
    const files = [];
    try {
      let stagedTotalBytes = 0;
      for (const [index, file] of inspectedFiles.entries()) {
        const remainingBytes = MAX_UPLOAD_TOTAL_BYTES - stagedTotalBytes;
        const staged = await this.#stageUploadFile(file, stageDirectory, index, remainingBytes);
        stagedTotalBytes += staged.size;
        if (stagedTotalBytes > MAX_UPLOAD_TOTAL_BYTES) {
          fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
        }
        files.push(staged.path);
      }

      await this.#assertCurrentDocument(connection, element.documentId);
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
    } finally {
      await this.#removeUploadStageDirectory(stageDirectory);
    }
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

  async #captureConfiguredRoot(candidate, holdHandle) {
    let handle = null;
    try {
      const resolved = normalizePath(await this.realpath(candidate));
      const metadata = await this.stat(resolved);
      if (!validDirectoryMetadata(metadata)) {
        fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
      }
      if (
        holdHandle &&
        this.realpath === nodeRealpath &&
        this.stat === nodeStat &&
        this.open === nodeOpen
      ) {
        handle = await this.open(
          resolved,
          fsConstants.O_RDONLY | fsConstants.O_DIRECTORY | fsConstants.O_NOFOLLOW,
        );
        const handleMetadata = await handle.stat();
        if (!sameFileIdentity(metadata, handleMetadata)) {
          await this.#closeFileHandle(handle);
          handle = null;
          fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
        }
      }
      return Object.freeze({
        path: resolved,
        identity: fileIdentity(metadata),
        handle,
      });
    } catch (error) {
      await this.#closeFileHandle(handle);
      if (error instanceof BrowserPageFeatureError) throw error;
      fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    }
  }

  #captureConfiguredRoots(candidates) {
    return (async () => {
      const roots = [];
      try {
        for (const candidate of candidates) {
          roots.push(await this.#captureConfiguredRoot(candidate, true));
        }
        return { roots: Object.freeze(roots), error: null };
      } catch (error) {
        await Promise.all(roots.map((root) => this.#closeFileHandle(root.handle)));
        return { roots: null, error };
      }
    })();
  }

  #captureConfiguredDownloadRoot(candidate) {
    return (async () => {
      try {
        return { root: await this.#captureConfiguredRoot(candidate, false), error: null };
      } catch (error) {
        return { root: null, error };
      }
    })();
  }

  async #getUploadRoots() {
    const result = await this.#uploadRootsSnapshot;
    if (result.error) {
      if (result.error instanceof BrowserPageFeatureError) throw result.error;
      fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    }
    return result.roots;
  }

  async #getDownloadRoot() {
    if (this.#downloadRoot === null) return null;
    const result = await this.#downloadRootSnapshot;
    if (result.error) {
      if (result.error instanceof BrowserPageFeatureError) throw result.error;
      fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    }
    return result.root;
  }

  async #assertConfiguredRoot(root) {
    let metadata;
    try {
      metadata = await this.stat(root.path);
      if (!validDirectoryMetadata(metadata) || !sameFileIdentity(root, metadata)) {
        fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
      }
      if (root.handle !== null) {
        const handleMetadata = await root.handle.stat();
        if (!validDirectoryMetadata(handleMetadata) || !sameFileIdentity(root, handleMetadata)) {
          fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
        }
      }
    } catch (error) {
      if (error instanceof BrowserPageFeatureError) throw error;
      fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    }
  }

  async #resolveUploadFile(candidate) {
    const normalized = normalizePath(candidate);
    const roots = await this.#getUploadRoots();
    if (roots.length === 0) fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    let resolved;
    try {
      resolved = normalizePath(await this.realpath(normalized));
      const matchingRoot = roots
        .filter((root) => isWithin(root.path, resolved))
        .sort((left, right) => right.path.length - left.path.length)[0];
      if (!matchingRoot) fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
      await this.#assertConfiguredRoot(matchingRoot);
      const metadata = await this.stat(resolved);
      if (!validFileMetadata(metadata, MAX_UPLOAD_FILE_BYTES)) {
        fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
      }
      return {
        path: resolved,
        size: metadata.size,
        identity: fileIdentity(metadata),
        root: matchingRoot,
      };
    } catch (error) {
      if (error instanceof BrowserPageFeatureError) throw error;
      fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    }
  }

  async #createUploadStageDirectory() {
    try {
      return normalizePath(await this.mkdtemp(path.join(nodeTmpdir(), "chariox-browser-upload-")));
    } catch {
      fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    }
  }

  async #removeUploadStageDirectory(stageDirectory) {
    try {
      await this.remove(stageDirectory, { recursive: true, force: true });
    } catch {
      // The staged path is never returned to the caller; failed cleanup cannot widen access.
    }
  }

  async #stageUploadFile(file, stageDirectory, index, remainingBytes) {
    if (file.size > remainingBytes) fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    const destinationPath = normalizePath(path.join(stageDirectory, `file-${index}`));
    try {
      const currentPath = normalizePath(await this.realpath(file.path));
      if (currentPath !== file.path) fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
      await this.#assertConfiguredRoot(file.root);
      if (this.stageUpload !== null) {
        const staged = await this.stageUpload({
          sourcePath: file.path,
          destinationPath,
          expectedSize: file.size,
          maxBytes: remainingBytes,
        });
        const stagedSize = staged?.size === undefined ? file.size : staged.size;
        if (
          staged?.path !== undefined && normalizePath(staged.path) !== destinationPath ||
          !Number.isSafeInteger(stagedSize) ||
          stagedSize < 0 ||
          stagedSize > MAX_UPLOAD_FILE_BYTES ||
          stagedSize > remainingBytes
        ) {
          fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
        }
        return { path: destinationPath, size: stagedSize };
      }
      return await this.#copyUploadToStage(file, destinationPath, remainingBytes);
    } catch (error) {
      if (error instanceof BrowserPageFeatureError) throw error;
      fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
    }
  }

  async #copyUploadToStage(file, destinationPath, remainingBytes) {
    const relativePath = path.relative(file.root.path, file.path);
    const sourcePath = file.root.handle === null
      ? file.path
      : path.join(`/proc/self/fd/${file.root.handle.fd}`, relativePath);
    let sourceHandle;
    let destinationHandle;
    try {
      sourceHandle = await this.open(
        sourcePath,
        fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW,
      );
      const openedMetadata = await sourceHandle.stat();
      if (
        !validFileMetadata(openedMetadata, remainingBytes) ||
        openedMetadata.size !== file.size ||
        !sameFileIdentity(file, openedMetadata)
      ) {
        fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
      }
      destinationHandle = await this.open(
        destinationPath,
        fsConstants.O_WRONLY |
          fsConstants.O_CREAT |
          fsConstants.O_EXCL |
          fsConstants.O_NOFOLLOW,
        0o600,
      );
      const buffer = Buffer.allocUnsafe(Math.min(UPLOAD_COPY_CHUNK_BYTES, openedMetadata.size || 1));
      let remaining = openedMetadata.size;
      while (remaining > 0) {
        const requested = Math.min(buffer.length, remaining);
        const result = await sourceHandle.read(buffer, 0, requested);
        if (!Number.isSafeInteger(result?.bytesRead) || result.bytesRead < 1 || result.bytesRead > requested) {
          fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
        }
        const written = await destinationHandle.write(buffer.subarray(0, result.bytesRead));
        if (written?.bytesWritten !== result.bytesRead) {
          fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
        }
        remaining -= result.bytesRead;
      }
      const finalSourceMetadata = await sourceHandle.stat();
      const finalDestinationMetadata = await destinationHandle.stat();
      if (
        !validFileMetadata(finalSourceMetadata, remainingBytes) ||
        finalSourceMetadata.size !== openedMetadata.size ||
        !sameFileIdentity(openedMetadata, finalSourceMetadata) ||
        !validFileMetadata(finalDestinationMetadata, remainingBytes) ||
        finalDestinationMetadata.size !== openedMetadata.size
      ) {
        fail(PAGE_FEATURE_ERROR_CODES.PATH_DENIED);
      }
      return { path: destinationPath, size: finalDestinationMetadata.size };
    } finally {
      await this.#closeFileHandle(destinationHandle);
      await this.#closeFileHandle(sourceHandle);
    }
  }

  async #closeFileHandle(handle) {
    try {
      await handle?.close?.();
    } catch {
      // A failed close cannot expose a caller path or change the validated bytes.
    }
  }

  async #assertCurrentDocument(connection, documentId) {
    let response;
    try {
      response = await connection.send("Page.getFrameTree", {});
    } catch {
      fail(PAGE_FEATURE_ERROR_CODES.FAILED);
    }
    if (response?.frameTree?.frame?.loaderId !== documentId) {
      fail(PAGE_FEATURE_ERROR_CODES.STALE_DOCUMENT);
    }
  }
}
