// MD-DISPLAY-02/04: flag-gated codec/repair policy over the protected host seam.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { decodePng, encodePng } from './kernel-browser-pixels.mjs';

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

export function dirtyTiles(previous, current) {
  const tiles = [];
  if (!previous || previous.width !== current.width || previous.height !== current.height) return tiles;
  const { width, height, pixels } = current;
  for (let y = 0; y < height; y += 128) for (let x = 0; x < width; x += 128) {
    const w = Math.min(128, width - x), h = Math.min(128, height - y);
    let changed = false;
    for (let row = y; row < y + h && !changed; row++) {
      const offset = (row * width + x) * 4;
      changed = !pixels.subarray(offset, offset + w * 4).equals(previous.pixels.subarray(offset, offset + w * 4));
    }
    if (!changed) continue;
    const data = Buffer.alloc(w * h * 4);
    for (let row = 0; row < h; row++) pixels.copy(data, row * w * 4, ((y + row) * width + x) * 4, ((y + row) * width + x + w) * 4);
    tiles.push({ x, y, width: w, height: h, data_base64: encodePng(w, h, data) });
  }
  return tiles;
}

export class DisplayStream {
  constructor(binding, { encoder = new PortableEncoder(), now = () => performance.now(), wait = delay } = {}) {
    Object.assign(this, binding); this.encoder = encoder; this.now = now; this.wait = wait;
    this.sequence = 0; this.previous = null; this.exact = false; this.nextSend = now();
    this.expires = Date.now() + 60_000;
  }
  invalidate() { this.previous = null; this.exact = false; }
  async frame(source, documentId, afterSequence) {
    const current = decodePng(source.data_base64, this.device_scale_factor);
    const bound = documentId === this.document_id && afterSequence === this.sequence;
    if (!bound) this.invalidate();
    const same = this.previous?.pixels.equals(current.pixels);
    if (same && this.exact) return null;
    const tiles = this.exact ? dirtyTiles(this.previous, current) : [];
    const full = { kind: 'png', data_base64: source.data_base64 };
    const patch = { kind: 'tiles', base_sequence: this.sequence, tiles };
    let payload;
    if (same || this.codec === 'png') payload = full;
    else if (tiles.length && JSON.stringify(patch).length < Math.min(48_000, JSON.stringify(full).length)) payload = patch;
    else payload = { kind: 'video', codec: 'vp09.00.10.08', key: true, data_base64: await this.encoder.encode(source.data_base64, this.bitrate) };
    const packet = { ...payload, subscription_id: this.subscription_id, tab_id: this.tab_id,
      generation: source.generation, document_id: documentId, sequence: this.sequence + 1,
      width: current.width, height: current.height, css_width: 1280, css_height: 800,
      device_scale_factor: this.device_scale_factor, colour: 'srgb' };
    const bytes = Math.ceil(Buffer.byteLength(JSON.stringify(packet)) * 4 / 3) + 1024; // reserve transport/encryption envelope
    if (bytes > 1024 * 1024) throw new Error('MD-DISPLAY: packet exceeds bounded egress');
    // No burst credit: account base64+metadata, including bootstrap and repairs.
    const at = Math.max(this.now(), this.nextSend) + bytes * 8000 / this.bitrate;
    await this.wait(Math.max(0, at - this.now())); this.nextSend = at;
    this.document_id = documentId; this.previous = current; this.exact = payload.kind !== 'video'; this.sequence++;
    return packet;
  }
  async close() { clearTimeout(this.timer); this.invalidate(); await this.encoder.close(); }
}
