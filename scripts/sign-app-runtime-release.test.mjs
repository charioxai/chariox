import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { generateKeyPairSync, sign, verify } from 'node:crypto';
import { chmod, cp, lstat, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { BUNDLE_TOOL_INPUTS, bundleSources, packageRuntime } from './package-app-runtime.mjs';
import { NATIVE_BUILD_INPUTS } from './app-runtime-ci-receipt.mjs';
import { sha256, stableJson } from './app-runtime-bundle-files.mjs';
import { executable, LAUNCHER_INPUTS, platformFiles, releasePaths } from './app-runtime-release-contract.mjs';
import { signRuntimeRelease } from './sign-app-runtime-release.mjs';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const git = (root, args) => execFileSync('git', ['-C', root, ...args],
  { encoding: 'utf8', timeout: 10000, stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const pem = (key, type) => key.export({ type, format: 'pem' });

// Text libraries and fake builder keys exercise signing/provenance only. This
// fixture neither executes native code nor claims a production runtime build.
async function fixture(t) {
  const parent = join(homedir(), '.chariox/dev');
  await mkdir(parent, { recursive: true, mode: 0o700 });
  const root = await mkdtemp(join(await realpath(parent), 'runtime-release-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const source = join(root, 'source'); const input = join(root, 'input');
  const raw = join(root, 'raw');
  for (const path of [source, input, raw, join(input, 'native')]) await mkdir(path, { mode: 0o700 });
  const contract = JSON.parse(await readFile(join(repository, 'apps/app-worker/bundle.lock.json')));
  const sources = await bundleSources(repository, contract);
  for (const path of new Set([...sources.map(file => file.source), ...BUNDLE_TOOL_INPUTS, ...LAUNCHER_INPUTS])) {
    await mkdir(dirname(join(source, path)), { recursive: true, mode: 0o700 });
    await cp(join(repository, path), join(source, path));
  }
  git(source, ['init', '-q']); git(source, ['add', '.']);
  git(source, ['-c', 'user.name=Release Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture']);
  const runtime = JSON.parse(await readFile(join(source, 'apps/app-worker/runtime.lock.json')));
  const files = [];
  for (const path of ['libnode.so.137', 'libchariox-app-runtime.so', 'NODE-LICENSE']) {
    const bytes = Buffer.from(`Not executable: ${path}\n`);
    await writeFile(join(raw, path), bytes, { mode: 0o600 });
    files.push({ path, size: bytes.length, sha256: sha256(bytes) });
  }
  const artifact = {
    schema: 'chariox.app-runtime-artifact.v1', runtimeVersion: runtime.runtimeVersion, workerAbi: 1,
    target: 'linux-x64', nodeVersion: runtime.node.version, nodeModuleAbi: runtime.node.moduleAbi,
    source: { url: runtime.node.url, sha256: runtime.node.sha256 },
    sourceCommit: git(source, ['rev-parse', 'HEAD']), sourceTree: git(source, ['rev-parse', 'HEAD^{tree}']),
    buildInputs: await Promise.all(NATIVE_BUILD_INPUTS.map(async path => ({ path, sha256: sha256(await readFile(join(source, path))) }))),
    toolchain: { cc: '12.2.0', cxx: '12.2.0', python: 'Python 3.11.2', make: 'GNU Make 4.3\nfixture' },
    dependencies: { 'libnode.so.137': ' (NEEDED) Shared library: [libc.so.6]',
      'libchariox-app-runtime.so': ' (NEEDED) Shared library: [libnode.so.137]' },
    files, signing: { status: 'unsigned', notarization: 'not-performed' },
    validation: { nativeBuild: 'completed' }, loading: runtime.loading,
  };
  await writeFile(join(raw, 'artifact-manifest.json'), JSON.stringify(artifact));
  const bundle = await packageRuntime({ nativeDirectory: raw, output: join(input, 'bundle'), target: 'linux-x64', repository: source });
  for (const path of ['chariox-app-worker', 'chariox-app-domain-entry', 'chariox-bwrap', ...platformFiles(bundle.target)]) {
    await mkdir(dirname(join(input, 'native', path)), { recursive: true, mode: 0o700 });
    await writeFile(join(input, 'native', path), `Not executable: ${path}\n`, { mode: 0o600 });
  }
  const bundlePaths = new Set([...bundle.files.map(file => file.path), 'bundle-manifest.json']);
  const entries = await Promise.all(releasePaths(bundle).map(async path => {
    const bytes = await readFile(join(input, bundlePaths.has(path) ? 'bundle' : 'native', path));
    return { path, size: bytes.length, sha256: sha256(bytes), executable: executable(path) };
  }));
  const proof = { schema: 'chariox.app-runtime-release-build.v1', target: bundle.target, sourceCommit: bundle.sourceCommit,
    bundleDigest: bundle.bundleDigest,
    launcherBuildInputs: await Promise.all(LAUNCHER_INPUTS.map(async path => ({ path, sha256: sha256(await readFile(join(source, path))) }))),
    files: entries };
  const builder = generateKeyPairSync('ed25519'); const release = generateKeyPairSync('ed25519');
  const bytes = Buffer.from(stableJson(proof));
  const options = { inputDirectory: input, builderAttestation: join(root, 'builder.json'),
    builderSignature: join(root, 'builder.sig'), trustedBuilderKey: join(root, 'builder.pem'),
    signingKey: join(root, 'release.pem'), output: join(root, 'release'), repository: source };
  for (const [path, body] of [[options.builderAttestation, bytes],
    [options.builderSignature, sign(null, bytes, builder.privateKey).toString('hex')],
    [options.trustedBuilderKey, pem(builder.publicKey, 'spki')], [options.signingKey, pem(release.privateKey, 'pkcs8')]]) {
    await writeFile(path, body, { mode: 0o600 });
  }
  return { options, proof, builder, release, root, input };
}

test('release signature covers exact installed graph and modes without self-enrolling trust', async t => {
  const f = await fixture(t);
  const receipt = await signRuntimeRelease(f.options);
  const bytes = await readFile(join(f.options.output, 'runtime-inventory.json'));
  const signature = await readFile(join(f.options.output, 'runtime-inventory.sig'), 'utf8');
  assert.equal(receipt.inventorySha256, sha256(bytes));
  assert.equal(receipt.publicKeyHex, f.release.publicKey.export({ type: 'spki', format: 'der' }).subarray(12).toString('hex'));
  assert.ok(verify(null, bytes, f.release.publicKey, Buffer.from(signature, 'hex')));
  assert.equal(receipt.installation, 'not-performed');
  assert.equal(receipt.executionValidation, 'not-performed');
  const inventory = JSON.parse(bytes);
  assert.deepEqual(inventory.files, f.proof.files);
  for (const entry of inventory.files) {
    const path = join(f.options.output, entry.path);
    assert.equal((await lstat(path)).mode & 0o7777, entry.executable ? 0o555 : 0o444);
    assert.equal(sha256(await readFile(path)), entry.sha256);
  }
  assert.equal((await lstat(join(f.options.output, '.runtime-lease'))).size, 0);
  await assert.rejects(readFile(join(f.options.output, 'runtime-enrollment.json')), { code: 'ENOENT' });
  await assert.rejects(readFile(join(f.options.output, 'release.pem')), { code: 'ENOENT' });
});

test('unattested platform substitution cannot be signed and failed assembly removes its output', async t => {
  const f = await fixture(t);
  const library = join(f.input, 'native/platform/libc.so.6');
  await writeFile(library, 'substituted native library');
  await assert.rejects(signRuntimeRelease(f.options), /digest or size mismatch/);
  await assert.rejects(lstat(f.options.output), { code: 'ENOENT' });
});

test('external builder authority and complete inventory are required before release signing', async t => {
  const f = await fixture(t);
  const unrelated = generateKeyPairSync('ed25519');
  await writeFile(f.options.trustedBuilderKey, pem(unrelated.publicKey, 'spki'));
  await assert.rejects(signRuntimeRelease(f.options), /external builder signature/);
  await writeFile(f.options.trustedBuilderKey, pem(f.builder.publicKey, 'spki'));
  await writeFile(join(f.input, 'native/undeclared'), 'not admitted');
  await assert.rejects(signRuntimeRelease(f.options), /undeclared/);
  await rm(join(f.input, 'native/undeclared'));
  await chmod(f.options.signingKey, 0o644);
  await assert.rejects(signRuntimeRelease(f.options), /private and owned/);
  await assert.rejects(lstat(f.options.output), { code: 'ENOENT' });
});

test('launcher security source changes cannot reuse old signed builder provenance', async t => {
  const f = await fixture(t);
  const source = join(f.options.repository, 'apps/app-worker/src/sandbox_linux.c');
  await writeFile(source, `${await readFile(source, 'utf8')}\n/* changed release input */\n`);
  await assert.rejects(signRuntimeRelease(f.options), /launcher source differs/);
  await assert.rejects(lstat(f.options.output), { code: 'ENOENT' });
});

test('oversized substituted native input is rejected at its signed size before copying', async t => {
  const f = await fixture(t);
  const entry = f.proof.files.find(file => file.path === 'platform/libc.so.6');
  await writeFile(join(f.input, 'native', entry.path), Buffer.alloc(entry.size + 1, 65));
  await assert.rejects(signRuntimeRelease(f.options), /bounded regular file/);
  await assert.rejects(lstat(f.options.output), { code: 'ENOENT' });
});
