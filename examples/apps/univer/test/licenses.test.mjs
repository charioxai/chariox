import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { checkLicenses, inventory, root } from '../scripts/licenses.mjs';

test('the installed App dependencies and assets match the reviewed pinned inventory', async () => {
  const checked = await checkLicenses();
  assert.ok(checked.packages.length > 100);
  assert.equal(checked.packages.some(p => p.id.startsWith('@univerjs-pro/')), false);
  const published = JSON.parse(await readFile(path.join(root, 'licenses.json')));
  assert.deepEqual(checked, published);
});

test('licence gate rejects added packages, changed source and Pro packages', async () => {
  assert.ok(process.env.TMPDIR, 'set TMPDIR outside the checkout');
  const fixture = await mkdtemp(path.join(process.env.TMPDIR, 'univer-licenses-'));
  try {
    await mkdir(path.join(fixture, 'backend'));
    await mkdir(path.join(fixture, 'licenses'));
    await writeFile(path.join(fixture, 'licenses/supplemental.json'), '{}');
    await writeFile(path.join(fixture, 'backend/Cargo.lock'), '# fixture lock');
    await writeFile(path.join(fixture, 'pnpm-lock.yaml'), 'lockfileVersion: 9.0');
    await writeFile(path.join(fixture, 'package.json'), '{"dependencies":{"tiny":"1.0.0"}}');
    const install = async (name, text) => {
      const directory = path.join(fixture, 'node_modules/.pnpm', name.replaceAll('/', '+') + '@1.0.0', 'node_modules', name);
      await mkdir(directory, { recursive: true });
      await writeFile(path.join(directory, 'package.json'), JSON.stringify({ name, version: '1.0.0', license: 'MIT' }));
      await writeFile(path.join(directory, 'LICENSE'), 'MIT fixture notice');
      await writeFile(path.join(directory, 'index.js'), text);
      return directory;
    };
    const tiny = await install('tiny', 'export const value = 1;');
    await writeFile(path.join(fixture, 'licenses.json'), JSON.stringify(await inventory(fixture)));
    await checkLicenses(fixture);
    await writeFile(path.join(tiny, 'index.js'), 'export const value = 2;');
    await assert.rejects(checkLicenses(fixture), /Unlisted or changed/);
    await writeFile(path.join(tiny, 'index.js'), 'export const value = 1;');
    const extra = await install('extra', 'export {};');
    await assert.rejects(checkLicenses(fixture), /Unlisted or changed/);
    await rm(path.resolve(extra, '../..'), { recursive: true });
    await install('@univerjs-pro/sheets', 'export {};');
    await assert.rejects(checkLicenses(fixture), /Pro package forbidden/);
  } finally { await rm(fixture, { recursive: true, force: true }); }
});
