const DEFAULT_MAX_NODES = 1_000;
const DEFAULT_MAX_STRING_BYTES = 512;
const DEFAULT_MAX_ATTRIBUTES = 24;
const DEFAULT_MAX_RESULT_BYTES = 1024 * 1024;

export const OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES = 4 * 1024 * 1024;

export const OBSERVATION_ERROR_CODES = Object.freeze({
  INVALID_ARGUMENT: "INVALID_ARGUMENT",
  CDP_PROTOCOL_INVALID: "CDP_PROTOCOL_INVALID",
  STALE_DOCUMENT: "STALE_DOCUMENT",
  ELEMENT_REFERENCE_INVALIDATED: "ELEMENT_REFERENCE_INVALIDATED",
  SNAPSHOT_TOO_LARGE: "SNAPSHOT_TOO_LARGE",
});

const ERROR_MESSAGES = Object.freeze({
  [OBSERVATION_ERROR_CODES.INVALID_ARGUMENT]: "browser observation argument is invalid",
  [OBSERVATION_ERROR_CODES.CDP_PROTOCOL_INVALID]: "browser observation CDP response is invalid",
  [OBSERVATION_ERROR_CODES.STALE_DOCUMENT]: "browser document changed during observation",
  [OBSERVATION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED]: "element reference is no longer valid",
  [OBSERVATION_ERROR_CODES.SNAPSHOT_TOO_LARGE]: "browser observation exceeds the byte limit",
});

export class BrowserObservationError extends Error {
  constructor(code) {
    super(ERROR_MESSAGES[code] ?? ERROR_MESSAGES[OBSERVATION_ERROR_CODES.INVALID_ARGUMENT]);
    this.name = "BrowserObservationError";
    this.code = code in ERROR_MESSAGES ? code : OBSERVATION_ERROR_CODES.INVALID_ARGUMENT;
  }
}

function fail(code) {
  throw new BrowserObservationError(code);
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requirePositiveInteger(value) {
  if (!Number.isSafeInteger(value) || value < 1) {
    fail(OBSERVATION_ERROR_CODES.INVALID_ARGUMENT);
  }
  return value;
}

function requireIdentifier(value) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 128 ||
    !/^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(value)
  ) {
    fail(OBSERVATION_ERROR_CODES.INVALID_ARGUMENT);
  }
  return value;
}

function observationLimits(raw = {}) {
  if (!isPlainObject(raw)) {
    fail(OBSERVATION_ERROR_CODES.INVALID_ARGUMENT);
  }
  const bounded = (value, fallback, maximum) => {
    if (value === undefined) return fallback;
    requirePositiveInteger(value);
    return Math.min(value, maximum);
  };
  return {
    maxNodes: bounded(raw.maxNodes, DEFAULT_MAX_NODES, DEFAULT_MAX_NODES),
    maxStringBytes: bounded(raw.maxStringBytes, DEFAULT_MAX_STRING_BYTES, DEFAULT_MAX_STRING_BYTES),
    maxAttributes: bounded(raw.maxAttributes, DEFAULT_MAX_ATTRIBUTES, DEFAULT_MAX_ATTRIBUTES),
    maxResultBytes: bounded(raw.maxResultBytes, DEFAULT_MAX_RESULT_BYTES, 4 * DEFAULT_MAX_RESULT_BYTES),
  };
}

function normalizeTab(raw) {
  if (!isPlainObject(raw)) {
    fail(OBSERVATION_ERROR_CODES.INVALID_ARGUMENT);
  }
  return {
    tabId: requireIdentifier(raw.tab_id ?? raw.tabId),
    generation: requirePositiveInteger(raw.generation),
    targetGeneration: requirePositiveInteger(raw.target_generation ?? raw.targetGeneration),
  };
}

