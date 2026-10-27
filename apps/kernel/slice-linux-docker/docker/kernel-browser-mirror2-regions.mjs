// MP-08/MP-10/MP-11: §2a kernel-private opaque-region leases.
// Public subscribe/input wiring belongs to 519. No client crop or OS handle.
import { randomUUID } from 'node:crypto';
import { assertNotCancelled } from './browser-controller-actions.mjs';
import {mediaLayerEvidence} from './kernel-browser-media-layers.mjs';

const refuse = reason => { const error = new Error(`MP-11: ${reason}`); error.mirrorReason = reason; throw error; };
const fields = new Set(['mirror_subscription_id', 'generation', 'document_id', 'sequence', 'region_id']);
const id = value => typeof value === 'string' && /^n[1-9][0-9]{0,15}$/.test(value);
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
function clipped(box, {width,height}) {
  if (!Array.isArray(box) || box.length !== 4 || !box.every(Number.isFinite)) refuse('region_geometry');
  const [x,y,w,h] = box, left = Math.max(0,x), top = Math.max(0,y), right = Math.min(width,x+w), bottom = Math.min(height,y+h);
  if (w <= 0 || h <= 0 || right <= left || bottom <= top) refuse('region_geometry');
  return {x:left,y:top,width:right-left,height:bottom-top};
}
export class Mirror2Regions {
  constructor(mirror) { this.mirror = mirror; this.host = mirror.host; this.leases = new Map(); }
  // Only unencoded records actually issued to this viewer can become leases.
  record(stream, packet) {
    if (packet.reset || !stream.regionRecords) stream.regionRecords = new Map();
    const records = stream.regionRecords;
    const drop = key => { const r=records.get(key); if(!r)return; records.delete(key); for(const child of r.children) drop(child); };
    const add = list => {
      for (const r of list ?? []) records.set(r.id, {parent:r.parent,kind:r.kind,foreign:r.foreign === true,children:new Set()});
      for (const r of list ?? []) records.get(r.parent)?.children.add(r.id);
    };
    add(packet.nodes);
    // Reparent moved nodes before removing an old parent's departed subtree.
    for (const op of packet.ops ?? []) if (op.op === 'children') for (const key of op.children ?? []) {
      const r=records.get(key); if(r){records.get(r.parent)?.children.delete(key);r.parent=op.id;}
    }
    for (const op of packet.ops ?? []) if (op.op === 'children') {
      const parent=records.get(op.id), retained=new Set(op.children ?? op.nodes.filter(r=>r.parent===op.id).map(r=>r.id));
      for (const key of parent?.children ?? []) if(!retained.has(key)) drop(key);
      add(op.nodes); if(parent)parent.children=retained;
    }
  }
  locked(stream, operation) {
    this.mirror.hurry(stream);
    const run = stream.chain.then(operation); stream.chain = run.catch(() => {}); return run;
  }
  check(target, scope, expected) {
    const {mirror_subscription_id:sid,generation,document_id,sequence,region_id} = target;
    const stream = this.mirror.service.require(sid,scope,generation), tab = this.host.tabs.get(stream.tab_id);
    if (expected && stream !== expected || stream.wire !== 2 || !tab || stream.document_id !== document_id || tab.document_id !== document_id ||
      stream.policy !== this.host.protection || stream.fallback || !Number.isSafeInteger(sequence) || sequence < stream.resetAt || sequence > stream.issued) refuse('region_epoch');
    const record = stream.regionRecords?.get(region_id);
    if (!record || record.kind !== 'tile' && !(record.kind === 'frame' && record.foreign)) refuse('region_target');
    const identities = [];
    for (let at=region_id;at;at=stream.regionRecords.get(at)?.parent) {
      if (identities.length >= 4096) refuse('region_target');
      const introduced = stream.introduced?.get(at), identity = stream.identity?.get(at) ?? 0;
      if (!introduced || introduced > sequence || identity > sequence) refuse('region_identity');
      identities.push([at,introduced,identity]);
    }
    return {stream,tab,identities};
  }
  async measure(stream, tab, target, scope, signal) {
    assertNotCancelled(signal);
    const boxes = await this.mirror.regionBoxes(stream,tab,signal);
    this.check(target,scope,stream); assertNotCancelled(signal);
    const box = boxes.find(b => b.id === target.region_id);
    if (!box) refuse('region_target');
    // A change of compositing (a fade, a rotation starting or ending) retires the lease: the viewer places a patch
    // over a composited region differently from one inside an untransformed element.
    const viewport = this.host.pageSize(tab.tab_id);
    return {box:box.box,rect:clipped(box.clip ?? box.box,viewport),viewport,composited:Boolean(box.distorted)};
  }
  async resolve(target, scope, {signal} = {}) {
    if (!target || Object.keys(target).some(k => !fields.has(k)) || !id(target.region_id) || typeof target.document_id !== 'string' ||
      typeof target.mirror_subscription_id !== 'string' || !Number.isSafeInteger(target.generation)) refuse('region_request');
    if(this.leases.size >= 32) refuse('region_limit');
    const request=Object.freeze({...target}), stream=this.check(request,scope).stream;
    return this.locked(stream,async()=>{
      const {tab,identities}=this.check(request,scope,stream), resetAt=stream.resetAt, policy=this.host.protection;
      const measured=await this.measure(stream,tab,request,scope,signal);
      if (!same(identities,this.check(request,scope,stream).identities)) refuse('region_identity');
      const scale=this.host.scales.get(tab.tab_id) ?? 1;
      if (![1,2].includes(scale)) refuse('region_geometry');
      const binding_id=`host-region-${randomUUID()}`;
      let retired=false,display=null;
      const retire=async()=>{
        if(retired)return; retired=true; this.leases.delete(binding_id);
        if(display && this.host.displays.get(display.subscription_id) === display) await this.host.removeDisplay(display.subscription_id);
      };
      const alive=()=>{
        try {
          if(retired || stream.resetAt !== resetAt || this.host.protection !== policy || this.host.tabs.get(tab.tab_id)?.target_id !== tab.target_id ||
            (this.host.scales.get(tab.tab_id) ?? 1) !== scale || !same(this.host.pageSize(tab.tab_id),measured.viewport) || !same(identities,this.check(request,scope,stream).identities)) return false;
          return true;
        } catch { return false; }
      };
      const validate=async()=>{
        if(!alive()){await retire();return false;}
        try {
          const now=await this.measure(stream,tab,request,scope);
          if(!alive() || !same(now,measured)){await retire();return false;}
          return true;
        } catch { await retire();return false; }
      };
      const lease=Object.freeze({binding_id,...request,tab_id:tab.tab_id,rect:Object.freeze(measured.rect),
        // Private placement before the producer's inward codec rounding. The
        // viewport clip may start inside an offscreen/ancestor-clipped tile.
        element_offset:Object.freeze({x:measured.rect.x-measured.box[0],y:measured.rect.y-measured.box[1]}),css_width:measured.viewport.width,css_height:measured.viewport.height,
        device_scale_factor:scale,source_revision:stream.issued,input_epoch:request.sequence,
        // isAlive is a cheap synchronous fence; isCurrent re-measures live bounds.
        // A native consumer MUST await isCurrent before readback/delivery/input.
        isAlive:alive,isCurrent:()=>this.locked(stream,validate),close:retire,
        // Private attestation evidence from our isolated observer, never from
        // the subscribe request or the viewer's tile/mask coordinates.
        attestationState:()=>this.locked(stream,async()=>{
          if(!await validate())refuse('region_retired');
          const boxes=await this.mirror.regionBoxes(stream,tab),media=[];
          const evidence=await mediaLayerEvidence(this.mirror,stream,boxes);
          for(const b of evidence){
            const checked=this.check({...request,sequence:stream.issued,region_id:b.id},scope,stream);
            media.push({...b,identities:checked.identities});
          }
          if(!alive())refuse('region_retired');
          return {generation:request.generation,document_id:request.document_id,sequence:request.sequence,media};
        }),
        attachDisplay:current=>{
          if(display || !alive() || current.observed_by !== scope || current.tab_id !== tab.tab_id || typeof current.subscription_id !== 'string') refuse('region_display');
          display=current; this.host.displays.set(current.subscription_id,current);
        },
        resolveInput:(input,caller,{signal:inputSignal}={})=>this.locked(stream,async()=>{
          if(caller !== scope || input?._agent_input || !input || !['click','scroll'].includes(input.kind) || !Number.isFinite(input.x) || !Number.isFinite(input.y) ||
            input.x < 0 || input.y < 0 || input.x >= measured.rect.width || input.y >= measured.rect.height) refuse('region_input');
          if(!await validate()) refuse('region_retired');
          const applied={...request,sequence:input.sequence}; this.check(applied,scope,stream);
          const action={kind:input.kind,node_id:request.region_id,x:measured.rect.x+input.x-measured.box[0],y:measured.rect.y+input.y-measured.box[1]};
          if(input.kind === 'scroll'){action.delta_x=input.delta_x;action.delta_y=input.delta_y;}
          const resolved=await this.mirror.resolveInput(stream,tab,{subscription_id:stream.id,sequence:input.sequence,action},scope,inputSignal);
          const guard=resolved.guard;
          return {...resolved,guard:async()=>{
            if(!await lease.isCurrent()) refuse('region_retired');
            assertNotCancelled(inputSignal); await guard?.();
          }};
        })
      });
      this.leases.set(binding_id,lease); return lease;
    });
  }
  retireStream(stream) { for(const lease of this.leases.values()) if(lease.mirror_subscription_id === stream.id) void lease.close().catch(()=>{}); }
}
