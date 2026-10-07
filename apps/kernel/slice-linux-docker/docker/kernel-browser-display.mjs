// MD-DISPLAY-02/04: flag-gated codec/repair policy over the protected host seam.
import {randomUUID} from 'node:crypto';
import { spawn } from 'node:child_process';
import {unlinkSync,mkdtempSync,openSync,writeSync,ftruncateSync,closeSync,rmdirSync} from 'node:fs';
import path from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { decodePng, encodePng, displayMaskRegions, displayNativeMaskSource } from './kernel-browser-pixels.mjs';
import {dirtyTiles, nativeDamageTiles} from './kernel-browser-tiles.mjs';
import {PixelWorker} from './kernel-browser-pixel-worker.mjs';
export {dirtyTiles} from './kernel-browser-tiles.mjs';
import { timestamp } from './kernel-browser-timing.mjs';

export function safeChildPid(child) {
  if (!Number.isSafeInteger(child?.pid) || child.pid <= 1) throw new Error('MD-DISPLAY: unsafe child PID');
  return child.pid;
}
export class PortableEncoder {
  constructor() { this.child = null; this.pending = null; this.failure = null; this.packets=new Set();this.nativeSession=randomUUID(); }
  async encode(png, bitrate, reset = false, codec = 'vp09.00.10.08', regions = []) {
    if(png?.nativeEncode&&codec==='avc1.420033')return this.nativeEncode(png,bitrate,reset,false);
    return typeof png==='object' ? this.exchange({raw:png,bitrate,reset,codec}) : this.exchange({png,bitrate,reset,codec,protected_regions:regions});
  }
  async encodeStripes(raw,bitrate,reset=false,codec='avc1.420033'){
    if(raw.nativeEncode&&codec==='avc1.420033')return this.nativeEncode(raw,bitrate,reset,true);
    return this.exchange({raw,bitrate,reset,codec,operation:'stripes'});
  }
    async nativeEncode(raw,bitrate,reset,stripes){
      const reply=await raw.nativeEncode({encoder:this.nativeSession,bitrate,reset,...(!stripes?{stripes:false}:{}),regions:raw[displayMaskRegions]??[]});
      if(!['native-x264','native-vaapi'].includes(reply.backend)||!Array.isArray(reply.stripes)&&reply.dropped!==true)throw Error('MP-11: native codec reply');
      this.backend=reply.backend==='native-vaapi'?'vaapi':'x264';this.hardwareFallback=reply.hardware_fallback===true;this.converter='libyuv';this.workers=1;
      if(reply.dropped)return {dropped:true};
      if(reply.stripes.length>8||reply.stripes.some(r=>Object.hasOwn(r,'data_base64')))throw Error('MP-11: native row headers');
      if(reply.packet){
        if(!process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT||!/^[a-f0-9]{32}\.json$/.test(reply.packet.name)||!Number.isSafeInteger(reply.packet.length)||reply.packet.length<1||reply.packet.length>1024*1024)throw Error('MP-11: native packet bounds');
        this.packets.add(reply.packet.name);
      }else if(reply.stripes.length)throw Error('MP-11: native packet missing');
      if(!stripes&&reply.stripes.length){if(reply.stripes.length!==1||reply.stripes[0].row!==0||reply.stripes[0].y!==0||reply.stripes[0].height!==raw.height||reply.whole!==true)throw Error('MP-11: native whole frame');this.nativeRevision=reply.revision;return {key:reply.stripes[0].key,packet:reply.packet,native_revision:reply.revision,native_deliver:raw.nativeDelivered};}
      this.nativeRevision=reply.revision;return {stripes:reply.stripes,native_revision:reply.revision,native_deliver:raw.nativeDelivered,...(reply.packet?{packet:reply.packet}:{})};
    }
  async hash(png) { return this.exchange({png,operation:'fingerprint'}); }
  async exchange(request) {
    if(request.raw)request={...request,protected_regions:request.raw[displayMaskRegions]??[]};
    if (this.failure) throw this.failure;
    if (!this.child) {
      const child = spawn(process.env.CHARIOX_BROWSER_DISPLAY_PYTHON || 'python3',
        ['-u', fileURLToPath(new URL('./kernel-browser-encoder.py', import.meta.url))],
        { stdio: ['pipe', 'pipe', 'ignore'] });
      this.child = child;
      const fail = () => {
        this.failure = new Error('MD-DISPLAY: portable encoder unavailable');
        this.pending?.reject(this.failure); this.pending = null;
      };
      child.on('error', fail); child.stdin.on('error', fail); child.on('exit', fail);
      let received = 0;
      child.stdout.on('data', bytes => { received += bytes.length; if (received > 6 * 1024 * 1024) fail(); });
      createInterface({ input: child.stdout }).on('line', line => {
        received = 0;
        try {
          const reply = JSON.parse(line);
          // MP-08/MP-10: helper diagnostics stay private, never in a frame.
          if(process.env.CHARIOX_BROWSER_DISPLAY_TIMING==='1'&&Array.isArray(reply.timings)&&reply.timings.length<=64){
            const stages=new Set(['codec_input_guard','codec_row_copy_hash','codec_row_init','codec_convert','codec_encode','codec_output_guard','codec_base64','codec_packetize']);
            const spans=reply.timings.filter(span=>Array.isArray(span)&&span.length===3&&stages.has(span[0])&&Number.isFinite(span[1])&&Number.isFinite(span[2])&&span[2]>=span[1]);
            if(this.timing?.batch)this.timing.batch(spans);else for(const span of spans)this.timing?.(...span);
          }
          if(reply.error)return fail();
          if(reply.dropped===true){
            this.backend=({libx264:'x264',libopenh264:'openh264',libvpx:'vp8'})[reply.backend]??reply.backend;
            this.converter=['libyuv','swscale'].includes(reply.converter)?reply.converter:null;
            this.pending?.resolve({dropped:true});this.pending=null;return;
          }
          if(this.pending?.stripes){
            if(!Array.isArray(reply.stripes)||reply.stripes.length>8)return fail();
            this.workers=reply.workers;this.backend=({libx264:'x264',libopenh264:'openh264',libvpx:'vp8'})[reply.backend];this.converter=['libyuv','swscale'].includes(reply.converter)?reply.converter:null;if(reply.packet){
              if(!process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT||!/^[a-f0-9]{32}\.json$/.test(reply.packet.name)||!Number.isSafeInteger(reply.packet.length)||reply.packet.length<1||reply.packet.length>1024*1024||reply.stripes.some(r=>Object.hasOwn(r,'data_base64')))return fail();
              this.packets.add(reply.packet.name);
            }
            this.pending.resolve({stripes:reply.stripes,...(reply.packet?{packet:reply.packet}:{})});
          }else if(this.pending?.hash){
            if(!/^[a-f0-9]{64}$/.test(reply.signature)||!Number.isInteger(reply.width)||!Number.isInteger(reply.height)||reply.width<1||reply.height<1||reply.width>2560||reply.height>1600)return fail();
            this.pending.resolve(reply);
          }else{
            if(typeof reply.data_base64!=='string'||reply.data_base64.length>4*1024*1024)return fail();
            this.backend=['vaapi','x264','openh264','vp8','vp9'].includes(reply.backend)?reply.backend:null;
            this.converter=null;
            this.pending?.resolve({data_base64:reply.data_base64,key:reply.key});
          }
          this.pending=null;
        } catch { fail(); }
      });
    }
    if (this.pending) throw new Error('MD-DISPLAY: encoder busy');
    try {
      return await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('MD-DISPLAY: encode timeout')), 10_000);
        this.pending = { stripes:request.operation==='stripes',hash:request.operation==='fingerprint', resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => { clearTimeout(timer); reject(error); } };
        if(request.raw?.[displayNativeMaskSource]){
          // MP-08/MP-10/MP-11: lease held by MotionEncoder until the native
          // helper finishes. No unmasked bytes enter a codec or packet file.
          this.child.stdin.write(JSON.stringify({...request,raw:{...request.raw,shared:request.raw[displayNativeMaskSource],mask_shared:true}})+'\n');
        }else if(request.raw?.shared){this.child.stdin.write(JSON.stringify(request)+'\n')}else if(request.raw){
          const {pixels,...raw}=request.raw,root=process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT;
          if(root){
            // MP-08/MP-10/MP-11: one private immutable request snapshot, not
            // an unmasked capture lease. The helper closes its mmap before
            // replying; only then can the next exchange overwrite this file.
            const at=timestamp();
            if(!Buffer.isBuffer(pixels)||pixels.length!==raw.length||pixels.length>2560*1600*4)throw Error('MP-11: encoder raster bound');
            if(!this.raster){
              const directory=mkdtempSync(path.join(root,'encoder-')),file=path.join(directory,'raster');
              this.raster={directory,file,fd:openSync(file,'wx',0o600)};
            }
            let offset=0;while(offset<pixels.length){const written=writeSync(this.raster.fd,pixels,offset,pixels.length-offset,offset);if(written<=0)throw Error('MP-11: encoder raster write');offset+=written;}
            ftruncateSync(this.raster.fd,pixels.length);
            this.timing?.('encoder_raster_handoff',at);
            this.child.stdin.write(JSON.stringify({...request,raw:{...raw,shared:{path:this.raster.file,length:pixels.length}}})+'\n');
          }else{this.child.stdin.write(JSON.stringify({...request,raw})+'\n');this.child.stdin.write(pixels)}
        }else this.child.stdin.write(JSON.stringify(request) + '\n');
      });
    } catch (error) { await this.close(); throw error; }
  }
  discard(encoded){
    const name=encoded?.packet?.name;
    if(name&&this.packets.delete(name))try{unlinkSync(path.join(process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT,name))}catch(error){if(error.code!=='ENOENT')throw error}
  }
  handedOff(encoded){if(encoded?.packet)this.packets.delete(encoded.packet.name);if(Number.isSafeInteger(encoded?.native_revision)){encoded.native_deliver?.(this.nativeSession,encoded.native_revision);this.nativeDeliveredRevision=encoded.native_revision;}}
  async close() {
    try{await this.closeChild()}finally{
      if(this.raster){const {fd,file,directory}=this.raster;this.raster=null;closeSync(fd);unlinkSync(file);rmdirSync(directory);}
    }
  }
  async closeChild() {
    for(const name of [...this.packets])this.discard({packet:{name}});
    const child = this.child; this.child = null;
    if (!child || child.pid === undefined) return;
    safeChildPid(child);
    if (child.exitCode !== null || child.signalCode !== null) return;
    child.stdin.end();
    for (let i = 0; i < 20 && child.exitCode === null && child.signalCode === null; i++) await delay(25);
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
    for (let i = 0; i < 20 && child.exitCode === null && child.signalCode === null; i++) await delay(25);
    if (child.exitCode === null && child.signalCode === null) {
      child.kill('SIGKILL');
      await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('MD-DISPLAY: owned encoder did not exit')), 2000);
        child.once('exit', () => { clearTimeout(timer); resolve(); });
      });
    }
  }
}

