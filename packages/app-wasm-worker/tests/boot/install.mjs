// MP-08 / MP-09 / MP-10 / MP-11: real boot packager, real authenticated boot installer, real WASM worker.
import { execFileSync } from 'node:child_process';
import { generateKeyPairSync } from 'node:crypto';
import { chmod, copyFile, mkdir, readFile, readlink, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { installBootRelease } from '../../../../deploy/runtime-release/boot-release.mjs';

const [root, worker] = process.argv.slice(2);
const sourceRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../../../..');
const put = async (path, bytes, mode = 0o755) => { await mkdir(dirname(path), { recursive: true }); await writeFile(path, bytes); await chmod(path, mode); };
const inputs = join(root, 'inputs');
await put(join(inputs, 'chariox-kernel'), '#!/bin/sh\nexit 0\n');
await put(join(inputs, 'chariox-managed-bootstrap'), '#!/bin/sh\nexit 0\n');
await put(join(inputs, 'chariox-relay'), '#!/bin/sh\nexit 0\n');
await copyFile(worker, join(inputs, 'chariox-app-wasm-worker'));
const { privateKey, publicKey } = generateKeyPairSync('ed25519');
await put(join(inputs, 'signing-key.pem'), privateKey.export({ type: 'pkcs8', format: 'pem' }), 0o600);
const source = join(root, 'source');
await put(join(source, 'README'), 'release source\n', 0o644);
for (const args of [['init', '-q'], ['add', '.'], ['-c', 'user.name=t', '-c', 'user.email=t@example.invalid', 'commit', '-qm', 'source']])
  execFileSync('git', ['-C', source, ...args]);
const output = join(root, 'release');
const packaged = JSON.parse(execFileSync('node', [join(sourceRoot, 'scripts/package-runtime-release.mjs'),
  '--enrollment-observer', join(inputs, 'chariox-managed-bootstrap'), '--bootstrap-unit', join(inputs, 'chariox-managed-bootstrap'),
  '--kernel', join(inputs, 'chariox-kernel'), '--bootstrap', join(inputs, 'chariox-managed-bootstrap'),
  '--worker', join(inputs, 'chariox-app-wasm-worker'), '--relay', join(inputs, 'chariox-relay'),
  '--signing-key', join(inputs, 'signing-key.pem'), '--source', source, '--output', output], { encoding: 'utf8' }));
const image = join(root, 'image');
await put(join(image, 'etc/chariox/boot/release-public-key'), publicKey.export({ type: 'spki', format: 'der' }).subarray(-32).toString('base64'), 0o644);
const fetcher = async url => {
  const file = join(output, url.pathname.replace('/immutable/', ''));
  return readFile(file).then(bytes => new Response(bytes), () => new Response(null, { status: 404 }));
};
await installBootRelease({ url: 'https://releases.example/immutable/', digest: packaged.digest, root: image, fetcher, activate: async () => {} });
// The kernel resolves its own executable, so the worker must sit beside the release's kernel.
process.stdout.write(join(image, 'usr/lib/chariox', await readlink(join(image, 'usr/lib/chariox/current')), 'usr/local/bin/chariox-app-wasm-worker'));
