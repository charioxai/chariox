import assert from "node:assert/strict";
import test from "node:test";
import { captureRegionMasks, protectedHostRegions } from "./kernel-browser-region-protection.mjs";

function fixture() {
  const state = { x: 10, fail: false, calls: [], shadow: false, shadowRootType: undefined };
  const connection = { async send(method) {
    state.calls.push(method);
    if (state.fail) throw new Error("metadata unavailable");
    if (method === "DOM.getDocument") return { root: { nodeId: 1, children: state.shadow ? [{ nodeId: 3, shadowRoots: [{ shadowRootType: state.shadowRootType }] }] : [] } };
    if (method === "DOM.querySelectorAll") return { nodeIds: [2] };
    assert.equal(method, "DOM.getBoxModel");
    return { model: { border: [state.x, 20, state.x + 30, 20, state.x + 30, 60, state.x, 60] } };
  } };
  return { state, connection };
}
test("region masks preserve stable protected field bounds using only CDP DOM metadata", async () => {
  const { state, connection } = fixture();
  const masks = await captureRegionMasks(connection, "session");
  assert.deepEqual(await masks.afterCapture(), [{ x: 10, y: 20, width: 30, height: 40 }]);
  assert(state.calls.every(method => method.startsWith("DOM.")));
});
for (const change of ["layout", "metadata"]) {
  test(`region masks cover the full frame after a ${change} change during capture`, async () => {
    const { state, connection } = fixture();
    const masks = await captureRegionMasks(connection, "session");
    if (change === "layout") state.x = 100;
    else state.fail = true;
    assert.deepEqual(await masks.afterCapture(), [{ x: 0, y: 0, width: 1280, height: 800 }]);
  });
}
test("region capture protects shadow hosts and refuses unavailable initial protection", async () => {
  const { state, connection } = fixture();
  state.shadow = true;
  const masks = await captureRegionMasks(connection, "session");
  assert.equal((await masks.afterCapture()).length, 2);
  state.fail = true;
  await assert.rejects(captureRegionMasks(connection, "session"), /unavailable/);
});
test("MP-11: native input internals do not protect ordinary fields; explicit protected fields remain masked", async () => {
  const { state, connection } = fixture();
  state.shadow = true;
  // MP-08/MP-11: user-agent and inspected open roots clear their host;
  // closed or unknown roots keep the host opaque.
  for (const [kind, count] of [["user-agent", 1], ["open", 1], ["closed", 2], [undefined, 2]]) {
    state.shadowRootType = kind;
    assert.equal((await protectedHostRegions(connection, "session")).length, count, String(kind));
  }
});

test('MP-11: structured mirror skips native control shadow trees but checks open roots and masks closed roots',async()=>{
 const connection={async send(method,params){
  if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'root',securityOrigin:'http://fixture'}}};
  if(method==='DOM.getDocument')return {root:{nodeId:1,children:[
   {nodeId:10,shadowRoots:[{nodeId:11,shadowRootType:'user-agent'}]},
   {nodeId:20,shadowRoots:[{nodeId:21,shadowRootType:'open',children:[{nodeId:22,localName:'input',attributes:['type','password']}]}]},
   {nodeId:30,shadowRoots:[{nodeId:31,shadowRootType:'closed'}]},
  ]}};
  if(method==='DOM.querySelectorAll')return {nodeIds:[]};
  assert.equal(method,'DOM.getBoxModel');assert([22,30].includes(params.nodeId));return {model:{border:[0,0,10,0,10,10,0,10]}};
 }};
 const masks=await captureRegionMasks(connection,'session',{});assert.equal((await masks.afterCapture()).length,2);
});

