const DEFAULT_MAX_EVENTS = 2_048;
const MAX_MAX_EVENTS = 16_384;
const DEFAULT_MAX_BYTES = 1_048_576;
const MAX_MAX_BYTES = 16 * 1_024 * 1_024;
const DEFAULT_MAX_EVENT_BYTES = 64 * 1_024;
const MAX_MAX_EVENT_BYTES = 256 * 1_024;
const DEFAULT_MAX_SERIALIZED_BYTES = 512 * 1_024;
const MAX_MAX_SERIALIZED_BYTES = 2 * 1_024 * 1_024;
const DEFAULT_POLL_LIMIT = 100;
const MAX_POLL_LIMIT = 200;
const MAX_ID_BYTES = 256;
const MAX_URL_BYTES = 2_048;
const MAX_SEEN_KEYS = 65_536;
const MAX_SERIALIZATION_NODES = 4_096;
const MIN_VALID_JSON_BYTES = 4;
const DEFAULT_MAX_LIFECYCLE_ENTRIES = 8_192;
const MAX_MAX_LIFECYCLE_ENTRIES = 65_536;
const REDACTED = "[redacted]";
const SAFE_FILENAME_EXTENSIONS = new Set([
  ".bin",
  ".csv",
  ".doc",
  ".docx",
  ".gif",
  ".html",
  ".jpeg",
  ".jpg",
  ".json",
  ".log",
  ".pdf",
  ".png",
  ".svg",
  ".txt",
  ".webp",
  ".xml",
  ".zip",
]);

const UTF8_ENCODER = typeof TextEncoder === "function" ? new TextEncoder() : null;

export const BROWSER_EVENT_LIMITS = Object.freeze({
  defaultMaxEvents: DEFAULT_MAX_EVENTS,
  maxMaxEvents: MAX_MAX_EVENTS,
  defaultMaxBytes: DEFAULT_MAX_BYTES,
  maxMaxBytes: MAX_MAX_BYTES,
  defaultMaxEventBytes: DEFAULT_MAX_EVENT_BYTES,
  maxMaxEventBytes: MAX_MAX_EVENT_BYTES,
  defaultMaxSerializedBytes: DEFAULT_MAX_SERIALIZED_BYTES,
  maxMaxSerializedBytes: MAX_MAX_SERIALIZED_BYTES,
  defaultMaxLifecycleEntries: DEFAULT_MAX_LIFECYCLE_ENTRIES,
  maxMaxLifecycleEntries: MAX_MAX_LIFECYCLE_ENTRIES,
  defaultPollLimit: DEFAULT_POLL_LIMIT,
  maxPollLimit: MAX_POLL_LIMIT,
});

export const DEFAULT_EVENT_CAPACITY = DEFAULT_MAX_EVENTS;
export const MAX_EVENT_CAPACITY = MAX_MAX_EVENTS;
export const DEFAULT_EVENT_BYTES = DEFAULT_MAX_BYTES;
export const MAX_EVENT_BYTES = MAX_MAX_BYTES;

export const BROWSER_EVENT_KINDS = Object.freeze([
  "console",
  "network_request",
  "network_response",
  "network_failed",
  "page_navigated",
  "dom_content_loaded",
  "page_loaded",
  "page_lifecycle",
  "frame_attached",
  "frame_detached",
  "dialog_opened",
  "dialog_closed",
  "target_created",
  "target_changed",
  "target_destroyed",
  "target_crashed",
  "download_started",
  "download_progress",
  "browser_connected",
  "browser_disconnected",
]);

const EVENT_KIND_SET = new Set(BROWSER_EVENT_KINDS);
export class BrowserEventError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserEventError";
    this.code = code;
  }
}

export class BrowserEventJournal {
  constructor(options = {}) {
    const maxEvents =
      options.maxEvents ?? options.maxCount ?? options.maxEventCount ?? options.capacity ?? DEFAULT_MAX_EVENTS;
    const maxBytes = options.maxBytes ?? options.byteBudget ?? DEFAULT_MAX_BYTES;
    const maxEventBytes =
      options.maxEventBytes ?? options.eventBytes ?? Math.min(DEFAULT_MAX_EVENT_BYTES, maxBytes);
    const maxSerializedBytes =
      options.maxSerializedBytes ?? options.serializationBytes ?? DEFAULT_MAX_SERIALIZED_BYTES;
    const maxLifecycleEntries =
      options.maxLifecycleEntries ??
      options.lifecycleEntries ??
      Math.min(MAX_MAX_LIFECYCLE_ENTRIES, Math.max(64, maxEvents * 4));

    validateLimit("browser event count", maxEvents, MAX_MAX_EVENTS, "browser_event_capacity_invalid");
    validateLimit("browser event byte budget", maxBytes, MAX_MAX_BYTES, "browser_event_bytes_invalid");
    validateLimit(
      "browser event size",
      maxEventBytes,
      MAX_MAX_EVENT_BYTES,
      "browser_event_size_invalid",
    );
    validateLimit(
      "browser event serialization budget",
      maxSerializedBytes,
      MAX_MAX_SERIALIZED_BYTES,
      "browser_event_serialization_invalid",
    );
    validateJournalSerializationBudget(maxSerializedBytes);
    validateLimit(
      "browser event lifecycle entries",
      maxLifecycleEntries,
      MAX_MAX_LIFECYCLE_ENTRIES,
      "browser_event_lifecycle_invalid",
    );
    if (maxEventBytes > maxBytes) {
      throw new BrowserEventError(
        "browser_event_size_invalid",
        "browser event size cannot exceed the journal byte budget",
      );
    }

    this.maxEvents = maxEvents;
    this.capacity = maxEvents;
    this.maxBytes = maxBytes;
    this.maxEventBytes = maxEventBytes;
    this.maxSerializedBytes = maxSerializedBytes;
    this.maxLifecycleEntries = maxLifecycleEntries;
    this.events = [];
    this.bytes = 0;
    this.nextSequenceId = 1;
    this.browserGeneration = 0;
    this.invalidTargets = new Set();
    this.invalidDocuments = new Set();
    this.targetDocuments = new Map();
    this.activeTargets = new Map();
    this.activeDocuments = new Map();
    this.compactedTargets = new Map();
    this.compactedDocuments = new Map();
    this.trustedTargets = new Set();
    this.trustedDocuments = new Set();
    this.targetAuthorityFences = new Map();
    this.documentAuthorityFences = new Map();
    this.targetLifecycleGap = false;
    this.documentLifecycleGap = false;
    this.seen = new Map();
  }

  get size() {
    return this.events.length;
  }

  get byteLength() {
    return this.bytes;
  }

  cursor() {
    return this.nextSequenceId - 1;
  }

