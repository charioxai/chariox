const MAX_IDENTIFIER_LENGTH = 128;
const MAX_WEBSOCKET_URL_LENGTH = 2048;

export const ERROR_CODES = Object.freeze({
  INVALID_ARGUMENT: "INVALID_ARGUMENT",
  INVALID_GENERATION: "INVALID_GENERATION",
  INVALID_TARGET: "INVALID_TARGET",
  TARGET_NOT_FOUND: "TARGET_NOT_FOUND",
  TAB_INVALIDATED: "TAB_INVALIDATED",
  STALE_GENERATION: "STALE_GENERATION",
  STALE_TARGET_GENERATION: "STALE_TARGET_GENERATION",
  VIEWPORT_BOUNDS: "VIEWPORT_BOUNDS",
  VIEWPORT_OWNER_MISMATCH: "VIEWPORT_OWNER_MISMATCH",
  VIEWPORT_OWNER_REQUIRED: "VIEWPORT_OWNER_REQUIRED",
  VIEWPORT_VERSION_REQUIRED: "VIEWPORT_VERSION_REQUIRED",
  STALE_VIEWPORT_VERSION: "STALE_VIEWPORT_VERSION",
});

const ERROR_MESSAGES = Object.freeze({
  INVALID_ARGUMENT: "Invalid registry argument.",
  INVALID_GENERATION: "Invalid browser generation.",
  INVALID_TARGET: "Invalid CDP target.",
  TARGET_NOT_FOUND: "The requested tab was not found.",
  TAB_INVALIDATED: "The tab identity has been invalidated.",
  STALE_GENERATION: "The browser generation is stale.",
  STALE_TARGET_GENERATION: "The target generation is stale.",
  VIEWPORT_BOUNDS: "The viewport is outside the supported bounds.",
  VIEWPORT_OWNER_MISMATCH: "The viewport owner does not match.",
  VIEWPORT_OWNER_REQUIRED: "A viewport owner is required.",
  VIEWPORT_VERSION_REQUIRED: "A viewport version is required.",
  STALE_VIEWPORT_VERSION: "The viewport version is stale.",
});

export const VIEWPORT_LIMITS = Object.freeze({
  minWidth: 320,
  maxWidth: 7680,
  minHeight: 200,
  maxHeight: 4320,
});

export const DEFAULT_VIEWPORT = Object.freeze({
  width: 1280,
  height: 800,
});

export class TabRegistryError extends Error {
  constructor(code) {
    super(ERROR_MESSAGES[code] ?? ERROR_MESSAGES.INVALID_ARGUMENT);
    this.name = "TabRegistryError";
    this.code = code in ERROR_MESSAGES ? code : ERROR_CODES.INVALID_ARGUMENT;
  }

  toJSON() {
    return { code: this.code, message: this.message };
  }
}

function fail(code) {
  throw new TabRegistryError(code);
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requireIdentifier(value, code = ERROR_CODES.INVALID_ARGUMENT) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > MAX_IDENTIFIER_LENGTH ||
    !/^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(value)
  ) {
    fail(code);
  }
  return value;
}

function requireGeneration(value) {
  if (!Number.isSafeInteger(value) || value < 1) {
    fail(ERROR_CODES.INVALID_GENERATION);
  }
  return value;
}

function optionalWebSocketUrl(value) {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value !== "string" || value.length === 0 || value.length > MAX_WEBSOCKET_URL_LENGTH) {
    fail(ERROR_CODES.INVALID_TARGET);
  }
  try {
    const parsed = new URL(value);
    if (parsed.protocol !== "ws:" && parsed.protocol !== "wss:") {
      fail(ERROR_CODES.INVALID_TARGET);
    }
  } catch {
    fail(ERROR_CODES.INVALID_TARGET);
  }
  return value;
}

