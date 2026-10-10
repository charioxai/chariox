// MP-08/MP-10: motion stays native resolution. Reduced whole-frame software
// motion is only a fallback while the host CPU is measurably saturated.
import {readFileSync} from 'node:fs';
export class CpuContention {
 constructor({read=()=>readFileSync('/proc/stat','utf8'),now=()=>performance.now(),engageIdle=.10,engageMs=2_000,releaseIdle=.35,holdMs=10_000,releaseMs=5_000}={}){
  Object.assign(this,{read,now,engageIdle,engageMs,releaseIdle,holdMs,releaseMs});this.sample=null;this.engagedAt=null;this.busySince=null;this.clearSince=null;this.idle=null;
 }
 // Aggregate host idle fraction since the previous probe (at most every 500 ms).
 probe(){
  const at=this.now();if(this.sample&&at-this.sample.at<500)return this.idle;
  let fields;try{fields=this.read().split('\n')[0].trim().split(/\s+/).slice(1).map(Number)}catch{return this.idle}
  if(fields.length<5||!fields.every(Number.isFinite))return this.idle;
  const total=fields.reduce((a,b)=>a+b,0),idle=fields[3]+(fields[4]??0);
  if(this.sample&&total>this.sample.total)this.idle=(idle-this.sample.idle)/(total-this.sample.total);
  this.sample={at,total,idle};return this.idle;
 }
 engaged(){
  const idle=this.probe(),at=this.now();if(idle===null)return false;
  // Sustained saturation only: a link or compile burst must not drop detail.
  if(this.engagedAt===null){
   if(idle>=this.engageIdle){this.busySince=null;return false}
   this.busySince??=at;if(at-this.busySince>=this.engageMs){this.engagedAt=at;this.busySince=null;this.clearSince=null}
   return this.engagedAt!==null;
  }
  if(idle>this.releaseIdle){this.clearSince??=at;if(at-this.engagedAt>=this.holdMs&&at-this.clearSince>=this.releaseMs){this.engagedAt=null;this.clearSince=null}}
  else this.clearSince=null;
  return this.engagedAt!==null;
 }
}
export const cpuContention=new CpuContention();
