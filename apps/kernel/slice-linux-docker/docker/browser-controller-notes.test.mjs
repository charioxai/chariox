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
      if (exception) return {exceptionDetails:exception===true?{text:'page exception must not escape'}:exception};
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
function protectedFrameFixture({oopif=false,protectedOwner=null,ownerFails=null,quote=null,retireOnRead=false,protectOnRead=false}={}) {
  const calls=[];let captured=false;
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
    if (method==='Page.getFrameTree') {const current=structuredClone(sessionTree(session));if(retireOnRead&&captured){const visit=t=>{if(t.frame.id==='grandchild')t.frame.loaderId='replacement';for(const child of t.childFrames??[])visit(child)};visit(current)}return {frameTree:current}};
    if (method==='Page.createIsolatedWorld') return {executionContextId:contexts[params.frameId]};
    if (method==='DOM.getFrameOwner') {
      if (ownerFails===params.frameId) throw new Error('owner unavailable');
      return {backendNodeId:params.frameId==='child'?7:8};
    }
    if (method==='DOM.resolveNode') return {object:{objectId:`owner-${params.backendNodeId}`}};
    if (method==='Runtime.callFunctionOn'&&protectOnRead&&captured)return{result:{value:true}};
    if (method==='Runtime.callFunctionOn') return {result:{value:params.objectId===`owner-${protectedOwner==='child'?7:8}`&&protectedOwner!==null}};
    if (method==='Runtime.releaseObject') return {};
    if (method==='DOM.getBoxModel') return {model:{content:[0,0,100,0,100,100,0,100]}};
    if (method==='Runtime.evaluate') {
      if (params.expression===NOTE_OBSERVER_EXPRESSION) return {result:{value:true}};
      if (params.expression==='({width:innerWidth,height:innerHeight})') return {result:{value:{width:100,height:100}}};
      if (params.contextId!==44) return {result:{value:null}};
      captured=true;
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
    }
    for (const ownerFails of ['child','grandchild']) {
      assert.equal((await protectedFrameFixture({oopif,ownerFails}).read()).selection,null,`fail closed ${ownerFails}`);
    }
  }
});

test('MP-08/MP-10/MP-11 unrelated iframe churn does not invalidate a main-document selection',async()=>{
 let reads=0;
 const root={id:'top',loaderId:'doc',url:'https://www.bbc.com/news'};
 const connection={async send(method,params){
  if(method==='Page.getFrameTree')return {frameTree:{frame:root,...(++reads>=3?{childFrames:[{frame:{id:'late-ad',parentId:'top',loaderId:'ad-doc'}}]}:{})}};
  if(method==='Target.getTargets')return{targetInfos:[]};
  if(method==='Runtime.evaluate')return{result:{value:params.expression===NOTE_OBSERVER_EXPRESSION?true:{quote:{exact:'BBC News',prefix:'',suffix:''},box_css:{x:1,y:2,width:30,height:10},hint:'range'}}};
  throw Error('unexpected '+method);
 }};
 const browser={ensureConnection:async()=>connection,ensureTargetSession:async()=>'root',ensureFocusWorld:async()=>({contextId:42})};
 const result=await observeBrowserNote(browser,{target_id:'tab',document_id:'doc'});
 assert.equal(result.selection.quote.exact,'BBC News');
});

test('MP-08/MP-11 selected frame replacement still refuses same-process and isolated captures',async()=>{
 for(const oopif of [false,true])await assert.rejects(protectedFrameFixture({oopif,retireOnRead:true}).read(),/MD-N2: frame changed during capture/);
});

test('MP-08/MP-11 a newly protected frame owner refuses the captured selection',async()=>{
 for(const oopif of [false,true])await assert.rejects(protectedFrameFixture({oopif,protectOnRead:true}).read(),/MD-N2: frame changed during capture/);
});

test('MP-08/MP-11 a selected document URL change still retires its captured anchor',async()=>{
 let reads=0;
 const connection={async send(method,params){
  if(method==='Page.getFrameTree')return{frameTree:{frame:{id:'top',loaderId:'doc',url:++reads>=3?'https://fixture.test/new-path':'https://fixture.test/'}}};
  if(method==='Target.getTargets')return{targetInfos:[]};
  if(method==='Runtime.evaluate')return{result:{value:params.expression===NOTE_OBSERVER_EXPRESSION?true:{quote:{exact:'selected',prefix:'',suffix:''},box_css:{x:1,y:2,width:30,height:10},hint:'range'}}};
  throw Error('unexpected '+method);
 }};
 const browser={ensureConnection:async()=>connection,ensureTargetSession:async()=>'root',ensureFocusWorld:async()=>({contextId:42})};
 await assert.rejects(observeBrowserNote(browser,{target_id:'tab',document_id:'doc'}),/MD-N2: frame changed during capture/);
});

// MP-08/MP-10/MP-11: preserve public observer causes, never exception/page data.
test('MP-08/MP-10/MP-11 Notes observation retains a closed exception class and observer source line',async()=>{
 const f=fixture({exception:{exception:{className:'TypeError',description:'TypeError: private-canary https://private.invalid/'},stackTrace:{callFrames:[{functionName:'capture',lineNumber:41,columnNumber:123,url:'private-canary'}]}}});
 await assert.rejects(()=>observeBrowserNote(f.browser,{target_id:'t',document_id:'d'}),error=>{
  assert.equal(error.code,'note_observer_type_error');assert.equal(error.noteObserverLine,42);assert(!error.message.includes('private-canary'));return true;
 });
});
test('MP-11 Notes observation rejects arbitrary exception labels and source positions',async()=>{
 const f=fixture({exception:{exception:{className:'private-canary',description:'private-canary'},stackTrace:{callFrames:[{functionName:'private-canary',lineNumber:41,url:'private-canary'}]}}});
 await assert.rejects(()=>observeBrowserNote(f.browser,{target_id:'t',document_id:'d'}),error=>{assert.equal(error.code,'note_observer_exception');assert.equal(error.noteObserverLine,undefined);assert(!error.message.includes('private-canary'));return true});
});
