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
