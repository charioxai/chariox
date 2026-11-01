import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const workflow = readFileSync(new URL('../.github/workflows/release-bundle.yml', import.meta.url), 'utf8');
const ci = readFileSync(new URL('../.github/workflows/ci.yml', import.meta.url), 'utf8');
const matrixEntry = platform => workflow.match(new RegExp(`- platform: ${platform}\\n((?:\\s{12}\\S.*\\n)+)`))?.[1] ?? '';

// MP-08/MP-10: the shipped Linux kernel resizes the Room browser through the native display worker,
// which is this same kernel ELF built with native-display; a featureless Linux kernel refuses every resize.
test('the Linux release kernel is built with native-display and its build libraries', () => {
  assert.match(matrixEntry('linux-x64'), /kernel_features: --features chariox-kernel\/native-display\n/);
  assert.match(workflow, /cargo build --release --locked \$\{\{ matrix\.kernel_features \}\} -p chariox-kernel /);
  const install = workflow.match(/- name: Install native display build dependencies\n\s+if: matrix\.platform == 'linux-x64'\n\s+run: \|\n\s+sudo apt-get update\n\s+sudo apt-get install -y (.+)\n/);
  assert.ok(install, 'the Linux build job installs the native display build libraries');
  const ciPackages = ci.match(/run: sudo apt-get install -y xvfb (libx11-dev .+)\n/)?.[1];
  assert.equal(install[1], ciPackages, 'release and CI compile native-display against the same libraries');
});

test('macOS release kernels stay featureless', () => {
  assert.doesNotMatch(matrixEntry('darwin-arm64'), /kernel_features/);
});

// MP-08/MP-10: exercise the actual ELF on a base image with no display stack.
test('the Linux release job executes the bundled kernel on minimal Ubuntu', () => {
  assert.match(workflow, /docker run --rm -v "\$PWD\/unsigned\/bin:\/bundle:ro" ubuntu:22\.04 \/bundle\/chariox-kernel --help/);
});

// MP-08/MP-09: a release kernel opens the WASM App worker only beside itself, so each platform's kernel build also
// builds the worker, in the same cargo invocation so both get one profile, one feature set and one tokio.
test('the release build compiles the WASM App worker with the kernel and ships it in bin/', () => {
  const build = workflow.match(/- name: Build the kernel, relay and helpers\n\s+run: \|\n((?:\s{10}.*\n)+)/)?.[1] ?? '';
  assert.match(build, /bins=\(chariox-kernel chariox-relay chariox-app-package chariox-app-wasm-worker /);
  assert.match(build, /cargo build --release --locked \$\{\{ matrix\.kernel_features \}\} -p chariox-kernel .*-p chariox-app-wasm-worker /);
  assert.match(build, /cp target\/release\/\{chariox-kernel,chariox-relay,chariox-app-package,chariox-app-wasm-worker\} unsigned\/bin\//);
});

test('the release build refuses a worker that does not report its identity', () => {
  assert.match(workflow, /unsigned\/bin\/chariox-app-wasm-worker identity/);
});

test('the macOS release job starts the signed worker under the hardened runtime', () => {
  const signed = workflow.indexOf('"$RUNNER_TEMP/signed/bin/chariox-app-wasm-worker" identity');
  assert.ok(signed > workflow.indexOf('node scripts/sign-macos-release.mjs --input "$RUNNER_TEMP/executables"'));
  assert.ok(signed < workflow.indexOf('- name: Assemble, sign and verify the bundle', workflow.indexOf('  macos:')));
});

// bun writes the executable's linked source maps beside it, and assemble refuses any file in bin/ that a bundle does not ship.
test('the release build moves the CLI source maps out of the executables', () => {
  const compile = workflow.indexOf('bun apps/cli/scripts/compile.mjs --target');
  const maps = workflow.indexOf('mv unsigned/bin/*.map "$RUNNER_TEMP/sourcemaps/"');
  assert.ok(maps > compile, 'the maps are moved after the compile');
  assert.ok(maps < workflow.indexOf('tar -czf unsigned-'), 'the maps are moved before the executables are archived');
});
