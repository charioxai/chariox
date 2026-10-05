// MP-08 / MP-10 / MP-11: rendered DOM text and read-only panes, no editable values or AX duplicates.
export class BrowserTextError extends Error {
  constructor(message) { super(message); this.code = "browser_text_invalid"; }
}
const EXCLUDED = new Set(["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "INPUT", "TEXTAREA", "SELECT"]);
const BLOCKS = new Set(["P", "DIV", "LI", "TR", "PRE", "H1", "H2", "H3", "H4", "H5", "H6", "SECTION", "ARTICLE", "HEADER", "FOOTER", "BLOCKQUOTE", "TEXTAREA"]);
export const MAX_TEXT_PAGE_BYTES = 1024;

export function renderedNodes(document, strings) {
  const nodes = document.nodes ?? {};
  const layout = document.layout ?? {};
  const result = new Map();
  // CDP textValue is the current native textarea value, including values set
  // after loading. Child DOM text is its initial value and may be obsolete.
  const paneValues = new Map((nodes.textValue?.index ?? []).map((index, i) =>
    [index, strings[nodes.textValue.value?.[i]]]));
  const hidden = new Set();
  const invisible = new Set();
  for (let i = 0; i < (layout.nodeIndex?.length ?? 0); i++) {
    const [visibility, opacity, display] = (layout.styles?.[i] ?? []).map(s => strings[s]);
    if (visibility === "hidden" || visibility === "collapse") invisible.add(layout.nodeIndex[i]);
    if (opacity === "0" || display === "none") hidden.add(layout.nodeIndex[i]);
  }
  for (let i = 0; i < (layout.nodeIndex?.length ?? 0); i++) {
    const index = layout.nodeIndex[i];
    // Computed visibility reflects descendant overrides. Ancestor opacity/display
    // still suppress the whole subtree, irrespective of the child's visibility.
    if (invisible.has(index)) continue;
    let parent = index;
    let excluded = false;
    // Parent indexes in DOMSnapshot are topologically ordered.
    while (Number.isInteger(parent) && parent >= 0) {
      if (hidden.has(parent) || nodes.nodeType?.[index] === 3 && EXCLUDED.has(strings[nodes.nodeName?.[parent]])) { excluded = true; break; }
      const next = nodes.parentIndex?.[parent];
      if (!(next < parent)) break;
      parent = next;
    }
    if (!excluded) {
      const attributes = nodes.attributes?.[index] ?? [];
      const readOnlyPane = strings[nodes.nodeName?.[index]] === "TEXTAREA" &&
        attributes.some((attribute, position) => position % 2 === 0 && strings[attribute]?.toLowerCase() === "readonly");
      result.set(index, readOnlyPane ? paneValues.get(index) : strings[layout.text?.[i]]);
    }
  }
  return result;
}

// DOMSnapshot separates same-renderer child documents from their embedding
// nodes. Styles in the child cannot report opacity/visibility on that owner.
function renderedDocuments(snapshot) {
  const documents = snapshot.documents ?? [];
  const strings = snapshot.strings ?? [];
  const rendered = documents.map(document => renderedNodes(document, strings));
  const owners = new Map();
  for (let document = 0; document < documents.length; document++) {
    const children = documents[document].nodes?.contentDocumentIndex ?? {};
    for (let i = 0; i < (children.index?.length ?? 0); i++) {
      owners.set(children.value?.[i], { document, node: children.index[i] });
    }
  }
  const visible = new Map();
  const visit = (index, pending = new Set()) => {
    if (visible.has(index)) return visible.get(index);
    if (pending.has(index)) return false;
    pending.add(index);
    const owner = owners.get(index);
    const included = !owner || rendered[owner.document]?.has(owner.node) && visit(owner.document, pending);
    visible.set(index, !!included);
    return !!included;
  };
  return documents.flatMap((document, index) => visit(index) ? [{ document, rendered: rendered[index] }] : []);
}

// Internal capture metadata only; it is removed before the controller reply.
// Isolated renderers need the same embedding check in their parent's snapshot.
export function renderedFrameOwners(snapshot) {
  const strings = snapshot.strings ?? [];
  return renderedDocuments(snapshot).flatMap(({ document, rendered }) =>
    [...rendered.keys()].filter(index => ["IFRAME", "FRAME"].includes(strings[document.nodes?.nodeName?.[index]]))
      .map(index => document.nodes?.backendNodeId?.[index]).filter(Number.isSafeInteger));
}

export function renderedSnapshotText(snapshot) {
  const strings = snapshot.strings ?? [];
  const documents = [];
  for (const { document, rendered } of renderedDocuments(snapshot)) {
    const nodes = document.nodes ?? {};
    let output = "";
    let previousBlock;
    let previousCell;
    for (const [index, text] of [...rendered].sort((a, b) => a[0] - b[0])) {
      if (strings[nodes.nodeName?.[index]] === "BR") { output += "\n"; continue; }
      const readOnlyPane = strings[nodes.nodeName?.[index]] === "TEXTAREA";
      if ((nodes.nodeType?.[index] !== 3 && !readOnlyPane) || !text) continue;
      let parent = readOnlyPane ? index : nodes.parentIndex?.[index];
      let block = -1;
      let cell = -1;
      while (Number.isInteger(parent) && parent >= 0) {
        const tag = strings[nodes.nodeName?.[parent]];
        if (cell < 0 && (tag === "TD" || tag === "TH")) cell = parent;
        if (BLOCKS.has(tag)) { block = parent; break; }
        const next = nodes.parentIndex?.[parent];
        if (!(next < parent)) break;
        parent = next;
      }
      if (previousBlock !== undefined && previousBlock !== block && !output.endsWith("\n")) output += "\n";
      else if (previousCell !== undefined && previousCell !== cell && cell >= 0) output += "\t";
      output += text;
      previousBlock = block;
      previousCell = cell;
    }
    if (output) documents.push(output);
  }
  return documents.join("\n");
}

export function browserTextPage(text, request = {}) {
  const { query = null, offset = 0, max_bytes: maxBytes = MAX_TEXT_PAGE_BYTES } = request;
  if ((query !== null && (typeof query !== "string" || Buffer.byteLength(query) > 2048)) ||
      !Number.isSafeInteger(offset) || offset < 0 ||
      !Number.isSafeInteger(maxBytes) || maxBytes < 4 || maxBytes > MAX_TEXT_PAGE_BYTES) {
    throw new BrowserTextError("browser text requires a bounded query, byte offset and max_bytes (4..1024)");
  }
  if (query) {
    const lines = text.split("\n");
    const keep = new Set();
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].includes(query)) for (let j = Math.max(0, i - 1); j <= Math.min(lines.length - 1, i + 1); j++) keep.add(j);
    }
    text = [...keep].map(i => lines[i]).join("\n");
  }
  const bytes = Buffer.from(text);
  const start = Math.min(offset, bytes.length);
  if (start < bytes.length && (bytes[start] & 0xc0) === 0x80) {
    throw new BrowserTextError("browser text offset must be a UTF-8 boundary");
  }
  let end = Math.min(start + maxBytes, bytes.length);
  while (end < bytes.length && (bytes[end] & 0xc0) === 0x80) end--;
  return { text: bytes.subarray(start, end).toString(), offset: start, next_offset: end < bytes.length ? end : null, total_bytes: bytes.length, query };
}
