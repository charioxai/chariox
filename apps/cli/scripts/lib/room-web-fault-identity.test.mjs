// MP-08 / MP-10: standard Browser packets permit ephemeral keys; stale admission must change scope.
import assert from 'node:assert/strict'
import { createHmac } from 'node:crypto'
import test from 'node:test'
import { faultRelayCredential } from './room-web-fault-matrix.mjs'
test('stale relay identity faults admission scope instead of optional Browser key metadata', () => {
  const ready = { relayScopedSecret: 'owned-test-fixture' }
  const claims = { realm_id: 'local-dev', expires_at_ms: Date.now() + 60000, public_key_thumbprint: '1'.repeat(64) }
  const payload = Buffer.from(JSON.stringify(claims)).toString('base64url')
  const original = `chariox-scoped-v1.${payload}.${createHmac('sha256', ready.relayScopedSecret).update(payload).digest('base64url')}`
  const [prefix, changed, signature] = faultRelayCredential(original, ready, 'stale-identity').split('.')
  const value = JSON.parse(Buffer.from(changed, 'base64url'))
  assert.equal(prefix, 'chariox-scoped-v1')
  assert.notEqual(value.realm_id, claims.realm_id)
  assert.equal(value.public_key_thumbprint, claims.public_key_thumbprint)
  assert.equal(signature, createHmac('sha256', ready.relayScopedSecret).update(changed).digest('base64url'))
})
