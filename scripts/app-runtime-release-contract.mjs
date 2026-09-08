// Release authority comes from an external builder key and release key. Neither
// a native build manifest nor a public key inside its directory is authority.
import { createPublicKey, verify } from 'node:crypto';
import { stableJson } from './app-runtime-bundle-files.mjs';
import { parseJson } from '../packages/app-sdk/src/json.js';

export const RELEASE_LIMIT = 536870912;
export const MANIFEST_LIMIT = 262144;
// Security launcher changes must not invalidate the expensive Node build cache.
// They have their own exact attested input set at final release assembly.
export const LAUNCHER_INPUTS = [
  'apps/app-worker/sandbox.lock.json', 'apps/app-worker/src/launcher.c',
  'apps/app-worker/src/launcher.h', 'apps/app-worker/src/runtime.h',
  'apps/app-worker/src/launch_record.c', 'apps/app-worker/src/launch_process.c',
  'apps/app-worker/src/sandbox_linux.c', 'apps/app-worker/src/linux_domain_entry.c',
].sort();
const HEX = /^[a-f0-9]{64}$/u;
const COMMIT = /^[a-f0-9]{40}$/u;
const equal = (a, b) => stableJson(a) === stableJson(b);

export function platformFiles(target) {
  const loader = { 'linux-x64': 'ld-linux-x86-64.so.2', 'linux-arm64': 'ld-linux-aarch64.so.1' }[target];
  if (!loader) throw new Error('Linux release target required; macOS needs its code-signing release path');
  return [loader, 'libc.so.6', 'libm.so.6', 'libstdc++.so.6', 'libgcc_s.so.1',
    'libpthread.so.0', 'libdl.so.2', 'librt.so.1'].map(name => `platform/${name}`).sort();
}

export function releasePaths(bundle) {
  return [...bundle.files.map(file => file.path), 'bundle-manifest.json',
    'chariox-app-worker', 'chariox-app-domain-entry', 'chariox-bwrap', ...platformFiles(bundle.target)].sort();
}

export function executable(path) {
  return ['chariox-app-worker', 'chariox-app-domain-entry', 'chariox-bwrap'].includes(path);
}

export function publicKey(bytes) {
  const key = createPublicKey(bytes);
  if (key.asymmetricKeyType !== 'ed25519') throw new Error('Ed25519 key required');
  return key;
}

function keys(value, expected) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || !equal(Object.keys(value).sort(), [...expected].sort())) throw new Error('invalid release fields');
}

export function verifyBuilder(bytes, signature, trustedKey, bundle, launcherInputs) {
  if (bytes.length > MANIFEST_LIMIT || signature.length !== 128
    || !/^[a-f0-9]{128}$/u.test(signature.toString('utf8'))
    || !verify(null, bytes, publicKey(trustedKey), Buffer.from(signature.toString('utf8'), 'hex')))
    throw new Error('invalid external builder signature');
  const proof = parseJson(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
  keys(proof, ['schema', 'target', 'sourceCommit', 'bundleDigest', 'launcherBuildInputs', 'files']);
  if (proof.schema !== 'chariox.app-runtime-release-build.v1' || proof.target !== bundle.target
    || !COMMIT.test(proof.sourceCommit) || proof.sourceCommit !== bundle.sourceCommit
    || proof.bundleDigest !== bundle.bundleDigest || !equal(proof.launcherBuildInputs, launcherInputs)
    || !Array.isArray(proof.files) || proof.files.length > 40
    || !equal(proof.files.map(file => file.path), releasePaths(bundle)))
    throw new Error('builder proof differs from the release contract');
  let size = 0;
  for (const file of proof.files) {
    keys(file, ['path', 'size', 'sha256', 'executable']);
    if (!Number.isSafeInteger(file.size) || file.size <= 0 || !HEX.test(file.sha256)
      || file.executable !== executable(file.path) || (size += file.size) > RELEASE_LIMIT)
      throw new Error('invalid builder file inventory');
  }
  return proof;
}

export function runtimeInventory(proof, bundle) {
  return {
    schema: 'chariox.app-runtime-inventory.v1', target: proof.target,
    runtimeVersion: bundle.runtimeVersion, workerAbi: bundle.workerAbi,
    nodeVersion: bundle.nodeVersion, nodeModuleAbi: bundle.nodeModuleAbi,
    sdkVersion: bundle.sdk.version, sourceCommit: proof.sourceCommit, files: proof.files,
  };
}
