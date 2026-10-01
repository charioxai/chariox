import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash, createPrivateKey, generateKeyPairSync, sign } from 'node:crypto';
import { chmod, copyFile, mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { runtimeReleaseFixture } from './app-runtime-release-fixture.mjs';
import { assemble, parseArgs, signBundle, verifyBundle } from './release-bundle.mjs';
import { signRuntimeRelease } from './sign-app-runtime-release.mjs';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const commit = 'a'.repeat(40);
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const keyHex = key => Buffer.from(key.export({ format: 'jwk' }).x, 'base64url').toString('hex');

async function put(path, contents, mode = 0o644) {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, contents);
  await chmod(path, mode);
}

// Built executables, the repository files a bundle carries and a release key.
async function bundleInputs(root, target) {
  const executables = join(root, 'executables');
  for (const name of ['chariox', 'chariox-kernel', 'chariox-relay', 'chariox-app-package'])
    await put(join(executables, 'bin', name), `#!/bin/sh\necho ${name}\n`, 0o755);
  const libexec = target === 'linux-x64' ? ['chariox-app-storage', 'chariox-app-runtime-install'] : ['chariox-app-runtime-install'];
  for (const name of libexec) await put(join(executables, 'libexec', name), `#!/bin/sh\necho ${name}\n`, 0o755);
  const source = join(root, 'source-root');
  for (const name of ['install-root.sh', 'install-user.sh', 'start-kernel.sh']) await put(join(source, 'deploy/local-linux', name), '#!/bin/sh\n', 0o755);
  for (const name of ['chariox-kernel.service', 'chariox-app-bwrap.apparmor']) await put(join(source, 'deploy/local-linux', name), name);
  await put(join(source, 'deploy/managed-kernel/chariox-app-storage.service'), '[Unit]\n');
  await mkdir(join(source, 'deploy/release-bundle'), { recursive: true });
  await copyFile(join(repository, 'deploy/release-bundle/install.sh'), join(source, 'deploy/release-bundle/install.sh'));
  await put(join(source, 'LICENSE'), 'license');
  const releaseKey = generateKeyPairSync('ed25519').privateKey;
  const releaseKeyPath = join(root, 'bundle-release.pem');
  await put(releaseKeyPath, releaseKey.export({ type: 'pkcs8', format: 'pem' }), 0o600);
  return { executables, sourceRoot: source, releaseKeyPath, releasePublic: keyHex(releaseKey) };
}

async function fixture(context, { target = 'linux-x64', runtimeCommit = commit } = {}) {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'chariox-release-bundle-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const inputs = await bundleInputs(root, target);
  // A signed App runtime release, as sign-app-runtime-release.mjs lays it out.
  const runtime = join(root, 'runtime');
  const payload = { 'chariox-app-worker': ['worker', 0o555], 'libnode.so.137': ['libnode', 0o444], 'sdk/src/index.js': ['export {}', 0o444] };
  const files = [];
  for (const [path, [contents, mode]] of Object.entries(payload)) {
    await put(join(runtime, path), contents, mode);
    files.push({ executable: (mode & 0o100) !== 0, path, sha256: sha256(Buffer.from(contents)), size: contents.length });
  }
  const runtimeKey = generateKeyPairSync('ed25519').privateKey;
  const inventory = Buffer.from(JSON.stringify({ files, schema: 'chariox.app-runtime-inventory.v1', sourceCommit: runtimeCommit, target }));
  await put(join(runtime, 'runtime-inventory.json'), inventory, 0o444);
  await put(join(runtime, 'runtime-inventory.sig'), sign(null, inventory, runtimeKey).toString('hex'), 0o444);
  await put(join(runtime, '.runtime-lease'), '', 0o444);
  const options = {
    platform: target, version: '0.2.0', sourceCommit: commit, executables: inputs.executables,
    runtime, runtimePublicKey: keyHex(runtimeKey), sourceRoot: inputs.sourceRoot, output: join(root, 'out', `chariox-0.2.0-${target}`),
  };
  return { root, options, runtimeKey, releaseKeyPath: inputs.releaseKeyPath, releasePublic: inputs.releasePublic };
}

