// A linux-x64 (or, on macOS, darwin) runtime release input with a separately signed builder
// attestation, for tests of sign-app-runtime-release.mjs and its consumers.
import { execFileSync } from 'node:child_process';
import { generateKeyPairSync, sign } from 'node:crypto';
import { cp, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { bundleToolInputs, bundleSources, packageRuntime } from './package-app-runtime.mjs';
import { nativeBuildInputs } from './app-runtime-ci-receipt.mjs';
import { sha256, stableJson } from './app-runtime-bundle-files.mjs';
import { executable, launcherInputs, nativeExecutables, platformFiles, releasePaths } from './app-runtime-release-contract.mjs';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const git = (root, args) => execFileSync('git', ['-C', root, ...args],
  { encoding: 'utf8', timeout: 10000, stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const pem = (key, type) => key.export({ type, format: 'pem' });

// Text libraries and fake builder keys exercise signing/provenance only. This
// fixture neither executes native code nor claims a production runtime build.
// macOS fixtures compile tiny Mach-O files so codesign has real code to sign.
const macho = (path, source, dynamic) => execFileSync('/usr/bin/clang', [...(dynamic ? ['-dynamiclib'] : []),
  '-O2', '-x', 'c', '-', '-o', path], { input: source, timeout: 60000, stdio: ['pipe', 'pipe', 'pipe'] });

export async function runtimeReleaseFixture(t, target = 'linux-x64') {
  const darwin = target.startsWith('darwin-');
  const parent = join(homedir(), '.chariox/dev');
  await mkdir(parent, { recursive: true, mode: 0o700 });
  const root = await mkdtemp(join(await realpath(parent), 'runtime-release-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const source = join(root, 'source'); const input = join(root, 'input');
  const raw = join(root, 'raw');
  for (const path of [source, input, raw, join(input, 'native')]) await mkdir(path, { mode: 0o700 });
  const contract = JSON.parse(await readFile(join(repository, 'apps/app-worker/bundle.lock.json')));
  const sources = await bundleSources(repository, contract);
  for (const path of new Set([...sources.map(file => file.source), ...bundleToolInputs(target), ...launcherInputs(target)])) {
    await mkdir(dirname(join(source, path)), { recursive: true, mode: 0o700 });
    await cp(join(repository, path), join(source, path));
  }
  git(source, ['init', '-q']); git(source, ['add', '.']);
  git(source, ['-c', 'user.name=Release Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture']);
  const runtime = JSON.parse(await readFile(join(source, 'apps/app-worker/runtime.lock.json')));
  const files = [];
  const { nodeLibrary, runtimeLibrary } = runtime.targets[target];
  for (const path of [nodeLibrary, runtimeLibrary, 'NODE-LICENSE']) {
    if (darwin && path !== 'NODE-LICENSE') macho(join(raw, path), `int ${path === nodeLibrary ? 'node' : 'runtime'}(void) { return 7; }\n`, true);
    else await writeFile(join(raw, path), `Not executable: ${path}\n`, { mode: 0o600 });
    const bytes = await readFile(join(raw, path));
    files.push({ path, size: bytes.length, sha256: sha256(bytes) });
  }
  const artifact = {
    schema: 'chariox.app-runtime-artifact.v1', runtimeVersion: runtime.runtimeVersion, workerAbi: 1,
    target, nodeVersion: runtime.node.version, nodeModuleAbi: runtime.node.moduleAbi,
    source: { url: runtime.node.url, sha256: runtime.node.sha256 },
    sourceCommit: git(source, ['rev-parse', 'HEAD']), sourceTree: git(source, ['rev-parse', 'HEAD^{tree}']),
    buildInputs: await Promise.all(nativeBuildInputs(target).map(async path => ({ path, sha256: sha256(await readFile(join(source, path))) }))),
    toolchain: darwin ? { cc: 'Apple clang version 17.0.0 (fixture)', cxx: 'Apple clang version 17.0.0 (fixture)',
      python: 'Python 3.11.9', make: 'GNU Make 3.81\nfixture', xcode: 'Xcode 16.4\nfixture' }
      : { cc: '12.2.0', cxx: '12.2.0', python: 'Python 3.11.2', make: 'GNU Make 4.3\nfixture' },
    dependencies: darwin ? { [nodeLibrary]: `${nodeLibrary}:\n\t/usr/lib/libSystem.B.dylib (fixture)`,
      [runtimeLibrary]: `${runtimeLibrary}:\n\t@rpath/${nodeLibrary} (fixture)` }
      : { 'libnode.so.137': ' (NEEDED) Shared library: [libc.so.6]',
        'libchariox-app-runtime.so': ' (NEEDED) Shared library: [libnode.so.137]' },
    files, signing: { status: 'unsigned', notarization: 'not-performed' },
    validation: { nativeBuild: 'completed' }, loading: runtime.loading,
  };
  await writeFile(join(raw, 'artifact-manifest.json'), JSON.stringify(artifact));
  const bundle = await packageRuntime({ nativeDirectory: raw, output: join(input, 'bundle'), target, repository: source });
  for (const path of [...nativeExecutables(target), ...platformFiles(bundle.target)]) {
    await mkdir(dirname(join(input, 'native', path)), { recursive: true, mode: 0o700 });
    if (darwin) macho(join(input, 'native', path), 'int main(void) { return 0; }\n', false);
    else await writeFile(join(input, 'native', path), `Not executable: ${path}\n`, { mode: 0o600 });
  }
  const bundlePaths = new Set([...bundle.files.map(file => file.path), 'bundle-manifest.json']);
  const entries = await Promise.all(releasePaths(bundle).map(async path => {
    const bytes = await readFile(join(input, bundlePaths.has(path) ? 'bundle' : 'native', path));
    return { path, size: bytes.length, sha256: sha256(bytes), executable: executable(path) };
  }));
  const proof = { schema: 'chariox.app-runtime-release-build.v1', target: bundle.target, sourceCommit: bundle.sourceCommit,
    bundleDigest: bundle.bundleDigest,
    launcherBuildInputs: await Promise.all(launcherInputs(target).map(async path => ({ path, sha256: sha256(await readFile(join(source, path))) }))),
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
  return { options, proof, builder, release, root, input, bundle };
}
