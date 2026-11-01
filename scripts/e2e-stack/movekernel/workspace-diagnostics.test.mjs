import assert from 'node:assert/strict';
import test from 'node:test';
import { checkpointDiagnostic } from './workspace-diagnostics.mjs';

test('MP-08/09/10/11 checkpoint diagnostics retain only static probe metadata', () => {
    const input = {component: 'provider.checkpoint', message: 'Native checkpoint probe failed',
        timestamp_ms: 123, provider_run_id: 'provider-run-2', stage: 'thread/resume',
        operation: 'thread/resume', failure_terms: ['missing field', 'model', 'private provider response'],
        error: 'private provider response', account: 'private account'};
    assert.deepEqual(checkpointDiagnostic(input), {timestamp_ms: 123, providerRunId: 'provider-run-2',
        stage: 'thread/resume', operation: 'thread/resume', failureTerms: ['missing field', 'model']});
    for (const key of ['provider_run_id', 'stage', 'operation']) {
        assert.equal(JSON.stringify(checkpointDiagnostic({...input, [key]: 'private value'})).includes('private'), false);
    }
    assert.equal(checkpointDiagnostic({...input, component: 'other'}), undefined);
    assert.equal(checkpointDiagnostic({...input, message: 'private message'}), undefined);
    assert.equal(checkpointDiagnostic(null), undefined);
});
