// F2..F6 of the LOCALFAST-A1 denial matrix. Imported by run-cells.mjs.
export function defineF2toF6(h) {
  const { admitWs, admitRtc, mint, sess, rtc, K, target, denied, verdictOf, sleep, LIST, goodHeaders, relayUrlFor, cell, newIdentity, wsCarrier, prove, connectedOk, refusedReason, relayCall, createRelayKeypair, RelayClientIdentity, directRequest } = h
  const isDenied = v => /^(denied|mint refused|upgrade HTTP|channel did not open)/.test(v)
  const rawProve = async (c, build, wait = 6000) => { const ch = await c.next(wait); if (ch.kind !== 'local_challenge') return { frame: ch }; await c.sendRaw(build(ch)); return { challenge: ch, frame: await c.next(wait) } }
  const attacker = () => { const kp = createRelayKeypair(); return { id: new RelayClientIdentity(kp.privateKey), pub: kp.publicKeyBase64 } }
  const ws = async (k, id, m) => wsCarrier(m.issued.endpoint, { headers: goodHeaders() })

  return {
    async F2() {
      const A = newIdentity(), B = newIdentity(), out = {}
      // a: grant minted for A's key, proof sent by B's key (a stolen grant string on another key)
      { const m = await mint('k1', A); const c = await ws('k1', A, m); const v = await prove(c, { grant: m.issued.grant, kernelKey: m.kernelKey, identity: B, origin: sess.origin })
        out.a_grantOfAProofByB = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); c.close()
        const c2 = await ws('k1', A, m); const v2 = await prove(c2, { grant: m.issued.grant, kernelKey: m.kernelKey, identity: A, origin: sess.origin })
        out.a2_grantAfterWrongKeyAttempt = connectedOk(v2) ? 'still redeemable by A' : 'burnt: ' + refusedReason(v2); c2.close() }
      // b: proof encrypted to a key that is not the kernel's
      { const at = attacker(); const m = await mint('k1', A); const c = await ws('k1', A, m); const v = await prove(c, { grant: m.issued.grant, kernelKey: at.pub, identity: A, origin: sess.origin })
        out.b_proofToWrongKernelKey = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); c.close() }
      // c: relay lease token bound to B's key thumbprint, local_browser_connect encrypted with A's key
      { const lease = await sess.lease(K.k1.daemon, B.publicKeyThumbprint); const m = await mint('k1', A, { token: lease.json?.relayToken })
        out.c_connectEncryptedByOtherKeyThanIdentity = m.issued ? 'GRANT ISSUED' : 'refused: ' + JSON.stringify(m.refusal ?? m.body).slice(0, 140) }
      // d: ciphertext tampered
      { const m = await mint('k1', A); const c = await ws('k1', A, m)
        const v = await rawProve(c, ch => { const p = A.encrypt(m.kernelKey, JSON.stringify({ grant: m.issued.grant, challenge: ch.challenge, origin: sess.origin, kernel_id: ch.kernel_id, endpoint_epoch: ch.endpoint_epoch })); const ct = Buffer.from(p.ciphertext, 'base64'); ct[0] ^= 1; return JSON.stringify({ kind: 'local_connect', proof: { ...p, ciphertext: ct.toString('base64') } }) })
        out.d_tamperedCiphertext = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); c.close() }
      // e: malformed frames
      for (const [name, build] of Object.entries({ notJson: () => 'hello', emptyProof: () => JSON.stringify({ kind: 'local_connect', proof: {} }), wrongKind: () => JSON.stringify({ kind: 'client_request', request_id: '1' }), oversized: () => JSON.stringify({ kind: 'local_connect', proof: { x: 'a'.repeat(20000) } }) })) {
        const m = await mint('k1', A); const c = await ws('k1', A, m); const v = await rawProve(c, build); out['e_' + name] = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); c.close() }
      // f: silent client
      { const m = await mint('k1', A); const c = await ws('k1', A, m); const t0 = Date.now(); const ch = await c.next(2000); const end = await c.next(9000)
        out.f_silentClient = { challengeSeen: ch.kind, afterMs: Date.now() - t0, endFrame: JSON.stringify(end).slice(0, 100) }; c.close() }
      // WebRTC: wrong sender key, wrong kernel key
      { const r = await admitRtc('k1', A, p => p); r.c?.close() }
      { const { sdp, carrier } = await rtc.offer(); const m = await mint('k1', A, { offer: sdp }); await carrier.accept(m.issued.answer)
        const v = await prove(carrier, { grant: m.issued.grant, kernelKey: m.kernelKey, identity: B, origin: sess.origin }); out.rtc_proofByOtherKey = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); carrier.close() }
      { const at = attacker(); const { sdp, carrier } = await rtc.offer(); const m = await mint('k1', A, { offer: sdp }); await carrier.accept(m.issued.answer)
        const v = await prove(carrier, { grant: m.issued.grant, kernelKey: at.pub, identity: A, origin: sess.origin }); out.rtc_proofToWrongKernelKey = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); carrier.close() }
      const must = ['a_grantOfAProofByB', 'b_proofToWrongKernelKey', 'd_tamperedCiphertext', 'e_notJson', 'e_emptyProof', 'e_wrongKind', 'e_oversized', 'rtc_proofByOtherKey', 'rtc_proofToWrongKernelKey']
      const bad = must.filter(k => !/^denied/.test(out[k])); if (!/^refused/.test(out.c_connectEncryptedByOtherKeyThanIdentity)) bad.push('c')
      cell('F2 invalid key denied (stolen grant, wrong kernel key, wrong sender key, tampered/garbled proof)', bad.length ? 'FAIL' : 'PASS', { ...out, notDenied: bad })
    },

    async F3() {
      const A = newIdentity(), out = {}
      // a: proof names another kernel
      { const m = await mint('k1', A); const c = await ws('k1', A, m); const v = await prove(c, { grant: m.issued.grant, kernelKey: m.kernelKey, identity: A, origin: sess.origin, mutate: p => ({ ...p, kernel_id: K.k2.daemon }) }); out.a_proofNamesOtherKernel = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); c.close() }
      // b: k1's grant presented at k2's endpoint (a second kernel of the same owner on the same machine)
      { const m1 = await mint('k1', A); const m2 = await mint('k2', A); const c = await wsCarrier(m2.issued.endpoint, { headers: goodHeaders() })
        const v = await prove(c, { grant: m1.issued.grant, kernelKey: m2.kernelKey, identity: A, origin: sess.origin }); out.b_grantOfK1AtK2Endpoint = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); c.close()
        const c1 = await wsCarrier(m1.issued.endpoint, { headers: goodHeaders() }); const v1 = await prove(c1, { grant: m2.issued.grant, kernelKey: m1.kernelKey, identity: A, origin: sess.origin }); out.b2_grantOfK2AtK1Endpoint = connectedOk(v1) ? 'ADMITTED' : 'denied: ' + refusedReason(v1); c1.close() }
      // c: a relay lease token for k1 used to reach k2
      { const l1 = await sess.lease(K.k1.daemon, A.publicKeyThumbprint); const r = await relayCall({ relayUrl: relayUrlFor(), token: l1.json.relayToken, target: target('k2'), identity: A, bodies: [{ local_browser_connect: {} }] })
        out.c_k1TokenAtK2 = r.responses[0]?.body?.LocalBrowserConnectIssued ? 'GRANT ISSUED' : 'refused: ' + JSON.stringify(r.responses[0]?.error ?? r.error ?? r.frames).slice(0, 140) }
      // d: lease for an unknown kernel id
      { const l = await sess.lease('kernel-0000000000000000', A.publicKeyThumbprint); out.d_leaseUnknownKernel = l.json?.relayToken ? 'TOKEN ISSUED' : `refused HTTP ${l.status}` }
      // e: request meant for k1 encrypted to k2's key
      { const l1 = await sess.lease(K.k1.daemon, A.publicKeyThumbprint); const m2 = await mint('k2', A)
        const r = await relayCall({ relayUrl: relayUrlFor(), token: l1.json.relayToken, target: target('k1'), identity: A, kernelKey: m2.kernelKey, bodies: [{ local_browser_connect: {} }] })
        out.e_connectEncryptedToOtherKernelKey = r.responses[0]?.body?.LocalBrowserConnectIssued ? 'GRANT ISSUED' : 'refused: ' + JSON.stringify(r.responses[0]?.error ?? r.error ?? 'no response').slice(0, 140) }
      // f: WebRTC grant of k1 on k2's channel
      { const { sdp, carrier } = await rtc.offer(); const m2 = await mint('k2', A, { offer: sdp }); const m1 = await mint('k1', A); await carrier.accept(m2.issued.answer)
        const v = await prove(carrier, { grant: m1.issued.grant, kernelKey: m2.kernelKey, identity: A, origin: sess.origin }); out.f_rtcGrantOfK1OnK2Channel = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); carrier.close() }
      // g: WebRTC proof naming another kernel
      { const r = await admitRtc('k1', A, p => ({ ...p, kernel_id: K.k2.daemon })); out.g_rtcProofNamesOtherKernel = verdictOf(r); r.c?.close() }
      const bad = Object.entries(out).filter(([k, v]) => !/^(denied|refused)/.test(String(v))).map(([k]) => k)
      cell('F3 wrong target denied (other kernel id, other kernel endpoint/channel, token for another target, unknown kernel)', bad.length ? 'FAIL' : 'PASS', { ...out, notDenied: bad })
    },

    async F4() {
      const A = newIdentity(), out = {}
      for (const [name, epoch] of Object.entries({ stale: 'stale-epoch', empty: '', k2sEpoch: null })) {
        const m = await mint('k1', A); const c = await ws('k1', A, m)
        let ep = epoch; if (name === 'k2sEpoch') { const m2 = await mint('k2', A); ep = m2.issued.endpoint_epoch }
        const v = await prove(c, { grant: m.issued.grant, kernelKey: m.kernelKey, identity: A, origin: sess.origin, mutate: p => ({ ...p, endpoint_epoch: ep }) }); out['ws_' + name] = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); c.close() }
      { const r = await admitRtc('k1', A, p => ({ ...p, endpoint_epoch: 'stale-epoch' })); out.rtc_stale = verdictOf(r); r.c?.close() }
      out.note = 'real restart epoch (F4r) is in cell F4r'
      const bad = Object.entries(out).filter(([k, v]) => k !== 'note' && !/^denied/.test(String(v))).map(([k]) => k)
      cell('F4 stale/foreign endpoint epoch denied', bad.length ? 'FAIL' : 'PASS', { ...out, notDenied: bad })
    },

    async F5() {
      const A = newIdentity(), out = {}
      // a: grant reused after a good admission, fresh connection and fresh proof
      { const m = await mint('k1', A); const c = await ws('k1', A, m); const v = await prove(c, { grant: m.issued.grant, kernelKey: m.kernelKey, identity: A, origin: sess.origin }); out.a_first = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v)
        const c2 = await ws('k1', A, m); const v2 = await prove(c2, { grant: m.issued.grant, kernelKey: m.kernelKey, identity: A, origin: sess.origin }); out.a_grantReused = connectedOk(v2) ? 'ADMITTED (REPLAY)' : 'denied: ' + refusedReason(v2); c2.close()
        // c: the captured local_connect frame replayed on a new connection (challenge differs)
        const m3 = await mint('k1', A); const c3 = await ws('k1', A, m3); let captured = null
        await rawProve(c3, ch => { captured = JSON.stringify({ kind: 'local_connect', proof: A.encrypt(m3.kernelKey, JSON.stringify({ grant: m3.issued.grant, challenge: ch.challenge, origin: sess.origin, kernel_id: ch.kernel_id, endpoint_epoch: ch.endpoint_epoch })) }); return 'x' })
        c3.close()
        const m4 = await mint('k1', A); const c4 = await ws('k1', A, m4); const v4 = await rawProve(c4, () => captured); out.c_capturedProofOnNewConnection = connectedOk(v4) ? 'ADMITTED (REPLAY)' : 'denied: ' + refusedReason(v4); c4.close()
        // g: a second local_connect on an already admitted socket
        c.send({ kind: 'local_connect', proof: A.encrypt(m.kernelKey, JSON.stringify({ grant: m.issued.grant, challenge: 'x', origin: sess.origin, kernel_id: m.issued.kernel_id, endpoint_epoch: m.issued.endpoint_epoch })) })
        const g = await c.next(3000); out.g_secondLocalConnectOnAdmitted = JSON.stringify(g).slice(0, 120); c.close() }
      // b: WebSocket grant presented on a WebRTC channel
      { const { sdp, carrier } = await rtc.offer(); const m = await mint('k1', A, { offer: sdp }); const mw = await mint('k1', A); await carrier.accept(m.issued.answer)
        const v = await prove(carrier, { grant: mw.issued.grant, kernelKey: m.kernelKey, identity: A, origin: sess.origin }); out.b_wsGrantOnRtcChannel = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); carrier.close() }
      // d/e: the same SDP offer sent twice; each grant is bound to its own channel, swapped grants are refused
      { const { sdp, carrier: ch1 } = await rtc.offer(); const m1 = await mint('k1', A, { offer: sdp }); const m2 = await mint('k1', A, { offer: sdp })
        out.d_sameOfferTwice = { firstAnswer: !!m1.issued?.answer, secondAnswer: !!m2.issued?.answer, secondRefusal: m2.issued ? null : JSON.stringify(m2.refusal ?? m2.body).slice(0, 120), distinctGrants: m1.issued?.grant !== m2.issued?.grant }
        await ch1.accept(m1.issued.answer)
        const v = await prove(ch1, { grant: m2.issued?.grant ?? 'none', kernelKey: m1.kernelKey, identity: A, origin: sess.origin }); out.e_grantOfReplayedOfferOnFirstChannel = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); ch1.close() }
      // f: the encrypted relay request (captured ciphertext) sent again over a new relay connection
      { const lease = await sess.lease(K.k1.daemon, A.publicKeyThumbprint); const token = lease.json.relayToken
        const enc = A.encrypt((await relayCall({ relayUrl: relayUrlFor(), token, target: target('k1'), identity: A, bodies: [] })).kernelKeyFromRelay, JSON.stringify({ local_browser_connect: {} }))
        const send = () => new Promise(res => { const w = new h.WebSocket(relayUrlFor()); const t = setTimeout(() => { w.terminate(); res('timeout') }, 8000)
          w.on('open', () => w.send(JSON.stringify({ kind: 'client_connect', auth_token: token, target: target('k1') })))
          w.on('message', d => { const f = JSON.parse(String(d)); if (f.kind === 'client_connected') w.send(JSON.stringify({ kind: 'client_request', request_id: 'replay-1', target: target('k1'), encrypted_request: enc })); else if (f.kind === 'client_response') { clearTimeout(t); w.terminate(); let g = null; try { g = JSON.parse(A.decrypt(f.encrypted_response)) } catch {} res(f.error ? 'refused ' + JSON.stringify(f.error).slice(0, 100) : (g?.LocalBrowserConnectIssued ? 'answered (grant for the same key)' : JSON.stringify(g).slice(0, 100))) } else if (f.kind === 'close' || f.kind === 'error') { clearTimeout(t); res(JSON.stringify(f).slice(0, 100)) } }) })
        out.f_relayEnvelopeReplay = [await send(), await send()] }
      const mustDeny = ['a_grantReused', 'b_wsGrantOnRtcChannel', 'c_capturedProofOnNewConnection', 'e_grantOfReplayedOfferOnFirstChannel']
      const bad = mustDeny.filter(k => !/^denied/.test(out[k])); if (out.a_first !== 'ADMITTED') bad.push('a_first')
      cell('F5 replayed grant / captured proof / replayed offer denied', bad.length ? 'FAIL' : 'PASS', { ...out, notDenied: bad })
    },

    async F6() {
      const A = newIdentity(), out = {}
      // pre-stage everything that needs to age, then wait once
      const mWs = await mint('k1', A); const cWs = await ws('k1', A, mWs)         // grant ages on a connected endpoint (challenge pending)
      const { sdp, carrier: cRtc } = await rtc.offer(); const mRtc = await mint('k1', A, { offer: sdp })
      const mWs2 = await mint('k1', A); const mAdm = await mint('k1', A)
      const leaseOld = await sess.lease(K.k1.daemon, A.publicKeyThumbprint)
      const admitted = await wsCarrier(mAdm.issued.endpoint, { headers: goodHeaders() }); const va = await prove(admitted, { grant: mAdm.issued.grant, kernelKey: mAdm.kernelKey, identity: A, origin: sess.origin }); out.adm_admitted = connectedOk(va)
      { const q = await directRequest(admitted, { identity: A, kernelKey: mAdm.kernelKey, target: target('k1'), body: LIST }); out.d0_requestBeforeDeadline = q.body ? Object.keys(q.body)[0] : 'refused: ' + JSON.stringify(q.error ?? q.frame).slice(0, 100) }
      const t0 = Date.now()
      // b: the 15-minute bootstrap token cannot mint a local grant (a long-lived identity)
      { const b = await sess.bootstrap(K.k1.daemon, A.publicKeyThumbprint); const m = await mint('k1', A, { token: b.json?.relayToken })
        out.b_longLivedBootstrapToken = m.issued ? 'GRANT ISSUED' : 'refused: ' + JSON.stringify(m.refusal ?? m.body ?? m.error).slice(0, 140) }
      // e: renewal rules while the lease is live (fresh lease token each, as the page does)
      const renew = async (grant, sequence, identity = A, token) => { const l = token ? { json: { relayToken: token } } : await sess.lease(K.k1.daemon, identity.publicKeyThumbprint)
        const r = await relayCall({ relayUrl: relayUrlFor(), token: l.json.relayToken, target: target('k1'), identity, bodies: [{ local_browser_renew: { grant, sequence } }] })
        const b = r.responses[0]?.body; return b?.LocalBrowserLeaseRenewed ? 'renewed next=' + b.LocalBrowserLeaseRenewed.next_sequence : 'refused: ' + JSON.stringify(r.responses[0]?.error ?? b ?? r.error).slice(0, 120) }
      out.e1_renewSeq1 = await renew(mAdm.issued.grant, 1)
      out.e2_renewSeq1Again = await renew(mAdm.issued.grant, 1)
      out.e3_renewByOtherKey = await renew(mAdm.issued.grant, 2, newIdentity())
      out.e4_renewUnknownGrant = await renew('nope-nope-nope', 1)
      { admitted.send({ kind: 'client_request', request_id: 'dr', target: target('k1'), encrypted_request: A.encrypt(mAdm.kernelKey, JSON.stringify({ local_browser_renew: { grant: mAdm.issued.grant, sequence: 3 } })) }); const f = await admitted.next(4000)
        out.e5_renewOverDirectCarrier = f.kind === 'client_response' ? (f.error ? 'refused ' + JSON.stringify(f.error).slice(0, 100) : 'ANSWERED') : JSON.stringify(f).slice(0, 100) }
      // wait until every staged credential is > 31 s old
      await sleep(Math.max(0, 33000 - (Date.now() - t0)))
      { cWs.close(); const cW = await ws('k1', A, mWs); const v = await prove(cW, { grant: mWs.issued.grant, kernelKey: mWs.kernelKey, identity: A, origin: sess.origin }); out.a_ws_grantOlderThan30s = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v) }
      // The kernel drops an RTC peer whose channel is not open after 15 s (rtc.rs OPEN_DEADLINE) and forgets its grant, and an opened channel must prove within 5 s (HANDSHAKE_DEADLINE), so a >30 s old grant can never reach the grant verifier over WebRTC. A channel that did not open proves nothing about expiry: report it as a limitation, never as a denial (expiry verifier: kernel test mp11_grant_is_single_use_and_bound_to_key_challenge_and_expiry; live proof of the same verifier: the WebSocket case above).
      const limitations = []
      { const opened = await cRtc.accept(mRtc.issued.answer); out.a_rtc_channelOpened = opened
        if (opened) { const v = await prove(cRtc, { grant: mRtc.issued.grant, kernelKey: mRtc.kernelKey, identity: A, origin: sess.origin }); out.a_rtc_grantOlderThan30s = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v) }
        else { out.a_rtc_grantOlderThan30s = 'inconclusive: channel did not open (transport drops an unopened peer at 15 s, before the 30 s grant can age)'; limitations.push('RTC grant expiry is not reachable through the real transport deadlines (15 s open, 5 s handshake); covered by the kernel unit test, not by this cell') } }
      { const r = await relayCall({ relayUrl: relayUrlFor(), token: leaseOld.json.relayToken, target: target('k1'), identity: A, bodies: [{ local_browser_connect: {} }] })
        out.c_expiredLeaseToken = r.responses[0]?.body?.LocalBrowserConnectIssued ? 'GRANT ISSUED' : 'refused: ' + JSON.stringify(r.responses[0]?.error ?? r.error ?? r.frames).slice(0, 140) }
      // the grant that was minted but whose socket stays idle: redeem after expiry on a fresh socket
      { const c = await ws('k1', A, mWs2); const v = await prove(c, { grant: mWs2.issued.grant, kernelKey: mWs2.kernelKey, identity: A, origin: sess.origin }); out.a2_freshSocketOldGrant = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); c.close() }
      // d: admitted session whose lease was renewed once (e1) must still end at its deadline; observe the close
      const end = await admitted.next(20000); out.d_admittedSessionEnd = { afterMs: Date.now() - t0, frame: JSON.stringify(end).slice(0, 160) }
      out.d_endIsExpiryClose = end?.kind === 'close' && /relay token expired|local browser lease expired/.test(end.reason ?? '')
      out.d_requestAfterDeadline = await (async () => { try { const q = await directRequest(admitted, { identity: A, kernelKey: mAdm.kernelKey, target: target('k1'), body: LIST, wait: 3000 }); return q.body ? 'ANSWERED ' + Object.keys(q.body)[0] : 'refused: ' + JSON.stringify(q.error ?? q.frame).slice(0, 100) } catch (e) { return 'refused: ' + e.message } })()
      out.d_renewedAfterExpiry = await renew(mAdm.issued.grant, 5)
      const mustDeny = ['a_ws_grantOlderThan30s', 'a2_freshSocketOldGrant']
      const bad = mustDeny.filter(k => !/^denied/.test(out[k]))
      for (const k of ['b_longLivedBootstrapToken', 'c_expiredLeaseToken', 'e2_renewSeq1Again', 'e3_renewByOtherKey', 'e4_renewUnknownGrant', 'd_renewedAfterExpiry']) if (!/^refused/.test(out[k])) bad.push(k)
      if (!/^renewed/.test(out.e1_renewSeq1)) bad.push('e1_renewSeq1(control)')
      if (out.a_rtc_grantOlderThan30s === 'ADMITTED') bad.push('a_rtc_grantOlderThan30s')
      if (out.d0_requestBeforeDeadline !== 'SessionsListed') bad.push('d0_requestBeforeDeadline(control)')
      if (!out.d_endIsExpiryClose) bad.push('d_endIsExpiryClose')
      if (!/^refused/.test(out.d_requestAfterDeadline)) bad.push('d_requestAfterDeadline')
      if (/ANSWERED/.test(out.e5_renewOverDirectCarrier)) bad.push('e5')
      cell('F6 expired credential denied (grant >30 s, expired lease token, long-lived identity, lease not renewed, renewal replay/other key/direct)', bad.length ? 'FAIL' : 'PASS', { ...out, notDenied: bad, limitations })
    },
  }
}
