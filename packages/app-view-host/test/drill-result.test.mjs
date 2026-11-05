// MP-08 / MP-09 / MP-10 / MP-11 — strict launch trees participate in acceptance.
import test from 'node:test';
import assert from 'node:assert/strict';
import { assessResults } from './drill-result.mjs';

const kinds = ['idle', 'literal', 'dns', 'frame-csp'];
function passingResults() {
  return {
    isolationNetwork: { passed: true },
    attacks: [{ name: 'fetch', blocked: true, redCapable: true, framePolicyPresent: true,
      runs: [{ processNetwork: { passed: false } }, { processNetwork: { passed: true } }] }],
    references: Array.from({ length: 4 }, () => ({ rendered: true, processNetwork: { passed: true } })),
    launch: { passed: true, rows: kinds.flatMap(kind => [
      { kind, control: false, network: { passed: true } },
      ...(kind === 'idle' ? [] : [{ kind, control: true, network: { passed: false } }]),
    ]) },
    isolation: { passed: true }, sourceBinding: { passed: true }, rejectedBeforeExecution: true,
  };
}

test('passing strict captures accept while deliberately weakened controls stay outside the gate', () => {
  const results = passingResults();
  assessResults(results);
  assert.equal(results.wholeTreePassed, true);
  assert.equal(results.containmentPassed, true);
  assert.deepEqual(results.failedChecks, []);
});

for (const kind of kinds) for (const failure of ['nonLoopback', 'unidentified', 'incomplete']) {
  test(`${kind}: a failed ${failure} capture rejects containment independently of launch controls`, () => {
    const results = passingResults();
    results.launch.rows.find(row => row.kind === kind && row.control === false).network = { passed: false, [failure]: 1 };
    assessResults(results);
    assert.equal(results.launch.passed, true);
    assert.equal(results.rendererPassed, true);
    assert.equal(results.wholeTreePassed, false);
    assert.equal(results.containmentPassed, false);
    assert.deepEqual(results.failedChecks, ['whole-browser-tree-zero-network']);
  });
}