function normalizeTarget(input) {
  if (!isPlainObject(input)) {
    fail(ERROR_CODES.INVALID_TARGET);
  }
  const targetId = input.target_id ?? input.targetId ?? input.id;
  const targetType = input.target_type ?? input.targetType ?? input.type ?? "page";
  if (typeof targetType !== "string" || targetType.length === 0 || targetType.length > 64) {
    fail(ERROR_CODES.INVALID_TARGET);
  }
  requireIdentifier(targetId, ERROR_CODES.INVALID_TARGET);
  const websocketUrl = optionalWebSocketUrl(input.websocket_url ?? input.websocketUrl ?? input.webSocketDebuggerUrl);
  return { targetId, targetType, websocketUrl };
}

function compareStrings(left, right) {
  if (left < right) return -1;
  if (left > right) return 1;
  return 0;
}

function compareTargets(left, right) {
  return (
    compareStrings(left.targetId, right.targetId) ||
    compareStrings(left.targetType, right.targetType) ||
    compareStrings(left.websocketUrl ?? "", right.websocketUrl ?? "")
  );
}

function cloneTab(record) {
  return {
    tab_id: record.tabId,
    target_id: record.targetId,
    target_type: record.targetType,
    generation: record.generation,
    target_generation: record.targetGeneration,
  };
}

function resultWithFlags(snapshot, changed, idempotent) {
  return { ...snapshot, changed, idempotent };
}

function requireDimension(value) {
  if (!Number.isSafeInteger(value)) {
    fail(ERROR_CODES.VIEWPORT_BOUNDS);
  }
  return value;
}

function readOwnerVersion(ownerOrRequest, maybeVersion) {
  if (isPlainObject(ownerOrRequest)) {
    return {
      ownerId: ownerOrRequest.owner_id ?? ownerOrRequest.ownerId,
      version: ownerOrRequest.version ?? maybeVersion,
    };
  }
  return { ownerId: ownerOrRequest, version: maybeVersion };
}

export class CanonicalViewport {
  constructor(options = {}) {
    if (!isPlainObject(options)) {
      fail(ERROR_CODES.INVALID_ARGUMENT);
    }
    const limits = options.limits ?? VIEWPORT_LIMITS;
    if (
      !isPlainObject(limits) ||
      !Number.isSafeInteger(limits.minWidth) ||
      !Number.isSafeInteger(limits.maxWidth) ||
      !Number.isSafeInteger(limits.minHeight) ||
      !Number.isSafeInteger(limits.maxHeight) ||
      limits.minWidth < 1 ||
      limits.maxWidth < limits.minWidth ||
      limits.minHeight < 1 ||
      limits.maxHeight < limits.minHeight
    ) {
      fail(ERROR_CODES.INVALID_ARGUMENT);
    }
    this._limits = Object.freeze({ ...limits });
    const initial = options.initial ?? DEFAULT_VIEWPORT;
    const width = requireDimension(initial.width);
    const height = requireDimension(initial.height);
    this._assertBounds(width, height);
    this._state = { width, height, owner_id: null, version: 0 };
    this._lastResize = null;
  }

  _assertBounds(width, height) {
    if (
      width < this._limits.minWidth ||
      width > this._limits.maxWidth ||
      height < this._limits.minHeight ||
      height > this._limits.maxHeight
    ) {
      fail(ERROR_CODES.VIEWPORT_BOUNDS);
    }
  }

  _snapshot() {
    return { ...this._state };
  }

  get limits() {
    return { ...this._limits };
  }

  getViewport() {
    return this._snapshot();
  }

  snapshot() {
    return this.getViewport();
  }

  claim(ownerOrRequest, maybeVersion) {
    const { ownerId, version } = readOwnerVersion(ownerOrRequest, maybeVersion);
    requireIdentifier(ownerId);
    const expectedVersion = version ?? this._state.version;
    if (!Number.isSafeInteger(expectedVersion) || expectedVersion < 0) {
      fail(ERROR_CODES.VIEWPORT_VERSION_REQUIRED);
    }
    if (expectedVersion !== this._state.version) {
      fail(ERROR_CODES.STALE_VIEWPORT_VERSION);
    }
    if (this._state.owner_id !== null && this._state.owner_id !== ownerId) {
      fail(ERROR_CODES.VIEWPORT_OWNER_MISMATCH);
    }
    if (this._state.owner_id === ownerId) {
      return resultWithFlags(this._snapshot(), false, true);
    }
    this._state.owner_id = ownerId;
    this._state.version += 1;
    this._lastResize = null;
    return resultWithFlags(this._snapshot(), true, false);
  }

