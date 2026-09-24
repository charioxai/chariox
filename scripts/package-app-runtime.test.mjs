import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { chmod, cp, link, mkdir, mkdtemp, readFile, readdir, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { bundleToolInputs, bundleSources, packageRuntime, validateBundleContract, verifyBundle } from './package-app-runtime.mjs';
import { builderDirectory, inventory, sha256, stableJson } from './app-runtime-bundle-files.mjs';
import { currentInputs, nativeBuildInputs } from './app-runtime-ci-receipt.mjs';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const fixtures = new Set();
test.after(async () => { for (const root of fixtures) await rm(root, { recursive: true, force: true }); });

function git(root, args) {
  return execFileSync('git', ['-C', root, ...args], { encoding: 'utf8', timeout: 10000, stdio: ['ignore', 'pipe', 'pipe'] }).trim();
}
function commit(root) {
  git(root, ['add', '.']);
  git(root, ['-c', 'user.name=Bundle Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture source']);
}

async function fixture(target = 'linux-x64') {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'chariox-runtime-bundle-'));
  fixtures.add(root);
  const source = join(root, 'source'); const nativeDirectory = join(root, 'native');
  await mkdir(source, { mode: 0o700 }); await mkdir(nativeDirectory, { mode: 0o700 });
  const contract = validateBundleContract(JSON.parse(await readFile(join(repository, 'apps/app-worker/bundle.lock.json'), 'utf8')));
  const sources = await bundleSources(repository, contract);
  for (const file of new Set([...sources.map(file => file.source), ...bundleToolInputs(target)])) {
    await mkdir(dirname(join(source, file)), { recursive: true, mode: 0o700 });
    await cp(join(repository, file), join(source, file));
  }
  git(source, ['init', '-q']); commit(source);
  const runtime = JSON.parse(await readFile(join(source, 'apps/app-worker/runtime.lock.json'), 'utf8'));
  const selected = runtime.targets[target];
  const files = [];
  for (const name of [selected.nodeLibrary, selected.runtimeLibrary, 'NODE-LICENSE']) {
    const bytes = Buffer.from(`NOT EXECUTABLE: tiny unsigned artifact fixture ${name}\n`);
    await writeFile(join(nativeDirectory, name), bytes);
    files.push({ path: name, size: bytes.length, sha256: sha256(bytes) });
  }
  const manifest = {
    schema: 'chariox.app-runtime-artifact.v1', runtimeVersion: runtime.runtimeVersion,
    workerAbi: runtime.workerAbi, target, nodeVersion: runtime.node.version, nodeModuleAbi: runtime.node.moduleAbi,
    source: { url: runtime.node.url, sha256: runtime.node.sha256 },
    sourceCommit: git(source, ['rev-parse', 'HEAD']), sourceTree: git(source, ['rev-parse', 'HEAD^{tree}']),
    buildInputs: await Promise.all(nativeBuildInputs(target).map(async path => ({ path, sha256: sha256(await readFile(join(source, path))) }))),
    toolchain: selected.platform === 'linux'
      ? { cc: '12.2.0', cxx: '12.2.0', python: 'Python 3.11.2', make: 'GNU Make 4.3\nfixture' }
      : { cc: 'Apple clang version 17.0.0', cxx: 'Apple clang version 17.0.0', python: 'Python 3.11.9',
        make: 'GNU Make 3.81\nfixture', xcode: 'Xcode 16.4\nBuild version 16F6' },
    dependencies: selected.platform === 'linux'
      ? { [selected.nodeLibrary]: ' (NEEDED) Shared library: [libc.so.6]', [selected.runtimeLibrary]: ` (NEEDED) Shared library: [${selected.nodeLibrary}]` }
      : { [selected.nodeLibrary]: `${selected.nodeLibrary}:\n /usr/lib/libSystem.B.dylib (compatibility version 1.0.0)`,
        [selected.runtimeLibrary]: `${selected.runtimeLibrary}:\n @rpath/${selected.nodeLibrary} (compatibility version 1.0.0)` },
    files, signing: { status: 'unsigned', notarization: 'not-performed' },
    validation: { nativeBuild: 'completed', runtimeExecution: 'not-performed' }, loading: runtime.loading,
  };
  await writeFile(join(nativeDirectory, 'artifact-manifest.json'), JSON.stringify(manifest));
  return { root, source, nativeDirectory, manifest, options: { repository: source, nativeDirectory, target, output: join(root, 'bundle') } };
}

