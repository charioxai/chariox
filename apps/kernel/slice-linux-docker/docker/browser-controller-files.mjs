import { mkdir, realpath, stat, statfs } from "node:fs/promises";
import path from "node:path";
import { assertNotCancelled, performBrowserAction } from "./browser-controller-actions.mjs";
import { assertBrowserFramesUnchanged } from "./browser-controller-frames.mjs";
import { stageBrowserUploadFiles } from "./browser-controller-upload-staging.mjs";

const MAX_UPLOAD_FILES = 20;
const MAX_UPLOAD_PATH_BYTES = 4_096;
const MAX_UPLOAD_TOTAL_BYTES = 512 * 1024 * 1024;
export const DEFAULT_MINIMUM_DOWNLOAD_FREE_BYTES = 256 * 1024 * 1024;

const defaultFileSystem = { mkdir, realpath, stat, statfs };

export class BrowserFileTransferError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserFileTransferError";
    this.code = code;
  }
}

export async function cancelBrowserDownload({
  connection, browserGeneration, requestedBrowserGeneration, guid, targetsByDownload,
}) {
  if (typeof guid !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(guid)) {
    throw new BrowserFileTransferError("browser_download_invalid", "download cancellation requires a bounded download identifier");
  }
  if (!Number.isSafeInteger(requestedBrowserGeneration) || requestedBrowserGeneration <= 0 || requestedBrowserGeneration !== browserGeneration) {
    throw new BrowserFileTransferError("stale_browser_generation", "download cancellation requires the current browser generation");
  }
  if (!targetsByDownload.has(guid)) {
    throw new BrowserFileTransferError("browser_download_not_active", "download is not active in this browser generation");
  }
  await connection.send("Browser.cancelDownload", { guid });
  return { browser_generation: browserGeneration, guid, cancellation_requested: true };
}

export async function configureBrowserDownloads({
  connection,
  sessionId,
  targetId,
  documentId,
  downloadDirectory,
  minimumFreeBytes = DEFAULT_MINIMUM_DOWNLOAD_FREE_BYTES,
  fileSystem = defaultFileSystem,
  signal,
}) {
  assertNotCancelled(signal);
  await assertCurrentDocument(connection, sessionId, targetId, documentId);
  assertNotCancelled(signal);
  if (typeof downloadDirectory !== "string" || !path.isAbsolute(downloadDirectory)) {
    throw new BrowserFileTransferError(
      "browser_download_unconfigured",
      "browser downloads require a configured absolute directory",
    );
  }
  let resolvedDirectory;
  try {
    await fileSystem.mkdir(downloadDirectory, { recursive: true, mode: 0o700 });
    resolvedDirectory = await fileSystem.realpath(downloadDirectory);
    const metadata = await fileSystem.stat(resolvedDirectory);
    if (!metadata.isDirectory()) {
      throw new Error("configured download path is not a directory");
    }
  } catch (error) {
    throw new BrowserFileTransferError(
      "browser_download_unavailable",
      `browser download directory is unavailable: ${String(error?.code ?? "filesystem_error")}`,
    );
  }
  await assertBrowserDownloadHeadroom({
    downloadDirectory: resolvedDirectory,
    minimumFreeBytes,
    fileSystem,
  });
  await assertCurrentDocument(connection, sessionId, targetId, documentId);
  assertNotCancelled(signal);
  await connection.send("Browser.setDownloadBehavior", {
    behavior: "allowAndName",
    downloadPath: resolvedDirectory,
    eventsEnabled: true,
  });
  return {
    target_id: targetId,
    document_id: documentId,
    enabled: true,
  };
}

export async function assertBrowserDownloadHeadroom({
  downloadDirectory,
  minimumFreeBytes = DEFAULT_MINIMUM_DOWNLOAD_FREE_BYTES,
  fileSystem = defaultFileSystem,
}) {
  if (typeof downloadDirectory !== "string" || !path.isAbsolute(downloadDirectory)) {
    throw new BrowserFileTransferError(
      "browser_download_unconfigured",
      "browser downloads require a configured absolute directory",
    );
  }
  if (!Number.isSafeInteger(minimumFreeBytes) || minimumFreeBytes < 0) {
    throw new BrowserFileTransferError(
      "browser_download_unconfigured",
      "browser download free-space reserve must be a non-negative integer",
    );
  }
  let filesystem;
  try {
    filesystem = await fileSystem.statfs(downloadDirectory);
  } catch (error) {
    throw new BrowserFileTransferError(
      "browser_download_unavailable",
      `browser download storage capacity is unavailable: ${String(error?.code ?? "filesystem_error")}`,
    );
  }
  const availableBytes = filesystemAvailableBytes(filesystem);
  if (availableBytes < BigInt(minimumFreeBytes)) {
    throw new BrowserFileTransferError(
      "browser_download_low_disk",
      `browser downloads need ${Math.ceil(minimumFreeBytes / (1024 * 1024))} MiB of free slice storage; free disk space and retry`,
    );
  }
}

