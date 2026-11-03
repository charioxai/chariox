// MP-08 / MP-09 / MP-10 / MP-11 AU-15: validate the live drill's service contract.
import assert from 'node:assert/strict'
import test from 'node:test'
import { createAuthService } from './service.mjs'
const post = (origin, route, body) => fetch(origin + route, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) })
const token = (origin, body) => fetch(origin + '/oauth2/token', { method: 'POST', body: new URLSearchParams(body) })

test('auth.md local discovery, anonymous exchange and accepted Bearer call', async t => {
  const service = await createAuthService(); t.after(() => service.close())
  const metadata = await (await fetch(service.origin + '/.well-known/oauth-protected-resource')).json()
  assert.equal(metadata.resource, service.origin + '/')
  const registered = await (await post(service.origin, '/agent/identity', { type: 'anonymous' })).json()
  assert.equal(registered.identity_assertion.split('.').length, 3, 'service issues a signed JWT assertion')
  const tampered = await token(service.origin, { grant_type: 'urn:ietf:params:oauth:grant-type:jwt-bearer', assertion: registered.identity_assertion + 'x', resource: metadata.resource })
  assert.equal(tampered.status, 400)
  const issued = await (await token(service.origin, { grant_type: 'urn:ietf:params:oauth:grant-type:jwt-bearer', assertion: registered.identity_assertion, resource: metadata.resource })).json()
  const api = await fetch(service.origin + '/api/check', { headers: { authorization: 'Bearer ' + issued.access_token } })
  assert.equal(api.status, 200)
  assert.deepEqual(await api.json(), { accepted: true })
  assert.throws(() => service.assertAbsent(issued.access_token), /credential appeared/)
  service.assertAbsent(JSON.stringify(service.counts))
})

test('auth.md Service Auth requires claim completion before credentials', async t => {
  const service = await createAuthService(); t.after(() => service.close())
  const registered = await (await post(service.origin, '/agent/identity', { type: 'service_auth', login_hint: 'owner@example.test' })).json()
  assert.ok(!registered.identity_assertion)
  const pending = await token(service.origin, { grant_type: 'urn:workos:agent-auth:grant-type:claim', claim_token: registered.claim_token })
  assert.equal(pending.status, 400)
  assert.equal((await pending.json()).error, 'authorization_pending')
  const attempt = new URL(registered.claim.verification_uri).searchParams.get('attempt')
  const completed = await post(service.origin, '/claim/complete', { attempt, email: 'owner@example.test', user_code: registered.claim.user_code })
  assert.equal(completed.status, 200)
  const response = await token(service.origin, { grant_type: 'urn:workos:agent-auth:grant-type:claim', claim_token: registered.claim_token })
  assert.equal(response.status, 200)
  const issued = await response.json()
  assert.ok(Boolean(issued.access_token))
  service.assertAbsent(JSON.stringify(service.counts))
})

test('auth.md issuer-only service advertises no in-scope flow', async t => {
  const service = await createAuthService({ identityOnly: true }); t.after(() => service.close())
  const metadata = await (await fetch(service.origin + '/.well-known/oauth-authorization-server')).json()
  assert.deepEqual(metadata.agent_auth.identity_types_supported, ['identity_assertion'])
  assert.equal((await post(service.origin, '/agent/identity', { type: 'anonymous' })).status, 400)
  assert.equal(service.counts.registrations, 0)
})