  record(input, context = {}) {
    if (input?.method) return this.recordCdp(input, context);

    const generation = this._acceptGeneration(input, context);
    if (generation === null) return null;
    const mapped = mapDirectEvent(input, context);
    if (!mapped) return null;
    return this._recordMapped(mapped, {
      generation,
      source: input,
      context,
    });
  }

  append(input, context = {}) {
    return this.record(input, context);
  }

  recordCdp(message, context = {}) {
    const generation = this._acceptGeneration(message, context);
    if (generation === null) return null;
    const mapped = mapCdpEvent(message, context);
    if (!mapped) return null;
    return this._recordMapped(mapped, {
      generation,
      source: message,
      context,
    });
  }

  poll(options = {}) {
    const {
      cursor = 0,
      limit = DEFAULT_POLL_LIMIT,
      actorId,
      tabId,
      maxSerializedBytes,
    } = options;
    if (!Number.isSafeInteger(cursor) || cursor < 0) {
      throw new BrowserEventError(
        "browser_event_cursor_invalid",
        "browser event cursor must be a non-negative integer",
      );
    }
    validateLimit("browser event limit", limit, MAX_POLL_LIMIT, "browser_event_limit_invalid");
    const serializationBudget = maxSerializedBytes ?? this.maxSerializedBytes;
    validateJournalSerializationBudget(serializationBudget);

    const expectedGeneration = readRequestedGeneration(options) ?? this.browserGeneration;
    const currentCursor = this.cursor();
    const oldestEventId = this.events[0]?.sequence_id ?? this.nextSequenceId;
    if (
      expectedGeneration !== this.browserGeneration ||
      cursor > currentCursor ||
      (this.events.length > 0 && cursor + 1 < oldestEventId)
    ) {
      return this._boundedBatch(
        {
          browser_generation: this.browserGeneration,
          events: [],
          next_cursor: currentCursor,
          replay_gap: true,
        },
        { cursor, maxSerializedBytes: serializationBudget },
      );
    }

    const wantedActor = normalizeIdentity(actorId);
    const wantedTab = normalizeIdentity(tabId);
    const events = [];
    let examinedCursor = cursor;
    for (const event of this.events) {
      if (event.sequence_id <= cursor) continue;
      examinedCursor = event.sequence_id;
      if (wantedActor !== null && event.actor_id !== wantedActor) continue;
      if (wantedTab !== null && event.tab_id !== wantedTab) continue;
      events.push(cloneJsonValue(event));
      if (events.length >= limit) break;
    }

    return this._boundedBatch(
      {
        browser_generation: this.browserGeneration,
        events,
        next_cursor: examinedCursor,
        replay_gap: false,
        high_water_cursor: examinedCursor,
      },
      { cursor, maxSerializedBytes: serializationBudget },
    );
  }

  snapshot({ maxSerializedBytes } = {}) {
    const serializationBudget = maxSerializedBytes ?? this.maxSerializedBytes;
    validateJournalSerializationBudget(serializationBudget);
    return this._boundedBatch(
      {
        browser_generation: this.browserGeneration,
        events: this.events.map(cloneJsonValue),
        next_cursor: this.cursor(),
        replay_gap: false,
        high_water_cursor: this.cursor(),
      },
      { maxSerializedBytes: serializationBudget },
    );
  }

  serialize(options = {}) {
    const maxBytes = options.maxBytes ?? this.maxSerializedBytes;
    validateJournalSerializationBudget(maxBytes);
    const value = Number.isSafeInteger(options.cursor)
      ? this.poll({ ...options, maxSerializedBytes: maxBytes })
      : this.snapshot({ maxSerializedBytes: maxBytes });
    return boundedCompleteSerialize(value, maxBytes);
  }

  toJSON() {
    return this.snapshot();
  }

  invalidateTarget(targetOrOptions, options = {}) {
    const input = identityOptions(targetOrOptions, options);
    const targetId = normalizeIdentity(input.targetId ?? input.target_id);
    if (targetId === null || !this._acceptInvalidationGeneration(input)) return false;
    const targetAuthorityGeneration = readOptionalLifecycleGeneration(input, "target");
    const explicitDocumentId = normalizeIdentity(input.documentId ?? input.document_id);
    const documentAuthorityGeneration = explicitDocumentId
      ? readOptionalLifecycleGeneration(input, "document")
      : null;
    const knownDocument = this.targetDocuments.get(targetId);
    this.invalidTargets.delete(targetId);
    this.invalidTargets.add(targetId);
    this.trustedTargets.delete(targetId);
    this.targetAuthorityFences.set(
      targetId,
      targetAuthorityGeneration,
    );
    if (knownDocument) this._markInvalidDocument(knownDocument);
    if (explicitDocumentId) {
      this._markInvalidDocument(explicitDocumentId);
      this.documentAuthorityFences.set(explicitDocumentId, documentAuthorityGeneration);
    }
    this.activeTargets.delete(targetId);
    this.targetDocuments.delete(targetId);
    this._trimLifecycleState();
    return true;
  }

  invalidateDocument(documentOrOptions, options = {}) {
    const input = identityOptions(documentOrOptions, options);
    const documentId = normalizeIdentity(input.documentId ?? input.document_id ?? input.id);
    if (documentId === null || !this._acceptInvalidationGeneration(input)) return false;
    const documentAuthorityGeneration = readOptionalLifecycleGeneration(input, "document");
    this._markInvalidDocument(documentId);
    this.trustedDocuments.delete(documentId);
    this.documentAuthorityFences.set(documentId, documentAuthorityGeneration);
    for (const [targetId, currentDocument] of this.targetDocuments) {
      if (currentDocument === documentId) this.targetDocuments.delete(targetId);
    }
    this.activeDocuments.delete(documentId);
    this._trimLifecycleState();
    return true;
  }

  restoreTarget(targetId) {
    const normalized = normalizeIdentity(targetId);
    if (normalized === null) return false;
    const wasInvalid = this.invalidTargets.delete(normalized);
    this.compactedTargets.delete(normalized);
    this.targetAuthorityFences.delete(normalized);
    this.trustedTargets.add(normalized);
    this.activeTargets.set(normalized, null);
    if (wasInvalid) this._clearSeenFor({ targetId: normalized });
    this._trimLifecycleState();
    return true;
  }

  restoreDocument(documentId) {
    const normalized = normalizeIdentity(documentId);
    if (normalized === null) return false;
    const wasInvalid = this.invalidDocuments.delete(normalized);
    this.compactedDocuments.delete(normalized);
    this.documentAuthorityFences.delete(normalized);
    this.trustedDocuments.add(normalized);
    this.activeDocuments.set(normalized, null);
    if (wasInvalid) this._clearSeenFor({ documentId: normalized });
    this._trimLifecycleState();
    return true;
  }

