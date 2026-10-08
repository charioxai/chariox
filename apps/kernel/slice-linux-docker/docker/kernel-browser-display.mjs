// MD-DISPLAY-02/04: flag-gated codec/repair policy over the protected host seam.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { decodePng, encodePng } from './kernel-browser-pixels.mjs';
import { timestamp } from './kernel-browser-timing.mjs';

export function safeChildPid(child) {
  if (!Number.isSafeInteger(child?.pid) || child.pid <= 1) throw new Error('MD-DISPLAY: unsafe child PID');
  return child.pid;
}
export class PortableEncoder {
  constructor() { this.child = null; this.pending = null; this.failure = null; }
  async encode(png, bitrate) {
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
          if (reply.error || typeof reply.data_base64 !== 'string' || reply.data_base64.length > 4 * 1024 * 1024) return fail();
          this.pending?.resolve(reply.data_base64); this.pending = null;
        } catch { fail(); }
      });
    }
    if (this.pending) throw new Error('MD-DISPLAY: encoder busy');
    try {
      return await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('MD-DISPLAY: encode timeout')), 10_000);
        this.pending = { resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => { clearTimeout(timer); reject(error); } };
        this.child.stdin.write(JSON.stringify({ png, bitrate }) + '\n');
      });
    } catch (error) { await this.close(); throw error; }
  }
  async close() {
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

export function dirtyTiles(previous, current, region = null) {
  const tiles = [];
  if (!previous || previous.width !== current.width || previous.height !== current.height) return tiles;
  const { width, height, pixels } = current;
  const left = region ? Math.floor(region.x / 128) * 128 : 0;
  const top = region ? Math.floor(region.y / 128) * 128 : 0;
  const right = region ? Math.min(width, region.x + region.width) : width;
  const bottom = region ? Math.min(height, region.y + region.height) : height;
  for (let y = top; y < bottom; y += 128) for (let x = left; x < right; x += 128) {
    const w = Math.min(128, width - x), h = Math.min(128, height - y);
    let changed = false;
    for (let row = y; row < y + h && !changed; row++) {
      const offset = (row * width + x) * 4;
      changed = !pixels.subarray(offset, offset + w * 4).equals(previous.pixels.subarray(offset, offset + w * 4));
    }
    if (!changed) continue;
    tiles.push(losslessRegion(current,x,y,w,h));
  }
  return tiles;
}

export class DisplayStream {
  constructor(binding, { encoder = new PortableEncoder(), now = () => performance.now(), wait = delay, timing = () => {} } = {}) {
    Object.assign(this, binding); this.encoder = encoder; this.now = now; this.wait = wait;
    this.sequence = 0; this.previous = null; this.exact = false;
    this.refillAt = now(); this.tokens = 0;
    this.expires = Date.now() + 60_000;
    this.timing = timing;
  }
  invalidate() { this.previous = null; this.exact = false; this.capture?.invalidate(); }
  async frame(source, documentId, afterSequence) {
    let at = timestamp();
    const current = source.pixels ?? decodePng(source.data_base64, this.device_scale_factor);
    this.timing('png_decode', at); at = timestamp();
    const bound = documentId === this.document_id && afterSequence === this.sequence;
    if (!bound) this.invalidate();
    const same = this.previous?.pixels.equals(current.pixels);
    if (same && this.exact) return null;
    // Private crop metadata comes only from the protected native capture. The
    // cached frame is cloned unchanged outside it; full verification has no hint.
    const clip = source.dirty_clip;
    const region = clip && { x:clip.x*this.device_scale_factor, y:clip.y*this.device_scale_factor,
      width:clip.width*this.device_scale_factor, height:clip.height*this.device_scale_factor };
    const tiles = this.exact ? dirtyTiles(this.previous, current, region) : [];
    this.timing('compare_tiles', at); at = timestamp();
    const png = () => typeof source.data_base64 === 'function' ? source.data_base64() : source.data_base64;
    const full = () => ({ kind: 'png', data_base64: png() });
    const patch = { kind: 'tiles', base_sequence: this.sequence, tiles };
    let payload;
    if (same || (bound && this.previous && !this.exact)) payload = full();
    else if (tiles.length && JSON.stringify(patch).length < Math.min(48_000, source.full_size_hint ?? JSON.stringify(full()).length)) payload = patch;
    else if (this.codec === 'png') payload = full();
    else payload = { kind: 'video', codec: 'vp09.00.10.08', key: true, data_base64: await this.encoder.encode(png(), this.bitrate) };
    this.timing('select_encode', at); at = timestamp();
    const packet = { ...payload, subscription_id: this.subscription_id, tab_id: this.tab_id,
      generation: source.generation, document_id: documentId, sequence: this.sequence + 1,
      width: current.width, height: current.height, css_width: 1280, css_height: 800,
      device_scale_factor: this.device_scale_factor, colour: 'srgb' };
    const bytes = Math.ceil(Buffer.byteLength(JSON.stringify(packet)) * 4 / 3) + 1024; // reserve transport/encryption envelope
    if (bytes > 1024 * 1024) throw new Error('MD-DISPLAY: packet exceeds bounded egress');
    this.timing('packet_serialize', at); at = timestamp();
    // MD-DISPLAY-02/04: bounded 16 KiB burst accrued while capture/input runs.
    // Account all base64/envelope bytes, including bootstrap and exact repairs.
    this.tokens = Math.min(16 * 1024, this.tokens + Math.max(0, this.now() - this.refillAt) * this.bitrate / 8000);
    await this.wait(Math.max(0, bytes - this.tokens) * 8000 / this.bitrate);
    this.tokens = Math.max(0, this.tokens - bytes); this.refillAt = this.now();
    this.timing('pacing', at);
    this.document_id = documentId; this.previous = current; this.exact = payload.kind !== 'video'; this.sequence++;
    return packet;
  }
  async close() { clearTimeout(this.timer); this.invalidate(); await this.encoder.close(); }
}