test('MP-11: in-process frames of any origin are inspected; a frame without a document or session stays opaque',async()=>{
 for(const kind of ['same','foreign','missing']){
  const boxed=[],reasons=[];const connection={async send(method,params){
   if(method==='DOM.getDocument')return {root:{nodeId:1,children:[{nodeId:2,localName:'iframe',frameId:'child',...(kind==='missing'?{}:{contentDocument:{nodeId:3,children:[{nodeId:4,localName:'input',attributes:['type','password']}]}})}]}};
   if(method==='DOM.querySelectorAll')return {nodeIds:[2]};
   assert.equal(method,'DOM.getBoxModel');boxed.push(params.nodeId);return {model:{border:[1,2,10,2,10,20,1,20]}};
  }};
  const masks=await captureRegionMasks(connection,'session',{record:r=>reasons.push(r)});await masks.afterCapture();
  assert.deepEqual(boxed,kind==='missing'?[2,2]:[4,4]);
  assert.deepEqual(reasons,kind==='missing'?['frame_session_unavailable','frame_session_unavailable']:[]);
 }
});

// MP-11: origin admission never overrides an explicit protected-region marker.
test('MP-11: same-origin iframe with explicit protection stays opaque',async()=>{
 for(const marker of ['data-chariox-secret','data-chariox-observation-protected','data-observation-protected']){
  const boxed=[];const connection={async send(method,params){
   if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'main',securityOrigin:'http://fixture'},childFrames:[{frame:{id:'child',securityOrigin:'http://fixture'}}]}};
   if(method==='DOM.getDocument')return {root:{nodeId:1,children:[{nodeId:2,localName:'iframe',frameId:'child',attributes:[marker,''],contentDocument:{nodeId:3,children:[]}}]}};
   if(method==='DOM.querySelectorAll')return {nodeIds:[2]};
   assert.equal(method,'DOM.getBoxModel');boxed.push(params.nodeId);return {model:{border:[1,2,10,2,10,20,1,20]}};
  }};
  const masks=await captureRegionMasks(connection,'session');assert.equal((await masks.afterCapture()).length,1);assert.deepEqual(boxed,[2,2]);
 }
});

test('MP-11: nested protected input attributes are ASCII case insensitive',async()=>{
 const boxed=[];const connection={async send(method,params){
  if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'main',securityOrigin:'http://fixture'},childFrames:[{frame:{id:'child',securityOrigin:'http://fixture'}}]}};
  if(method==='DOM.getDocument')return {root:{nodeId:1,children:[{nodeId:2,localName:'iframe',frameId:'child',contentDocument:{nodeId:3,children:[{nodeId:4,localName:'input',attributes:['type','PASSWORD']},{nodeId:5,localName:'input',attributes:['autocomplete','CC-NUMBER']}]}}]}};
  if(method==='DOM.querySelectorAll')return {nodeIds:[2]};assert.equal(method,'DOM.getBoxModel');boxed.push(params.nodeId);return {model:{border:[0,0,10,0,10,10,0,10]}};
 }};const masks=await captureRegionMasks(connection,'session',{});assert.equal((await masks.afterCapture()).length,2);assert.deepEqual(new Set(boxed),new Set([4,5]));
});