test('a Linux bundle is assembled, signed and verified, and any change is refused', async (context) => {
  const { options, releaseKeyPath, releasePublic, runtimeKey } = await fixture(context);
  const assembled = await assemble(options);
  const manifest = JSON.parse(await readFile(join(options.output, 'manifest.json'), 'utf8'));
  assert.equal(manifest.schema, 'chariox.release-bundle.v1');
  assert.deepEqual([manifest.version, manifest.platform, manifest.sourceCommit], ['0.2.0', 'linux-x64', commit]);
  assert.deepEqual(manifest.runtime, { inventorySha256: sha256(await readFile(join(options.runtime, 'runtime-inventory.json'))), publicKeyHex: keyHex(runtimeKey) });
  assert.equal(manifest.macosSigning, null);
  assert.deepEqual(manifest.files.map(file => file.path), [
    'LICENSE', 'bin/chariox', 'bin/chariox-app-package', 'bin/chariox-kernel', 'bin/chariox-relay',
    'deploy/local-linux/chariox-app-bwrap.apparmor', 'deploy/local-linux/chariox-kernel.service', 'deploy/local-linux/install-root.sh',
    'deploy/local-linux/install-user.sh', 'deploy/local-linux/start-kernel.sh', 'deploy/managed-kernel/chariox-app-storage.service',
    'install.sh', 'libexec/chariox-app-runtime-install', 'libexec/chariox-app-storage',
    'runtime/.runtime-lease', 'runtime/chariox-app-worker', 'runtime/libnode.so.137', 'runtime/runtime-inventory.json',
    'runtime/runtime-inventory.sig', 'runtime/sdk/src/index.js',
  ]);
  const mode = path => manifest.files.find(file => file.path === path).mode;
  assert.deepEqual(['bin/chariox', 'install.sh', 'runtime/chariox-app-worker', 'runtime/libnode.so.137', 'LICENSE'].map(mode), ['0755', '0755', '0555', '0444', '0644']);
  assert.equal(assembled.files, manifest.files.length);

  await assert.rejects(verifyBundle(options.output, releasePublic), /manifest\.sig/);
  const signed = await signBundle(options.output, releaseKeyPath);
  assert.equal(signed.publicKeyHex, releasePublic);
  assert.equal(signed.manifestSha256, sha256(await readFile(join(options.output, 'manifest.json'))));
  await assert.rejects(signBundle(options.output, releaseKeyPath), /already signed/);
  assert.equal((await verifyBundle(options.output, releasePublic)).version, '0.2.0');
  await assert.rejects(verifyBundle(options.output, keyHex(generateKeyPairSync('ed25519').privateKey)), /does not verify/);

  const kernel = join(options.output, 'bin/chariox-kernel');
  const original = await readFile(kernel);
  await writeFile(kernel, 'tampered');
  await assert.rejects(verifyBundle(options.output, releasePublic), /bin\/chariox-kernel does not match/);
  await writeFile(kernel, original);
  await writeFile(join(options.output, 'bin/extra'), 'x');
  await assert.rejects(verifyBundle(options.output, releasePublic), /bin\/extra is not in the manifest/);
  await rm(join(options.output, 'bin/extra'));
  await rm(join(options.output, 'LICENSE'));
  await assert.rejects(verifyBundle(options.output, releasePublic), /missing LICENSE/);
});