function filesystemAvailableBytes(filesystem) {
  const available = filesystem?.bavail;
  const blockSize = filesystem?.bsize;
  if (
    !["bigint", "number"].includes(typeof available) ||
    !["bigint", "number"].includes(typeof blockSize)
  ) {
    throw new BrowserFileTransferError(
      "browser_download_unavailable",
      "browser download storage capacity is invalid",
    );
  }
  if (
    (typeof available === "number" && (!Number.isSafeInteger(available) || available < 0)) ||
    (typeof blockSize === "number" && (!Number.isSafeInteger(blockSize) || blockSize < 0)) ||
    (typeof available === "bigint" && available < 0n) ||
    (typeof blockSize === "bigint" && blockSize < 0n)
  ) {
    throw new BrowserFileTransferError(
      "browser_download_unavailable",
      "browser download storage capacity is invalid",
    );
  }
  const availableBytes = BigInt(available) * BigInt(blockSize);
  if (availableBytes < 0n) {
    throw new BrowserFileTransferError(
      "browser_download_unavailable",
      "browser download storage capacity is invalid",
    );
  }
  return availableBytes;
}

export async function uploadBrowserFiles({
  connection,
  sessionId,
  targetId,
  documentId,
  nodeRef,
  filePaths,
  uploadRoots,
  fileSystem = defaultFileSystem,
  assertContext = async () => {},
  signal,
  stageUploads = stageBrowserUploadFiles,
  clickChooser = performBrowserAction,
}) {
  assertNotCancelled(signal);
  await assertCurrentDocument(connection, sessionId, targetId, documentId);
  const backendNodeId = parseBackendNodeReference(nodeRef);
  if (!Array.isArray(filePaths) || filePaths.length === 0 || filePaths.length > MAX_UPLOAD_FILES) {
    throw invalidUpload(`browser upload requires 1 through ${MAX_UPLOAD_FILES} files`);
  }
  if (!Array.isArray(uploadRoots) || uploadRoots.length === 0) {
    throw new BrowserFileTransferError(
      "browser_upload_denied",
      "browser uploads require a configured file root",
    );
  }

  const roots = await resolveUploadRoots(uploadRoots, fileSystem);
  const files = [];
  const metadataByFile = [];
  let totalBytes = 0;
  for (const candidate of filePaths) {
    if (
      typeof candidate !== "string" ||
      !path.isAbsolute(candidate) ||
      !utf8ByteLengthAtMost(candidate, MAX_UPLOAD_PATH_BYTES)
    ) {
      throw invalidUpload("browser upload paths must be bounded absolute paths");
    }
    let resolved;
    let metadata;
    try {
      resolved = await fileSystem.realpath(candidate);
      metadata = await fileSystem.stat(resolved, { bigint: true });
    } catch (error) {
      throw invalidUpload(`browser upload file is unavailable: ${String(error?.code ?? "filesystem_error")}`);
    }
    if (!roots.some((root) => isWithinRoot(root, resolved))) {
      throw new BrowserFileTransferError(
        "browser_upload_denied",
        "browser upload file is outside configured roots",
      );
    }
    const size = Number(metadata.size);
    if (!metadata.isFile() || !Number.isSafeInteger(size) || size < 0) {
      throw invalidUpload("browser uploads require regular files with a bounded size");
    }
    totalBytes += size;
    if (!Number.isSafeInteger(totalBytes) || totalBytes > MAX_UPLOAD_TOTAL_BYTES) {
      throw invalidUpload(`browser upload exceeds ${MAX_UPLOAD_TOTAL_BYTES} total bytes`);
    }
    files.push(resolved);
    metadataByFile.push({ ...metadata, size });
  }

  const staged = await stageUploads({ files, metadata: metadataByFile,
    browserIdentity: connection.browserInstanceId, signal, connection }).catch(error => {
    if (error?.code === "browser_action_cancelled") throw error;
    throw new BrowserFileTransferError("browser_upload_staging_unavailable",
      error?.code === "browser_upload_staging_unavailable" ? error.message : "upload staging could not prepare private files");
  });
  let objectId;
  let chooserIntercepted = false;
  let chooserWait;
  let assertChooserContext = async () => {};
  let chooserDocument;
  try {
    await assertContext();
    await assertCurrentDocument(connection, sessionId, targetId, documentId);
    assertNotCancelled(signal);
    const resolved = await connection.send("DOM.resolveNode", { backendNodeId }, sessionId);
    objectId = resolved?.object?.objectId;
    if (!objectId) throw staleFileInput();
    const inspected = await connection.send("Runtime.callFunctionOn", {
      objectId,
      functionDeclaration: `function() {
        if (!this.isConnected || this.ownerDocument !== this.ownerDocument.defaultView?.document) return "detached";
        return this.localName === "input" && this.type === "file" ? "file" : "invalid";
      }`,
      returnByValue: true,
      awaitPromise: false,
    }, sessionId);
    if (inspected?.exceptionDetails || inspected?.result?.value !== "file") {
      if (inspected?.result?.value === "invalid") {
        // MP-08/MP-10/MP-11: only a real chooser opened by the observed,
        // actionable control can bind its hidden input. Never guess a selector.
        await assertContext();
        const tree = await connection.send("Page.getFrameTree", {}, sessionId);
        if (tree?.frameTree?.frame?.loaderId !== documentId) throw staleFileInput();
        chooserDocument = await owningDocument(connection, sessionId, objectId);
        if (!findFrame(tree.frameTree, chooserDocument.frameId)) throw staleFileInput();
        // Local child frames share this renderer session and ordinary backend
        // refs. Fence the owning loader and every ancestor, plus any isolated
        // renderer parents supplied by withBrowserActionFrame.
        assertChooserContext = async () => {
          await assertContext();
          await assertBrowserFramesUnchanged(connection, [{ sessionId, tree: tree.frameTree }]);
        };
        await assertChooserContext();
        await connection.send("Runtime.releaseObject", { objectId }, sessionId);
        objectId = undefined;
        await connection.send("Page.setInterceptFileChooserDialog", { enabled: true }, sessionId);
        chooserIntercepted = true;
        const abort = () => chooserWait?.cancel();
        signal?.addEventListener("abort", abort, { once: true });
        let event;
        try {
          await clickChooser({ connection, sessionId, targetId, documentId, nodeRef,
            action: { kind: "click" }, assertContext: assertChooserContext, signal,
            withInput: operation => {
              chooserWait = connection.waitForEvent("Page.fileChooserOpened", 5_000, sessionId);
              return operation();
            } });
          event = await chooserWait?.promise;
        } finally { signal?.removeEventListener("abort", abort); }
        assertNotCancelled(signal);
        await assertContext();
        await assertCurrentDocument(connection, sessionId, targetId, documentId);
        await assertChooserContext();
        if (event?.sessionId !== sessionId || event?.params?.frameId !== chooserDocument.frameId
            || !Number.isSafeInteger(event?.params?.backendNodeId) || event.params.backendNodeId <= 0
            || !["selectSingle", "selectMultiple"].includes(event.params.mode)
            || (event.params.mode === "selectSingle" && files.length !== 1)) {
          throw invalidUpload("observed control did not open a matching bounded file chooser");
        }
        const chosen = await connection.send("DOM.resolveNode", { backendNodeId: event.params.backendNodeId }, sessionId);
        objectId = chosen?.object?.objectId;
        if (!objectId) throw staleFileInput();
        const inputDocument = await owningDocument(connection, sessionId, objectId);
        if (inputDocument.frameId !== chooserDocument.frameId || inputDocument.backendNodeId !== chooserDocument.backendNodeId) throw staleFileInput();
      } else {
        throw staleFileInput();
      }
    }
    // Resolving and inspecting the node cross asynchronous renderer calls.
    // Recheck both the owning document and its parents before exposing files.
    await assertContext();
    await assertCurrentDocument(connection, sessionId, targetId, documentId);
    assertNotCancelled(signal);
    // Dispatch may have succeeded even if its reply is lost. Retain these
    // browser-owned File backing bytes across CDP/controller reconnects.
    await assertChooserContext();
    await staged.markExposed();
    await assertChooserContext();
    await assertContext();
    await assertCurrentDocument(connection, sessionId, targetId, documentId);
    assertNotCancelled(signal);
    await connection.send(
      "DOM.setFileInputFiles",
      { objectId, files: staged.files },
      sessionId,
    );
  } catch (error) {
    if (error?.code !== "browser_cdp_command_failed") throw error;
    throw staleFileInput();
  } finally {
    chooserWait?.cancel();
    if (chooserIntercepted) await connection.send("Page.setInterceptFileChooserDialog", { enabled: false }, sessionId).catch(() => {});
    if (objectId) await connection.send("Runtime.releaseObject", { objectId }, sessionId).catch(() => {});
    await staged.discard();
  }
  return {
    target_id: targetId,
    document_id: documentId,
    file_count: files.length,
    total_bytes: totalBytes,
  };
}

