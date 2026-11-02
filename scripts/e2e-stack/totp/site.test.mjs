import assert from 'node:assert/strict'
import test from 'node:test'
import { startTotpSite, totpAt } from './site.mjs'

async function post(origin, path, fields, cookie) {
  const response = await fetch(new URL(path, origin), {
    method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded', ...(cookie ? { cookie } : {}) },
    body: new URLSearchParams(fields), redirect: 'manual',
  })
  return { status: response.status, text: await response.text(), cookie: response.headers.get('set-cookie') }
}

test('enrollment rejects invalid codes; enabled TOTP signs in at the next time step', async t => {
  let now = 1_800_000_000_000
  const site = await startTotpSite({ now: () => now })
  t.after(() => site.close())
  const enrollment = await fetch(`${site.origin}/enable`)
  const sessionCookie = enrollment.headers.get('set-cookie').split(';', 1)[0]
  assert.doesNotMatch(await enrollment.text(), new RegExp(site.supplySeedToOwner()))
  assert.equal((await post(site.origin, '/enable', { code: 'not-a-code' }, sessionCookie)).status, 401)
  const seed = site.supplySeedToOwner()
  const enrolled = await post(site.origin, '/enable', { code: totpAt(Buffer.from(decodeBase32(seed)), now) }, sessionCookie)
  assert.equal(enrolled.status, 200)
  assert.match(enrolled.text, /enabled/)
  await fetch(`${site.origin}/logout`, { headers: { cookie: sessionCookie } })
  now += 30_000
  const loginCode = totpAt(Buffer.from(decodeBase32(seed)), now)
  const signedIn = await post(site.origin, '/login', { code: loginCode })
  assert.equal(signedIn.status, 200)
  assert.match(signedIn.text, /Signed in/)
  const replay = await post(site.origin, '/login', { code: loginCode })
  assert.equal(replay.status, 401)
  assert.match(replay.text, /Invalid or replayed code/)
  assert.deepEqual(site.acceptedPeriods(), [Math.floor(now / 30_000) - 1, Math.floor(now / 30_000)])
  assert.deepEqual(site.stats(), { enrollmentSuccesses: 1, loginSuccesses: 1, rejectedCodes: 2, replayRejections: 1, backupSuccesses: 0 })
})

test('a backup code can be used once as a sibling sign-in credential', async t => {
  const now = 1_800_000_000_000
  const site = await startTotpSite({ now: () => now })
  t.after(() => site.close())
  const enrollment = await fetch(`${site.origin}/enable`)
  const cookie = enrollment.headers.get('set-cookie').split(';', 1)[0]
  const code = totpAt(Buffer.from(decodeBase32(site.supplySeedToOwner())), now)
  assert.equal((await post(site.origin, '/enable', { code }, cookie)).status, 200)
  assert.deepEqual(site.acceptedPeriods(), [Math.floor(now / 30_000)])
  const backup = site.backupCodeForOwner()
  assert.equal((await post(site.origin, '/login', { backup })).status, 200)
  assert.equal((await post(site.origin, '/login', { backup })).status, 401)
  assert.equal(site.stats().backupSuccesses, 1)
  assert.equal(site.acceptedPeriods().length, 1, 'recovery does not count as a TOTP period')
})

test('absence checks cover factor values even before a successful submission', async t => {
  const now = 1_800_000_000_000
  const site = await startTotpSite({ now: () => now })
  t.after(() => site.close())
  const code = totpAt(Buffer.from(decodeBase32(site.supplySeedToOwner())), now)
  assert.equal(site.checkAbsence(code), false)
  assert.equal(site.checkAbsence(site.backupCodeForOwner()), false)
  assert.equal(site.checkAbsence('ordinary public tool output'), true)
})

function decodeBase32(text) {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'
  let bits = 0, value = 0
  const out = []
  for (const char of text) {
    value = (value << 5) | alphabet.indexOf(char); bits += 5
    if (bits >= 8) { out.push((value >>> (bits - 8)) & 255); bits -= 8 }
  }
  return out
}
