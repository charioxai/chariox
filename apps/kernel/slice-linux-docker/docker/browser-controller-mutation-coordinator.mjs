import { createHash } from "node:crypto";

const MAX_IDENTIFIER_BYTES = 128;
const MAX_OPERATION_BYTES = 64;
const ATTRIBUTION_FIELDS = Object.freeze([
  "action_id",
  "actor_id",
  "browser_generation",
  "operation",
  "tab_id",
  "target_generation",
]);
const MUTATION_IDENTITY_FIELDS = Object.freeze([
  "target_id",
  "page_id",
  "document_id",
  "arguments",
  "payload",
]);

export const MUTATION_ERROR_CODES = Object.freeze({
  INVALID_ARGUMENT: "MUTATION_INVALID_ARGUMENT",
  ACTION_ID_CONFLICT: "MUTATION_ACTION_ID_CONFLICT",
  BROWSER_GENERATION_STALE: "MUTATION_BROWSER_GENERATION_STALE",
  TAB_GENERATION_STALE: "MUTATION_TAB_GENERATION_STALE",
  QUEUE_SATURATED: "MUTATION_QUEUE_SATURATED",
  CANCELLED: "MUTATION_CANCELLED",
  INDETERMINATE: "MUTATION_INDETERMINATE",
});

export class BrowserMutationError extends Error {
  constructor(code) {
    super(code);
    this.name = "BrowserMutationError";
    this.code = code;
  }
}

function fail(code) {
  throw new BrowserMutationError(code);
}

function isPlainObject(value) {
  if (value === null || typeof value !== "object") return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function identifier(value) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    Buffer.byteLength(value, "utf8") > MAX_IDENTIFIER_BYTES ||
    !/^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(value)
  ) {
    fail(MUTATION_ERROR_CODES.INVALID_ARGUMENT);
  }
  return value;
}

function positiveInteger(value) {
  if (!Number.isSafeInteger(value) || value < 1) {
    fail(MUTATION_ERROR_CODES.INVALID_ARGUMENT);
  }
  return value;
}

function normalizeAttribution(value) {
  if (!isPlainObject(value)) fail(MUTATION_ERROR_CODES.INVALID_ARGUMENT);
  const allowed = new Set([...ATTRIBUTION_FIELDS, ...MUTATION_IDENTITY_FIELDS]);
  if (Object.keys(value).some((key) => !allowed.has(key))) {
    fail(MUTATION_ERROR_CODES.INVALID_ARGUMENT);
  }
  for (const key of ATTRIBUTION_FIELDS) {
    if (!Object.prototype.hasOwnProperty.call(value, key)) {
      fail(MUTATION_ERROR_CODES.INVALID_ARGUMENT);
    }
  }
  if (
    typeof value.operation !== "string" ||
    value.operation.length === 0 ||
    Buffer.byteLength(value.operation, "utf8") > MAX_OPERATION_BYTES ||
    !/^[a-z][a-z0-9_]*$/.test(value.operation)
  ) {
    fail(MUTATION_ERROR_CODES.INVALID_ARGUMENT);
  }
  return Object.freeze({
    action_id: identifier(value.action_id),
    actor_id: identifier(value.actor_id),
    browser_generation: positiveInteger(value.browser_generation),
    operation: value.operation,
    tab_id: identifier(value.tab_id),
    target_generation: positiveInteger(value.target_generation),
  });
}

function optionalIdentifier(value) {
  return value === undefined || value === null ? null : identifier(value);
}

function normalizeJsonValue(value, seen = new Set()) {
  if (value === undefined || value === null) return null;
  if (typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) fail(MUTATION_ERROR_CODES.INVALID_ARGUMENT);
    return Object.is(value, -0) ? 0 : value;
  }
  if (!Array.isArray(value) && !isPlainObject(value)) {
    fail(MUTATION_ERROR_CODES.INVALID_ARGUMENT);
  }
  if (seen.has(value)) fail(MUTATION_ERROR_CODES.INVALID_ARGUMENT);
  seen.add(value);
  let normalized;
  if (Array.isArray(value)) {
    normalized = value.map((entry) => normalizeJsonValue(entry, seen));
  } else {
    normalized = Object.create(null);
    for (const key of Object.keys(value).sort()) {
      if (value[key] !== undefined) {
        normalized[key] = normalizeJsonValue(value[key], seen);
      }
    }
  }
  seen.delete(value);
  return normalized;
}

