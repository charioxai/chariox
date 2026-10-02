#!/usr/bin/env node
// Signed TEXT graph for installer mechanics. Never an executable App runtime.
import assert from 'node:assert/strict';
import { generateKeyPairSync, createHash, sign } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const [sourceArgument, outputArgument, evidenceArgument] = process.argv.slice(2);
assert.ok(sourceArgument && outputArgument && evidenceArgument && process.argv.length === 5,
  'usage: fixture.mjs CANDIDATE_SOURCE NEW_OUTPUT OWN_EVIDENCE');
const source = resolve(sourceArgument), output = resolve(outputArgument), evidence = resolve(evidenceArgument);
const sourceCommit = execFileSync('git', ['-C', source, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
const { releasePaths, executable } = await import(pathToFileURL(join(source, 'scripts/app-runtime-release-contract.mjs')));
const lock = JSON.parse(await readFile(join(source, 'apps/app-worker/bundle.lock.json')));
const runtime = JSON.parse(await readFile(join(source, 'apps/app-worker/runtime.lock.json')));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const paths = releasePaths({ target: 'linux-x64', files: [
  ...['CHARIOX-LICENSE', 'NODE-LICENSE', 'runtime.lock.json', 'bundle.lock.json',
    'native-artifact-manifest.json', ...lock.bootstrap, ...lock.sdkFiles.map(path => `sdk/${path}`),
    'libnode.so.137', 'libchariox-app-runtime.so'].map(path => ({ path })),
] });
await mkdir(output, { mode: 0o700 });
const { publicKey, privateKey } = generateKeyPairSync('ed25519');
const records = [];
for (const version of [1, 2]) {
  const directory = join(output, `v${version}`);
  await mkdir(directory, { mode: 0o700 });
  const files = [];
  for (const path of paths) {
    const bytes = Buffer.from(`B6 non-executable installer fixture v${version}: ${path}\n`);
    await mkdir(dirname(join(directory, path)), { recursive: true, mode: 0o755 });
    await writeFile(join(directory, path), bytes, { flag: 'wx', mode: executable(path) ? 0o555 : 0o444 });
    files.push({ path, size: bytes.length, sha256: hash(bytes), executable: executable(path) });
  }
  const bytes = Buffer.from(JSON.stringify({ schema: 'chariox.app-runtime-inventory.v1', target: 'linux-x64',
    runtimeVersion: runtime.runtimeVersion, workerAbi: runtime.workerAbi, nodeVersion: runtime.node.version,
    nodeModuleAbi: runtime.node.moduleAbi, sdkVersion: lock.sdk.version, sourceCommit, files }));
  await writeFile(join(directory, 'runtime-inventory.json'), bytes, { flag: 'wx', mode: 0o444 });
  await writeFile(join(directory, 'runtime-inventory.sig'), sign(null, bytes, privateKey).toString('hex'),
    { flag: 'wx', mode: 0o444 });
  await writeFile(join(directory, '.runtime-lease'), '', { flag: 'wx', mode: 0o444 });
  records.push({ version, inventorySha256: hash(bytes), files: files.length });
}
// The signing key exists only in memory. Persist public trust separately.
await writeFile(join(evidence, 'installer-fixture.json'), JSON.stringify({
  qualification: 'non-executable signed text graph; installer mechanics only', sourceCommit,
  publicKeyHex: publicKey.export({ type: 'spki', format: 'der' }).subarray(12).toString('hex'), records,
}, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
