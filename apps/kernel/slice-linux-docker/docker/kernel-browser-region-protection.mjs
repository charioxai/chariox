// MP-08/MP-10/MP-11: only live Vault-filled plain fields from the shared
// collector's recorded fill targets enter native/CDP masks.
import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
import {maskPng,cropProtectedPng,displayMaskRegions,displayFullMaskRegions} from './kernel-browser-pixels.mjs';
import {measurePageProtection,protectionDigest} from './browser-protection-regions.mjs';
import {locateBrowserRegions} from './browser-observation-regions.mjs';
const retry = () => Object.assign(Error('MP-11: fill-target capture unavailable; retry'), {code:'fill_capture_retry'});

// MP-11: page protection can change without repainting any pixel. A native
// tracker retires on navigation and on type changes of tracked Vault fields;
// display callers pass whether their latest capture had protected regions.
export function regionProtectionChanged(message,sessionId,scope) {
  if(message.sessionId!==sessionId)return false;
  if(message.method==='DOM.documentUpdated')return true;
  const attribute=['DOM.attributeModified','DOM.attributeRemoved'].includes(message.method),name=message.params?.name;
  if(typeof scope==='object'&&scope!==null)return message.method==='Page.frameNavigated'||attribute&&name==='type'&&Boolean(scope.targets);
  return message.method.startsWith('DOM.childNode')||attribute&&(name==='type'||scope===true&&['style','class'].includes(name));
}
export const protectionDeclared = message => ['DOM.documentUpdated','DOM.childNodeInserted','DOM.attributeModified','DOM.attributeRemoved'].includes(message.method);

export async function captureProtectionFence(connection,sessionId,targetId,policy,{measure=measurePageProtection,record=()=>{}}={}) {
  const sample=async()=>{
    if(policy.unknown)throw retry();
    try {return await measure(connection,sessionId,targetId,policy,{includeHidden:true});}
    catch {record('fill_unbound');throw retry();}
  };
  const before=await sample(),beforeAt=performance.timeOrigin+performance.now();
  return {beforeAt,async afterCapture({width,height,captured_ms}) {
    const after=await sample(),sx=width/after?.viewport?.[0],sy=height/after?.viewport?.[1];
    if(!before||!after||protectionDigest(before)!==protectionDigest(after)||!(sx>0)||Math.abs(sx-sy)>1e-9||
      Number.isFinite(captured_ms)&&captured_ms<beforeAt) {record('fill_changed');throw retry();}
    return after.regions.map(([x,y,w,h])=>{
      const left=Math.floor(x*sx),top=Math.floor(y*sy);
      return {x:left,y:top,width:Math.ceil((x+w)*sx)-left,height:Math.ceil((y+h)*sy)-top};
    });
  }};
}
// Mirror tiles fence their policy's fill targets. Without a policy there is
// no fill target and no generic field, frame or shadow mask to measure.
export async function captureRegionMasks(connection,sessionId,options={}) {
  if(!options.policy)return {beforeAt:-Infinity,async afterCapture(){return [];}};
  return captureProtectionFence(connection,sessionId,options.targetId,options.policy,options);
}

// MP-11: a kernel-known protected target: a policy target or a recorded Vault fill (any tab, any frame).
export const hasProtectedTarget=(policy,browser)=>Boolean(policy?.targets?.length||browser?.fillTargets?.size);

export class NativeRegionProtection {
  constructor(connection,sessionId,options={}) {Object.assign(this,{connection,sessionId,options});this.revision=0;}
  retire() {this.revision++;this.guard=null;}
  tracker() {return {targets:hasProtectedTarget(this.options.policy,this.options.browser)};}
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
    if(!guard||!Number.isFinite(raw.captured_ms)||raw.captured_ms<this.beforeAt){this.options.record?.('fill_unbound');throw retry();}
    const after=await this.measure();
    if(this.guard!==guard)throw Error('MP-11: native region fence retired');
    if(JSON.stringify(guard.before)!==JSON.stringify(after)) {
      this.guard=null;this.options.record?.('fill_changed');throw retry();
    }
    return after.map(([x,y,width,height])=>({x,y,width,height}));
  }
}

export async function captureProtectedDisplay(host,tab,clip=null,optimizeForSpeed=true){
  const scale=host.scales.get(tab.tab_id)??host.viewport().device_scale_factor;let frame;
  {
    const {protected_regions,...captured}=await host.screenshot(tab,null,true,'png',optimizeForSpeed);
    frame={...captured,[displayMaskRegions]:protected_regions,data_base64:protected_regions.length?maskPng(captured.data_base64,protected_regions.map(r=>[r.x,r.y,r.width,r.height]),scale):captured.data_base64};
  }
  const cropped=cropProtectedPng(frame.data_base64,clip,scale);
  const full=!clip||clip.width===geometry.width&&clip.height===geometry.height;
  const regions=(frame[displayMaskRegions]??[]).map(r=>({x:(r.x-(full?0:clip.x*scale))*(clip?.scale??1),y:(r.y-(full?0:clip.y*scale))*(clip?.scale??1),width:r.width*(clip?.scale??1),height:r.height*(clip?.scale??1)}));
  return {...frame,...cropped,[displayMaskRegions]:regions,[displayFullMaskRegions]:frame[displayMaskRegions]};
}
