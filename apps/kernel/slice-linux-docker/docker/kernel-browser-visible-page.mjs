// MP-08/MP-10/MP-11: whole-page fallback captures the visible owned page;
// observation never resizes Chrome or the shared desktop. The ordinary exact
// RGB attestation and per-frame Vault/document/active-Tab guards still apply.
import {LinuxCapture} from './kernel-browser-native.mjs';
import {windowRegionGeometry} from './kernel-browser-window-region.mjs';
import {cropProtectedPng,displayMaskRegions} from './kernel-browser-pixels.mjs';
export function visiblePageGeometry(window,viewport){
 if(window?.role!=='tabbed'||typeof window.isCurrent!=='function'||!window.isCurrent()||!Number.isSafeInteger(window.xid)||window.xid<=1)
  throw Error('MP-11: visible page binding retired');
 const p=window.page_rect;
 if(viewport&&(![viewport.clientWidth,viewport.clientHeight].every(n=>Number.isSafeInteger(n)&&n>0)||viewport.zoom!==1||viewport.scale!==1))throw Error('MP-11: visible page layout unavailable');
 if(!p||p.top<=0)throw Error('MP-11: visible page origin unavailable');
 const width=Math.min(p.width,viewport?viewport.clientWidth*window.scale:p.width),height=Math.min(p.height,viewport?viewport.clientHeight*window.scale:p.height);
 return windowRegionGeometry('page',{id:window.xid,width:window.bounds.width,height:window.bounds.height},
  {left:p.left,top:p.top,width:width-width%2,height:height-height%2},p.height,window.scale,p);
}
export class VisiblePageCapture extends LinuxCapture{
 constructor({window,viewport,...options}){
  if(!viewport)throw Error('MP-11: visible page layout unavailable');
  const region=visiblePageGeometry(window,viewport);
  if(options.scale!==window.scale)throw Error('MP-11: visible page density differed');
  const screenshot=options.screenshot;
  super({...options,size:{width:region.css_width,height:region.css_height},screenshot:async()=>{
   const shot=await screenshot();
   const clip={x:0,y:0,width:region.css_width,height:region.css_height};
   const masks=(shot[displayMaskRegions]??[]).map(({x,y,width,height})=>{
    const l=Math.max(0,x),t=Math.max(0,y),r=Math.min(region.width,x+width),b=Math.min(region.height,y+height);
    return {x:l,y:t,width:r-l,height:b-t};
   }).filter(r=>r.width>0&&r.height>0);
   return {...cropProtectedPng(shot.data_base64,clip,options.scale),[displayMaskRegions]:masks};
  }});
  this.region=region;this.window=window;this.validateCurrent=options.validateCurrent;this.publishCurrent=options.publishCurrent;
  // MP-08/MP-11: a navigation preview shares the region capture's live Chrome-UI fence.
  this.occluded=options.occluded??(()=>{throw Error('MP-11: visible page occlusion unavailable');});
 }
 valid(){return super.valid()&&this.window?.isCurrent()===true;}
 expectedSize(){return [this.region.width,this.region.height];}
 async prepareReadback(){
  if(!this.valid()||typeof this.validateCurrent!=='function'||!await this.validateCurrent())throw Error('MP-11: visible page binding retired');
  return {width:this.region.width,height:this.region.height,window_region:this.region.window_region};
 }
}
