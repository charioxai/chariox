import test from 'node:test';
import assert from 'node:assert/strict';
import {inspectMirrorCustomElements} from './kernel-browser-mirror-custom-elements.mjs';
function fixture(closed=false){const state={closed,released:0,admitted:[]};return {state,world:{sessionId:'s',contextId:1,connection:{async send(method,params){
 if(method==='Runtime.evaluate')return {result:{objectId:'hosts'}};
 if(method==='Runtime.getProperties')return {result:[{name:'0',value:{subtype:'node',objectId:'host'}},{name:'length',value:{value:1}}]};
 if(method==='DOM.describeNode')return {node:{nodeType:1,backendNodeId:1,shadowRoots:state.closed?[{shadowRootType:typeof state.closed==='string'?state.closed:'closed'}]:[]}};
 if(method==='Runtime.callFunctionOn'){state.admitted.push(params.arguments[0].value);return {result:{value:true}}}
 if(method==='Runtime.releaseObjectGroup'){state.released++;return {}}throw Error(method);
 }}}}}
test('custom light DOM is admitted only by native metadata; closed/unknown roots stay opaque',async()=>{
 for(const closed of [false,true,'open','unknown']){const {world,state}=fixture(closed),admission=await inspectMirrorCustomElements(world);assert.deepEqual(state.admitted,[closed!==false&&closed!=='open']);await admission.verify();await admission.release();assert.equal(state.released,1)}
});
test('attaching a closed root after light-DOM sampling invalidates the entire frame',async()=>{
 const {world,state}=fixture(),admission=await inspectMirrorCustomElements(world);state.closed=true;await assert.rejects(admission.verify(),/changed/);await admission.release();assert.equal(state.released,1);
});
test('unavailable native metadata cannot turn an unknown custom tag into public DOM',async()=>{
 const {world,state}=fixture();world.connection.send=async(method)=>{if(method==='Runtime.releaseObjectGroup'){state.released++;return {}}if(method==='Runtime.evaluate')return {result:{objectId:'hosts'}};if(method==='Runtime.getProperties')return {result:[{name:'length',value:{value:1}}]};throw Error(method)};await assert.rejects(inspectMirrorCustomElements(world),/invalid native/);assert.equal(state.released,1);assert.deepEqual(state.admitted,[]);
});
test('opaque block-in-inline flow is admitted from native geometry and rechecked at the frame fence',async()=>{
 const {world,state}=fixture(true),send=world.connection.send;let flow={display:'block',width:'1265px',height:'19px','margin-top':'16px','margin-bottom':'16px'},admitted;
 world.connection.send=async(method,params)=>{
  if(method==='DOM.describeNode')return {node:{nodeType:1,backendNodeId:1,shadowRoots:[{shadowRootType:'closed',backendNodeId:2}]}};
  if(method==='DOM.resolveNode')return {object:{objectId:'closed-root'}};
  if(method==='Runtime.callFunctionOn'&&params.objectId==='closed-root'){
   assert(!/textContent|innerHTML|nodeValue|attributes/.test(params.functionDeclaration),'geometry cannot read opaque bytes');return {result:{value:flow}};
  }
  if(method==='Runtime.callFunctionOn')admitted=params.arguments[1].value;
  return send(method,params);
 };
 const admission=await inspectMirrorCustomElements(world);assert.deepEqual(admitted,flow);await admission.verify();flow={...flow,height:'20px'};await assert.rejects(admission.verify(),/changed/);await admission.release();assert.equal(state.released,1);
});