  isTargetInvalid(targetId) {
    return this.invalidTargets.has(normalizeIdentity(targetId));
  }

  isDocumentInvalid(documentId) {
    return this.invalidDocuments.has(normalizeIdentity(documentId));
  }

  _acceptGeneration(source, context) {
    const incoming = readIncomingGeneration(source, context);
    if (incoming === null && this.browserGeneration === 0) return null;
    if (incoming === null) return this.browserGeneration;
    if (this.browserGeneration === 0) {
      this._resetGeneration(incoming);
      return incoming;
    }
    if (incoming < this.browserGeneration) return null;
    if (incoming > this.browserGeneration) this._resetGeneration(incoming);
    return this.browserGeneration;
  }

  _resetGeneration(generation) {
    this.browserGeneration = generation;
    this.events = [];
    this.bytes = 0;
    this.nextSequenceId = 1;
    this.invalidTargets.clear();
    this.invalidDocuments.clear();
    this.targetDocuments.clear();
    this.activeTargets.clear();
    this.activeDocuments.clear();
    this.compactedTargets.clear();
    this.compactedDocuments.clear();
    this.trustedTargets.clear();
    this.trustedDocuments.clear();
    this.targetAuthorityFences.clear();
    this.documentAuthorityFences.clear();
    this.targetLifecycleGap = false;
    this.documentLifecycleGap = false;
    this.seen.clear();
  }

  _acceptInvalidationGeneration(input) {
    const generation = readGeneration(input);
    if (generation === null) return this.browserGeneration > 0;
    if (this.browserGeneration === 0) {
      this._resetGeneration(generation);
      return true;
    }
    return generation === this.browserGeneration;
  }

  _recordMapped(mapped, { generation, source, context }) {
    const dedupKey = makeDedupKey(source, mapped, generation);
    const prior = dedupKey === null ? null : this.seen.get(dedupKey);
    if (prior) return cloneJsonValue(prior.event);

    const authority = resolveLifecycleAuthority(context, mapped);
    if (!this._admitLifecycle(mapped, authority)) return null;
    if (this._isInvalid(mapped)) return null;

    const sequenceId = this.nextSequenceId;
    const baseEvent = {
      sequence_id: sequenceId,
      event_id: sequenceId,
      browser_generation: generation,
      actor_id: mapped.actorId ?? null,
      tab_id: mapped.tabId ?? null,
      target_id: mapped.targetId ?? null,
      document_id: mapped.documentId ?? null,
      kind: mapped.kind,
      data: mapped.data,
    };
    const event = fitEvent(baseEvent, this.maxEventBytes);
    if (!event) return null;
    const eventBytes = utf8ByteLength(completeStableStringify(event));
    if (eventBytes > this.maxBytes) return null;

    while (
      this.events.length >= this.maxEvents ||
      (this.events.length > 0 && this.bytes + eventBytes > this.maxBytes)
    ) {
      const evicted = this.events.shift();
      this.bytes -= utf8ByteLength(completeStableStringify(evicted));
      this._forgetSeen(evicted);
    }
    if (this.bytes + eventBytes > this.maxBytes) return null;

    this.events.push(event);
    this.bytes += eventBytes;
    this.nextSequenceId += 1;
    if (dedupKey !== null) {
      this.seen.set(dedupKey, {
        event,
        targetId: event.target_id,
        documentId: event.document_id,
      });
    }
    this._trimSeen();
    this._prepareLifecycle(mapped);
    this._recordActiveLifecycle(mapped, authority);
    this._finishLifecycle(mapped);
    return cloneJsonValue(event);
  }

  _prepareLifecycle(mapped) {
    if (mapped.kind !== "page_navigated") return;
    if (mapped.targetId) {
      const oldDocument = this.targetDocuments.get(mapped.targetId);
      if (oldDocument && oldDocument !== mapped.documentId) {
        this.invalidateDocument(oldDocument);
      }
    }
  }

  _finishLifecycle(mapped) {
    if (mapped.kind === "target_destroyed" || mapped.kind === "target_crashed") {
      if (mapped.targetId) this.invalidateTarget(mapped.targetId);
      if (mapped.documentId) this.invalidateDocument(mapped.documentId);
    }
  }

  _isInvalid(mapped) {
    return Boolean(
      (mapped.targetId && this.invalidTargets.has(mapped.targetId)) ||
        (mapped.documentId && this.invalidDocuments.has(mapped.documentId)),
    );
  }

  _admitLifecycle(mapped, authority) {
    if (mapped.targetId) {
      const targetId = mapped.targetId;
      const targetGeneration = authority.targetGeneration;
      const activeGeneration = this.activeTargets.get(targetId);
      if (this.invalidTargets.has(targetId)) {
        const compactedFence = this.compactedTargets.get(targetId);
        if (
          !authority.targetAuthoritative ||
          !this._passesAuthorityFence(
            this.targetAuthorityFences,
            targetId,
            targetGeneration,
            compactedFence,
          )
        ) {
          return false;
        }
        this.invalidTargets.delete(targetId);
        this.targetAuthorityFences.delete(targetId);
        this.compactedTargets.delete(targetId);
      } else if (
        this.compactedTargets.has(targetId) &&
        !this.trustedTargets.has(targetId) &&
        !this.activeTargets.has(targetId)
      ) {
        if (!authority.targetAuthoritative) this._lifecycleEvidenceGap("target");
        if (!this._passesAuthorityFence(
          this.targetAuthorityFences,
          targetId,
          targetGeneration,
          this.compactedTargets.get(targetId),
        )) {
          return false;
        }
        this.compactedTargets.delete(targetId);
        this.targetAuthorityFences.delete(targetId);
      } else if (
        this.targetLifecycleGap &&
        !this.trustedTargets.has(targetId) &&
        !this.activeTargets.has(targetId) &&
        !authority.targetAuthoritative
      ) {
        this._lifecycleEvidenceGap("target");
      }
      if (
        activeGeneration !== undefined &&
        targetGeneration !== null &&
        activeGeneration !== null &&
        targetGeneration < activeGeneration
      ) {
        return false;
      }
    }

    if (mapped.documentId) {
      const documentId = mapped.documentId;
      const documentGeneration = authority.documentGeneration;
      const activeGeneration = this.activeDocuments.get(documentId);
      if (this.invalidDocuments.has(documentId)) {
        const compactedFence = this.compactedDocuments.get(documentId);
        if (
          !authority.documentAuthoritative ||
          !this._passesAuthorityFence(
            this.documentAuthorityFences,
            documentId,
            documentGeneration,
            compactedFence,
          )
        ) {
          return false;
        }
        this.invalidDocuments.delete(documentId);
        this.documentAuthorityFences.delete(documentId);
        this.compactedDocuments.delete(documentId);
      } else if (
        this.compactedDocuments.has(documentId) &&
        !this.trustedDocuments.has(documentId) &&
        !this.activeDocuments.has(documentId)
      ) {
        if (!authority.documentAuthoritative) this._lifecycleEvidenceGap("document");
        if (!this._passesAuthorityFence(
          this.documentAuthorityFences,
          documentId,
          documentGeneration,
          this.compactedDocuments.get(documentId),
        )) {
          return false;
        }
        this.compactedDocuments.delete(documentId);
        this.documentAuthorityFences.delete(documentId);
      } else if (
        this.documentLifecycleGap &&
        !this.trustedDocuments.has(documentId) &&
        !this.activeDocuments.has(documentId) &&
        !authority.documentAuthoritative
      ) {
        this._lifecycleEvidenceGap("document");
      }
      if (
        activeGeneration !== undefined &&
        documentGeneration !== null &&
        activeGeneration !== null &&
        documentGeneration < activeGeneration
      ) {
        return false;
      }
    }
    return true;
  }

