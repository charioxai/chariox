// Builds the componentize-js App. Usage: node build-js-app.mjs <out.wasm> [--aot]
import { componentize } from '@bytecodealliance/componentize-js';
import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
const root = process.env.SPIKE_ROOT ?? fileURLToPath(new URL('../', import.meta.url));
const started = performance.now();
const { component } = await componentize({
  sourcePath: `${root}apps/js-app/app.js`,
  witWorld: await readFile(`${root}wit/app.wit`, 'utf8'),
  worldName: 'app',
  disableFeatures: ['http', 'fetch-event'],
  enableAot: process.argv.includes('--aot'),
});
await writeFile(process.argv[2], component);
console.log(`built ${process.argv[2]} ${component.length} bytes in ${Math.round(performance.now() - started)} ms`);