test('assembly refuses inputs that are not the release it names', async (context) => {
  const { root, options } = await fixture(context);
  const refused = async (change, pattern) => {
    await assert.rejects(assemble({ ...options, ...change }), pattern);
    await assert.rejects(readFile(join(options.output, 'manifest.json')), /ENOENT/);
  };
  await refused({ runtimePublicKey: keyHex(generateKeyPairSync('ed25519').privateKey) }, /runtime inventory signature does not verify/);
  await refused({ sourceCommit: 'b'.repeat(40) }, /runtime source a+ is not b+/);
  await refused({ version: 'v0.2' }, /semantic version/);
  await refused({ platform: 'windows-x64' }, /--platform must be one of/);
  await refused({ macosSigningReceipt: join(root, 'receipt.json') }, /only to macOS bundles/);
  const darwin = await fixture(context, { target: 'darwin-arm64' });
  await refused({ runtime: darwin.options.runtime, runtimePublicKey: darwin.options.runtimePublicKey }, /runtime target darwin-arm64 is not linux-x64/);

  await writeFile(join(options.runtime, 'added.js'), 'x');
  await refused({}, /runtime file added\.js is not in the signed inventory/);
  await rm(join(options.runtime, 'added.js'));
  const executable = path => join(options.executables, path);
  await chmod(executable('bin/chariox-relay'), 0o644);
  await refused({}, /bin\/chariox-relay is not executable/);
  await chmod(executable('bin/chariox-relay'), 0o755);
  await writeFile(executable('bin/chariox-shell'), 'x');
  await refused({}, /does not ship: bin\/chariox-shell/);
  await rm(executable('bin/chariox-shell'));
  await rm(executable('libexec/chariox-app-storage'));
  await symlink(executable('bin/chariox-relay'), executable('libexec/chariox-app-storage'));
  await refused({}, /libexec\/chariox-app-storage must be a regular file/);
  await rm(executable('libexec/chariox-app-storage'));
  await refused({}, /libexec\/chariox-app-storage is missing/);

  await mkdir(options.output, { recursive: true });
  await assert.rejects(assemble(options), /already exists/);
});

test('a macOS bundle carries only executables the signing receipt codesigned and notarized', async (context) => {
  const { root, options, releaseKeyPath, releasePublic } = await fixture(context, { target: 'darwin-arm64' });
  await assert.rejects(assemble(options), /needs --macos-signing-receipt/);
  const files = [];
  for (const path of ['bin/chariox', 'bin/chariox-app-package', 'bin/chariox-kernel', 'bin/chariox-relay', 'libexec/chariox-app-runtime-install'])
    files.push({ path, role: 'executable', sha256: sha256(await readFile(join(options.executables, path))) });
  const receipt = { schema: 'chariox.macos-release-signing.v1', identity: 'Developer ID Application: Example (TEAM123456)', teamId: 'TEAM123456',
    files, notarization: { submissionId: 'submission-1', status: 'Accepted' } };
  const receiptPath = join(root, 'receipt.json');
  const write = value => writeFile(receiptPath, JSON.stringify(value));

  await write({ ...receipt, notarization: { status: 'Invalid' } });
  await assert.rejects(assemble({ ...options, macosSigningReceipt: receiptPath }), /no accepted notarization/);
  await write({ ...receipt, files: files.filter(file => file.path !== 'bin/chariox') });
  await assert.rejects(assemble({ ...options, macosSigningReceipt: receiptPath }), /bin\/chariox is not in the macOS signing receipt/);
  await write({ ...receipt, files: files.map(file => file.path === 'bin/chariox-kernel' ? { ...file, sha256: 'f'.repeat(64) } : file) });
  await assert.rejects(assemble({ ...options, macosSigningReceipt: receiptPath }), /bin\/chariox-kernel differs from the codesigned/);

  await write(receipt);
  await assemble({ ...options, macosSigningReceipt: receiptPath });
  const manifest = JSON.parse(await readFile(join(options.output, 'manifest.json'), 'utf8'));
  assert.deepEqual(manifest.macosSigning, { identity: receipt.identity, teamId: 'TEAM123456', notarySubmissionId: 'submission-1',
    receiptSha256: sha256(await readFile(receiptPath)) });
  assert.ok(!manifest.files.some(file => file.path.startsWith('deploy/') || file.path === 'install.sh' || file.path.includes('chariox-app-storage')));
  await signBundle(options.output, releaseKeyPath);
  assert.equal((await verifyBundle(options.output, releasePublic)).platform, 'darwin-arm64');
});

test('the release key must be private to the signer, and arguments are exact', async (context) => {
  const { options, releaseKeyPath } = await fixture(context);
  await assemble(options);
  await chmod(releaseKeyPath, 0o640);
  await assert.rejects(signBundle(options.output, releaseKeyPath), /no group or other permissions/);
  assert.deepEqual(parseArgs(['verify', '--bundle', 'b', '--public-key', 'k']), { command: 'verify', options: { bundle: 'b', publicKey: 'k' } });
  assert.throws(() => parseArgs(['sign', '--bundle', 'b', '--signing-key']), /--signing-key needs a value/);
  assert.throws(() => parseArgs(['sign', '--key', 'k']), /unknown argument for sign: --key/);
  assert.throws(() => parseArgs(['verify', '--bundle', 'a', '--bundle', 'b']), /given twice/);
  assert.throws(() => parseArgs(['publish']), /usage/);
});

