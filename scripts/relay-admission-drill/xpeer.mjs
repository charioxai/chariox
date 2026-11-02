// MP-08/MP-09/MP-10/MP-11 G09: a third real kernel (own CHARIOX_HOME) enrolled on the same account; the drill uses its identity as the "other kernel" sender.
import assert from 'node:assert/strict'
import { mkdir, writeFile } from 'node:fs/promises'
const { freePorts, kernelEnv, makeDirs, enrollDevice } = await import((process.env.AMRIG_RIG ?? '/root/.chariox/dev/amrig/rig') + '/worker.mjs')
const repo = new URL('../..', import.meta.url).pathname.replace(/\/$/, '')
const { spawnOwned } = await import(`${repo}/apps/kernel/slice-linux-docker/owned-process-signals.mjs`)
const { LocalIpcClient } = await import(`${repo}/packages/kernel-client/dist/ipc.js`)
const requests = await import(`${repo}/packages/kernel-client/dist/ipc-requests.js`)
const { chromium } = await import('/w/valenv/tools/node_modules/playwright-core/index.mjs')
const [origin, dir] = process.argv.slice(2)
const unwrap = (r, k) => { if (r.Error) throw new Error(JSON.stringify(r.Error).slice(0, 200)); assert.ok(r[k], `missing ${k}`); return r[k] }
await makeDirs(dir)
const ports = await freePorts(5)
const env = kernelEnv({ ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith('CHARIOX_'))), PATH: '/root/.local/bin:' + process.env.PATH }, dir, ports, { CHARIOX_DAEMON_ID: process.env.XPEER_ID ?? 'amh-xpeer', CHARIOX_DAEMON_ALIAS: 'amh xpeer', CHARIOX_MACHINE_ALIAS: 'amh xpeer' })
const proc = spawnOwned(process.env.XKERNEL, [], { cwd: repo, env, stdio: ['ignore', 'ignore', 'ignore'], detached: true })
const client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 15000 })
for (let i = 0; ; i++) { try { await client.send(requests.listSessionsRequest()); break } catch { assert.ok(i < 160, 'kernel start'); await new Promise(r => setTimeout(r, 250)) } }
const st = await enrollDevice({ client, requests, unwrap, chromium, origin, clientId: process.env.XPEER_ID ?? 'amh-xpeer', alias: 'amh xpeer' })
await writeFile(`${dir}/xpeer.json`, JSON.stringify({ pid: proc.pid, machineId: st.machine_id, ports }))
console.log(JSON.stringify({ pid: proc.pid, machineId: st.machine_id, kernelPort: ports[0] }))
process.exit(0)
