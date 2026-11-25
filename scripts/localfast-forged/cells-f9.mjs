// F9: Vault ownership. Two owners on two Cloud accounts (stacks A and B), each with a drill kernel and one Vault entry holding a test marker.
// A forged client admitted on A's kernel must never read or fill B's entry, and B's marker must never appear in anything A's side receives (and back).
import { pathToFileURL } from 'node:url'
import { randomBytes } from 'node:crypto'
import { repo, cloudSession } from './forged-client.mjs'

export function defineF9(h) {
  const { note, mint, sess, K, target, cell, newIdentity, wsCarrier, prove, connectedOk, goodHeaders, sleep, refusedReason, relayCall, relayUrlFor, directRequest, admitRtc, WebSocket } = h
  const ipc = async (dir, port) => { process.env.CHARIOX_HOME = `${dir}/chariox`; const { LocalIpcClient } = await import(pathToFileURL(`${repo}/packages/kernel-client/dist/ipc.js`).href); return new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`) }
  const mkEntry = async (dir, port, name, marker) => {
    const c = await ipc(dir, port)
    // a throwaway drill Vault: random passphrase kept in this process only, never printed or stored
    const pass = randomBytes(18).toString('hex')
    await c.send({ CreateCredentialVault: { origin: { kind: 'passphrase', passphrase: pass } } }).catch(() => {})
    await c.send({ UnlockCredentialVault: { passphrase: pass, duration: { kind: 'until_kernel_stops' }, surface: 'terminal' } }).catch(e => { throw new Error('vault unlock: ' + String(e.message).slice(0, 160)) })
    const r = await c.send({ OwnerVault: { action: 'create', name, kind: 'Password', use_binding: { target: { kind: 'site', origin: 'https://example.com' }, value_kind: 'secret' }, value: marker } })
    await c.close?.().catch(() => {}); return r
  }
  return {
    async F9() {
      const out = {}; const transcript = []
      const markerA = 'FC9-MARKER-A-' + randomBytes(6).toString('hex'), markerB = 'FC9-MARKER-B-' + randomBytes(6).toString('hex')
      const ra = await mkEntry('/var/tmp/forgedclient/k1', K.k1.port, 'fc9-entry-a-' + Date.now().toString(36), markerA)
      const rb = await mkEntry('/var/tmp/forgedclient/k4', K.k4.port, 'fc9-entry-b-' + Date.now().toString(36), markerB)
      const idOf = r => JSON.stringify(r).match(/"id":"([^"]+)"/)?.[1]
      const idA = idOf(ra), idB = idOf(rb); out.setup = { createdA: !!idA, createdB: !!idB, aReply: idA ? 'ok' : JSON.stringify(ra).slice(0, 200), bReply: idB ? 'ok' : JSON.stringify(rb).slice(0, 200) }
      if (!idA || !idB) return cell('F9 Vault ownership (forged client never reads or fills another owner\'s entry)', 'FAIL', { ...out, note: 'harness could not create the test entries' })
      const record = (label, v) => { transcript.push({ label, v: JSON.stringify(v) }); return v }
      const A = newIdentity()
      // A's own direct session: owner control
      const m = await mint('k1', A); const c = await wsCarrier(m.issued.endpoint, { headers: goodHeaders() })
      const v = await prove(c, { grant: m.issued.grant, kernelKey: m.kernelKey, identity: A, origin: sess.origin })
      if (!connectedOk(v)) return cell('F9 Vault ownership', 'FAIL', { ...out, note: 'control not admitted' })
      const own = record('A reveals A (direct)', await directRequest(c, { identity: A, kernelKey: m.kernelKey, target: target('k1'), body: { OwnerVault: { action: 'reveal', id: idA } } }))
      out.control_ownerRevealsOwnEntryDirect = JSON.stringify(own).includes(markerA) ? 'owner reads own entry (expected)' : 'NOT readable: ' + JSON.stringify(own).slice(0, 160)
      // cross-owner over A's admitted direct session
      out.a_revealBOnAKernel = (JSON.stringify(record('A reveals B-id on k1', await directRequest(c, { identity: A, kernelKey: m.kernelKey, target: target('k1'), body: { OwnerVault: { action: 'reveal', id: idB } } }))).slice(0, 200))
      out.a_replaceBOnAKernel = (JSON.stringify(record('A replaces B-id on k1', await directRequest(c, { identity: A, kernelKey: m.kernelKey, target: target('k1'), body: { OwnerVault: { action: 'replace', id: idB, value: 'FC9-OVERWRITE' } } }))).slice(0, 200))
      const toB = record('A addresses B kernel over A direct', await directRequest(c, { identity: A, kernelKey: m.kernelKey, target: { daemon_id: K.k4.daemon }, body: { OwnerVault: { action: 'reveal', id: idB } } }))
      out.b_requestAddressedToBKernelOverADirect = JSON.stringify(toB).slice(0, 200)
      c.close()
      // B's side
      const sessB = await cloudSession({ stack: K.k4.stack })
      try {
        const B = newIdentity(); const bootB = await sessB.bootstrap(K.k4.daemon, B.publicKeyThumbprint); const relayB = bootB.json?.relayUrl
        // B's own session works (control) and cannot read A's entry
        const leaseB = await sessB.lease(K.k4.daemon, B.publicKeyThumbprint)
        const own2 = record('B reveals B (relay)', await relayCall({ relayUrl: relayB, token: leaseB.json?.relayToken, target: { daemon_id: K.k4.daemon }, identity: B, bodies: [{ OwnerVault: { action: 'reveal', id: idB } }] }))
        out.control_ownerRevealsOwnEntryRelayB = JSON.stringify(own2).includes(markerB) ? 'owner reads own entry (expected)' : 'NOT readable: ' + JSON.stringify(own2).slice(0, 160)
        const bOnA = record('B reveals A-id on kB', await relayCall({ relayUrl: relayB, token: leaseB.json?.relayToken, target: { daemon_id: K.k4.daemon }, identity: B, bodies: [{ OwnerVault: { action: 'reveal', id: idA } }] }))
        out.d_bRevealsAIdOnBKernel = JSON.stringify(bOnA.responses[0]?.error ?? bOnA.responses[0]?.body ?? bOnA.error).slice(0, 200)
        // B asks Cloud A's kernel (lease from B's account), B's token at A's relay, A's token at B's relay
        const leaseBforA = await sessB.lease(K.k1.daemon, B.publicKeyThumbprint); out.e_cloudBLeaseForAKernel = leaseBforA.json?.relayToken ? 'TOKEN ISSUED' : `refused HTTP ${leaseBforA.status}`
        const bTokenAtRelayA = await relayCall({ relayUrl: relayUrlFor(), token: leaseB.json?.relayToken, target: target('k1'), identity: B, bodies: [{ OwnerVault: { action: 'reveal', id: idA } }] })
        out.f_bTokenAtRelayAForAKernel = JSON.stringify(bTokenAtRelayA.responses[0] ?? bTokenAtRelayA.error ?? bTokenAtRelayA.frames).slice(0, 160); record('B token at relay A', bTokenAtRelayA)
        const leaseA = await sess.lease(K.k1.daemon, A.publicKeyThumbprint)
        const aTokenAtRelayB = await relayCall({ relayUrl: relayB, token: leaseA.json?.relayToken, target: { daemon_id: K.k4.daemon }, identity: A, bodies: [{ OwnerVault: { action: 'reveal', id: idB } }] })
        out.g_aTokenAtRelayBForBKernel = JSON.stringify(aTokenAtRelayB.responses[0] ?? aTokenAtRelayB.error ?? aTokenAtRelayB.frames).slice(0, 160); record('A token at relay B', aTokenAtRelayB)
        // grants cross-presented on the loopback endpoints of the other owner's kernel
        const mB = await mintOn(sessB, K.k4, B, relayB); out.h_bGrantMinted = !!mB.issued
        const cB = await wsCarrier(mB.issued.endpoint, { headers: { Origin: sessB.origin } })
        const vA = await prove(cB, { grant: m.issued.grant, kernelKey: mB.kernelKey, identity: A, origin: sessB.origin })   // A's (spent) grant, A's key, B's endpoint
        out.i_aGrantAtBEndpoint = connectedOk(vA) ? 'ADMITTED' : 'denied: ' + refusedReason(vA); cB.close()
        const m2 = await mint('k1', A); const cA2 = await wsCarrier(m2.issued.endpoint, { headers: goodHeaders() })
        const vB = await prove(cA2, { grant: mB.issued.grant, kernelKey: m2.kernelKey, identity: B, origin: sess.origin })     // B's grant, B's key, A's endpoint
        out.j_bGrantAtAEndpoint = connectedOk(vB) ? 'ADMITTED' : 'denied: ' + refusedReason(vB); cA2.close()
        // the local CLI listener of B's kernel: no token, A's relay token as bearer, hostile Origin
        const probe = (port, headers) => new Promise(res => { const w = new WebSocket(`ws://127.0.0.1:${port}/kernel`, { headers, handshakeTimeout: 4000 }); const t = setTimeout(() => { w.terminate(); res('timeout') }, 5000); w.on('unexpected-response', (_q, r) => { clearTimeout(t); res('HTTP ' + r.statusCode) }); w.on('open', () => { clearTimeout(t); w.terminate(); res('OPEN') }); w.on('error', e => { clearTimeout(t); res('error ' + e.message.slice(0, 60)) }) })
        out.k_localListenerNoToken = { b: await probe(K.k4.port, {}), a: await probe(K.k1.port, {}) }
        out.l_localListenerRelayTokenAsBearer = await probe(K.k4.port, { Authorization: 'Bearer ' + (leaseA.json?.relayToken ?? 'x') })
        out.m_localListenerHostileOrigin = await probe(K.k4.port, { Origin: 'https://evil.example' })
      } finally { await sessB.close() }
      out.leak_markerBInAnythingASideReceived = transcript.filter(t => /^A /.test(t.label) && t.v.includes(markerB)).map(t => t.label)
      out.leak_markerAInAnythingBSideReceived = transcript.filter(t => /^B /.test(t.label) && t.v.includes(markerA)).map(t => t.label)
      out.transcriptLabels = transcript.map(t => t.label)
      const notOk = []
      for (const k of ['a_revealBOnAKernel', 'a_replaceBOnAKernel', 'b_requestAddressedToBKernelOverADirect', 'd_bRevealsAIdOnBKernel']) if (!/error|not found|refused|unknown|invalid|no such|not_found|mismatch|\"code\"/i.test(out[k]) || out[k].includes('Revealed') || out[k].includes('Saved')) notOk.push(k)
      if (!/^refused/.test(out.e_cloudBLeaseForAKernel)) notOk.push('e')
      for (const k of ['i_aGrantAtBEndpoint', 'j_bGrantAtAEndpoint']) if (!/^denied/.test(out[k])) notOk.push(k)
      for (const k of ['f_bTokenAtRelayAForAKernel', 'g_aTokenAtRelayBForBKernel']) if (/Revealed|marker/i.test(out[k]) || /"body":\{"Revealed/.test(out[k])) notOk.push(k)
      if (out.k_localListenerNoToken.b !== 'HTTP 401' || out.k_localListenerNoToken.a !== 'HTTP 401') notOk.push('k')
      if (out.l_localListenerRelayTokenAsBearer !== 'HTTP 401') notOk.push('l')
      if (!/^HTTP 4/.test(out.m_localListenerHostileOrigin)) notOk.push('m')
      if (out.leak_markerBInAnythingASideReceived.length || out.leak_markerAInAnythingBSideReceived.length) notOk.push('leak')
      if (!/expected/.test(out.control_ownerRevealsOwnEntryDirect) || !/expected/.test(out.control_ownerRevealsOwnEntryRelayB)) notOk.push('controls')
      cell('F9 Vault ownership: a forged client never reads or fills another owner\'s Vault entry (two owners, two stacks)', notOk.length ? 'FAIL' : 'PASS', { ...out, notOk, markers: 'test values, not printed' })
    },
  }
}

async function mintOn(session, k, identity, relayUrl) {
  const { relayCall } = await import('./forged-client.mjs')
  const lease = await session.lease(k.daemon, identity.publicKeyThumbprint)
  const r = await relayCall({ relayUrl, token: lease.json?.relayToken, target: { daemon_id: k.daemon }, identity, bodies: [{ local_browser_connect: {} }] })
  return { issued: r.responses[0]?.body?.LocalBrowserConnectIssued, kernelKey: r.kernelKeyFromRelay }
}
