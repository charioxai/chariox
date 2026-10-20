// MP-08/MP-10/MP-11: surface isolation and lease-bound crop coordinates.
import test from 'node:test';
import assert from 'node:assert/strict';
import {windowRegionGeometry,projectRegionMasks,NativeWindowRegionCapture,overlayAttestation} from './kernel-browser-window-region.mjs';
test('MP-08/MP-10 page and native Chrome crops share native pixels at both densities',()=>{
 for(const scale of [1,2]){
  const window={width:1280*scale,height:888*scale},content=800*scale;
  const chrome=windowRegionGeometry('browser_chrome',window,{left:0,top:0,width:1280*scale,height:88*scale},content,scale);
  assert.equal(chrome.css_width,1280);assert.equal(chrome.css_height,88);
  const page=windowRegionGeometry('page',window,{left:20*scale,top:108*scale,width:320*scale,height:240*scale},content,scale,{left:0,top:88*scale,width:1280*scale,height:content});
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
 assert.throws(()=>windowRegionGeometry('page',window,rect,800,1,{left:0,top:88,width:1280,height:800}),/boundary/);
 assert.throws(()=>windowRegionGeometry('root',window,rect,800,1),/region/);
 assert.throws(()=>windowRegionGeometry('browser_chrome',{...window,height:Infinity},rect,800,1),/region/);
});
test('MP-11 region coordinates and Chrome chords require a current kernel lease',()=>{
 const config={surface:'browser_chrome',window:{width:1280,height:888},rect:{left:0,top:0,width:1280,height:88},contentHeight:800,contentRect:{left:0,top:88,width:1280,height:800},scale:1,
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
 const source=new NativeWindowRegionCapture({surface:'page',window:{id:42,width:1280,height:888},rect:{left:0,top:88,width:100,height:50},contentHeight:800,contentRect:{left:0,top:88,width:1280,height:800},scale:1,
  isCurrent:()=>true,validateCurrent:async()=>{checks++;return false;},connection:{},policy:{},allowed:()=>true});
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

// MP-08/MP-10/MP-11: native coordinates include GTK insets; CDP and masks do not.
test('MP-08/MP-10/MP-11 page crop subtracts the authoritative native origin without rounding it',()=>{
 const page={left:4,top:87,width:1272,height:709};
 const g=windowRegionGeometry('page',{id:42,width:1280,height:800},{left:14,top:107,width:100,height:50},709,1,page);
 assert.equal(g.pageLeft,10);assert.equal(g.pageTop,20);assert.equal(g.contentTop,87);
 assert.deepEqual(g.window_region,{id:42,width:1280,height:800,left:14,top:107});
});
test('MP-11 page crop refuses absent or out-of-page native origin',()=>{
 const w={id:42,width:1280,height:800},r={left:4,top:87,width:100,height:50};
 assert.throws(()=>windowRegionGeometry('page',w,r,709,1),/origin|boundary/);
 for(const page of [{left:NaN,top:87,width:1272,height:709},{left:4,top:87,width:1280,height:709},{left:4,top:87,width:1272,height:710}])
  assert.throws(()=>windowRegionGeometry('page',w,r,709,1,page),/origin|boundary/);
});

// Protocol 530: the band reads its window inventory from its own worker; the worker's bound owner PID
// is the only owner (the worker refuses any other field).
test('MP-08/MP-11 a band asks its own worker for the window inventory, nothing else does',async()=>{
 const requests=[];const row={id:42,x:0,y:0,width:2560,height:1774,visible:true,focused:true,override_redirect:false,transient_for:0,frame_insets:[0,0,0,0]};
 const source=Object.assign(Object.create(NativeWindowRegionCapture.prototype),{surface:'browser_chrome',valid:()=>true,
  nativeWorker:{request:async(op,values)=>(requests.push([op,values]),{windows:[row]})}});
 assert.deepEqual(await source.windows(),[row]);assert.deepEqual(requests,[['windows',{}]]);
 source.surface='browser_auxiliary';await assert.rejects(source.windows(),/inventory lease/);
 source.surface='browser_chrome';source.nativeWorker.request=async()=>({windows:[{...row,pid:1}]});await assert.rejects(source.windows(),/inventory binding/);
});

// Coordinator 12:22Z ask 1: the omnibox dropdown drawn inside the tabbed window streams as a kernel rect of it, below the band only.
test('MP-11 an in-window popup is a rect of the tabbed window below the band',()=>{
 const window={id:42,width:2560,height:1774},content=1600;
 const g=windowRegionGeometry('browser_auxiliary',window,{left:238,top:174,width:2156,height:176},content,2,undefined,true);
 assert.deepEqual(g.window_region,{width:2560,height:1774,left:238,top:174,id:42});
 for(const [r,c] of [[{left:238,top:172,width:2156,height:176},content],[{left:238,top:174,width:2156,height:176},undefined],[{left:500,top:174,width:2156,height:176},content]])
  assert.throws(()=>windowRegionGeometry('browser_auxiliary',window,r,c,2,undefined,true),/region|window|boundary/);
 assert.throws(()=>windowRegionGeometry('browser_auxiliary',{width:2560,height:1774},{left:238,top:174,width:2156,height:176},content,2,undefined,true),/region/);
 // DPR 1: Chrome's band is 87 rows (odd); the page overlay starts there, the even band crop at 86.
 const page=windowRegionGeometry('browser_auxiliary',{id:42,width:1280,height:800},{left:0,top:87,width:1280,height:712},713,1,undefined,true);
 assert.equal(page.pageTop,0);assert.equal(page.css_height,712);
});

// Coordinator 12:22Z ask 1: under a browser-modal dialog the page shows behind a uniform scrim (k = 0.4 in
// real Chrome 155) plus the dialog box. Only that relation proves this Tab at this geometry.
test('MP-11 the page overlay attests the scrimmed page and bounds what differs',()=>{
 const width=320,height=200,reference={width,height,pixels:Buffer.alloc(width*height*4)};
 let seed=7;for(let i=0;i<reference.pixels.length;i+=4){for(let c=0;c<3;c++){seed=(seed*1103515245+12345)>>>0;reference.pixels[i+c]=40+(seed>>>24)%200;}reference.pixels[i+3]=255;}
 // native: the overlay raster (frame columns on each side), the page at (frame, 0) behind a scrim k
 const native=(k,{box=null,shift=0,frame=0}={})=>{const W=width+2*frame,pixels=Buffer.alloc(W*height*4,60);for(let y=0;y<height;y++)for(let x=0;x<width;x++){const o=(y*W+x+frame)*4,s=(y*width+Math.min(width-1,x+shift))*4;
  const inBox=box&&x>=box[0]&&x<box[2]&&y>=box[1]&&y<box[3];for(let c=0;c<3;c++)pixels[o+2-c]=inBox?255:Math.round(reference.pixels[s+c]*k);}return {width:W,height,pixels};};
 const ok=overlayAttestation(reference,native(0.4,{box:[100,0,220,60]}),null);
 assert.ok(Math.abs(ok.scrim-0.4)<0.01);assert.ok(ok.inliers>0.85);assert.deepEqual([ok.dx,ok.dy],[0,0]);
 const framed=overlayAttestation(reference,native(0.4,{box:[100,0,220,60],frame:4}),null);
 assert.deepEqual([framed.dx,framed.dy],[4,0],'Chrome\'s window frame: the page 4 px inside, its masks follow');
 assert.throws(()=>overlayAttestation(reference,native(0.4,{shift:1}),null),/mismatch/,'another geometry');
 assert.throws(()=>overlayAttestation(reference,native(1),null),/without a scrim/,'no scrim (not a dialog)');
 assert.throws(()=>overlayAttestation(reference,native(0.4,{box:[0,0,320,90]}),null),/mismatch/,'at most a fifth may differ');
 assert.throws(()=>overlayAttestation(reference,{...native(0.4),width:318},null),/geometry/);
 const masked=new Uint8Array(width*height).fill(1);
 assert.throws(()=>overlayAttestation(reference,native(0.4),masked),/masked/,'a masked page proves nothing');
});

// r8 union live drill (1440x900@1): the dialog overlay is the binding's exact page rect inside Chrome's
// window frame (x 4, y 87, 1272 x 708 in an 800-row window); its capture must accept that rect.
test('MP-11 a dialog page overlay accepts the exact native page rect inside the window frame',()=>{
 const base={surface:'browser_auxiliary',inWindow:true,pageOverlay:true,pageDocument:'doc-a',window:{id:42,width:1280,height:800},contentHeight:713,isCurrent:()=>true,
  display:{},pid:7,connection:{},sessionId:'s',tab:{tab_id:'t',target_id:'x',document_id:'chrome-window-1-1'},scale:1,policy:{},allowed:()=>true,timing:()=>{}};
 const s=new NativeWindowRegionCapture({...base,rect:{left:4,top:87,width:1272,height:708}});
 assert.equal(s.region.css_width,1272);assert.equal(s.region.css_height,708);assert.equal(s.region.pageTop,0);
 assert.throws(()=>new NativeWindowRegionCapture({...base,rect:{left:4,top:86,width:1272,height:708}}),/region/,'never a band row');
 assert.throws(()=>new NativeWindowRegionCapture({...base,rect:{left:4,top:88,width:1272,height:708}}),/page overlay/,'starts at the page top');
 assert.throws(()=>new NativeWindowRegionCapture({...base,pageDocument:undefined,rect:{left:4,top:87,width:1272,height:708}}),/page overlay/);
});

// mirror r37 source review: the omnibox dropdown closes without an X event; until the owner retires it,
// its crop shows page pixels. The active Tab's masks (page origin from Chrome's page rect) cover them.
test('MP-11 an in-window popup over the page masks the active Tab\'s protected fields on every readback',async()=>{
 const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 const base={surface:'browser_auxiliary',inWindow:true,window:{id:42,width:2560,height:1774},contentHeight:1600,isCurrent:()=>true,
  rect:{left:238,top:174,width:2156,height:176},pageRect:{left:8,top:174,width:2544,height:1600},
  display:{},pid:7,connection:{},sessionId:'s-a',tab:{tab_id:'a',target_id:'target-a',document_id:'chrome-window-1-1-9'},scale:2,
  policy:{targets:[{target_id:'target-a'}],values:[]},browser:{},allowed:()=>true,timing:()=>{}};
 assert.throws(()=>new NativeWindowRegionCapture({...base,pageRect:undefined}),/auxiliary region/,'no page origin: refused');
 const measure=NativeRegionProtection.prototype.measure;
 NativeRegionProtection.prototype.measure=async()=>[[300,40,200,60]];// a Vault-filled field, page raster px
 try{
  const source=new NativeWindowRegionCapture(base),regions=source.protection();
  assert.equal(regions.tracker().targets,true,'attribute-only protection changes retire its frames');
  await regions.refresh();
  assert.deepEqual(await regions.regions({captured_ms:performance.timeOrigin+performance.now()+1}),
   [{x:70,y:40,width:200,height:60}],'the field at the crop offset');
  let fenced=0;source.fence=()=>{fenced++;};
  source.onCdp({sessionId:'s-a',method:'Page.frameNavigated',params:{frame:{}}});
  assert.equal(fenced,1,'the active Tab\'s navigation retires the popup frames');
 }finally{NativeRegionProtection.prototype.measure=measure;}
});

// Review #1016 @53ad3e893 P1: the overlay's first readbacks are masked at the zero offset. When the attestation finds
// the page inside Chrome's window frame (here 4 px), none of them may be admitted, nor any readback still in flight.
test('MP-11 a dialog overlay retires its zero-offset frames when the attestation shifts the masks',async()=>{
 const {NativeRegionProtection}=await import('./kernel-browser-region-protection.mjs');
 const {encodePng}=await import('./kernel-browser-pixels.mjs');
 const width=320,height=200,frame=4,W=width+2*frame,reference=Buffer.alloc(width*height*4),native=Buffer.alloc(W*height*4,60);
 let seed=11;for(let i=0;i<reference.length;i+=4){for(let c=0;c<3;c++){seed=(seed*1103515245+12345)>>>0;reference[i+c]=40+(seed>>>24)%200;}reference[i+3]=255;}
 for(let y=0;y<height;y++)for(let x=0;x<width;x++)for(let c=0;c<3;c++)native[(y*W+x+frame)*4+2-c]=Math.round(reference[(y*width+x)*4+c]*0.4);
 const measure=NativeRegionProtection.prototype.measure;
 NativeRegionProtection.prototype.measure=async()=>[[100,40,80,30]];// a Vault-filled field, page raster px
 try{
  const source=new NativeWindowRegionCapture({surface:'browser_auxiliary',inWindow:true,pageOverlay:true,pageDocument:'doc-a',
   window:{id:42,width:W,height:300},contentHeight:213,rect:{left:0,top:87,width:W,height:200},isCurrent:()=>true,
   display:{},pid:7,connection:{send:async method=>method==='Page.getFrameTree'?{frameTree:{frame:{loaderId:'doc-a'}}}:{}},sessionId:'s',
   tab:{tab_id:'t',target_id:'x',document_id:'chrome-window-1-1'},scale:1,hostScale:1,policy:{targets:[],values:[]},browser:{},allowed:()=>true,timing:()=>{}});
  source.screenshot=async()=>({data_base64:encodePng(width,height,reference)});
  source.regions=source.protection();await source.regions.refresh();
  let released=0;source.latest={raw:{width:W,height,pixels:native,serial:1,captured_ms:0,release(){released++;}},serial:1};
  const revision=source.regionRevision;
  await source.attestOverlay();
  assert.equal(source.sample(),null,'no frame masked at the zero offset is admitted');
  assert.equal(released,1,'the cached zero-offset raster is released');
  assert.ok(source.regionRevision>revision,'a readback in flight under the old projection is rejected');
  assert.deepEqual(await source.regions.regions({captured_ms:performance.timeOrigin+performance.now()+1}),[{x:104,y:40,width:80,height:30}],'the field at the attested offset');
 }finally{NativeRegionProtection.prototype.measure=measure;}
});

// MP-08/MP-10/MP-11 review #1016 P1: metadata completes before the one final
// lease query. Re-projection during that query retires pixels before any mask read.
test('MP-11 mask re-projection during the single final lease check retires held pixels before reading',async()=>{
 let release,checks=0,released=0,encoded=0,pixelReads=0;
 const source=new NativeWindowRegionCapture({surface:'page',window:{id:42,width:1280,height:888},rect:{left:0,top:88,width:100,height:50},contentHeight:800,contentRect:{left:0,top:88,width:1280,height:800},scale:1,
  isCurrent:()=>true,validateCurrent:()=>{checks++;return new Promise(resolve=>{release=resolve;})},policy:{},allowed:()=>true,connection:{},tab:{tab_id:'t',document_id:'d'},timing:()=>{}});
 source.valid=()=>!source.closed;source.attested=true;
 source.regions={guard:{},beforeAt:0,regions:async()=>[],retire(){}};
 source.pending={width:100,height:50,get pixels(){pixelReads++;return Buffer.alloc(100*50*4)},captured_ms:1,serial:1,release:()=>{released++;}};
 source.subscribe(()=>{encoded++;});
 const publishing=source.publish();
 for(let n=0;n<50&&!release;n++)await new Promise(resolve=>setImmediate(resolve));
 assert.ok(release,'publication waits in its final lease check');
 source.retireFrames();source.regions={guard:{},beforeAt:0,regions:async()=>[],retire(){}};// the attestation's new projection
 release(true);await publishing;
 assert.equal(source.latest,null,'the old projection\'s raster is not cached');
 assert.equal(encoded,0,'nor delivered');assert.equal(released,1,'and is released');assert.equal(checks,1);assert.equal(pixelReads,0,'retired readback bytes never enter the masker');
 await source.close();
});

test('MP-11 overlay startup admits only a readback of the attested projection',async()=>{
 const source=new NativeWindowRegionCapture({surface:'page',window:{id:42,width:1280,height:888},rect:{left:0,top:88,width:100,height:50},contentHeight:800,contentRect:{left:0,top:88,width:1280,height:800},scale:1,
  isCurrent:()=>true,policy:{},allowed:()=>true,connection:{},timing:()=>{}});
 source.latest={raw:{release(){}},revision:source.regionRevision};
 assert.equal(source.currentSample(),true);
 source.regionRevision++;
 assert.equal(source.currentSample(),false,'a frame of the previous projection does not satisfy startup');
});

test('MP-08/MP-10/MP-11 region pixel comparison waits for a Vault-filled target, while the frame-mask owner remains installed',async()=>{
 for(const target of ['none','policy','browser']){
  let compared=0,closed=0;
  const source=new NativeWindowRegionCapture({surface:'page',window:{id:42,width:1280,height:888},rect:{left:0,top:88,width:100,height:50},contentHeight:800,contentRect:{left:0,top:88,width:1280,height:800},scale:1,
   isCurrent:()=>true,allowed:()=>true,connection:{subscribe:()=>()=>{}},policy:{targets:target==='policy'?[{}]:[],values:[]},browser:{fillTargets:new Map(target==='browser'?[['fill',{}]]:[])},
   attestationState:async()=>{compared++;throw Error('comparison reached')}});
  source.valid=()=>true;source.close=async()=>{closed++};source.createRasterPool=async()=>{throw Error('readback reached')};
  await assert.rejects(source.start(),target==='none'?/readback reached/:/comparison reached/);
  assert.equal(compared,target==='none'?0:1);assert.equal(closed,1);assert.equal(typeof source.regions.regions,'function','every readback still uses the existing Vault mask owner');
 }
});
