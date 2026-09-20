import test from "node:test";
import assert from "node:assert/strict";

import {
  BrowserObservationStore,
  OBSERVATION_ERROR_CODES,
} from "./browser-controller-observations.mjs";

const TAB = Object.freeze({
  tab_id: "tab-1",
  generation: 3,
  target_generation: 2,
});

function frameTree(documentId) {
  return { frameTree: { frame: { loaderId: documentId } } };
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
  constructor({ documents = ["document-1"], accessibilityResults, domResults } = {}) {
    this.documents = [...documents];
    this.accessibilityResults = accessibilityResults ?? [accessibility()];
    this.domResults = domResults ?? [domSnapshot()];
    this.calls = [];
    this.frameIndex = 0;
    this.captureIndex = 0;
  }

  async send(method, params) {
    this.calls.push({ method, params });
    if (method === "Page.getFrameTree") {
      const index = Math.min(this.frameIndex++, this.documents.length - 1);
      return frameTree(this.documents[index]);
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
    snapshot_revision: 2,
    backend_node_id: 11,
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
  assert.equal(store.resolve(TAB, stable.accessibility_nodes[0].element_ref).snapshot_revision, 1);
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
