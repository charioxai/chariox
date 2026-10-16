// MP-08/MP-11: pixels are protected before every native/codec/crop boundary.
import test from 'node:test';
import assert from 'node:assert/strict';
import {encodePng,decodePng,maskPng,maskNativeRaster,cropProtectedPng,displayMaskRegions} from './kernel-browser-pixels.mjs';
import {NativeRegionProtection,regionProtectionChanged,captureProtectedDisplay} from './kernel-browser-region-protection.mjs';
import {CompositorSource} from './kernel-browser-compositor.mjs';

const region={x:900,y:200,width:150,height:80};
const raw=()=>({width:1280,height:800,format:'bgr0',pixels:Buffer.alloc(1280*800*4,255),shared:{path:'/never-open-unmasked',length:1280*800*4},readRegion(){throw Error('unmasked region read');}});
test('MP-08/MP-10 admitted masked motion uses a source serial hint while exact pixels stay protected',()=>{
 const before=maskNativeRaster({...raw(),serial:1},[region]);
 const after=maskNativeRaster({...raw(),serial:2},[region],[region]);
 assert.equal(before.signature,'masked-1');assert.equal(after.signature,'masked-2');
 assert(before.pixels.equals(after.pixels),'the serial is scheduling only; it cannot establish byte inequality');
 assert.equal(after.pixels[(240*1280+950)*4],0);
 assert.equal(after.shared,undefined);
});
test('MP-08/MP-10/MP-11 contiguous small damage reuses only the immutable masked base',()=>{
 const previous=maskNativeRaster({...raw(),serial:1},[region]);
 const damage=[1100,20,1116,36];let reads=0;
 const source={width:1280,height:800,length:1280*800*4,format:'bgr0',serial:2,damage,
  copyPixels(){throw Error('full readback must not run for a contiguous small protected update')},
  readRegion(x,y,w,h){reads++;assert.deepEqual([x,y,w,h],[1100,20,16,16]);return Buffer.alloc(w*h*4,77)}};
 const next=maskNativeRaster(source,[region],[region],previous);
 assert.equal(reads,1);assert.equal(next.pixels[(20*1280+1100)*4],77);
 assert.equal(previous.pixels[(20*1280+1100)*4],255,'the exact masked base stays immutable');
 assert.equal(next.pixels[(240*1280+950)*4],0);assert.equal(next.shared,undefined);
 for(const changed of [{serial:3},{width:2560},{}]){
  if(!Object.keys(changed).length)changed.previousRegions=[];
  assert.throws(()=>maskNativeRaster({...source,...changed},[region],changed.previousRegions??[region],previous),/full readback/);
 }
});
test('MP-11 native masking removes the shared-file bypass and preserves immutable source pixels',()=>{
 const original=raw(),masked=maskNativeRaster(original,[region]);
 assert.equal(original.pixels[(240*1280+950)*4],255);
 assert.equal(masked.pixels[(240*1280+950)*4],0);
 assert.equal(masked.pixels[0],255);
 assert.equal(masked.shared,undefined);assert.equal(masked.readRegion,undefined);
 assert.deepEqual(masked[displayMaskRegions],[region]);
 assert.equal(JSON.stringify(masked).includes('kernel display protection'),false,'trusted masks never extend the public wire shape');
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
function regionFence(){
 const state={x:900,fail:false};
 const fence=new NativeRegionProtection({},'session');
 fence.measure=async()=>{
  if(state.fail)throw Error('metadata unavailable');
  return [[state.x,200,150,80]];
 };
 return {state,fence};
}
test('MP-11 native region fences bind readback time, race recovery and unavailable metadata',async()=>{
 const {state,fence}=regionFence();await fence.refresh();
 const sample={width:1280,height:800,captured_ms:fence.beforeAt+1};
 assert.deepEqual(await fence.regions(sample),[region]);
 await assert.rejects(fence.regions({...sample,captured_ms:fence.beforeAt-1}),/unavailable/);
 state.x=800;await assert.rejects(fence.regions({...sample,captured_ms:fence.beforeAt+1}),/unavailable/);
 assert.equal(fence.guard,null,'moving targets refuse release instead of masking unrelated content');
 await fence.refresh();
 assert.deepEqual(await fence.regions({...sample,captured_ms:fence.beforeAt+1}),[{...region,x:800}]);
 state.fail=true;await assert.rejects(fence.regions({...sample,captured_ms:fence.beforeAt+1}),/unavailable/);
});
test('MP-11 raw CDP frames wake a fresh protected capture before fingerprinting/full-video encoding',async()=>{
 const pixels=raw().pixels,unmasked=encodePng(1280,800,pixels),masked=maskPng(unmasked,[[900,200,150,80]]);let emitted;
 const source=new CompositorSource({connection:{send:async()=>({})},sessionId:'session',tab:{tab_id:'tab',document_id:'doc'},policy:{},allowed:()=>true,
  protect:async()=>({data_base64:masked}),hasher:{hash:async data=>{assert.equal(data,masked);return {width:1280,height:800,signature:'masked'};},close:async()=>{}}});
 source.attested=true;source.subscribe(sample=>emitted=sample);source.pendingImage={data:unmasked,receivedAt:0,format:'png'};
 await source.processLatest();assert.equal(emitted.data_base64,masked);await source.close();
});
test('MP-11 trusted mask metadata stays aligned through native crops and DPR2 thumbnails without a wire field',async()=>{
 for(const scale of [1,2]){
  const width=1280*scale,height=800*scale,png=encodePng(width,height,Buffer.alloc(width*height*4,255));
  const mask={x:900*scale,y:200*scale,width:150*scale,height:80*scale};
  // The screenshot path already masks exact Vault targets; this helper only
  // crops its protected pixels and carries private region metadata.
  const protectedPng=maskPng(png,[[mask.x,mask.y,mask.width,mask.height]],scale);
  const tab={tab_id:'tab',document_id:'doc'},host={scales:new Map([['tab',scale]]),screenshot:async()=>({tab_id:'tab',document_id:'doc',width,height,data_base64:protectedPng,protected_regions:[mask]})};
  const crop=await captureProtectedDisplay(host,tab,{x:890,y:190,width:50,height:50,scale:1});
  assert.deepEqual(crop[displayMaskRegions],[{x:10*scale,y:10*scale,width:150*scale,height:80*scale}]);
  assert.equal(decodePng(crop.data_base64).pixels[(30*scale*crop.width+30*scale)*4],0);
  const thumbnail=await captureProtectedDisplay(host,tab,{x:400,y:800,width:1280,height:800,scale:1/8});
  assert.deepEqual(thumbnail[displayMaskRegions],[{x:900*scale/8,y:200*scale/8,width:150*scale/8,height:80*scale/8}]);
  assert.equal(Object.keys(thumbnail).some(k=>k.includes('protect')||k.includes('mask')),false);
 }
});
test('MP-11 type changes on tracked Vault fields retire native observations; generic markers do not',()=>{
 const scope={targets:true};
 assert(regionProtectionChanged({sessionId:'session',method:'DOM.attributeModified',params:{name:'type'}},'session',scope));
 assert.equal(regionProtectionChanged({sessionId:'session',method:'DOM.attributeModified',params:{name:'type'}},'session',{targets:false}),false);
 for(const name of ['autocomplete','data-chariox-secret','data-chariox-observation-protected','data-observation-protected'])
  assert.equal(regionProtectionChanged({sessionId:'session',method:'DOM.attributeModified',params:{name}},'session',scope),false);
 const style={sessionId:'session',method:'DOM.attributeModified',params:{name:'style'}};
 assert.equal(regionProtectionChanged(style,'session',false),false);
 assert.equal(regionProtectionChanged(style,'session',scope),false);
 assert.equal(regionProtectionChanged(style,'foreign',true),false);
 assert(regionProtectionChanged({sessionId:'session',method:'DOM.documentUpdated'},'session',scope));
 assert(regionProtectionChanged({sessionId:'session',method:'Page.frameNavigated'},'session',scope));
});

test('MP-08/MP-10 trusted empty DOM protection uses events instead of per-frame tree transfer',async()=>{
 let calls=0;const connection={send:async method=>{calls++;if(method==='DOM.getDocument')return {root:{nodeId:1}};assert.equal(method,'DOM.querySelectorAll');return {nodeIds:[]};}};
 const guard=new NativeRegionProtection(connection,'s');await guard.refresh();const baseline=calls;
 for(let i=0;i<20;i++)assert.deepEqual(await guard.regions({width:1280,height:800,captured_ms:guard.beforeAt+1}),[]);
 assert.equal(calls,baseline,'static regionless frames cannot request/serialize full DOM trees');
 guard.retire();await assert.rejects(guard.regions({width:1280,height:800,captured_ms:Infinity}),/unavailable|retired/);
});
test('MP-11 a trusted metadata reply after protection retirement cannot reactivate admission',async()=>{
 let enter,release;const entered=new Promise(r=>enter=r),held=new Promise(r=>release=r);
 const guard=new NativeRegionProtection({},'s');
 guard.measure=async()=>{enter();await held;return [];};
 const pending=guard.refresh();await entered;guard.retire();release();await assert.rejects(pending,/retired/);
 assert.equal(guard.guard,null);
});

test('MP-08/MP-10/MP-11 stable trusted masks preserve small native damage; mask transitions repair the full viewport',()=>{
 const original={...raw(),damage:[1100,20,1180,60]},regions=[region];
 const stable=maskNativeRaster(original,regions,regions);
 assert.deepEqual(stable.damage,original.damage);
 assert.equal(stable.pixels[(240*1280+950)*4],0);
 original.pixels.fill(17);assert.equal(stable.pixels[(30*1280+1110)*4],255,'protected snapshot is immutable');
 for(const previous of [undefined,[],[{...region,x:800}], [{x:0,y:0,width:1280,height:800}]]){
  assert.deepEqual(maskNativeRaster(raw(),regions,previous).damage,[0,0,1280,800]);
 }
});
