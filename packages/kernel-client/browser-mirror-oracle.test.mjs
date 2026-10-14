// MP-08/MP-10/MP-11 M0: supplementary oracle regressions; no live acceptance claim.
import{test}from'node:test';import assert from'node:assert/strict';
import{unionArea,mirrorCoverageMetrics,oracleRecords,mirrorMetricFailures,captureOracleSnapshot,expectedPanelRaster}from'./browser-mirror-oracle.mjs';
const record=(key,box=[0,0,10,10],text='hello')=>({key,type:3,name:'#text',text,box,fragments:[box],color:'rgb(0, 0, 0)'});
const set=records=>({records,uncapped:true,totalNodes:records.length,width:1280,height:800,documents:1});
test('MP-10: overlapping rectangles are counted once and clipped',()=>{assert.equal(unionArea([[0,0,10,10],[5,0,10,10]]),150);assert.equal(unionArea([[-5,-5,10,10]]),25)})
test('MP-10: full coverage does not hide drift or raster failure',()=>{
 const m=mirrorCoverageMetrics(set([record('a')]),set([record('a',[1,0,10,10])]));assert.equal(m.C_text,1);assert.equal(m.C_area,1);assert.equal(m.drift.maxCssPx,1);assert.equal(mirrorMetricFailures(m).length,2)
})
test('MP-10: missing text and duplicate text use independent occurrence counts',()=>{
 const m=mirrorCoverageMetrics(set([record('a'),record('a',[20,0,10,10])]),set([record('a')]));assert.equal(m.C_text,.5);assert.equal(m.C_area,.5);assert.equal(m.text.total,2)
})
test('MP-10: records after 5000 still affect the denominator',()=>{
 const records=Array.from({length:6001},(_,i)=>record(String(i)));const m=mirrorCoverageMetrics(set(records),set(records.slice(0,6000)));assert.equal(m.text.total,6001);assert.equal(m.C_text,6000/6001)
})
test('MP-10: capped observations cannot qualify',()=>{assert.throws(()=>mirrorCoverageMetrics({...set([]),uncapped:false},set([])),/capped/)})
test('MP-10: CDP layout maps node indexes, scroll and hidden text independently',()=>{
 const snapshot={strings:['#document','HTML','#text','visible','hidden','block','visible','1','rgb(0, 0, 0)','rgba(0, 0, 0, 0)','0px'],documents:[{scrollOffsetY:100,nodes:{nodeName:[0,1,2,2],nodeType:[9,1,3,3],nodeValue:[0,0,3,4],attributes:[[],[],[],[]]},layout:{nodeIndex:[1,2,3],bounds:[[0,0,1280,1000],[0,110,10,10],[0,0,10,10]],styles:Array.from({length:3},()=>[5,6,7,8,9,10,10,10,10])},textBoxes:{layoutIndex:[1,2],bounds:[[0,110,10,10],[0,0,10,10]]}}]};
 const result=oracleRecords(snapshot);assert.equal(result.totalNodes,4);assert.equal(result.records.length,1);assert.equal(result.records[0].text,'visible');assert.deepEqual(result.records[0].box,[0,10,10,10])
})

test('MP-10: uncapped snapshot normalization uses observed CDP layout/CSS ratio',async()=>{
 const calls=[];const snapshot=await captureOracleSnapshot(async(method,params)=>{calls.push([method,params]);return method==='Page.getLayoutMetrics'?{layoutViewport:{clientWidth:2530},cssLayoutViewport:{clientWidth:1265}}:{documents:[],strings:[]}});assert.equal(snapshot.coordinateScale,2);assert.equal(calls[1][0],'DOMSnapshot.captureSnapshot');assert.equal(calls[1][1].includeDOMRects,true);assert(!Object.hasOwn(calls[1][1],'maxNodes'))
})

test('MP-10: native projection uses source pixels and the fractional panel origin',()=>{
 const pixels=new Uint8Array([0,0,0,255,100,100,100,255,200,200,200,255,255,255,255,255]),native={width:2,height:2,pixels}
 const r=expectedPanelRaster(native,{x:.25,y:.25,width:2,height:2},{x:.25,y:.25,width:2,height:2},2,2);assert.deepEqual(r.data,pixels)
 const shifted=expectedPanelRaster(native,{x:.5,y:.25,width:2,height:2},{x:.25,y:.25,width:2,height:2},2,2);assert.equal(shifted.data[0],25)
})

const frameSnapshot=(external=false)=>{
 const strings=['#document','IFRAME','#text','outside','block','visible','1','rgb(0, 0, 0)','transparent','0px','main','child','url(icon.png)','linear-gradient(red,blue)','DIV'];
 const style=[4,5,6,7,8,9,9,9,9,-1],nodes={nodeName:[0,1],nodeType:[9,1],nodeValue:[0,0],attributes:[[],[]],backendNodeId:[1,2],contentDocumentIndex:{index:external?[]:[1],value:external?[]:[1]}};
 return {strings,documents:[{frameId:10,nodes,layout:{nodeIndex:[1],bounds:[[10,10,100,100]],styles:[style]},textBoxes:{layoutIndex:[],bounds:[]}},{frameId:11,nodes:{nodeName:[0,2],nodeType:[9,3],nodeValue:[0,3],attributes:[[],[]]},layout:{nodeIndex:[1],bounds:[[0,200,20,20]],styles:[style]},textBoxes:{layoutIndex:[0],bounds:[[0,200,20,20]]}}]};
}
test('MP-10: child text outside its frame viewport is not visible native paint',()=>{assert.equal(oracleRecords(frameSnapshot()).records.some(r=>r.text==='outside'),false)})
test('MP-10: a visible out-of-process frame without evidence cannot be uncapped',()=>{const s=frameSnapshot(true);s.documents.pop();assert.throws(()=>oracleRecords(s),/frame evidence/)})
test('MP-10: transparent URL and gradient backgrounds contribute paint',()=>{for(const background of [12,13]){const s=frameSnapshot(true);s.documents.pop();s.documents[0].nodes.nodeName[1]=14;s.documents[0].layout.styles[0][9]=background;assert.equal(oracleRecords(s).records.length,1)}})
test('MP-10: remote child evidence is stitched and clipped in the owner content viewport',()=>{
 const s=frameSnapshot(true);const child={strings:s.strings,documents:[s.documents.pop()],coordinateScale:1};child.documents[0].layout.bounds[0]=[0,20,20,20];child.documents[0].textBoxes.bounds[0]=[0,20,20,20];s.externalFrames={'main:1':child};const r=oracleRecords(s);assert.equal(r.documents,2);assert.equal(r.totalNodes,4);assert.deepEqual(r.records.find(r=>r.text==='outside').box,[10,30,20,20]);
 child.documents[0].scrollOffsetY=100;assert.equal(oracleRecords(s).records.some(r=>r.text==='outside'),false)
})
