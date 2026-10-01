import assert from 'node:assert/strict';
import { generateKeyPairSync, verify } from 'node:crypto';
import { chmod, lstat, readFile, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { sha256 } from './app-runtime-bundle-files.mjs';
import { runtimeReleaseFixture as fixture } from './app-runtime-release-fixture.mjs';
import { signRuntimeRelease } from './sign-app-runtime-release.mjs';

const pem = (key, type) => key.export({ type, format: 'pem' });

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
  // The loader is every worker's ELF interpreter, so it must be executable; the libraries are not.
  const mode = async path => (await lstat(join(f.options.output, path))).mode & 0o7777;
  const loader = inventory.files.find(entry => entry.path.startsWith('platform/ld-linux'));
  assert.ok(loader, 'the Linux fixture signs a platform loader');
  assert.equal(await mode(loader.path), 0o555);
  assert.equal(await mode('platform/libc.so.6'), 0o444);
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

test('darwin graphs contain only the Seatbelt launcher and require the developer path', async () => {
  const { launcherInputs, nativeExecutables } = await import('./app-runtime-release-contract.mjs');
  const bundle = { target: 'darwin-arm64', files: [{ path: 'libnode.137.dylib' }] };
  assert.deepEqual(releasePaths(bundle), ['bundle-manifest.json', 'chariox-app-worker', 'libnode.137.dylib']);
  assert.deepEqual(platformFiles('darwin-arm64'), []);
  assert.deepEqual(nativeExecutables('darwin-x64'), ['chariox-app-worker']);
  assert.ok(launcherInputs('darwin-arm64').includes('apps/app-worker/src/sandbox_macos.c'));
  assert.ok(!launcherInputs('darwin-arm64').includes('apps/app-worker/src/sandbox_linux.c'));
  assert.deepEqual(launcherInputs('linux-x64'), LAUNCHER_INPUTS);
});

test('the production signer refuses darwin bundles before writing any output', async t => {
  const parent = join(homedir(), '.chariox/dev');
  await mkdir(parent, { recursive: true, mode: 0o700 });
  const root = await mkdtemp(join(await realpath(parent), 'runtime-release-darwin-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(join(root, 'input/bundle'), { recursive: true, mode: 0o700 });
  await writeFile(join(root, 'input/bundle/bundle-manifest.json'), JSON.stringify({ target: 'darwin-arm64' }), { mode: 0o600 });
  await assert.rejects(signRuntimeRelease({ inputDirectory: join(root, 'input'), builderAttestation: join(root, 'b.json'),
    builderSignature: join(root, 'b.sig'), trustedBuilderKey: join(root, 'b.pem'), signingKey: join(root, 'r.pem'),
    output: join(root, 'release') }), /Developer ID signing path/);
  await assert.rejects(lstat(join(root, 'release')), { code: 'ENOENT' });
});
