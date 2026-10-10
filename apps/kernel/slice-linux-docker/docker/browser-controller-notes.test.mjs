// MD-N2 / MP-08 / MP-10 / MP-11: fake transport tests only; browser drill is separate.
import assert from 'node:assert/strict';
import test from 'node:test';
import { observeBrowserNote, NOTE_OBSERVER_EXPRESSION } from './browser-controller-notes.mjs';

function fixture({contextId=42,after='d',exception=false}={}) {
  const calls=[];let frames=0;
  const world={contextId};
  const connection={async send(method,params,session){
    calls.push({method,params,session});
    if (method==='Target.getTargets') return {targetInfos:[]};
    if (method==='Page.getFrameTree') return {frameTree:{frame:{id:'f',loaderId:++frames>1?after:'d',url:'https://fixture.test/'}}};
    if (method==='Runtime.evaluate') {
      if (params.expression===NOTE_OBSERVER_EXPRESSION) return {result:{value:true}};
      if (exception) return {exceptionDetails:{text:'page exception must not escape'}};
      return {result:{value:{quote:{exact:'selected',prefix:'before',suffix:'after'},box_css:{x:1,y:2,width:30,height:10},hint:'range'}}};
    }
  }};
  return {calls,browser:{ensureConnection:async()=>connection,ensureTargetSession:async()=>'s',ensureFocusWorld:async()=>world}};
}
test('MD-N2 / MP-11: selection executes in the #607 isolated context, never main world',async()=>{
  const {browser,calls}=fixture();
  const result=await observeBrowserNote(browser,{target_id:'t',document_id:'d'});
  assert.equal(result.selection.quote.exact,'selected');
  const evaluations=calls.filter(c=>c.method==='Runtime.evaluate');
  assert.equal(evaluations.length,2);
  assert.ok(evaluations.every(c=>c.params.contextId===42));
  assert.ok(evaluations[0].params.expression.includes("addEventListener('selectionchange'"));
  assert.ok(evaluations[0].params.expression.includes('MutationObserver'));
  assert.ok(!evaluations[0].params.expression.includes('postMessage'));
});
test('MD-N2 / MP-11: refuse missing world, stale document and exceptions without fallback',async()=>{
  for (const options of [{contextId:undefined},{contextId:NaN},{contextId:0},{after:'replaced'},{exception:true}]) {
    const f=fixture(options);
    if (Object.hasOwn(options,'contextId')) f.browser.ensureFocusWorld=async()=>({contextId:options.contextId});
    await assert.rejects(()=>observeBrowserNote(f.browser,{target_id:'t',document_id:'d'}),/MD-N2|document changed/);
    assert.ok(f.calls.filter(c=>c.method==='Runtime.evaluate').every(c=>Number.isSafeInteger(c.params.contextId)&&c.params.contextId>0));
  }
});
test('MD-N2 / MP-10: serialized quotes cannot inject evaluator code',async()=>{
  const {browser,calls}=fixture();
  const quote={exact:'"); globalThis.forbidden=true; //',prefix:'🙂',suffix:'after'};
  await observeBrowserNote(browser,{target_id:'t',document_id:'d',quote});
  assert.deepEqual(calls.filter(c=>c.method==='Runtime.evaluate').slice(1).map(c=>c.params.expression),[
    `globalThis.__charioxNotes.reanchor(${JSON.stringify(quote)})`,
    `globalThis.__charioxNotes.reanchor(${JSON.stringify(quote)},true)`,
  ]);
  await assert.rejects(()=>observeBrowserNote(browser,{target_id:'t',document_id:'d',quote:{exact:'x'.repeat(16385),prefix:'',suffix:''}}),/invalid quote/);
});

test('MD-N2 / MP-11: frame geometry refuses a missing isolated context without main-world fallback',async()=>{
  const evaluations=[];let worldCount=0;
  const tree={frame:{id:'top',loaderId:'d',url:'https://fixture.test/'},childFrames:[{frame:{id:'child',parentId:'top',loaderId:'child-d',url:'https://fixture.test/frame'}}]};
  const connection={async send(method,params){
    if (method==='Target.getTargets') return {targetInfos:[]};
    if (method==='Page.getFrameTree') return {frameTree:tree};
    if (method==='Page.createIsolatedWorld') return {executionContextId:params.frameId==='top'?41:++worldCount===1?43:undefined};
    if (method==='DOM.getFrameOwner') return {backendNodeId:1};
    if (method==='DOM.resolveNode') return {object:{objectId:'owner'}};
    if (method==='Runtime.callFunctionOn') return {result:{value:false}};
    if (method==='Runtime.releaseObject') return {};
    if (method==='DOM.getBoxModel') return {model:{content:[0,0,100,0,100,100,0,100]}};
    if (method==='Runtime.evaluate') {
      evaluations.push(params);
      if (params.expression===NOTE_OBSERVER_EXPRESSION) return {result:{value:true}};
      return {result:{value:params.contextId===42?null:{quote:{exact:'child',prefix:'',suffix:''},box_css:{x:1,y:2,width:30,height:10},hint:'range'}}};
    }
  }};
  const browser={ensureConnection:async()=>connection,ensureTargetSession:async()=>'s',ensureFocusWorld:async()=>({contextId:42})};
  await assert.rejects(()=>observeBrowserNote(browser,{target_id:'t',document_id:'d'}),/isolated frame geometry unavailable/);
  assert.ok(evaluations.every(e=>Number.isSafeInteger(e.contextId)&&e.contextId>0));
  assert.ok(!evaluations.some(e=>e.expression==='({width:innerWidth,height:innerHeight})'));
});