// MP-08/MP-10/MP-11: one exact raster extraction path for display refinement
// and DOM fallback. Pixels must already have crossed the protection barrier.
export function losslessRegion(frame,x,y,width,height) {
  if(![x,y,width,height].every(Number.isSafeInteger)||x<0||y<0||width<1||height<1||x+width>frame.width||y+height>frame.height)throw Error('MP-11: invalid lossless raster region');
  const pixels=Buffer.alloc(width*height*4);
  for(let row=0;row<height;row++)frame.pixels.copy(pixels,row*width*4,((y+row)*frame.width+x)*4,((y+row)*frame.width+x+width)*4);
  return {x,y,width,height,data_base64:encodePng(width,height,pixels)};
}

export const exactPatchLimit=bitrate=>Math.floor(Math.min(192_000,Math.max(24_000,bitrate/8*.5*.75-4096)));

export class DisplayStream {
  constructor(binding, { encoder = new PortableEncoder(), now = () => performance.now(), wait = delay, timing = () => {} } = {}) {
    this.pixels=new PixelWorker();Object.assign(this, binding); this.encoder = encoder; this.now = now; this.wait = wait;
    this.sequence = 0; this.previous = null; this.exact = false; this.repair = null;this.repairSerial=null;
    this.refillAt = now(); this.tokens = 0;
    this.expires = Date.now() + 60_000;
    this.timing = timing;
    this.encoder.timing = timing;
  }
  canPatchNative(sample) {
    return Boolean(this.previous) && !this.repair && sample.serial === this.compositorSerial + 1 &&
      nativeDamageTiles(sample.raw, true) !== null;
  }
  acceptsCredit(after) { return Number.isSafeInteger(after) && after >= Math.max(0,this.sequence-8) && after <= this.sequence; }
  invalidate() { this.motionActive=false;this.compositorSerial=null;this.previous = null; this.exact = false; this.repair = null;this.repairSerial=null; this.capture?.invalidate();this.refiner?.invalidate();this.producer?.invalidate(); }
  async frame(source, documentId, afterSequence, validate = async () => true, currentBinding = () => true) {
    try{return await this.buildFrame(source,documentId,afterSequence,validate,currentBinding)}
    catch(error){this.encoder.discard?.(source.encoded);throw error}
  }
  async buildFrame(source, documentId, afterSequence, validate, currentBinding) {
    let at = timestamp();
    const current = source.motion || source.native_tiles || source.native_exact ? {width:source.width,height:source.height,signature:source.data_base64,pixels:null}
      : source.pixels ?? await this.pixels.run('decode',{data:source.data_base64,scale:this.device_scale_factor});
    this.timing('png_decode', at); at = timestamp();
    // Already-admitted 419 credits may lag the delivered sequence. Eight
    // frames is the hard recovery window; clients still validate every base.
    const bound = documentId === this.document_id && this.acceptsCredit(afterSequence);
    if (source.native_tiles && !bound) throw Error('MD-DISPLAY: native patch base lost');
    const wasExact = this.exact;
    if (!bound) {
      this.invalidate();
      // Selection can already have taken a delta from the producer. Resetting
      // only future work cannot make that packet independently decodable.
      if (source.encoded && (source.encoded.stripes ? source.encoded.stripes.length!==8||source.encoded.stripes.some(r=>!r.key) : !source.encoded.key)) {this.encoder.discard?.(source.encoded);return null;}
    }
    const same = current.signature ? this.previous?.signature === current.signature : Boolean(this.previous?.pixels && this.previous.pixels.equals(current.pixels));
    // Taken packets have already advanced the persistent codec reference chain.
    // Only suppress duplicates before encoding; every encoded dependency ships.
    if (same && (this.exact || source.motion) && !source.encoded && !source.native_tiles) return null;
    // Private crop metadata comes only from the protected native capture. The
    // cached frame is cloned unchanged outside it; full verification has no hint.
    const clip = source.dirty_clip;
    const region = clip && { x:clip.x*this.device_scale_factor, y:clip.y*this.device_scale_factor,
      width:clip.width*this.device_scale_factor, height:clip.height*this.device_scale_factor };
    const tiles = this.exact && this.previous?.pixels && !source.motion && !source.native_tiles ? await this.pixels.run('tiles',{previous:this.previous,current,region}) : [];
    this.timing('compare_tiles', at); at = timestamp();
    const png = () => typeof source.data_base64 === 'function' ? source.data_base64() : source.data_base64;
    const full = () => ({ kind: 'png', data_base64: png() });
    const patch = { kind: 'tiles', base_sequence: this.sequence, tiles };
    // Half a second of negotiated frame budget, including outer base64. One
    // credit remains outstanding; narrow links reduce batch size/cadence.
    const patchLimit = exactPatchLimit(this.bitrate);
    let payload, repair = null;
    if (source.native_tiles) payload = {kind:'tiles', base_sequence:this.sequence, tiles:source.native_tiles};
    else if (bound && (same || source.settled_verified) && (!this.exact || !this.previous?.pixels) && !source.motion) {
      const exact = full();
      if (JSON.stringify(exact).length <= patchLimit&&(!source.repair_tiles||JSON.stringify(source.repair_tiles).length+128>=JSON.stringify(exact).length)) payload = exact;
      else {
        // A verified capture may supersede the immutable RGB snapshot while
        // repair batches are still queued. Never mark its older tiles exact.
        if(this.repair && (!same || this.repairSerial!==source.refinement_serial))this.repair=null;
        const remaining = this.repair ?? source.repair_tiles ?? await this.pixels.run('tiles',{previous:null,current,all:true});
        const batch = []; let size = 128;
        for (const tile of remaining) {
          const cost = JSON.stringify(tile).length + 1;
          if (batch.length && size + cost > patchLimit) break;
          batch.push(tile); size += cost;
        }
        payload = { kind:'tiles', base_sequence:this.sequence, tiles:batch };
        repair = remaining.slice(batch.length);
      }
    } else if (!source.motion && tiles.length && JSON.stringify(patch).length < Math.min(patchLimit, source.full_size_hint ?? JSON.stringify(full()).length)) payload = patch;
    else if (this.codec === 'png'||source.force_lossless) payload = full();
    else {
      const encoded = source.encoded ?? await this.encoder.encode(png(), this.bitrate, !this.dependencies || !bound || !this.previous || this.exact || Boolean(this.repair),this.codec,source[displayMaskRegions]??[]);
      if(encoded.dropped)payload=full();
      else if(encoded.stripes){payload={kind:'stripes',base_sequence:this.sequence,stripes:encoded.stripes,...(encoded.packet?{native_packet:encoded.packet}:{})};}
      else {
      if(!this.dependencies && typeof encoded!=='string' && !encoded.key)throw Error('MD-DISPLAY: unnegotiated dependent frame');
      payload = { kind:'video', codec:this.codec, ...(typeof encoded === 'string' ? {key:true,data_base64:encoded} : encoded.packet ? {key:encoded.key,native_packet:encoded.packet} : encoded) };
      }
    }
    this.timing('select_encode', at); at = timestamp();
    const packet = { ...payload, subscription_id: this.subscription_id, tab_id: this.tab_id,
      generation: source.generation, document_id: documentId, sequence: this.sequence + 1,
      width: source.motion ? (this.css_width??1280)*this.device_scale_factor : current.width, height: source.motion ? (this.css_height??800)*this.device_scale_factor : current.height, css_width: this.css_width??1280, css_height: this.css_height??800,
      device_scale_factor: this.device_scale_factor, colour: 'srgb' };
    const bytes = Math.ceil((Buffer.byteLength(JSON.stringify(packet))+(payload.native_packet?.length??0)) * 4 / 3) + 1024; // reserve transport/encryption envelope
    if (bytes > 1024 * 1024) throw new Error('MD-DISPLAY: packet exceeds bounded egress');
    this.timing('packet_serialize', at); at = timestamp();
    // MD-DISPLAY-02/04: bounded 16 KiB burst accrued while capture/input runs.
    // Account all base64/envelope bytes, including bootstrap and exact repairs.
    this.tokens = Math.min(16 * 1024, this.tokens + Math.max(0, this.now() - this.refillAt) * this.bitrate / 8000);
    // Absolute time, rather than repeated relative slices: a busy event loop
    // can oversleep a timer. Charge those real milliseconds once, never again.
    this.refillAt = this.now();
    // MP-08/MP-10: one input frame borrows at most32KiB. Keep the debt
    // so later input/motion repays it; large repairs never bypass the ceiling.
    const urgent=source.input_triggered===true&&bytes<=32768&&this.tokens>=0;
    const deadline = this.now() + (urgent?0:Math.max(0,bytes-this.tokens)*8000/this.bitrate);
    if(deadline<=this.now())await this.wait(0);
    while(this.now()<deadline){
      if(!currentBinding()){
        if(['video','stripes'].includes(payload.kind))this.invalidate();else{this.capture?.invalidate();this.refiner?.invalidate();this.repair=null;}
        this.encoder.discard?.(source.encoded);return null;
      }
      const before=this.now(),slice=Math.min(deadline-before,8);await this.wait(slice);
      // Deterministic test clocks may not advance; real clocks always do.
      if(this.now()===before)break;
    }
    this.tokens = Math.min(16*1024, this.tokens + Math.max(0,this.now()-this.refillAt)*this.bitrate/8000)-bytes;
    this.tokens = Math.max(urgent?-32768:0,this.tokens); this.refillAt = this.now();
    this.timing('pacing', at);
    if (!await validate()) { if(['video','stripes'].includes(payload.kind))this.invalidate();else{this.capture?.invalidate();this.refiner?.invalidate();this.repair=null;}this.encoder.discard?.(source.encoded);return null; }
    this.encoder.handedOff?.(source.encoded);
    this.document_id = documentId; this.previous = current; this.repair = repair?.length ? repair : null;this.repairSerial=source.refinement_serial;
    // A small exact patch can acknowledge input over a lossy video base. It
    // certifies only its damaged pixels; idle native verification still repairs
    // the untouched raster before the whole frame becomes exact.
    this.exact = source.native_tiles ? wasExact : !['video','stripes'].includes(payload.kind) && !this.repair; this.sequence++;
    if(!['video','stripes'].includes(payload.kind))this.producer?.retireUnsent();
    return packet;
  }
  async close() { clearTimeout(this.timer); this.invalidate(); await this.producer?.close();await this.refiner?.close();await this.pixels.close();await this.encoder.close(); }
}
