// Runs the jco-transpiled async probe: N concurrent run(1) calls on one
// instance, each awaiting a 50 ms async host import.
import { readFile } from 'node:fs/promises';
import { WASIShim } from '@bytecodealliance/preview2-shim/instantiation';
const dir = process.argv[2];
const calls = Number(process.argv[3] ?? 8);
const { instantiate } = await import(`${dir}/probe.js`);
const getCoreModule = async path => WebAssembly.compile(await readFile(`${dir}/${path}`));
const imports = {
  ...new WASIShim().getImportObject(),
  'spike:probe/kernel': { get: async key => { await new Promise(r => setTimeout(r, 50)); return key; } },
};
try {
  const probe = await instantiate(getCoreModule, imports);
  const started = performance.now();
  const results = await Promise.allSettled(Array.from({ length: calls }, () => probe.run(1)));
  const ok = results.filter(r => r.status === 'fulfilled').length;
  console.log(JSON.stringify({ node: process.version, calls, ok, totalMs: +(performance.now() - started).toFixed(1), firstError: String(results.find(r => r.status === 'rejected')?.reason ?? '').slice(0, 160) }));
} catch (error) {
  console.log(JSON.stringify({ node: process.version, failed: String(error?.message ?? error).slice(0, 200) }));
}