function normalizeMutationIdentity(rawAttribution, rawIdentity) {
  const inlineIdentity = {};
  for (const key of MUTATION_IDENTITY_FIELDS) {
    if (Object.prototype.hasOwnProperty.call(rawAttribution, key)) {
      inlineIdentity[key] = rawAttribution[key];
    }
  }
  if (rawIdentity !== undefined) {
    if (!isPlainObject(rawIdentity)) fail(MUTATION_ERROR_CODES.INVALID_ARGUMENT);
    const allowed = new Set(MUTATION_IDENTITY_FIELDS);
    if (Object.keys(rawIdentity).some((key) => !allowed.has(key))) {
      fail(MUTATION_ERROR_CODES.INVALID_ARGUMENT);
    }
    Object.assign(inlineIdentity, rawIdentity);
  }
  return Object.freeze({
    target_id: optionalIdentifier(inlineIdentity.target_id),
    page_id: optionalIdentifier(inlineIdentity.page_id),
    document_id: optionalIdentifier(inlineIdentity.document_id),
    arguments: normalizeJsonValue(inlineIdentity.arguments),
    payload: normalizeJsonValue(inlineIdentity.payload),
  });
}

function fingerprint(attribution, identity) {
  const semanticIdentity = {
    ...attribution,
    target_id: identity.target_id,
    page_id: identity.page_id,
    document_id: identity.document_id,
    arguments: identity.arguments,
    payload: identity.payload,
  };
  const semanticDigest = createHash("sha256")
    .update(JSON.stringify(semanticIdentity))
    .digest("hex");
  return JSON.stringify({ ...attribution, semantic_digest: semanticDigest });
}

function finiteLimit(value, fallback) {
  const result = value ?? fallback;
  if (!Number.isSafeInteger(result) || result < 1) {
    throw new TypeError("coordinator limits must be positive integers");
  }
  return result;
}

export class BrowserMutationCoordinator {
  constructor(options = {}) {
    if (!isPlainObject(options)) throw new TypeError("options must be a plain object");
    this.maxTabs = finiteLimit(options.maxTabs, 256);
    this.maxQueuedPerTab = finiteLimit(options.maxQueuedPerTab, 16);
    this.maxCompleted = finiteLimit(options.maxCompleted, 1024);
    this.maxInvalidatedTabs = finiteLimit(options.maxInvalidatedTabs, this.maxTabs);
    this.browserGeneration = options.browserGeneration === undefined
      ? null
      : positiveInteger(options.browserGeneration);
    this.queues = new Map();
    this.active = new Map();
    this.actions = new Map();
    this.completedOrder = [];
    this.invalidatedTargetGeneration = new Map();
    this.invalidationOverflowed = false;
  }

  mutate(rawAttribution, run, rawIdentity) {
    const attribution = normalizeAttribution(rawAttribution);
    if (typeof run !== "function") fail(MUTATION_ERROR_CODES.INVALID_ARGUMENT);
    const identity = normalizeMutationIdentity(rawAttribution, rawIdentity);
    const actionFingerprint = fingerprint(attribution, identity);
    const existing = this.actions.get(attribution.action_id);
    if (existing) {
      if (existing.fingerprint !== actionFingerprint) {
        fail(MUTATION_ERROR_CODES.ACTION_ID_CONFLICT);
      }
      return existing.promise;
    }

    if (this.invalidationOverflowed) {
      fail(MUTATION_ERROR_CODES.QUEUE_SATURATED);
    }
    this._admitGeneration(attribution.browser_generation);
    const invalidatedGeneration = this.invalidatedTargetGeneration.get(attribution.tab_id) ?? 0;
    if (attribution.target_generation <= invalidatedGeneration) {
      fail(MUTATION_ERROR_CODES.TAB_GENERATION_STALE);
    }

    let queue = this.queues.get(attribution.tab_id);
    if (!queue) {
      const admittedTabs = new Set([...this.queues.keys(), ...this.active.keys()]);
      if (!admittedTabs.has(attribution.tab_id) && admittedTabs.size >= this.maxTabs) {
        fail(MUTATION_ERROR_CODES.QUEUE_SATURATED);
      }
      queue = [];
      this.queues.set(attribution.tab_id, queue);
    }
    if (queue.length >= this.maxQueuedPerTab) {
      fail(MUTATION_ERROR_CODES.QUEUE_SATURATED);
    }

    const abortController = new AbortController();
    let resolvePromise;
    let rejectPromise;
    const promise = new Promise((resolve, reject) => {
      resolvePromise = resolve;
      rejectPromise = reject;
    });
    const record = {
      actionId: attribution.action_id,
      attribution,
      abortController,
      fingerprint: actionFingerprint,
      promise,
      reject: rejectPromise,
      resolve: resolvePromise,
      run,
      outcome: null,
    };
    this.actions.set(attribution.action_id, record);
    queue.push(record);
    this._pump(attribution.tab_id);
    return promise;
  }

