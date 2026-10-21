// MP-08/MP-10/MP-11: compositor evidence cannot come from CSS or another frame.
import test from 'node:test';import assert from 'node:assert/strict';
import {mediaLayerEvidence} from './kernel-browser-media-layers.mjs';
function fixture(){
 let listener,loader='doc',nodeName='VIDEO',backend=42,layers=[{backendNodeId:42,drawsContent:true,width:40,height:20}],paintOnly=false;const calls=[];
 const connection={subscribe(fn){listener=fn;return()=>{listener=null}},async send(method,p,sessionId){calls.push({method,sessionId});
  if(method==='LayerTree.enable')listener({sessionId,method:'LayerTree.layerTreeDidChange',params:{layers:paintOnly?[]:layers}});
  if(method==='Page.captureScreenshot')throw Error('MP-08: compositor evidence must not resize the visible surface');
  if(method==='Runtime.evaluate'&&p.awaitPromise){if(paintOnly)listener({sessionId,method:'LayerTree.layerTreeDidChange',params:{layers}});return{result:{value:true}}}
  if(method==='Runtime.evaluate')return{result:{objectId:'media'}};
  if(method==='DOM.describeNode')return{node:{nodeType:1,nodeName,backendNodeId:backend}};
  return{};
 }};
 const world={connection,sessionId:'own',contextId:77,ref:'observer',frame:{id:'frame',loaderId:'doc'}};
 const stream={document_id:'doc',world,frames:new Map()},mirror={frames:{loaderId:async()=>loader}};
 const boxes=[{id:'n3',box:[20,20,40,20],media:true}];
 return{mirror,stream,boxes,calls,world,setLoader:v=>loader=v,setNode:v=>nodeName=v,setBackend:v=>backend=v,setLayers:v=>layers=v,afterPaint:()=>paintOnly=true,subscribed:()=>!!listener};
}
test('MP-08/MP-10/MP-11 static media needs a frame commit without a screenshot resize',async()=>{
 const f=fixture();f.afterPaint();assert.equal((await mediaLayerEvidence(f.mirror,f.stream,f.boxes)).length,1);
 assert.equal(f.calls.filter(c=>c.method==='Page.captureScreenshot').length,0);
});
test('MP-11 only a real current media node in its compositor tree proves exclusion',async()=>{
 const f=fixture(),[proof]=await mediaLayerEvidence(f.mirror,f.stream,f.boxes);
 assert.equal(proof.backend_node_id,42);assert.equal(proof.loader_id,'doc');assert.deepEqual(proof.box,[20,20,40,20]);assert.equal(f.subscribed(),false);
 assert.equal(f.calls.at(-1).method,'LayerTree.disable');
 for(const modify of [f=>f.setNode('DIV'),f=>f.setBackend(99),f=>f.setLoader('stale'),f=>f.setLayers([])]){
  const h=fixture();modify(h);assert.deepEqual(await mediaLayerEvidence(h.mirror,h.stream,h.boxes),[]);
 }
});
test('MP-10 mirrorregion: a composited (rotated, translucent) media element is excluded by its on-screen box',async()=>{
 const f=fixture();f.boxes[0].distorted='transform';
 const [proof]=await mediaLayerEvidence(f.mirror,f.stream,f.boxes);assert.equal(proof.backend_node_id,42);assert.deepEqual(proof.box,[20,20,40,20]);
});
test('MP-11 OOPIF exclusions need the child observation and its own session',async()=>{
 const f=fixture();f.boxes[0].id='n1000000003';
 assert.deepEqual(await mediaLayerEvidence(f.mirror,f.stream,f.boxes),[],'unattached foreign frame supplies no evidence');
 f.stream.frames.set('owner',{slot:1,loaderId:'doc',child:{...f.world,sessionId:'child'}});
 assert.equal((await mediaLayerEvidence(f.mirror,f.stream,f.boxes)).length,1);
 assert(f.calls.every(c=>c.sessionId==='child'));
 f.setLoader('next');assert.deepEqual(await mediaLayerEvidence(f.mirror,f.stream,f.boxes),[]);
});
