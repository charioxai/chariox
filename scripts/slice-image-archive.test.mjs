import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmod, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { gunzipSync } from 'node:zlib';

import { ARCHIVE, DESCRIPTOR, SCHEMA, exportSliceImage, validateSliceImage } from './slice-image-archive.mjs';

const revision = `sha256:${'e'.repeat(64)}`;
const layers = [`sha256:${'1'.repeat(64)}`, `sha256:${'2'.repeat(64)}`];
const saved = Buffer.from('docker save output\n'.repeat(5000));

// A Docker that knows one image: inspect reports its labels and layers, save streams fixed bytes.
async function fakeDocker(root, labels = { 'io.chariox.runtime-source-revision': revision, 'io.chariox.relay-peer-protocol-version': '128' }) {
  await writeFile(join(root, 'saved.bin'), saved);
  await writeFile(join(root, 'docker'), `#!/usr/bin/env node
const args = process.argv.slice(2);
if (args[0] === 'image' && args[1] === 'inspect') console.log(JSON.stringify([{ Architecture: 'amd64', Config: { Labels: ${JSON.stringify(labels)} }, RootFS: { Layers: ${JSON.stringify(layers)} } }]));
else if (args[0] === 'tag') {}
else if (args[0] === 'save') process.stdout.write(require('node:fs').readFileSync(${JSON.stringify(join(root, 'saved.bin'))}));
else { console.error('unexpected ' + args.join(' ')); process.exit(2); }
`);
  await chmod(join(root, 'docker'), 0o755);
  process.env.PATH = `${root}:${process.env.PATH.split(':').filter(entry => !entry.includes('chariox-slice-image-')).join(':')}`;
}

test('export writes a gzip docker archive and a descriptor the verifier accepts', async (context) => {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'chariox-slice-image-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  await fakeDocker(root);
  const output = join(root, 'out');
  const result = await exportSliceImage('chariox-slice-linux:0.1.0', output);
  const archive = await readFile(join(output, ARCHIVE));
  assert.deepEqual(gunzipSync(archive), saved);
  const descriptor = JSON.parse(await readFile(join(output, DESCRIPTOR), 'utf8'));
  assert.deepEqual(descriptor, { schema: SCHEMA, architecture: 'amd64', tag: `chariox-slice-release:${'e'.repeat(64)}`, runtimeSourceRevision: revision,
    relayPeerProtocolVersion: '128', rootfsLayers: layers, archive: { name: ARCHIVE, size: archive.length, sha256: createHash('sha256').update(archive).digest('hex') } });
  assert.equal(result.sha256, descriptor.archive.sha256);
  assert.equal((await validateSliceImage(output)).tag, descriptor.tag);

  await writeFile(join(output, ARCHIVE), Buffer.concat([archive.subarray(0, -1), Buffer.from('X')]));
  await assert.rejects(validateSliceImage(output), /does not match the SHA-256/);
  await writeFile(join(output, ARCHIVE), archive.subarray(0, -1));
  await assert.rejects(validateSliceImage(output), /is not the \d+ byte file/);
  await writeFile(join(output, ARCHIVE), archive);
  await writeFile(join(output, 'notes.txt'), 'x');
  await assert.rejects(validateSliceImage(output), /unexpected files/);
});

test('export refuses an image the provisioner did not build', async (context) => {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'chariox-slice-image-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  await fakeDocker(root, { 'io.chariox.runtime-source-revision': 'unknown', 'io.chariox.relay-peer-protocol-version': '128' });
  await assert.rejects(exportSliceImage('chariox-slice-linux:0.1.0', join(root, 'out')), /no runtime source revision label/);
});
