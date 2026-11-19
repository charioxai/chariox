// Assembles the App as a developer-push directory: the manifest, the TypeScript entry and its sources, the signed
// declarations, the icon, the licences of what the window carries, and the window itself as one self-contained view.
// usage: node scripts/build-package.mjs <absolute output directory outside the checkout>
import { build, transform } from 'esbuild';
import { cp, mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { encodeView } from '../../../../packages/app-view-host/bundle.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const output = process.argv[2];
if (!output || !isAbsolute(output) || !relative(root, output).startsWith('..')) {
  throw Error('usage: node scripts/build-package.mjs /absolute/output/directory (outside the checkout)');
}
const modules = join(root, 'node_modules/@primer');

/** Only the icons the window names are carried (every quoted word of the view that is an Octicon's name). */
async function octicons() {
  const data = JSON.parse(await readFile(join(modules, 'octicons/build/data.json'), 'utf8'));
  const used = new Set();
  for (const file of await readdir(join(root, 'view'))) {
    if (!file.endsWith('.mjs')) continue;
    for (const [, name] of (await readFile(join(root, 'view', file), 'utf8')).matchAll(/'([a-z][a-z0-9-]*)'/g)) if (data[name]) used.add(name);
  }
  const icons = Object.fromEntries([...used].sort().map(name => {
    const size = data[name].heights['16'];
    // Octicons ship `<path d="..." fill-rule="...">` markup; the window builds SVG nodes, so keep the attributes.
    const paths = [...size.path.matchAll(/<path ([^>]*?)\/?>/g)].map(([, attributes]) => Object.fromEntries([...attributes.matchAll(/([\w-]+)="([^"]*)"/g)].map(([, key, value]) => [key, value])));
    return [name, { width: size.width, height: 16, paths }];
  }));
  return `export const icons = ${JSON.stringify(icons)};\n`;
}

async function view() {
  const scratch = `${output}-build`;
  await mkdir(scratch, { recursive: true });
  await writeFile(join(scratch, 'icons.mjs'), await octicons());
  const bundled = await build({
    entryPoints: [join(root, 'view/index.mjs')], bundle: true, write: false, format: 'esm', target: 'es2022', minify: true,
    legalComments: 'inline', alias: { 'octicons-subset': join(scratch, 'icons.mjs') }, logLevel: 'warning',
  });
  const script = bundled.outputFiles[0].text;
  const themes = ['light', 'dark'].map(name => join(modules, `primitives/dist/css/functional/themes/${name}.css`));
  const sheets = [...themes, join(modules, 'css/dist/primer.css'), join(root, 'view/style.css')];
  const css = (await Promise.all(sheets.map(file => readFile(file, 'utf8')))).join('\n');
  const style = (await transform(css, { loader: 'css', minify: true, legalComments: 'none' })).code;
  for (const match of style.matchAll(/url\(([^)]*)\)/g)) {
    if (!/^(?:data:|#)/i.test(match[1].trim().replace(/^["']/, ''))) throw Error(`The window's CSS names an external asset: ${match[1]}`);
  }
  if (/\bimport\s*\(/.test(script)) throw Error('The window kept a dynamic import; bundle it statically');
  await writeFile(join(scratch, 'app.js'), script);
  await writeFile(join(scratch, 'app.css'), style);
  return encodeView({ schema: 'chariox.app-view-source.v1', rootId: 'github', script, style });
}

await rm(output, { recursive: true, force: true });
await rm(`${output}-build`, { recursive: true, force: true });
await mkdir(join(output, 'bundle/ui'), { recursive: true });
await cp(join(root, 'app.json'), join(output, 'app.json'));
await cp(join(root, 'app.ts'), join(output, 'app.ts'));
await cp(join(root, 'src'), join(output, 'src'), { recursive: true });
await cp(join(root, 'bundle/schemas'), join(output, 'bundle/schemas'), { recursive: true });
await cp(join(root, 'ui/icon.svg'), join(output, 'bundle/ui/icon.svg'));
await writeFile(join(output, 'bundle/ui/index.html'), await view());
await mkdir(join(output, 'bundle/licenses'), { recursive: true });
for (const [name, directory] of [['primer-css', 'css'], ['primer-octicons', 'octicons'], ['primer-primitives', 'primitives']]) {
  await cp(join(modules, directory, 'LICENSE'), join(output, 'bundle/licenses', `${name}.LICENSE`));
}
await cp(join(root, 'THIRD-PARTY-NOTICES.md'), join(output, 'bundle/licenses/THIRD-PARTY-NOTICES.md'));
console.log(`built ${output} (${(await stat(join(output, 'bundle/ui/index.html'))).size} byte view)`);
