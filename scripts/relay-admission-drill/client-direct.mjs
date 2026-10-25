import './client-env.mjs'
import { appendFileSync } from 'node:fs'
const repo = new URL('../..', import.meta.url).pathname.replace(/\/$/, '')
const { CloudClient } = await import(`${repo}/apps/cli/dist/cloud-client.js`)
const requests = await import(`${repo}/packages/kernel-client/dist/ipc-requests.js`)
const { client: localClient, unwrap, EVID, sleep } = await import('./lib.mjs')
const H = localClient(Number(process.env.H_PID)), W = localClient(Number(process.env.W_PID))
const cloud = new CloudClient(); const worker = await cloud.connect(process.env.ADMISSION_DRILL_WORKER_KERNEL)
const [sid, agent] = process.argv.slice(2)
const att = unwrap(await worker.send(requests.attachToSessionRequest(sid, 'amh-direct2')), 'SessionAttached').attachment.id
const homeBefore = JSON.stringify(unwrap(await H.send(requests.getSessionHistoryOutlineRequest('18e1e74b03ef4bd9', null, 20)), 'SessionHistoryOutline')).length
let r
try { r = await worker.send(requests.submitPromptRequest(sid, att, agent, 'Reply with exactly the single word direct-ok and nothing else.', [])); r = { ok: JSON.stringify(r).slice(0, 300) } } catch (e) { r = { refused: e.code, message: String(e.message).slice(0, 200) } }
console.log('submit on the worker backing session:', JSON.stringify(r))
await sleep(25000)
const st = JSON.stringify(unwrap(await worker.send(requests.getSessionStateRequest(sid)), 'SessionState')).includes('direct-ok')
const homeAfter = JSON.stringify(unwrap(await H.send(requests.getSessionHistoryOutlineRequest('18e1e74b03ef4bd9', null, 20)), 'SessionHistoryOutline')).length
const out = { submit: r, answer_in_worker_state: st, home_history_len_before: homeBefore, home_history_len_after: homeAfter }
console.log(JSON.stringify(out)); appendFileSync(`${EVID}/client-direct.jsonl`, JSON.stringify(out) + '\n')
cloud.stop(); process.exit(0)
