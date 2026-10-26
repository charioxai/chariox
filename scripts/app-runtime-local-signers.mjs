// Operator-provisioned identities only. Release execution never provisions or
// rotates a signer; the protected inventory approves its public fingerprint and
// records a synthetic challenge signed after restoring the separate backup.
import { createHash, createPrivateKey, createPublicKey, verify } from 'node:crypto';
import { constants } from 'node:fs';
import { lstat, open, realpath } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';

const proofSchema = 'chariox.local-signing-backup-restore.v1';

export function backupRestoreChallenge(role, epoch, publicKeySha256, scope) {
  return Buffer.from(`${proofSchema}:${role}:${epoch}:${publicKeySha256}:${scope}`);
}

async function protectedFile(path, store) {
  if (typeof path !== 'string' || !isAbsolute(path)) throw new Error('local signer paths must be explicit absolute paths');
  const resolved = resolve(path);
  const local = relative(store, resolved);
  if (!local || local === '..' || local.startsWith(`..${sep}`) || isAbsolute(local)) {
    throw new Error('local signer inputs must be inside the dedicated ~/.chariox/keys store');
  }
  let directory = dirname(resolved);
  while (true) {
    const metadata = await lstat(directory);
    if (!metadata.isDirectory() || metadata.isSymbolicLink() || (metadata.mode & 0o7777) !== 0o700 || metadata.uid !== process.getuid()) {
      throw new Error('local signing directories must be owned by the operator with mode 0700 and no symlinks');
    }
    if (directory === store) break;
    directory = dirname(directory);
  }
  if (await realpath(store) !== store) throw new Error('local key store must not have symlink ancestors');
  const file = await open(resolved, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const metadata = await file.stat();
    if (!metadata.isFile() || metadata.nlink !== 1 || (metadata.mode & 0o7777) !== 0o600 || metadata.uid !== process.getuid() || metadata.size > 16_384) {
      throw new Error('local signing inputs must be bounded operator-owned files with mode 0600');
    }
    const bytes = Buffer.alloc(16_385);
    const { bytesRead } = await file.read(bytes, 0, bytes.length, 0);
    if (bytesRead > 16_384) throw new Error('local signing input exceeds its bound');
    return bytes.subarray(0, bytesRead);
  } finally { await file.close(); }
}

export async function loadLocalSigners({ builderKey, signingKey, signerInventory }, home = homedir()) {
  if (!builderKey || !signingKey || !signerInventory) {
    throw new Error('local release requires explicit builderKey, signingKey and signerInventory; missing keys require authorized provisioning or rotation');
  }
  const store = join(home, '.chariox/keys');
  let inventory;
  let keys;
  try {
    inventory = JSON.parse(await protectedFile(signerInventory, store));
    keys = {
      builder: createPrivateKey(await protectedFile(builderKey, store)),
      release: createPrivateKey(await protectedFile(signingKey, store)),
    };
  } catch {
    // No parser/crypto error may include private input or silently replace it.
    throw new Error('local signer inputs unavailable or invalid; retain the approved identity and use explicit authorized provisioning or rotation', { cause: undefined });
  }
  if (inventory.schema !== 'chariox.local-signers.v1' || !Number.isSafeInteger(inventory.epoch) || inventory.epoch < 1) {
    throw new Error('local signer inventory schema or epoch is invalid');
  }
  for (const role of ['builder', 'release']) {
    const key = keys[role];
    const approved = inventory.signers?.[role];
    if (key.asymmetricKeyType !== 'ed25519') throw new Error('local signers must be Ed25519');
    const publicKey = createPublicKey(key);
    const fingerprint = createHash('sha256').update(publicKey.export({ type: 'spki', format: 'der' })).digest('hex');
    if (!/^[0-9a-f]{64}$/.test(approved?.publicKeySha256 ?? '') || approved.publicKeySha256 !== fingerprint) {
      throw new Error(`local ${role} public fingerprint does not match the approved inventory`);
    }
    const proof = approved.backupRestoreProof;
    if (!['same-machine', 'off-device'].includes(proof?.scope) || !/^[0-9a-f]{128}$/.test(proof?.signatureHex ?? '') ||
        !verify(null, backupRestoreChallenge(role, inventory.epoch, fingerprint, proof.scope), publicKey, Buffer.from(proof.signatureHex, 'hex'))) {
      throw new Error(`local ${role} protected backup restoration proof is missing or invalid`);
    }
  }
  if (createPublicKey(keys.builder).export({ type: 'spki', format: 'der' }).equals(createPublicKey(keys.release).export({ type: 'spki', format: 'der' }))) {
    throw new Error('local builder and release must use separate approved identities');
  }
  return keys;
}