// MP-08/MP-10/MP-11 phase 1.3: real-site admission (hidden tracking frames,
// web-component shadow hosts, offscreen frames) without weakening masks.
import { NativeRegionProtection, regionProtectionChanged } from "./kernel-browser-region-protection.mjs";
const notRendered = () => { throw new Error("Protocol error (DOM.getBoxModel): Could not compute box model."); };
function site(children, boxes, calls = []) {
  return { calls, async send(method, params) {
    calls.push(method);
    if (method === "DOM.getDocument") return { root: { nodeId: 1, localName: "html", children } };
    if (method === "DOM.querySelectorAll") { const ids = []; const walk = n => { if (["iframe", "frame"].includes(n.localName) || n.localName === "input" && /password/i.test(n.attributes?.[1] ?? "")) ids.push(n.nodeId); (n.children ?? []).forEach(walk); }; children.forEach(walk); return { nodeIds: ids }; }
    assert.equal(method, "DOM.getBoxModel");
    const box = boxes[params.nodeId];
    if (box === undefined) notRendered();
    const [x, y, w, h] = box; return { model: { border: [x, y, x + w, y, x + w, y + h, x, y + h] } };
  } };
}
test("MP-11 hidden frames and fields render nothing; an unboundable explicit marker masks the whole viewport", async () => {
  const hidden = site([{ nodeId: 2, localName: "iframe" }, { nodeId: 3, localName: "input", attributes: ["type", "password"] }, { nodeId: 4, localName: "iframe" }], { 4: [10, 10, 100, 50] });
  assert.deepEqual(await protectedHostRegions(hidden, "s"), [{ x: 10, y: 10, width: 100, height: 50 }]);
  const contents = site([{ nodeId: 2, localName: "div", attributes: ["data-chariox-secret", ""] }], {});
  assert.deepEqual(await protectedHostRegions(contents, "s"), [{ x: 0, y: 0, width: 1280, height: 800 }]);
});
test("MP-11 open shadow roots are inspected: the host is cleared, nested fields and frames stay masked", async () => {
  const children = [{ nodeId: 2, localName: "x-card", shadowRoots: [{ nodeId: 3, shadowRootType: "open", children: [
    { nodeId: 4, localName: "input", attributes: ["type", "Password"] }, { nodeId: 5, localName: "iframe" }, { nodeId: 6, localName: "span" }] }] }];
  const regions = await protectedHostRegions(site(children, { 2: [0, 0, 500, 500], 4: [20, 20, 100, 20], 5: [20, 60, 300, 150] }), "s");
  assert.deepEqual(regions, [{ x: 20, y: 20, width: 100, height: 20 }, { x: 20, y: 60, width: 300, height: 150 }]);
});
test("MP-11 offscreen and empty frames are not regions; partly visible frames are clipped", async () => {
  const children = [{ nodeId: 2, localName: "iframe" }, { nodeId: 3, localName: "iframe" }, { nodeId: 4, localName: "iframe" }];
  const regions = await protectedHostRegions(site(children, { 2: [0, 2000, 300, 250], 3: [50, 50, 0, 0], 4: [1200, 700, 300, 250] }), "s");
  assert.deepEqual(regions, [{ x: 1200, y: 700, width: 80, height: 100 }]);
});
test("MP-11 oversized trees degrade to a whole-viewport mask instead of refusing capture", async () => {
  const wide = Array.from({ length: 1100 }, (_, n) => ({ nodeId: 10 + n, localName: "iframe" }));
  assert.deepEqual(await protectedHostRegions(site(wide, {}), "s"), [{ x: 0, y: 0, width: 1280, height: 800 }]);
});
test("MP-08/MP-11 a native tracker retires only on events that can add protection", () => {
  const tracked = { sessions: new Set(["s"]), nodes: new Set(["s 7"]) }, e = (method, params) => ({ sessionId: "s", method, params });
  for (const name of ["type", "autocomplete", "data-chariox-secret", "data-chariox-observation-protected", "data-observation-protected"])
    assert.equal(regionProtectionChanged(e("DOM.attributeModified", { name }), "s", tracked), true, name);
  for (const name of ["style", "class", "aria-hidden"]) assert.equal(regionProtectionChanged(e("DOM.attributeModified", { name }), "s", tracked), false, name);
  const plain = { nodeId: 20, localName: "div", childNodeCount: 1, children: [{ nodeId: 21, localName: "span", childNodeCount: 0 }] };
  assert.equal(regionProtectionChanged(e("DOM.childNodeInserted", { node: plain }), "s", tracked), false);
  for (const node of [{ nodeId: 22, localName: "iframe" }, { nodeId: 23, localName: "x-a", shadowRoots: [{}] }, { nodeId: 24, localName: "div", childNodeCount: 3, children: [] },
    { nodeId: 25, localName: "div", childNodeCount: 1, children: [{ nodeId: 26, localName: "input", attributes: ["autocomplete", "one-time-code"] }] }, undefined])
    assert.equal(regionProtectionChanged(e("DOM.childNodeInserted", { node }), "s", tracked), true, JSON.stringify(node));
  assert.equal(regionProtectionChanged(e("DOM.childNodeRemoved", { nodeId: 7 }), "s", tracked), true);
  assert.equal(regionProtectionChanged(e("DOM.childNodeRemoved", { nodeId: 8 }), "s", tracked), false);
  for (const method of ["DOM.documentUpdated", "DOM.shadowRootPushed", "DOM.childNodeCountUpdated"]) assert.equal(regionProtectionChanged(e(method, {}), "s", tracked), true, method);
  assert.equal(regionProtectionChanged({ ...e("DOM.documentUpdated", {}), sessionId: "other" }, "s", tracked), false);
});
test("MP-08/MP-11 native movement re-measures tracked nodes, masks moving frames and pipelined older readbacks", async () => {
  const boxes = { 2: [100, 100, 300, 200] }, calls = [];
  const guard = new NativeRegionProtection(site([{ nodeId: 2, localName: "iframe" }], boxes, calls), "s");
  await guard.refresh();const inspections = calls.filter(m => m === "DOM.getDocument").length;
  const now = () => performance.timeOrigin + performance.now(), raw = at => ({ width: 1280, height: 800, captured_ms: at });
  assert.deepEqual(await guard.regions(raw(now())), [{ x: 100, y: 100, width: 300, height: 200 }]);
  const pipelined = now(); boxes[2] = [100, 40, 300, 200];
  assert.deepEqual(await guard.regions(raw(now())), [{ x: 0, y: 0, width: 1280, height: 800 }], "moving protected frame masks the whole frame");
  assert.deepEqual(await guard.regions(raw(pipelined)), [{ x: 0, y: 0, width: 1280, height: 800 }], "a readback older than the new measurement");
  assert.deepEqual(await guard.regions(raw(now())), [{ x: 100, y: 40, width: 300, height: 200 }], "settled bounds restore masks");
  assert.equal(calls.filter(m => m === "DOM.getDocument").length, inspections, "movement never re-transfers the DOM");
});

