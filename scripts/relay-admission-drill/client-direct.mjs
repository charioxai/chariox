// G09: a second client of the same account reaches the leased worker kernel through the relay and tries the hidden backing session
// of a leased agent directly (attach, prompt, state read, close). Usage: node client-direct.mjs <backing session> <backing agent> <home session>
// The worker must refuse every request with `leased_backing_session_refused` naming the home session, and the home history must not move.
import './client-env.mjs'
import { appendFileSync } from 'node:fs'
const repo = new URL('../..', import.meta.url).pathname.replace(/\/$/, '')
const { CloudClient } = await import(`${repo}/apps/cli/dist/cloud-client.js`)
const requests = await import(`${repo}/packages/kernel-client/dist/ipc-requests.js`)
const { client: localClient, unwrap, EVID, sleep } = await import('./lib.mjs')
const H = localClient(Number(process.env.H_PID))
const cloud = new CloudClient(); const worker = await cloud.connect(process.env.ADMISSION_DRILL_WORKER_KERNEL)
const [sid, agent, homeSid] = process.argv.slice(2)
const homeLen = async () => JSON.stringify(unwrap(await H.send(requests.getSessionHistoryOutlineRequest(homeSid, null, 20)), 'SessionHistoryOutline')).length
const attempt = async (step, request) => {
  let outcome
  try { outcome = { answered: JSON.stringify(await worker.send(request)).slice(0, 240) } } catch (e) { outcome = { refused: e.code, message: String(e.message).slice(0, 260) } }
  const rec = { step, ...outcome }; console.log(JSON.stringify(rec)); appendFileSync(`${EVID}/client-direct.jsonl`, JSON.stringify({ at: new Date().toISOString(), ...rec }) + '\n'); return rec
}
const before = await homeLen()
const recs = [
  await attempt('attach', requests.attachToSessionRequest(sid, 'amh-direct3')),
  await attempt('prompt', requests.submitPromptRequest(sid, 'attachment-forged', agent, 'Reply with exactly the single word direct-ok and nothing else.', [])),
  await attempt('state', requests.getSessionStateRequest(sid)),
]
await sleep(15000)
const after = await homeLen()
const out = { all_refused_with_home_session: recs.every(r => r.refused === 'leased_backing_session_refused' && r.message.includes(homeSid)), home_history_len_before: before, home_history_len_after: after }
console.log(JSON.stringify(out)); appendFileSync(`${EVID}/client-direct.jsonl`, JSON.stringify(out) + '\n')
cloud.stop(); process.exit(0)