function frameIdentities(frameTree) {
  const root = frameTree?.frameTree;
  if (!isPlainObject(root) || !isPlainObject(root.frame)) {
    fail(OBSERVATION_ERROR_CODES.CDP_PROTOCOL_INVALID);
  }
  const identities = new Map();
  const visit = (entry, path, isRoot = false) => {
    const frame = entry?.frame;
    if (!isPlainObject(frame)) {
      fail(OBSERVATION_ERROR_CODES.CDP_PROTOCOL_INVALID);
    }
    const loaderId = frame.loaderId ?? "";
    if (typeof loaderId !== "string" || loaderId.length > 512) {
      fail(OBSERVATION_ERROR_CODES.CDP_PROTOCOL_INVALID);
    }
    if (
      frame.id !== undefined &&
      (typeof frame.id !== "string" || frame.id.length === 0 || frame.id.length > 512)
    ) {
      fail(OBSERVATION_ERROR_CODES.CDP_PROTOCOL_INVALID);
    }
    const frameId = typeof frame.id === "string"
      ? `frame:${frame.id}`
      : `frame-path:${path}`;
    const identityKey = isRoot ? "root" : frameId;
    if (identities.has(identityKey)) {
      fail(OBSERVATION_ERROR_CODES.CDP_PROTOCOL_INVALID);
    }
    identities.set(identityKey, loaderId);
    if (entry.childFrames !== undefined && !Array.isArray(entry.childFrames)) {
      fail(OBSERVATION_ERROR_CODES.CDP_PROTOCOL_INVALID);
    }
    const children = Array.isArray(entry.childFrames) ? entry.childFrames : [];
    children.forEach((child, index) => visit(child, `${path}.${index}`));
  };
  visit(root, "root", true);
  const rootDocumentId = identities.get("root");
  if (typeof rootDocumentId !== "string" || rootDocumentId.length === 0) {
    fail(OBSERVATION_ERROR_CODES.CDP_PROTOCOL_INVALID);
  }
  return identities;
}

function sameFrameIdentities(left, right) {
  if (!(left instanceof Map) || !(right instanceof Map) || left.size !== right.size) {
    return false;
  }
  for (const [frameId, loaderId] of left) {
    if (right.get(frameId) !== loaderId) return false;
  }
  return true;
}

function boundedString(value, maxBytes) {
  if (typeof value !== "string") return "";
  let result = "";
  let bytes = 0;
  for (const character of value) {
    const width = Buffer.byteLength(character, "utf8");
    if (bytes + width > maxBytes) break;
    result += character;
    bytes += width;
  }
  return result;
}

function arrayValue(value, index) {
  return Array.isArray(value) ? value[index] : undefined;
}

function snapshotString(strings, index, maxBytes) {
  return Number.isSafeInteger(index)
    ? boundedString(strings[index], maxBytes)
    : "";
}

function validBackendNodeId(value) {
  return Number.isSafeInteger(value) && value > 0 ? value : null;
}

function sensitiveAttribute(nodeName, attributeName) {
  const node = nodeName.toLowerCase();
  const attribute = attributeName.toLowerCase();
  if (
    attribute === "authorization" ||
    attribute === "cookie" ||
    attribute.includes("password") ||
    attribute.includes("secret") ||
    attribute.includes("token") ||
    attribute.includes("credential") ||
    attribute.includes("api-key") ||
    attribute.includes("api_key")
  ) {
    return true;
  }
  return attribute === "value" && ["input", "textarea", "option"].includes(node);
}

function normalizedSecretKey(value) {
  return String(value ?? "").toLowerCase().replace(/[^a-z0-9]/g, "");
}

function looksSecretKey(value, { queryParameter = false } = {}) {
  const key = normalizedSecretKey(value);
  return (
    key === "authorization" ||
    key === "cookie" ||
    key === "credential" ||
    key === "password" ||
    key === "passwd" ||
    key === "secret" ||
    key === "apikey" ||
    key === "accesskey" ||
    key === "privatekey" ||
    key === "bearer" ||
    key === "auth" ||
    key === "nonce" ||
    key === "session" ||
    key === "sessionid" ||
    key === "signature" ||
    key === "sig" ||
    key === "hmac" ||
    key === "jwt" ||
    (queryParameter && key === "key") ||
    key.endsWith("token") ||
    key.endsWith("secret") ||
    key.endsWith("credential") ||
    key.endsWith("signature")
  );
}

