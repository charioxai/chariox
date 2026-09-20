const DEFAULT_MAX_NODES = 1_000;
const DEFAULT_MAX_STRING_BYTES = 512;
const DEFAULT_MAX_ATTRIBUTES = 24;
const DEFAULT_MAX_RESULT_BYTES = 1024 * 1024;

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

function currentDocumentId(frameTree) {
  const documentId = frameTree?.frameTree?.frame?.loaderId;
  if (typeof documentId !== "string" || documentId.length === 0 || documentId.length > 512) {
    fail(OBSERVATION_ERROR_CODES.CDP_PROTOCOL_INVALID);
  }
  return documentId;
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

function compactAttributes(strings, indexes, nodeName, limits) {
  const result = {};
  const values = Array.isArray(indexes) ? indexes : [];
  for (let index = 0; index + 1 < values.length; index += 2) {
    if (Object.keys(result).length >= limits.maxAttributes) break;
    const name = snapshotString(strings, values[index], limits.maxStringBytes);
    if (!name) continue;
    result[name] = sensitiveAttribute(nodeName, name)
      ? "[redacted]"
      : snapshotString(strings, values[index + 1], limits.maxStringBytes);
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

function compactAccessibility(raw, draft, limits) {
  const source = Array.isArray(raw?.nodes) ? raw.nodes : [];
  const nodes = source.slice(0, limits.maxNodes);
  const referenceByAxNodeId = new Map();
  for (const node of nodes) {
    const reference = elementReference(draft, node?.backendDOMNodeId);
    if (reference && typeof node?.nodeId === "string") {
      referenceByAxNodeId.set(node.nodeId, reference);
    }
  }
  return {
    truncated: source.length > nodes.length,
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
  let sourceCount = 0;
  for (let documentIndex = 0; documentIndex < documents.length; documentIndex += 1) {
    const document = documents[documentIndex];
    const data = document?.nodes ?? {};
    const backendIds = Array.isArray(data.backendNodeId) ? data.backendNodeId : [];
    sourceCount += backendIds.length;
    const layoutBounds = boundsByNodeIndex(document?.layout);
    for (let nodeIndex = 0; nodeIndex < backendIds.length; nodeIndex += 1) {
      if (nodes.length >= limits.maxNodes) break;
      const reference = elementReference(draft, backendIds[nodeIndex]);
      if (!reference) continue;
      const parentIndex = arrayValue(data.parentIndex, nodeIndex);
      const parentReference = Number.isSafeInteger(parentIndex)
        ? elementReference(draft, backendIds[parentIndex])
        : null;
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
  return { truncated: sourceCount > nodes.length, nodes };
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
    const before = currentDocumentId(await connection.send("Page.getFrameTree", {}));
    const previous = this.statesByTabId.get(tab.tabId);
    const base = tabStateMatches(previous, tab) ? previous : null;
    const draft = makeDraft(base, before, this.nextReferenceEpoch);
    if (base === null || base.documentId !== before) {
      this.nextReferenceEpoch += 1;
    }
    draft.generation = tab.generation;
    draft.targetGeneration = tab.targetGeneration;

    const [accessibilityRaw, domRaw] = await Promise.all([
      connection.send("Accessibility.getFullAXTree", {}),
      connection.send("DOMSnapshot.captureSnapshot", {
        computedStyles: [],
        includeDOMRects: true,
        includePaintOrder: false,
      }),
    ]);
    const after = currentDocumentId(await connection.send("Page.getFrameTree", {}));
    if (after !== before) {
      fail(OBSERVATION_ERROR_CODES.STALE_DOCUMENT);
    }

    draft.revision += 1;
    const accessibility = compactAccessibility(accessibilityRaw, draft, limits);
    const dom = compactDom(domRaw, draft, limits);
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

  resolve(rawTab, elementRef) {
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
    return {
      tab_id: tab.tabId,
      browser_generation: tab.generation,
      target_generation: tab.targetGeneration,
      document_id: state.documentId,
      snapshot_revision: state.revision,
      backend_node_id: backendNodeId,
    };
  }
}
