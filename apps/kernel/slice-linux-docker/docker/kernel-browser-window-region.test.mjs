// MP-08/MP-10/MP-11: surface isolation and lease-bound crop coordinates.
import test from 'node:test';
import assert from 'node:assert/strict';
import {windowRegionGeometry,projectRegionMasks,NativeWindowRegionCapture,overlayAttestation} from './kernel-browser-window-region.mjs';
test('MP-08/MP-10 page and native Chrome crops share native pixels at both densities',()=>{
 for(const scale of [1,2]){
  const window={width:1280*scale,height:888*scale},content=800*scale;
  const chrome=windowRegionGeometry('browser_chrome',window,{left:0,top:0,width:1280*scale,height:88*scale},content,scale);
  assert.equal(chrome.css_width,1280);assert.equal(chrome.css_height,88);
  const page=windowRegionGeometry('page',window,{left:20*scale,top:108*scale,width:320*scale,height:240*scale},content,scale);
  assert.equal(page.pageLeft,20*scale);assert.equal(page.pageTop,20*scale);
  assert.equal(page.css_width,320);assert.equal(page.css_height,240);
  assert.deepEqual(projectRegionMasks([{x:22*scale,y:23*scale,width:10*scale,height:4*scale}],page.pageLeft,page.pageTop),
   [{x:2*scale,y:3*scale,width:10*scale,height:4*scale}]);
 }
});
test('MP-11 crops cannot spill across Chrome/page or name a malformed raster',()=>{
 const window={width:1280,height:888},rect={left:0,top:0,width:1280,height:88};
 for(const bad of [{...rect,height:90},{...rect,left:-1},{...rect,width:1282},{...rect,height:87}])
  assert.throws(()=>windowRegionGeometry('browser_chrome',window,bad,800,1),/region/);
 assert.throws(()=>windowRegionGeometry('page',window,rect,800,1),/boundary/);
 assert.throws(()=>windowRegionGeometry('root',window,rect,800,1),/region/);
 assert.throws(()=>windowRegionGeometry('browser_chrome',{...window,height:Infinity},rect,800,1),/region/);
});
test('MP-11 region coordinates and Chrome chords require a current kernel lease',()=>{
 const config={surface:'browser_chrome',window:{width:1280,height:888},rect:{left:0,top:0,width:1280,height:88},contentHeight:800,scale:1,
  isCurrent:()=>false,policy:{},allowed:()=>true};
 assert.throws(()=>new NativeWindowRegionCapture({...config,isCurrent:null}),/lease/);
 const source=new NativeWindowRegionCapture(config);
 assert.equal(source.point(1279,87),true);assert.equal(source.point(0,88),false);
 assert.equal(source.point(NaN,0),false);assert.equal(source.click(0,88),false);
 assert.equal(source.chord(0x6c,2),false);
});

test('MP-11 only a kernel-resolved exact X window may disambiguate owned equal-size windows',()=>{
 const w={id:42,width:1280,height:888},rect={left:0,top:0,width:1280,height:88};
 assert.equal(windowRegionGeometry('browser_chrome',w,rect,800,1).window_region.id,42);
 for(const id of [0,1,-1,NaN,2**32])assert.throws(()=>windowRegionGeometry('browser_chrome',{...w,id},rect,800,1),/window/);
});
test('MP-11 async mirror geometry retirement precedes raster publication',async()=>{
 let checks=0,releases=0,encoded=0;
 const source=new NativeWindowRegionCapture({surface:'page',window:{id:42,width:1280,height:888},rect:{left:0,top:88,width:100,height:50},contentHeight:800,scale:1,
  isCurrent:()=>true,validateCurrent:async()=>{checks++;return false;},policy:{},allowed:()=>true});
 source.valid=()=>!source.closed;source.attested=true;
 source.regions={guard:{},beforeAt:0,regions:async()=>[],retire(){}};
 source.pending={width:100,height:50,pixels:Buffer.alloc(100*50*4),captured_ms:1,serial:1,release:()=>{releases++;}};
 source.subscribe(()=>{encoded++;});await source.publish();await source.close();
 assert.equal(checks,1);assert.equal(encoded,0);assert.equal(source.latest,null);assert.equal(releases,1);
});