// MP-08/MP-10/MP-11: CDP resolves the observed control's actual Document,
// without URL/origin guessing or trusting a page-selected frame identifier.
async function owningDocument(connection, sessionId, objectId) {
  const resolved = await connection.send("Runtime.callFunctionOn", {
    objectId, functionDeclaration: "function() { return this.isConnected ? this.ownerDocument : null; }",
    returnByValue: false, awaitPromise: false,
  }, sessionId);
  const documentObject = resolved?.result?.objectId;
  if (resolved?.exceptionDetails || !documentObject) throw staleFileInput();
  try {
    const { node } = await connection.send("DOM.describeNode", { objectId: documentObject }, sessionId);
    if (!Number.isSafeInteger(node?.backendNodeId) || node.backendNodeId <= 0) throw staleFileInput();
    // DOM.describeNode does not expose frameId on Document nodes. The
    // multi-document snapshot binds that Document backend ID to its CDP frame.
    const snapshot = await connection.send("DOMSnapshot.captureSnapshot", { computedStyles: [] }, sessionId);
    const document = snapshot.documents?.find(doc => doc.nodes?.backendNodeId?.[0] === node.backendNodeId);
    const frameId = snapshot.strings?.[document?.frameId];
    if (typeof frameId !== "string" || !frameId) throw staleFileInput();
    return { frameId, backendNodeId: node.backendNodeId };
  } finally {
    await connection.send("Runtime.releaseObject", { objectId: documentObject }, sessionId).catch(() => {});
  }
}
function findFrame(tree, frameId) {
  if (tree?.frame?.id === frameId) return tree.frame;
  for (const child of tree?.childFrames ?? []) { const frame = findFrame(child, frameId); if (frame) return frame; }
}