test('deterministic complete bundle contains pinned bootstrap/SDK graph and unchanged native proof', async () => {
  const f = await fixture();
  const before = currentInputs(f.source).input_hash;
  const first = await packageRuntime(f.options);
  const second = await packageRuntime({ ...f.options, output: join(f.root, 'second') });
  assert.equal(first.bundleDigest, second.bundleDigest);
  assert.deepEqual(await verifyBundle(f.options.output), first);
  assert.equal(first.native.inputStatus, 'matches-current-source');
  assert.equal(first.native.receiptInputHash, before);
  assert.equal(first.signing.enrollment, 'not-performed');
  assert.ok(first.files.some(file => file.path === 'bootstrap.cjs'));
  assert.ok(first.files.some(file => file.path === 'sdk/src/transport.js'));
  assert.ok(first.files.some(file => file.path === 'sdk/package.json'));
  assert.deepEqual(await readFile(join(f.options.output, 'native-artifact-manifest.json')),
    await readFile(join(f.nativeDirectory, 'artifact-manifest.json')));
});

test('bootstrap and SDK changes alter bundle identity without changing native receipt inputs', async () => {
  const f = await fixture();
  const nativeHash = currentInputs(f.source).input_hash;
  let prior = await packageRuntime(f.options);
  for (const [index, path] of ['apps/app-worker/src/bootstrap.cjs', 'packages/app-sdk/src/index.js'].entries()) {
    await writeFile(join(f.source, path), `${await readFile(join(f.source, path), 'utf8')}\n// recorded JS fixture change\n`);
    commit(f.source);
    assert.equal(currentInputs(f.source).input_hash, nativeHash);
    const next = await packageRuntime({ ...f.options, output: join(f.root, `changed-${index}`) });
    assert.notEqual(next.bundleDigest, prior.bundleDigest);
    assert.equal(next.native.receiptInputHash, nativeHash);
    prior = next;
  }
});

test('macOS artifacts use their exact native graph and retain deterministic unsigned bundle identity', async () => {
  const f = await fixture('darwin-arm64');
  const nativeHash = currentInputs(f.source, f.options.target).input_hash;
  const first = await packageRuntime(f.options);
  assert.deepEqual(await verifyBundle(f.options.output), first);
  assert.equal(first.native.receiptInputHash, nativeHash);
  assert.ok(first.buildInputs.some(input => input.path === 'apps/app-worker/macos-build-profile.json'));
  assert.ok(!first.buildInputs.some(input => input.path === '.github/workflows/app-runtime-native.yml'));
  const second = await packageRuntime({ ...f.options, output: join(f.root, 'second') });
  assert.equal(first.bundleDigest, second.bundleDigest);
  const source = join(f.source, 'apps/app-worker/src/bootstrap.cjs');
  await writeFile(source, `${await readFile(source, 'utf8')}\n// JS-only bundle revision\n`); commit(f.source);
  const changed = await packageRuntime({ ...f.options, output: join(f.root, 'changed') });
  assert.notEqual(changed.bundleDigest, first.bundleDigest);
  assert.equal(changed.native.receiptInputHash, nativeHash);
});

test('macOS graph omissions and Linux graph substitution cannot claim macOS provenance', async () => {
  for (const substitute of [false, true]) {
    const f = await fixture('darwin-arm64');
    const index = f.manifest.buildInputs.findIndex(input => input.path === 'scripts/app-runtime-macos-command.py');
    if (substitute) f.manifest.buildInputs[index].path = 'scripts/run-app-runtime-native-ci.sh';
    else f.manifest.buildInputs.splice(index, 1);
    await writeFile(join(f.nativeDirectory, 'artifact-manifest.json'), JSON.stringify(f.manifest));
    await assert.rejects(packageRuntime({ ...f.options, allowHistorical: true }), /provenance|digest/);
  }
});

test('macOS native changes require historical mode while unrelated Linux inputs do not affect its receipt', async () => {
  const f = await fixture('darwin-arm64');
  const before = currentInputs(f.source, f.options.target).input_hash;
  await mkdir(join(f.source, '.github/workflows'), { recursive: true });
  await writeFile(join(f.source, '.github/workflows/app-runtime-native.yml'), '# unrelated Linux-only change\n');
  commit(f.source);
  assert.equal(currentInputs(f.source, f.options.target).input_hash, before);
  await packageRuntime(f.options);
  const path = join(f.source, 'scripts/app-runtime-macos-command.py');
  await writeFile(path, `${await readFile(path, 'utf8')}\n# new native build owner revision\n`); commit(f.source);
  const options = { ...f.options, output: join(f.root, 'changed') };
  await assert.rejects(packageRuntime(options), /historical/);
  const changed = await packageRuntime({ ...options, allowHistorical: true });
  assert.equal(changed.native.inputStatus, 'historical-source-only');
  assert.equal(changed.native.receiptInputHash, null);
  assert.deepEqual(await verifyBundle(options.output), changed);
});

