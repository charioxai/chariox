import test from "node:test";
import assert from "node:assert/strict";

import {
  BrowserObservationStore,
  OBSERVATION_ERROR_CODES,
  OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES,
} from "./browser-controller-observations.mjs";

const TAB = Object.freeze({
  tab_id: "tab-1",
  generation: 3,
  target_generation: 2,
});

function frameTree(documentId, childDocumentId = null) {
  const root = { frame: { id: "frame-main", loaderId: documentId } };
  if (childDocumentId !== null) {
    root.childFrames = [{ frame: { id: "child-frame", loaderId: childDocumentId } }];
  }
  return { frameTree: root };
}

function accessibility({ password = "secret-value", extra = [] } = {}) {
  return {
    nodes: [
      {
        nodeId: "ax-root",
        backendDOMNodeId: 10,
        childIds: ["ax-input"],
        role: { value: "document" },
        name: { value: "Account" },
      },
      {
        nodeId: "ax-input",
        backendDOMNodeId: 11,
        parentId: "ax-root",
        role: { value: "textbox" },
        name: { value: "Password" },
        value: { value: password },
        properties: [{ name: "protected", value: { value: true } }],
      },
      ...extra,
    ],
  };
}

function domSnapshot({ password = "secret-value", scriptText = "token=hidden" } = {}) {
  const strings = [
    "HTML",
    "INPUT",
    "type",
    "password",
    "value",
    password,
    "SCRIPT",
    "#text",
    scriptText,
  ];
  return {
    strings,
    documents: [{
      frameId: "frame-main",
      nodes: {
        backendNodeId: [10, 11, 12, 13],
        parentIndex: [-1, 0, 0, 2],
        nodeType: [1, 1, 1, 3],
        nodeName: [0, 1, 6, 7],
        nodeValue: [-1, -1, -1, 8],
        attributes: [[], [2, 3, 4, 5], [], []],
      },
      layout: {
        nodeIndex: [1],
        bounds: [[10, 20, 200, 30]],
      },
    }],
  };
}

class FakeConnection {
  constructor({ documents = ["document-1"], frameTrees, accessibilityResults, domResults } = {}) {
    this.documents = [...documents];
    this.frameTrees = frameTrees ?? this.documents.map((documentId) => frameTree(documentId));
    this.accessibilityResults = accessibilityResults ?? [accessibility()];
    this.domResults = domResults ?? [domSnapshot()];
    this.calls = [];
    this.frameIndex = 0;
    this.captureIndex = 0;
  }

  async send(method, params, timeoutMs, options) {
    this.calls.push({ method, params, timeoutMs, options });
    if (method === "Page.getFrameTree") {
      const index = Math.min(this.frameIndex++, this.frameTrees.length - 1);
      return this.frameTrees[index];
    }
    if (method === "Accessibility.getFullAXTree") {
      return this.accessibilityResults[Math.min(this.captureIndex, this.accessibilityResults.length - 1)];
    }
    if (method === "DOMSnapshot.captureSnapshot") {
      const result = this.domResults[Math.min(this.captureIndex, this.domResults.length - 1)];
      this.captureIndex += 1;
      return result;
    }
    throw new Error(`unexpected method ${method}`);
  }
}

test("keeps opaque element references stable across snapshots of one document", async () => {
  const store = new BrowserObservationStore();
  const connection = new FakeConnection({ documents: ["document-1"] });
  const first = await store.capture({ connection, tab: TAB });
  const second = await store.capture({ connection, tab: TAB });

  assert.equal(first.snapshot_revision, 1);
  assert.equal(second.snapshot_revision, 2);
  assert.equal(first.accessibility_nodes[1].element_ref, second.accessibility_nodes[1].element_ref);
  assert.equal(first.accessibility_nodes[1].element_ref, "element-1-2");
  assert.doesNotMatch(JSON.stringify(first), /backend_node|backendDOMNodeId|secret-value/);
  assert.equal(first.accessibility_nodes[1].value, "[redacted]");
  assert.equal(first.dom_nodes[1].attributes.value, "[redacted]");
  assert.equal(first.dom_nodes[3].text, "");
  assert.deepEqual(first.dom_nodes[1].bounds, { x: 10, y: 20, width: 200, height: 30 });

  const resolved = store.resolve(TAB, first.accessibility_nodes[1].element_ref);
  assert.deepEqual(resolved, {
    tab_id: "tab-1",
    browser_generation: 3,
    target_generation: 2,
    document_id: "document-1",
    frame_id: "frame-main",
    main_frame_id: "frame-main",
    snapshot_revision: 2,
    backend_node_id: 11,
  });
});

