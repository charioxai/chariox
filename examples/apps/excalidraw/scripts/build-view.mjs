import { build } from 'esbuild';
import { cp, mkdir, readFile, stat, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { checkLicenses, root, files, sha256 } from './licenses.mjs';
import { npmPackages } from '../../../../packages/app-sdk-tools/npm-notices.mjs';

const output = process.argv[2];
if (!output || !isAbsolute(output) || !relative(root, output).startsWith('..')) {
  throw Error('usage: node scripts/build-view.mjs /absolute/output/bundle (outside the checkout)');
}
const inventory = await checkLicenses();
await mkdir(dirname(output), { recursive: true });
try { await mkdir(output); } catch (error) {
  if (error.code === 'EEXIST') throw Error('Choose a fresh bundle directory; an old output could retain unreviewed assets.');
  throw error;
}
await mkdir(join(output, 'ui'));
const result = await build({
  absWorkingDir: root, entryPoints: ['src/main.jsx'], outdir: join(output, 'ui'), entryNames: 'app',
  bundle: true, splitting: true, format: 'esm', target: 'es2022', minify: true,
  conditions: ['production'], define: { 'process.env.NODE_ENV': '"production"' },
  loader: { '.woff2': 'file' },
  legalComments: 'linked', metafile: true,
  plugins: [{ name: 'diagram-only', setup(build) {
    build.onResolve({ filter: /^@excalidraw\/mermaid-to-excalidraw$/ }, () => ({ path: join(root, 'src/mermaid.js') }));
    build.onLoad({ filter: /dist\/prod\/chunk-K2UTITRG\.js$/ }, async ({ path }) => {
      const bytes = await readFile(path);
      if (sha256(bytes) !== '72b54e8e9b3c17c69f1dd5e40203bfaed62313bc611b86ecb8a1625a12562e51') throw Error('Review the changed Excalidraw font registration before updating its pin.');
      const source = bytes.toString('utf8');
      if (source.split('n(Mn,...b1),').length !== 2) throw Error('Expected exactly one pinned Xiaolai registration.');
      // Keep the fallback family and metrics, with zero downloadable faces.
      return { contents: source.replace('n(Mn,...b1),', 'n(Mn),'), loader: 'js', resolveDir: dirname(path) };
    });
  } }],
});
const inputs = Object.fromEntries(Object.keys(result.metafile.inputs).map(file => [resolve(root, file), {}]));
for (const pkg of await npmPackages(inputs)) {
  if (!inventory.packages[pkg.name + '@' + pkg.version]) throw Error(`unlisted dependency in bundle: ${pkg.name}@${pkg.version}`);
}
// Worker sources use dynamic imports; esbuild includes them in the same graph.
const prod = join(root, 'node_modules/@excalidraw/excalidraw/dist/prod');
await cp(join(prod, 'fonts'), join(output, 'ui/fonts'), { recursive: true, filter: source => !source.split(sep).includes('Xiaolai') });
// Upstream ships GPL+font-exception Liberation 1.05. Never distribute it.
await cp(join(root, 'assets/LiberationSans-Regular.woff2'), join(output, 'ui/fonts/Liberation/LiberationSans-Regular.woff2'));
await cp(join(root, 'schemas'), join(output, 'schemas'), { recursive: true });
await cp(join(root, 'licenses'), join(output, 'licenses'), { recursive: true });
await cp(join(root, 'licenses.json'), join(output, 'licenses/inventory.json'));
await cp(join(root, 'LICENSE'), join(output, 'licenses/APP-LICENSE'));
// Preserve notices for the entire pinned dependency closure, including code
// already bundled by upstream before it reaches this esbuild invocation.
const notices = [];
for (const file of await files(join(root, 'node_modules/.pnpm'))) {
  if (/(?:^|\/)(?:license|licence|copying|notice|copyright)[^/]*$/i.test(file)) {
    notices.push(`\n--- ${file} ---\n` + await readFile(join(root, 'node_modules/.pnpm', file), 'utf8'));
  }
}
await writeFile(join(output, 'licenses/DEPENDENCY-NOTICES.txt'), notices.join('\n'));
await writeFile(join(output, 'ui/index.html'), '<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Excalidraw</title><link rel="stylesheet" href="app.css"><style>html,body,#root{height:100%;margin:0}</style><script src="assets.js"></script><script type="module" src="app.js"></script></head><body><div id="root"></div></body></html>');
await writeFile(join(output, 'ui/assets.js'), 'window.EXCALIDRAW_ASSET_PATH = new URL("./", document.currentScript.src).href;');
await writeFile(join(output, 'view-build.json'), JSON.stringify({ inputs: Object.keys(result.metafile.inputs).sort() }, null, 2) + '\n');
let uiBytes = 0;
for (const file of await files(join(output, 'ui'))) uiBytes += (await stat(join(output, 'ui', file))).size;
if (uiBytes > 8 * 1024 * 1024) throw Error(`UI exceeds the browser host's 8 MiB asset budget: ${uiBytes} bytes`);
console.log(`Built Excalidraw view: ${resolve(output)}`);