  _passesAuthorityFence(fences, identity, generation, fallbackFence = undefined) {
    if (!Number.isSafeInteger(generation) || generation <= 0) return false;
    const fence = fences.has(identity) ? fences.get(identity) : fallbackFence;
    if (fence !== undefined && fence !== null && generation <= fence) return false;
    return true;
  }

  _lifecycleEvidenceGap(kind) {
    throw new BrowserEventError(
      "browser_event_lifecycle_gap",
      `authoritative ${kind} lifecycle generation is required after bounded compaction`,
    );
  }

  _recordActiveLifecycle(mapped, authority) {
    if (mapped.targetId) {
      this.activeTargets.delete(mapped.targetId);
      this.activeTargets.set(
        mapped.targetId,
        authority.targetGeneration,
      );
    }
    if (mapped.documentId) {
      this.activeDocuments.delete(mapped.documentId);
      this.activeDocuments.set(
        mapped.documentId,
        authority.documentGeneration,
      );
    }
    if (mapped.targetId && mapped.documentId) {
      this.targetDocuments.delete(mapped.targetId);
      this.targetDocuments.set(mapped.targetId, mapped.documentId);
    }
    this._trimLifecycleState();
  }

  _clearSeenFor({ targetId, documentId }) {
    for (const [key, entry] of this.seen) {
      if (
        (targetId && entry.targetId === targetId) ||
        (documentId && entry.documentId === documentId)
      ) {
        this.seen.delete(key);
      }
    }
  }

  _forgetSeen(event) {
    for (const [key, entry] of this.seen) {
      if (entry.event.sequence_id === event.sequence_id) this.seen.delete(key);
    }
  }

  _trimSeen() {
    while (this.seen.size > Math.min(MAX_SEEN_KEYS, Math.max(64, this.maxEvents * 4))) {
      this.seen.delete(this.seen.keys().next().value);
    }
  }

  _markInvalidDocument(documentId) {
    this.invalidDocuments.delete(documentId);
    this.invalidDocuments.add(documentId);
    this.activeDocuments.delete(documentId);
    for (const [targetId, currentDocument] of this.targetDocuments) {
      if (currentDocument === documentId) this.targetDocuments.delete(targetId);
    }
  }

  _trimLifecycleState() {
    while (this.invalidTargets.size > this.maxLifecycleEntries) {
      const identity = this.invalidTargets.values().next().value;
      const fence = this.targetAuthorityFences.get(identity) ?? null;
      this.invalidTargets.delete(identity);
      this._compactLifecycle("target", identity, fence);
      this.targetAuthorityFences.delete(identity);
    }
    while (this.invalidDocuments.size > this.maxLifecycleEntries) {
      const identity = this.invalidDocuments.values().next().value;
      const fence = this.documentAuthorityFences.get(identity) ?? null;
      this.invalidDocuments.delete(identity);
      this._compactLifecycle("document", identity, fence);
      this.documentAuthorityFences.delete(identity);
    }
    while (this.activeTargets.size > this.maxLifecycleEntries) {
      this.activeTargets.delete(this.activeTargets.keys().next().value);
    }
    while (this.activeDocuments.size > this.maxLifecycleEntries) {
      this.activeDocuments.delete(this.activeDocuments.keys().next().value);
    }
    while (this.trustedTargets.size > this.maxLifecycleEntries) {
      this.trustedTargets.delete(this.trustedTargets.values().next().value);
    }
    while (this.trustedDocuments.size > this.maxLifecycleEntries) {
      this.trustedDocuments.delete(this.trustedDocuments.values().next().value);
    }
    while (this.targetDocuments.size > this.maxLifecycleEntries) {
      this.targetDocuments.delete(this.targetDocuments.keys().next().value);
    }
    while (this.targetAuthorityFences.size > this.maxLifecycleEntries) {
      const identity = this.targetAuthorityFences.keys().next().value;
      const fence = this.targetAuthorityFences.get(identity) ?? null;
      this.targetAuthorityFences.delete(identity);
      this._compactLifecycle("target", identity, fence);
    }
    while (this.documentAuthorityFences.size > this.maxLifecycleEntries) {
      const identity = this.documentAuthorityFences.keys().next().value;
      const fence = this.documentAuthorityFences.get(identity) ?? null;
      this.documentAuthorityFences.delete(identity);
      this._compactLifecycle("document", identity, fence);
    }
  }

  _compactLifecycle(kind, identity, fence) {
    const compacted = kind === "target" ? this.compactedTargets : this.compactedDocuments;
    compacted.delete(identity);
    compacted.set(identity, fence ?? null);
    while (compacted.size > this.maxLifecycleEntries) {
      compacted.delete(compacted.keys().next().value);
      if (kind === "target") this.targetLifecycleGap = true;
      else this.documentLifecycleGap = true;
    }
  }

