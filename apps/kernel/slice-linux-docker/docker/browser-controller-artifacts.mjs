// MP-08/MP-10/MP-11: bytes from the actual controller browser only.
import { constants } from "node:fs";
import { open, realpath, mkdtemp, writeFile, rm, mkdir } from "node:fs/promises";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import path from "node:path";
import { assertNotCancelled } from "./browser-controller-actions.mjs";

export const MAX_BROWSER_ARTIFACT_BYTES = 8 * 1024 * 1024;
export class BrowserArtifactError extends Error {
  constructor(message) { super(message); this.code = "browser_artifact_invalid"; }
}
const fail = message => new BrowserArtifactError(message);
const digest = bytes => createHash("sha256").update(bytes).digest("hex");
export function artifactBytes(bytes, displayName, mimeType) {
  if (bytes.length > MAX_BROWSER_ARTIFACT_BYTES) throw fail("Browser artifact exceeds the bounded byte range");
  return { data_base64: bytes.toString("base64"), sha256: digest(bytes), size_bytes: bytes.length,
    display_name: safeName(displayName), mime_type: mimeType };
}
export function safeName(value) {
  if (typeof value !== "string" || !value || Buffer.byteLength(value) > 255
      || /[\/\\\x00-\x1f\x7f]/.test(value) || [".", ".."].includes(value)) throw fail("Browser artifact requires a safe filename");
  return value;
}

export async function readCompletedDownload({ directory, downloads, guid, targetId, documentId, browserGeneration }) {
  if (typeof guid !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(guid)) throw fail("invalid observed download GUID");
  const download = downloads.get(guid);
  if (!download || download.state !== "completed" || download.targetId !== targetId
      || download.documentId !== documentId || download.browserGeneration !== browserGeneration) {
    throw fail("download is incomplete, retired, or belongs to another tab/document/browser");
  }
  const root = await realpath(directory);
  const file = await open(path.join(root, guid), constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const before = await file.stat({ bigint: true });
    if (!before.isFile() || before.size < 0n || before.size > BigInt(MAX_BROWSER_ARTIFACT_BYTES)
        || (download.expectedBytes !== undefined && Number(before.size) !== download.expectedBytes)) throw fail("download is not a bounded regular file");
    const bytes = Buffer.alloc(Number(before.size));
    let offset = 0;
    while (offset < bytes.length) {
      const result = await file.read(bytes, offset, bytes.length - offset, offset);
      if (!result.bytesRead) throw fail("download changed while reading");
      offset += result.bytesRead;
    }
    const after = await file.stat({ bigint: true });
    if (["size", "mtimeNs", "ctimeNs", "ino", "dev"].some(key => before[key] !== after[key])) throw fail("download changed while reading");
    const mime = bytes.subarray(0, 5).toString() === "%PDF-" ? "application/pdf"
      : download.filename.endsWith(".txt") ? "text/plain" : "application/octet-stream";
    return artifactBytes(bytes, download.filename, mime);
  } finally { await file.close(); }
}

// MP-08/MP-10/MP-11: encrypted kernel-approved artifact bytes are materialized
// privately, then use the same descriptor-pinned uploader and durable stager.
export async function withUploadArtifacts(entries, signal, upload) {
  if (!Array.isArray(entries) || !entries.length || entries.length > 20) throw fail("invalid upload artifact count");
  let total = 0;
  const decoded = entries.map(entry => {
    const name = safeName(entry?.display_name);
    if (typeof entry?.data_base64 !== "string" || entry.data_base64.length > Math.ceil(MAX_BROWSER_ARTIFACT_BYTES / 3) * 4) throw fail("invalid upload artifact bytes");
    const bytes = Buffer.from(entry.data_base64, "base64");
    total += bytes.length;
    if (bytes.toString("base64") !== entry.data_base64 || total > MAX_BROWSER_ARTIFACT_BYTES
        || bytes.length !== entry.size_bytes || digest(bytes) !== entry.sha256) throw fail("upload artifact byte provenance mismatch");
    return { name, bytes };
  });
  const root = await mkdtemp(path.join(await realpath(tmpdir()), "chariox-browser-artifacts-"));
  try {
    const files = [];
    for (const [index, entry] of decoded.entries()) {
      assertNotCancelled(signal);
      const folder = path.join(root, String(index));
      await mkdir(folder, { mode: 0o700 });
      const file = path.join(folder, entry.name);
      await writeFile(file, entry.bytes, { mode: 0o600, flag: "wx" });
      files.push(file);
    }
    assertNotCancelled(signal);
    return await upload(files, [root]);
  } catch (error) {
    if (error instanceof BrowserArtifactError || error?.code === "browser_action_cancelled" || error?.code?.startsWith("stale_") || error?.code?.startsWith("browser_")) throw error;
    throw fail("private upload artifact materialization failed");
  } finally { await rm(root, { recursive: true, force: true }); }
}

