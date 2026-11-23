// F8: revoke the kernel's Cloud authority while a burst of mutations is queued on an admitted direct carrier.
import { mkdirSync, readFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { pathToFileURL } from 'node:url'
import { repo, here } from './forged-client.mjs'

export function defineF8(h) {
  const { statePath, note, mint, admitRtc, sess, K, target, cell, newIdentity, wsCarrier, prove, connectedOk, goodHeaders, sleep, directRequest } = h
  const ipc = async (dir, port) => {
    process.env.CHARIOX_HOME = `${dir}/chariox`
    const { LocalIpcClient } = await import(pathToFileURL(`${repo}/packages/kernel-client/dist/ipc.js`).href)
    return new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`)
  }
  return {
    async F8() {
      const dir = '/var/tmp/forgedclient/k3'; const work = `${dir}/work/room`; mkdirSync(work, { recursive: true }); execFileSync('git', ['init', '-q', work])
      const local = await ipc(dir, K.k3.port); const out = {}; note('F8 ipc client up')
      let lastRaw = ''; const created = async () => { const r = JSON.stringify(await local.send({ ListSessions: null })); lastRaw = r.slice(0, 160); return new Set(r.match(/fc8-[\w-]+/g) ?? []).size }
      const defaults = { provider: 'opencode', account_profile: 'default', model: 'opencode/big-pickle', effort: 'low', permission_level: 'yolo', execution_mode: 'build' }
      const run = Date.now().toString(36); const mutation = i => ({ CreateSession: { workspace_id: work, worktree_id: work, alias: `fc8-${run}-${i}`, agent_defaults: defaults } })
      for (const carrierKind of ['ws', 'rtc']) {
        const id = newIdentity(); let c, m
        if (carrierKind === 'ws') { for (let i = 0; i < 40; i++) { m = await mint('k3', id); if (m.issued) break; await sleep(3000) } note('F8 mint', !!m.issued); c = await wsCarrier(m.issued.endpoint, { headers: goodHeaders() }); const v = await prove(c, { grant: m.issued.grant, kernelKey: m.kernelKey, identity: id, origin: sess.origin }); if (!connectedOk(v)) { out[carrierKind] = 'not admitted'; continue } }
        else { const r = await admitRtc('k3', id); if (!r.c || !connectedOk(r.v)) { out[carrierKind] = 'not admitted'; continue } c = r.c; m = r.m }
        const kernelKey = m.kernelKey; note('F8 admitted', carrierKind, 'sessions now', await created())
        // control: the same burst without a revoke must create every session (so a stopped burst is the revoke's doing)
        { const b0 = await created(); const ids = []; for (let i = 0; i < 10; i++) { const q = `ctl${i}`; ids.push(q); c.send({ kind: 'client_request', request_id: q, target: target('k3'), encrypted_request: id.encrypt(kernelKey, JSON.stringify(mutation(`ctl-${carrierKind}${i}`))) }) }
          let got = 0, ok = 0; while (got < 10) { const f = await c.next(15000); if (f.kind === 'client_response') { got++; if (!f.error) ok++ } else if (f.timeout || f.closed !== undefined) break }
          out["control_" + carrierKind] = { sent: 10, responses: got, ok, createdDelta: (await created()) - b0 } }
        const base = await created(); const N = 40
        const sent = Date.now()
        for (let i = 0; i < N; i++) c.send({ kind: 'client_request', request_id: `m${i}`, target: target('k3'), encrypted_request: id.encrypt(kernelKey, JSON.stringify(mutation(`${carrierKind}${i}`))) })
        // revoke while the burst is still queued: after the first response arrives
        const frames = []; let first = null; let tRevoke = null, tRevoked = null
        const reader = (async () => { for (;;) { const f = await c.next(15000); frames.push({ t: Date.now(), f: f.kind ? { kind: f.kind, id: f.request_id, reason: f.reason, err: f.error ? JSON.stringify(f.error).slice(0, 120) : 0 } : f }); if (f.kind === 'close' || f.closed !== undefined || f.timeout) return } })()
        const watch = (async () => { while (!frames.some(x => x.f.kind === 'client_response' && !x.f.err)) await sleep(2); first = Date.now(); tRevoke = Date.now(); await local.send({ LogoutCloudRelay: { revoke_client: false, revoke_machine: false } }); tRevoked = Date.now() })()
        await Promise.all([reader, watch])
        const close = frames.find(x => x.f.kind === 'close' || x.f.closed !== undefined)
        const respOk = frames.filter(x => x.f.kind === 'client_response' && !x.f.err).length
        const respAfterRevoke = frames.filter(x => x.f.kind === 'client_response' && x.t > tRevoked + 250).length
        const atClose = await created(); await sleep(6000); const later = await created()
        out[carrierKind] = { sent: N, responsesOk: respOk, closeFrame: close?.f, closeAfterRevokeCallMs: close ? close.t - tRevoke : null, createdBeforeBurst: base, createdAtCloseSettle: atClose, createdSixSecondsLater: later, lastListHead: lastRaw, responsesMoreThan250msAfterRevoke: respAfterRevoke, framesTail: frames.slice(-3).map(x => x.f) }
        c.close()
        // the logout hides the kernel's sessions, so count after re-pairing: nothing may have run after the close
        const stack = K.k3.stack; execFileSync(process.execPath, [`${here}/setup.mjs`, stack, String(K.k3.port), dir, 'fc-k3', statePath], { stdio: 'ignore' })
        Object.assign(K.k3, JSON.parse(readFileSync(statePath, 'utf8')))
        const final = await created(); out[carrierKind].createdAfterRepair = final - base; await sleep(3000); out[carrierKind].createdAfterRepairLater = (await created()) - base
        { const list = (await local.send({ ListSessions: null })).SessionsListed.sessions.filter(x => String(x.alias ?? '').startsWith(`fc8-${run}-${carrierKind}`))
          const ts = list.map(x => x.created_at_ms ?? x.agents?.[0]?.created_at_ms).filter(Number.isFinite).sort((a, b) => a - b)
          out[carrierKind].executedTimes = { count: ts.length, firstMsAfterRevokeCallStarted: ts[0] - tRevoke, lastMsAfterRevokeCallStarted: ts[ts.length - 1] - tRevoke, afterRevokeCallReturned: ts.filter(t => t > tRevoked).length, afterReturnedPlus250: ts.filter(t => t > tRevoked + 250).length, revokeCallMs: tRevoked - tRevoke } }
        out[carrierKind].mutationsExecutedMinusAnswered = out[carrierKind].createdAfterRepair - respOk
      }
      const good = w => w && typeof w === 'object' && w.closeFrame?.reason === 'local browser authority revoked' && w.responsesMoreThan250msAfterRevoke === 0 && w.closeAfterRevokeCallMs <= 1500
        && w.createdAfterRepairLater === w.createdAfterRepair && w.executedTimes?.afterRevokeCallReturned === 0
      const ok = out.control_ws?.createdDelta === 10 && out.control_rtc?.createdDelta === 10 && good(out.ws) && good(out.rtc)
      await local.close?.().catch(() => {})
      cell('F8 revoke during a queued mutation burst stops it (WebSocket and WebRTC)', ok ? 'PASS' : 'FAIL', out)
    },
  }
}
