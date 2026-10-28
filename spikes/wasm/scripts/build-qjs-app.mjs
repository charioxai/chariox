// Builds the QuickJS App. Usage: node build-qjs-app.mjs <out.wasm> [--opt-size]
import { componentize } from 'componentize-qjs';
import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
const root = process.env.SPIKE_SRC ?? fileURLToPath(new URL('../', import.meta.url));
const entry = `${root}apps/qjs-app/app.js`;
const started = performance.now();
const { component } = await componentize({
  witPath: `${root}wit/app.wit`,
  jsSource: await readFile(entry, 'utf8'),
  jsPath: entry,
  world: 'app',
  optSize: process.argv.includes('--opt-size'),
});
await writeFile(process.argv[2], component);
console.log(`built ${process.argv[2]} ${component.length} bytes in ${Math.round(performance.now() - started)} ms`);