  _boundedBatch(batch, { cursor = null, maxSerializedBytes = this.maxSerializedBytes } = {}) {
    const events = batch.events.map(cloneJsonValue);
    const makeResult = (end) => {
      const retainedEvents = events.slice(0, end);
      const deliveredCursor =
        retainedEvents.at(-1)?.sequence_id ??
        (Number.isSafeInteger(cursor)
          ? cursor
          : events[0]?.sequence_id !== undefined
            ? Math.max(0, events[0].sequence_id - 1)
            : 0);
      const nextCursor = end < events.length
        ? deliveredCursor
        : batch.replay_gap === true || batch.head_discarded === true
          ? batch.next_cursor
          : Number.isSafeInteger(batch.next_cursor)
            ? batch.next_cursor
            : deliveredCursor;
      const result = {
        browser_generation: batch.browser_generation,
        events: retainedEvents,
        next_cursor: nextCursor,
        replay_gap: batch.replay_gap === true || batch.head_discarded === true,
      };
      if (end < events.length) {
        result.high_water_cursor = Number.isSafeInteger(batch.high_water_cursor)
          ? batch.high_water_cursor
          : batch.next_cursor;
      }
      return result;
    };
    const fits = (end) =>
      utf8ByteLength(completeStableStringify(makeResult(end))) <= maxSerializedBytes;

    if (fits(events.length)) return makeResult(events.length);
    if (events.length === 0 || !fits(0)) {
      throw new BrowserEventError(
        "browser_event_serialization_impossible",
        "browser event serialization budget cannot fit the required envelope",
      );
    }

    let low = 0;
    let high = events.length - 1;
    let best = 0;
    while (low <= high) {
      const middle = Math.floor((low + high) / 2);
      if (fits(middle)) {
        best = middle;
        low = middle + 1;
      } else {
        high = middle - 1;
      }
    }
    return makeResult(best);
  }
}