test("binds every element reference to its owning frame", async () => {
  const store = new BrowserObservationStore();
  const connection = new FakeConnection({
    accessibilityResults: [{
      nodes: [
        {
          nodeId: "main",
          backendDOMNodeId: 10,
          frameId: "frame-main",
          role: { value: "document" },
        },
        {
          nodeId: "child-button",
          backendDOMNodeId: 21,
          frameId: "frame-child",
          role: { value: "button" },
        },
      ],
    }],
    domResults: [{
      strings: ["HTML", "BUTTON"],
      documents: [
        {
          frameId: "frame-main",
          nodes: {
            backendNodeId: [10],
            parentIndex: [-1],
            nodeType: [1],
            nodeName: [0],
            nodeValue: [-1],
            attributes: [[]],
          },
          layout: { nodeIndex: [], bounds: [] },
        },
        {
          frameId: "frame-child",
          nodes: {
            backendNodeId: [21],
            parentIndex: [-1],
            nodeType: [1],
            nodeName: [1],
            nodeValue: [-1],
            attributes: [[]],
          },
          layout: { nodeIndex: [0], bounds: [[1, 2, 30, 20]] },
        },
      ],
    }],
  });
  const snapshot = await store.capture({ connection, tab: TAB });
  const child = snapshot.accessibility_nodes.find((node) => node.role === "button");
  assert.deepEqual(store.resolve(TAB, child.element_ref), {
    tab_id: "tab-1",
    browser_generation: 3,
    target_generation: 2,
    document_id: "document-1",
    frame_id: "frame-child",
    main_frame_id: "frame-main",
    snapshot_revision: 1,
    backend_node_id: 21,
  });
});

test("invalidates old references after a committed document navigation", async () => {
  const store = new BrowserObservationStore();
  const first = await store.capture({ connection: new FakeConnection(), tab: TAB });
  const oldReference = first.accessibility_nodes[1].element_ref;
  const nextConnection = new FakeConnection({ documents: ["document-2"] });
  const second = await store.capture({ connection: nextConnection, tab: TAB });

  assert.equal(second.document_id, "document-2");
  assert.equal(second.snapshot_revision, 1);
  assert.equal(second.accessibility_nodes[1].element_ref, "element-2-2");
  assert.throws(
    () => store.resolve(TAB, "element-99"),
    (error) => error.code === OBSERVATION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED,
  );
  assert.throws(
    () => store.resolve(TAB, oldReference),
    (error) => error.code === OBSERVATION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED,
  );
});

test("rejects navigation during capture without committing a partial snapshot", async () => {
  const store = new BrowserObservationStore();
  const stable = await store.capture({ connection: new FakeConnection(), tab: TAB });
  await assert.rejects(
    store.capture({
      connection: new FakeConnection({ documents: ["document-1", "document-2"] }),
      tab: TAB,
    }),
    (error) => error.code === OBSERVATION_ERROR_CODES.STALE_DOCUMENT,
  );
  assert.throws(
    () => store.resolve(TAB, stable.accessibility_nodes[0].element_ref),
    (error) => error.code === OBSERVATION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED,
  );
});

test("invalidates references when a child frame navigates during capture", async () => {
  const store = new BrowserObservationStore();
  const first = await store.capture({
    connection: new FakeConnection({
      frameTrees: [frameTree("document-1", "child-1"), frameTree("document-1", "child-1")],
    }),
    tab: TAB,
  });
  const reference = first.accessibility_nodes[0].element_ref;

  await assert.rejects(
    store.capture({
      connection: new FakeConnection({
        frameTrees: [frameTree("document-1", "child-1"), frameTree("document-1", "child-2")],
      }),
      tab: TAB,
    }),
    (error) => error.code === OBSERVATION_ERROR_CODES.STALE_DOCUMENT,
  );
  assert.throws(
    () => store.resolve(TAB, reference),
    (error) => error.code === OBSERVATION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED,
  );
});

