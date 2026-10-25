// MP-08 / MP-10 / MP-11: real public projection casing and successful completion requirements.
import test from 'node:test';
import assert from 'node:assert/strict';
import { inspectWorker } from './worker-recovery.mjs';
const run = { key: 'lane-r1', launch: { sessionId: 'room', agentId: 'agent' } };
function rig(activity, providerState = 'Running') {
  return async request => request.GetSessionState ? { SessionState: {
    session: { permission_level: 'required' }, agents: [{ id: 'agent', provider: 'claude' }], agent_activity: { agent: activity },
  } } : { ProviderRun: { provider_run: { state: providerState, last_activity_at_ms: 1000 } } };
}
test('an active recovered provider uses the exact public Running state', async () => {
  const worker = await inspectWorker(rig({ status: 'working', active_turn: { provider_run_id: 'provider-run-1' } }), run);
  assert.equal(worker.providerActivityAtMs, 1000);
  assert.equal(worker.provider, 'claude'); assert.equal(worker.permission, 'required');
});
test('Starting and Ended providers cannot certify recovery', async () => {
  for (const state of ['Starting', 'Ended']) assert.equal((await inspectWorker(rig({ status: 'working', active_turn: { provider_run_id: 'provider-run-1' } }, state), run)).providerActivityAtMs, undefined);
});
test('only a successful completed turn is completion proof', async () => {
  for (const status of ['completed', 'cancelled', 'failed']) {
    const worker = await inspectWorker(rig({ status: 'idle', last_completed_turn: { settlement_status: status, completed_at_ms: 2000 } }), run);
    assert.equal(worker.completedSuccessfully, status === 'completed');
  }
});
