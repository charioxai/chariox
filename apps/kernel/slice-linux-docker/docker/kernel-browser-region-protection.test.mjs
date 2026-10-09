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
  state.shadowRootType = "user-agent";
  assert.equal((await protectedHostRegions(connection, "session")).length, 1);
  for (const kind of ["open", "closed", undefined]) {
    state.shadowRootType = kind;
    assert.equal((await protectedHostRegions(connection, "session")).length, 2);
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
