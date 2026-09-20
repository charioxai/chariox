import assert from "node:assert/strict";
import test from "node:test";
import { captureBrowserSnapshot } from "./browser-controller-snapshot.mjs";

const root = { frame: { id: "root", loaderId: "page-1" }, childFrames: [
  { frame: { id: "child", loaderId: "child-1" } },
] };

function fixture({ after = root, frames = root } = {}) {
  let reads = 0;
  const requestedFrames = [];
  return {
    requestedFrames,
    async send(method, params) {
      if (method === "Page.getFrameTree") return { frameTree: reads++ === 0 ? frames : after };
      if (method === "DOMSnapshot.captureSnapshot") return { documents: [], strings: [] };
      assert.equal(method, "Accessibility.getFullAXTree");
      requestedFrames.push(params.frameId);
      const base = params.frameId === "root" ? 100 : 200;
      // Frame-local AX identifiers deliberately collide across documents.
      return { nodes: [
        { nodeId: "1", backendDOMNodeId: base, childIds: ["2"], role: { value: "textbox" }, name: { value: params.frameId } },
        { nodeId: "2", backendDOMNodeId: base + 1, parentId: "1", role: { value: "StaticText" } },
      ] };
    },
  };
}

const capture = (connection, limits) => captureBrowserSnapshot({
  connection, sessionId: "page", targetId: "target", documentId: "page-1",
  browserGeneration: 1, snapshotRevision: 1, limits,
});

test("snapshot preserves each frame's labels and frame-local AX relationships", async () => {
  const snapshot = await capture(fixture());
  assert.deepEqual(snapshot.accessibility_nodes.map((node) => [node.node_ref, node.parent_ref, node.child_refs]), [
    ["backend:100", null, ["backend:101"]], ["backend:101", "backend:100", []],
    ["backend:200", null, ["backend:201"]], ["backend:201", "backend:200", []],
  ]);
  assert.deepEqual(snapshot.accessibility_nodes.filter((node) => node.role === "textbox").map((node) => node.name), ["root", "child"]);
});

test("accessibility node budget is shared across frames", async () => {
  const connection = fixture();
  const snapshot = await capture(connection, { maxNodes: 2 });
  assert.equal(snapshot.accessibility_nodes.length, 2);
  assert.deepEqual(connection.requestedFrames, ["root"]);
});

test("snapshot rejects a child document navigation even when the top document stays unchanged", async () => {
  const after = structuredClone(root);
  after.childFrames[0].frame.loaderId = "child-2";
  await assert.rejects(capture(fixture({ after })), { code: "stale_document_reference" });
});

test("snapshot rejects excessive frame count before reading accessibility trees", async () => {
  const connection = fixture();
  await assert.rejects(capture(connection, { maxFrames: 1 }), { code: "snapshot_frame_limit" });
  assert.deepEqual(connection.requestedFrames, []);
});

test("classifies sensitive snapshot fields before truncation", async () => {
  const frameTree = { frame: { id: "root", loaderId: "page-1" } };
  const rawDom = {
    strings: [
      "DIV",
      "field-password",
      "visible-secret",
      "info",
      "safe-prefix-token=hidden",
      "billing-payment",
      "card-number",
      "form-private-input",
      "private-value",
      "ordinary",
      "public-value",
    ],
    documents: [{
      documentURL: -1,
      nodes: {
        backendNodeId: [23],
        parentIndex: [-1],
        nodeType: [1],
        nodeName: [0],
        nodeValue: [-1],
        attributes: [[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]],
      },
    }],
  };
  const connection = {
    async send(method) {
      if (method === "Page.getFrameTree") return { frameTree };
      if (method === "Accessibility.getFullAXTree") {
        return { nodes: [{
          nodeId: "1",
          backendDOMNodeId: 23,
          role: { value: "textbox" },
          name: { value: "safe-prefix-password" },
          value: { value: "visible-private-value" },
        }] };
      }
      assert.equal(method, "DOMSnapshot.captureSnapshot");
      return rawDom;
    },
  };

  const snapshot = await capture(connection, { maxStringLength: 8 });
  const attributes = snapshot.dom_nodes[0].attributes;
  assert.equal(attributes["field-pa"], "[redacted]");
  assert.equal(attributes.info, "[redacted]");
  assert.equal(attributes["billing-"], "[redacted]");
  assert.equal(attributes["form-pri"], "[redacted]");
  assert.equal(attributes.ordinary, "public-v");
  assert.equal(snapshot.accessibility_nodes[0].value, "[redacted]");
  assert.doesNotMatch(
    JSON.stringify(snapshot),
    /visible-secret|hidden|card-number|private-value|password|payment|private-input/,
  );
});
