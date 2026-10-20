// MP-08/MP-10/MP-11: kernel-private region producer for the shared video pump.
// A consumer supplies an admitted kernel lease and kernel-resolved window
// geometry. This is not a client API for naming windows, PIDs or policies.
import {setTimeout as delay} from 'node:timers/promises';
import {LinuxCapture,attestationMask,statusBand} from './kernel-browser-native.mjs';
import {startNativeReadback,nativeReadbackFailure} from './kernel-browser-native-readback.mjs';
import {attestNativeRegion,regionProtection} from './kernel-browser-region-attestation.mjs';
import {cropProtectedPng,decodePng,displayMaskRegions} from './kernel-browser-pixels.mjs';
import {assertCurrentDocument} from './browser-controller-actions.mjs';
import {checkedInventory} from './kernel-browser-native-windows.mjs';

export function windowRegionGeometry(surface,window,rect,contentHeight,scale,contentRect,inWindow=false){
 const {width,height}=rect,top=rect.top,left=rect.left;
 if(window.id!==undefined&&(!Number.isSafeInteger(window.id)||window.id<=1||window.id>0xffffffff))throw Error('MP-11: invalid owned X window');
 const auxiliary=surface==='browser_auxiliary';
 // Browser-owned dropdowns have no page boundary. The owner binds their
 // exact X window; only its whole inward-even raster may reach the pump.
 // Chrome UI drawn inside the tabbed window (the omnibox dropdown, a dialog's
 // page overlay) is the owner's kernel-resolved rect of that window, entirely below the band.
 if(auxiliary&&inWindow){
  if(window.id===undefined||contentHeight===undefined||top<window.height-contentHeight)throw Error('MP-11: invalid native auxiliary region');
 }else if(auxiliary){
  if(window.id===undefined||contentHeight!==undefined||left!==0||top!==0||
   width!==window.width-window.width%2||height!==window.height-window.height%2)
   throw Error('MP-11: invalid native auxiliary region');
  contentHeight=window.height;
 }
 if(!['page','browser_chrome','browser_auxiliary'].includes(surface)||![1,2].includes(scale)||
   ![window.width,window.height,contentHeight,left,top,width,height].every(Number.isSafeInteger)||
   window.width<2||window.width>2560||window.height<2||window.height>2400||
   width<2||width>2560||height<2||height>1600||width%2||height%2||
   contentHeight<2||contentHeight>window.height||left<0||top<0||
   left>window.width-width||top>window.height-height)throw Error('MP-11: invalid native window region');
 let contentTop=window.height-contentHeight,contentLeft=0;
 if(surface==='page'){
  if(!contentRect||![contentRect.left,contentRect.top,contentRect.width,contentRect.height].every(Number.isSafeInteger)||
   contentRect.left<0||contentRect.top<0||contentRect.width<2||contentRect.height!==contentHeight||
   contentRect.left+contentRect.width>window.width||contentRect.top+contentRect.height>window.height)
   throw Error('MP-11: native page origin unavailable');
  contentTop=contentRect.top;contentLeft=contentRect.left;
  if(left<contentLeft||left+width>contentLeft+contentRect.width||top+height>contentTop+contentRect.height)
   throw Error('MP-11: native region crosses page boundary');
 }
 if(!auxiliary&&(surface==='browser_chrome'?top+height>contentTop:top<contentTop))
  throw Error('MP-11: native region crosses surface boundary');
 return Object.freeze({width,height,contentHeight,css_width:width/scale,css_height:height/scale,
  pageLeft:left-contentLeft,pageTop:top-contentTop,contentTop,
  ...(contentRect?{contentRect:Object.freeze({...contentRect})}:{}),
  window_region:Object.freeze({width:window.width,height:window.height,left,top,...(window.id===undefined?{}:{id:window.id})})});
}
// MP-08/MP-11 (coordinator 12:22Z): under a browser-modal Chrome dialog the page area shows the page
// behind a uniform scrim (native = k x page, real Chrome 155: k = 0.4) plus the dialog box. Chrome's
// restored window draws a frame (4 DIP each side in real captures), so the page sits a few px inside
// the overlay: the offset (0..8 DIP across, +-2 DIP down) is found on a sparse grid, then at least four
// fifths of the unmasked reference pixels must be exactly the scrimmed page there (+-2): this Tab at
// this geometry; what differs (the dialog, a spinner) is bounded, never trusted. `masked` indexes the
// reference. Returns the offset so the page's masks follow it.
export function overlayAttestation(reference,raw,masked,scale=1){
 if(!raw||raw.width<reference.width||raw.width-reference.width>16*scale||Math.abs(raw.height-reference.height)>4*scale)throw Error('MP-11: native overlay attestation geometry');
 const n=raw.pixels,r=reference.pixels,W=reference.width,H=reference.height,NW=raw.width,NH=raw.height;
 const at=(x,y,dx,dy)=>{const nx=x+dx,ny=y+dy;return nx>=0&&ny>=0&&nx<NW&&ny<NH?(ny*NW+nx)*4:-1;};
 const inlier=(i,j,k)=>Math.abs(n[j+2]-r[i]*k)<=2&&Math.abs(n[j+1]-r[i+1]*k)<=2&&Math.abs(n[j]-r[i+2]*k)<=2;
 const scrim=(dx,dy,step)=>{const ratios=[];for(let y=0;y<H;y+=step)for(let x=0;x<W;x+=step){const i=(y*W+x)*4,j=at(x,y,dx,dy);if(j>=0&&!masked?.[i/4]&&r[i]>64)ratios.push(n[j+2]/r[i]);}ratios.sort((a,b)=>a-b);return ratios.length?ratios[ratios.length>>1]:0;};
 let best={share:-1};
 for(let dy=-2*scale;dy<=2*scale;dy++)for(let dx=0;dx<=8*scale;dx++){
  const k=scrim(dx,dy,7);let c=0,ok=0;
  for(let y=0;y<H;y+=7)for(let x=0;x<W;x+=7){const i=(y*W+x)*4,j=at(x,y,dx,dy);if(j<0||masked?.[i/4])continue;c++;if(inlier(i,j,k))ok++;}
  if(c&&ok/c>best.share)best={share:ok/c,dx,dy};
 }
 const {dx,dy}=best,k=scrim(dx,dy,5);let compared=0,inliers=0;
 for(let y=0;y<H;y++)for(let x=0;x<W;x++){const i=(y*W+x)*4,j=at(x,y,dx,dy);if(j<0||masked?.[i/4])continue;compared++;if(inlier(i,j,k))inliers++;}
 if(compared<W*H/40)throw Error('MP-11: native overlay attestation masked');
 const counts={scrim:k,inliers:inliers/compared,dx,dy};
 if(!(k>=0.25&&k<=0.75))throw Object.assign(Error('MP-11: native overlay without a scrim'),counts);
 if(inliers<compared*0.8)throw Object.assign(Error('MP-11: native overlay attestation mismatch'),counts);
 return counts;
}
export function projectRegionMasks(regions,left,top){
 return regions.map(({x,y,width,height})=>({x:x-left,y:y-top,width,height}));
}