test('historical native sources require explicit evidence mode and cannot claim a current receipt', async () => {
  const f = await fixture();
  const input = join(f.source, 'apps/app-worker/src/node_runtime.cc');
  await writeFile(input, `${await readFile(input, 'utf8')}\n// new native revision\n`); commit(f.source);
  await assert.rejects(packageRuntime(f.options), /historical/);
  const bundle = await packageRuntime({ ...f.options, allowHistorical: true });
  assert.equal(bundle.native.inputStatus, 'historical-source-only');
  assert.equal(bundle.native.receiptInputHash, null);
  assert.equal(bundle.native.evidence, 'unsigned-native-manifest-only');
  assert.equal(bundle.signing.status, 'unsigned');
});

test('native source, target, version, dependency and file tampering is rejected', async () => {
  for (const mutate of [
    f => { f.manifest.buildInputs[0].sha256 = 'f'.repeat(64); },
    f => { f.manifest.buildInputs.pop(); },
    f => { f.manifest.target = 'linux-arm64'; },
    f => { f.manifest.nodeVersion = '0.0.0'; },
    f => { f.manifest.toolchain.cc = '999'; },
    f => { f.manifest.dependencies['libnode.so.137'] = '(NEEDED) Shared library: [unbundled.so]'; },
    f => { f.manifest.signing.status = 'signed'; },
  ]) {
    const f = await fixture(); mutate(f);
    await writeFile(join(f.nativeDirectory, 'artifact-manifest.json'), JSON.stringify(f.manifest));
    await assert.rejects(packageRuntime(f.options));
  }
  const f = await fixture();
  await writeFile(join(f.nativeDirectory, 'libnode.so.137'), 'tampered');
  await assert.rejects(packageRuntime(f.options), /mismatch/);
  assert.ok(!(await readdir(f.root)).includes('bundle'), 'failed partial bundle is cleaned');
});

test('missing/extra SDK sources and wrong package metadata cannot make incomplete bundles', async () => {
  for (const mode of ['extra', 'missing', 'metadata']) {
    const f = await fixture();
    if (mode === 'extra') await writeFile(join(f.source, 'packages/app-sdk/src/extra.js'), 'export {};');
    else if (mode === 'missing') await rm(join(f.source, 'packages/app-sdk/src/peer.js'));
    else {
      const path = join(f.source, 'packages/app-sdk/package.json');
      const metadata = JSON.parse(await readFile(path, 'utf8')); metadata.dependencies = { unpinned: '*' };
      await writeFile(path, JSON.stringify(metadata));
    }
    commit(f.source);
    await assert.rejects(packageRuntime(f.options), /SDK/);
  }
});

test('links, extra artifact files and output replacement are rejected without overwriting outputs', async () => {
  for (const mode of ['symlink', 'hardlink', 'extra', 'empty']) {
    const f = await fixture();
    const source = join(f.nativeDirectory, 'libnode.so.137');
    if (mode === 'symlink' || mode === 'hardlink') {
      const external = join(f.root, 'outside'); await cp(source, external); await rm(source);
      if (mode === 'symlink') await symlink(external, source); else await link(external, source);
    } else if (mode === 'empty') await mkdir(join(f.nativeDirectory, 'empty'));
    else await writeFile(join(f.nativeDirectory, 'extra.so'), 'unlisted');
    await assert.rejects(packageRuntime(f.options));
  }
  const f = await fixture();
  await mkdir(f.options.output); await writeFile(join(f.options.output, 'keep'), 'owned');
  await assert.rejects(packageRuntime(f.options));
  assert.equal(await readFile(join(f.options.output, 'keep'), 'utf8'), 'owned');
});

