import { createHash } from 'node:crypto';
import { readFile, readdir } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const json = async file => JSON.parse(await readFile(file, 'utf8'));

export async function files(directory, prefix = '') {
  const result = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (entry.isDirectory()) result.push(...await files(join(directory, entry.name), prefix + entry.name + '/'));
    else if (entry.isFile()) result.push(prefix + entry.name);
  }
  return result.sort();
}

export async function inspectInventory() {
  const supplemental = await json(join(root, 'licenses/supplemental.json'));
  const packages = {};
  const store = join(root, 'node_modules/.pnpm');
  for (const entry of await readdir(store, { withFileTypes: true })) {
    if (!entry.isDirectory() || entry.name === 'node_modules') continue;
    const directory = join(store, entry.name, 'node_modules');
    for (const child of await readdir(directory, { withFileTypes: true })) {
      if (!child.isDirectory()) continue;
      const children = child.name.startsWith('@')
        ? (await readdir(join(directory, child.name), { withFileTypes: true })).filter(c => c.isDirectory()).map(c => join(directory, child.name, c.name))
        : [join(directory, child.name)];
      for (const path of children) {
        const metadata = await json(join(path, 'package.json'));
        const id = metadata.name + '@' + metadata.version;
        const notices = {};
        for (const name of (await readdir(path)).filter(name => /^(licen[cs]e|notice|copying|copyright)/i.test(name)).sort()) {
          notices[name] = sha256(await readFile(join(path, name)));
        }
        if (!Object.keys(notices).length) {
          const source = supplemental[id];
          if (!source) throw Error(`missing licence text: ${id}`);
          const digest = sha256(await readFile(join(root, source.path)));
          if (digest !== source.sha256) throw Error(`changed supplemental notice: ${id}`);
          notices[source.path] = digest;
        }
        const license = metadata.license ?? metadata.licenses?.[0]?.type ?? (metadata.name === 'khroma' ? 'MIT' : null);
        packages[id] = { license, metadata: sha256(await readFile(join(path, 'package.json'))), notices };
      }
    }
  }
  const prod = join(root, 'node_modules/@excalidraw/excalidraw/dist/prod');
  const assets = {};
  for (const file of await files(prod)) assets[file] = sha256(await readFile(join(prod, file)));
  const retained = {};
  for (const file of await files(join(root, 'licenses'))) retained[file] = sha256(await readFile(join(root, 'licenses', file)));
  return {
    lock: sha256(await readFile(join(root, 'pnpm-lock.yaml'))), packages, assets, retained,
    backendLock: sha256(await readFile(join(root, 'backend/Cargo.lock'))),
    replacementLiberation: sha256(await readFile(join(root, 'assets/LiberationSans-Regular.woff2'))),
  };
}

export function verifyInventory(actual, approved) {
  if (actual.lock !== approved.lock) throw Error('dependency lock changed; review the complete licence inventory');
  if (actual.backendLock !== approved.backendLock) throw Error('backend dependency lock changed; review the component licence inventory');
  for (const [id, entry] of Object.entries(actual.packages)) {
    if (!approved.packages[id]) throw Error(`unlisted dependency: ${id}`);
    if (JSON.stringify(entry) !== JSON.stringify(approved.packages[id])) throw Error(`unreviewed dependency: ${id}`);
    if (!['MIT', 'ISC', 'BSD-3-Clause', 'Apache-2.0', '0BSD', 'Unlicense', '(MIT AND Zlib)', 'CC0-1.0', '(MPL-2.0 OR Apache-2.0)'].includes(entry.license)) {
      throw Error(`licence not permitted: ${id}: ${entry.license}`);
    }
  }
  for (const field of ['assets', 'retained']) {
    for (const key of new Set([...Object.keys(actual[field]), ...Object.keys(approved[field])])) {
      if (actual[field][key] !== approved[field][key]) throw Error(`unreviewed asset: ${field}/${key}`);
    }
  }
  if (actual.replacementLiberation !== approved.replacementLiberation) throw Error('unreviewed Liberation replacement');
}

export async function checkLicenses() {
  const actual = await inspectInventory();
  verifyInventory(actual, await json(join(root, 'licenses.json')));
  return actual;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const inventory = await checkLicenses();
  console.log(`Reviewed ${Object.keys(inventory.packages).length} dependencies and ${Object.keys(inventory.assets).length} upstream assets.`);
}
