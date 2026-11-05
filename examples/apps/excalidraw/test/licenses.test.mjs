import test from 'node:test';
import assert from 'node:assert/strict';
import { verifyInventory, inspectInventory } from '../scripts/licenses.mjs';
import { sha256, files } from '../scripts/licenses.mjs';
import { readFile, stat } from 'node:fs/promises';
import { join } from 'node:path';

test('MP-08/MP-10/MP-11: reject an unlisted dependency or changed font', async () => {
  const actual = await inspectInventory();
  const inventory = structuredClone(actual);
  assert.doesNotThrow(() => verifyInventory(actual, inventory));
  delete inventory.packages[Object.keys(inventory.packages)[0]];
  assert.throws(() => verifyInventory(actual, inventory), /unlisted dependency/);
  const changed = structuredClone(actual);
  changed.assets['fonts/Virgil/Virgil-Regular.woff2'] = 'unreviewed';
  assert.throws(() => verifyInventory(changed, actual), /unreviewed asset/);
});

test('MP-08/MP-10/MP-11: the distributed Liberation font is the OFL replacement', async () => {
  assert(process.env.CHARIOX_EXCALIDRAW_BUNDLE, 'set the external bundle path');
  const actual = await inspectInventory();
  const distributed = sha256(await readFile(join(process.env.CHARIOX_EXCALIDRAW_BUNDLE, 'ui/fonts/Liberation/LiberationSans-Regular.woff2')));
  assert.equal(distributed, actual.replacementLiberation);
  assert.notEqual(distributed, actual.assets['fonts/Liberation/LiberationSans-Regular.woff2']);
});

test('MP-08/MP-10/MP-11: UI fits the browser asset budget', async () => {
  const ui = join(process.env.CHARIOX_EXCALIDRAW_BUNDLE, 'ui');
  const paths = await files(ui);
  let bytes = 0;
  for (const path of paths) bytes += (await stat(join(ui, path))).size;
  assert(bytes <= 8 * 1024 * 1024, `UI exceeds 8 MiB: ${bytes}`);
  assert(!paths.some(path => path.startsWith('fonts/Xiaolai/')));
  for (const path of paths.filter(path => path.endsWith('.js'))) assert(!(await readFile(join(ui, path), 'utf8')).includes('./fonts/Xiaolai/'));
});