  transfer(fromOrRequest, maybeToOwner, maybeVersion) {
    let fromOwnerId;
    let toOwnerId;
    let version;
    if (isPlainObject(fromOrRequest)) {
      fromOwnerId = fromOrRequest.from_owner_id ?? fromOrRequest.fromOwnerId;
      toOwnerId = fromOrRequest.to_owner_id ?? fromOrRequest.toOwnerId;
      version = fromOrRequest.version ?? maybeToOwner;
    } else {
      fromOwnerId = fromOrRequest;
      toOwnerId = maybeToOwner;
      version = maybeVersion;
    }
    requireIdentifier(fromOwnerId);
    requireIdentifier(toOwnerId);
    const expectedVersion = version ?? this._state.version;
    if (!Number.isSafeInteger(expectedVersion) || expectedVersion < 0) {
      fail(ERROR_CODES.VIEWPORT_VERSION_REQUIRED);
    }
    if (expectedVersion !== this._state.version) {
      fail(ERROR_CODES.STALE_VIEWPORT_VERSION);
    }
    if (this._state.owner_id === null || this._state.owner_id !== fromOwnerId) {
      fail(ERROR_CODES.VIEWPORT_OWNER_MISMATCH);
    }
    if (toOwnerId === fromOwnerId) {
      return resultWithFlags(this._snapshot(), false, true);
    }
    this._state.owner_id = toOwnerId;
    this._state.version += 1;
    this._lastResize = null;
    return resultWithFlags(this._snapshot(), true, false);
  }

  resize(ownerOrRequest, maybeVersion, maybeWidth, maybeHeight) {
    let ownerId;
    let version;
    let width;
    let height;
    if (isPlainObject(ownerOrRequest)) {
      ownerId = ownerOrRequest.owner_id ?? ownerOrRequest.ownerId;
      version = ownerOrRequest.version ?? maybeVersion;
      width = ownerOrRequest.width;
      height = ownerOrRequest.height;
    } else {
      ownerId = ownerOrRequest;
      version = maybeVersion;
      width = maybeWidth;
      height = maybeHeight;
    }
    requireIdentifier(ownerId);
    if (this._state.owner_id === null) {
      fail(ERROR_CODES.VIEWPORT_OWNER_REQUIRED);
    }
    if (this._state.owner_id !== ownerId) {
      fail(ERROR_CODES.VIEWPORT_OWNER_MISMATCH);
    }
    if (!Number.isSafeInteger(version) || version < 0) {
      fail(ERROR_CODES.VIEWPORT_VERSION_REQUIRED);
    }
    width = requireDimension(width);
    height = requireDimension(height);
    this._assertBounds(width, height);

    if (version !== this._state.version) {
      const isExactRetry =
        this._lastResize !== null &&
        this._lastResize.ownerId === ownerId &&
        this._lastResize.requestVersion === version &&
        this._lastResize.width === width &&
        this._lastResize.height === height &&
        this._lastResize.resultVersion === this._state.version;
      if (isExactRetry) {
        return resultWithFlags(this._snapshot(), false, true);
      }
      fail(ERROR_CODES.STALE_VIEWPORT_VERSION);
    }

    if (this._state.width === width && this._state.height === height) {
      return resultWithFlags(this._snapshot(), false, true);
    }

    const requestVersion = this._state.version;
    this._state.width = width;
    this._state.height = height;
    this._state.version += 1;
    this._lastResize = {
      ownerId,
      requestVersion,
      width,
      height,
      resultVersion: this._state.version,
    };
    return resultWithFlags(this._snapshot(), true, false);
  }

  toJSON() {
    return this.getViewport();
  }
}

