import { createHash } from 'node:crypto';
import { readdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = fileURLToPath(new URL('..', import.meta.url));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const allowed = new Set(['MIT', 'Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause', 'ISC', '0BSD']);
const noticeName = /^(?:licen[cs]e|copying|notice|copyright)(?:[.-]|$)/i;

async function files(directory, prefix = '') {
  const result = [];
  for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name, 'en'))) {
    if (entry.name === 'node_modules') continue;
    const relative = prefix + entry.name;
    if (entry.isDirectory()) result.push(...await files(path.join(directory, entry.name), relative + '/'));
    else if (entry.isFile()) result.push(relative);
    else throw new Error(`Review dependency symlink: ${directory}/${relative}`);
  }
  return result;
}

export async function installedPackages(directory = root) {
  const store = path.join(directory, 'node_modules/.pnpm'), packages = new Map();
  const supplemental = JSON.parse(await readFile(path.join(directory, 'licenses/supplemental.json'), 'utf8'));
  for (const entry of await readdir(store, { withFileTypes: true })) {
    if (!entry.isDirectory() || entry.name === 'node_modules') continue;
    const modules = path.join(store, entry.name, 'node_modules');
    for (const child of await readdir(modules, { withFileTypes: true })) {
      if (!child.isDirectory()) continue;
      const base = path.join(modules, child.name);
      const roots = child.name.startsWith('@')
        ? (await readdir(base, { withFileTypes: true })).filter(e => e.isDirectory()).map(e => path.join(base, e.name)) : [base];
      for (const location of roots) {
        const metadata = JSON.parse(await readFile(path.join(location, 'package.json'), 'utf8'));
        const id = `${metadata.name}@${metadata.version}`;
        if (metadata.name.startsWith('@univerjs-pro/') || metadata.name.includes('univer-pro')) throw new Error(`Pro package forbidden: ${id}`);
        if (!allowed.has(metadata.license)) throw new Error(`Review dependency licence: ${id} (${metadata.license})`);
        const names = await files(location), digest = createHash('sha256');
        for (const name of names) { digest.update(name + '\0'); digest.update(await readFile(path.join(location, name))); }
        const notices = names.filter(name => noticeName.test(path.basename(name)));
        for (const name of names.filter(name => /^readme(?:\.md)?$/i.test(name))) {
          const text = await readFile(path.join(location, name), 'utf8');
          if (/^#+ licen[cs]e\s*$/im.test(text) && /Permission (?:is hereby granted|to use, copy)/.test(text)) notices.push(name);
        }
        const extra = supplemental[id];
        if (extra) {
          const text = await readFile(path.join(directory, 'licenses', extra.path));
          if (hash(text) !== extra.sha256 || !/^https:\/\/raw\.githubusercontent\.com\/[^/]+\/[^/]+\/[0-9a-f]{40}\//.test(extra.source)) throw new Error(`Unpinned supplemental notice: ${id}`);
        }
        if (!notices.length && !extra) throw new Error(`Missing licence text: ${id}`);
        const record = { id, license: metadata.license, sha256: digest.digest('hex'), notices,
          assets: names.filter(name => /\.(?:woff2?|ttf|otf|png|jpe?g|gif|svg|wasm)$/i.test(name)),
          ...(extra ? { supplemental: extra } : {}) };
        const previous = packages.get(id);
        if (previous && JSON.stringify(previous.record) !== JSON.stringify(record)) throw new Error(`Conflicting installed copies: ${id}`);
        packages.set(id, { record, location });
      }
    }
  }
  return [...packages.values()].sort((a, b) => a.record.id.localeCompare(b.record.id, 'en'));
}

export async function inventory(directory = root) {
  const lock = await readFile(path.join(directory, 'pnpm-lock.yaml'));
  if (/@univerjs-pro[+/]|univer-pro/i.test(lock.toString())) throw new Error('Pro package forbidden in lockfile');
  const manifest = JSON.parse(await readFile(path.join(directory, 'package.json'), 'utf8'));
  for (const [name, version] of Object.entries({ ...manifest.dependencies, ...manifest.devDependencies })) {
    if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`Pin exact dependency: ${name}`);
  }
  const packages = await installedPackages(directory);
  return { format: 1, nodeLockSha256: hash(lock), backendLockSha256: hash(await readFile(path.join(directory, 'backend/Cargo.lock'))),
    packages: packages.map(p => p.record) };
}

export async function checkLicenses(directory = root) {
  const expected = JSON.parse(await readFile(path.join(directory, 'licenses.json'), 'utf8'));
  const actual = await inventory(directory);
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error('Unlisted or changed dependency, asset or lockfile; review and update licenses.json explicitly');
  return actual;
}

export async function notices(directory = root) {
  const sections = ['Chariox Univer App\n' + await readFile(path.join(directory, 'LICENSE'), 'utf8')];
  for (const { record, location } of await installedPackages(directory)) {
    for (const file of record.notices) sections.push(`${record.id} / ${file}\n${await readFile(path.join(location, file), 'utf8')}`);
    if (record.supplemental) sections.push(`${record.id}\nSource: ${record.supplemental.source}\n${await readFile(path.join(directory, 'licenses', record.supplemental.path), 'utf8')}`);
  }
  return sections.join('\n\n------------------------------\n\n');
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.includes('--record-reviewed-inventory')) {
    await writeFile(path.join(root, 'licenses.json'), JSON.stringify(await inventory(), null, 2) + '\n');
  } else {
    const checked = await checkLicenses();
    console.log(`Checked ${checked.packages.length} pinned dependencies; no Pro or unlisted code/assets`);
  }
}
