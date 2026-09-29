// Runtime-limits drill (V-RUN-05, V1-INT-03): each tool breaks one contract or
// grows one resource, so the kernel's deadline and memory handling can be
// observed from outside. No tool touches anything but this App's own process.
//   hang              a tool call that never settles and ignores its abort signal
//   arm_startup_hang  the next `startup` lifecycle callback never settles (once)
//   arm_shutdown_hang the next `shutdown` lifecycle callback never settles
//   wasm_grow         grow one WebAssembly.Memory and touch every page
//   threads           start worker threads that each hold a touched Buffer
//   status            what this process holds now
import { existsSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { Worker } from 'node:worker_threads';

const never = () => new Promise(() => {});
const rssMb = () => Math.round(process.memoryUsage().rss / 1048576);

export default function register(chariox) {
  const startupMarker = join(chariox.paths.data, 'hang-next-startup');
  let hangShutdown = false;
  const memories = [];
  const threads = [];

  chariox.lifecycle.on('startup', async () => {
    if (!existsSync(startupMarker)) return null;
    rmSync(startupMarker, { force: true });
    await chariox.log.write('warn', 'startup callback will not settle').catch(() => {});
    return never();
  });
  chariox.lifecycle.on('shutdown', () => (hangShutdown ? never() : null));

  chariox.tools.register('hang', async () => {
    await chariox.log.write('warn', 'tool call will not settle').catch(() => {});
    return never();
  });
  chariox.tools.register('arm_startup_hang', async () => {
    writeFileSync(startupMarker, 'once');
    return { armed: 'startup' };
  });
  chariox.tools.register('arm_shutdown_hang', async () => {
    hangShutdown = true;
    return { armed: 'shutdown' };
  });
  chariox.tools.register('wasm_grow', async ({ mb, step_mb = 32 }) => {
    const started = Date.now();
    const pagesPerStep = (step_mb * 1024 * 1024) / 65536;
    const targetPages = (mb * 1024 * 1024) / 65536;
    // The schema caps mb at 4096 (65536 pages), so only an external limit fails a step.
    const memory = new WebAssembly.Memory({ initial: 1, maximum: targetPages });
    memories.push(memory);
    let error = null;
    while (memory.buffer.byteLength < mb * 1024 * 1024) {
      try {
        const before = memory.buffer.byteLength;
        memory.grow(Math.min(pagesPerStep, targetPages - before / 65536));
        new Uint8Array(memory.buffer, before).fill(7);
      } catch (e) { error = { name: e?.name ?? 'Error', message: String(e?.message ?? e).slice(0, 200) }; break; }
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    return { heldMb: Math.round(memory.buffer.byteLength / 1048576), error, rssMb: rssMb(), ms: Date.now() - started };
  });
  chariox.tools.register('threads', async ({ count, mb_each = 16 }) => {
    const started = Date.now();
    let error = null;
    for (let i = 0; i < count; i += 1) {
      try {
        const worker = new Worker(new URL('./thread.mjs', import.meta.url), { workerData: { mb: mb_each } });
        worker.on('error', () => {});
        // Counted once its Buffer is held; dropped when it exits.
        await new Promise((resolve, reject) => {
          worker.once('message', resolve);
          worker.once('error', reject);
          worker.once('exit', (code) => reject(Object.assign(new Error(`thread exited ${code}`), { code: 'THREAD_EXITED' })));
        });
        threads.push(worker);
        worker.once('exit', () => { threads.splice(threads.indexOf(worker), 1); });
      } catch (e) { error = { name: e?.name ?? 'Error', code: e?.code ?? null, message: String(e?.message ?? e).slice(0, 200) }; break; }
    }
    return { threads: threads.length, error, rssMb: rssMb(), ms: Date.now() - started };
  });
  chariox.tools.register('status', async () => ({
    rssMb: rssMb(), threads: threads.length,
    wasmMb: Math.round(memories.reduce((sum, m) => sum + m.buffer.byteLength, 0) / 1048576),
    hangShutdown, startupArmed: existsSync(startupMarker),
  }));
}
