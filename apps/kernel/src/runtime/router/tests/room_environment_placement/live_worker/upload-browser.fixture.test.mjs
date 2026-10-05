// MP-08/MP-10/MP-11: unsafe fixture IDs never reach a signal-zero probe.
import test from 'node:test';
import assert from 'node:assert/strict';
import { fixtureBrowserPid } from './upload-browser.fixture.mjs';
test('MP-08/MP-10/MP-11 fixture health probe rejects missing, system and noninteger PIDs', () => {
  for (const pid of [0,1,-1,undefined,NaN,Infinity,1.5,'20'])
    assert.throws(() => fixtureBrowserPid({browser:{pid}}));
  assert.throws(() => fixtureBrowserPid({}));
  assert.equal(fixtureBrowserPid({browser:{pid:20}}),20);
});
