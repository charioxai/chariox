// MP-08 / MP-10 / MP-11: real packager, verified Setup install, real WASM worker.
import { execFileSync } from 'node:child_process';
import { generateKeyPairSync, sign } from 'node:crypto';
import { chmod, copyFile, mkdir, readFile, readlink, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assemble, SLICE_PROVISIONER } from '../../../../scripts/release-bundle.mjs';
import { SLICE_DOCKERFILE } from '../../../../scripts/slice-build-context.mjs';
import { installLocal } from '../../../../apps/setup/installer.mjs';

const [root, worker] = process.argv.slice(2);
const sourceRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../../../..');
async function put(path, bytes, mode = 0o644) {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, bytes);
  await chmod(path, mode);
}
const executables = join(root, 'executables');
for (const name of ['chariox', 'chariox-kernel', 'chariox-relay', 'chariox-app-package', 'chariox-setup'])
  await put(join(executables, 'bin', name), '#!/bin/sh\necho 967\n', 0o755);
await copyFile(worker, join(executables, 'bin/chariox-app-wasm-worker'));
await chmod(join(executables, 'bin/chariox-app-wasm-worker'), 0o755);
const sliceBuildContext = join(root, 'slice-context');
await put(join(sliceBuildContext, 'apps/kernel/slice-linux-docker/runtime-source-roots.txt'), 'apps/kernel\n');
await put(join(sliceBuildContext, SLICE_PROVISIONER), '#!/bin/sh\n', 0o755);
await put(join(sliceBuildContext, SLICE_DOCKERFILE), 'FROM scratch\nCOPY apps/kernel /kernel\n');
const version = '0.3.0', platform = `${process.platform}-${process.arch}`;
const name = `chariox-${version}-${platform}`, bundle = join(root, name);
const keys = generateKeyPairSync('ed25519');
const options = { platform, version, sourceCommit: 'a'.repeat(40), executables, sliceBuildContext, sourceRoot, output: bundle };
if (platform === 'darwin-arm64') {
  const { createHash } = await import('node:crypto');
  const files = [];
  for (const name of ['chariox', 'chariox-kernel', 'chariox-relay', 'chariox-app-package', 'chariox-setup', 'chariox-app-wasm-worker']) {
    const path = `bin/${name}`;
    files.push({ path, role: 'executable', sha256: createHash('sha256').update(await readFile(join(executables, path))).digest('hex') });
  }
  options.macosSigningReceipt = join(root, 'receipt.json');
  await writeFile(options.macosSigningReceipt, JSON.stringify({ schema: 'chariox.macos-release-signing.v1', identity: 'Developer ID Application: Fixture', files, notarization: { status: 'Accepted' } }));
}
await assemble(options);
const manifest = await readFile(join(bundle, 'manifest.json'));
const signature = sign(null, manifest, keys.privateKey).toString('hex');
await writeFile(join(bundle, 'manifest.sig'), signature);
const archive = join(root, 'release.tar.gz');
execFileSync('tar', ['-czf', archive, '-C', root, name]);
const responses = new Map([['manifest.json', manifest], ['manifest.sig', signature], ['tar.gz', await readFile(archive)]]);
const server = createServer((req, res) => {
  const suffix = req.url.slice(`/v${version}/${name}.`.length);
  if (!responses.has(suffix)) { res.writeHead(404); res.end(); return; }
  res.end(responses.get(suffix));
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
try {
  const home = join(root, 'home'); await mkdir(home);
  await installLocal({ home, version, platform, installOnly: true,
    publicKeyHex: Buffer.from(keys.publicKey.export({ format: 'jwk' }).x, 'base64url').toString('hex'),
    releaseBase: `http://127.0.0.1:${server.address().port}`, apiUrl: 'http://127.0.0.1:1',
    extractorSource: await readFile(join(sourceRoot, 'deploy/runtime-release/extract-release.py'), 'utf8'),
    servicePersistence: async () => {}, serviceManager: async () => 'LoadState=not-found\nFragmentPath=\nDropInPaths=\nActiveState=inactive\nUnitFileState=disabled\n',
  });
  const install = join(home, '.local/share/chariox/machines/local');
  process.stdout.write(join(install, await readlink(join(install, 'current')), 'bin/chariox-app-wasm-worker'));
} finally {
  await new Promise(resolve => server.close(resolve));
}
