// MP-08/MP-10/MP-11: kernel-private region producer for the shared video pump.
// A consumer supplies an admitted kernel lease and kernel-resolved window
// geometry. This is not a client API for naming windows, PIDs or policies.
import {setTimeout as delay} from 'node:timers/promises';
import {LinuxCapture,attestationMask,statusBand} from './kernel-browser-native.mjs';
import {startNativeReadback} from './kernel-browser-native-readback.mjs';
import {NativeRegionProtection} from './kernel-browser-region-protection.mjs';
import {cropProtectedPng,decodePng,displayMaskRegions} from './kernel-browser-pixels.mjs';
import {assertCurrentDocument} from './browser-controller-actions.mjs';

export function windowRegionGeometry(surface,window,rect,contentHeight,scale){
 const {width,height}=rect,top=rect.top,left=rect.left;
 if(window.id!==undefined&&(!Number.isSafeInteger(window.id)||window.id<=1||window.id>0xffffffff))throw Error('MP-11: invalid owned X window');
 const auxiliary=surface==='browser_auxiliary';
 // Browser-owned dropdowns have no page boundary. The owner binds their
 // exact X window; only its whole inward-even raster may reach the pump.
 if(auxiliary){
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
 const contentTop=window.height-contentHeight;
 if(!auxiliary&&(surface==='browser_chrome'?top+height>contentTop:top<contentTop))
  throw Error('MP-11: native region crosses surface boundary');
 return Object.freeze({width,height,contentHeight,css_width:width/scale,css_height:height/scale,
  pageLeft:left,pageTop:top-contentTop,contentTop,
  window_region:Object.freeze({width:window.width,height:window.height,left,top,...(window.id===undefined?{}:{id:window.id})})});
}
export function projectRegionMasks(regions,left,top){
 return regions.map(({x,y,width,height})=>({x:x-left,y:y-top,width,height}));
}

export class NativeWindowRegionCapture extends LinuxCapture {
 constructor({surface,window,rect,contentHeight,isCurrent,validateCurrent,...options}){
  super(options);
  if(typeof isCurrent!=='function')throw Error('MP-11: admitted region lease required');
  this.surface=surface;this.isCurrent=isCurrent;this.validateCurrent=validateCurrent;
  this.region=windowRegionGeometry(surface,window,rect,contentHeight,this.scale);
 }
 valid(){return super.valid()&&this.isCurrent?.()===true;}
 onCdp(message){
  if(this.surface==='page'){super.onCdp(message);return;}
  // Chrome belongs to the window lease, not the navigated page. Native
  // Destroy/Configure events retire the worker's exact owned-window binding.
  if(message.method==='Chariox.browserDisconnected')this.fence();
 }
 async start(){
  if(!this.valid()||this.validateCurrent&&!await this.validateCurrent())throw Error('MP-11: native region lease retired');
  this.off=this.connection.subscribe(m=>this.onCdp(m));
  try{
   const region=this.region;
   if(this.surface==='page'){
    this.regions=new NativeRegionProtection(this.connection,this.sessionId,
     {policy:this.policy,browser:this.browser,tab:this.tab,scale:this.scale});
    const measure=this.regions.measure.bind(this.regions);
    this.regions.measure=async()=> (await measure()).map(([x,y,w,h])=>
     [x-region.pageLeft,y-region.pageTop,w,h]);
   }else{
    // Chrome UI is proved disjoint from page pixels: the band boundary or
    // a whole separate auxiliary window selected by the browser owner.
    // The kernel policy/lease still gates every sample and asynchronous commit.
    const protection={guard:null,beforeAt:-Infinity,
     async refresh(){this.guard={};this.beforeAt=performance.timeOrigin+performance.now();},
     retire(){this.guard=null;},tracker(){return {targets:false};},
     async regions(raw){if(!this.guard||raw.captured_ms<this.beforeAt)throw Error('MP-11: fill-target capture unavailable');return [];} };
    this.regions=protection;
   }
   await this.regions.refresh();
   this.poolRoot=await this.createRasterPool();
   startNativeReadback(this,{width:region.width,height:region.height,window_region:region.window_region});
   for(let n=0;n<200&&!this.latest&&!this.closed;n++)await delay(10);
   if(!this.latest||!this.valid())throw Error('MP-08: native region readback unavailable');
   if(this.surface==='page'){
    const shot=await this.screenshot();
    const clip={x:region.pageLeft/this.scale,y:region.pageTop/this.scale,
     width:region.css_width,height:region.css_height};
    const reference=decodePng(cropProtectedPng(shot.data_base64,clip,this.scale).data_base64,this.scale);
    const raw=this.latest?.raw;
    if(!raw||raw.width!==reference.width||raw.height!==reference.height)throw Error('MP-11: native region attestation geometry');
    const fullBand=statusBand({width:region.window_region.width,height:region.contentHeight},this.hostScale);
    const masks=projectRegionMasks([...(shot[displayMaskRegions]??[]),fullBand],region.pageLeft,region.pageTop);
    const masked=attestationMask(masks,raw.width,raw.height);
    if(masked&&masked.reduce((n,v)=>n+v,0)>raw.width*raw.height*.9)throw Error('MP-11: native region attestation masked');
    const pixels=raw.pixels;
    for(let i=0;i<pixels.length;i+=4)if(!masked?.[i/4]&&
     (reference.pixels[i]!==pixels[i+2]||reference.pixels[i+1]!==pixels[i+1]||reference.pixels[i+2]!==pixels[i]))
      throw Error('MP-11: native region attestation mismatch');
    await assertCurrentDocument(this.connection,this.sessionId,this.tab.target_id,this.tab.document_id);
   }
   if(!this.valid()||this.validateCurrent&&!await this.validateCurrent())throw Error('MP-11: native region lease retired');
   this.attested=true;return this;
  }catch(error){await this.close();throw error;}
 }
 point(x,y){return Number.isInteger(x)&&Number.isInteger(y)&&x>=0&&y>=0&&
  x<this.region.css_width&&y<this.region.css_height;}
 click(x,y){return this.point(x,y)?super.click(x,y):false;}
 wheel(x,y,dx,dy){return this.point(x,y)?super.wheel(x,y,dx,dy):false;}
 chord(keysym,modifiers=0){
  if(this.surface!=='browser_chrome'||!this.attested||!this.valid()||
   !Number.isSafeInteger(keysym)||!(keysym>=0x20&&keysym<=0x7e||keysym>=0xff08&&keysym<=0xffff)||
   !Number.isSafeInteger(modifiers)||modifiers<0||modifiers>15)return false;
  return this.control({key_chord:[keysym,modifiers]},true);
 }
}