// MP-08/MP-10/MP-11: retain only allowlisted real CDP metadata. Query, fragment,
// credentials, cookie/auth headers, request/response bodies are never retained.
function publicUrl(value) {
  try { const url = new URL(value); if (!["http:", "https:"].includes(url.protocol)) return "";
    return `${url.protocol}//${url.host}${url.pathname}`.slice(0, 4096); } catch { return ""; }
}
function headers(value) {
  return Object.entries(value ?? {}).filter(([key]) => ["accept", "content-type", "content-length"].includes(key.toLowerCase()))
    .filter(([, value]) => typeof value === "string" && Buffer.byteLength(value) <= 1024)
    .map(([name, value]) => ({ name: name.toLowerCase(), value }));
}
export class BrowserPassiveCapture {
  constructor() { this.clear(); }
  clear() { this.entries = []; this.requests = new Map(); this.dropped = 0; }
  record(message, { targetId, documentId }) {
    const p = message.params ?? {};
    if (!targetId || typeof message.sessionId !== "string" || typeof p.requestId !== "string") return;
    const key = `${message.sessionId}:${p.requestId}`;
    let record = this.requests.get(key);
    if (message.method === "Network.requestWillBeSent") {
      if (this.entries.length === 500) { this.entries.shift(); this.dropped++; }
      if (p.redirectResponse && record?.current) {
        this.response(record.current, p.redirectResponse);
        record.current.extra_expected = typeof p.redirectHasExtraInfo === "boolean" ? p.redirectHasExtraInfo : undefined;
      }
      const entry = { target_id: targetId, document_id: p.loaderId || documentId, request_id: p.requestId,
        startedDateTime: Number.isFinite(p.wallTime) ? new Date(p.wallTime * 1000).toISOString() : null,
        request: { method: String(p.request?.method ?? "").slice(0, 16), url: publicUrl(p.request?.url), headers: headers(p.request?.headers) },
        response: { status: 0, headers: [], content: { size: 0, mimeType: "" } },
        bodies_redacted: true };
      Object.defineProperty(entry, "extra_expected", { writable: true, value: undefined });
      record ??= { hops: [], extra: [], current: null };
      record.current = entry; record.hops.push(entry);
      this.requests.set(key, record); this.entries.push(entry);
      this.mergeExtra(record);
    } else if (message.method === "Network.requestWillBeSentExtraInfo") {
      record ??= { hops: [], extra: [], current: null };
      record.extra.push(headers(p.headers)); this.requests.set(key, record); this.mergeExtra(record);
    } else if (message.method === "Network.responseReceived" && record?.current) {
      this.response(record.current, p.response);
      record.current.extra_expected = typeof p.hasExtraInfo === "boolean" ? p.hasExtraInfo : undefined;
      this.mergeExtra(record);
    }
    else if (message.method === "Network.loadingFinished" && record?.current && Number.isFinite(p.encodedDataLength)) record.current.response.content.size = Math.max(0, p.encodedDataLength);
    if (this.requests.size > 500) { this.requests.delete(this.requests.keys().next().value); this.dropped++; }
    if (record?.hops.length > 20 || record?.extra.length > 20) { this.requests.delete(key); this.dropped++; }
  }
  mergeExtra(record) {
    let index = 0;
    for (const entry of record.hops) {
      // MP-08/MP-10/MP-11: CDP may omit extra info for individual hops.
      // Wait for its explicit flag before attributing any queued headers.
      if (entry.extra_expected === undefined) break;
      if (!entry.extra_expected) continue;
      if (index >= record.extra.length) break;
      entry.request.headers = [...new Map([...entry.request.headers, ...record.extra[index++]].map(header => [header.name, header])).values()];
      entry.request.extra_info_observed = true;
    }
  }
  response(entry, response) {
    entry.response.status = Number.isSafeInteger(response?.status) ? response.status : 0;
    entry.response.headers = headers(response?.headers);
    entry.response.content.mimeType = String(response?.mimeType ?? "").slice(0, 128);
  }
  capture(targetId, documentId) {
    return artifactBytes(Buffer.from(JSON.stringify({ log: { version: "1.2", creator: { name: "Chariox actual CDP", version: "420" },
      entries: this.entries.filter(entry => entry.target_id === targetId && entry.document_id === documentId) }, dropped: this.dropped,
      redaction: "cookie/auth headers, query/fragment, bodies omitted" })), "browser-network.har", "application/json");
  }
}
