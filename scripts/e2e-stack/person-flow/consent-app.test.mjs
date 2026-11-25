import assert from 'node:assert/strict'
import test from 'node:test'
import { startConsentApp } from './consent-app.mjs'

const post = (origin, path, fields) => fetch(new URL(path, origin), {
  method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(fields),
})
const open = async origin => {
  const page = await (await fetch(`${origin}/authorize?state=s1`)).text()
  return /data-grant="([a-f0-9]+)"/.exec(page)[1]
}

test('the code reaches only the owner phone and the confirmation needs it', async t => {
  const app = await startConsentApp()
  t.after(() => app.close())
  const grant = await open(app.origin)
  assert.equal((await fetch(`${app.origin}/owner-phone`)).status, 401, 'the phone refuses anyone without the owner token')
  assert.equal((await fetch(`${app.origin}/owner-phone`, { headers: { authorization: `Bearer ${app.ownerToken}` } })).status, 404, 'no code before Allow')
  assert.equal((await post(app.origin, `/grants/${grant}/confirm`, { code: '000000' })).status, 409, 'confirm before Allow is refused')
  assert.equal((await post(app.origin, `/grants/${grant}/allow`, {})).status, 200)
  const phone = await fetch(`${app.origin}/owner-phone`, { headers: { authorization: `Bearer ${app.ownerToken}` } })
  const { code } = await phone.json()
  assert.match(code, /^\d{6}$/)
  const wrong = await post(app.origin, `/grants/${grant}/confirm`, { code: code === '111111' ? '222222' : '111111' })
  assert.equal(wrong.status, 422)
  assert.match((await wrong.json()).error, /not accepted/)
  const right = await post(app.origin, `/grants/${grant}/confirm`, { code })
  assert.equal(right.status, 200)
  const { location } = await right.json()
  assert.match(await (await fetch(new URL(location, app.origin))).text(), /Connected as test-owner/)
  assert.equal((await post(app.origin, `/grants/${grant}/confirm`, { code })).status, 409, 'a grant is consumed once')
  const status = await (await fetch(`${app.origin}/status`)).json()
  assert.deepEqual(status.events.map(e => e.kind), ['consent_shown', 'allow_clicked', 'code_sent', 'code_rejected', 'confirmed', 'callback_seen'])
  assert.ok(!JSON.stringify(status).includes(code), 'the status log never carries the code')
})

test('three wrong codes lock the grant even for the right code', async t => {
  const app = await startConsentApp()
  t.after(() => app.close())
  const grant = await open(app.origin)
  await post(app.origin, `/grants/${grant}/allow`, {})
  const { code } = await (await fetch(`${app.origin}/owner-phone`, { headers: { authorization: `Bearer ${app.ownerToken}` } })).json()
  const wrong = code === '111111' ? '222222' : '111111'
  for (let i = 0; i < 3; i++) assert.equal((await post(app.origin, `/grants/${grant}/confirm`, { code: wrong })).status, i < 2 ? 422 : 423)
  assert.equal((await post(app.origin, `/grants/${grant}/confirm`, { code })).status, 423)
  assert.equal((await (await fetch(`${app.origin}/status`)).json()).events.at(-1).kind, 'locked')
})

test('Deny ends at the callback with access_denied and never issues a code', async t => {
  const app = await startConsentApp()
  t.after(() => app.close())
  const grant = await open(app.origin)
  const denied = await post(app.origin, `/grants/${grant}/deny`, {})
  assert.equal(denied.status, 200)
  const { location } = await denied.json()
  assert.match(await (await fetch(new URL(location, app.origin))).text(), /Access denied/)
  assert.equal((await fetch(`${app.origin}/owner-phone`, { headers: { authorization: `Bearer ${app.ownerToken}` } })).status, 404)
  assert.equal((await post(app.origin, `/grants/${grant}/allow`, {})).status, 409)
})
