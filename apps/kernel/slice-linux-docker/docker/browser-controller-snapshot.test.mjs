import assert from "node:assert/strict";
import test from "node:test";
import * as observations from "./browser-controller-snapshot.mjs";
const { captureBrowserSnapshot, redactObservation } = observations;

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

test('MP-08/MP-10/MP-11 rendered box appearance comes from computed layout, not page attributes', async () => {
  const connection = {
    async send(method, params) {
      if (method === 'Page.getFrameTree') return {frameTree:{frame:{loaderId:'page-1'}}};
      if (method === 'Accessibility.getFullAXTree') return {nodes:[]};
      const strings = ['DIV','visible','1','block','rgb(30, 40, 50)','rgb(60, 70, 80)','2px','dashed'];
      assert.ok(params.computedStyles.includes('background-color'));
      return {strings,documents:[{nodes:{backendNodeId:[1],nodeType:[1],nodeName:[0],parentIndex:[-1],attributes:[[]]},
        layout:{nodeIndex:[0],bounds:[[10,20,40,50]],styles:[[1,2,3,4,5,6,7]]}}]};
    },
  };
  const snapshot = await capture(connection);
  assert.deepEqual(snapshot.dom_nodes[0].bounds,{x:10,y:20,width:40,height:50});
  assert.equal(snapshot.dom_nodes[0].attributes['chariox-rendered-background-color'],'rgb(30, 40, 50)');
  assert.equal(snapshot.dom_nodes[0].attributes['chariox-rendered-border-top-style'],'dashed');
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

test("MP-08/MP-10/MP-11 Vault page-copy AX and DOM are scrubbed before bounded compaction", async () => {
  const syntheticValue = "vault-page-copy-regression-value";
  const connection = {
    async send(method) {
      if (method === "Page.getFrameTree") return { frameTree: { frame: { id: "root", loaderId: "page-1" } } };
      if (method === "DOMSnapshot.captureSnapshot") return {
        strings: ["#text", syntheticValue],
        documents: [{ nodes: { backendNodeId: [2], nodeType: [3], nodeName: [0], nodeValue: [1], parentIndex: [-1], attributes: [[]] }, layout: { nodeIndex: [0], bounds: [[0, 0, 200, 20]] } }],
      };
      assert.equal(method, "Accessibility.getFullAXTree");
      return { nodes: [
        { nodeId: "password", backendDOMNodeId: 1, role: { value: "textbox" }, name: { value: "Password" },
          value: { value: syntheticValue }, properties: [{ name: "protected", value: { value: true } }] },
        { nodeId: "echo", backendDOMNodeId: 2, role: { value: "StaticText" }, name: { value: syntheticValue } },
      ] };
    },
  };
  const snapshot = await captureBrowserSnapshot({ connection, sessionId: "page", targetId: "target", documentId: "page-1",
    browserGeneration: 1, snapshotRevision: 1, protectedValues: [syntheticValue], limits: { maxStringLength: 16 } });
  assert.equal(snapshot.accessibility_nodes[0].value, "[redacted]");
  assert.equal(snapshot.accessibility_nodes[1].name, "[redacted]");
  assert.equal(snapshot.dom_nodes[0].text, "[redacted]");
  assert.equal(JSON.stringify(snapshot).includes(syntheticValue.slice(0, 16)), false);
});

test("MP-08/MP-10/MP-11 known-value scrubbing covers nested results, keys and simple transforms", () => {
  const syntheticValue = "Synthetic<value+ä>";
  const variants = [syntheticValue, syntheticValue.toLowerCase(), syntheticValue.toUpperCase(),
    encodeURIComponent(syntheticValue), encodeURIComponent(syntheticValue).toLowerCase(),
    Buffer.from(syntheticValue).toString("base64"), Buffer.from(syntheticValue).toString("base64url"),
    Buffer.from(syntheticValue).toString("hex"), Buffer.from(syntheticValue).toString("hex").toUpperCase(),
    "Synthetic&lt;value+ä&gt;"];
  const result = redactObservation({ [syntheticValue]: variants, nested: { output: syntheticValue } }, [syntheticValue]);
  assert.deepEqual(result["[redacted]"], variants.map(() => "[redacted]"));
  assert.equal(result.nested.output, "[redacted]");
});

test("MP-08/MP-10/MP-11 preserves finite numeric AX values with protected-value redaction", async () => {
  const values = [0, -0.5, 42, 91827, Infinity, NaN, true];
  const connection = {
    async send(method) {
      if (method === "Page.getFrameTree") return { frameTree: { frame: { id: "root", loaderId: "page-1" } } };
      if (method === "DOMSnapshot.captureSnapshot") return { documents: [], strings: [] };
      return { nodes: values.map((value, index) => ({
        nodeId: String(index + 1), backendDOMNodeId: index + 1,
        role: { value: "slider" }, value: { type: "number", value },
        properties: index === 2 ? [{ name: "protected", value: { value: true } }] : [],
      })) };
    },
  };
  const snapshot = await captureBrowserSnapshot({ connection, sessionId: "page", targetId: "target", documentId: "page-1",
    browserGeneration: 1, snapshotRevision: 1, protectedValues: ["91827"], limits: { maxStringLength: 16 } });
  assert.deepEqual(snapshot.accessibility_nodes.map(node => node.value), ["0", "-0.5", "[redacted]", "[redacted]", "", "", ""]);
});
