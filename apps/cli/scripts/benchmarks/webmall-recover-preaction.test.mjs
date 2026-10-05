// MP-08/MP-10/MP-11: never turn an ambiguous or acted attempt into a setup retry.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { assertUnactedSliceStart } from './webmall-recover-preaction.mjs'
const setup = { status: 'RED', firstFailingSeam: 'slice_start', cleanup: ['owned_room_deleted', 'owned_slice_deleted'] }
test('MP-08/MP-10 accepts only unacted, settled slice-start failure', () => assertUnactedSliceStart(setup))
test('MP-08/MP-10 rejects every provider admission marker, including ambiguous submit', () => {
  for (const key of ['promptId', 'providerStartedAt', 'submissionRequestedAt', 'agentId', 'turnId', 'lifecycle']) {
    assert.throws(() => assertUnactedSliceStart({ ...setup, [key]: 'possible-action' }))
  }
  assert.throws(() => assertUnactedSliceStart({ ...setup, providerToolCalls: 1 }))
})
test('MP-08/MP-10/MP-11 rejects acted budget failure and unsettled cleanup', () => {
  assert.throws(() => assertUnactedSliceStart({ ...setup, firstFailingSeam: 'provider_settlement', providerToolCalls: 82 }))
  assert.throws(() => assertUnactedSliceStart({ ...setup, cleanup: ['owned_room_deleted'] }))
  assert.throws(() => assertUnactedSliceStart({ ...setup, providerUnauthorized: true }))
})
