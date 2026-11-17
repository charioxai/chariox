// MP-08/09/10/11 AV1: turn a reference build into one signed view. Keys are
// ephemeral and remain in memory; these output trust files are drill pins only.
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, join, isAbsolute, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { packView } from './pack.mjs';
import { canonical } from './bundle.mjs';

const repositoryRoot = fileURLToPath(new URL('../../', import.meta.url));

export async function referenceSource(name, buildDirectory) {
  let script = await readFile(join(buildDirectory, 'ui/app.js'), 'utf8');
  let style = await readFile(join(buildDirectory, 'ui/app.css'), 'utf8');
  // Upstream's generated demo utility is unused by the reference App. Never
  // carry its example.com background URL into an offline view.
  if (name === 'univer') style = style.replace(/[^{}]*\{[^{}]*url\(["']?https:\/\/example\.com\/a-b\.svg["']?\)[^{}]*\}/g, '');
  if (name === 'excalidraw') {
    // The pinned library normally derives these font URLs at runtime. Resolve
    // each literal to packaged bytes before signing, with no CDN fallback.
    const fonts = [...script.matchAll(/"(\.\/fonts\/[^"\\]+\.woff2)"/g)].map(match => match[1]);
    if (!fonts.length) throw new Error('Pinned Excalidraw font paths changed; review the client packer');
    for (const path of new Set(fonts)) {
      const bytes = await readFile(join(buildDirectory, 'ui', path));
      script = script.replaceAll(JSON.stringify(path), JSON.stringify('data:font/woff2;base64,' + bytes.toString('base64')));
    }
    script = 'window.EXCALIDRAW_ASSET_PATH="https://app-assets.invalid/";\n' + script;
  }
  const metadata = JSON.parse(await readFile(join(buildDirectory, name === 'excalidraw' ? 'view-build.json' : 'ui-build.json'), 'utf8'));
  for (const output of Object.values(metadata.outputs)) {
    if (output.imports.some(item => item.kind !== 'url-token' && (item.external || item.kind === 'dynamic-import' || item.path.endsWith('.js')))) throw new Error('App view contains an unresolved module dependency');
  }
  if (/\bimport\s*\(/.test(script)) throw new Error('App view retained a dynamic import; bundle it statically');
  for (const match of style.matchAll(/url\(([^)]*)\)/g)) {
    if (!/^(?:data:|#)/i.test(match[1].trim().replace(/^["']/, ''))) throw new Error('App CSS retained an external asset');
  }
  return { schema: 'chariox.app-view-source.v1', rootId: name === 'excalidraw' ? 'root' : 'app', script, style };
}

export async function signReference(name, buildDirectory, output, adapt = script => script) {
  const manifest = JSON.parse(await readFile(join(repositoryRoot, 'examples/apps', name, 'app.json'), 'utf8'));
  const source = await referenceSource(name, buildDirectory);
  source.script = adapt(source.script);
  const key = await crypto.subtle.generateKey('Ed25519', true, ['sign', 'verify']);
  const signed = await packView(manifest, source, key);
  await mkdir(output, { recursive: true });
  await writeFile(join(output, name + '.view.json'), canonical(signed.bundle));
  await writeFile(join(output, name + '.trust.json'), canonical(signed.trust));
  return signed;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [name, input, output] = process.argv.slice(2);
  if (!['excalidraw', 'univer'].includes(name) || !input || !output || !isAbsolute(output)
      || resolve(output) === resolve(repositoryRoot)
      || resolve(output).startsWith(repositoryRoot.endsWith(sep) ? repositoryRoot : repositoryRoot + sep)) throw new Error('usage: build-reference.mjs <excalidraw|univer> <built view directory> <absolute output outside checkout>');
  await signReference(name, resolve(input), resolve(output));
}
