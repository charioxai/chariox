import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
import {maskPng,opaqueFrame,cropProtectedPng,displayMaskRegions,displayFullMaskRegions} from './kernel-browser-pixels.mjs';
import {assertCurrentDocument} from './browser-controller-actions.mjs';
// MP-08/MP-11: captureProtectedPage already masks exact Vault fill targets.
export async function protectedHostRegions() {return [];}
export async function captureRegionMasks() {return {hasRegions:false,async afterCapture(){return [];}};}
// Wake native capture on geometry/type changes; no generic mask policy.
export function regionProtectionChanged(message,sessionId) {
  return message.sessionId===sessionId && (message.method==='DOM.documentUpdated'||message.method.startsWith('DOM.childNode')||
    ['DOM.attributeModified','DOM.attributeRemoved'].includes(message.method)&&['type','style','class'].includes(message.params?.name));
}

export class NativeRegionProtection {
  constructor(connection,sessionId){Object.assign(this,{connection,sessionId});this.revision=0;}
  retire(){this.revision++;this.guard=null;}
  async refresh(){
    const revision=this.revision,guard=await captureRegionMasks(this.connection,this.sessionId);
    if(revision!==this.revision)throw Error('MP-11: native region fence retired');
    this.guard=guard;this.beforeAt=performance.timeOrigin+performance.now();
  }
  async regions(raw){
    const guard=this.guard;
    if(!guard||!Number.isFinite(raw.captured_ms))throw Error('MP-11: native region fence unavailable');
    // MP-08/MP-10/MP-11: getDocument enables trusted DOM events. LinuxCapture
    // retires this snapshot on every protection/tree change, including during
    // attestation. A stable empty snapshot needs no per-frame DOM transfer.
    if(raw.captured_ms<this.beforeAt)throw Error('MP-11: native capture preceded its fence');
    const result=await guard.afterCapture(raw);
    if(this.guard!==guard)throw Error('MP-11: native region fence retired');
    if(guard.changed)await this.refresh();return result;
  }
}

// MP-08/MP-11: bind masks to a full viewport before any encoder or crop sees
// it. Keep this policy below clients and preserve the ordinary wire shape.
export async function captureProtectedDisplay(host,tab,clip=null,optimizeForSpeed=true){
  const scale=host.scales.get(tab.tab_id)??1;let frame;
  try{
    const {protected_regions,...captured}=await host.screenshot(tab,null,true,'png',optimizeForSpeed);
    frame={...captured,[displayMaskRegions]:protected_regions,data_base64:protected_regions.length?maskPng(captured.data_base64,protected_regions.map(r=>[r.x,r.y,r.width,r.height]),scale):captured.data_base64};
  }catch(error){throw error;}
  const cropped=cropProtectedPng(frame.data_base64,clip,scale);
  const full=!clip||clip.width===geometry.width&&clip.height===geometry.height;
  const regions=(frame[displayMaskRegions]??[]).map(r=>({x:(r.x-(full?0:clip.x*scale))*(clip?.scale??1),y:(r.y-(full?0:clip.y*scale))*(clip?.scale??1),width:r.width*(clip?.scale??1),height:r.height*(clip?.scale??1)}));
  return {...frame,...cropped,[displayMaskRegions]:regions,[displayFullMaskRegions]:frame[displayMaskRegions]};
}
