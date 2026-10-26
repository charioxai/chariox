// G09: a local terminal of the leased worker kernel (IPC with local auth, no Cloud login of its own) tries the hidden backing
// session of a leased agent directly (attach, prompt, state read). Usage: node local-direct.mjs <backing session> <backing agent> <home session>
// The worker must refuse every request with `leased_backing_session_refused` naming the home session, and the home history must not move.
import { appendFileSync } from 'node:fs'
const repo = new URL('../..', import.meta.url).pathname.replace(/\/$/, '')
const requests = await import(`${repo}/packages/kernel-client/dist/ipc-requests.js`)
const { client, unwrap, EVID, sleep } = await import('./lib.mjs')
const H = client(Number(process.env.H_PID)), W = client(Number(process.env.W_PID))
const [sid, agent, homeSid] = process.argv.slice(2)
const homeLen = async () => JSON.stringify(unwrap(await H.send(requests.getSessionHistoryOutlineRequest(homeSid, null, 20)), 'SessionHistoryOutline')).length
const attempt = async (step, request) => {
  let rec
  try {
    const r = await W.send(request)
    rec = r.Error ? { step, refused: r.Error.code ?? 'error', message: String(r.Error.message ?? JSON.stringify(r.Error)).slice(0, 260) } : { step, answered: Object.keys(r).join(',') }
  } catch (e) { rec = { step, refused: e.code, message: String(e.message).slice(0, 260) } }
  console.log(JSON.stringify(rec)); appendFileSync(`${EVID}/local-direct.jsonl`, JSON.stringify({ at: new Date().toISOString(), ...rec }) + '\n'); return rec
}
const before = await homeLen()
const recs = [
  await attempt('attach', requests.attachToSessionRequest(sid, 'amh-local-direct')),
  await attempt('prompt', requests.submitPromptRequest(sid, 'attachment-forged', agent, 'Reply with exactly the single word local-direct-ok and nothing else.', [])),
  await attempt('state', requests.getSessionStateRequest(sid)),
]
await sleep(15000)
const after = await homeLen()
const out = { all_refused_with_home_session: recs.every(r => r.refused === 'leased_backing_session_refused' && r.message.includes(homeSid)), home_history_len_before: before, home_history_len_after: after }
console.log(JSON.stringify(out)); appendFileSync(`${EVID}/local-direct.jsonl`, JSON.stringify(out) + '\n')
process.exit(0)