// MP-11 (owner 2026-10-08): isolated frames are inspected through their flat
// session; only protected regions are masked, in top-viewport coordinates.
function isolated({ transformed = false, failing = false, children = [] } = {}) {
  const calls = [];
  const owner = transformed ? { width: 300, height: 200, border: [500, 350, 650, 350, 650, 450, 500, 450], content: [500, 350, 650, 350, 650, 450, 500, 450] }
    : { width: 306, height: 206, border: [500, 50, 806, 50, 806, 256, 500, 256], content: [503, 53, 803, 53, 803, 253, 503, 253] };
  return { calls, async send(method, params, session) {
    calls.push([method, session, params?.nodeId]);
    if (method === "DOM.getDocument") {
      if (session === "c" && failing) throw new Error("Target closed");
      return { root: session === "c" ? { nodeId: 1, localName: "html", children: [{ nodeId: 5, localName: "input", attributes: ["type", "password"] }] }
        : { nodeId: 1, localName: "html", children: children.length ? children : [{ nodeId: 2, localName: "iframe", frameId: "F" }] } };
    }
    if (method === "DOM.querySelectorAll") return { nodeIds: [] };
    assert.equal(method, "DOM.getBoxModel");
    if (session === "c") return { model: { border: [20, 30, 80, 30, 80, 42, 20, 42] } };
    if (params.nodeId === 2) return { model: owner };
    return { model: { width: 1280, height: 800, border: [0, 0, 1280, 0, 1280, 800, 0, 800] } };
  } };
}
test("MP-11 an isolated frame's protected field is masked at its owner offset; the rest of the frame stays visible", async () => {
  const reasons = [], frames = id => id === "F" ? "c" : null;
  assert.deepEqual(await protectedHostRegions(isolated(), "s", { frames, record: r => reasons.push(r) }), [{ x: 523, y: 83, width: 60, height: 12 }]);
  assert.deepEqual(reasons, []);
  // A transformed owner cannot map local boxes: its whole border box is masked.
  assert.deepEqual(await protectedHostRegions(isolated({ transformed: true }), "s", { frames }), [{ x: 500, y: 350, width: 150, height: 100 }]);
});
test("MP-11 an isolated frame that cannot be inspected is masked whole and the reason is recorded", async () => {
  for (const [options, reason] of [[{}, "frame_session_unavailable"], [{ frames: () => "c", failing: true }, "frame_uninspectable"]]) {
    const reasons = [];
    assert.deepEqual(await protectedHostRegions(isolated(options), "s", { ...options, record: r => reasons.push(r) }), [{ x: 500, y: 50, width: 306, height: 206 }]);
    assert.deepEqual(reasons, [reason]);
  }
});
test("MP-11 a protection marker on a frame's owner or ancestor still masks the whole frame", async () => {
  for (const children of [[{ nodeId: 3, localName: "div", attributes: ["data-chariox-secret", ""], children: [{ nodeId: 2, localName: "iframe", frameId: "F" }] }],
    [{ nodeId: 2, localName: "iframe", frameId: "F", attributes: ["data-observation-protected", ""] }]]) {
    const connection = isolated({ children });
    const regions = await protectedHostRegions(connection, "s", { frames: () => "c" });
    assert(regions.some(r => r.x === 500 && r.y === 50 && r.width === 306 && r.height === 206), JSON.stringify(regions));
    assert(!connection.calls.some(([, session]) => session === "c"), "a marked frame is never unmasked by inspection");
  }
});
test("MP-11 the native tracker retires on isolated-frame DOM, navigation and attachment events", async () => {
  const guard = new NativeRegionProtection(isolated(), "s", { frames: () => "c" });
  await guard.refresh();
  const tracker = guard.tracker(), e = (sessionId, method, params = {}) => ({ sessionId, method, params });
  assert.deepEqual([...tracker.sessions], ["s", "c"]);
  assert.equal(regionProtectionChanged(e("c", "DOM.childNodeInserted", { node: { nodeId: 9, localName: "input", attributes: ["type", "password"] } }), "s", tracker), true);
  assert.equal(regionProtectionChanged(e("c", "DOM.childNodeRemoved", { nodeId: 5 }), "s", tracker), true);
  assert.equal(regionProtectionChanged(e("s", "DOM.childNodeRemoved", { nodeId: 5 }), "s", tracker), false);
  assert.equal(regionProtectionChanged(e("c", "Page.frameNavigated"), "s", tracker), true);
  assert.equal(regionProtectionChanged(e("s", "Target.attachedToTarget", { targetInfo: { type: "iframe" } }), "s", tracker), true);
  assert.equal(regionProtectionChanged(e("s", "Target.attachedToTarget", { targetInfo: { type: "worker" } }), "s", tracker), false);
  assert.equal(regionProtectionChanged(e("x", "DOM.documentUpdated"), "s", tracker), false);
  const now = performance.timeOrigin + performance.now();
  assert.deepEqual(await guard.regions({ width: 1280, height: 800, captured_ms: now }), [{ x: 523, y: 83, width: 60, height: 12 }]);
});

