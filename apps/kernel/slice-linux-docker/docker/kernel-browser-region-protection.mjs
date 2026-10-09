// MP-08/MP-11: only live Vault-filled plain fields enter native/CDP masks.
import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
import {maskPng,cropProtectedPng,displayMaskRegions,displayFullMaskRegions} from './kernel-browser-pixels.mjs';
import {measurePageProtection,protectionDigest} from './browser-protection-regions.mjs';
const retry = () => Object.assign(Error('MP-11: fill capture must retry'), {code:'fill_capture_retry'});

export function regionProtectionChanged(message,sessionId) {
  return message.sessionId===sessionId && (message.method==='DOM.documentUpdated'||message.method.startsWith('DOM.childNode')||
    ['DOM.attributeModified','DOM.attributeRemoved'].includes(message.method)&&['type','style','class'].includes(message.params?.name));
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
export const captureRegionMasks = (connection,sessionId,options={}) => captureProtectionFence(connection,sessionId,options.targetId,options.policy??{targets:[],values:[],unknown:false},options);

export class NativeRegionProtection {
  constructor(connection,sessionId,options={}) {Object.assign(this,{connection,sessionId,options});this.revision=0;}
  tracker(){return {sessions:new Set([this.sessionId]),nodes:null};}
  retire(){this.revision++;this.guard=null;}
  get beforeAt(){return this.guard?.beforeAt??Infinity;}
  async refresh(){
    const revision=this.revision,guard=await captureRegionMasks(this.connection,this.sessionId,this.options);
    if(revision!==this.revision)throw retry();
    this.guard=guard;
  }
  async regions(raw){
    const guard=this.guard;if(!guard)throw retry();
    const regions=await guard.afterCapture(raw);if(this.guard!==guard)throw retry();return regions;
  }
}

// MP-08/MP-11: bind masks to a full viewport before any encoder or crop sees
// it. Keep this policy below clients and preserve the ordinary wire shape.
export async function captureProtectedDisplay(host,tab,clip=null,optimizeForSpeed=true){
  const scale=host.scales.get(tab.tab_id)??1;let frame;
  {
    const {protected_regions,...captured}=await host.screenshot(tab,null,true,'png',optimizeForSpeed);
    frame={...captured,[displayMaskRegions]:protected_regions,data_base64:protected_regions.length?maskPng(captured.data_base64,protected_regions.map(r=>[r.x,r.y,r.width,r.height]),scale):captured.data_base64};
  }
  const cropped=cropProtectedPng(frame.data_base64,clip,scale);
  const full=!clip||clip.width===geometry.width&&clip.height===geometry.height;
  const regions=(frame[displayMaskRegions]??[]).map(r=>({x:(r.x-(full?0:clip.x*scale))*(clip?.scale??1),y:(r.y-(full?0:clip.y*scale))*(clip?.scale??1),width:r.width*(clip?.scale??1),height:r.height*(clip?.scale??1)}));
  return {...frame,...cropped,[displayMaskRegions]:regions,[displayFullMaskRegions]:frame[displayMaskRegions]};
}