function staleFileInput() {
  return new BrowserFileTransferError(
    "stale_element_reference",
    "browser file input is no longer attached to the current document",
  );
}

async function resolveUploadRoots(uploadRoots, fileSystem) {
  const roots = [];
  for (const root of uploadRoots) {
    if (typeof root !== "string" || !path.isAbsolute(root)) {
      throw new BrowserFileTransferError(
        "browser_upload_denied",
        "browser upload roots must be absolute directories",
      );
    }
    try {
      const resolved = await fileSystem.realpath(root);
      const metadata = await fileSystem.stat(resolved);
      if (!metadata.isDirectory()) throw new Error("upload root is not a directory");
      roots.push(resolved);
    } catch (error) {
      throw new BrowserFileTransferError(
        "browser_upload_denied",
        `browser upload root is unavailable: ${String(error?.code ?? "filesystem_error")}`,
      );
    }
  }
  return roots;
}

async function assertCurrentDocument(connection, sessionId, targetId, documentId) {
  const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
  if (frameTree?.frameTree?.frame?.loaderId !== documentId) {
    throw new BrowserFileTransferError(
      "stale_document_reference",
      `browser target ${JSON.stringify(targetId)} moved away from the requested document`,
    );
  }
}

function parseBackendNodeReference(nodeRef) {
  if (typeof nodeRef !== "string" || !/^backend:[1-9][0-9]*$/.test(nodeRef)) {
    throw invalidUpload("browser upload requires a valid controller node reference");
  }
  const backendNodeId = Number(nodeRef.slice("backend:".length));
  if (!Number.isSafeInteger(backendNodeId)) {
    throw invalidUpload("browser upload node reference exceeds the safe integer range");
  }
  return backendNodeId;
}

function isWithinRoot(root, candidate) {
  const relative = path.relative(root, candidate);
  return relative === "" || (
    relative !== ".." &&
    !relative.startsWith(`..${path.sep}`) &&
    !path.isAbsolute(relative)
  );
}

function invalidUpload(message) {
  return new BrowserFileTransferError("browser_upload_invalid", message);
}

function utf8ByteLengthAtMost(value, limit) {
  let length = 0;
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    length += codePoint <= 0x7f
      ? 1
      : codePoint <= 0x7ff
        ? 2
        : codePoint <= 0xffff
          ? 3
          : 4;
    if (length > limit) return false;
  }
  return true;
}
