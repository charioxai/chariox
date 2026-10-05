#!/usr/bin/env node
// Assembles, signs and verifies one versioned Chariox release bundle per platform
// (docs/CHARIOX_DISTRIBUTION_PLAN.md). It runs on the CI builder and never runs
// the artifacts.
//
//   release-bundle.mjs slice-context --source-root DIR --source-commit SHA --output DIR
//   release-bundle.mjs assemble --platform linux-x64|darwin-arm64 --version V
//       --source-commit SHA --executables DIR --runtime DIR --runtime-public-key HEX
//       --slice-build-context DIR [--macos-signing-receipt FILE] [--source-root DIR]
//       --output DIR
//   release-bundle.mjs sign --bundle DIR --signing-key PEM
//   release-bundle.mjs verify --bundle DIR --public-key HEX
//
// `slice-context` exports, from the source commit, the slice build context a
// release kernel runs its slice provisioner from: the provisioner's
// runtime-source-roots.txt roots (which it fingerprints and builds the slice
// image from) and the one Dockerfile input outside them, in the repository layout.
// `assemble` copies already built (and, on macOS, already codesigned and
// notarized) artifacts into the bundle layout and writes manifest.json: the
// version, platform, source commit, the signed App runtime's inventory digest
// and key, and every file's size, mode and SHA-256. `sign` signs the exact
// manifest bytes with the release key (Ed25519, hex in manifest.sig). `verify`
// checks that signature against a key from the release authority, then every
// file's size and SHA-256, refuses any file the manifest does not name, and
// re-verifies the App runtime's own signed inventory.
import { spawnSync } from 'node:child_process';
import { createPrivateKey, createPublicKey, sign as signBytes, verify as verifyBytes } from 'node:crypto';
import { chmod, copyFile, lstat, mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { relativeFile, sha256, stableJson } from './app-runtime-bundle-files.mjs';

export const SCHEMA = 'chariox.release-bundle.v1';
const MACOS_RECEIPT_SCHEMA = 'chariox.macos-release-signing.v1';
const RUNTIME_SCHEMA = 'chariox.app-runtime-inventory.v1';
const RUNTIME_CONTROL = ['runtime-inventory.json', 'runtime-inventory.sig', '.runtime-lease'];
// Release kernels look for this beside their bin/ (apps/kernel/src/slice/local_docker.rs).
export const SLICE_CONTEXT = 'share/chariox/slice-build-context';
const SLICE_ROOTS = 'apps/kernel/slice-linux-docker/runtime-source-roots.txt';
export const SLICE_PROVISIONER = 'apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh';
// The slice Dockerfile's only input outside the runtime source roots.
const SLICE_EXTRA_INPUTS = ['apps/browser-session-import'];
const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

const rustBinaries = ['chariox-kernel', 'chariox-relay', 'chariox-app-package'];
export const PLATFORMS = {
  'linux-x64': {
    bin: ['chariox', ...rustBinaries],
    // The App storage helper is Linux only.
    libexec: ['chariox-app-storage', 'chariox-app-runtime-install'],
    linuxInstaller: true,
  },
  'darwin-arm64': {
    bin: ['chariox', ...rustBinaries],
    libexec: ['chariox-app-runtime-install'],
    linuxInstaller: false,
  },
};

const fail = message => { throw new Error(message); };
const hex = (value, length, name) => (typeof value === 'string' && new RegExp(`^[0-9a-f]{${length}}$`).test(value))
  ? value : fail(`${name} must be ${length} lowercase hexadecimal characters`);

export function publicKeyFromHex(value) {
  hex(value, 64, 'the public key');
  return createPublicKey({ format: 'jwk', key: { kty: 'OKP', crv: 'Ed25519', x: Buffer.from(value, 'hex').toString('base64url') } });
}

function publicKeyHex(key) {
  return Buffer.from(createPublicKey(key).export({ format: 'jwk' }).x, 'base64url').toString('hex');
}

function verifySignature(bytes, signature, keyHex, what) {
  const text = signature.toString('utf8');
  if (!/^[0-9a-f]{128}$/.test(text)) fail(`${what} signature must be 128 lowercase hexadecimal characters`);
  if (!verifyBytes(null, bytes, publicKeyFromHex(keyHex), Buffer.from(text, 'hex'))) fail(`${what} signature does not verify with key ${keyHex}`);
}

// Every regular file below root, as sorted relative paths. Links and special files are refused.
async function files(root) {
  const found = [];
  async function walk(directory) {
    for (const name of (await readdir(directory)).sort()) {
      const path = join(directory, name);
      const metadata = await lstat(path);
      const relativePath = relative(root, path).split('\\').join('/');
      if (metadata.isDirectory()) await walk(path);
      else if (metadata.isFile()) found.push(relativeFile(relativePath));
      else fail(`${relativePath} is a link or special file`);
    }
  }
  await walk(root);
  return found;
}

async function regularFile(path, what) {
  const metadata = await lstat(path).catch(() => fail(`${what} is missing: ${path}`));
  if (!metadata.isFile()) fail(`${what} must be a regular file, not a link: ${path}`);
  return metadata;
}

/** Verifies a signed App runtime release directory; returns its identity. */
export async function verifyRuntime(directory, keyHex, platform, sourceCommit) {
  const inventoryBytes = await readFile(join(directory, 'runtime-inventory.json'));
  verifySignature(inventoryBytes, await readFile(join(directory, 'runtime-inventory.sig')), keyHex, 'runtime inventory');
  const inventory = JSON.parse(inventoryBytes.toString('utf8'));
  if (inventory.schema !== RUNTIME_SCHEMA) fail(`runtime inventory schema must be ${RUNTIME_SCHEMA}`);
  if (inventory.target !== platform) fail(`runtime target ${inventory.target} is not ${platform}`);
  if (inventory.sourceCommit !== sourceCommit) fail(`runtime source ${inventory.sourceCommit} is not ${sourceCommit}`);
  const expected = new Map(inventory.files.map(file => [relativeFile(file.path), file]));
  const present = await files(directory);
  for (const path of present) {
    if (RUNTIME_CONTROL.includes(path)) continue;
    const entry = expected.get(path) ?? fail(`runtime file ${path} is not in the signed inventory`);
    const bytes = await readFile(join(directory, path));
    if (bytes.length !== entry.size || sha256(bytes) !== entry.sha256) fail(`runtime file ${path} does not match the signed inventory`);
    expected.delete(path);
  }
  if (expected.size) fail(`runtime is missing ${[...expected.keys()].join(', ')}`);
  return { inventorySha256: sha256(inventoryBytes), publicKeyHex: keyHex };
}

function git(sourceRoot, args, maxBuffer = 1 << 20) {
  const result = spawnSync('git', ['-C', sourceRoot, ...args], { maxBuffer, env: {
    PATH: process.env.PATH ?? '/usr/bin:/bin', HOME: '/nonexistent', GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null', LC_ALL: 'C' } });
  if (result.status !== 0) fail(`git ${args[0]} failed: ${result.stderr?.toString().trim() || result.error?.message}`);
  return result.stdout;
}

/** Exports the slice build context of `commit` from the Git repository `sourceRoot`. */
export async function exportSliceBuildContext(sourceRoot, commit, output) {
  hex(commit, 40, '--source-commit');
  const target = resolve(output ?? fail('--output is required'));
  const roots = git(resolve(sourceRoot), ['show', `${commit}:${SLICE_ROOTS}`]).toString('utf8').split('\n').filter(Boolean);
  if (!roots.length || roots.some(root => !/^[A-Za-z0-9._-]+(?:\/[A-Za-z0-9._-]+)*$/.test(root) || root.split('/').includes('..')))
    fail(`${SLICE_ROOTS} lists an invalid root`);
  const archive = git(resolve(sourceRoot), ['archive', '--format=tar', commit, '--', SLICE_ROOTS, ...roots, ...SLICE_EXTRA_INPUTS], 512 << 20);
  await mkdir(target, { mode: 0o755 });
  const unpacked = spawnSync('tar', ['-x', '-f', '-', '-C', target], { input: archive, maxBuffer: 1 << 20 });
  if (unpacked.status !== 0) {
    await rm(target, { recursive: true, force: true });
    fail(`the slice build context did not unpack: ${unpacked.stderr?.toString().trim()}`);
  }
  return { output: target, sourceCommit: commit, files: (await files(target)).length };
}

/** Checks #678's signing receipt covers every executable with the bytes being bundled. */
function verifyMacosReceipt(receipt, digests) {
  if (receipt?.schema !== MACOS_RECEIPT_SCHEMA) fail(`the macOS signing receipt must have schema ${MACOS_RECEIPT_SCHEMA}`);
  if (typeof receipt.identity !== 'string' || !receipt.identity.startsWith('Developer ID Application: '))
    fail('the macOS signing receipt must name a Developer ID Application identity');
  if (receipt.notarization?.status !== 'Accepted') fail('the macOS signing receipt has no accepted notarization');
  const signed = new Map((receipt.files ?? []).map(file => [file.path, file]));
  for (const [path, digest] of digests) {
    const file = signed.get(path) ?? fail(`${path} is not in the macOS signing receipt`);
    if (file.role === 'data') fail(`${path} was not codesigned`);
    if (file.sha256 !== digest) fail(`${path} differs from the codesigned and notarized bytes`);
  }
  return { identity: receipt.identity, teamId: receipt.teamId, notarySubmissionId: receipt.notarization.submissionId };
}

function modeOf(path, sourceMode) {
  if (path.startsWith('runtime/')) return sourceMode & 0o555; // the signed runtime is read-only
  if (path.startsWith('bin/') || path.startsWith('libexec/') || path === 'install.sh') return 0o755;
  return sourceMode & 0o111 ? 0o755 : 0o644;
}

async function place(source, output, path) {
  const target = join(output, path);
  await mkdir(dirname(target), { recursive: true, mode: 0o755 });
  await copyFile(source, target);
  await chmod(target, modeOf(path, (await stat(source)).mode));
}

async function writeManifest(output, header) {
  const entries = [];
  for (const path of await files(output)) {
    const bytes = await readFile(join(output, path));
    entries.push({ path, size: bytes.length, sha256: sha256(bytes), mode: ((await stat(join(output, path))).mode & 0o777).toString(8).padStart(4, '0') });
  }
  const manifest = { schema: SCHEMA, ...header, files: entries };
  await writeFile(join(output, 'manifest.json'), `${JSON.stringify(JSON.parse(stableJson(manifest)), null, 2)}\n`, { mode: 0o644 });
  return manifest;
}

export async function assemble(options) {
  const platform = PLATFORMS[options.platform] ?? fail(`--platform must be one of ${Object.keys(PLATFORMS).join(', ')}`);
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(options.version ?? '')) fail('--version must be a semantic version');
  hex(options.sourceCommit, 40, '--source-commit');
  const output = resolve(options.output ?? fail('--output is required'));
  if (await lstat(output).then(() => true, () => false)) fail(`${output} already exists`);
  const sourceRoot = resolve(options.sourceRoot ?? repositoryRoot);
  const runtime = await verifyRuntime(resolve(options.runtime), options.runtimePublicKey, options.platform, options.sourceCommit);

  // --executables has the bundle's bin/ and libexec/ layout, the layout the macOS
  // signing tool keeps, so its receipt names the same paths.
  const executables = resolve(options.executables ?? fail('--executables is required'));
  const setup = await lstat(join(executables, "bin/chariox-setup")).then(() => ["bin/chariox-setup"], error => error.code === "ENOENT" ? [] : Promise.reject(error));
  const inputs = [...platform.bin.map(name => `bin/${name}`), ...setup, ...platform.libexec.map(name => `libexec/${name}`)]
    .map(path => [join(executables, path), path]);
  for (const [source, path] of inputs) {
    if (!((await regularFile(source, path)).mode & 0o100)) fail(`${path} is not executable: ${source}`);
  }
  const unexpected = (await files(executables)).filter(path => !inputs.some(([, expected]) => expected === path));
  if (unexpected.length) fail(`--executables has files a ${options.platform} bundle does not ship: ${unexpected.join(', ')}`);
  let macosSigning = null;
  if (options.platform.startsWith('darwin-')) {
    if (!options.macosSigningReceipt) fail('a macOS bundle needs --macos-signing-receipt from scripts/sign-macos-release.mjs');
    const receiptBytes = await readFile(resolve(options.macosSigningReceipt));
    const digests = new Map();
    for (const [source, path] of inputs) digests.set(path, sha256(await readFile(source)));
    macosSigning = { ...verifyMacosReceipt(JSON.parse(receiptBytes.toString('utf8')), digests), receiptSha256: sha256(receiptBytes) };
  } else if (options.macosSigningReceipt) fail('--macos-signing-receipt applies only to macOS bundles');

  const installer = [];
  if (platform.linuxInstaller) {
    const local = join(sourceRoot, 'deploy/local-linux');
    for (const name of ['install-root.sh', 'install-user.sh', 'start-kernel.sh', 'chariox-kernel.service', 'chariox-app-bwrap.apparmor'])
      await regularFile(join(local, name), `deploy/local-linux/${name}`);
    for (const path of await files(local)) installer.push([join(local, path), `deploy/local-linux/${path}`]);
    // install-root.sh installs these units from ../managed-kernel.
    const units = join(sourceRoot, 'deploy/managed-kernel');
    await regularFile(join(units, 'chariox-app-storage.service'), 'deploy/managed-kernel/chariox-app-storage.service');
    for (const name of ['chariox-app-storage.service', 'chariox-docker-admission-locks.service']) {
      if (await lstat(join(units, name)).then(() => true, () => false)) installer.push([join(units, name), `deploy/managed-kernel/${name}`]);
    }
    installer.push([join(sourceRoot, 'deploy/release-bundle/install.sh'), 'install.sh']);
  }
  installer.push([join(sourceRoot, 'LICENSE'), 'LICENSE']);
  const sliceContext = resolve(options.sliceBuildContext ?? fail('--slice-build-context is required'));
  for (const path of [SLICE_ROOTS, SLICE_PROVISIONER]) await regularFile(join(sliceContext, path), `the slice build context's ${path}`);

  await mkdir(dirname(output), { recursive: true });
  await mkdir(output, { mode: 0o755 });
  try {
    for (const [source, path] of [...inputs, ...installer]) {
      await regularFile(source, path);
      await place(source, output, path);
    }
    for (const path of await files(resolve(options.runtime))) await place(join(resolve(options.runtime), path), output, `runtime/${path}`);
    for (const path of await files(sliceContext)) await place(join(sliceContext, path), output, `${SLICE_CONTEXT}/${path}`);
    const manifest = await writeManifest(output, {
      name: 'chariox', version: options.version, platform: options.platform, sourceCommit: options.sourceCommit,
      runtime, macosSigning,
    });
    return { output, version: manifest.version, platform: manifest.platform, files: manifest.files.length,
      manifestSha256: sha256(await readFile(join(output, 'manifest.json'))), runtime };
  } catch (error) {
    await rm(output, { recursive: true, force: true });
    throw error;
  }
}

/** Checks the manifest signature, every listed file and that nothing else is present. */
export async function verifyBundle(bundle, keyHex) {
  const directory = resolve(bundle);
  const manifestBytes = await readFile(join(directory, 'manifest.json'));
  verifySignature(manifestBytes, await readFile(join(directory, 'manifest.sig')), keyHex, 'bundle manifest');
  return checkContents(directory, manifestBytes, true);
}

async function checkContents(directory, manifestBytes, signed) {
  const manifest = JSON.parse(manifestBytes.toString('utf8'));
  if (manifest.schema !== SCHEMA) fail(`manifest schema must be ${SCHEMA}`);
  const listed = new Map(manifest.files.map(file => [relativeFile(file.path), file]));
  const present = (await files(directory)).filter(path => path !== 'manifest.json' && (!signed || path !== 'manifest.sig'));
  for (const path of present) {
    const entry = listed.get(path) ?? fail(`${path} is not in the manifest`);
    const bytes = await readFile(join(directory, path));
    // Modes are recorded for the archive; an unpacking umask may narrow or widen them,
    // and the installers set their own.
    if (bytes.length !== entry.size || sha256(bytes) !== entry.sha256) fail(`${path} does not match the manifest`);
    listed.delete(path);
  }
  if (listed.size) fail(`bundle is missing ${[...listed.keys()].join(', ')}`);
  await verifyRuntime(join(directory, 'runtime'), manifest.runtime.publicKeyHex, manifest.platform, manifest.sourceCommit);
  if (manifest.runtime.inventorySha256 !== sha256(await readFile(join(directory, 'runtime/runtime-inventory.json'))))
    fail('the runtime inventory digest does not match the manifest');
  return { version: manifest.version, platform: manifest.platform, sourceCommit: manifest.sourceCommit,
    files: manifest.files.length, manifestSha256: sha256(manifestBytes), runtime: manifest.runtime };
}

export async function signBundle(bundle, signingKey) {
  const directory = resolve(bundle);
  const metadata = await stat(signingKey);
  if (metadata.uid !== process.getuid() || (metadata.mode & 0o077) !== 0)
    fail('the release signing key must be owned by the invoking user with no group or other permissions');
  if (await lstat(join(directory, 'manifest.sig')).then(() => true, () => false)) fail('the bundle is already signed');
  const manifestBytes = await readFile(join(directory, 'manifest.json'));
  const summary = await checkContents(directory, manifestBytes, false);
  const key = createPrivateKey(await readFile(signingKey));
  if (key.asymmetricKeyType !== 'ed25519') fail('the release signing key must be an Ed25519 private key');
  await writeFile(join(directory, 'manifest.sig'), signBytes(null, manifestBytes, key).toString('hex'), { mode: 0o644 });
  return { ...summary, publicKeyHex: publicKeyHex(key) };
}

const COMMANDS = {
  'slice-context': ['source-root', 'source-commit', 'output'],
  assemble: ['platform', 'version', 'source-commit', 'executables', 'runtime', 'runtime-public-key', 'slice-build-context', 'macos-signing-receipt', 'source-root', 'output'],
  sign: ['bundle', 'signing-key'],
  verify: ['bundle', 'public-key'],
};

export function parseArgs(argv) {
  const [command, ...rest] = argv;
  const allowed = COMMANDS[command] ?? fail(`usage: release-bundle.mjs ${Object.keys(COMMANDS).join('|')} [options]`);
  const options = {};
  for (let index = 0; index < rest.length; index += 2) {
    const name = rest[index]?.startsWith('--') ? rest[index].slice(2) : undefined;
    if (!name || !allowed.includes(name)) fail(`unknown argument for ${command}: ${rest[index]}`);
    const value = rest[index + 1];
    if (value === undefined || value.startsWith('--')) fail(`--${name} needs a value`);
    const key = name.replace(/-([a-z])/g, (_match, letter) => letter.toUpperCase());
    if (key in options) fail(`--${name} given twice`);
    options[key] = value;
  }
  return { command, options };
}

if (typeof Bun === "undefined" && process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const { command, options } = parseArgs(process.argv.slice(2));
    const result = command === 'slice-context' ? await exportSliceBuildContext(options.sourceRoot ?? repositoryRoot, options.sourceCommit, options.output)
      : command === 'assemble' ? await assemble(options)
      : command === 'sign' ? await signBundle(options.bundle ?? fail('--bundle is required'), options.signingKey ?? fail('--signing-key is required'))
        : await verifyBundle(options.bundle ?? fail('--bundle is required'), options.publicKey ?? fail('--public-key is required'));
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  } catch (error) {
    process.stderr.write(`release-bundle: ${error.message}\n`);
    process.exit(1);
  }
}
