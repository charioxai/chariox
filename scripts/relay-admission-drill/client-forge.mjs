// G09 client -> kernel boundary: a real second client (own CLI cloud login/identity) sends mismatched requests through the real relay.
import './client-env.mjs'
import { readFileSync, appendFileSync } from 'node:fs'
const repo = new URL('../..', import.meta.url).pathname.replace(/\/$/, '')
const { CloudClient } = await import(`${repo}/apps/cli/dist/cloud-client.js`)
const { LocalIpcClient } = await import(`${repo}/packages/kernel-client/dist/ipc.js`)
const { RelayClientIdentity, createRelayKeypair } = await import(`${repo}/packages/kernel-client/dist/relay-crypto.js`)
const requests = await import(`${repo}/packages/kernel-client/dist/ipc-requests.js`)
const { client: localClient, unwrap, EVID, STATE } = await import('./lib.mjs')
const ids = JSON.parse(readFileSync(`${STATE}/ids.json`, 'utf8'))
const [S1, S2] = Object.keys(ids.sessions); const A = ids.sessions[S1], B = ids.sessions[S2]
const tag = process.argv[2] ?? 'cli'
const cloud = new CloudClient()
const H = localClient(Number(process.env.H_PID)), W = localClient(Number(process.env.W_PID))
const out = []
const call = async (c, req, ms = 40000) => {
  try { const r = await Promise.race([c.send(req), new Promise((_, rej) => setTimeout(() => rej(Object.assign(new Error('driver timeout'), { code: 'driver_timeout' })), ms))]); return r.Error ? { kind: 'refused', code: r.Error.code, message: String(r.Error.message).slice(0, 200) } : { kind: 'accepted', variant: Object.keys(r)[0], body: JSON.stringify(r).slice(0, 200) } }
  catch (e) { return { kind: 'refused', code: e.code, message: String(e.message).slice(0, 200) } }
}
async function record(row, key, expect, c, req, note = '') {
  const o = await call(c, req); const verdict = expect === 'refused' ? (o.kind === 'refused' ? 'ok-refused' : 'ACCEPTED-P1?') : (o.kind === 'accepted' ? 'ok-accepted' : 'unexpected-refusal')
  const rec = { tag, row, key, expect, request: Object.keys(req).join(',').slice(0, 60), ...o, verdict, note }; out.push(rec); appendFileSync(`${EVID}/client-cases.jsonl`, JSON.stringify(rec) + '\n')
  console.log(verdict.padEnd(18), `${row} ${key}`.padEnd(40), JSON.stringify(req).slice(0, 70).padEnd(72), o.kind, o.code ?? o.variant ?? '', (o.message ?? '').slice(0, 90))
  return o
}
const snap = async () => ({ s1: unwrap(await H.send(requests.listAgentsRequest(S1)), 'AgentsListed').agents.map(a => [a.id, a.is_processing, a.remote_execution?.execution_lease_id]), s2: unwrap(await H.send(requests.listAgentsRequest(S2)), 'AgentsListed').agents.map(a => [a.id, a.is_processing, a.remote_execution?.execution_lease_id]), hist1: JSON.stringify(unwrap(await H.send(requests.getSessionHistoryOutlineRequest(S1, null, 20)), 'SessionHistoryOutline')).length, hist2: JSON.stringify(unwrap(await H.send(requests.getSessionHistoryOutlineRequest(S2, null, 20)), 'SessionHistoryOutline')).length })
const before = await snap()
const home = await cloud.connect(ids.home.kernel)
await record('G09', 'valid client->home', 'accepted', home, requests.listSessionsRequest(), 'second client, own identity, through the relay')
await record('G09', 'valid client->home', 'accepted', home, requests.listAgentsRequest(S1))
// sender: token issued for the real key, requests encrypted with another key
const grant = await cloud.issue(ids.home.kernel)
const wrong = new LocalIpcClient(grant.relayUrl, { relayAuthToken: grant.token, relayIdentity: new RelayClientIdentity(createRelayKeypair().privateKey), targetDaemonId: ids.home.kernel })
await record('G09', 'sender key != token key', 'refused', wrong, requests.listSessionsRequest())
await record('G09', 'sender key != token key', 'refused', wrong, requests.listAgentsRequest(S1))
wrong.destroy?.()
// kernel/machine: valid client of the same account, requests meant for the home sent to other kernels
const worker = await cloud.connect(ids.worker.kernel)
const wsess = await call(worker, requests.listSessionsRequest()); console.log('worker sessions visible to a client:', wsess.kind, wsess.body?.slice(0, 160))
await record('G09', 'kernel=worker (home session ids)', 'refused', worker, requests.listAgentsRequest(S1))
await record('G09', 'kernel=worker (home session ids)', 'refused', worker, requests.getSessionStateRequest(S1))
const xk = await cloud.connect(ids.x.kernel)
await record('G09', 'kernel=x (home session ids)', 'refused', xk, requests.listAgentsRequest(S1))
await record('G09', 'kernel=x (home session ids)', 'refused', xk, requests.getSessionStateRequest(S2))
// Room: an attachment of room S2 used against room S1, and an unattached session
const att2 = unwrap(await home.send(requests.attachToSessionRequest(S2, `amh-forge-${tag}`)), 'SessionAttached').attachment.id
await record('G09', 'room: S2 attachment on S1', 'refused', home, requests.submitPromptRequest(S1, att2, A.agent, 'amh-forged-room-prompt: do nothing', []))
await record('G09', 'room: S2 attachment on S1', 'refused', home, requests.cancelActivePromptRequest(S1, att2))
await record('G09', 'room: unknown attachment', 'refused', home, requests.submitPromptRequest(S1, 'attachment-0000deadbeef', A.agent, 'amh-forged-room-prompt: do nothing', []))
await record('G09', 'room: agent of S2 via S2 attachment but S1 id', 'refused', home, requests.submitPromptRequest(S2, att2, A.agent, 'amh-forged-agent-prompt: do nothing', []))
// lease: drive the worker's hidden backing agent of A directly, bypassing the home
const wh = unwrap(await W.send(requests.getDaemonHealthRequest()), 'DaemonHealth').projection
const backing = (wh.projection_invariants?.mismatches ?? []).map(m => m.session_id).filter(Boolean)
const direct = [...new Set(backing)].slice(0, 3)
for (const sid of direct) await record('G09', `lease: worker backing session ${sid.slice(-6)}`, 'refused', worker, requests.attachToSessionRequest(sid, `amh-forge-${tag}`))
await record('G09', 'lease: worker backing session unknown', 'refused', worker, requests.submitPromptRequest('18e1e84aeb113b2e', 'attachment-0000deadbeef', null, 'amh-forged-direct-prompt: do nothing', []))
const after = await snap()
const same = JSON.stringify(before) === JSON.stringify(after)
const summary = { tag, unchanged: same, counts: out.reduce((m, r) => (m[r.verdict] = (m[r.verdict] ?? 0) + 1, m), {}) }
appendFileSync(`${EVID}/client-cases.jsonl`, JSON.stringify({ summary }) + '\n'); console.log(JSON.stringify(summary))
if (!same) console.log('BEFORE', JSON.stringify(before), '\nAFTER', JSON.stringify(after))
cloud.stop(); process.exit(0)
