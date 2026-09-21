const MAX_ARTIFACT_PATH_BYTES = 4 * 1024;
const MAX_ARTIFACT_ID_BYTES = 128;
const MAX_STAGE_BYTES = 64 * 1024 * 1024;
const DEFAULT_MAX_ARTIFACTS = 16;
const DEFAULT_MAX_ARTIFACT_BYTES = 256 * 1024 * 1024;
const DEFAULT_MAX_ARTIFACT_AGE_MS = 60_000;

export const UPLOAD_ARTIFACT_KIND = "chariox.sealed-upload-artifact";

export const UPLOAD_STAGING_ERROR_CODES = Object.freeze({
  UNAVAILABLE: "UPLOAD_STAGING_UNAVAILABLE",
  INVALID_ARTIFACT: "UPLOAD_ARTIFACT_INVALID",
  LIMIT: "UPLOAD_STAGING_LIMIT",
  INVALID_INPUT: "UPLOAD_STAGING_INVALID_INPUT",
});

export class UploadStagingError extends Error {
  constructor(code) {
    super(code);
    this.name = "UploadStagingError";
    this.code = code;
  }
}

function fail(code) {
  throw new UploadStagingError(code);
}

function isPlainObject(value) {
  if (value === null || typeof value !== "object") return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function boundedIdentifier(value, maximum) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    Buffer.byteLength(value, "utf8") > maximum ||
    /[\u0000-\u001f\u007f]/.test(value)
  ) {
    fail(UPLOAD_STAGING_ERROR_CODES.INVALID_INPUT);
  }
  return value;
}

function normalizeInput(input) {
  if (!isPlainObject(input)) fail(UPLOAD_STAGING_ERROR_CODES.INVALID_INPUT);
  const tabId = boundedIdentifier(input.tab_id, MAX_ARTIFACT_ID_BYTES);
  const documentId = boundedIdentifier(input.document_id, MAX_ARTIFACT_ID_BYTES);
  if (!Number.isSafeInteger(input.backend_node_id) || input.backend_node_id < 1) {
    fail(UPLOAD_STAGING_ERROR_CODES.INVALID_INPUT);
  }
  return Object.freeze({
    tab_id: tabId,
    document_id: documentId,
    backend_node_id: input.backend_node_id,
  });
}

function inputKey(input) {
  return JSON.stringify([input.tab_id, input.document_id, input.backend_node_id]);
}

function readNow(now) {
  const value = Number(now());
  if (!Number.isFinite(value)) fail(UPLOAD_STAGING_ERROR_CODES.INVALID_INPUT);
  return value;
}

function normalizeStageBytes(request) {
  if (!isPlainObject(request)) fail(UPLOAD_STAGING_ERROR_CODES.INVALID_INPUT);
  const bytes = request.bytes;
  if (
    !(bytes instanceof Uint8Array) ||
    bytes.byteLength > MAX_STAGE_BYTES ||
    !Number.isSafeInteger(request.expectedSize) ||
    request.expectedSize < 0 ||
    request.expectedSize !== bytes.byteLength ||
    !Number.isSafeInteger(request.maxBytes) ||
    request.maxBytes < bytes.byteLength
  ) {
    fail(UPLOAD_STAGING_ERROR_CODES.INVALID_INPUT);
  }
  return bytes;
}

function normalizeArtifact(raw, broker) {
  if (!isPlainObject(raw)) fail(UPLOAD_STAGING_ERROR_CODES.INVALID_ARTIFACT);
  if (
    raw.kind !== UPLOAD_ARTIFACT_KIND ||
    raw.immutable !== true ||
    raw.sealed !== true ||
    raw.separate_uid !== true ||
    raw.broker_owned !== true
  ) {
    fail(UPLOAD_STAGING_ERROR_CODES.INVALID_ARTIFACT);
  }
  const path = boundedIdentifier(raw.path, MAX_ARTIFACT_PATH_BYTES);
  if (!path.startsWith("/")) fail(UPLOAD_STAGING_ERROR_CODES.INVALID_ARTIFACT);
  if (!Number.isSafeInteger(raw.size) || raw.size < 0 || raw.size > MAX_STAGE_BYTES) {
    fail(UPLOAD_STAGING_ERROR_CODES.INVALID_ARTIFACT);
  }
  const artifactId = boundedIdentifier(raw.artifact_id, MAX_ARTIFACT_ID_BYTES);
  const release = typeof raw.release === "function"
    ? raw.release
    : typeof broker?.release === "function"
      ? () => broker.release({ artifact_id: artifactId })
      : null;
  if (release === null) fail(UPLOAD_STAGING_ERROR_CODES.INVALID_ARTIFACT);
  return Object.freeze({
    kind: UPLOAD_ARTIFACT_KIND,
    artifact_id: artifactId,
    path,
    size: raw.size,
    immutable: true,
    sealed: true,
    separate_uid: true,
    broker_owned: true,
    release,
  });
}

