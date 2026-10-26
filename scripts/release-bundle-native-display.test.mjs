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
  const install = workflow.match(/- name: Install native display build dependencies\n\s+if: matrix\.platform == 'linux-x64'\n\s+run: sudo apt-get install -y (.+)\n/);
  assert.ok(install, 'the Linux build job installs the native display build libraries');
  const ciPackages = ci.match(/run: sudo apt-get install -y xvfb (libx11-dev .+)\n/)?.[1];
  assert.equal(install[1], ciPackages, 'release and CI compile native-display against the same libraries');
});

test('macOS release kernels stay featureless', () => {
  assert.doesNotMatch(matrixEntry('darwin-arm64'), /kernel_features/);
});
