// MP-08 / MP-09 / MP-10 / MP-11 — strict launch trees participate in acceptance.
import test from 'node:test';
import assert from 'node:assert/strict';
import { assessResults } from './drill-result.mjs';
import { networkCases } from './network-cases.mjs';

const kinds = ['idle', 'literal', 'dns', 'frame-csp'];
function passingResults() {
  return {
    isolationNetwork: { passed: true },
    attacks: [1, 2].flatMap(dpr => networkCases('http://localhost').map(([name]) => ({
      name, dpr, blocked: true, redCapable: true, framePolicyPresent: true,
      runs: [{ processNetwork: { passed: false } }, { processNetwork: { passed: true } }],
    }))),
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

for (const [name, filter] of [
  ['subset filter', rows => rows.filter(row => row.name === 'fetch')],
  ['unmatched filter', () => []],
  ['missing DPR2', rows => rows.filter(row => row.dpr === 1)],
  ['duplicate replacing a missing case', rows => [...rows.slice(1), rows[1]]],
  ['extra duplicate', rows => [...rows, rows[0]]],
  ['unexpected case', rows => rows.map((row, index) => index ? row : { ...row, name: 'unexpected' })],
  ['unexpected DPR', rows => rows.map((row, index) => index ? row : { ...row, dpr: 3 })],
]) {
  test(`${name} cannot report full containment acceptance`, () => {
    const results = passingResults();
    results.attacks = filter(results.attacks);
    assessResults(results);
    assert.equal(results.wholeTreePassed, true);
    assert.equal(results.containmentPassed, false);
    assert.ok(results.failedChecks.includes('complete-attack-matrix'));
  });
}