test('bundle integrity rejects modified, missing, duplicated and undeclared file inventories', async () => {
  for (const mode of ['contents', 'missing', 'extra', 'duplicate', 'native-proof-mismatch']) {
    const f = await fixture(); const manifest = await packageRuntime(f.options);
    if (mode === 'contents') {
      const file = join(f.options.output, 'sdk/src/index.js'); await chmod(file, 0o600); await writeFile(file, 'changed');
    } else if (mode === 'missing') await rm(join(f.options.output, 'bootstrap.cjs'));
    else if (mode === 'extra') await writeFile(join(f.options.output, 'extra.js'), 'unexpected');
    else {
      if (mode === 'duplicate') manifest.files.push(manifest.files[0]);
      else {
        const file = manifest.files.find(file => file.path === 'libnode.so.137');
        const bytes = Buffer.from('changed native bytes');
        await chmod(join(f.options.output, file.path), 0o600); await writeFile(join(f.options.output, file.path), bytes);
        file.size = bytes.length; file.sha256 = sha256(bytes);
      }
      const { bundleDigest, ...body } = manifest; void bundleDigest;
      manifest.bundleDigest = `sha256:${sha256(stableJson(body))}`;
      await chmod(join(f.options.output, 'bundle-manifest.json'), 0o600);
      await writeFile(join(f.options.output, 'bundle-manifest.json'), stableJson(manifest));
    }
    await assert.rejects(verifyBundle(f.options.output));
  }
});

test('rehashing a manifest cannot invent validation, provenance, or oversized trusted sources', async () => {
  for (const mode of ['validation', 'input-digest', 'historical-receipt', 'missing-source', 'source-mismatch', 'large-source', 'sdk-metadata']) {
    const f = await fixture(); const manifest = await packageRuntime(f.options);
    if (mode === 'validation') manifest.validation.embeddedRuntime = 'passed';
    else if (mode === 'input-digest') manifest.native.inputDigest = `sha256:${'0'.repeat(64)}`;
    else if (mode === 'historical-receipt') manifest.native.inputStatus = 'historical-source-only';
    else if (mode === 'missing-source') manifest.buildInputs.pop();
    else if (mode === 'source-mismatch') manifest.buildInputs.find(input => input.path.endsWith('bootstrap.cjs')).sha256 = '0'.repeat(64);
    else {
      const selected = mode === 'large-source' ? 'bootstrap.cjs' : 'sdk/package.json';
      const bytes = mode === 'large-source' ? Buffer.alloc(262145, 0x20) : Buffer.from(JSON.stringify({ name: '@chariox/app-sdk', version: '999' }));
      const file = manifest.files.find(file => file.path === selected);
      await chmod(join(f.options.output, selected), 0o600); await writeFile(join(f.options.output, selected), bytes);
      file.size = bytes.length; file.sha256 = sha256(bytes);
      const source = selected === 'bootstrap.cjs' ? 'apps/app-worker/src/bootstrap.cjs' : 'packages/app-sdk/package.json';
      manifest.buildInputs.find(input => input.path === source).sha256 = file.sha256;
    }
    const { bundleDigest, ...body } = manifest; void bundleDigest;
    manifest.bundleDigest = `sha256:${sha256(stableJson(body))}`;
    await chmod(join(f.options.output, 'bundle-manifest.json'), 0o600);
    await writeFile(join(f.options.output, 'bundle-manifest.json'), stableJson(manifest));
    await assert.rejects(verifyBundle(f.options.output));
  }
});

test('builder directories and streaming inventory enforce their declared trust and entry bounds', async () => {
  const f = await fixture();
  await chmod(f.nativeDirectory, 0o777);
  await assert.rejects(packageRuntime(f.options), /directory chain/);
  await chmod(f.nativeDirectory, 0o700);
  const unsafe = join(f.root, 'unsafe'); const nested = join(unsafe, 'nested');
  await mkdir(unsafe); await mkdir(nested);
  await chmod(unsafe, 0o777);
  await assert.rejects(builderDirectory(nested), /directory chain/);
  await chmod(unsafe, 0o700);
  for (let index = 0; index < 4; ++index) await writeFile(join(nested, `${index}`), 'fixture');
  await assert.rejects(inventory(nested, 1), /limit/);
});

test('hidden working-tree changes cannot impersonate committed bootstrap sources', async () => {
  const f = await fixture();
  const path = 'apps/app-worker/src/bootstrap.cjs';
  git(f.source, ['update-index', '--assume-unchanged', path]);
  await writeFile(join(f.source, path), '// not the committed bootstrap');
  assert.equal(git(f.source, ['status', '--porcelain', '--', path]), '');
  await assert.rejects(packageRuntime(f.options), /committed bytes/);
});
