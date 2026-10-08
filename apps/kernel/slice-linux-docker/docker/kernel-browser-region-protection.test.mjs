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
 const masks=await captureRegionMasks(connection,'session',{mirrorStructured:true});assert.equal((await masks.afterCapture()).length,2);
});

test('MP-11: structured same-origin frames inspect nested protected fields; foreign/missing documents remain opaque',async()=>{
 for(const kind of ['same','foreign','missing']){
  const boxed=[];const connection={async send(method,params){
   if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'main',securityOrigin:'http://fixture'},childFrames:[{frame:{id:'child',securityOrigin:kind==='foreign'?'http://foreign':'http://fixture'}}]}};
   if(method==='DOM.getDocument')return {root:{nodeId:1,children:[{nodeId:2,localName:'iframe',frameId:'child',...(kind==='missing'?{}:{contentDocument:{nodeId:3,children:[{nodeId:4,localName:'input',attributes:['type','password']}]}})}]}};
   if(method==='DOM.querySelectorAll')return {nodeIds:[2]};
   assert.equal(method,'DOM.getBoxModel');boxed.push(params.nodeId);return {model:{border:[1,2,10,2,10,20,1,20]}};
  }};
  const masks=await captureRegionMasks(connection,'session',{mirrorStructured:true});await masks.afterCapture();
  assert.deepEqual(boxed,kind==='same'?[4,4]:[2,2]);
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
  const masks=await captureRegionMasks(connection,'session',{mirrorStructured:true});assert.equal((await masks.afterCapture()).length,1);assert.deepEqual(boxed,[2,2]);
 }
});

test('MP-11: nested protected input attributes are ASCII case insensitive',async()=>{
 const boxed=[];const connection={async send(method,params){
  if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'main',securityOrigin:'http://fixture'},childFrames:[{frame:{id:'child',securityOrigin:'http://fixture'}}]}};
  if(method==='DOM.getDocument')return {root:{nodeId:1,children:[{nodeId:2,localName:'iframe',frameId:'child',contentDocument:{nodeId:3,children:[{nodeId:4,localName:'input',attributes:['type','PASSWORD']},{nodeId:5,localName:'input',attributes:['autocomplete','CC-NUMBER']}]}}]}};
  if(method==='DOM.querySelectorAll')return {nodeIds:[2]};assert.equal(method,'DOM.getBoxModel');boxed.push(params.nodeId);return {model:{border:[0,0,10,0,10,10,0,10]}};
 }};const masks=await captureRegionMasks(connection,'session',{mirrorStructured:true});assert.equal((await masks.afterCapture()).length,2);assert.deepEqual(new Set(boxed),new Set([4,5]));
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
  const tracked = new Set([7]), e = (method, params) => ({ sessionId: "s", method, params });
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