function containsSecretAssignment(value) {
  const assignment = /(?:^|[^A-Za-z0-9])["']?([A-Za-z][A-Za-z0-9_.-]{1,96})["']?\s*[:=]\s*(?:"[^"]*"|'[^']*'|[^\s&#,;}\]]+)/g;
  for (const match of String(value ?? "").matchAll(assignment)) {
    if (looksSecretKey(match[1])) return true;
  }
  return false;
}

function containsSecretValue(value) {
  if (typeof value !== "string" || value.length === 0) return false;
  if (containsSecretAssignment(value)) return true;
  if (/(?:^|[^A-Za-z0-9_])(?:[A-Za-z][A-Za-z0-9+.-]*:)?\/\/[^/\s:@]+:[^/\s@]*@/i.test(value)) {
    return true;
  }
  try {
    const parsed = new URL(value.startsWith("//") ? `http:${value}` : value, "http://snapshot.invalid");
    if (parsed.username || parsed.password) return true;
    for (const [key] of parsed.searchParams) {
      if (looksSecretKey(key, { queryParameter: true })) return true;
    }
  } catch {
    // Non-URL attribute values are covered by the assignment check above.
  }
  try {
    const decoded = decodeURIComponent(value);
    return decoded !== value && containsSecretAssignment(decoded);
  } catch {
    return false;
  }
}

function compactAttributeValue(
  strings,
  valueIndex,
  nodeName,
  attributeName,
  siblingValues,
  limits,
) {
  if (sensitiveAttribute(nodeName, attributeName)) return "[redacted]";
  if (
    nodeName.toLowerCase() === "meta" &&
    attributeName.toLowerCase() === "content" &&
    looksSecretKey(siblingValues.get("name"))
  ) {
    return "[redacted]";
  }
  const value = snapshotString(strings, valueIndex, limits.maxStringBytes);
  return containsSecretValue(value) ? "[redacted]" : value;
}

function compactAttributes(strings, indexes, nodeName, limits) {
  const result = {};
  const values = Array.isArray(indexes) ? indexes : [];
  const siblingValues = new Map();
  for (let index = 0; index + 1 < values.length; index += 2) {
    const name = snapshotString(strings, values[index], limits.maxStringBytes);
    if (name && !siblingValues.has(name.toLowerCase())) {
      siblingValues.set(
        name.toLowerCase(),
        snapshotString(strings, values[index + 1], limits.maxStringBytes),
      );
    }
  }
  for (let index = 0; index + 1 < values.length; index += 2) {
    if (Object.keys(result).length >= limits.maxAttributes) break;
    const name = snapshotString(strings, values[index], limits.maxStringBytes);
    if (!name) continue;
    result[name] = compactAttributeValue(
      strings,
      values[index + 1],
      nodeName,
      name,
      siblingValues,
      limits,
    );
  }
  return result;
}

function boundsByNodeIndex(layout) {
  const result = new Map();
  const nodeIndexes = Array.isArray(layout?.nodeIndex) ? layout.nodeIndex : [];
  const bounds = Array.isArray(layout?.bounds) ? layout.bounds : [];
  for (let index = 0; index < nodeIndexes.length; index += 1) {
    const nodeIndex = nodeIndexes[index];
    const value = bounds[index];
    if (
      Number.isSafeInteger(nodeIndex) &&
      Array.isArray(value) &&
      value.length >= 4 &&
      value.slice(0, 4).every(Number.isFinite)
    ) {
      result.set(nodeIndex, {
        x: value[0],
        y: value[1],
        width: value[2],
        height: value[3],
      });
    }
  }
  return result;
}

function makeDraft(previous, documentId, referenceEpoch) {
  if (previous?.documentId === documentId) {
    return {
      ...previous,
      refsByBackendId: new Map(previous.refsByBackendId),
      backendIdByRef: new Map(previous.backendIdByRef),
      frameIdentities: new Map(previous.frameIdentities ?? []),
    };
  }
  return {
    documentId,
    revision: 0,
    referenceEpoch,
    nextElementOrdinal: 1,
    refsByBackendId: new Map(),
    backendIdByRef: new Map(),
  };
}

function elementReference(draft, backendNodeId) {
  const validId = validBackendNodeId(backendNodeId);
  if (validId === null) return null;
  const existing = draft.refsByBackendId.get(validId);
  if (existing) return existing;
  const reference = `element-${draft.referenceEpoch}-${draft.nextElementOrdinal++}`;
  draft.refsByBackendId.set(validId, reference);
  draft.backendIdByRef.set(reference, validId);
  return reference;
}

function pruneReferences(draft, liveBackendIds) {
  for (const [backendNodeId, reference] of draft.refsByBackendId) {
    if (!liveBackendIds.has(backendNodeId)) {
      draft.refsByBackendId.delete(backendNodeId);
      draft.backendIdByRef.delete(reference);
    }
  }
  for (const [reference, backendNodeId] of draft.backendIdByRef) {
    if (!liveBackendIds.has(backendNodeId)) {
      draft.backendIdByRef.delete(reference);
      draft.refsByBackendId.delete(backendNodeId);
    }
  }
}

function compactAccessibility(raw, draft, limits) {
  const source = Array.isArray(raw?.nodes) ? raw.nodes : [];
  const nodes = source.slice(0, limits.maxNodes);
  const referenceByAxNodeId = new Map();
  const liveBackendIds = new Set();
  for (const node of nodes) {
    const reference = elementReference(draft, node?.backendDOMNodeId);
    if (reference) {
      liveBackendIds.add(node.backendDOMNodeId);
      if (typeof node?.nodeId === "string") {
        referenceByAxNodeId.set(node.nodeId, reference);
      }
    }
  }
  return {
    truncated: source.length > nodes.length,
    liveBackendIds,
    nodes: nodes.flatMap((node) => {
      const reference = elementReference(draft, node?.backendDOMNodeId);
      if (!reference) return [];
      const properties = new Map(
        (Array.isArray(node?.properties) ? node.properties : [])
          .filter((property) => typeof property?.name === "string")
          .map((property) => [property.name, property?.value?.value]),
      );
      const role = boundedString(node?.role?.value, limits.maxStringBytes);
      const protectedValue = properties.get("protected") === true || role.toLowerCase().includes("password");
      const rawValue = boundedString(node?.value?.value, limits.maxStringBytes);
      return [{
        element_ref: reference,
        parent_ref: referenceByAxNodeId.get(node?.parentId) ?? null,
        child_refs: (Array.isArray(node?.childIds) ? node.childIds : [])
          .map((childId) => referenceByAxNodeId.get(childId))
          .filter(Boolean),
        role,
        name: boundedString(node?.name?.value, limits.maxStringBytes),
        description: boundedString(node?.description?.value, limits.maxStringBytes),
        value: protectedValue && rawValue ? "[redacted]" : rawValue,
        ignored: node?.ignored === true,
        disabled: properties.get("disabled") === true,
        focused: properties.get("focused") === true,
      }];
    }),
  };
}

function compactDom(raw, draft, limits) {
  const strings = Array.isArray(raw?.strings) ? raw.strings : [];
  const documents = Array.isArray(raw?.documents) ? raw.documents : [];
  const nodes = [];
  const sourceCount = documents.reduce((total, document) => {
    const backendIds = Array.isArray(document?.nodes?.backendNodeId)
      ? document.nodes.backendNodeId
      : [];
    return total + backendIds.length;
  }, 0);
  const liveBackendIds = new Set();
  for (let documentIndex = 0; documentIndex < documents.length; documentIndex += 1) {
    const document = documents[documentIndex];
    const data = document?.nodes ?? {};
    const backendIds = Array.isArray(data.backendNodeId) ? data.backendNodeId : [];
    const layoutBounds = boundsByNodeIndex(document?.layout);
    for (let nodeIndex = 0; nodeIndex < backendIds.length; nodeIndex += 1) {
      if (nodes.length >= limits.maxNodes) break;
      const reference = elementReference(draft, backendIds[nodeIndex]);
      if (!reference) continue;
      liveBackendIds.add(backendIds[nodeIndex]);
      const parentIndex = arrayValue(data.parentIndex, nodeIndex);
      const parentReference = Number.isSafeInteger(parentIndex)
        ? elementReference(draft, backendIds[parentIndex])
        : null;
      if (parentReference) {
        liveBackendIds.add(backendIds[parentIndex]);
      }
      const nodeType = arrayValue(data.nodeType, nodeIndex);
      const nodeName = snapshotString(strings, arrayValue(data.nodeName, nodeIndex), limits.maxStringBytes);
      const parentName = Number.isSafeInteger(parentIndex)
        ? snapshotString(strings, arrayValue(data.nodeName, parentIndex), limits.maxStringBytes)
        : "";
      const exposeText = (nodeType === 3 || nodeType === 4) && !["script", "style"].includes(parentName.toLowerCase());
      const protectedText = ["input", "textarea", "option"].includes(parentName.toLowerCase());
      nodes.push({
        element_ref: reference,
        parent_ref: parentReference,
        document_index: documentIndex,
        node_type: Number.isSafeInteger(nodeType) ? nodeType : 0,
        node_name: nodeName,
        text: exposeText
          ? protectedText
            ? "[redacted]"
            : snapshotString(strings, arrayValue(data.nodeValue, nodeIndex), limits.maxStringBytes)
          : "",
        attributes: compactAttributes(
          strings,
          arrayValue(data.attributes, nodeIndex),
          nodeName,
          limits,
        ),
        bounds: layoutBounds.get(nodeIndex) ?? null,
      });
    }
    if (nodes.length >= limits.maxNodes) break;
  }
  return { truncated: sourceCount > nodes.length, liveBackendIds, nodes };
}

function assertResultBounded(result, maximumBytes) {
  const encoded = JSON.stringify(result);
  if (Buffer.byteLength(encoded, "utf8") > maximumBytes) {
    fail(OBSERVATION_ERROR_CODES.SNAPSHOT_TOO_LARGE);
  }
}

function tabStateMatches(state, tab) {
  return state?.generation === tab.generation && state?.targetGeneration === tab.targetGeneration;
}

function resolvedReference(tab, state, backendNodeId) {
  return {
    tab_id: tab.tabId,
    browser_generation: tab.generation,
    target_generation: tab.targetGeneration,
    document_id: state.documentId,
    snapshot_revision: state.revision,
    backend_node_id: backendNodeId,
  };
}

export class BrowserObservationStore {
  constructor() {
    this.statesByTabId = new Map();
    this.nextReferenceEpoch = 1;
  }

  reconcile(rawTabs) {
    if (!Array.isArray(rawTabs)) fail(OBSERVATION_ERROR_CODES.INVALID_ARGUMENT);
    const active = new Map(rawTabs.map((raw) => {
      const tab = normalizeTab(raw);
      return [tab.tabId, tab];
    }));
    for (const [tabId, state] of this.statesByTabId) {
      const tab = active.get(tabId);
      if (!tab || !tabStateMatches(state, tab)) {
        this.statesByTabId.delete(tabId);
      }
    }
  }

  invalidate(tabId) {
    requireIdentifier(tabId);
    this.statesByTabId.delete(tabId);
  }

  clear() {
    this.statesByTabId.clear();
  }

  async capture({ connection, tab: rawTab, limits: rawLimits } = {}) {
    if (typeof connection?.send !== "function") {
      fail(OBSERVATION_ERROR_CODES.INVALID_ARGUMENT);
    }
    const tab = normalizeTab(rawTab);
    const limits = observationLimits(rawLimits);
    const beforeIdentities = frameIdentities(await connection.send("Page.getFrameTree", {}));
    const before = beforeIdentities.get("root");
    const previous = this.statesByTabId.get(tab.tabId);
    if (
      previous &&
      (!tabStateMatches(previous, tab) || !sameFrameIdentities(previous.frameIdentities, beforeIdentities))
    ) {
      this.invalidate(tab.tabId);
    }
    const base = tabStateMatches(previous, tab) && sameFrameIdentities(previous?.frameIdentities, beforeIdentities)
      ? previous
      : null;
    const draft = makeDraft(base, before, this.nextReferenceEpoch);
    draft.frameIdentities = new Map(beforeIdentities);
    if (base === null || base.documentId !== before) {
      this.nextReferenceEpoch += 1;
    }
    draft.generation = tab.generation;
    draft.targetGeneration = tab.targetGeneration;

    const [accessibilityRaw, domRaw] = await Promise.all([
      connection.send("Accessibility.getFullAXTree", {}, undefined, {
        maxResponseBytes: OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES,
      }),
      connection.send("DOMSnapshot.captureSnapshot", {
        computedStyles: [],
        includeDOMRects: true,
        includePaintOrder: false,
      }, undefined, {
        maxResponseBytes: OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES,
      }),
    ]);
    const afterIdentities = frameIdentities(await connection.send("Page.getFrameTree", {}));
    const after = afterIdentities.get("root");
    if (after !== before || !sameFrameIdentities(beforeIdentities, afterIdentities)) {
      this.invalidate(tab.tabId);
      fail(OBSERVATION_ERROR_CODES.STALE_DOCUMENT);
    }

    draft.revision += 1;
    const accessibility = compactAccessibility(accessibilityRaw, draft, limits);
    const dom = compactDom(domRaw, draft, limits);
    pruneReferences(
      draft,
      new Set([...accessibility.liveBackendIds, ...dom.liveBackendIds]),
    );
    const result = {
      browser_generation: tab.generation,
      tab_id: tab.tabId,
      target_generation: tab.targetGeneration,
      document_id: before,
      snapshot_revision: draft.revision,
      accessibility_nodes: accessibility.nodes,
      dom_nodes: dom.nodes,
      truncated: accessibility.truncated || dom.truncated,
    };
    assertResultBounded(result, limits.maxResultBytes);
    this.statesByTabId.set(tab.tabId, draft);
    return result;
  }

  resolve(rawTab, elementRef, options = undefined) {
    const tab = normalizeTab(rawTab);
    requireIdentifier(elementRef);
    const state = this.statesByTabId.get(tab.tabId);
    if (!tabStateMatches(state, tab)) {
      fail(OBSERVATION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED);
    }
    const backendNodeId = state.backendIdByRef.get(elementRef);
    if (!Number.isSafeInteger(backendNodeId)) {
      fail(OBSERVATION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED);
    }
    const connection = typeof options?.send === "function" ? options : options?.connection;
    if (connection === undefined) {
      return resolvedReference(tab, state, backendNodeId);
    }
    if (typeof connection?.send !== "function") {
      fail(OBSERVATION_ERROR_CODES.INVALID_ARGUMENT);
    }
    return this._resolveWithLiveIdentity(tab, state, backendNodeId, connection);
  }

  async _resolveWithLiveIdentity(tab, state, backendNodeId, connection) {
    const liveIdentities = frameIdentities(await connection.send("Page.getFrameTree", {}));
    if (!sameFrameIdentities(state.frameIdentities, liveIdentities)) {
      this.invalidate(tab.tabId);
      fail(OBSERVATION_ERROR_CODES.STALE_DOCUMENT);
    }
    if (this.statesByTabId.get(tab.tabId) !== state || !tabStateMatches(state, tab)) {
      fail(OBSERVATION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED);
    }
    return resolvedReference(tab, state, backendNodeId);
  }
}
