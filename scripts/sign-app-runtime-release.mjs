#!/usr/bin/env node
// Offline Linux release assembly. The trusted builder supplies a signed exact
// inventory of bundle + launcher + bubblewrap + fixed ELF dependency files.
// This tool copies verified bytes and signs the installed-runtime inventory. It
// executes no input artifact, installs no root authority, and certifies no drill.
import { createPrivateKey, createPublicKey, sign } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { chmod, lstat, open, realpath, rm, writeFile } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { digestAndCopy, inventory, outputDirectory, readSmall, sha256, stableJson } from './app-runtime-bundle-files.mjs';
import { verifyBundle } from './package-app-runtime.mjs';
import { executable, LAUNCHER_INPUTS, MANIFEST_LIMIT, RELEASE_LIMIT, releasePaths, runtimeInventory, verifyBuilder } from './app-runtime-release-contract.mjs';

const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const equal = (a, b) => stableJson(a) === stableJson(b);
const keyBytes = path => readSmall(dirname(resolve(path)), basename(path), 4096);
function sourceBytes(repository, commit, path) {
  return execFileSync('git', ['-C', repository, 'show', `${commit}:${path}`],
    { timeout: 10000, maxBuffer: MANIFEST_LIMIT, stdio: ['ignore', 'pipe', 'pipe'] });
}

export async function signRuntimeRelease({ inputDirectory, builderAttestation, builderSignature,
  trustedBuilderKey, signingKey, output, repository = REPOSITORY }) {
  repository = await realpath(repository);
  inputDirectory = await realpath(inputDirectory);
  const bundleRoot = join(inputDirectory, 'bundle');
  const bundle = await verifyBundle(bundleRoot);
  // Historical native evidence is useful for debugging, but cannot become a
  // release by attaching a new signature to the old manifest.
  if (bundle.native.inputStatus !== 'matches-current-source') throw new Error('historical native artifact cannot be released');
  for (const input of bundle.buildInputs) {
    if (sha256(sourceBytes(repository, bundle.sourceCommit, input.path)) !== input.sha256
      || sha256(await readSmall(repository, input.path, MANIFEST_LIMIT)) !== input.sha256)
      throw new Error('release sources differ from builder source identity');
  }
  const launcherInputs = [];
  for (const path of LAUNCHER_INPUTS) {
    const digest = sha256(sourceBytes(repository, bundle.sourceCommit, path));
    if (digest !== sha256(await readSmall(repository, path, MANIFEST_LIMIT)))
      throw new Error('launcher source differs from builder source identity');
    launcherInputs.push({ path, sha256: digest });
  }
  const proof = verifyBuilder(
    await readSmall(dirname(resolve(builderAttestation)), basename(builderAttestation), MANIFEST_LIMIT),
    await readSmall(dirname(resolve(builderSignature)), basename(builderSignature), 128),
    await keyBytes(trustedBuilderKey), bundle, launcherInputs,
  );
  const bundlePaths = new Set([...bundle.files.map(file => file.path), 'bundle-manifest.json']);
  const sourcePath = path => bundlePaths.has(path) ? `bundle/${path}` : `native/${path}`;
  if (!equal(await inventory(inputDirectory, 40), releasePaths(bundle).map(sourcePath).sort()))
    throw new Error('release input contains missing or undeclared files');
  const keyMetadata = await lstat(resolve(signingKey));
  if (!keyMetadata.isFile() || keyMetadata.uid !== process.getuid() || keyMetadata.mode & 0o077)
    throw new Error('release signing key must be private and owned');
  const key = createPrivateKey(await keyBytes(signingKey));
  if (key.asymmetricKeyType !== 'ed25519') throw new Error('Ed25519 release key required');
  const owned = await outputDirectory(output, repository, inputDirectory);
  try {
    for (const expected of proof.files) {
      await digestAndCopy(inputDirectory, sourcePath(expected.path), output, expected.path, expected.size, expected);
      if (executable(expected.path)) {
        await chmod(join(output, expected.path), 0o555);
        const file = await open(join(output, expected.path), 'r');
        try { await file.sync(); } finally { await file.close(); }
      }
    }
    const body = runtimeInventory(proof, bundle);
    const bytes = Buffer.from(stableJson(body));
    if (bytes.length > MANIFEST_LIMIT || proof.files.reduce((total, file) => total + file.size, bytes.length + 128) > RELEASE_LIMIT)
      throw new Error('signed runtime exceeds its byte limit');
    const signature = sign(null, bytes, key).toString('hex');
    for (const [name, contents] of [['runtime-inventory.json', bytes], ['runtime-inventory.sig', signature], ['.runtime-lease', '']]) {
      await writeFile(join(output, name), contents, { flag: 'wx', mode: 0o444 });
      const file = await open(join(output, name), 'r');
      try { await file.sync(); } finally { await file.close(); }
    }
    // Installer copies these bytes into a root-owned version directory and
    // independently selects the trusted key/digest. No self-enrollment file is
    // placed in this directory. Builder and release keys remain external.
    const directories = [...new Set(proof.files.map(file => dirname(file.path)).filter(path => path !== '.'))];
    for (const path of directories.sort((a, b) => b.length - a.length)) {
      const directory = await open(join(output, path), 'r');
      try { await directory.sync(); } finally { await directory.close(); }
    }
    const directory = await open(output, 'r');
    try { await directory.sync(); } finally { await directory.close(); }
    const parent = await open(dirname(output), 'r');
    try { await parent.sync(); } finally { await parent.close(); }
    const publicDer = createPublicKey(key).export({ type: 'spki', format: 'der' });
    if (publicDer.length !== 44) throw new Error('unexpected Ed25519 public key encoding');
    return { target: proof.target, sourceCommit: proof.sourceCommit,
      inventorySha256: sha256(bytes), publicKeyHex: publicDer.subarray(12).toString('hex'),
      installation: 'not-performed', executionValidation: 'not-performed' };
  } catch (error) {
    const current = await lstat(output).catch(() => null);
    if (current?.dev === owned.dev && current?.ino === owned.ino) await rm(output, { recursive: true, force: true });
    throw error;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const names = new Map([['--input', 'inputDirectory'], ['--builder-attestation', 'builderAttestation'],
      ['--builder-signature', 'builderSignature'], ['--trusted-builder-key', 'trustedBuilderKey'],
      ['--signing-key', 'signingKey'], ['--output', 'output']]);
    const options = {};
    for (let index = 2; index < process.argv.length; index += 2) {
      const key = names.get(process.argv[index]);
      if (!key || Object.hasOwn(options, key) || !process.argv[index + 1]) throw new Error('invalid release arguments');
      options[key] = process.argv[index + 1];
    }
    if (Object.keys(options).length !== names.size) throw new Error('missing release arguments');
    process.stdout.write(`${stableJson(await signRuntimeRelease(options))}\n`);
  } catch { process.stderr.write('app_runtime_release_failed\n'); process.exitCode = 1; }
}
