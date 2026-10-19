// MP-08/MP-11: unchanged-capture fence regression.
import test from 'node:test';
import assert from 'node:assert/strict';
import {ProtectionGate,measureBrowserProtection,awaitPresented} from './browser-protection-regions.mjs';

for (const result of [true,false,'failure']) {
  test(`MP-08/MP-11: presentation session settles on ${result}`, async () => {
    const calls=[];
    const connection={async send(method,params,sessionId) {
      calls.push({method,params,sessionId});
      if(method==='Target.attachToTarget')return {sessionId:'owned-session'};
      if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'frame'}}};
      if(method==='Page.createIsolatedWorld')return {executionContextId:7};
      if(method==='Runtime.evaluate') {
        assert.equal(sessionId,'owned-session');assert.equal(params.awaitPromise,true);
        assert.match(params.expression,/setTimeout\(\(\) => resolve\(false\), 500\)/);
        if(result==='failure')throw Error('MP-11 synthetic CDP failure');
        return {result:{value:result}};
      }
      return {};
    }};
    const browser={async ensureConnection(){return connection}};
    if(result===true)await awaitPresented(browser,[{target_id:'visible-page'}]);
    else await assert.rejects(awaitPresented(browser,[{target_id:'visible-page'}]),result===false?/did not present/:/synthetic CDP failure/);
    assert.deepEqual(calls[0],{method:'Target.attachToTarget',params:{targetId:'visible-page',flatten:true},sessionId:undefined});
    assert.deepEqual(calls.at(-1),{method:'Target.detachFromTarget',params:{sessionId:'owned-session'},sessionId:undefined});
  });
}
test('stream gate adopts only stable protection and releases only verified frames', async () => {
  const sequence = ['A', 'A', 'A', 'B', 'B', null, 'B'];
  let clock = 0;
  const gate = new ProtectionGate(async () => { const next = sequence.shift(); if (next === null) throw new Error('unbound'); return { pages: [next] }; });
  const steps = [];
  for (let i = 0; i < 7; i++) steps.push({ ...(await gate.step(() => ++clock)), serial: gate.protectionSerial });
  assert.deepEqual(steps.map(({ verified, changed, serial }) => [verified, changed, serial]), [
    [null, false, 0], // first sight: withheld
    [null, true, 1],  // stable across a presented frame: adopt
    [1, false, 1],    // frames captured with 1 before this step are released
    [null, true, 0],  // changed layout: drop frames, withhold
    [null, true, 2],  // new layout stable: adopt
    [null, true, 0],  // unbound measurement: withhold
    [null, false, 0], // one sighting is not enough
  ]);
  assert.equal(gate.protection, null);
});

test('MP-08/MP-10/MP-11 no filled browser field needs no renderer geometry or presentation fence',async()=>{
 const browser={fillTargets:new Map(),ensureConnection:async()=>{assert.fail('no filled field requires CDP work')}};
 const policy={unknown:false,values:['public-registration-only'],targets:[{kind:'browser',target_id:'ordinary',node_ref:'backend:1',document_id:'doc'}]};
 assert.deepEqual(await measureBrowserProtection(browser,policy),{pages:[]});
 await awaitPresented(browser,[]);
 browser.fillTargets.set('pending',{kind:'browser',target_id:'ordinary',node_ref:'backend:1',document_id:'doc',value_hash:'public-hash',pending:true});
 await assert.rejects(measureBrowserProtection(browser,policy),/no filled field requires CDP work/,'pending fills retain the capture proof');
 await assert.rejects(measureBrowserProtection(browser,{...policy,unknown:true}),/fill policy unavailable/);
});

// MP-11: generic password/search policies no longer inspect page fields.
test('MP-11 no recorded fill targets means no generic password or echo scan', async () => {
  const {measurePageProtection}=await import('./browser-protection-regions.mjs');
  const calls=[];const connection={async send(method) {
    calls.push(method);
    if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'f',loaderId:'l',url:'https://example.test/'}}};
    if(method==='Page.createIsolatedWorld')return {executionContextId:1};
    if(method==='Runtime.evaluate')return {result:{value:{visible:true,dpr:1,width:1280,height:800}}};
    if(method==='Page.getLayoutMetrics')return {cssVisualViewport:{zoom:1}};
    assert.fail('unregistered fields cannot trigger an inspection');
  }};
  assert.deepEqual((await measurePageProtection(connection,'s','t',{targets:[],values:['registered']})).regions,[]);
  assert(calls.every(m=>!m.startsWith('DOM.')));
});

// MP-11: a show/hide replacement may resize while retaining its position.
test('MP-11 resized replacement at the unique original position stays measurable', async()=>{
 const {recordBrowserFill,finishBrowserFill,measurePageProtection}=await import('./browser-protection-regions.mjs');
 const value='synthetic-resize-value';let phase='before';
 const tree={frame:{id:'f',loaderId:'l',url:'https://example.test/'}};
 const connection={async send(method,params={}) {
  if(method==='Page.getFrameTree')return {frameTree:tree};
  if(method==='Page.createIsolatedWorld')return {executionContextId:1};
  if(method==='Page.getLayoutMetrics')return {cssVisualViewport:{zoom:1}};
  if(method==='Target.getTargets')return {targetInfos:[]};
  if(method==='Runtime.evaluate')return {result:{value:{visible:true,dpr:1,width:1280,height:800}}};
  if(method==='DOMSnapshot.captureSnapshot')return {strings:['f','FORM','INPUT'],documents:[{
   frameId:0,nodes:{backendNodeId:phase==='swapped'?[1,3,4]:[1,2,3],nodeName:[1,2,2],parentIndex:[-1,0,0]},
   layout:{nodeIndex:[1,2],bounds:phase==='swapped'?[[80,30,220,40],[80,150,240,40]]:[[80,150,220,40],[80,30,220,40]]}
  }]};
  if(method==='DOM.resolveNode')return {object:{objectId:String(params.backendNodeId)}};
  if(method==='Runtime.releaseObject')return {};
  if(method==='Runtime.callFunctionOn') {
   if(!params.functionDeclaration.includes('return {exists:'))return {};
   const id=Number(params.objectId);
   return {result:{value:{exists:!(id===2&&phase==='swapped'),editable:true,unprovedPaint:false,contenteditable:false,changed:false,password:id!==4,textShadow:'none',value:phase==='before'&&id===2?'':value}}};
  }
  if(method==='DOM.getBoxModel'){
   assert.equal(params.backendNodeId,4,'the preceding password field is not the replacement');
   return {model:{border:[80,150,320,150,320,190,80,190]}};
  }
  assert.fail('unexpected CDP call '+method);
 }};
 const target=await recordBrowserFill(connection,{sessionId:'s',targetId:'t',documentId:'l',nodeRef:'backend:2',browserGeneration:1},value,1);
 phase='filled';await finishBrowserFill(connection,target);phase='swapped';
 assert.deepEqual((await measurePageProtection(connection,'s','t',{targets:[target]})).regions,[[80,150,240,40]]);
});
