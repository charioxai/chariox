import { readFile } from 'node:fs/promises';
import { join, resolve, sep } from 'node:path';

export async function npmPackages(inputs) {
  const packages = new Map();
  for (const input of Object.keys(inputs)) {
    const file = resolve(input);
    const parts = file.split(sep), index = parts.lastIndexOf('node_modules');
    if (index < 0) continue;
    const directory = parts.slice(0, index + (parts[index + 1].startsWith('@') ? 3 : 2)).join(sep);
    const metadata = JSON.parse(await readFile(join(directory, 'package.json'), 'utf8'));
    if (typeof metadata.name !== 'string' || typeof metadata.version !== 'string') throw Error(`npm package metadata lacks name/version: ${directory}`);
    const license = metadata.license && typeof metadata.license === 'object' ? metadata.license.type : metadata.license;
    const entry = packages.get(directory) ?? { name: metadata.name, version: metadata.version, license: license ?? null, directory, inputs: [] };
    entry.inputs.push(file);
    packages.set(directory, entry);
  }
  return [...packages.values()].sort((a, b) => a.name.localeCompare(b.name));
}
