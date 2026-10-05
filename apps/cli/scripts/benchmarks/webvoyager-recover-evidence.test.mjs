// MP-08/MP-10/MP-11: recovery cannot replay an admitted solver or rescore a valid judgement.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { assertUnactedRoomFailure, assertFailedJudge } from './webvoyager-recover-evidence.mjs'

const boundary = { effort: 'high', judgeEffort: 'low', model: 'gpt-6.1-sol', judgeModel: 'gpt-6.1-sol',
  maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000, sandboxCompatibility: false,
  cleanupValid: true, cleanup: { complete: true, sessionGone: true, attachmentDetached: true, slices: [{ gone: true }], errors: [] } }
const unacted = { ...boundary, firstFailingSeam: 'room_create', harnessValid: false, judgeValid: false, owned: { agentId: null } }
const failedJudge = { ...boundary, firstFailingSeam: 'official_prompt_judge', harnessValid: true, judgeValid: false,
  turnLifecycle: 'completed', promptId: 'original-prompt', providerToolCalls: 2, mutatingActions: 1, forbiddenTools: [], providerError: false,
  actions: [{ mode: 'browser' }], screenshots: ['screenshot1.png'], answer: 'original factual answer' }

test('MP-08/MP-10 admits only proven unacted and fully cleaned Room startup failure', () => {
  assertUnactedRoomFailure(unacted)
  for (const key of ['agentId', 'promptId', 'submissionRequestedAt', 'providerStartedAt', 'turnId', 'turnLifecycle']) {
    assert.throws(() => assertUnactedRoomFailure({ ...unacted, [key]: 'possible-provider-admission' }))
  }
  for (const patch of [{ owned: { agentId: 'possibly-spawned' } }, { providerToolCalls: 1 }, { actions: [{}] },
    { toolTrace: [{}] }, { firstFailingSeam: 'final_capture' }, { cleanupValid: false }, { sandboxCompatibility: true },
    { maxToolCalls: 81 }, { effort: 'low' }, { providerUnauthorized: true }]) assert.throws(() => assertUnactedRoomFailure({ ...unacted, ...patch }))
})
test('MP-08/MP-10 same-input judge recovery excludes valid, acted-budget and incomplete solver results', () => {
  assertFailedJudge(failedJudge)
  for (const patch of [{ judgeValid: true }, { harnessValid: false }, { firstFailingSeam: 'final_capture' },
    { providerError: true }, { providerToolCalls: 81 }, { mutatingActions: 16 }, { turnLifecycle: 'failed' },
    { screenshots: ['admission.png'] }, { actions: [{ mode: 'computer' }] }, { forbiddenTools: ['shell'] }, { cleanupValid: false }]) {
    assert.throws(() => assertFailedJudge({ ...failedJudge, ...patch }))
  }
})
