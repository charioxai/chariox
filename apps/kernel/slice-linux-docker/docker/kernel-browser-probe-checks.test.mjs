import test from 'node:test';
import assert from 'node:assert/strict';
import { checks, readDetectionRows } from './kernel-browser-probe-checks.mjs';

const check = id => checks.find(c => c.id === id);
const intoliIds = ['user-agent', 'webdriver', 'chrome', 'permissions', 'plugins-length', 'languages'];
const sannysoftIds = ['user-agent', 'webdriver', 'advanced-webdriver', 'chrome', 'permissions', 'plugins-length', 'plugins-type', 'languages'];
const row = (id, text = 'ok', failed = false) => ({ name: id, id: `${id}-result`, text, failed });
// A page whose rows are read by the check; `rows` may change between reads to model a page that is still running.
const pageWith = (...snapshots) => {
  let reads = 0;
  return { navigate: async () => {}, evaluate: async () => JSON.stringify(snapshots[Math.min(reads++, snapshots.length - 1)]), reads: () => reads };
};
const fast = { attempts: 3, interval: 1 };

for (const [id, ids] of [['sannysoft', sannysoftIds], ['intoli', intoliIds]]) {
  test(`MP-08/10/11 ${id}: a completed table with no failed row is the outcome none`, async () => {
    const result = await check(id).run(pageWith(ids.map(i => row(i))), fast);
    assert.equal(result.outcome, 'none');
  });

  test(`MP-08/10/11 ${id}: failed rows are named once every result is in`, async () => {
    const result = await check(id).run(pageWith(ids.map(i => row(i, 'x', i === 'webdriver' || i === 'chrome'))), fast);
    assert.equal(result.outcome, 'webdriver, chrome');
  });

  test(`MP-08/10/11 ${id}: an empty table is incomplete, never a pass`, async () => {
    await assert.rejects(check(id).run(pageWith([]), fast), /incomplete/);
  });

  test(`MP-08/10/11 ${id}: an error or challenge page without the result rows is incomplete`, async () => {
    await assert.rejects(check(id).run(pageWith([{ name: 'Just a moment...', id: '', text: '', failed: false }]), fast), /incomplete/);
  });

  test(`MP-08/10/11 ${id}: a partial table (a result still empty or missing) is incomplete`, async () => {
    await assert.rejects(check(id).run(pageWith(ids.map(i => row(i, i === 'permissions' ? '' : 'ok'))), fast), /incomplete/);
    await assert.rejects(check(id).run(pageWith(ids.slice(0, -2).map(i => row(i))), fast), /incomplete/);
  });

  test(`MP-08/10/11 ${id}: a table that finishes after the first read is waited for`, async () => {
    const page = pageWith(ids.slice(0, 3).map(i => row(i)), ids.map(i => row(i)));
    assert.equal((await check(id).run(page, fast)).outcome, 'none');
    assert.equal(page.reads(), 2);
  });
}

test('MP-08/10/11 readDetectionRows stops after the attempts and says which rows are missing', async () => {
  const page = pageWith(intoliIds.slice(0, 2).map(i => row(i)));
  await assert.rejects(readDetectionRows(page, intoliIds.map(i => `${i}-result`), fast), /incomplete.*chrome-result/);
  assert.equal(page.reads(), 3);
});
