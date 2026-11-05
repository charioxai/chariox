// MP-08 / MP-09 / MP-10 / MP-11 — one acceptance result for every strict capture.
export function assessResults(results) {
  results.wholeTreePassed = results.isolationNetwork.passed && results.attacks.every(item => item.runs[1].processNetwork.passed)
    && results.references.every(row => row.processNetwork?.passed)
    && results.launch.rows.filter(row => row.control === false).every(row => row.network.passed);
  results.rendererPassed = results.references.length === 4 && results.references.every(row => row.rendered && !row.failure);
  results.containmentPassed = results.launch.passed && results.wholeTreePassed && results.attacks.every(item => item.blocked && item.redCapable && item.framePolicyPresent)
    && results.isolation.passed && results.sourceBinding.passed && results.rejectedBeforeExecution;
  results.failedChecks = results.attacks.filter(item => !item.blocked || !item.redCapable || !item.framePolicyPresent).map(item => item.name);
  if (!results.launch.passed) results.failedChecks.push('independent-launch-frame-controls');
  if (!results.wholeTreePassed) results.failedChecks.push('whole-browser-tree-zero-network');
  if (!results.isolation.passed) results.failedChecks.push('storage/native-input/compilation');
  if (!results.sourceBinding.passed) results.failedChecks.push('host-source-binding');
  if (!results.rejectedBeforeExecution) results.failedChecks.push('signature-before-execution');
}
