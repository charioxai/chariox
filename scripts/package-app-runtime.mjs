#!/usr/bin/env node
// Assemble/verify unsigned runtime files only. Never download, build or execute.
import { execFileSync } from 'node:child_process';
import { lstat, realpath, rm, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parseJson } from '../packages/app-sdk/src/json.js';
import { currentInputs, fingerprint, NATIVE_BUILD_INPUTS } from './app-runtime-ci-receipt.mjs';
import { checkDependencies, checkToolchainVersions, validateLock } from './build-app-runtime.mjs';
import { digestAndCopy, inventory, outputDirectory, readSmall, relativeFile, sha256, stableJson } from './app-runtime-bundle-files.mjs';

const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const RUNTIME_LOCK = 'apps/app-worker/runtime.lock.json';
const BUNDLE_LOCK = 'apps/app-worker/bundle.lock.json';
const BASE_NATIVE_INPUTS = [RUNTIME_LOCK, 'apps/app-worker/src/runtime.h', 'apps/app-worker/src/node_runtime.cc',
  'scripts/build-app-runtime.mjs', 'scripts/app-runtime-ci-resources.mjs', 'scripts/run-app-runtime-native-ci.sh', '.github/workflows/app-runtime-native.yml'];
export const BUNDLE_TOOL_INPUTS = [BUNDLE_LOCK, 'scripts/package-app-runtime.mjs', 'scripts/app-runtime-bundle-files.mjs',
  'scripts/package-app-runtime.test.mjs', ...NATIVE_BUILD_INPUTS];
