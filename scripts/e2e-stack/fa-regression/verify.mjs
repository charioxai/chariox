// Proves each FA fixture works with a real call and writes a receipt without secrets: node verify.mjs <receipt.json>
import { readFileSync, writeFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import assert from 'node:assert/strict'
const base = Number(process.env.FA_PORT_BASE ?? 18100)
const state = process.env.FA_STATE_DIR ?? `${process.env.HOME}/.chariox/dev/amunblock/fa`
const sec = Object.fromEntries(readFileSync(`${state}/secrets.env`, 'utf8').trim().split('\n').map(l => [l.slice(0, l.indexOf('=')), l.slice(l.indexOf('=') + 1)]))
const issuer = process.env.FA_OIDC_ISSUER ?? `http://127.0.0.1:${base + 2}/dex`
const redirect = (process.env.FA_REDIRECT_URIS ?? 'http://127.0.0.1:18200/callback').split(',')[0]
const receipt = {}
// Mailpit
{
  const subject = 'fa-verify-' + Date.now()
  const r = await fetch(`http://127.0.0.1:${base + 1}/api/v1/send`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ From: { Email: 'app@fa.invalid' }, To: [{ Email: 'alice@fa.invalid' }], Subject: subject, Text: 'verify' }) })
  assert.ok(r.ok, 'mailpit send ' + r.status)
  const list = await (await fetch(`http://127.0.0.1:${base + 1}/api/v1/messages`)).json()
  assert.ok(list.messages.some(m => m.Subject === subject), 'mailpit message listed')
  receipt.mailpit = { ok: true }
}
// dex: authorization code login for two users, distinct subjects
async function login(user, password) {
  const jar = new Map(); const cookie = () => [...jar].map(([k, v]) => `${k}=${v}`).join('; ')
  const hop = async (url, init = {}) => { const r = await fetch(url, { redirect: 'manual', ...init, headers: { ...(init.headers ?? {}), cookie: cookie() } }); for (const c of r.headers.getSetCookie?.() ?? []) { const [kv] = c.split(';'); jar.set(kv.split('=')[0], kv.slice(kv.indexOf('=') + 1)) } return r }
  let r = await hop(`${issuer}/auth?client_id=fa-app&redirect_uri=${encodeURIComponent(redirect)}&response_type=code&scope=${encodeURIComponent('openid email')}&state=s1`)
  let loc = new URL(r.headers.get('location'), issuer + '/')
  for (let i = 0; i < 5; i++) { r = await hop(loc.href); if (r.status === 200) break; loc = new URL(r.headers.get('location'), issuer + '/') }
  const html = await r.text()
  const form = html.match(/action="([^"]+)"/); assert.ok(form, 'login form'); const post = new URL(form[1].replace(/&amp;/g, '&'), issuer + '/')
  r = await hop(post.href, { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded' }, body: `login=${encodeURIComponent(user)}&password=${encodeURIComponent(password)}` })
  for (let i = 0; i < 6 && r.status >= 300 && r.status < 400; i++) { const next = new URL(r.headers.get('location'), issuer + '/'); if (next.href.startsWith(redirect)) { const code = next.searchParams.get('code'); assert.ok(code, 'code'); const t = await fetch(`${issuer}/token`, { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded', authorization: 'Basic ' + Buffer.from(`fa-app:${sec.FA_OIDC_CLIENT_SECRET}`).toString('base64') }, body: `grant_type=authorization_code&code=${code}&redirect_uri=${encodeURIComponent(redirect)}` }); const tj = await t.json(); assert.ok(tj.id_token, 'id_token ' + JSON.stringify(tj).slice(0, 80)); return JSON.parse(Buffer.from(tj.id_token.split('.')[1], 'base64url').toString()) } r = await hop(next.href) }
  throw new Error('oidc login did not reach redirect')
}
{
  const disc = await (await fetch(`${issuer}/.well-known/openid-configuration`)).json(); assert.equal(disc.issuer, issuer)
  const a = await login('alice@fa.invalid', sec.FA_ALICE_PASSWORD), b = await login('bob@fa.invalid', sec.FA_BOB_PASSWORD)
  assert.equal(a.email, 'alice@fa.invalid'); assert.equal(b.email, 'bob@fa.invalid'); assert.notEqual(a.sub, b.sub)
  receipt.oidc = { ok: true, issuer, distinctSubjects: true }
}
// Postgres through the container's psql
{
  const q = sql => execFileSync('docker', ['exec', '-e', `PGPASSWORD=${sec.FA_DB_PASSWORD}`, 'amunblock-fa-postgres', 'psql', '-h', '127.0.0.1', '-U', 'app', '-d', 'app', '-tAc', sql], { encoding: 'utf8' }).trim()
  q('create table if not exists fa_verify(id serial primary key, v text)'); q("insert into fa_verify(v) values ('x')")
  assert.ok(Number(q('select count(*) from fa_verify')) >= 1); q('drop table fa_verify'); receipt.postgres = { ok: true }
}
// stripe-mock
{
  const h = { authorization: 'Bearer sk_test_fa', 'content-type': 'application/x-www-form-urlencoded' }
  const c = await (await fetch(`http://127.0.0.1:${base + 4}/v1/payment_intents`, { method: 'POST', headers: h, body: 'amount=1000&currency=usd' })).json()
  assert.ok(c.id?.startsWith('pi_'), JSON.stringify(c).slice(0, 100))
  const g = await (await fetch(`http://127.0.0.1:${base + 4}/v1/payment_intents/${c.id}`, { headers: h })).json(); assert.ok(g.id?.startsWith('pi_'))
  receipt.stripeMock = { ok: true, mode: 'stripe-mock (not Stripe test mode)' }
}
receipt.date = new Date().toISOString()
writeFileSync(process.argv[2], JSON.stringify(receipt, null, 1)); console.log(JSON.stringify(receipt))
