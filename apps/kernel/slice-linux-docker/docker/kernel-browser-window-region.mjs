// MP-08/MP-10/MP-11: kernel-private region producer for the shared video pump.
// A consumer supplies an admitted kernel lease and kernel-resolved window
// geometry. This is not a client API for naming windows, PIDs or policies.
import {setTimeout as delay} from 'node:timers/promises';
import {LinuxCapture} from './kernel-browser-native.mjs';
import {startNativeReadback} from './kernel-browser-native-readback.mjs';
import {attestNativeRegion,regionProtection} from './kernel-browser-region-attestation.mjs';
import {checkedInventory} from './kernel-browser-native-windows.mjs';

export function windowRegionGeometry(surface,window,rect,contentHeight,scale,contentRect){
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
export function projectRegionMasks(regions,left,top){
 return regions.map(({x,y,width,height})=>({x:x-left,y:y-top,width,height}));
}

export class NativeWindowRegionCapture extends LinuxCapture {
 constructor({surface,window,rect,contentHeight,contentRect,isCurrent,validateCurrent,attestationState,...options}){
  super(options);
  if(typeof isCurrent!=='function')throw Error('MP-11: admitted region lease required');
  this.surface=surface;this.isCurrent=isCurrent;this.validateCurrent=validateCurrent;this.attestationState=attestationState;
  this.region=windowRegionGeometry(surface,window,rect,contentHeight,this.scale,contentRect);
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
    this.regions=regionProtection(this,region.pageLeft,region.pageTop);
    await attestNativeRegion(this);
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

   if(!this.valid()||this.validateCurrent&&!await this.validateCurrent())throw Error('MP-11: native region lease retired');
   this.attested=true;return this;
  }catch(error){await this.close();throw error;}
 }
 point(x,y){return Number.isInteger(x)&&Number.isInteger(y)&&x>=0&&y>=0&&
  x<this.region.css_width&&y<this.region.css_height;}
 click(x,y,button=1){return this.point(x,y)&&(button===1||this.surface==='browser_chrome')?super.click(x,y,button):false;}
 wheel(x,y,dx,dy){return this.point(x,y)?super.wheel(x,y,dx,dy):false;}
 // Protocol 530: a primary-button drag inside the band (Tab reorder).
 drag(x,y,toX,toY){return this.surface==='browser_chrome'&&this.point(x,y)&&this.point(toX,toY)?super.drag(x,y,toX,toY):false;}
 // Protocol 530: the band's worker reads the owned display's window inventory (no extra process).
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
