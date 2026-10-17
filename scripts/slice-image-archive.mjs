#!/usr/bin/env node
// The prebuilt slice image of a release (docs/CHARIOX_DISTRIBUTION_PLAN.md).
//
//   slice-image-archive.mjs export --image TAG --output DIR
//
// Runs on the CI builder after provision-linux-docker-slice.sh build-image built
// TAG from the exported slice build context. It writes DIR/chariox-slice-linux.tar.gz
// (docker save, gzip) and DIR/slice-image.json: the image's runtime-source and
// protocol labels, its root filesystem layers and the archive's size and SHA-256.
// release-bundle.mjs assemble ships both inside the signed manifest; the installed
// provisioner verifies them again before `docker load` (released-slice-image.py).
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream, createWriteStream } from 'node:fs';
import { lstat, mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { pipeline } from 'node:stream/promises';
import { pathToFileURL } from 'node:url';
import { createGzip } from 'node:zlib';

export const SCHEMA = 'chariox.slice-image.v1';
export const DESCRIPTOR = 'slice-image.json';
export const ARCHIVE = 'chariox-slice-linux.tar.gz';
const RELEASE_TAG = /^chariox-slice-release:[A-Za-z0-9_.-]{1,100}$/;
const ARCHITECTURES = new Set(['amd64', 'arm64']);

const fail = message => { throw new Error(message); };

function docker(args) {
  const result = spawnSync('docker', args, { encoding: 'utf8', maxBuffer: 64 << 20 });
  if (result.status !== 0) fail(`docker ${args[0]} failed: ${result.stderr?.trim() || result.error?.message}`);
  return result.stdout;
}

/** Reads what the provisioner compares against: the labels, architecture and layers of a built image. */
function inspectImage(image) {
  const [inspected] = JSON.parse(docker(['image', 'inspect', image]));
  const labels = inspected.Config?.Labels ?? {};
  const revision = labels['io.chariox.runtime-source-revision'];
  const protocol = labels['io.chariox.relay-peer-protocol-version'];
  if (!revision || revision === 'unknown') fail(`${image} has no runtime source revision label; build it with provision-linux-docker-slice.sh build-image`);
  if (!/^\d+$/.test(protocol ?? '')) fail(`${image} has no relay peer protocol version label`);
  if (!ARCHITECTURES.has(inspected.Architecture)) fail(`${image} has unsupported architecture ${inspected.Architecture}`);
  return { revision, protocol, architecture: inspected.Architecture, layers: inspected.RootFS?.Layers ?? [] };
}

export async function exportSliceImage(image, output) {
  const target = resolve(output ?? fail('--output is required'));
  const { revision, protocol, architecture, layers } = inspectImage(image ?? fail('--image is required'));
  const tag = `chariox-slice-release:${revision.replace(/^sha256:/, '')}`;
  if (!RELEASE_TAG.test(tag)) fail(`runtime source revision ${revision} cannot name a release tag`);
  docker(['tag', image, tag]);
  await mkdir(target, { mode: 0o755 });
  const hash = createHash('sha256');
  let size = 0;
  const save = spawn('docker', ['save', tag], { stdio: ['ignore', 'pipe', 'inherit'] });
  const exited = new Promise((done, failed) => {
    save.on('error', failed);
    save.on('close', code => (code === 0 ? done() : failed(new Error(`docker save exited with ${code}`))));
  });
  const compressed = createGzip({ level: 9 });
  compressed.on('data', chunk => { hash.update(chunk); size += chunk.length; });
  await Promise.all([pipeline(save.stdout, compressed, createWriteStream(join(target, ARCHIVE), { mode: 0o644 })), exited]);
  const descriptor = { schema: SCHEMA, architecture, tag, runtimeSourceRevision: revision, relayPeerProtocolVersion: protocol,
    rootfsLayers: layers, archive: { name: ARCHIVE, size, sha256: hash.digest('hex') } };
  await writeFile(join(target, DESCRIPTOR), `${JSON.stringify(descriptor, null, 2)}\n`, { mode: 0o644 });
  return { output: target, tag, architecture, size, sha256: descriptor.archive.sha256 };
}

/** The descriptor and archive of one directory agree: exactly the two files, the archive's size and SHA-256 as listed. */
export async function validateSliceImage(directory) {
  const descriptorBytes = await readFile(join(directory, DESCRIPTOR)).catch(() => fail(`slice image: ${DESCRIPTOR} is missing in ${directory}`));
  const descriptor = JSON.parse(descriptorBytes.toString('utf8'));
  const { archive, rootfsLayers } = descriptor;
  const valid = descriptor.schema === SCHEMA && ARCHITECTURES.has(descriptor.architecture) && RELEASE_TAG.test(descriptor.tag ?? '')
    && /^\d+$/.test(descriptor.relayPeerProtocolVersion ?? '') && typeof descriptor.runtimeSourceRevision === 'string'
    && Array.isArray(rootfsLayers) && rootfsLayers.length > 0 && rootfsLayers.every(layer => /^sha256:[a-f0-9]{64}$/.test(layer))
    && archive?.name === ARCHIVE && Number.isSafeInteger(archive.size) && archive.size > 0 && /^[a-f0-9]{64}$/.test(archive.sha256 ?? '');
  if (!valid) fail(`slice image: ${DESCRIPTOR} is not a ${SCHEMA} descriptor`);
  const metadata = await lstat(join(directory, ARCHIVE)).catch(() => fail(`slice image: ${ARCHIVE} is missing in ${directory}`));
  if (!metadata.isFile() || metadata.size !== archive.size) fail(`slice image: ${ARCHIVE} is not the ${archive.size} byte file the descriptor lists`);
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(join(directory, ARCHIVE))) hash.update(chunk);
  if (hash.digest('hex') !== archive.sha256) fail(`slice image: ${ARCHIVE} does not match the SHA-256 in ${DESCRIPTOR}`);
  const extra = (await readdir(directory)).filter(name => name !== DESCRIPTOR && name !== ARCHIVE);
  if (extra.length) fail(`slice image: unexpected files in ${directory}: ${extra.join(', ')}`);
  return descriptor;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const [command, ...rest] = process.argv.slice(2);
    const options = Object.fromEntries(rest.map((value, index) => [value, rest[index + 1]]).filter((_pair, index) => index % 2 === 0));
    if (command !== 'export' || Object.keys(options).some(name => !['--image', '--output'].includes(name)))
      fail('usage: slice-image-archive.mjs export --image TAG --output DIR');
    process.stdout.write(`${JSON.stringify(await exportSliceImage(options['--image'], options['--output']), null, 2)}\n`);
  } catch (error) {
    process.stderr.write(`slice-image-archive: ${error.message}\n`);
    process.exit(1);
  }
}
