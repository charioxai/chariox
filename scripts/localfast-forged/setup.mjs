// Pairs one drill kernel with a private pr-stack through the real device approval and writes its identity to a state file.
// usage: node setup.mjs <stack> <kernel-port> <kernel-dir> <alias> <statefile>   (kernel-dir = the drill kernel's CHARIOX_HOME parent)
import assert from 'node:assert/strict'
import { writeFileSync, readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import { repo } from './forged-client.mjs'
const [stack, port, kdir, alias, statefile] = process.argv.slice(2)
process.env.CHARIOX_HOME = `${kdir}/chariox`
const { LocalIpcClient } = await import(pathToFileURL(`${repo}/packages/kernel-client/dist/ipc.js`).href)
const { chromium } = await import('/w/valenv/tools/node_modules/playwright-core/index.mjs')
const { signInValidation } = await import('/w/valenv/scripts/sign-in.mjs')
const origin = 'https://' + JSON.parse(readFileSync(`/w/valenv/prs/${stack}/metadata.json`, 'utf8')).hostname
const sleep = ms => new Promise(r => setTimeout(r, ms))
const until = async (label, f, ms = 90000, every = 500) => { const end = Date.now() + ms; let last; while (Date.now() < end) { try { const v = await f(); if (v) return v } catch (e) { last = e } await sleep(every) } throw new Error('timed out: ' + label + (last ? ' ' + last.message : '')) }
const client = await until('kernel answers', async () => { const c = new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`); await c.send({ RelayStatus: null }); return c })
const rpc = async r => { const x = await client.send(r); if (x.Error) throw new Error(Object.keys(r)[0] + ': ' + x.Error.code + ' ' + String(x.Error.message).slice(0, 200)); return x }
let status = (await rpc({ RelayStatus: null })).RelayStatus.status
const browser = await chromium.launch({ executablePath: '/opt/google/chrome/chrome', headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] })
try {
  const page = await (await browser.newContext()).newPage()
  await signInValidation(page, origin, '/terminal?ui=os')
  const { login } = (await rpc({ StartCloudRelayLogin: { api_url: origin, client_id: alias, client_alias: alias, machine_id: status.machine_id, machine_alias: alias } })).CloudRelayLoginStarted
  assert.equal(new URL(login.verification_url).origin, origin)
  await page.goto(login.verification_url)
  await page.getByRole('button', { name: /^(Approve|Allow|Connect)( |$)/i }).first().click({ timeout: 30000 })
  await until('device approved', async () => { const r = await rpc({ PollCloudRelayLogin: { api_url: origin, device_code: login.device_code } }); assert.notEqual(r.CloudRelayLoginPolled.result.status, 'denied'); return r.CloudRelayLoginPolled.result.status === 'approved' }, 100000, 2000)
  await rpc({ ConnectCloudRelay: null })
  status = await until('on the relay', async () => { const s = (await rpc({ RelayStatus: null })).RelayStatus.status; return s.connected || s.state === 'connected' || s.relay_connected ? s : null }, 120000, 1000)
  const state = { stack, origin, port: Number(port), daemon: status.daemon_id, machine: status.machine_id, protocol: status.local_protocol ?? status.protocol_version ?? null, at: new Date().toISOString() }
  writeFileSync(statefile, JSON.stringify(state, null, 1)); console.log(JSON.stringify(state))
} finally { await client.close?.().catch(() => {}); await browser.close().catch(() => {}) }