// MP-08 / MP-11: a frame inherits protection from its owner element and every
// ancestor of that owner, in same-process and out-of-process parents.
function protectedFrameFixture({oopif=false,protectedOwner=null,ownerFails=null,quote=null}={}) {
  const calls=[];
  const grandchild={frame:{id:'grandchild',parentId:'child',loaderId:'grandchild-d',url:'https://inner.test/'}};
  const child={frame:{id:'child',parentId:'top',loaderId:'child-d',url:'https://child.test/'},childFrames:oopif?[]:[grandchild]};
  const tree={frame:{id:'top',loaderId:'d',url:'https://fixture.test/'},childFrames:[child]};
  const sessionTree=session=>session==='child'?{...child,childFrames:[grandchild]}:tree;
  const contexts={top:42,child:43,grandchild:44};
  const connection={async send(method,params,session){
    calls.push({method,params,session});
    if (method==='Target.getTargets') return {targetInfos:oopif?[{type:'iframe',targetId:'child'}]:[]};
    if (method==='Target.attachToTarget') return {sessionId:params.targetId};
    if (method==='Target.detachFromTarget') return {};
    if (method==='Page.getFrameTree') return {frameTree:structuredClone(sessionTree(session))};
    if (method==='Page.createIsolatedWorld') return {executionContextId:contexts[params.frameId]};
    if (method==='DOM.getFrameOwner') {
      if (ownerFails===params.frameId) throw new Error('owner unavailable');
      return {backendNodeId:params.frameId==='child'?7:8};
    }
    if (method==='DOM.resolveNode') return {object:{objectId:`owner-${params.backendNodeId}`}};
    if (method==='Runtime.callFunctionOn') return {result:{value:params.objectId===`owner-${protectedOwner==='child'?7:8}`&&protectedOwner!==null}};
    if (method==='Runtime.releaseObject') return {};
    if (method==='DOM.getBoxModel') return {model:{content:[0,0,100,0,100,100,0,100]}};
    if (method==='Runtime.evaluate') {
      if (params.expression===NOTE_OBSERVER_EXPRESSION) return {result:{value:true}};
      if (params.expression==='({width:innerWidth,height:innerHeight})') return {result:{value:{width:100,height:100}}};
      if (params.contextId!==44) return {result:{value:null}};
      if (quote) return {result:{value:{anchor_state:'attached',box_css:{x:1,y:2,width:30,height:10},hint:'range'}}};
      return {result:{value:{quote:{exact:'synthetic-private-context',prefix:'',suffix:''},box_css:{x:1,y:2,width:30,height:10},hint:'range'}}};
    }
    throw new Error(`Unexpected CDP method: ${method}`);
  }};
  return {calls,read:()=>observeBrowserNote({ensureConnection:async()=>connection,ensureTargetSession:async()=>'top',ensureFocusWorld:async()=>({contextId:42})},{target_id:'t',document_id:'d',...(quote?{quote}:{})})};
}
test('MP-08 / MP-11: protected frame owners and owner ancestors hide every nested frame selection',async()=>{
  for (const oopif of [false,true]) {
    const open=await protectedFrameFixture({oopif}).read();
    assert.equal(open.selection.quote.exact,'synthetic-private-context');
    // 'child' marks the outer iframe (an ancestor of the inner frame's owner).
    for (const protectedOwner of ['child','grandchild']) {
      const f=protectedFrameFixture({oopif,protectedOwner});
      const result=await f.read();
      assert.equal(result.selection,null,`${protectedOwner} oopif=${oopif}`);
      assert.ok(!f.calls.some(c=>c.method==='Runtime.evaluate'&&c.params.contextId===44),'protected frame must not be observed');
      const anchored=await protectedFrameFixture({oopif,protectedOwner,quote:{exact:'synthetic-private-context',prefix:'',suffix:''}}).read();
      assert.equal(anchored.anchoring.anchor_state,'missing');
    }
    for (const ownerFails of ['child','grandchild']) {
      assert.equal((await protectedFrameFixture({oopif,ownerFails}).read()).selection,null,`fail closed ${ownerFails}`);
    }
  }
});
