const DEFAULT_MAX_NODES = 5_000;
const DEFAULT_MAX_STRING_LENGTH = 2_048;
const DEFAULT_MAX_ATTRIBUTES = 32;
const DEFAULT_MAX_FRAMES = 64;

export class BrowserSnapshotError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserSnapshotError";
    this.code = code;
  }
}

export async function captureBrowserSnapshot({
  connection,
  sessionId,
  targetId,
  documentId,
  browserGeneration,
  snapshotRevision,
  limits = {},
  protectedValues = [],
}) {
  // MP-08/MP-10/MP-11: scrub the raw CDP strings before compaction can
  // truncate an echoed value and make exact-value redaction impossible.
  const rawConnection = connection;
  connection = {
    send: async (...args) => redactObservation(await rawConnection.send(...args), protectedValues),
  };
  const options = snapshotLimits(limits);
  const frames = snapshotFrames(
    await assertCurrentDocument(connection, sessionId, targetId, documentId),
    options.maxFrames,
  );
  const [accessibility, dom] = await Promise.all([
    captureFrameAccessibility(connection, sessionId, frames, options),
    connection.send(
      "DOMSnapshot.captureSnapshot",
      {
        computedStyles: [],
        includeDOMRects: true,
        includePaintOrder: false,
      },
      sessionId,
    ),
  ]);
  const currentFrames = snapshotFrames(
    await assertCurrentDocument(connection, sessionId, targetId, documentId),
    options.maxFrames,
  );
  if (JSON.stringify(frames) !== JSON.stringify(currentFrames)) {
    throw new BrowserSnapshotError(
      "stale_document_reference",
      "browser frame tree changed during snapshot capture",
    );
  }
  const compactedDom = compactDomSnapshot(dom, options);
  return {
    browser_generation: browserGeneration,
    target_id: targetId,
    document_id: documentId,
    snapshot_revision: snapshotRevision,
    accessibility_nodes: accessibility.nodes,
    accessibility_truncated: accessibility.truncated,
    ...compactedDom,
  };
}

// MD-N2 / MP-08/MP-10/MP-11: one variant policy for raw span protection
// and ordinary observation redaction, including kernel-replayed retired values.
export function observationProtectedVariants(protectedValues) {
  const variants = new Set();
  for (const secret of protectedValues) {
    if (typeof secret !== "string" || !secret) continue;
    for (const variant of [secret, secret.toLowerCase(), secret.toUpperCase(),
      encodeURIComponent(secret), encodeURIComponent(secret).toLowerCase(),
      JSON.stringify(secret).slice(1, -1),
      secret.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;").replaceAll("'", "&#39;"),
      Buffer.from(secret).toString("base64"), Buffer.from(secret).toString("base64url"),
      Buffer.from(secret).toString("hex"), Buffer.from(secret).toString("hex").toUpperCase()]) {
      variants.add(variant);
    }
  }
  return [...variants].sort((a, b) => b.length - a.length);
}

export function redactObservation(value, protectedValues) {
  const ordered = observationProtectedVariants(protectedValues);
  const scrub = (input) => {
    if (typeof input === "string") {
      for (const variant of ordered) input = input.replaceAll(variant, "[redacted]");
      return input;
    }
    if (Array.isArray(input)) return input.map(scrub);
    if (input && typeof input === "object") return Object.fromEntries(
      Object.entries(input).map(([key, child]) => [scrub(key), scrub(child)]),
    );
    return input;
  };
  return scrub(value);
}