export function sanitizeUrl(value) {
  if (typeof value !== "string" || value.length === 0) return "";
  if (value.length > 16_384) return "[redacted]";
  try {
    const parsed = new URL(value);
    if (parsed.protocol === "http:" || parsed.protocol === "https:") {
      return boundedString(parsed.origin, MAX_URL_BYTES);
    }
    if (parsed.protocol === "about:") {
      return boundedString(value.split(/[?#]/, 1)[0], 128);
    }
    return boundedString(parsed.protocol, 32);
  } catch {
    return "[invalid]";
  }
}

export const sanitizedUrl = sanitizeUrl;

export function redactHeaders(headers, { maxHeaders = 64 } = {}) {
  if (!isRecord(headers)) return {};
  const result = {};
  const names = Object.keys(headers)
    .sort()
    .slice(0, Math.max(0, Math.min(64, maxHeaders)));
  for (const name of names) {
    const normalized = boundedString(name.toLowerCase(), 128);
    if (!normalized || isSensitiveKey(normalized)) continue;
    result[normalized] = REDACTED;
  }
  return result;
}

export function redactValue(key, value, maxBytes = MAX_URL_BYTES) {
  if (isSensitiveKey(key)) return REDACTED;
  if (typeof value === "string") return boundedString(value, maxBytes);
  if (typeof value === "boolean") return value;
  if (typeof value === "number" && Number.isFinite(value)) return value;
  return null;
}

export function boundedSerialize(value, maxBytes = DEFAULT_MAX_SERIALIZED_BYTES) {
  validateSerializationBudget(maxBytes);
  const serialized = stableStringify(value);
  if (utf8ByteLength(serialized) <= maxBytes) return serialized;
  const marker = '{"truncated":true}';
  if (utf8ByteLength(marker) <= maxBytes) return marker;
  return "null";
}

function mapCdpEvent(message, context) {
  if (!isRecord(message)) return null;
  const method = typeof message.method === "string" ? message.method : "";
  const params = isRecord(message.params) ? message.params : {};
  const base = resolveAttribution(message, context);
  const hasOverride = (name, overrides) =>
    Object.prototype.hasOwnProperty.call(overrides, name) && overrides[name] !== undefined;
  const makeEvent = (kind, data, overrides = {}) => ({
    kind,
    actorId: hasOverride("actorId", overrides) ? overrides.actorId : base.actorId,
    tabId: hasOverride("tabId", overrides) ? overrides.tabId : base.tabId,
    targetId: hasOverride("targetId", overrides) ? overrides.targetId : base.targetId,
    documentId: hasOverride("documentId", overrides) ? overrides.documentId : base.documentId,
    data,
    sourceId: explicitSourceId(message, params),
  });

  switch (method) {
    case "Runtime.consoleAPICalled":
      return makeEvent("console", {
        console_type: boundedString(params.type, 64),
        argument_count: boundedCount(params.args?.length),
      });
    case "Network.requestWillBeSent":
      return makeEvent("network_request", {
        request_id: safeIdentity(params.requestId),
        method: boundedString(params.request?.method, 32),
        url: sanitizeUrl(params.request?.url),
        resource_type: boundedString(params.type, 64),
      });
    case "Network.responseReceived":
      return makeEvent("network_response", {
        request_id: safeIdentity(params.requestId),
        status: boundedStatus(params.response?.status),
        url: sanitizeUrl(params.response?.url),
        resource_type: boundedString(params.type, 64),
        mime_type: boundedString(params.response?.mimeType, 128),
      });
    case "Network.loadingFailed":
      return makeEvent("network_failed", {
        request_id: safeIdentity(params.requestId),
        error_text: sanitizedNetworkError(params.errorText),
        canceled: params.canceled === true,
        resource_type: boundedString(params.type, 64),
      });
    case "Page.frameNavigated": {
      const frame = isRecord(params.frame) ? params.frame : {};
      if (frame.parentId) return null;
      const targetId = base.targetId ?? safeIdentity(frame.targetId);
      const attribution = resolveAttribution(message, context, {
        targetId,
        documentId: safeIdentity(frame.loaderId),
      });
      return makeEvent(
        "page_navigated",
        { url: sanitizeUrl(frame.url) },
        attribution,
      );
    }
    case "Page.domContentEventFired":
      return makeEvent("dom_content_loaded", {});
    case "Page.loadEventFired":
      return makeEvent("page_loaded", {});
    case "Page.lifecycleEvent":
      return makeEvent("page_lifecycle", {
        lifecycle_name: boundedString(params.name, 64),
      });
    case "Page.frameAttached":
      return makeEvent("frame_attached", { frame_id: safeIdentity(params.frameId) });
    case "Page.frameDetached":
      return makeEvent("frame_detached", { frame_id: safeIdentity(params.frameId) });
    case "Page.javascriptDialogOpening":
      return makeEvent("dialog_opened", {
        dialog_type: boundedString(params.type, 32),
        has_message: typeof params.message === "string" && params.message.length > 0,
        has_default_prompt:
          typeof params.defaultPrompt === "string" && params.defaultPrompt.length > 0,
      });
    case "Page.javascriptDialogClosed":
      return makeEvent("dialog_closed", {
        result: params.result === true,
        user_input_present:
          typeof params.userInput === "string" && params.userInput.length > 0,
      });
    case "Target.targetCreated":
    case "Target.targetInfoChanged": {
      const target = isRecord(params.targetInfo) ? params.targetInfo : {};
      if (target.type !== "page") return null;
      const targetId = safeIdentity(target.targetId);
      const attribution = resolveAttribution(message, context, { targetId, documentId: null });
      return makeEvent(
        method === "Target.targetCreated" ? "target_created" : "target_changed",
        { url: sanitizeUrl(target.url) },
        attribution,
      );
    }
    case "Target.targetDestroyed": {
      const targetId = safeIdentity(params.targetId);
      const attribution = resolveAttribution(message, context, { targetId, documentId: null });
      return makeEvent("target_destroyed", {}, attribution);
    }
    case "Target.targetCrashed": {
      const targetId = safeIdentity(params.targetId) ?? base.targetId;
      const attribution = resolveAttribution(message, context, { targetId });
      return makeEvent("target_crashed", {
        status: boundedString(params.status, 64),
        error_code: boundedErrorCode(params.errorCode),
      }, attribution);
    }
    case "Inspector.targetCrashed":
      return makeEvent("target_crashed", {
        status: "crashed",
        error_code: null,
      });
    case "Browser.downloadWillBegin": {
      const targetId = safeCall(context?.targetIdForFrame, params.frameId) ?? base.targetId;
      const attribution = resolveAttribution(message, context, { targetId, documentId: null });
      return makeEvent("download_started", {
        guid: safeIdentity(params.guid),
        url: sanitizeUrl(params.url),
        ...filenameProjection(params.suggestedFilename),
      }, attribution);
    }
    case "Browser.downloadProgress": {
      const targetId = safeCall(context?.targetIdForDownload, params.guid) ?? base.targetId;
      const attribution = resolveAttribution(message, context, { targetId, documentId: null });
      const reason = safeCall(context?.downloadCancellationReason, params.guid);
      return makeEvent("download_progress", {
        guid: safeIdentity(params.guid),
        state: boundedString(params.state, 32),
        received_bytes: boundedNumber(params.receivedBytes),
        total_bytes: boundedNumber(params.totalBytes),
        ...(safeDownloadReason(reason) ? { cancellation_reason: safeDownloadReason(reason) } : {}),
      }, attribution);
    }
    case "Chariox.browserConnected":
      return makeEvent("browser_connected", {}, { targetId: null, documentId: null });
    case "Chariox.browserDisconnected":
      return makeEvent("browser_disconnected", {}, { targetId: null, documentId: null });
    default:
      return null;
  }
}

function mapDirectEvent(input, context) {
  if (!isRecord(input)) return null;
  const kind = typeof input.kind === "string" ? input.kind : input.type;
  if (!EVENT_KIND_SET.has(kind)) return null;
  const data = isRecord(input.data) ? input.data : input;
  const base = resolveAttribution(input, context, {
    actorId: input.actorId ?? input.actor_id,
    tabId: input.tabId ?? input.tab_id,
    targetId: input.targetId ?? input.target_id,
    documentId: input.documentId ?? input.document_id,
  });
  return {
    kind,
    actorId: base.actorId,
    tabId: base.tabId,
    targetId: base.targetId,
    documentId: base.documentId,
    data: mapDirectData(kind, data),
    sourceId: explicitSourceId(input, data),
  };
}

function mapDirectData(kind, data) {
  switch (kind) {
    case "console":
      return {
        console_type: boundedString(data.console_type ?? data.consoleType ?? data.type, 64),
        argument_count: boundedCount(data.argument_count ?? data.argumentCount ?? data.args?.length),
      };
    case "network_request":
      return {
        request_id: safeIdentity(data.request_id ?? data.requestId),
        method: boundedString(data.method, 32),
        url: sanitizeUrl(data.url),
        resource_type: boundedString(data.resource_type ?? data.resourceType, 64),
      };
    case "network_response":
      return {
        request_id: safeIdentity(data.request_id ?? data.requestId),
        status: boundedStatus(data.status),
        url: sanitizeUrl(data.url),
        resource_type: boundedString(data.resource_type ?? data.resourceType, 64),
        mime_type: boundedString(data.mime_type ?? data.mimeType, 128),
      };
    case "network_failed":
      return {
        request_id: safeIdentity(data.request_id ?? data.requestId),
        error_text: sanitizedNetworkError(data.error_text ?? data.errorText),
        canceled: data.canceled === true,
        resource_type: boundedString(data.resource_type ?? data.resourceType, 64),
      };
    case "page_navigated":
    case "target_created":
    case "target_changed":
      return { url: sanitizeUrl(data.url) };
    case "page_lifecycle":
      return { lifecycle_name: boundedString(data.lifecycle_name ?? data.name, 64) };
    case "frame_attached":
    case "frame_detached":
      return { frame_id: safeIdentity(data.frame_id ?? data.frameId) };
    case "dom_content_loaded":
    case "page_loaded":
    case "target_destroyed":
    case "browser_connected":
    case "browser_disconnected":
      return {};
    case "dialog_opened":
      return {
        dialog_type: boundedString(data.dialog_type ?? data.type, 32),
        has_message: data.has_message === true || data.hasMessage === true,
        has_default_prompt: data.has_default_prompt === true || data.hasDefaultPrompt === true,
      };
    case "dialog_closed":
      return {
        result: data.result === true,
        user_input_present: data.user_input_present === true || data.userInputPresent === true,
      };
    case "target_crashed":
      return {
        status: boundedString(data.status, 64),
        error_code: boundedErrorCode(data.error_code ?? data.errorCode),
      };
    case "download_started":
      return {
        guid: safeIdentity(data.guid),
        url: sanitizeUrl(data.url),
        ...filenameProjection(data.suggested_filename ?? data.suggestedFilename),
      };
    case "download_progress": {
      const reason = data.cancellation_reason ?? data.cancellationReason;
      return {
        guid: safeIdentity(data.guid),
        state: boundedString(data.state, 32),
        received_bytes: boundedNumber(data.received_bytes ?? data.receivedBytes),
        total_bytes: boundedNumber(data.total_bytes ?? data.totalBytes),
        ...(safeDownloadReason(reason) ? { cancellation_reason: safeDownloadReason(reason) } : {}),
      };
    }
    default:
      return {};
  }
}

function resolveAttribution(message, context = {}, overrides = {}) {
  const sessionId = message?.sessionId ?? message?.session_id;
  const frameId = message?.params?.frameId ?? message?.params?.frame_id;
  const hasOverride = (name) =>
    Object.prototype.hasOwnProperty.call(overrides, name) && overrides[name] !== undefined;
  const targetId = normalizeIdentity(
    hasOverride("targetId")
      ? overrides.targetId
      : safeCall(context.targetIdForSession, sessionId) ??
        safeCall(context.targetIdForFrame, frameId) ??
        context.targetId ??
        context.target_id ??
        message?.targetId ??
        message?.target_id,
  );
  const documentId = normalizeIdentity(
    hasOverride("documentId")
      ? overrides.documentId
      : safeCall(context.documentIdForTarget, targetId) ??
        context.documentId ??
        context.document_id ??
        message?.documentId ??
        message?.document_id,
  );
  const tabId = normalizeIdentity(
    hasOverride("tabId")
      ? overrides.tabId
      : safeCall(context.tabIdForTarget, targetId) ??
        safeCall(context.tabIdForSession, sessionId) ??
        safeCall(context.tabIdForFrame, frameId) ??
        context.tabId ??
        context.tab_id ??
        message?.tabId ??
        message?.tab_id,
  );
  const actorId = normalizeIdentity(
    hasOverride("actorId")
      ? overrides.actorId
      : safeCall(context.actorIdForTarget, targetId) ??
        safeCall(context.actorIdForSession, sessionId) ??
        context.actorId ??
        context.actor_id ??
        message?.actorId ??
        message?.actor_id,
  );
  return { actorId, tabId, targetId, documentId };
}

function explicitSourceId(message, params) {
  const candidates = [
    message?.eventId,
    message?.event_id,
    message?.sequenceId,
    message?.sequence_id,
    message?.meta?.eventId,
    message?.sourceId,
    message?.source_id,
    params?.eventId,
    params?.event_id,
    params?.sequenceId,
    params?.sequence_id,
    params?.sourceId,
    params?.source_id,
  ];
  for (const candidate of candidates) {
    if (typeof candidate === "string" && candidate.length > 0) return boundedString(candidate, MAX_ID_BYTES);
    if (Number.isSafeInteger(candidate) && candidate >= 0) return String(candidate);
  }
  return null;
}

function makeDedupKey(source, mapped, generation) {
  const sourceId = mapped.sourceId;
  if (sourceId === null) return null;
  const identity = {
    generation,
    source_id: sourceId,
    source_method: source?.method ?? source?.kind ?? source?.type ?? null,
  };
  return completeStableStringify(identity);
}

function fitEvent(event, maxBytes) {
  let candidate = event;
  if (utf8ByteLength(completeStableStringify(candidate)) <= maxBytes) return candidate;
  candidate = { ...event, data: {}, data_truncated: true };
  if (utf8ByteLength(completeStableStringify(candidate)) <= maxBytes) return candidate;
  candidate = {
    sequence_id: event.sequence_id,
    event_id: event.event_id,
    browser_generation: event.browser_generation,
    kind: event.kind,
    data: {},
    truncated: true,
  };
  return utf8ByteLength(completeStableStringify(candidate)) <= maxBytes ? candidate : null;
}

function fitValue(value, depth = 0, seen = new Set(), state = { nodes: 0 }) {
  state.nodes += 1;
  if (state.nodes > MAX_SERIALIZATION_NODES) return "[truncated]";
  if (depth > 8) return "[truncated]";
  if (value === null || typeof value === "boolean") return value;
  if (typeof value === "string") return boundedString(value, MAX_URL_BYTES);
  if (typeof value === "number") return Number.isFinite(value) ? value : null;
  if (typeof value === "bigint") return null;
  if (typeof value !== "object") return null;
  if (seen.has(value)) return "[circular]";
  seen.add(value);
  if (Array.isArray(value)) {
    const result = value
      .slice(0, 128)
      .map((entry) => fitValue(entry, depth + 1, seen, state));
    seen.delete(value);
    return result;
  }
  const result = {};
  for (const key of Object.keys(value).sort().slice(0, 128)) {
    if (isSensitiveKey(key)) continue;
    try {
      result[key] = fitValue(value[key], depth + 1, seen, state);
    } catch {
      result[key] = "[unavailable]";
    }
  }
  seen.delete(value);
  return result;
}

function stableStringify(value) {
  return JSON.stringify(fitValue(value));
}

function cloneJsonValue(value) {
  return JSON.parse(completeStableStringify(value));
}

function completeStableStringify(value) {
  return JSON.stringify(completeJsonValue(value));
}

function completeJsonValue(value) {
  if (value === null || typeof value === "boolean" || typeof value === "string") return value;
  if (typeof value === "number") return Number.isFinite(value) ? value : null;
  if (Array.isArray(value)) return value.map(completeJsonValue);
  if (value && typeof value === "object") {
    const result = {};
    for (const key of Object.keys(value).sort()) result[key] = completeJsonValue(value[key]);
    return result;
  }
  return null;
}

function boundedCompleteSerialize(value, maxBytes) {
  const serialized = completeStableStringify(value);
  if (utf8ByteLength(serialized) <= maxBytes) return serialized;
  throw new BrowserEventError(
    "browser_event_serialization_impossible",
    "browser event serialization exceeded its validated budget",
  );
}

function utf8ByteLength(value) {
  if (UTF8_ENCODER) return UTF8_ENCODER.encode(value).byteLength;
  return unescape(encodeURIComponent(value)).length;
}

function boundedString(value, maxBytes) {
  if (typeof value !== "string") return "";
  const limit = Number.isSafeInteger(maxBytes) && maxBytes > 0 ? maxBytes : 1;
  let result = "";
  let bytes = 0;
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    const safeCharacter = codePoint < 0x20 || codePoint === 0x7f ? "�" : character;
    const width = utf8ByteLength(safeCharacter);
    if (bytes + width > limit) break;
    result += safeCharacter;
    bytes += width;
  }
  return result;
}

function safeIdentity(value) {
  const normalized = normalizeIdentity(value);
  return normalized === null ? null : boundedString(normalized, MAX_ID_BYTES);
}

function normalizeIdentity(value) {
  if (typeof value !== "string" || value.length === 0) return null;
  return boundedString(value, MAX_ID_BYTES);
}

function boundedCount(value) {
  return Number.isSafeInteger(value) && value >= 0 ? Math.min(value, 10_000) : 0;
}

function boundedNumber(value) {
  return Number.isFinite(value) && value >= 0 ? Math.min(Math.floor(value), Number.MAX_SAFE_INTEGER) : 0;
}

function boundedStatus(value) {
  return Number.isFinite(value) && value >= 0 ? Math.min(Math.floor(value), 999) : 0;
}

function boundedErrorCode(value) {
  return Number.isSafeInteger(value) ? value : null;
}

function sanitizedNetworkError(value) {
  if (typeof value !== "string") return "";
  return /^net::[A-Z0-9_]+$/.test(value) ? value : REDACTED;
}

function filenameProjection(value) {
  if (typeof value !== "string" || value.length === 0 || value.length > 16_384) return {};
  const filename = value.split(/[\\/]/).at(-1);
  const match = /\.([a-z0-9]{1,16})$/i.exec(filename);
  if (!match) return {};
  const extension = `.${match[1].toLowerCase()}`;
  return SAFE_FILENAME_EXTENSIONS.has(extension)
    ? { suggested_extension: extension }
    : {};
}

function safeDownloadReason(value) {
  return ["canceled", "interrupted", "user_canceled", "disk_full", "dangerous"].includes(value)
    ? value
    : null;
}

function isSensitiveKey(value) {
  const key = String(value).toLowerCase().replace(/[^a-z0-9]/g, "");
  return (
    key === "authorization" ||
    key === "proxyauthorization" ||
    key === "cookie" ||
    key === "setcookie" ||
    key === "xcsrftoken" ||
    key === "csrf" ||
    key.includes("token") ||
    key.includes("secret") ||
    key.includes("password") ||
    key.includes("credential") ||
    key.includes("body") ||
    key.includes("postdata") ||
    key.includes("userinput") ||
    key === "prompt" ||
    key.includes("prompttext") ||
    key === "defaultprompt"
  );
}

function validateLimit(label, value, maximum, code) {
  if (!Number.isSafeInteger(value) || value <= 0 || value > maximum) {
    throw new BrowserEventError(code, `${label} must be between 1 and ${maximum}`);
  }
}

function validateSerializationBudget(value) {
  validateLimit(
    "browser event serialization budget",
    value,
    MAX_MAX_SERIALIZED_BYTES,
    "browser_event_serialization_invalid",
  );
  if (value < MIN_VALID_JSON_BYTES) {
    throw new BrowserEventError(
      "browser_event_serialization_invalid",
      `browser event serialization budget must be at least ${MIN_VALID_JSON_BYTES} bytes`,
    );
  }
}

function validateJournalSerializationBudget(value) {
  validateSerializationBudget(value);
  const minimum = minimumJournalEnvelopeBytes();
  if (value < minimum) {
    throw new BrowserEventError(
      "browser_event_serialization_invalid",
      `browser event serialization budget must be at least ${minimum} bytes for a journal envelope`,
    );
  }
}

function minimumJournalEnvelopeBytes() {
  const base = {
    browser_generation: 0,
    events: [],
    next_cursor: 0,
    replay_gap: false,
  };
  const continuation = { ...base, high_water_cursor: 0 };
  return Math.max(
    utf8ByteLength(completeStableStringify(base)),
    utf8ByteLength(completeStableStringify(continuation)),
  );
}

function generationState(value) {
  if (!isRecord(value)) return { present: false, valid: true, value: null };
  const keys = ["browserGeneration", "browser_generation", "generation"].filter((key) =>
    Object.prototype.hasOwnProperty.call(value, key),
  );
  if (keys.length === 0) return { present: false, valid: true, value: null };
  let candidate;
  for (const key of keys) {
    let raw;
    try {
      raw = value[key];
    } catch {
      return { present: true, valid: false, value: null };
    }
    if (!Number.isSafeInteger(raw) || raw <= 0) {
      return { present: true, valid: false, value: null };
    }
    if (candidate === undefined) candidate = raw;
    else if (candidate !== raw) return { present: true, valid: false, value: null };
  }
  return { present: true, valid: true, value: candidate };
}

function throwInvalidGeneration() {
  throw new BrowserEventError(
    "browser_event_generation_invalid",
    "browser event generation must be a positive safe integer",
  );
}

function readGeneration(value) {
  const state = generationState(value);
  if (!state.present) return null;
  if (!state.valid) throwInvalidGeneration();
  return state.value;
}

function readIncomingGeneration(source, context) {
  const contextState = generationState(context);
  const sourceState = generationState(source);
  if ((contextState.present && !contextState.valid) || (sourceState.present && !sourceState.valid)) {
    throwInvalidGeneration();
  }
  if (contextState.present) return contextState.value;
  if (sourceState.present) return sourceState.value;
  return null;
}

function readRequestedGeneration(options) {
  return readGeneration(options);
}

function resolveLifecycleAuthority(context, mapped) {
  const targetGeneration = readAuthoritativeLifecycleGeneration(
    context,
    "target",
    mapped.targetId,
  );
  const documentGeneration = readAuthoritativeLifecycleGeneration(
    context,
    "document",
    mapped.documentId,
  );
  return {
    targetGeneration,
    documentGeneration,
    targetAuthoritative: targetGeneration !== null,
    documentAuthoritative: documentGeneration !== null,
  };
}

function readAuthoritativeLifecycleGeneration(context, kind, identity) {
  if (!identity || !isRecord(context)) return null;
  const suffix = kind === "target" ? "Target" : "Document";
  const callbacks = [
    context[`active${suffix}GenerationFor${suffix}`],
    context[`${kind}GenerationFor${suffix}`],
    context[`get${suffix}Generation`],
  ];
  for (const callback of callbacks) {
    const candidate = safeCall(callback, identity);
    if (candidate !== null && candidate !== undefined) {
      return normalizeLifecycleGeneration(candidate);
    }
  }

  const maps = [
    context[`active${suffix}Generations`],
    context[`${kind}Generations`],
    context[`active_${kind}_generations`],
  ];
  for (const map of maps) {
    const candidate = lookupIdentity(map, identity);
    if (candidate !== undefined && candidate !== null) {
      return normalizeLifecycleGeneration(candidate);
    }
  }

  for (const key of [
    `active${suffix}Generation`,
    `${kind}Generation`,
    `active_${kind}_generation`,
    `${kind}_generation`,
  ]) {
    if (Object.prototype.hasOwnProperty.call(context, key)) {
      return normalizeLifecycleGeneration(context[key]);
    }
  }
  return null;
}

function lookupIdentity(container, identity) {
  if (container instanceof Map) return container.get(identity);
  if (isRecord(container) && Object.prototype.hasOwnProperty.call(container, identity)) {
    return container[identity];
  }
  return undefined;
}

function normalizeLifecycleGeneration(value) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new BrowserEventError(
      "browser_event_lifecycle_generation_invalid",
      "active lifecycle generation must be a positive safe integer",
    );
  }
  return value;
}

function readOptionalLifecycleGeneration(value, kind) {
  if (!isRecord(value)) return null;
  const keys = [
    `${kind}Generation`,
    `${kind}_generation`,
    `${kind}Epoch`,
    `${kind}_epoch`,
  ].filter((key) => Object.prototype.hasOwnProperty.call(value, key));
  if (keys.length === 0) return null;
  const candidate = value[keys[0]];
  return normalizeLifecycleGeneration(candidate);
}

function identityOptions(value, options) {
  if (typeof value === "string") return { ...options, targetId: value, id: value };
  return { ...(isRecord(value) ? value : {}), ...options };
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function safeCall(fn, ...args) {
  if (typeof fn !== "function") return null;
  try {
    return fn(...args);
  } catch {
    return null;
  }
}
