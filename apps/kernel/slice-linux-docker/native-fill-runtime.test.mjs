// MP-08/MP-11: native insertion and observation must see the shipped AT-SPI bindings.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

test('MP-08/MP-11: Computer helpers can import distro AT-SPI bindings', async () => {
  const dockerfile = await readFile(new URL('./docker/Dockerfile', import.meta.url), 'utf8');
  const screen = await readFile(new URL('./docker/slice-screen.sh', import.meta.url), 'utf8');
  const runtime = dockerfile.slice(dockerfile.lastIndexOf('\nFROM '));
  assert.match(runtime, /\n\s+python3-pyatspi\s+\\/);
  assert.match(runtime, /\n\s+python3-gi\s+\\/);
  assert.match(runtime, /python3 -m venv --system-site-packages \/opt\/chariox-selkies/);
  assert.match(runtime, /\/opt\/chariox-selkies\/bin\/python -c ['"]import pyatspi/);
  assert.match(screen, /\/opt\/chariox-selkies\/bin\/python .*slice-keyboard\.py["'] secret /);
  assert.match(screen, /(?:\/opt\/chariox-selkies\/bin\/python|\/usr\/bin\/python3) .*slice-observation-mask\.py/);
  if (/\/usr\/bin\/python3 .*slice-observation-mask\.py/.test(screen)) assert.match(runtime, /\n\s+python3-pil\s+\\/);
});
