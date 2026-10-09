// MP-08/MP-10/MP-11: only the shared collector's recorded Vault fill fields.
import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
import {cropProtectedPng,displayMaskRegions,displayFullMaskRegions} from './kernel-browser-pixels.mjs';
import {measurePageProtection,protectionDigest} from './browser-protection-regions.mjs';
import {locateBrowserRegions} from './browser-observation-regions.mjs';

// Compatibility exports for mirror callers; generic masks no longer exist.
export async function protectedHostRegions() { return []; }
export async function captureRegionMasks() { return {async afterCapture() { return []; }}; }
export function regionProtectionChanged(message,sessionId,scope) {
  if(message.sessionId!==sessionId)return false;
  return message.method==='DOM.documentUpdated'||message.method==='Page.frameNavigated'||
    Boolean(scope?.targets)&&['DOM.attributeModified','DOM.attributeRemoved'].includes(message.method)&&message.params?.name==='type';
}
export async function captureProtectionFence(connection,sessionId,targetId,policy,{measure=measurePageProtection}={}) {
  const before=await measure(connection,sessionId,targetId,policy,{includeHidden:true}).catch(()=>null);
  return {async afterCapture({width,height}) {
    const after=await measure(connection,sessionId,targetId,policy,{includeHidden:true}).catch(()=>null);
    const sx=width/after?.viewport?.[0],sy=height/after?.viewport?.[1];
    if(!before||!after||protectionDigest(before)!==protectionDigest(after)||!(sx>0)||Math.abs(sx-sy)>1e-9)
      throw Error('MP-11: fill-target capture unavailable; retry');
    return after.regions.map(([x,y,w,h])=>({x:Math.floor(x*sx),y:Math.floor(y*sy),width:Math.ceil((x+w)*sx)-Math.floor(x*sx),height:Math.ceil((y+h)*sy)-Math.floor(y*sy)}));
  }};
}
export class NativeRegionProtection {
  constructor(connection,sessionId,options={}) {Object.assign(this,{connection,sessionId,options});this.revision=0;}
  retire() {this.revision++;this.guard=null;}
  tracker() {return {targets:Boolean(this.options.policy?.targets?.length||this.options.browser?.fillTargets?.size)};}
  get beforeAt() {return this.fencedAt??-Infinity;}
  async measure() {
    const {policy={targets:[],values:[]},browser,tab,scale=1}=this.options;
    if(!policy.targets.length&&!browser?.fillTargets?.size)return [];
    return locateBrowserRegions(policy.targets,browser,policy.values,{contentTarget:tab.target_id,contentScale:scale});
  }
  async refresh() {
    const revision=this.revision,before=await this.measure();
    if(revision!==this.revision)throw Error('MP-11: native region fence retired');
    this.guard={before};this.fencedAt=performance.timeOrigin+performance.now();
  }
  async regions(raw) {
    const guard=this.guard;
    if(!guard||!Number.isFinite(raw.captured_ms)||raw.captured_ms<this.beforeAt)throw Error('MP-11: fill-target capture unavailable; retry');
    const after=await this.measure();
    if(this.guard!==guard)throw Error('MP-11: native region fence retired');
    if(JSON.stringify(guard.before)!==JSON.stringify(after)) {
      this.guard=null;throw Error('MP-11: fill-target capture unavailable; retry');
    }
    return after.map(([x,y,width,height])=>({x,y,width,height}));
  }
}

export async function captureProtectedDisplay(host,tab,clip=null,optimizeForSpeed=true){
  const scale=host.scales.get(tab.tab_id)??1;let frame;
  const {protected_regions,...captured}=await host.screenshot(tab,null,true,'png',optimizeForSpeed);
  frame={...captured,[displayMaskRegions]:protected_regions};
  const cropped=cropProtectedPng(frame.data_base64,clip,scale);
  const full=!clip||clip.width===geometry.width&&clip.height===geometry.height;
  const regions=(frame[displayMaskRegions]??[]).map(r=>({x:(r.x-(full?0:clip.x*scale))*(clip?.scale??1),y:(r.y-(full?0:clip.y*scale))*(clip?.scale??1),width:r.width*(clip?.scale??1),height:r.height*(clip?.scale??1)}));
  return {...frame,...cropped,[displayMaskRegions]:regions,[displayFullMaskRegions]:frame[displayMaskRegions]};
}
