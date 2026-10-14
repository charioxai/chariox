// MP-08/MP-10/MP-11 M0: supplementary oracle regressions; no live acceptance claim.
import{test}from'node:test';import assert from'node:assert/strict';
import{unionArea,mirrorCoverageMetrics,oracleRecords,mirrorMetricFailures}from'./browser-mirror-oracle.mjs';
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
