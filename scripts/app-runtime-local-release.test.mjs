import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { createHash, createPublicKey, generateKeyPairSync, sign } from 'node:crypto';
import { chmod, lstat, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { backupRestoreChallenge, loadLocalSigners } from './app-runtime-local-signers.mjs';
import { dockerAdmissionSetupCommand, localRelease } from './app-runtime-local-release.mjs';

test('the macOS developer receipt names the boot installer and quotes the checkout path', () => {
  const command = dockerAdmissionSetupCommand("/tmp/Chariox owner's checkout");
  // Parse the printed command without executing sudo or touching any host files.
  const parsed = spawnSync('sh', ['-c', `set -- ${command}; printf '%s\\n' "$@"`], { encoding: 'utf8' });
  assert.equal(parsed.status, 0, parsed.stderr);
  assert.deepEqual(parsed.stdout.trim().split('\n'), ['sudo', '/usr/bin/python3',
    "/tmp/Chariox owner's checkout/deploy/local-macos/install-docker-admission-locks.py"]);
});

test('MP-11-RD-F3 ordinary local release requires explicit retained signers before packaging', async () => {
  await assert.rejects(localRelease({ nativeDirectory: '/nonexistent-mp11-native', output: '/nonexistent-mp11-output', target: 'darwin-arm64' }),
    /explicit builderKey, signingKey and signerInventory/);
});

async function withSigners(run) {
  const home = await mkdtemp(join(tmpdir(), 'mp11-local-signers-'));
  const store = join(home, '.chariox/keys');
  await mkdir(store, { recursive: true, mode: 0o700 });
  const inputs = { builderKey: join(store, 'builder.pem'), signingKey: join(store, 'release.pem'), signerInventory: join(store, 'inventory.json') };
  const pairs = { builder: generateKeyPairSync('ed25519'), release: generateKeyPairSync('ed25519') };
  const inventory = { schema: 'chariox.local-signers.v1', epoch: 1, signers: {} };
  for (const [role, pair] of Object.entries(pairs)) {
    await writeFile(role === 'builder' ? inputs.builderKey : inputs.signingKey, pair.privateKey.export({ type: 'pkcs8', format: 'pem' }), { mode: 0o600 });
    const publicKeySha256 = createHash('sha256').update(pair.publicKey.export({ type: 'spki', format: 'der' })).digest('hex');
    inventory.signers[role] = { publicKeySha256, backupRestoreProof: { scope: 'same-machine',
      signatureHex: sign(null, backupRestoreChallenge(role, 1, publicKeySha256, 'same-machine'), pair.privateKey).toString('hex') } };
  }
  await writeFile(inputs.signerInventory, JSON.stringify(inventory), { mode: 0o600 });
  try { await run({ home, store, inputs, inventory, pairs }); }
  finally { await rm(home, { recursive: true, force: true }); }
}

test('MP-11-RD-F3 retained signers match approved public fingerprints and restoration proofs', async () => {
  await withSigners(async ({ home, inputs, pairs }) => {
    const before = await readFile(inputs.builderKey);
    const loaded = await loadLocalSigners(inputs, home);
    assert.deepEqual(createPublicKey(loaded.builder).export({ type: 'spki', format: 'der' }), pairs.builder.publicKey.export({ type: 'spki', format: 'der' }));
    assert.equal((await readFile(inputs.builderKey)).equals(before), true);
    assert.equal((await lstat(inputs.builderKey)).mode & 0o777, 0o600);
  });
});

test('MP-11-RD-F3 missing keys never regenerate and replacement keys require rotation', async () => {
  await withSigners(async ({ home, inputs }) => {
    await rm(inputs.builderKey);
    await assert.rejects(loadLocalSigners(inputs, home), /authorized provisioning or rotation/);
    await assert.rejects(lstat(inputs.builderKey), { code: 'ENOENT' });
    const replacement = generateKeyPairSync('ed25519');
    await writeFile(inputs.builderKey, replacement.privateKey.export({ type: 'pkcs8', format: 'pem' }), { mode: 0o600 });
    await assert.rejects(loadLocalSigners(inputs, home), /fingerprint/);
  });
});

test('MP-11-RD-F3 rejects insecure paths, permissions and symlinks without repairing them', async () => {
  await withSigners(async ({ home, store, inputs }) => {
    await assert.rejects(loadLocalSigners({ ...inputs, builderKey: join(home, 'dev/builder.pem') }, home), /unavailable or invalid/);
    await chmod(inputs.builderKey, 0o644);
    await assert.rejects(loadLocalSigners(inputs, home), /unavailable or invalid/);
    assert.equal((await lstat(inputs.builderKey)).mode & 0o777, 0o644);
    await chmod(inputs.builderKey, 0o600);
    await chmod(store, 0o755);
    await assert.rejects(loadLocalSigners(inputs, home), /unavailable or invalid/);
    await chmod(store, 0o700);
    const alias = join(store, 'alias.pem');
    await symlink(inputs.builderKey, alias);
    await assert.rejects(loadLocalSigners({ ...inputs, builderKey: alias }, home), /unavailable or invalid/);
  });
});

test('MP-11-RD-F3 both roles require a valid epoch-bound protected backup restoration proof', async () => {
  for (const role of ['builder', 'release']) {
    for (const mutate of [record => { delete record.backupRestoreProof; }, record => { record.backupRestoreProof.signatureHex = '0'.repeat(128); }]) {
      await withSigners(async ({ home, inputs, inventory }) => {
        mutate(inventory.signers[role]);
        await writeFile(inputs.signerInventory, JSON.stringify(inventory));
        await assert.rejects(loadLocalSigners(inputs, home), /restoration proof/);
      });
    }
  }
  await withSigners(async ({ home, inputs, inventory }) => {
    inventory.epoch++;
    await writeFile(inputs.signerInventory, JSON.stringify(inventory));
    await assert.rejects(loadLocalSigners(inputs, home), /restoration proof/);
  });
});