export class BrowserTabRegistry {
  constructor(options = {}) {
    if (!isPlainObject(options)) {
      fail(ERROR_CODES.INVALID_ARGUMENT);
    }
    this._browserGeneration = null;
    this._nextOrdinal = 1;
    this._recordsByTabId = new Map();
    this._activeByTarget = new Map();
    this._targetEpochs = new Map();
    this.viewport = new CanonicalViewport(options.viewport ?? {});
    if (options.generation !== undefined) {
      this._browserGeneration = requireGeneration(options.generation);
    }
  }

  get generation() {
    return this._browserGeneration;
  }

  _ensureGeneration(generation) {
    requireGeneration(generation);
    if (this._browserGeneration === null) {
      this._browserGeneration = generation;
      return;
    }
    if (generation < this._browserGeneration) {
      fail(ERROR_CODES.STALE_GENERATION);
    }
    if (generation === this._browserGeneration) {
      return;
    }
    for (const record of this._activeByTarget.values()) {
      record.active = false;
      record.invalidatedCode = ERROR_CODES.STALE_GENERATION;
    }
    this._activeByTarget.clear();
    this._browserGeneration = generation;
  }

  _targetEpochKey(generation, targetId) {
    return `${generation}\u0000${targetId}`;
  }

  _allocateTarget(target) {
    const epochKey = this._targetEpochKey(this._browserGeneration, target.targetId);
    const targetGeneration = (this._targetEpochs.get(epochKey) ?? 0) + 1;
    this._targetEpochs.set(epochKey, targetGeneration);
    const ordinal = this._nextOrdinal++;
    const record = {
      ordinal,
      tabId: `tab-${ordinal}`,
      targetId: target.targetId,
      targetType: target.targetType,
      websocketUrl: target.websocketUrl,
      generation: this._browserGeneration,
      targetGeneration,
      active: true,
      invalidatedCode: null,
    };
    this._recordsByTabId.set(record.tabId, record);
    this._activeByTarget.set(record.targetId, record);
    return record;
  }

  _detach(record, code = ERROR_CODES.TAB_INVALIDATED) {
    record.active = false;
    record.invalidatedCode = code;
    if (this._activeByTarget.get(record.targetId) === record) {
      this._activeByTarget.delete(record.targetId);
    }
  }

  reconcile(generation, targets, options = {}) {
    this._ensureGeneration(generation);
    if (!Array.isArray(targets)) {
      fail(ERROR_CODES.INVALID_TARGET);
    }
    if (!isPlainObject(options)) {
      fail(ERROR_CODES.INVALID_ARGUMENT);
    }
    const normalized = targets.map(normalizeTarget).sort(compareTargets);
    const uniqueTargets = [];
    let duplicatesSuppressed = 0;
    for (const target of normalized) {
      if (uniqueTargets.length > 0 && uniqueTargets[uniqueTargets.length - 1].targetId === target.targetId) {
        duplicatesSuppressed += 1;
      } else {
        uniqueTargets.push(target);
      }
    }

    const reconnect = options.reconnect === true || options.authoritative === false || options.preserveMissing === true;
    const seenTargetIds = new Set();
    const added = [];
    const updated = [];
    for (const target of uniqueTargets) {
      seenTargetIds.add(target.targetId);
      const current = this._activeByTarget.get(target.targetId);
      if (current === undefined) {
        added.push(cloneTab(this._allocateTarget(target)));
        continue;
      }
      current.targetType = target.targetType;
      current.websocketUrl = target.websocketUrl;
      updated.push(cloneTab(current));
    }

    const detached = [];
    if (!reconnect) {
      for (const record of [...this._activeByTarget.values()]) {
        if (!seenTargetIds.has(record.targetId)) {
          this._detach(record);
          detached.push(cloneTab(record));
        }
      }
    }
    return {
      generation: this._browserGeneration,
      added,
      updated,
      detached,
      duplicates_suppressed: duplicatesSuppressed,
      tabs: this.listTabs(),
    };
  }

  reconcileTargets(generation, targets, options = {}) {
    return this.reconcile(generation, targets, options);
  }