// MP-11: a protected CDP capture's getDocument resets this session's node ids.
// Native measurement uses backend ids, so masks survive; a stale session id
// (fallback only) fails closed instead of reading as "not rendered".
test("MP-11 native measurement survives another inspector's getDocument; stale session ids fail closed", async () => {
  let generation = 0;
  const connection = { async send(method, params) {
    if (method === "DOM.getDocument") { generation++; return { root: { nodeId: 1, backendNodeId: 1, localName: "html", children: [{ nodeId: 10 * generation, backendNodeId: 77, localName: "input", attributes: ["type", "password"] }] } }; }
    if (method === "DOM.querySelectorAll") return { nodeIds: [] };
    assert.equal(method, "DOM.getBoxModel");
    if (params.nodeId !== undefined && params.nodeId !== 10 * generation) throw new Error("Protocol error (DOM.getBoxModel): Could not find node with given id");
    return { model: { border: [5, 6, 25, 6, 25, 16, 5, 16] } };
  } };
  const guard = new NativeRegionProtection(connection, "s");
  await guard.refresh();
  await connection.send("DOM.getDocument", {});
  const now = () => performance.timeOrigin + performance.now();
  for (let readback = 0; readback < 2; readback++)
    assert.deepEqual(await guard.regions({ width: 1280, height: 800, captured_ms: now() }), [{ x: 5, y: 6, width: 20, height: 10 }], "readback " + readback);
  // Without backend ids the stale id must not read as an unrendered field.
  const legacy = { async send(method, params) {
    if (method === "DOM.getDocument") { generation++; return { root: { nodeId: 1, localName: "html", children: [{ nodeId: 10 * generation, localName: "input", attributes: ["type", "password"] }] } }; }
    return connection.send(method, params);
  } };
  const masks = await captureRegionMasks(legacy, "s", { reinspect: false });
  await legacy.send("DOM.getDocument", {});
  assert.deepEqual(await masks.afterCapture(), [{ x: 0, y: 0, width: 1280, height: 800 }]);
  assert.equal(masks.failed, true);
});

