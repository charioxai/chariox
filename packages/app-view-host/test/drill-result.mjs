// MP-08 / MP-09 / MP-10 / MP-11 — one acceptance result for every strict capture.
import { assessNetwork, packetsPassed } from './network-assessment.mjs';

export function assessResults(results) {
  const captures = [results.isolationNetwork, ...results.attacks.map(item => item.runs[1].processNetwork),
    ...results.references.map(row => row.processNetwork), ...results.launch.rows.filter(row => row.control === false).map(row => row.network)];
  const exemptions = new Map();
  results.wholeTreePassed = captures.map(network => {
    const assessment = network ? assessNetwork(network, results.packetCapture) : { passed: false, exemptions: [] };
    if (network) network.assessment = assessment;
    for (const row of assessment.exemptions) exemptions.set(`${row.pid}:${row.tid}:${row.file}:${row.line}`, { directory: network.directory, ...row });
    return assessment.passed;
  }).every(Boolean) && (!('packetCapture' in results) || packetsPassed(results.packetCapture));
  results.networkExemptions = results.wholeTreePassed ? [...exemptions.values()] : [];
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