test('MP-08/MP-10/MP-11 auxiliary producer consumes the owned whole-window binding at both densities',()=>{
 for(const scale of [1,2]){
  const window={id:43,width:641*scale,height:201*scale};
  const rect={left:0,top:0,width:window.width-window.width%2,height:window.height-window.height%2};
  const source=new NativeWindowRegionCapture({surface:'browser_auxiliary',window,rect,scale,
   isCurrent:()=>true,policy:{},allowed:()=>true});
  assert.equal(source.region.window_region.id,43);
  assert.equal(source.region.contentTop,0);
  assert.equal(source.region.css_width,rect.width/scale);
  assert.equal(source.region.css_height,rect.height/scale);
  assert.equal(source.point(rect.width/scale-1,rect.height/scale-1),true);
  assert.equal(source.point(rect.width/scale,0),false);
  // Browser shortcuts still belong to the band arbiter.
  source.attested=true;assert.equal(source.chord(0x6c,2),false);
 }
});
test('MP-11 auxiliary regions require an exact window and cannot crop arbitrary page bounds',()=>{
 const window={id:43,width:640,height:200},rect={left:0,top:0,width:640,height:200};
 for(const [w,r,content] of [
  [{width:640,height:200},rect,undefined],
  [window,{...rect,left:2,width:638},undefined],
  [window,{...rect,top:2,height:198},undefined],
  [window,{...rect,height:198},undefined],
  [window,rect,200],
 ])assert.throws(()=>windowRegionGeometry('browser_auxiliary',w,r,content,1),/region|window/);
});
// Protocol 530: the band reads its window inventory from its own worker; the worker's bound owner PID
// is the only owner (the worker refuses any other field).
test('MP-08/MP-11 a band asks its own worker for the window inventory, nothing else does',async()=>{
 const requests=[];const row={id:42,x:0,y:0,width:2560,height:1774,visible:true,focused:true,override_redirect:false,transient_for:0};
 const source=Object.assign(Object.create(NativeWindowRegionCapture.prototype),{surface:'browser_chrome',valid:()=>true,
  nativeWorker:{request:async(op,values)=>(requests.push([op,values]),{windows:[row]})}});
 assert.deepEqual(await source.windows(),[row]);assert.deepEqual(requests,[['windows',{}]]);
 source.surface='browser_auxiliary';await assert.rejects(source.windows(),/inventory lease/);
 source.surface='browser_chrome';source.nativeWorker.request=async()=>({windows:[{...row,pid:1}]});await assert.rejects(source.windows(),/inventory binding/);
});

// Coordinator 12:22Z ask 1: the omnibox dropdown drawn inside the tabbed window streams as a kernel rect of it, below the band only.
test('MP-11 an in-window popup is a rect of the tabbed window below the band',()=>{
 const window={id:42,width:2560,height:1774},content=1600;
 const g=windowRegionGeometry('browser_auxiliary',window,{left:238,top:174,width:2156,height:176},content,2,true);
 assert.deepEqual(g.window_region,{width:2560,height:1774,left:238,top:174,id:42});
 for(const [r,c] of [[{left:238,top:172,width:2156,height:176},content],[{left:238,top:174,width:2156,height:176},undefined],[{left:500,top:174,width:2156,height:176},content]])
  assert.throws(()=>windowRegionGeometry('browser_auxiliary',window,r,c,2,true),/region|window|boundary/);
 assert.throws(()=>windowRegionGeometry('browser_auxiliary',{width:2560,height:1774},{left:238,top:174,width:2156,height:176},content,2,true),/region/);
});

// Coordinator 12:22Z ask 1: under a browser-modal dialog the page shows behind a uniform scrim (k = 0.4 in
// real Chrome 155) plus the dialog box. Only that relation proves this Tab at this geometry.
test('MP-11 the page overlay attests the scrimmed page and bounds what differs',()=>{
 const width=320,height=200,reference={width,height,pixels:Buffer.alloc(width*height*4)};
 let seed=7;for(let i=0;i<reference.pixels.length;i+=4){for(let c=0;c<3;c++){seed=(seed*1103515245+12345)>>>0;reference.pixels[i+c]=40+(seed>>>24)%200;}reference.pixels[i+3]=255;}
 const native=(k,{box=null,shift=0}={})=>{const pixels=Buffer.alloc(width*height*4);for(let y=0;y<height;y++)for(let x=0;x<width;x++){const o=(y*width+x)*4,s=(y*width+Math.min(width-1,x+shift))*4;
  const inBox=box&&x>=box[0]&&x<box[2]&&y>=box[1]&&y<box[3];for(let c=0;c<3;c++)pixels[o+2-c]=inBox?255:Math.round(reference.pixels[s+c]*k);}return {width,height,pixels};};
 const ok=overlayAttestation(reference,native(0.4,{box:[100,0,220,60]}),null);
 assert.ok(Math.abs(ok.scrim-0.4)<0.01);assert.ok(ok.inliers>0.85);
 assert.throws(()=>overlayAttestation(reference,native(0.4,{shift:1}),null),/mismatch/,'another geometry');
 assert.throws(()=>overlayAttestation(reference,native(1),null),/without a scrim/,'no scrim (not a dialog)');
 assert.throws(()=>overlayAttestation(reference,native(0.4,{box:[0,0,320,90]}),null),/mismatch/,'at most a fifth may differ');
 assert.throws(()=>overlayAttestation(reference,{...native(0.4),width:318},null),/geometry/);
 const masked=new Uint8Array(width*height).fill(1);
 assert.throws(()=>overlayAttestation(reference,native(0.4),masked),/masked/,'a masked page proves nothing');
});
