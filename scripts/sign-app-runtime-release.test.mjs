import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { generateKeyPairSync, sign, verify } from 'node:crypto';
import { chmod, cp, lstat, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { bundleToolInputs, bundleSources, packageRuntime } from './package-app-runtime.mjs';
import { nativeBuildInputs } from './app-runtime-ci-receipt.mjs';
import { sha256, stableJson } from './app-runtime-bundle-files.mjs';
import { codesignCopy } from './app-runtime-macos-codesign.mjs';
import { executable, LAUNCHER_INPUTS, launcherInputs, macosCodePaths, nativeExecutables, platformFiles, releasePaths } from './app-runtime-release-contract.mjs';
import { signRuntimeRelease } from './sign-app-runtime-release.mjs';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const git = (root, args) => execFileSync('git', ['-C', root, ...args],
  { encoding: 'utf8', timeout: 10000, stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const pem = (key, type) => key.export({ type, format: 'pem' });

// Text libraries and fake builder keys exercise signing/provenance only. This
// fixture neither executes native code nor claims a production runtime build.
// macOS fixtures compile tiny Mach-O files so codesign has real code to sign.
const macho = (path, source, dynamic) => execFileSync('/usr/bin/clang', [...(dynamic ? ['-dynamiclib'] : []),
  '-O2', '-x', 'c', '-', '-o', path], { input: source, timeout: 60000, stdio: ['pipe', 'pipe', 'pipe'] });

async function fixture(t, target = 'linux-x64') {
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

// macOS: codesign first, then sign the Chariox inventory over the signed bytes.
// These run real ad-hoc codesign on tiny compiled Mach-O files.
const onMac = process.platform === 'darwin' ? false : 'codesign runs on macOS';
const IDENTITY = 'Developer ID Application: Chariox Fixture (ABCDE12345)';
const real = command => {
  const result = spawnSync(command[0], command.slice(1), { encoding: 'utf8', timeout: 60000, stdio: ['ignore', 'pipe', 'pipe'] });
  return { status: result.status, stdout: result.stdout ?? '', stderr: result.stderr ?? '' };
};
// No Developer ID exists here. Signatures, their checks and entitlements are
// real and ad hoc; only the facts an Apple-issued identity and the notary
// service would supply are simulated: the identity's display lines, its
// certificate chain requirement, notarization and the Gatekeeper verdict.
function simulatedDeveloperId(command) {
  const [tool, ...args] = command;
  if (tool === '/usr/bin/codesign' && args.includes('--sign')) return real(command.map(word => word === IDENTITY ? '-' : word));
  if (tool === '/usr/bin/codesign' && args.includes('-R')) return { status: 0, stdout: '', stderr: '' };
  if (tool === '/usr/bin/codesign' && args[0] === '--display' && args[1] === '--verbose=2') {
    const result = real(command);
    return { ...result, stderr: `${result.stderr}Authority=${IDENTITY}\nTeamIdentifier=ABCDE12345\nTimestamp=fixture\n` };
  }
  if (tool === '/usr/bin/xcrun') return args[1] === 'submit'
    ? { status: 0, stdout: JSON.stringify({ id: 'fixture-submission', status: 'Accepted' }), stderr: '' }
    : { status: 1, stdout: '', stderr: 'no notary log for a fixture' };
  if (tool === '/usr/sbin/spctl') return { status: 0, stdout: '', stderr: `${args.at(-1)}: accepted\nsource=Notarized Developer ID\n` };
  return real(command);
}
const sourcePath = (bundle, path) => bundle.files.some(file => file.path === path) || path === 'bundle-manifest.json'
  ? `bundle/${path}` : `native/${path}`;
const codeFiles = bundle => macosCodePaths(bundle).map(path => ({ path: sourcePath(bundle, path),
  executable: executable(path), jit: path === 'chariox-app-worker' }));

// Offsets in a thin 64-bit Mach-O: the __text section, and the first code page
// hash in the embedded CodeDirectory.
function textOffset(bytes) {
  for (let index = 0, offset = 32; index < bytes.readUInt32LE(16); index += 1, offset += bytes.readUInt32LE(offset + 4)) {
    if (bytes.readUInt32LE(offset) !== 0x19) continue;
    for (let section = offset + 72; section < offset + 72 + 80 * bytes.readUInt32LE(offset + 64); section += 80)
      if (bytes.toString('latin1', section, section + 16).replace(/\0+$/u, '') === '__text') return bytes.readUInt32LE(section + 48);
  }
  throw new Error('no __text section');
}
function codeHashOffset(bytes) {
  const directory = bytes.indexOf(Buffer.from([0xfa, 0xde, 0x0c, 0x02]), textOffset(bytes));
  assert.ok(directory > 0);
  return directory + bytes.readUInt32BE(directory + 16);
}

async function adHoc(f, name = 'codesigned', code = codeFiles(f.bundle)) {
  const output = join(f.root, name);
  await codesignCopy({ input: f.input, output, identity: '-', code });
  return output;
}

async function assertSignedOver(f, codesigned, receipt) {
  const bytes = await readFile(join(f.options.output, 'runtime-inventory.json'));
  assert.equal(receipt.inventorySha256, sha256(bytes));
  assert.ok(verify(null, bytes, f.release.publicKey, Buffer.from(await readFile(join(f.options.output, 'runtime-inventory.sig'), 'utf8'), 'hex')));
  const inventory = JSON.parse(bytes);
  const code = macosCodePaths(f.bundle);
  assert.deepEqual(code, ['chariox-app-worker', 'libchariox-app-runtime.dylib', 'libnode.137.dylib']);
  assert.deepEqual(inventory.files.map(file => file.path), f.proof.files.map(file => file.path));
  for (const entry of inventory.files) {
    const attested = f.proof.files.find(file => file.path === entry.path);
    const signed = await readFile(join(codesigned, sourcePath(f.bundle, entry.path)));
    assert.equal(sha256(await readFile(join(f.options.output, entry.path))), entry.sha256);
    assert.equal(entry.sha256, sha256(signed));
    assert.equal(entry.size, signed.length);
    if (code.includes(entry.path)) assert.notEqual(entry.sha256, attested.sha256);
    else assert.deepEqual(entry, attested);
  }
  // Optional evidence for the installer's ignored cross-host test.
  const evidence = process.env.CHARIOX_CODESIGNED_RUNTIME_EVIDENCE;
  if (evidence) {
    await cp(f.options.output, join(evidence, 'runtime'), { recursive: true });
    await writeFile(join(evidence, 'release-receipt.json'), `${stableJson(receipt)}\n`);
  }
}

test('macOS developer release signs its inventory over codesigned bytes', { skip: onMac }, async t => {
  const f = await fixture(t, 'darwin-arm64');
  const codesigned = await adHoc(f);
  const receipt = await signRuntimeRelease({ ...f.options, developerRuntime: true, codesigned });
  assert.equal(receipt.codeSignatures, 'checked');
  await assertSignedOver(f, codesigned, receipt);
});

test('macOS production release: sign-macos-release.mjs, then the inventory over its output', {
  skip: onMac || (!await lstat(join(repository, 'scripts/sign-macos-release.mjs')).catch(() => null)
    && 'needs scripts/sign-macos-release.mjs from #678') }, async t => {
  const { signMacosRelease } = await import('./sign-macos-release.mjs');
  const f = await fixture(t, 'darwin-arm64');
  const codesigned = join(f.root, 'codesigned');
  const signing = await signMacosRelease({ input: f.input, output: codesigned, identity: IDENTITY,
    keychainProfile: 'chariox-fixture', teamId: 'ABCDE12345' }, { run: simulatedDeveloperId, platform: 'darwin' });
  assert.equal(signing.receipt.runtimeInventory, 'not-signed');
  const options = { ...f.options, codesigned, codesignIdentity: IDENTITY };
  await assert.rejects(signRuntimeRelease({ ...options, codesignIdentity: 'Developer ID Application: Other (ZZZZZ99999)' },
    { run: simulatedDeveloperId }), /not signed by the named Developer ID identity/);
  await assert.rejects(lstat(f.options.output), { code: 'ENOENT' });
  const receipt = await signRuntimeRelease(options, { run: simulatedDeveloperId });
  assert.equal(receipt.codeSignatures, 'checked-developer-id-notarized');
  for (const file of signing.receipt.files) {
    const entry = JSON.parse(await readFile(join(f.options.output, 'runtime-inventory.json'))).files
      .find(item => sourcePath(f.bundle, item.path) === file.path);
    assert.equal(entry.sha256, file.sha256);
  }
  await assertSignedOver(f, codesigned, receipt);
});

test('tampered or re-signed macOS code cannot be released', { skip: onMac }, async t => {
  const f = await fixture(t, 'darwin-arm64');
  const refuse = async (codesigned, pattern) => {
    await assert.rejects(signRuntimeRelease({ ...f.options, developerRuntime: true, codesigned }), pattern);
    await assert.rejects(lstat(f.options.output), { code: 'ENOENT' });
  };
  const library = 'bundle/libnode.137.dylib';
  // A changed code byte, with or without a fresh valid signature over it.
  const changed = await adHoc(f, 'changed');
  const bytes = await readFile(join(changed, library));
  bytes[textOffset(bytes)] ^= 1;
  await writeFile(join(changed, library), bytes);
  await refuse(changed, /outside the code signature/);
  assert.equal(real(['/usr/bin/codesign', '--force', '--sign', '-', '--options', 'runtime', join(changed, library)]).status, 0);
  await refuse(changed, /outside the code signature/);
  // A damaged signature over the attested code.
  const damaged = await adHoc(f, 'damaged');
  const signed = await readFile(join(damaged, library));
  signed[codeHashOffset(signed)] ^= 1;
  await writeFile(join(damaged, library), signed);
  await refuse(damaged, /signature check for libnode.137.dylib failed/);
  // A worker with more or fewer entitlements than JIT (dylibs carry none).
  const entitled = await adHoc(f, 'entitled');
  const extra = join(f.root, 'extra.entitlements');
  await writeFile(extra, (await readFile(new URL('./macos-app-worker.entitlements', import.meta.url), 'utf8'))
    .replace('<key>com.apple.security.cs.allow-jit</key>', '$&\n\t<true/>\n\t<key>com.apple.security.cs.disable-library-validation</key>'));
  assert.equal(real(['/usr/bin/codesign', '--force', '--sign', '-', '--options', 'runtime', '--entitlements', extra,
    join(entitled, 'native/chariox-app-worker')]).status, 0);
  await refuse(entitled, /unexpected entitlements: com.apple.security.cs.allow-jit, com.apple.security.cs.disable-library-validation/);
  const bare = await adHoc(f, 'bare', codeFiles(f.bundle).map(file => ({ ...file, jit: false })));
  await refuse(bare, /chariox-app-worker has unexpected entitlements: none/);
  // A missing hardened runtime, and an unsigned copy.
  const plain = await adHoc(f, 'plain');
  assert.equal(real(['/usr/bin/codesign', '--force', '--sign', '-', join(plain, library)]).status, 0);
  await refuse(plain, /lacks the hardened runtime/);
  const unsigned = join(f.root, 'unsigned');
  await cp(f.input, unsigned, { recursive: true });
  await refuse(unsigned, /signature check for|lacks the hardened runtime/);
  // A changed data file in the codesigned copy.
  const data = await adHoc(f, 'data');
  await chmod(join(data, 'bundle/bootstrap.cjs'), 0o644);
  await writeFile(join(data, 'bundle/bootstrap.cjs'), 'substituted');
  await refuse(data, /digest or size mismatch|bounded regular file/);
});

test('only macOS releases take a codesigned copy', async t => {
  const f = await fixture(t);
  await assert.rejects(signRuntimeRelease({ ...f.options, codesigned: f.input }), /only macOS releases/);
  await assert.rejects(lstat(f.options.output), { code: 'ENOENT' });
});
