// MP-08 / MP-09 / MP-10 / MP-11: a real CLI timeout wake must retain its batch.
import test from 'node:test';
import assert from 'node:assert/strict';
const { execute } = await import(process.env.LANES_MODULE || '../lanes.mjs');
test('WaitEvents exit 5 returns the archived timeout batch', async () => {
  const result = await execute(['-e', 'console.log(JSON.stringify({events:[],next_cursor:7,timed_out:true})); process.exitCode=5'], '', process.execPath, false, [0, 5]);
  assert.deepEqual(result, { events: [], next_cursor: 7, timed_out: true });
});
test('execute rejects exit codes its caller has not explicitly accepted', async () => {
  await assert.rejects(execute(['-e', 'process.exitCode=3'], '', process.execPath, false, [0, 5]), { code: 'cli_failed' });
  await assert.rejects(execute(['-e', 'process.exitCode=5'], '', process.execPath), { code: 'cli_failed' });
});