test("redacts secret-bearing attribute values while preserving public URLs", async () => {
  const strings = [
    "A",
    "href",
    "https://public.example/docs/start",
    "src",
    "https://cdn.example/app.js?access_token=private",
    "action",
    "https://user:password@example.test/submit",
    "data-info",
    "kind=public token=private",
    "signed",
    "https://cdn.example/app.js?X-Amz-Signature=private",
    "json",
    "{\"access_token\":\"private\"}",
    "relative",
    "//user:password@example.test/path",
  ];
  const rawDom = {
    strings,
    documents: [{
      nodes: {
        backendNodeId: [21],
        parentIndex: [-1],
        nodeType: [1],
        nodeName: [0],
        nodeValue: [-1],
        attributes: [[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]],
      },
    }],
  };
  const store = new BrowserObservationStore();
  const snapshot = await store.capture({
    connection: new FakeConnection({ domResults: [rawDom] }),
    tab: TAB,
  });
  const attributes = snapshot.dom_nodes[0].attributes;
  assert.equal(attributes.href, "https://public.example/docs/start");
  assert.equal(attributes.src, "[redacted]");
  assert.equal(attributes.action, "[redacted]");
  assert.equal(attributes["data-info"], "[redacted]");
  assert.equal(attributes.signed, "[redacted]");
  assert.equal(attributes.json, "[redacted]");
  assert.equal(attributes.relative, "[redacted]");
  assert.doesNotMatch(JSON.stringify(snapshot), /private|password/);
});

test("uses the finite raw snapshot budget without changing compact result bounds", async () => {
  const padding = "x".repeat(70 * 1024);
  const connection = new FakeConnection({
    accessibilityResults: [{ nodes: [], padding }],
    domResults: [{ ...domSnapshot(), padding }],
  });
  const store = new BrowserObservationStore();
  const snapshot = await store.capture({ connection, tab: TAB });
  assert.equal(snapshot.accessibility_nodes.length, 0);
  const snapshotCalls = connection.calls.filter(({ method }) => (
    method === "Accessibility.getFullAXTree" || method === "DOMSnapshot.captureSnapshot"
  ));
  assert.equal(snapshotCalls.length, 2);
  assert.deepEqual(
    snapshotCalls.map(({ options }) => options?.maxResponseBytes),
    [OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES, OBSERVATION_RAW_SNAPSHOT_LIMIT_BYTES],
  );
});

test("invalidates references when registry generation or target generation changes", async () => {
  const store = new BrowserObservationStore();
  const snapshot = await store.capture({ connection: new FakeConnection(), tab: TAB });
  const reference = snapshot.accessibility_nodes[0].element_ref;
  store.reconcile([{ ...TAB, target_generation: 3 }]);
  assert.throws(
    () => store.resolve(TAB, reference),
    (error) => error.code === OBSERVATION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED,
  );

  await store.capture({ connection: new FakeConnection(), tab: { ...TAB, target_generation: 3 } });
  store.reconcile([{ ...TAB, generation: 4, target_generation: 1 }]);
  assert.throws(
    () => store.resolve({ ...TAB, target_generation: 3 }, "element-2-1"),
    (error) => error.code === OBSERVATION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED,
  );
});

test("bounds node counts and result bytes deterministically", async () => {
  const extra = Array.from({ length: 8 }, (_, index) => ({
    nodeId: `extra-${index}`,
    backendDOMNodeId: 20 + index,
    role: { value: "button" },
    name: { value: `Button ${index}` },
  }));
  const store = new BrowserObservationStore();
  const snapshot = await store.capture({
    connection: new FakeConnection({ accessibilityResults: [accessibility({ extra })] }),
    tab: TAB,
    limits: { maxNodes: 3 },
  });
  assert.equal(snapshot.accessibility_nodes.length, 3);
  assert.equal(snapshot.truncated, true);

  const freshStore = new BrowserObservationStore();
  await assert.rejects(
    freshStore.capture({
      connection: new FakeConnection(),
      tab: TAB,
      limits: { maxResultBytes: 64 },
    }),
    (error) => error.code === OBSERVATION_ERROR_CODES.SNAPSHOT_TOO_LARGE,
  );
  assert.throws(
    () => freshStore.resolve(TAB, "element-1-1"),
    (error) => error.code === OBSERVATION_ERROR_CODES.ELEMENT_REFERENCE_INVALIDATED,
  );
});

test("fails closed on malformed document identity", async () => {
  const store = new BrowserObservationStore();
  const connection = {
    async send(method) {
      if (method === "Page.getFrameTree") return { frameTree: { frame: {} } };
      throw new Error("snapshot should not start");
    },
  };
  await assert.rejects(
    store.capture({ connection, tab: TAB }),
    (error) => error.code === OBSERVATION_ERROR_CODES.CDP_PROTOCOL_INVALID,
  );
});
