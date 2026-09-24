#!/usr/bin/env node
// Local developer runtime release: this machine is the builder. It compiles the
// launcher, assembles the bundle from a native artifact, signs a builder proof
// and the runtime inventory with private local keys, and prints the one-time
// root enrollment command. It never runs sudo or installs trust itself.
// Production releases use the separate builder and Developer ID signing path.
import { createPrivateKey, createPublicKey, generateKeyPairSync, sign } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { chmod, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { sha256, stableJson } from './app-runtime-bundle-files.mjs';
import { packageRuntime } from './package-app-runtime.mjs';
import { executable, launcherInputs, nativeExecutables, platformFiles, releasePaths } from './app-runtime-release-contract.mjs';
import { signRuntimeRelease } from './sign-app-runtime-release.mjs';

const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const KEYS = join(homedir(), '.chariox/dev/app-runtime-keys');

async function localKey(name) {
  await mkdir(KEYS, { recursive: true, mode: 0o700 });
  const path = join(KEYS, `${name}.pem`);
  const existing = await stat(path).catch(() => null);
  if (!existing) {
    const { privateKey } = generateKeyPairSync('ed25519');
    await writeFile(path, privateKey.export({ type: 'pkcs8', format: 'pem' }), { mode: 0o600, flag: 'wx' });
  }
  await chmod(path, 0o600);
  return path;
}

function compileLauncher(output) {
  const source = join(REPOSITORY, 'apps/app-worker/src');
  const result = spawnSync('/usr/bin/clang', ['-std=c11', '-Wall', '-Wextra', '-Werror', '-O2', '-I', source,
    ...['launcher.c', 'launch_record.c', 'launch_process.c', 'sandbox_macos.c'].map(file => join(source, file)),
    '-o', output], { encoding: 'utf8', timeout: 120_000, env: { PATH: '/usr/bin:/bin', LANG: 'C' } });
  if (result.status !== 0) throw new Error(`launcher compile failed: ${result.stderr}`);
}

export async function localRelease({ nativeDirectory, output, target }) {
  if (!target.startsWith('darwin-')) throw new Error('local releases are macOS developer runtimes');
  const scratch = await mkdtemp(join(homedir(), '.chariox/dev/app-runtime-local-'));
  try {
    const input = join(scratch, 'input');
    await mkdir(join(input, 'native'), { recursive: true, mode: 0o700 });
    await packageRuntime({ nativeDirectory: resolve(nativeDirectory), output: join(input, 'bundle'), target });
    for (const name of nativeExecutables(target)) compileLauncher(join(input, 'native', name));
    const bundle = JSON.parse(await readFile(join(input, 'bundle/bundle-manifest.json'), 'utf8'));
    const bundlePaths = new Set([...bundle.files.map(file => file.path), 'bundle-manifest.json']);
    const files = [];
    for (const path of releasePaths(bundle)) {
      const bytes = await readFile(join(input, bundlePaths.has(path) ? 'bundle' : 'native', path));
      files.push({ path, size: bytes.length, sha256: sha256(bytes), executable: executable(path) });
    }
    if (platformFiles(target).length) throw new Error('unexpected platform graph');
    const inputs = [];
    for (const path of launcherInputs(target)) inputs.push({ path, sha256: sha256(await readFile(join(REPOSITORY, path))) });
    const proof = Buffer.from(stableJson({ schema: 'chariox.app-runtime-release-build.v1', target,
      sourceCommit: bundle.sourceCommit, bundleDigest: bundle.bundleDigest, launcherBuildInputs: inputs, files }));
    const builderKey = createPrivateKey(await readFile(await localKey('local-builder')));
    const builderPublic = join(scratch, 'builder.pub.pem');
    await writeFile(builderPublic, createPublicKey(builderKey).export({ type: 'spki', format: 'pem' }));
    await writeFile(join(scratch, 'builder.json'), proof);
    await writeFile(join(scratch, 'builder.sig'), sign(null, proof, builderKey).toString('hex'));
    const receipt = await signRuntimeRelease({ inputDirectory: input, builderAttestation: join(scratch, 'builder.json'),
      builderSignature: join(scratch, 'builder.sig'), trustedBuilderKey: builderPublic,
      signingKey: await localKey('local-release'), output: resolve(output) });
    return { ...receipt, output: resolve(output),
      enroll: `sudo <chariox-app-runtime-install> install --source ${resolve(output)} --trusted-public-key-hex ${receipt.publicKeyHex} --inventory-sha256 ${receipt.inventorySha256}` };
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [native, output, target = 'darwin-arm64'] = process.argv.slice(2);
  if (!native || !output) {
    process.stderr.write('usage: app-runtime-local-release.mjs NATIVE_DIR OUTPUT_DIR [darwin-arm64]\n');
    process.exit(2);
  }
  process.stdout.write(`${stableJson(await localRelease({ nativeDirectory: native, output, target }))}\n`);
}
