#!/usr/bin/env node
// Build only the portable backend. Signing/install use app-package::v2.
import { build, version as esbuildVersion } from 'esbuild';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { npmPackages } from './npm-notices.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const [entry, output] = process.argv.slice(2);
if (!entry || !output || process.argv.length !== 4) throw Error('usage: node build-js.mjs <entry.js|ts> <component.wasm>');
const root = process.env.CHARIOX_APP_TOOLCHAIN;
if (!root) throw Error('set CHARIOX_APP_TOOLCHAIN to the directory created by build-toolchain.py');
const pin = JSON.parse(await readFile(join(here, 'toolchain.json')));
const installed = JSON.parse(await readFile(join(root, 'toolchain-build.json')));
const compiler = join(root, 'componentize-qjs');
const hash = createHash('sha256').update(await readFile(compiler)).digest('hex');
if (installed.revision !== pin.quickjs.revision || installed.snapshotPatchSha256 !== pin.quickjs.snapshotPatchSha256 || installed.cancelPatchSha256 !== pin.quickjs.cancelPatchSha256 || installed.compilerSha256 !== hash) throw Error('QuickJS toolchain mismatch; rebuild with build-toolchain.py');
const inventoryHash = createHash('sha256').update(await readFile(join(root, 'notices/guest/inventory.json'))).digest('hex');
if (installed.licenseInventorySha256 !== inventoryHash) throw Error('QuickJS licence inventory mismatch; rebuild with build-toolchain.py');
const checker = process.env.CHARIOX_APP_COMPONENT_CHECK;
if (!checker) throw Error('set CHARIOX_APP_COMPONENT_CHECK to the chariox-app-component binary');
const scratchRoot = process.env.TMPDIR;
if (!scratchRoot) throw Error('set TMPDIR to a scratch directory outside the repository');
const scratch = await mkdtemp(join(scratchRoot, 'app-js-'));
const run = (command, args, cwd) => {
  const result = spawnSync(command, args, { cwd, encoding: 'utf8', timeout: 300000, maxBuffer: 4 * 1024 * 1024 });
  if (result.error) throw result.error;
  if (result.status !== 0) throw Error(`${command} failed (${result.status}): ${result.stderr}`);
};
try {
  const bundle = join(scratch, 'app.js'), component = join(scratch, 'app.wasm');
  const built = await build({
    entryPoints: [resolve(entry)], outfile: bundle, bundle: true,
    format: 'esm', platform: 'neutral', target: 'es2023', conditions: ['browser'], mainFields: ['browser', 'module', 'main'],
    // WIT imports stay component imports; Node builtins fail at the bundler.
    external: ['chariox:*', 'wasi:*'],
    alias: { '@chariox/app-sdk-wasm': resolve(here, '../app-sdk-js/src/index.js') },
    legalComments: 'eof', sourcemap: false, minify: true, metafile: true,
  });
  const npm = await npmPackages(built.metafile.inputs);
  const npmNotices = join(scratch, 'npm-notices');
  const licenseResult = spawnSync('python3', [join(here, 'licenses.py'), '--npm', npmNotices], { input: JSON.stringify(npm), encoding: 'utf8' });
  if (licenseResult.status !== 0) throw Error(`npm licence check failed: ${licenseResult.stderr}`);
  run(compiler, ['--wit', resolve(here, '../app-wit/wit'), '--world', 'app', '--js', 'app.js', '--module-root', '.', '-o', component], scratch);
  // Check actual component bytes, including dependencies' imports.
  const admitted = join(scratch, 'admitted.wasm');
  run(checker, [component, admitted]);
  const notices = {};
  const collect = async (directory, prefix) => {
    for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      const name = `${prefix}/${entry.name}`;
      if (entry.isDirectory()) await collect(join(directory, entry.name), name);
      else if (entry.isFile()) notices[name] = await readFile(join(directory, entry.name), 'utf8');
    }
  };
  await collect(join(root, 'notices/guest'), 'quickjs');
  const guestNotices = Object.fromEntries(Object.keys(notices).sort().map(key => [key.slice('quickjs/'.length), notices[key]]));
  if (createHash('sha256').update(JSON.stringify(guestNotices)).digest('hex') !== installed.guestNoticesSha256) throw Error('QuickJS notice contents mismatch; rebuild with build-toolchain.py');
  await collect(npmNotices, 'npm');
  notices['chariox-sdk/LICENSE'] = await readFile(resolve(here, '../../LICENSE'), 'utf8');
  const expectedEsbuild = JSON.parse(await readFile(join(here, 'package.json'), 'utf8')).dependencies.esbuild;
  if (esbuildVersion !== expectedEsbuild) throw Error('esbuild version mismatch; install the frozen pnpm lockfile');
  notices['chariox-sdk/BUILD.json'] = JSON.stringify({ sdkVersion: '0.1.0', rust: pin.rust, quickjs: pin.quickjs, esbuild: esbuildVersion });
  const leb = value => {
    const bytes = [];
    do { let byte = value & 127; value >>>= 7; if (value) byte |= 128; bytes.push(byte); } while (value);
    return Buffer.from(bytes);
  };
  const name = Buffer.from('chariox:licenses');
  const payload = Buffer.concat([leb(name.length), name, Buffer.from(JSON.stringify(notices))]);
  const withNotices = Buffer.concat([await readFile(admitted), Buffer.from([0]), leb(payload.length), payload]);
  await writeFile(admitted, withNotices);
  run(checker, [admitted]);
  // Cross-device rename is avoided: the admitted bytes are published beside
  // the output, then renamed so an existing backend survives failed builds.
  const temporary = `${resolve(output)}.pending-${process.pid}`;
  try {
    await writeFile(temporary, await readFile(admitted), { flag: 'wx' });
    await rename(temporary, resolve(output));
  } finally { await rm(temporary, { force: true }); }
} finally { await rm(scratch, { recursive: true, force: true }); }