  invalidateTab(tabId, targetGeneration) {
    const normalizedTabId = identifier(tabId);
    const normalizedGeneration = positiveInteger(targetGeneration);
    const previous = this.invalidatedTargetGeneration.get(normalizedTabId) ?? 0;
    if (previous > 0 || this.invalidatedTargetGeneration.size < this.maxInvalidatedTabs) {
      this.invalidatedTargetGeneration.set(normalizedTabId, Math.max(previous, normalizedGeneration));
    } else {
      // Preserve safety with bounded memory: after exact tombstone capacity is
      // exhausted, reject every mutation until a new browser generation begins.
      this.invalidationOverflowed = true;
    }
    this._cancelQueued(
      normalizedTabId,
      MUTATION_ERROR_CODES.CANCELLED,
      (record) => record.attribution.target_generation <= normalizedGeneration,
    );
    const active = this.active.get(normalizedTabId);
    if (active && active.attribution.target_generation <= normalizedGeneration) {
      active.abortController.abort(MUTATION_ERROR_CODES.INDETERMINATE);
    }
  }

  advanceBrowserGeneration(nextGeneration) {
    const normalizedGeneration = positiveInteger(nextGeneration);
    if (this.browserGeneration !== null && normalizedGeneration <= this.browserGeneration) {
      fail(MUTATION_ERROR_CODES.BROWSER_GENERATION_STALE);
    }
    this.browserGeneration = normalizedGeneration;
    for (const tabId of [...this.queues.keys()]) {
      this._cancelQueued(tabId, MUTATION_ERROR_CODES.CANCELLED);
    }
    for (const record of this.active.values()) {
      record.abortController.abort(MUTATION_ERROR_CODES.INDETERMINATE);
    }
    this.invalidatedTargetGeneration.clear();
    this.invalidationOverflowed = false;
  }

  snapshot() {
    const active = [...this.active.values()]
      .map((record) => record.attribution)
      .sort((left, right) => left.tab_id.localeCompare(right.tab_id));
    const queued = [...this.queues.entries()]
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([tabId, records]) => ({
        tab_id: tabId,
        action_ids: records.map((record) => record.attribution.action_id),
      }));
    return {
      browser_generation: this.browserGeneration,
      active,
      queued,
      completed_count: this.completedOrder.length,
      invalidated_tab_count: this.invalidatedTargetGeneration.size,
      invalidation_overflowed: this.invalidationOverflowed,
    };
  }

  _admitGeneration(generation) {
    if (this.browserGeneration === null) {
      this.browserGeneration = generation;
    } else if (generation !== this.browserGeneration) {
      fail(MUTATION_ERROR_CODES.BROWSER_GENERATION_STALE);
    }
  }

  _cancelQueued(tabId, code, shouldCancel = () => true) {
    const queue = this.queues.get(tabId) ?? [];
    const retained = [];
    for (const record of queue) {
      if (!shouldCancel(record)) {
        retained.push(record);
        continue;
      }
      const error = new BrowserMutationError(code);
      record.outcome = "rejected";
      record.reject(error);
      this._retainCompleted(record, record.actionId);
      this._releaseExecution(record);
    }
    if (retained.length === 0) this.queues.delete(tabId);
    else this.queues.set(tabId, retained);
  }

  _pump(tabId) {
    if (this.active.has(tabId)) return;
    const queue = this.queues.get(tabId);
    const record = queue?.shift();
    if (!record) {
      this.queues.delete(tabId);
      return;
    }
    if (queue.length === 0) this.queues.delete(tabId);
    this.active.set(tabId, record);
    Promise.resolve()
      .then(() => {
        if (record.abortController.signal.aborted) {
          throw new BrowserMutationError(MUTATION_ERROR_CODES.INDETERMINATE);
        }
        return record.run({ signal: record.abortController.signal, attribution: record.attribution });
      })
      .then(
        (result) => this._settle(record, "fulfilled", result),
        (error) => this._settle(record, "rejected", error),
      );
  }

  _settle(record, outcome, value) {
    const { tab_id: tabId } = record.attribution;
    const wasAborted = record.abortController.signal.aborted;
    const terminalOutcome = wasAborted ? "rejected" : outcome;
    const terminalValue = wasAborted
      ? new BrowserMutationError(MUTATION_ERROR_CODES.INDETERMINATE)
      : value;
    record.outcome = terminalOutcome;
    if (terminalOutcome === "fulfilled") {
      record.resolve(terminalValue);
    } else {
      record.reject(terminalValue);
    }
    if (this.active.get(tabId) === record) this.active.delete(tabId);
    this._retainCompleted(record, record.actionId);
    this._releaseExecution(record);
    this._pump(tabId);
  }

  _releaseExecution(record) {
    delete record.actionId;
    delete record.attribution;
    delete record.run;
    delete record.resolve;
    delete record.reject;
    delete record.abortController;
    Object.freeze(record);
  }

  _retainCompleted(record, actionId) {
    this.completedOrder.push(actionId);
    while (this.completedOrder.length > this.maxCompleted) {
      const actionId = this.completedOrder.shift();
      if (this.actions.get(actionId)?.outcome !== null) this.actions.delete(actionId);
    }
  }
}