// MP-08/MP-11: layout entries of one DOMSnapshot document whose rendered text
// echoes a protected value. Rendered text, incl. CSS-generated content
// (::before, ::after, ::marker, ::first-letter, counters) that has no DOM
// value, is flattened over the whole document in visual order (snapshots list
// pseudo-elements before children), so a value split across nested or sibling
// elements still matches. A match protects every contributing entry and the
// box of their nearest common laid-out ancestor; unreadable text or a match
// without such a container fails closed.
const PSEUDO_ORDER = { "::marker": 0, "::first-letter": 1, "::before": 2, "::after": 5 };
export function renderedTextEchoes(strings, document, protectedValues) {
  const echoes = new Set();
  if (!protectedValues.length) return echoes;
  const nodes = document.nodes ?? {}, layout = document.layout ?? {};
  const count = nodes.nodeName?.length ?? 0, parent = [], depth = [], children = [], roots = [], entries = new Map();
  for (let i = 0; i < count; i++) {
    const up = nodes.parentIndex?.[i] ?? -1;
    if (!Number.isInteger(up) || up >= i) throw new Error("MP-11: unordered snapshot");
    parent.push(up < 0 ? -1 : up); depth.push(up < 0 ? 0 : depth[up] + 1); children.push([]);
    (up < 0 ? roots : children[up]).push(i);
  }
  for (let k = 0; k < (layout.nodeIndex?.length ?? 0); k++) {
    const node = layout.nodeIndex[k];
    if (!Number.isInteger(node) || node < 0 || node >= count) throw new Error("MP-11: unknown layout node");
    (entries.get(node) ?? entries.set(node, []).get(node)).push(k);
  }
  const order = (i) => { const name = strings[nodes.nodeName[i]] ?? ""; return PSEUDO_ORDER[name] ?? (name.startsWith("::") ? 3 : 4); };
  let flat = "";
  const spans = []; // [start, end, layout entry, node], in flat order.
  for (const stack = roots.reverse(); stack.length;) {
    const i = stack.pop();
    for (const k of entries.get(i) ?? []) {
      const piece = layout.text?.[k];
      if (piece === -1) continue;
      if (typeof strings[piece] !== "string") throw new Error("MP-11: unreadable layout text");
      if (strings[piece]) spans.push([flat.length, flat.length + strings[piece].length, k, i]);
      flat += strings[piece];
    }
    const next = children[i].sort((a, b) => order(a) - order(b));
    for (let c = next.length - 1; c >= 0; c--) stack.push(next[c]);
  }
  const common = (a, b) => { while (a !== b && a >= 0 && b >= 0) depth[a] >= depth[b] ? (a = parent[a]) : (b = parent[b]); return a === b ? a : -1; };
  for (const variant of observationProtectedVariants(protectedValues)) {
    for (let at = flat.indexOf(variant); at >= 0; at = flat.indexOf(variant, at + 1)) {
      let low = 0, high = spans.length;
      while (low < high) { const mid = (low + high) >> 1; spans[mid][1] <= at ? (low = mid + 1) : (high = mid); }
      let container;
      for (let s = low; s < spans.length && spans[s][0] < at + variant.length; s++) {
        echoes.add(spans[s][2]);
        container = container === undefined ? spans[s][3] : common(container, spans[s][3]);
      }
      while (container >= 0 && !entries.has(container)) container = parent[container];
      if (!(container >= 0)) throw new Error("MP-11: rendered text without a laid-out container");
      entries.get(container).forEach((k) => echoes.add(k));
    }
  }
  return echoes;
}

function snapshotLimits(rawLimits) {
  return {
    maxNodes: positiveBound(rawLimits.maxNodes, DEFAULT_MAX_NODES),
    maxStringLength: positiveBound(
      rawLimits.maxStringLength,
      DEFAULT_MAX_STRING_LENGTH,
    ),
    maxAttributes: positiveBound(
      rawLimits.maxAttributes,
      DEFAULT_MAX_ATTRIBUTES,
    ),
    maxFrames: positiveBound(rawLimits.maxFrames, DEFAULT_MAX_FRAMES),
  };
}

function positiveBound(value, fallback) {
  return Number.isSafeInteger(value) && value > 0
    ? Math.min(value, fallback)
    : fallback;
}

