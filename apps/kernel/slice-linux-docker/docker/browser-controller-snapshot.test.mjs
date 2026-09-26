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
  // The cut is reported even though dropped nodes could leave fewer than the bound.
  assert.equal(snapshot.accessibility_truncated, true);
  assert.equal((await capture(fixture(), {})).accessibility_truncated, false);
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

test("snapshot carries the control states a screen reader announces", async () => {
  const connection = {
    async send(method, params) {
      if (method === "Page.getFrameTree") return { frameTree: { frame: { id: "root", loaderId: "page-1" } } };
      if (method === "DOMSnapshot.captureSnapshot") return { documents: [], strings: [] };
      const property = (name, value) => ({ name, value: { value } });
      return { nodes: [
        { nodeId: "1", backendDOMNodeId: 1, role: { value: "checkbox" }, properties: [property("checked", "true"), property("required", true)] },
        { nodeId: "2", backendDOMNodeId: 2, role: { value: "checkbox" }, properties: [property("checked", "false")] },
        { nodeId: "3", backendDOMNodeId: 3, role: { value: "button" }, properties: [property("pressed", "mixed"), property("expanded", false)] },
        { nodeId: "4", backendDOMNodeId: 4, role: { value: "textbox" }, properties: [property("invalid", "false"), property("focused", true)] },
        { nodeId: "5", backendDOMNodeId: 5, role: { value: "option" }, properties: [property("selected", true), property("invalid", "true")] },
      ] };
    },
  };
  const snapshot = await capture(connection);
  assert.deepEqual(snapshot.accessibility_nodes.map((node) => node.states), [
    ["checked", "required"], ["not checked"], ["mixed", "collapsed"], [], ["selected", "invalid"],
  ]);
});
