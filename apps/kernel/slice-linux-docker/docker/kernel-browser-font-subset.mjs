// MP-10 first-visit bytes: a web font travels as the glyphs the page can show, not as the whole face. Decoded, subset
// and encoded again by HarfBuzz and the WOFF2 codec in wasm (apps/kernel/vendor/font-subset).
import { createHash } from 'node:crypto';
import { Worker } from 'node:worker_threads';

// A face below this size travels whole: the saving would not pay for the worker's round trip.
export const SUBSET_MIN_BYTES = 32 * 1024;
const BROTLI_QUALITY = 9, IDLE_MS = 20_000, KEPT_FACES = 32;
// What any page may show after it loads, whatever the text it started with: ASCII, the no-break space, typographic
// punctuation, the euro sign and the trade mark sign.
const STANDARD_RANGES = [[0x20, 0x7e], [0xa0, 0xa0], [0x2010, 0x2027], [0x2030, 0x203a], [0x20ac, 0x20ac], [0x2122, 0x2122]];
// The letters later text will bring: a script the page really uses (this many distinct letters in one 128-code-point
// block, fewer for Latin-1 and Latin Extended-A, whose accents any language of the page may add) keeps its whole block;
// a stray character keeps itself only. Ideographs (the large blocks from U+3000) always keep themselves only.
const LATIN_BLOCKS = new Set([1, 2]), LATIN_LETTERS = 6, SCRIPT_LETTERS = 24, BLOCK_LIMIT = 0x3000;

export function subsetCodePoints(pageCodePoints) {
  const wanted = new Set(), blocks = new Map();
  for (const [first, last] of STANDARD_RANGES) for (let at = first; at <= last; at++) wanted.add(at);
  for (const code of new Set(pageCodePoints)) {
    wanted.add(code);
    if (code >= 0x80 && code < BLOCK_LIMIT) blocks.set(code >> 7, (blocks.get(code >> 7) ?? 0) + 1);
  }
  for (const [block, letters] of blocks) if (letters >= (LATIN_BLOCKS.has(block) ? LATIN_LETTERS : SCRIPT_LETTERS)) for (let at = block << 7; at < (block + 1) << 7; at++) wanted.add(at);
  return Uint32Array.from(wanted).sort();
}

class Subsetter {
  #worker = null; #ready = null; #idle = null; #next = 1; #pending = new Map();
  #start() {
    this.#worker = new Worker(new URL('./kernel-browser-font-subset-worker.mjs', import.meta.url));
    this.#ready = new Promise((resolve, reject) => { this.#worker.once('message', resolve); this.#worker.once('error', reject); });
    const failed = error => { for (const { reject } of this.#pending.values()) reject(error); this.#pending.clear(); this.#stop(); };
    this.#worker.on('error', failed);
    this.#worker.on('exit', code => failed(new Error(`font subsetter worker exited with ${code}`)));
    this.#worker.on('message', ({ id, bytes }) => { const call = this.#pending.get(id); if (!call) return; this.#pending.delete(id); call.resolve(bytes ? Buffer.from(bytes.buffer, bytes.byteOffset, bytes.length) : null); this.#rest(); });
  }
  #stop() { clearTimeout(this.#idle); this.#idle = null; const worker = this.#worker; this.#worker = this.#ready = null; worker?.terminate(); }
  // The wasm's memory only grows: an idle worker is dropped, and until then it does not keep the process alive.
  #rest() { clearTimeout(this.#idle); if (!this.#pending.size) { this.#worker?.unref(); this.#idle = setTimeout(() => this.#stop(), IDLE_MS); this.#idle.unref(); } }
  async run(bytes, codePoints) {
    if (!this.#worker) this.#start();
    clearTimeout(this.#idle);
    this.#worker.ref();
    await this.#ready;
    const id = this.#next++, copy = new Uint8Array(bytes);
    return new Promise((resolve, reject) => { this.#pending.set(id, { resolve, reject }); this.#worker.postMessage({ id, bytes: copy, codePoints, quality: BROTLI_QUALITY }, [copy.buffer]); });
  }
}

// The subsets this process made, by the digest of the face they came from: a later page of the site shows the same
// bytes, so the viewer's retained copy still names them. A page that needs a letter the subset lacks gets a new one.
export class FontSubsets {
  constructor() { this.subsetter = new Subsetter(); this.faces = new Map(); }
  // The face as it should travel, or null when it stays whole.
  // `pageCodePoints` reads the page's characters, asked only for a face large enough to subset.
  async subset(bytes, pageCodePoints) {
    if (bytes.length < SUBSET_MIN_BYTES) return null;
    const whole = createHash('sha256').update(bytes).digest('hex'), wanted = subsetCodePoints(await pageCodePoints()), kept = this.faces.get(whole);
    if (kept && wanted.every(code => kept.codePoints.has(code))) return kept.bytes;
    const codePoints = kept ? subsetCodePoints([...kept.codePoints, ...wanted]) : wanted, out = await this.subsetter.run(bytes, codePoints);
    if (!out || out.length >= bytes.length) return null;
    this.faces.delete(whole); this.faces.set(whole, { bytes: out, codePoints: new Set(codePoints) });
    for (const stale of [...this.faces.keys()].slice(0, Math.max(0, this.faces.size - KEPT_FACES))) this.faces.delete(stale);
    return out;
  }
}