async function assertCurrentDocument(connection, sessionId, targetId, documentId) {
  const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
  const currentDocumentId = frameTree?.frameTree?.frame?.loaderId;
  if (currentDocumentId !== documentId) {
    throw new BrowserSnapshotError(
      "stale_document_reference",
      `browser target ${JSON.stringify(targetId)} moved from document ${JSON.stringify(documentId)} to ${JSON.stringify(currentDocumentId ?? null)}`,
    );
  }
  return frameTree?.frameTree;
}

function snapshotFrames(frameTree, maxFrames) {
  const pending = [frameTree];
  const frames = [];
  while (pending.length) {
    const tree = pending.shift();
    if (frames.length + pending.length >= maxFrames) {
      throw new BrowserSnapshotError("snapshot_frame_limit", "browser frame tree exceeded its frame bound");
    }
    frames.push({ id: tree?.frame?.id, loaderId: tree?.frame?.loaderId });
    pending.push(...(tree?.childFrames ?? []));
  }
  return frames;
}

// Reports whether it cut the tree at the node bound, since dropped and
// duplicate nodes can leave fewer nodes than the bound after a cut.
async function captureFrameAccessibility(connection, sessionId, frames, options) {
  const nodes = [];
  const seen = new Set();
  let truncated = false;
  for (const frame of frames) {
    if (nodes.length >= options.maxNodes) {
      truncated = true;
      break;
    }
    const tree = await connection.send(
      "Accessibility.getFullAXTree",
      frame.id ? { frameId: frame.id } : {},
      sessionId,
    );
    const remaining = options.maxNodes - nodes.length;
    if (Array.isArray(tree?.nodes) && tree.nodes.length > remaining) truncated = true;
    // AX node IDs belong to each frame's tree. Resolve their relationships
    // before joining frames through the shared backend DOM references.
    for (const node of compactAccessibilityNodes(tree?.nodes, { ...options, maxNodes: remaining })) {
      if (!seen.has(node.node_ref)) {
        seen.add(node.node_ref);
        nodes.push(node);
      }
    }
  }
  return { nodes, truncated };
}

function compactAccessibilityNodes(rawNodes, options) {
  const nodes = Array.isArray(rawNodes) ? rawNodes.slice(0, options.maxNodes) : [];
  const referenceByNodeId = new Map();
  for (const node of nodes) {
    const reference = backendNodeReference(node?.backendDOMNodeId);
    if (reference && typeof node?.nodeId === "string") {
      referenceByNodeId.set(node.nodeId, reference);
    }
  }
  return nodes.flatMap((node) => {
    const nodeRef = backendNodeReference(node?.backendDOMNodeId);
    if (!nodeRef) {
      return [];
    }
    const properties = new Map(
      (Array.isArray(node?.properties) ? node.properties : [])
        .filter((property) => typeof property?.name === "string")
        .map((property) => [property.name, property?.value?.value]),
    );
    const role = compactString(node?.role?.value, options.maxStringLength);
    const protectedValue =
      properties.get("protected") === true ||
      role.toLowerCase().includes("password");
    const rawValue = compactString(node?.value?.value, options.maxStringLength);
    return [{
      node_ref: nodeRef,
      parent_ref: referenceByNodeId.get(node?.parentId) ?? null,
      child_refs: (Array.isArray(node?.childIds) ? node.childIds : [])
        .map((childId) => referenceByNodeId.get(childId))
        .filter(Boolean),
      role,
      name: compactString(node?.name?.value, options.maxStringLength),
      description: compactString(
        node?.description?.value,
        options.maxStringLength,
      ),
      value: protectedValue && rawValue ? "[redacted]" : rawValue,
      ignored: node?.ignored === true,
      disabled: properties.get("disabled") === true,
      focused: properties.get("focused") === true,
      states: announcedStates(properties),
    }];
  });
}

