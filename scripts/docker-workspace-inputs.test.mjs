import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const root = new URL('../', import.meta.url);
const cargo = await readFile(new URL('Cargo.toml', root), 'utf8');
const members = [...cargo.match(/members\s*=\s*\[([\s\S]*?)\]/)[1].matchAll(/"([^"]+)"/g)].map(match => match[1]);
const covers = (sources, member) => sources.some(source => source === member || member.startsWith(`${source}/`));

for (const file of ['docker/publication/Dockerfile', 'apps/kernel/slice-linux-docker/docker/Dockerfile']) {
  test(`${file} includes every Rust workspace member in its build stage`, async () => {
    const dockerfile = await readFile(new URL(file, root), 'utf8');
    const builder = dockerfile.split(/^FROM /m).find(stage => /^[^\n]+ AS rust-builder\s*\n/i.test(stage));
    assert.ok(builder, 'Rust build stage exists');
    const sources = [...builder.matchAll(/^COPY\s+(\S+)\s+\S+\s*$/gm)].map(match => match[1]);
    assert.deepEqual(members.filter(member => !covers(sources, member)), [], 'workspace manifests must exist before Cargo loads the workspace');
  });
}

test('slice runtime source inputs include every Rust workspace member', async () => {
  const inputs = (await readFile(new URL('apps/kernel/slice-linux-docker/runtime-source-roots.txt', root), 'utf8')).split(/\r?\n/).filter(line => line && !line.startsWith('#'));
  assert.deepEqual(members.filter(member => !covers(inputs, member)), [], 'source-built slices need every workspace manifest');
});
