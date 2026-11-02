// MP-08 / MP-09 / MP-10 / MP-11 AU-11: regression checks for the disposable service contract.
import assert from 'node:assert/strict'
import test from 'node:test'
import { startKeyService } from './service.mjs'

test('issued key authenticates, recorded resource revokes it, replacement works', async () => {
  const service = await startKeyService()
  try {
    const key = service.issue()
    assert.equal(await service.check(key), 200)
    const created = await fetch(`${service.origin}/resources`, { method: 'POST', headers: { authorization: `Bearer ${key.value}` } })
    assert.equal(created.status, 201)
    const resource = await created.json()
    assert.ok(resource.id)
    const revoked = await fetch(`${service.origin}${resource.revoke_path}`, { method: 'DELETE', headers: { authorization: `Bearer ${key.value}` } })
    assert.equal(revoked.status, 200)
    const body = await revoked.json()
    assert.ok(body.revoked && body.echo === key.value, 'private echo challenge is present')
    assert.equal(await service.check(key), 401)
    assert.equal(await service.check(service.issue()), 200)
    assert.ok(!JSON.stringify(service.events).includes(key.value), 'service receipts contain metadata only')
  } finally { await service.close() }
})