// The control states a screen reader announces, named as the kernel expects.
function announcedStates(properties) {
  const tristate = (name, on, off) => {
    const value = properties.get(name);
    if (value === undefined) return [];
    if (value === "mixed") return ["mixed"];
    return [value === true || value === "true" ? on : off];
  };
  const invalid = properties.get("invalid");
  return [
    ...tristate("checked", "checked", "not checked"),
    ...tristate("pressed", "pressed", "not pressed"),
    ...(properties.has("expanded") ? [properties.get("expanded") === true ? "expanded" : "collapsed"] : []),
    ...(properties.get("selected") === true ? ["selected"] : []),
    ...(properties.get("required") === true ? ["required"] : []),
    ...(invalid !== undefined && invalid !== false && invalid !== "false" ? ["invalid"] : []),
  ];
}

function compactDomSnapshot(rawSnapshot, options) {
  const strings = Array.isArray(rawSnapshot?.strings) ? rawSnapshot.strings : [];
  const documents = Array.isArray(rawSnapshot?.documents)
    ? rawSnapshot.documents
    : [];
  const compacted = [];
  const ownerByDocumentIndex = new Map();
  const shadowRoots = [];
  const documentCount = Math.min(documents.length, options.maxNodes);
  for (let documentIndex = 0; documentIndex < documentCount; documentIndex += 1) {
    const document = documents[documentIndex];
    if (compacted.length >= options.maxNodes) {
      break;
    }
    const nodes = document?.nodes ?? {};
    const backendNodeIds = Array.isArray(nodes.backendNodeId)
      ? nodes.backendNodeId
      : [];
    const layoutBounds = boundsByNodeIndex(document?.layout);
    const contentDocuments = rareDataByIndex(nodes.contentDocumentIndex);
    const shadowRootTypes = rareDataByIndex(nodes.shadowRootType);
    for (
      let nodeIndex = 0;
      nodeIndex < backendNodeIds.length && compacted.length < options.maxNodes;
      nodeIndex += 1
    ) {
      const nodeRef = backendNodeReference(backendNodeIds[nodeIndex]);
      if (!nodeRef) {
        continue;
      }
      const parentIndex = arrayValue(nodes.parentIndex, nodeIndex);
      const parentRef = Number.isSafeInteger(parentIndex)
        ? backendNodeReference(backendNodeIds[parentIndex])
        : null;
      const nodeType = arrayValue(nodes.nodeType, nodeIndex);
      const nodeName = snapshotString(
        strings,
        arrayValue(nodes.nodeName, nodeIndex),
        options.maxStringLength,
      );
      const text =
        nodeType === 3 || nodeType === 4
          ? snapshotString(
              strings,
              arrayValue(nodes.nodeValue, nodeIndex),
              options.maxStringLength,
            )
          : "";
      const contentDocumentIndex = contentDocuments.get(nodeIndex);
      if (
        Number.isSafeInteger(contentDocumentIndex) &&
        contentDocumentIndex >= 0 &&
        contentDocumentIndex < documentCount
      ) {
        ownerByDocumentIndex.set(contentDocumentIndex, nodeRef);
      }
      const shadowRootTypeIndex = shadowRootTypes.get(nodeIndex);
      const shadowRootType = snapshotString(
        strings,
        shadowRootTypeIndex,
        options.maxStringLength,
      );
      if (shadowRootType) {
        shadowRoots.push({
          node_ref: nodeRef,
          shadow_root_type: shadowRootType,
        });
      }
      compacted.push({
        node_ref: nodeRef,
        parent_ref: parentRef,
        document_index: documentIndex,
        node_type: Number.isSafeInteger(nodeType) ? nodeType : 0,
        node_name: nodeName,
        text,
        attributes: compactAttributes(
          strings,
          arrayValue(nodes.attributes, nodeIndex),
          nodeName,
          options,
        ),
        bounds: layoutBounds.get(nodeIndex) ?? null,
      });
    }
  }
  return {
    dom_documents: documents.slice(0, documentCount).map((document, documentIndex) => ({
      document_index: documentIndex,
      url: snapshotString(
        strings,
        document?.documentURL,
        options.maxStringLength,
      ),
      owner_node_ref: ownerByDocumentIndex.get(documentIndex) ?? null,
    })),
    shadow_roots: shadowRoots,
    dom_nodes: compacted,
  };
}