test("MP-11 CDP capture fence uses agreeing shared measurements and fails closed otherwise", async () => {
  const { captureProtectionFence } = await import("./kernel-browser-region-protection.mjs");
  const policy = { unknown: false, values: [], targets: [] };
  const page = (regions, extra = {}) => ({ url: "https://x/", document_id: "d", dpr: 1, scale: 1, viewport: [1280, 800], regions, withheld: [], ...extra });
  // A second attached session would reset the page's DPR emulation.
  const connection = { async send(method) { throw new Error("unexpected " + method); } };
  const run = async (results, raster = { width: 1280, height: 800 }, options = {}) => {
    let n = 0; const recorded = [];
    const fence = await captureProtectionFence(connection, "page-session", "target", options.policy ?? policy, { record: r => recorded.push(r), measure: async (c, sessionId, targetId, p, o) => {
      assert.equal(targetId, "target"); assert.equal(sessionId, "page-session"); assert.deepEqual(o, { hidden: true });
      const result = results[n++]; if (result instanceof Error) throw result; return result; } });
    return { masks: await fence.afterCapture(raster), recorded };
  };
  const full = [{ x: 0, y: 0, width: 1280, height: 800 }];
  assert.deepEqual((await run([page([[10, 20, 30, 40]], { withheld: ["uninspected_frame"] }), page([[10, 20, 30, 40]], { withheld: ["uninspected_frame"] })])),
    { masks: [{ x: 10, y: 20, width: 30, height: 40 }], recorded: ["uninspected_frame"] });
  assert.deepEqual((await run([page([[10, 20, 30, 40]]), page([[11, 20, 30, 40]])])).masks, full, "layout changed during the capture");
  assert.deepEqual(await run([page([]), new Error("MP-11: frame changed during protection")]), { masks: full, recorded: ["fence_unbound frame changed during protection", "fence_unbound"] }, "failed re-measurement");
  assert.deepEqual((await run([null, null])).masks, full, "unbound page");
  assert.deepEqual((await run([page([[10.25, 20, 30, 40]]), page([[10.25, 20, 30, 40]])], { width: 2560, height: 1600 })).masks, [{ x: 20, y: 40, width: 61, height: 80 }], "uniform view-image scale maps outward");
  assert.deepEqual((await run([page([]), page([])], { width: 2560, height: 800 })).masks, [{ x: 0, y: 0, width: 2560, height: 800 }], "raster not uniformly bound to the measured viewport");
  assert.deepEqual((await run([], undefined, { policy: { ...policy, unknown: true } })).masks, full, "unknown policy");
});
