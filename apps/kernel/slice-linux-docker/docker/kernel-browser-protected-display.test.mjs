// MP-08/MP-11: pixels are protected before every native/codec/crop boundary.
import test from 'node:test';
import assert from 'node:assert/strict';
import {encodePng,decodePng,maskPng,maskNativeRaster,cropProtectedPng} from './kernel-browser-pixels.mjs';
import {NativeRegionProtection,regionProtectionChanged} from './kernel-browser-region-protection.mjs';
import {CompositorSource} from './kernel-browser-compositor.mjs';

const region={x:900,y:200,width:150,height:80};
const raw=()=>({width:1280,height:800,format:'bgr0',pixels:Buffer.alloc(1280*800*4,255),shared:{path:'/never-open-unmasked',length:1280*800*4},readRegion(){throw Error('unmasked region read');}});
test('MP-11 native masking removes the shared-file bypass and preserves immutable source pixels',()=>{
 const original=raw(),masked=maskNativeRaster(original,[region]);
 assert.equal(original.pixels[(240*1280+950)*4],255);
 assert.equal(masked.pixels[(240*1280+950)*4],0);
 assert.equal(masked.pixels[0],255);
 assert.equal(masked.shared,undefined);assert.equal(masked.readRegion,undefined);
 assert.deepEqual(masked.damage,[0,0,1280,800]);
 const changed=raw();changed.pixels[(240*1280+950)*4]=37;
 assert.equal(maskNativeRaster(changed,[region]).signature,masked.signature,'private-only changes have no displayed fingerprint');
 assert.equal(maskNativeRaster(original,[]),original,'an admitted regionless raster keeps its lease');
});
test('MP-11 protected exact crops and motion thumbnails cannot recover the original field',()=>{
 const original=raw(),protectedPng=maskPng(encodePng(1280,800,original.pixels),[[900,200,150,80]]);
 const exact=cropProtectedPng(protectedPng,{x:890,y:190,width:50,height:50,scale:1});
 const decoded=decodePng(exact.data_base64);assert.equal(decoded.pixels[(30*50+30)*4],0);assert.equal(decoded.pixels[0],255);
 const thumb=cropProtectedPng(protectedPng,{x:400,y:800,width:1280,height:800,scale:1/8});
 assert.equal(thumb.width,160);assert.equal(thumb.height,100);
 assert.equal(decodePng(thumb.data_base64).pixels[(30*160+119)*4],0);
 assert.throws(()=>cropProtectedPng(protectedPng,{x:-1,y:0,width:50,height:50,scale:1}),/bounds/);
});
function connection(){
 const state={x:900,fail:false};
 return {state,send:async method=>{
  if(state.fail)throw Error('metadata unavailable');
  if(method==='DOM.getDocument')return {root:{nodeId:1}};
  if(method==='DOM.querySelectorAll')return {nodeIds:[2]};
  assert.equal(method,'DOM.getBoxModel');return {model:{border:[state.x,200,state.x+150,200,state.x+150,280,state.x,280]}};
 }};
}
test('MP-11 native region fences bind readback time, race recovery and unavailable metadata',async()=>{
 const c=connection(),fence=new NativeRegionProtection(c,'session');await fence.refresh();
 const sample={width:1280,height:800,captured_ms:fence.beforeAt+1};
 assert.deepEqual(await fence.regions(sample),[region]);
 assert.deepEqual(await fence.regions({...sample,captured_ms:fence.beforeAt-1}),[{x:0,y:0,width:1280,height:800}]);
 c.state.x=800;assert.deepEqual(await fence.regions({...sample,captured_ms:fence.beforeAt+1}),[{x:0,y:0,width:1280,height:800}]);
 assert.deepEqual(await fence.regions({...sample,captured_ms:fence.beforeAt+1}),[{...region,x:800}]);
 c.state.fail=true;await assert.rejects(fence.regions({...sample,captured_ms:fence.beforeAt+1}),/unavailable/);
});
test('MP-11 raw CDP frames wake a fresh protected capture before fingerprinting/full-video encoding',async()=>{
 const pixels=raw().pixels,unmasked=encodePng(1280,800,pixels),masked=maskPng(unmasked,[[900,200,150,80]]);let emitted;
 const source=new CompositorSource({connection:{send:async()=>({})},sessionId:'session',tab:{tab_id:'tab',document_id:'doc'},policy:{},allowed:()=>true,
  protect:async()=>({data_base64:masked}),hasher:{hash:async data=>{assert.equal(data,masked);return {width:1280,height:800,signature:'masked'};},close:async()=>{}}});
 source.attested=true;source.subscribe(sample=>emitted=sample);source.pendingImage={data:unmasked,receivedAt:0,format:'png'};
 await source.processLatest();assert.equal(emitted.data_base64,masked);await source.close();
});
test('MP-11 attribute-only protected/layout changes retire cached native and CDP observations',()=>{
 for(const name of ['type','autocomplete','data-chariox-secret','data-chariox-observation-protected','data-observation-protected'])
  assert(regionProtectionChanged({sessionId:'session',method:'DOM.attributeModified',params:{name}},'session',false));
 const style={sessionId:'session',method:'DOM.attributeModified',params:{name:'style'}};
 assert.equal(regionProtectionChanged(style,'session',false),false);
 assert.equal(regionProtectionChanged(style,'session',true),true);
 assert.equal(regionProtectionChanged(style,'foreign',true),false);
 assert(regionProtectionChanged({sessionId:'session',method:'DOM.childNodeInserted'},'session',false));
});