/**
 * Retains broker-owned upload artifacts for the lifetime Chromium may read
 * them. The broker must create a sealed artifact in a namespace that the
 * controller/Chromium UID cannot write or unlink. A pathname with 0600/0700
 * permissions is deliberately not accepted as a capability. The explicit
 * separate_uid marker is part of the broker contract so an uncertain or
 * same-UID implementation fails closed.
 */
export class UploadArtifactLeaseStore {
  #broker;
  #now;
  #setTimeout;
  #clearTimeout;
  #maxArtifacts;
  #maxBytes;
  #maxAgeMs;
  #records = new Map();
  #inputs = new Map();

  constructor({
    broker = null,
    now = Date.now,
    setTimeout: setTimeoutFn = setTimeout,
    clearTimeout: clearTimeoutFn = clearTimeout,
    maxArtifacts = DEFAULT_MAX_ARTIFACTS,
    maxBytes = DEFAULT_MAX_ARTIFACT_BYTES,
    maxAgeMs = DEFAULT_MAX_ARTIFACT_AGE_MS,
  } = {}) {
    if (
      (broker !== null && typeof broker?.stage !== "function") ||
      typeof now !== "function" ||
      typeof setTimeoutFn !== "function" ||
      typeof clearTimeoutFn !== "function" ||
      !Number.isSafeInteger(maxArtifacts) ||
      maxArtifacts < 1 ||
      !Number.isSafeInteger(maxBytes) ||
      maxBytes < 1 ||
      !Number.isSafeInteger(maxAgeMs) ||
      maxAgeMs < 1
    ) {
      throw new TypeError("invalid upload artifact lease store dependencies");
    }
    this.#broker = broker;
    this.#now = now;
    this.#setTimeout = setTimeoutFn;
    this.#clearTimeout = clearTimeoutFn;
    this.#maxArtifacts = maxArtifacts;
    this.#maxBytes = maxBytes;
    this.#maxAgeMs = maxAgeMs;
  }