export class NativeWindowRegionCapture extends LinuxCapture {
 constructor({surface,window,rect,contentHeight,contentRect,isCurrent,validateCurrent,attestationState,inWindow,pageOverlay,pageDocument,pageRect,occluded,...options}){
  super(options);
  if(surface==='page'&&typeof occluded==='function')this.occluded=occluded;
  if(typeof isCurrent!=='function')throw Error('MP-11: admitted region lease required');
  this.surface=surface;this.isCurrent=isCurrent;this.validateCurrent=validateCurrent;this.attestationState=attestationState;
  this.region=windowRegionGeometry(surface,window,rect,contentHeight,this.scale,contentRect,inWindow===true);
  // A browser-modal dialog's page overlay: the page area of the tabbed window (its page document fences it).
  this.pageOverlay=inWindow===true&&pageOverlay===true;this.pageDocument=pageDocument;
  if(this.pageOverlay&&typeof pageDocument!=='string')throw Error('MP-11: invalid native page overlay');
  if(this.pageOverlay&&this.region.pageTop!==0)throw Error('MP-11: invalid native page overlay');
  // An in-window popup is drawn over the page and closes without an X event: until the owner retires it,
  // its crop may show page pixels. The active Tab's masks, at the page's native origin, apply to every readback.
  this.overPage=inWindow===true&&!this.pageOverlay;
  if(this.overPage&&(!pageRect||![pageRect.left,pageRect.top].every(Number.isSafeInteger)))throw Error('MP-11: invalid native auxiliary region');
  this.pageRect=pageRect;
 }
 // Protection of a crop that is not a whole page region.
 protection(){
  if(this.pageOverlay)return regionProtection(this,0,0);
  if(this.overPage){const {left,top}=this.region.window_region;return regionProtection(this,left-this.pageRect.left,top-this.pageRect.top);}
  // The band is proved disjoint from page pixels by its boundary; a separate auxiliary window is
  // selected by the browser owner. The kernel policy/lease still gates every sample and asynchronous commit.
  return {guard:null,beforeAt:-Infinity,
   async refresh(){this.guard={};this.beforeAt=performance.timeOrigin+performance.now();},
   retire(){this.guard=null;},tracker(){return {targets:false};},
   async regions(raw){if(!this.guard||raw.captured_ms<this.beforeAt)throw Error('MP-11: fill-target capture unavailable');return [];} };
 }
 valid(){return super.valid()&&this.isCurrent?.()===true;}
 // Diagnostics for the dialog's page overlay (labels and counts only).
 fence(){if(this.pageOverlay&&!this.closed)this.timing(`native_overlay_fenced ${this.attested?'attested':'starting'}`,performance.timeOrigin+performance.now());super.fence();}
 subscribe(fn){if(!this.pageOverlay)return super.subscribe(fn);let first=true;return super.subscribe(frame=>{if(first){first=false;this.timing('native_overlay_first_frame',performance.timeOrigin+performance.now());}fn(frame);});}
 onCdp(message){
  if(this.surface==='page'||this.pageOverlay||this.overPage){super.onCdp(message);return;}
  // Chrome belongs to the window lease, not the navigated page. Native
  // Destroy/Configure events retire the worker's exact owned-window binding.
  if(message.method==='Chariox.browserDisconnected')this.fence();
 }
 async start(){
  if(!this.valid()||this.validateCurrent&&!await this.validateCurrent())throw Error('MP-11: native region lease retired');
  this.off=this.connection.subscribe(m=>this.onCdp(m));
  try{
   await this.connection.presentationFence?.recoverFailed?.();
   const region=this.region;
   if(this.surface==='page'){
    this.regions=regionProtection(this,region.pageLeft,region.pageTop);
    // Ordinary page animation must not hold a person's video patch behind pixel equality.
    // Only a product-filled target needs this extra comparison; every frame still has its Vault mask fence.
    if(this.regions.tracker().targets)await attestNativeRegion(this);
   }else this.regions=this.protection();
   await this.regions.refresh();
   this.poolRoot=await this.createRasterPool();
   startNativeReadback(this,{width:region.width,height:region.height,window_region:region.window_region});
   if(!await this.waitForCurrentSample())throw nativeReadbackFailure(this,'MP-08: native region readback unavailable');
   if(this.pageOverlay){
    await this.attestOverlay();
    if(!await this.waitForCurrentSample())throw nativeReadbackFailure(this,'MP-08: native region readback unavailable');
   }
   if(!this.valid()||this.validateCurrent&&!await this.validateCurrent())throw Error('MP-11: native region lease retired');
   this.attested=true;return this;
  }catch(error){if(this.pageOverlay)this.timing(`native_overlay_refused ${String(error?.message).replace(/[^a-z_ -]/gi,'').slice(0,80)}`,performance.timeOrigin+performance.now());await this.close();throw error;}
 }
 // The dialog's page overlay: Chrome's active Tab behind the scrim, at the offset the attestation finds;
 // the page's Vault masks are re-measured there before any frame leaves.
 async attestOverlay(){
  // The reference is the page's own viewport cropped to the native page area (the emulated viewport may
  // be larger than Chrome's view, which shows its top-left part).
  const region=this.region,shot=await this.screenshot(),page=decodePng(shot.data_base64,this.scale);
  const width=Math.min(page.width,region.width),height=Math.min(page.height,region.height);
  const reference=width<page.width||height<page.height?decodePng(cropProtectedPng(shot.data_base64,{x:0,y:0,width:width/this.scale,height:height/this.scale},this.scale).data_base64,this.scale):page;
  // The overlay raster is the native page area: its pixels are page coordinates (up to the attested offset).
  const masks=[...(shot[displayMaskRegions]??[]),statusBand({width:region.width,height:region.height},this.hostScale)];
  const masked=attestationMask(masks,reference.width,reference.height);
  if(masked&&masked.reduce((n,v)=>n+v,0)>reference.width*reference.height*.9)throw Error('MP-11: native region attestation masked');
  // The scrim fades in: a refused frame is retried on a later readback (counts only are recorded).
  for(let attempt=0;;attempt++){
   const raw=this.latest?.raw;
   try{const {scrim,inliers,dx,dy}=overlayAttestation(reference,raw,masked,this.scale);
    // Review #1016 P1: the readbacks so far were masked at the zero offset; none may be admitted.
    this.retireFrames();this.regions=regionProtection(this,-dx,-dy);await this.regions.refresh();this.wake(true);
    this.timing(`native_overlay_admitted ${scrim.toFixed(3)} ${inliers.toFixed(3)} ${dx},${dy} ${attempt}`,performance.timeOrigin+performance.now());break;}
   catch(error){this.timing(`native_overlay_attempt ${attempt} ${error.scrim?.toFixed?.(3)} ${error.inliers?.toFixed?.(3)} ${error.dx},${error.dy} ${reference.width}x${reference.height} ${raw?.width}x${raw?.height}`,performance.timeOrigin+performance.now());
    if(attempt>=3||error.scrim===undefined||!this.valid())throw error;await delay(150);}
  }
  await assertCurrentDocument(this.connection,this.sessionId,this.tab.target_id,this.pageDocument);
 }
 point(x,y){return Number.isInteger(x)&&Number.isInteger(y)&&x>=0&&y>=0&&
  x<this.region.css_width&&y<this.region.css_height;}
 click(x,y,button=1){return this.point(x,y)&&(button===1||this.surface==='browser_chrome')?super.click(x,y,button):false;}
 wheel(x,y,dx,dy){return this.point(x,y)?super.wheel(x,y,dx,dy):false;}
 // Protocol 530: a primary-button drag inside the band (Tab reorder).
 drag(x,y,toX,toY){return this.surface==='browser_chrome'&&this.point(x,y)&&this.point(toX,toY)?super.drag(x,y,toX,toY):false;}
 // Protocol 530: the band's worker reads the owned display's window inventory (no extra process).
 // L1: the worker announces inventory changes with one coalesced header; a read follows.
 onWindowsChanged(fn){(this.windowWakes??=new Set()).add(fn);return()=>this.windowWakes.delete(fn);}
 windowsChanged(){for(const fn of [...this.windowWakes??[]])fn();}
 async windows(){
  if(this.surface!=='browser_chrome'||!this.nativeWorker||!this.valid())throw Error('MP-11: native window inventory lease');
  const reply=await this.nativeWorker.request('windows',{}); // the worker's own bound owner PID
  if(!this.valid()||!Array.isArray(reply?.windows))throw Error('MP-11: native window inventory lease');
  return checkedInventory(reply.windows);
 }
 chord(keysym,modifiers=0){
  if(!['browser_chrome','browser_auxiliary'].includes(this.surface)||!this.attested||!this.valid()||
   !Number.isSafeInteger(keysym)||!(keysym>=0x20&&keysym<=0x7e||keysym>=0xff08&&keysym<=0xffff)||
   !Number.isSafeInteger(modifiers)||modifiers<0||modifiers>15)return false;
  return this.control({key_chord:[keysym,modifiers]},true);
 }
}