  reconcileReconnect(generation, targets) {
    return this.reconcile(generation, targets, { reconnect: true });
  }

  attachTarget(generation, target) {
    return this.reconcile(generation, [target], { reconnect: true });
  }

  detachTarget(generation, targetId) {
    this._ensureGeneration(generation);
    requireIdentifier(targetId, ERROR_CODES.INVALID_TARGET);
    const record = this._activeByTarget.get(targetId);
    if (record === undefined) {
      fail(ERROR_CODES.TARGET_NOT_FOUND);
    }
    this._detach(record);
    return cloneTab(record);
  }

  invalidateTarget(generation, targetId) {
    return this.detachTarget(generation, targetId);
  }

  _findRecord(tabId, expectedGeneration, expectedTargetGeneration) {
    requireIdentifier(tabId, ERROR_CODES.TARGET_NOT_FOUND);
    const record = this._recordsByTabId.get(tabId);
    if (record === undefined) {
      fail(ERROR_CODES.TARGET_NOT_FOUND);
    }
    if (expectedGeneration !== undefined && expectedGeneration !== record.generation) {
      if (expectedGeneration < record.generation) {
        fail(ERROR_CODES.STALE_GENERATION);
      }
      fail(ERROR_CODES.STALE_GENERATION);
    }
    if (!record.active) {
      fail(record.invalidatedCode === ERROR_CODES.STALE_GENERATION ? ERROR_CODES.STALE_GENERATION : ERROR_CODES.TAB_INVALIDATED);
    }
    if (record.generation !== this._browserGeneration) {
      fail(ERROR_CODES.STALE_GENERATION);
    }
    if (expectedTargetGeneration !== undefined && expectedTargetGeneration !== record.targetGeneration) {
      fail(ERROR_CODES.STALE_TARGET_GENERATION);
    }
    return record;
  }

  getTab(tabId, options = {}) {
    if (!isPlainObject(options)) {
      fail(ERROR_CODES.INVALID_ARGUMENT);
    }
    const expectedGeneration = options.generation ?? options.expectedGeneration;
    const expectedTargetGeneration = options.target_generation ?? options.targetGeneration;
    return cloneTab(this._findRecord(tabId, expectedGeneration, expectedTargetGeneration));
  }

  resolveTarget(tabId, options = {}) {
    const record = this._findRecord(
      tabId,
      options.generation ?? options.expectedGeneration,
      options.target_generation ?? options.targetGeneration,
    );
    return {
      ...cloneTab(record),
      websocket_url: record.websocketUrl,
    };
  }

  getTarget(tabId, options = {}) {
    return this.resolveTarget(tabId, options);
  }

  listTabs() {
    return [...this._activeByTarget.values()]
      .sort((left, right) => left.ordinal - right.ordinal)
      .map(cloneTab);
  }

  getViewport() {
    return this.viewport.getViewport();
  }

  claimViewport(ownerOrRequest, maybeVersion) {
    return this.viewport.claim(ownerOrRequest, maybeVersion);
  }

  transferViewport(fromOrRequest, maybeToOwner, maybeVersion) {
    return this.viewport.transfer(fromOrRequest, maybeToOwner, maybeVersion);
  }

  resizeViewport(ownerOrRequest, maybeVersion, maybeWidth, maybeHeight) {
    return this.viewport.resize(ownerOrRequest, maybeVersion, maybeWidth, maybeHeight);
  }

  setViewport(ownerOrRequest, maybeVersion, maybeWidth, maybeHeight) {
    return this.resizeViewport(ownerOrRequest, maybeVersion, maybeWidth, maybeHeight);
  }

  snapshot() {
    return {
      generation: this._browserGeneration,
      tabs: this.listTabs(),
      viewport: this.getViewport(),
    };
  }

  toJSON() {
    return this.snapshot();
  }

  serialize() {
    return JSON.stringify(this.snapshot());
  }
}

export const createBrowserTabRegistry = (options = {}) => new BrowserTabRegistry(options);