  async stage(request = {}) {
    if (typeof this.#broker?.stage !== "function" || this.#broker.separate_uid !== true) {
      fail(UPLOAD_STAGING_ERROR_CODES.UNAVAILABLE);
    }
    normalizeStageBytes(request);
    let raw;
    try {
      raw = await this.#broker.stage(request);
    } catch (error) {
      if (error instanceof UploadStagingError) throw error;
      fail(UPLOAD_STAGING_ERROR_CODES.UNAVAILABLE);
    }
    return normalizeArtifact(raw, this.#broker);
  }

  async discard(artifacts = []) {
    if (!Array.isArray(artifacts)) return;
    await Promise.all(artifacts.map((artifact) => this.#releaseArtifact(artifact)));
  }

  async assertCanReplace(inputValue, artifacts) {
    await this.#validateReplacement(inputValue, artifacts);
  }

  async replace(inputValue, artifacts) {
    const { input, normalized, oldRecords } = await this.#validateReplacement(inputValue, artifacts);
    for (const record of oldRecords) this.#removeRecord(record);
    const expiresAt = readNow(this.#now) + this.#maxAgeMs;
    const newIds = new Set();
    for (const artifact of normalized) {
      const record = { artifact, input, expiresAt, timer: null };
      this.#records.set(artifact.artifact_id, record);
      newIds.add(artifact.artifact_id);
      record.timer = this.#scheduleExpiry(record);
    }
    this.#inputs.set(inputKey(input), newIds);
    await Promise.all(oldRecords.map((record) => this.#releaseArtifact(record.artifact)));
    return this.snapshot();
  }

  async releaseForDocument(inputValue) {
    if (!isPlainObject(inputValue)) fail(UPLOAD_STAGING_ERROR_CODES.INVALID_INPUT);
    const tabId = boundedIdentifier(inputValue.tab_id, MAX_ARTIFACT_ID_BYTES);
    const documentId = boundedIdentifier(inputValue.document_id, MAX_ARTIFACT_ID_BYTES);
    const records = [...this.#records.values()].filter((record) => (
      record.input.tab_id === tabId && record.input.document_id === documentId
    ));
    await this.#releaseRecords(records);
    return this.snapshot();
  }

  async releaseForOtherDocuments(inputValue) {
    if (!isPlainObject(inputValue)) fail(UPLOAD_STAGING_ERROR_CODES.INVALID_INPUT);
    const tabId = boundedIdentifier(inputValue.tab_id, MAX_ARTIFACT_ID_BYTES);
    const documentId = boundedIdentifier(inputValue.document_id, MAX_ARTIFACT_ID_BYTES);
    const records = [...this.#records.values()].filter((record) => (
      record.input.tab_id === tabId && record.input.document_id !== documentId
    ));
    await this.#releaseRecords(records);
    return this.snapshot();
  }

  async releaseForTab(tabIdValue) {
    const tabId = boundedIdentifier(tabIdValue, MAX_ARTIFACT_ID_BYTES);
    const records = [...this.#records.values()].filter((record) => record.input.tab_id === tabId);
    await this.#releaseRecords(records);
    return this.snapshot();
  }

  async expire() {
    const now = readNow(this.#now);
    const records = [...this.#records.values()].filter((record) => record.expiresAt <= now);
    await this.#releaseRecords(records);
    return this.snapshot();
  }

  async shutdown() {
    const records = [...this.#records.values()];
    for (const record of records) this.#removeRecord(record);
    await Promise.all(records.map((record) => this.#releaseArtifact(record.artifact)));
    return this.snapshot();
  }

  snapshot() {
    let bytes = 0;
    for (const record of this.#records.values()) bytes += record.artifact.size;
    return Object.freeze({ count: this.#records.size, bytes });
  }

  async #validateReplacement(inputValue, artifacts) {
    const input = normalizeInput(inputValue);
    if (!Array.isArray(artifacts) || artifacts.length === 0) {
      fail(UPLOAD_STAGING_ERROR_CODES.INVALID_ARTIFACT);
    }
    await this.expire();
    const normalized = artifacts.map((artifact) => normalizeArtifact(artifact, this.#broker));
    const ids = new Set();
    for (const artifact of normalized) {
      if (ids.has(artifact.artifact_id) || this.#records.has(artifact.artifact_id)) {
        fail(UPLOAD_STAGING_ERROR_CODES.INVALID_ARTIFACT);
      }
      ids.add(artifact.artifact_id);
    }
    const key = inputKey(input);
    const oldIds = this.#inputs.get(key) ?? new Set();
    const oldRecords = [...oldIds].map((id) => this.#records.get(id)).filter(Boolean);
    const retainedCount = this.#records.size - oldRecords.length;
    const retainedBytes = [...this.#records.values()]
      .filter((record) => !oldIds.has(record.artifact.artifact_id))
      .reduce((total, record) => total + record.artifact.size, 0);
    const newBytes = normalized.reduce((total, artifact) => total + artifact.size, 0);
    if (
      retainedCount + normalized.length > this.#maxArtifacts ||
      retainedBytes + newBytes > this.#maxBytes
    ) {
      fail(UPLOAD_STAGING_ERROR_CODES.LIMIT);
    }
    return { input, normalized, oldRecords };
  }

  #scheduleExpiry(record) {
    const delay = Math.max(1, record.expiresAt - readNow(this.#now));
    const timer = this.#setTimeout(() => {
      void this.expire();
    }, delay);
    timer?.unref?.();
    return timer;
  }

  async #releaseRecords(records) {
    for (const record of records) this.#removeRecord(record);
    await Promise.all(records.map((record) => this.#releaseArtifact(record.artifact)));
  }

  #removeRecord(record) {
    if (!record || this.#records.get(record.artifact.artifact_id) !== record) return;
    this.#records.delete(record.artifact.artifact_id);
    if (record.timer !== null) this.#clearTimeout(record.timer);
    const key = inputKey(record.input);
    const ids = this.#inputs.get(key);
    ids?.delete(record.artifact.artifact_id);
    if (ids?.size === 0) this.#inputs.delete(key);
  }

  async #releaseArtifact(artifact) {
    if (!isPlainObject(artifact) || typeof artifact.release !== "function") return;
    try {
      await artifact.release();
    } catch {
      // The lease is removed locally before release. Broker cleanup remains
      // idempotent and its artifact TTL bounds any broker-side residue.
    }
  }
}
