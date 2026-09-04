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
}) {
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
    accessibility_nodes: accessibility,
    ...compactedDom,
  };
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

async function captureFrameAccessibility(connection, sessionId, frames, options) {
  const nodes = [];
  const seen = new Set();
  for (const frame of frames) {
    if (nodes.length >= options.maxNodes) break;
    const tree = await connection.send(
      "Accessibility.getFullAXTree",
      frame.id ? { frameId: frame.id } : {},
      sessionId,
    );
    // AX node IDs belong to each frame's tree. Resolve their relationships
    // before joining frames through the shared backend DOM references.
    for (const node of compactAccessibilityNodes(tree?.nodes, { ...options, maxNodes: options.maxNodes - nodes.length })) {
      if (!seen.has(node.node_ref)) {
        seen.add(node.node_ref);
        nodes.push(node);
      }
    }
  }
  return nodes;
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
    }];
  });
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