function rareDataByIndex(rawData) {
  const indexes = Array.isArray(rawData?.index) ? rawData.index : [];
  const values = Array.isArray(rawData?.value) ? rawData.value : [];
  const result = new Map();
  for (let offset = 0; offset < indexes.length && offset < values.length; offset += 1) {
    const nodeIndex = indexes[offset];
    if (Number.isSafeInteger(nodeIndex)) {
      result.set(nodeIndex, values[offset]);
    }
  }
  return result;
}

function boundsByNodeIndex(layout) {
  const result = new Map();
  const nodeIndices = Array.isArray(layout?.nodeIndex) ? layout.nodeIndex : [];
  const bounds = Array.isArray(layout?.bounds) ? layout.bounds : [];
  for (let index = 0; index < nodeIndices.length; index += 1) {
    const nodeIndex = nodeIndices[index];
    const rawBounds = bounds[index];
    if (
      !Number.isSafeInteger(nodeIndex) ||
      !Array.isArray(rawBounds) ||
      rawBounds.length < 4 ||
      !rawBounds.slice(0, 4).every(Number.isFinite)
    ) {
      continue;
    }
    result.set(nodeIndex, {
      x: rawBounds[0],
      y: rawBounds[1],
      width: rawBounds[2],
      height: rawBounds[3],
    });
  }
  return result;
}

function compactAttributes(strings, rawAttributes, nodeName, options) {
  const attributes = {};
  const indexes = Array.isArray(rawAttributes) ? rawAttributes : [];
  for (
    let index = 0;
    index + 1 < indexes.length &&
    Object.keys(attributes).length < options.maxAttributes;
    index += 2
  ) {
    const name = snapshotString(
      strings,
      indexes[index],
      options.maxStringLength,
    );
    if (!name) {
      continue;
    }
    const value = snapshotString(
      strings,
      indexes[index + 1],
      options.maxStringLength,
    );
    attributes[name] = shouldRedactAttribute(nodeName, name)
      ? "[redacted]"
      : value;
  }
  return attributes;
}

function shouldRedactAttribute(nodeName, attributeName) {
  const normalizedNodeName = nodeName.toLowerCase();
  const normalizedAttributeName = attributeName.toLowerCase();
  if (
    normalizedAttributeName.includes("password") ||
    normalizedAttributeName.includes("secret") ||
    normalizedAttributeName.includes("token") ||
    normalizedAttributeName === "authorization" ||
    normalizedAttributeName === "cookie"
  ) {
    return true;
  }
  return (
    normalizedAttributeName === "value" &&
    ["input", "textarea", "option"].includes(normalizedNodeName)
  );
}

function backendNodeReference(backendNodeId) {
  return Number.isSafeInteger(backendNodeId) && backendNodeId > 0
    ? `backend:${backendNodeId}`
    : null;
}

function compactString(value, maxLength) {
  if (typeof value !== "string") {
    return "";
  }
  let result = "";
  let byteLength = 0;
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    const characterBytes = codePoint <= 0x7f
      ? 1
      : codePoint <= 0x7ff
        ? 2
        : codePoint <= 0xffff
          ? 3
          : 4;
    if (byteLength + characterBytes > maxLength) {
      break;
    }
    result += character;
    byteLength += characterBytes;
  }
  return result;
}

function snapshotString(strings, index, maxLength) {
  return Number.isSafeInteger(index)
    ? compactString(strings[index], maxLength)
    : "";
}

function arrayValue(value, index) {
  return Array.isArray(value) ? value[index] : undefined;
}
