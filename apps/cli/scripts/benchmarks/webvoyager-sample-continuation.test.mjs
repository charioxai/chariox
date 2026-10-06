// MP-08/MP-10/MP-11: no failed-task replay or continuation over ambiguous ownership.
import test from 'node:test'
import assert from 'node:assert/strict'
import { validateSampleContinuation } from './webvoyager-sample-continuation.mjs'
test('MP-10 continuation selects untouched tasks and fences failed or ambiguous submissions', () => {
  const source = { image: 'image', runtimeSourceRevision: 'revision', protocol: '423', clientIpcSha256: 'client', artifacts: [] }
  const input = { source, sample: [{ taskId: 'done' }, { taskId: 'untouched' }] }
  const row = { taskId: 'done', runId: 'run', source, cleanupValid: true, model: 'gpt-6.1-sol', effort: 'high',
    judgeModel: 'gpt-6.1-sol', judgeEffort: 'low', maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000, promptVariant: 'recovery-v1' }
  const args = { input, priorInput: input, rows: [row], campaign: { finishedAt: 'done', completed: false, settled: 1,
    cleanup: { stateRemoved: true, workspaceRemoved: true, unresolvedRooms: [], unresolvedSlices: [], processes: [{ stopped: true }] } },
    admissions: [{ taskId: 'done', scope: 'wvanalysis-60', runId: 'run' }] }
  assert.deepEqual(validateSampleContinuation(args), [{ taskId: 'untouched' }])
  assert.throws(() => validateSampleContinuation({ ...args, admissions: [...args.admissions, { taskId: 'untouched' }] }), /ambiguous admission/)
  assert.throws(() => validateSampleContinuation({ ...args, rows: [row, row], campaign: { ...args.campaign, settled: 2 } }), /duplicate/)
  assert.throws(() => validateSampleContinuation({ ...args, priorInput: { ...input, sample: [] } }), /sample changed/)
  assert.throws(() => validateSampleContinuation({ ...args, rows: [{ ...row, cleanupValid: false }] }))
  assert.throws(() => validateSampleContinuation({ ...args, rows: [{ ...row, promptVariant: 'frozen' }] }))
})
test('MP-11 attachment-absence settlement preserves invalidity and rejects live provider ownership', () => {
  const source = { image: 'image', runtimeSourceRevision: 'revision', protocol: '423', clientIpcSha256: 'client', artifacts: [] }
  const input = { source, sample: [{ taskId: 'failed' }, { taskId: 'untouched' }] }, receipts = []
  const runtime = { root: 'root', workspace: 'workspace', cleanup: { unresolvedRooms: [], unresolvedSlices: [] } }
  const row = { taskId: 'failed', runId: 'run', source, cleanupValid: false, turnLifecycle: 'completed', providerError: false,
    cleanup: { sessionGone: true, slices: [{ gone: true }], errors: [{ seam: 'attachment_detach', errorCode: 'attachment_not_found' }] },
    containerGone: true, volumesGone: true, model: 'gpt-6.1-sol', effort: 'high', judgeModel: 'gpt-6.1-sol', judgeEffort: 'low',
    maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000, promptVariant: 'recovery-v1' }
  const settlement = { ...runtime, source, priorReceipts: receipts, providerSubmissions: 0, originalFailuresRetained: true,
    stateRemoved: true, workspaceRemoved: true, processes: [{ stopped: true }], processScanOwnedMatches: [],
    dockerContainersRemaining: 0, dockerVolumesRemaining: 0, settledTaskIds: ['failed'] }
  const args = { input, priorInput: input, rows: [row], campaign: { finishedAt: 'done', completed: false, settled: 1 },
    admissions: [{ taskId: 'failed', scope: 'wvanalysis-60', runId: 'run' }], settlement, runtime, receipts }
  assert.deepEqual(validateSampleContinuation(args), [{ taskId: 'untouched' }]); assert.equal(row.cleanupValid, false)
  for (const unsafe of [{ ...row, turnLifecycle: 'running' }, { ...row, judgeFailure: { cleanupFailed: true } },
    { ...row, cleanup: { ...row.cleanup, errors: [{ seam: 'room_delete', errorCode: 'failed' }] } }]) {
    assert.throws(() => validateSampleContinuation({ ...args, rows: [unsafe] }))
  }
  assert.throws(() => validateSampleContinuation({ ...args, settlement: { ...settlement, priorReceipts: ['changed'] } }))
})
