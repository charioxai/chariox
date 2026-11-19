// The Slack page as the self-contained client view a window carries as the App's UI entry.
// usage: node scripts/build-view.mjs <absolute output directory outside the checkout>
// Writes <output>/index.html next to nothing else: the page's script and style are inside it.
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, isAbsolute, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { encodeView } from '../../../../packages/app-view-host/bundle.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const output = process.argv[2];
if (!output || !isAbsolute(output) || !relative(root, output).startsWith('..')) {
  throw Error('usage: node scripts/build-view.mjs /absolute/output/directory (outside the checkout)');
}
// esbuild is the SDK tools' own build dependency (the same one that builds the backend).
const { build } = createRequire(new URL('../../../../packages/app-sdk-tools/package.json', import.meta.url))('esbuild');
const bundled = await build({
  entryPoints: [join(root, 'view/app.js')], bundle: true, write: false, format: 'esm', target: 'es2022', minify: true, legalComments: 'none',
});
await mkdir(output, { recursive: true });
await writeFile(join(output, 'index.html'), encodeView({
  schema: 'chariox.app-view-source.v1', rootId: 'app', script: bundled.outputFiles[0].text, style: await readFile(join(root, 'view/style.css'), 'utf8'),
}));
