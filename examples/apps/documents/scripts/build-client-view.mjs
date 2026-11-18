// The Documents page as the self-contained client view that a window carries as the App's UI entry.
// usage: node scripts/build-client-view.mjs <absolute output directory outside the checkout>
// Writes <output>/index.html. The bundler is the SDK tools' own (packages/app-sdk-tools).
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, isAbsolute, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { encodeView } from '../../../../packages/app-view-host/bundle.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

export async function buildView() {
  const { build } = createRequire(join(root, '../../../packages/app-sdk-tools/package.json'))('esbuild');
  const bundled = await build({
    absWorkingDir: root, entryPoints: ['src/main.js'], bundle: true, write: false, format: 'esm', target: 'es2022', minify: true,
  });
  const style = await readFile(join(root, 'src/style.css'), 'utf8');
  return encodeView({ schema: 'chariox.app-view-source.v1', rootId: 'app', script: bundled.outputFiles[0].text, style });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const output = process.argv[2];
  if (!output || !isAbsolute(output) || !relative(root, output).startsWith('..')) {
    throw Error('usage: node scripts/build-client-view.mjs /absolute/output/directory (outside the checkout)');
  }
  await mkdir(output, { recursive: true });
  await writeFile(join(output, 'index.html'), await buildView());
}
