// The font subsetter's wasm (apps/kernel/vendor/font-subset) in a worker thread: a face takes tens of milliseconds
// to decode, subset and encode, which the mirror's event loop must not wait for.
import { readFileSync } from 'node:fs';
import { parentPort } from 'node:worker_threads';

let memory;
// The module's libc asks for these four calls; it reads no file and opens no socket.
const wasi = {
  fd_close: () => 0,
  fd_seek: () => 70,
  fd_write: (_fd, iovs, count, written) => { const view = new DataView(memory.buffer); let total = 0; for (let at = 0; at < count; at++) total += view.getUint32(iovs + at * 8 + 4, true); view.setUint32(written, total, true); return 0; },
  proc_exit: code => { throw new Error(`font subsetter exited with ${code}`); },
  random_get: (pointer, length) => { crypto.getRandomValues(new Uint8Array(memory.buffer, pointer, length)); return 0; },
};
const { instance } = await WebAssembly.instantiate(readFileSync(new URL('./kernel-browser-font-subset.wasm', import.meta.url)), { wasi_snapshot_preview1: wasi });
const { font_alloc: alloc, font_free: free, font_subset: subset, _initialize: initialize } = instance.exports;
memory = instance.exports.memory;
initialize();

parentPort.on('message', ({ id, bytes, codePoints, quality }) => {
  const input = alloc(bytes.length), wanted = alloc(codePoints.length * 4), length = alloc(4);
  let reply = null;
  try {
    new Uint8Array(memory.buffer, input, bytes.length).set(bytes);
    new Uint32Array(memory.buffer, wanted, codePoints.length).set(codePoints);
    const out = subset(input, bytes.length, wanted, codePoints.length, quality, length);
    if (out) { reply = new Uint8Array(memory.buffer, out, new DataView(memory.buffer).getUint32(length, true)).slice(); free(out); }
  } finally { free(input); free(wanted); free(length); }
  parentPort.postMessage({ id, bytes: reply }, reply ? [reply.buffer] : []);
});
parentPort.postMessage({ ready: true });