test('a bundle carries the runtime that the production signer released outside the checkout', async (t) => {
  const f = await runtimeReleaseFixture(t);
  // The signer refuses an output inside its source checkout, so CI signs into $RUNNER_TEMP.
  await assert.rejects(signRuntimeRelease({ ...f.options, output: join(f.options.repository, 'runtime') }), /separate from source/);
  const receipt = await signRuntimeRelease(f.options);
  const inputs = await bundleInputs(f.root, 'linux-x64');
  const output = join(f.root, 'out', 'chariox-0.2.0-linux-x64');
  const assembled = await assemble({ platform: 'linux-x64', version: '0.2.0', sourceCommit: f.proof.sourceCommit,
    executables: inputs.executables, runtime: f.options.output, runtimePublicKey: receipt.publicKeyHex,
    sourceRoot: inputs.sourceRoot, output });
  assert.deepEqual(assembled.runtime, { inventorySha256: receipt.inventorySha256, publicKeyHex: receipt.publicKeyHex });
  await signBundle(output, inputs.releaseKeyPath);
  assert.equal((await verifyBundle(output, inputs.releasePublic)).runtime.inventorySha256, receipt.inventorySha256);
  await assert.rejects(assemble({ platform: 'linux-x64', version: '0.2.0', sourceCommit: 'c'.repeat(40), executables: inputs.executables,
    runtime: f.options.output, runtimePublicKey: receipt.publicKeyHex, sourceRoot: inputs.sourceRoot, output: `${output}-other` }), /runtime source/);
});

// install.sh checks the bundle with python3 and OpenSSL 3 (Ed25519 raw verification), as on a Linux host.
const opensslEd25519 = process.platform === 'linux'
  && /^OpenSSL 3/.test(spawnSync('openssl', ['version'], { encoding: 'utf8' }).stdout ?? '');

test('install.sh --check accepts only the bundle and runtime signed by their keys', { skip: !opensslEd25519 && 'needs Linux with OpenSSL 3' }, async (context) => {
  const { options, releaseKeyPath, releasePublic } = await fixture(context);
  await assemble(options);
  await signBundle(options.output, releaseKeyPath);
  const check = key => spawnSync(join(options.output, 'install.sh'), ['--check', '--release-key', key], { encoding: 'utf8' });
  const accepted = check(releasePublic);
  assert.equal(accepted.status, 0, accepted.stderr);
  assert.match(accepted.stdout, /is Chariox 0\.2\.0, signed by the release key/);
  assert.match(check(keyHex(generateKeyPairSync('ed25519').privateKey)).stderr, /does not verify with the release key/);
  // A manifest signed by the release key still needs a valid runtime inventory signature.
  const inventorySig = join(options.output, 'runtime/runtime-inventory.sig');
  const inventory = await readFile(join(options.output, 'runtime/runtime-inventory.json'));
  await chmod(inventorySig, 0o644);
  await writeFile(inventorySig, sign(null, inventory, generateKeyPairSync('ed25519').privateKey).toString('hex'));
  const manifestPath = join(options.output, 'manifest.json');
  const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  const entry = manifest.files.find(file => file.path === 'runtime/runtime-inventory.sig');
  entry.sha256 = sha256(await readFile(inventorySig));
  await writeFile(manifestPath, JSON.stringify(manifest));
  const releaseKey = createPrivateKey(await readFile(releaseKeyPath));
  await writeFile(join(options.output, 'manifest.sig'), sign(null, await readFile(manifestPath), releaseKey).toString('hex'));
  assert.match(check(releasePublic).stderr, /runtime-inventory\.sig does not verify with the runtime key in the signed manifest/);
  await writeFile(join(options.output, 'runtime/sdk/src/index.js'), 'changed');
  assert.match(check(releasePublic).stderr, /runtime\/sdk\/src\/index\.js does not match the signed manifest/);
  assert.match(execFileSync(join(options.output, 'install.sh'), ['--help'], { encoding: 'utf8' }), /usage: sudo \.\/install\.sh/);
});