const MAX_MANIFEST = 262144;
const HEX = /^[a-f0-9]{64}$/;
const COMMIT = /^[a-f0-9]{40}$/;
const json = bytes => parseJson(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
const equal = (a, b) => stableJson(a) === stableJson(b);
const sorted = values => [...values].sort();
const orderInputs = inputs => [...inputs].sort((a, b) => a.path < b.path ? -1 : 1);
const inputDigest = inputs => `sha256:${sha256(stableJson(orderInputs(inputs)))}`;

function git(repository, args, bytes = false) {
  return execFileSync('git', ['-C', repository, ...args], { encoding: bytes ? undefined : 'utf8',
    maxBuffer: MAX_MANIFEST, timeout: 10000, stdio: ['ignore', 'pipe', 'pipe'] });
}

export function validateBundleContract(contract) {
  if (contract?.schema !== 'chariox.app-runtime-bundle-contract.v1'
    || !equal(contract.sdk, { name: '@chariox/app-sdk', version: '0.4.0', appContractVersion: 1, wireVersion: 1 })
    || !equal(sorted(contract.bootstrap ?? []), ['bootstrap-config.cjs', 'bootstrap.cjs'])
    || !equal(contract.limits, { files: 32, sourceFileBytes: 262144, manifestBytes: MAX_MANIFEST, bundleBytes: 536870912 })
    || !Array.isArray(contract.sdkFiles) || contract.sdkFiles.length > 24
    || new Set(contract.sdkFiles).size !== contract.sdkFiles.length
    || !['package.json', 'src/index.js', 'src/internal.js', 'src/transport.js'].every(file => contract.sdkFiles.includes(file)))
    throw new Error('invalid pinned runtime bundle contract');
  for (const file of contract.sdkFiles) {
    relativeFile(file);
    if (file !== 'package.json' && !/^src\/[a-z-]+\.(?:js|d\.ts)$/u.test(file)) throw new Error('invalid SDK source inventory');
  }
  return contract;
}

async function validateSdk(sdkRoot, contract) {
  const actual = ['package.json', ...(await inventory(join(sdkRoot, 'src'), 24)).map(file => `src/${file}`)];
  if (!equal(sorted(actual), sorted(contract.sdkFiles))) throw new Error('SDK source graph differs from its pinned inventory');
  const metadata = json(await readSmall(sdkRoot, 'package.json', contract.limits.sourceFileBytes));
  if (metadata.name !== contract.sdk.name || metadata.version !== contract.sdk.version || metadata.type !== 'module'
    || metadata.exports?.['.']?.default !== './src/index.js' || metadata.exports?.['./internal']?.default !== './src/internal.js'
    || Object.keys(metadata.dependencies ?? {}).length || Object.keys(metadata.optionalDependencies ?? {}).length
    || Object.keys(metadata.peerDependencies ?? {}).length) throw new Error('SDK package metadata differs from pinned self-contained graph');
}

export async function bundleSources(repository, contract) {
  await validateSdk(join(repository, 'packages/app-sdk'), contract);
  return mappedSources(contract);
}

function mappedSources(contract) {
  return [
    { source: 'LICENSE', path: 'CHARIOX-LICENSE' },
    { source: RUNTIME_LOCK, path: 'runtime.lock.json' }, { source: BUNDLE_LOCK, path: 'bundle.lock.json' },
    ...contract.bootstrap.map(file => ({ source: `apps/app-worker/src/${file}`, path: file })),
    ...contract.sdkFiles.map(file => ({ source: `packages/app-sdk/${file}`, path: `sdk/${file}` })),
  ].sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
}

async function nativeProvenance(repository, manifest, allowHistorical) {
  if (!COMMIT.test(manifest.sourceCommit) || !COMMIT.test(manifest.sourceTree)
    || git(repository, ['rev-parse', `${manifest.sourceCommit}^{tree}`]).trim() !== manifest.sourceTree
    || !Array.isArray(manifest.buildInputs) || manifest.buildInputs.length > NATIVE_BUILD_INPUTS.length
    || new Set(manifest.buildInputs.map(input => input.path)).size !== manifest.buildInputs.length
    || !BASE_NATIVE_INPUTS.every(path => manifest.buildInputs.some(input => input.path === path)))
    throw new Error('native build provenance is incomplete');
  for (const input of manifest.buildInputs) {
    if (!NATIVE_BUILD_INPUTS.includes(input.path) || !HEX.test(input.sha256)
      || sha256(git(repository, ['show', `${manifest.sourceCommit}:${input.path}`], true)) !== input.sha256)
      throw new Error('native build source digest mismatch');
  }
  const tree = git(repository, ['ls-tree', '-r', '-z', manifest.sourceCommit, '--', ...NATIVE_BUILD_INPUTS]).split('\0').filter(Boolean).map(line => {
    const match = /^(100644|100755) (blob) ([a-f0-9]{40})\t(.+)$/u.exec(line);
    if (!match) throw new Error('native provenance contains non-regular source');
    return { mode: match[1], type: match[2], sha: match[3], path: match[4] };
  });
  const currentIdentity = currentInputs(repository);
  const current = await Promise.all(NATIVE_BUILD_INPUTS.map(async path => ({ path,
    sha256: sha256(await readSmall(repository, path, MAX_MANIFEST)) })));
  const matches = equal(orderInputs(manifest.buildInputs), orderInputs(current))
    && tree.length === NATIVE_BUILD_INPUTS.length && fingerprint(tree) === currentIdentity.input_hash;
  if (!matches && !allowHistorical) throw new Error('native inputs are historical; explicit --allow-historical-native is required for unsigned evidence');
  return { sourceCommit: manifest.sourceCommit, sourceTree: manifest.sourceTree,
    inputStatus: matches ? 'matches-current-source' : 'historical-source-only',
    inputDigest: inputDigest(manifest.buildInputs),
    receiptInputHash: matches ? currentIdentity.input_hash : null,
    evidence: 'unsigned-native-manifest-only' };
}

function nativeFiles(manifest, runtime, target) {
  const selected = runtime.targets?.[target];
  if (!selected || manifest.schema !== 'chariox.app-runtime-artifact.v1'
    || manifest.target !== target || manifest.runtimeVersion !== runtime.runtimeVersion || manifest.workerAbi !== runtime.workerAbi
    || manifest.nodeVersion !== runtime.node.version || manifest.nodeModuleAbi !== runtime.node.moduleAbi
    || !equal(manifest.source, { url: runtime.node.url, sha256: runtime.node.sha256 })
    || !equal(manifest.signing, { status: 'unsigned', notarization: 'not-performed' })
    || manifest.validation?.nativeBuild !== 'completed'
    || manifest.loading?.mode !== 'sandbox-before-dlopen' || manifest.loading?.launcherLinksNode !== false
    || !Array.isArray(manifest.files) || manifest.files.length !== 3
    || !equal(sorted(manifest.files.map(file => file.path)), sorted([selected.nodeLibrary, selected.runtimeLibrary, 'NODE-LICENSE'])))
    throw new Error('native artifact does not match the pinned runtime contract');
  for (const file of manifest.files) {
    if (!Number.isSafeInteger(file.size) || file.size < 1 || file.size > 536870912 || !HEX.test(file.sha256))
      throw new Error('invalid native file inventory');
  }
  checkToolchainVersions(runtime.toolchains[selected.toolchain], manifest.toolchain ?? {}, selected.platform);
  for (const library of [selected.nodeLibrary, selected.runtimeLibrary]) {
    if (typeof manifest.dependencies?.[library] !== 'string') throw new Error('missing native library dependency evidence');
    checkDependencies(selected.platform, manifest.dependencies[library], selected);
  }
  return manifest.files;
}

export async function packageRuntime({ nativeDirectory, output, target, allowHistorical = false, repository = REPOSITORY }) {
  repository = await realpath(repository);
  nativeDirectory = await realpath(nativeDirectory);
  const contract = validateBundleContract(json(await readSmall(repository, BUNDLE_LOCK, MAX_MANIFEST)));
  const runtime = validateLock(json(await readSmall(repository, RUNTIME_LOCK, MAX_MANIFEST)));
  const sourceFiles = await bundleSources(repository, contract);
  const nativeBytes = await readSmall(nativeDirectory, 'artifact-manifest.json', MAX_MANIFEST);
  const native = json(nativeBytes);
  const files = nativeFiles(native, runtime, target);
  if (!equal(await inventory(nativeDirectory, contract.limits.files), sorted(['artifact-manifest.json', ...files.map(file => file.path)])))
    throw new Error('native artifact has missing or undeclared files');
  const provenance = await nativeProvenance(repository, native, allowHistorical);
  const selectedInputs = sorted([...new Set([...sourceFiles.map(file => file.source), ...BUNDLE_TOOL_INPUTS])]);
  if (git(repository, ['status', '--porcelain', '--', ...selectedInputs])) throw new Error('bundle sources must be committed');
  const sourceCommit = git(repository, ['rev-parse', 'HEAD']).trim();
  const buildInputs = await Promise.all(selectedInputs.map(async path => {
    const digest = sha256(await readSmall(repository, path, MAX_MANIFEST));
    if (digest !== sha256(git(repository, ['show', `${sourceCommit}:${path}`], true)))
      throw new Error('bundle source differs from its committed bytes');
    return { path, sha256: digest };
  }));
  let total = nativeBytes.length;
  const expected = files.reduce((sum, file) => sum + file.size, total);
  if (expected > contract.limits.bundleBytes) throw new Error('bundle exceeds byte ceiling');
  const owned = await outputDirectory(output, repository, nativeDirectory);
  try {
    const entries = [];
    for (const file of files) entries.push(await digestAndCopy(nativeDirectory, file.path, output, file.path, contract.limits.bundleBytes, file));
    entries.push(await digestAndCopy(nativeDirectory, 'artifact-manifest.json', output, 'native-artifact-manifest.json', MAX_MANIFEST,
      { size: nativeBytes.length, sha256: sha256(nativeBytes) }));
    for (const file of sourceFiles) {
      const entry = await digestAndCopy(repository, file.source, output, file.path, contract.limits.sourceFileBytes);
      if (entry.sha256 !== buildInputs.find(input => input.path === file.source).sha256) throw new Error('bundle source changed during assembly');
      entries.push(entry);
    }
    entries.sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
    total = entries.reduce((sum, file) => sum + file.size, 0);
    const body = {
      schema: 'chariox.app-runtime-bundle.v1', target, runtimeVersion: runtime.runtimeVersion,
      workerAbi: runtime.workerAbi, nodeVersion: runtime.node.version, nodeModuleAbi: runtime.node.moduleAbi,
      sdk: contract.sdk, sourceCommit, buildInputs, native: provenance, files: entries,
      signing: { status: 'unsigned', notarization: 'not-performed', enrollment: 'not-performed' },
      validation: { assembly: 'completed', embeddedRuntime: 'not-performed', containment: 'not-performed' },
    };
    const manifest = { ...body, bundleDigest: `sha256:${sha256(stableJson(body))}` };
    const bytes = `${stableJson(manifest)}\n`;
    if (entries.length + 1 > contract.limits.files || Buffer.byteLength(bytes) > MAX_MANIFEST
      || total + Buffer.byteLength(bytes) > contract.limits.bundleBytes) throw new Error('bundle exceeded inventory or byte ceiling');
    await writeFile(join(output, 'bundle-manifest.json'), bytes, { flag: 'wx', mode: 0o444 });
    return manifest;
  } catch (error) {
    const current = await lstat(output).catch(() => null);
    if (current?.dev === owned.dev && current?.ino === owned.ino) await rm(output, { recursive: true, force: true });
    throw error;
  }
}

export async function verifyBundle(directory) {
  directory = await realpath(directory);
  const manifestBytes = await readSmall(directory, 'bundle-manifest.json', MAX_MANIFEST);
  const manifest = json(manifestBytes);
  const { bundleDigest, ...body } = manifest;
  if (manifest.schema !== 'chariox.app-runtime-bundle.v1' || bundleDigest !== `sha256:${sha256(stableJson(body))}`
    || !equal(manifest.signing, { status: 'unsigned', notarization: 'not-performed', enrollment: 'not-performed' }))
    throw new Error('invalid unsigned bundle manifest or digest');
  const contract = validateBundleContract(json(await readSmall(directory, 'bundle.lock.json', MAX_MANIFEST)));
  const runtime = validateLock(json(await readSmall(directory, 'runtime.lock.json', MAX_MANIFEST)));
  const native = json(await readSmall(directory, 'native-artifact-manifest.json', MAX_MANIFEST));
  const libraries = nativeFiles(native, runtime, manifest.target);
  await validateSdk(join(directory, 'sdk'), contract);
  if (manifest.runtimeVersion !== runtime.runtimeVersion || manifest.workerAbi !== runtime.workerAbi
    || manifest.nodeVersion !== runtime.node.version || manifest.nodeModuleAbi !== runtime.node.moduleAbi
    || !equal(manifest.sdk, contract.sdk) || manifest.native?.sourceCommit !== native.sourceCommit
    || manifest.native?.sourceTree !== native.sourceTree || manifest.native?.evidence !== 'unsigned-native-manifest-only'
    || !['matches-current-source', 'historical-source-only'].includes(manifest.native?.inputStatus)
    || !COMMIT.test(manifest.sourceCommit) || !COMMIT.test(native.sourceCommit) || !COMMIT.test(native.sourceTree)
    || !Array.isArray(native.buildInputs) || native.buildInputs.length > NATIVE_BUILD_INPUTS.length
    || !BASE_NATIVE_INPUTS.every(path => native.buildInputs.some(input => input.path === path))
    || new Set(native.buildInputs.map(input => input.path)).size !== native.buildInputs.length
    || native.buildInputs.some(input => !NATIVE_BUILD_INPUTS.includes(input.path) || !HEX.test(input.sha256))
    || manifest.native.inputDigest !== inputDigest(native.buildInputs)
    || (manifest.native.inputStatus === 'historical-source-only' ? manifest.native.receiptInputHash !== null
      : !HEX.test(manifest.native.receiptInputHash))
    || !equal(manifest.validation, { assembly: 'completed', embeddedRuntime: 'not-performed', containment: 'not-performed' }))
    throw new Error('bundle metadata differs from its artifact contracts');
  const expected = sorted(['CHARIOX-LICENSE', 'runtime.lock.json', 'bundle.lock.json', 'native-artifact-manifest.json',
    ...libraries.map(file => file.path), ...contract.bootstrap, ...contract.sdkFiles.map(file => `sdk/${file}`)]);
  if (!Array.isArray(manifest.files) || !equal(manifest.files.map(file => file.path), expected)
    || !equal(await inventory(directory, contract.limits.files), sorted(['bundle-manifest.json', ...expected])))
    throw new Error('bundle contains missing, duplicate or undeclared files');
  const sources = mappedSources(contract);
  const expectedInputs = sorted([...new Set([...sources.map(file => file.source), ...BUNDLE_TOOL_INPUTS])]);
  if (!Array.isArray(manifest.buildInputs) || !equal(manifest.buildInputs.map(input => input.path), expectedInputs)
    || manifest.buildInputs.some(input => !HEX.test(input.sha256))
    || sources.some(file => manifest.buildInputs.find(input => input.path === file.source).sha256
      !== manifest.files.find(entry => entry.path === file.path).sha256)
    || manifest.native.inputStatus === 'matches-current-source' && (!equal(sorted(native.buildInputs.map(input => input.path)), sorted(NATIVE_BUILD_INPUTS))
      || native.buildInputs.some(input => manifest.buildInputs.find(current => current.path === input.path).sha256 !== input.sha256)))
    throw new Error('bundle source inventory differs from its provenance');
  let total = manifestBytes.length;
  for (const file of manifest.files) {
    if (file.mode !== '0444' || !HEX.test(file.sha256) || !Number.isSafeInteger(file.size) || file.size < 0)
      throw new Error('invalid bundle file metadata');
    total += file.size;
    if (total > contract.limits.bundleBytes) throw new Error('bundle exceeds byte ceiling');
    const ceiling = libraries.some(library => library.path === file.path)
      ? contract.limits.bundleBytes : contract.limits.sourceFileBytes;
    await digestAndCopy(directory, file.path, null, null, ceiling, file);
  }
  for (const file of libraries) {
    const packaged = manifest.files.find(entry => entry.path === file.path);
    if (file.size !== packaged.size || file.sha256 !== packaged.sha256) throw new Error('bundle differs from native build inventory');
  }
  return manifest;
}

function options(argv) {
  const mode = argv.shift();
  if (!['assemble', 'verify'].includes(mode)) throw new Error('expected assemble or verify');
  const values = {};
  for (let index = 0; index < argv.length; ++index) {
    const flag = argv[index];
    if (Object.hasOwn(values, flag)) throw new Error('duplicate option');
    if (flag === '--allow-historical-native') values[flag] = true;
    else if (['--native-dir', '--output', '--target', '--directory'].includes(flag) && argv[index + 1]) values[flag] = argv[++index];
    else throw new Error('invalid bundle option');
  }
  if (mode === 'verify' && !equal(Object.keys(values), ['--directory'])
    || mode === 'assemble' && (!values['--native-dir'] || !values['--output'] || !values['--target'] || values['--directory']))
    throw new Error('assemble requires --native-dir --output --target; verify requires --directory');
  return { mode, ...values };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const args = options(process.argv.slice(2));
    const manifest = args.mode === 'verify' ? await verifyBundle(args['--directory'])
      : await packageRuntime({ nativeDirectory: args['--native-dir'], output: args['--output'], target: args['--target'], allowHistorical: args['--allow-historical-native'] ?? false });
    process.stdout.write(`${stableJson({ bundleDigest: manifest.bundleDigest, target: manifest.target, signing: manifest.signing, native: manifest.native })}\n`);
  } catch { process.stderr.write('app_runtime_bundle_failed\n'); process.exitCode = 1; }
}
