// F4r: the endpoint epoch really changes when the kernel restarts; a grant and epoch from before the restart are refused after it.
import { execFileSync } from 'node:child_process'
export function defineF4r(h) {
  const { mint, sess, cell, newIdentity, wsCarrier, prove, connectedOk, refusedReason, goodHeaders, sleep, admitRtc, rtc, K } = h
  return {
    async F4r() {
      const out = {}; const A = newIdentity()
      const before = await mint('k2', A); out.epochBefore = before.issued.endpoint_epoch
      const { sdp, carrier } = await rtc.offer(); const rBefore = await mint('k2', A, { offer: sdp }); out.rtcEpochBefore = rBefore.issued.endpoint_epoch; carrier.close()
      execFileSync('systemctl', ['restart', 'forgedclient-k2']); const t0 = Date.now()
      let after; for (;;) { await sleep(3000); after = await mint('k2', A); if (after.issued || Date.now() - t0 > 120000) break }
      out.restartToGrantMs = Date.now() - t0
      if (!after.issued) return cell('F4r real restart epoch', 'FAIL', { ...out, note: 'kernel did not come back' })
      out.epochAfter = after.issued.endpoint_epoch; out.epochChanged = out.epochBefore !== out.epochAfter
      // old grant + old epoch at the new endpoint
      { const c = await wsCarrier(after.issued.endpoint, { headers: goodHeaders() }); const v = await prove(c, { grant: before.issued.grant, kernelKey: after.kernelKey, identity: A, origin: sess.origin, mutate: p => ({ ...p, endpoint_epoch: before.issued.endpoint_epoch }) }); out.oldGrantOldEpoch = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); c.close() }
      // old grant + current epoch (a client that learned the new epoch but holds a pre-restart grant)
      { const c = await wsCarrier(after.issued.endpoint, { headers: goodHeaders() }); const v = await prove(c, { grant: before.issued.grant, kernelKey: after.kernelKey, identity: A, origin: sess.origin }); out.oldGrantNewEpoch = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); c.close() }
      // fresh grant is admitted (control)
      { const c = await wsCarrier(after.issued.endpoint, { headers: goodHeaders() }); const v = await prove(c, { grant: after.issued.grant, kernelKey: after.kernelKey, identity: A, origin: sess.origin }); out.control_freshGrant = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); c.close() }
      // WebRTC: pre-restart grant on a post-restart channel
      { const { sdp: s2, carrier: c2 } = await rtc.offer(); const m2 = await mint('k2', A, { offer: s2 }); out.rtcEpochAfter = m2.issued.endpoint_epoch; await c2.accept(m2.issued.answer)
        const v = await prove(c2, { grant: rBefore.issued.grant, kernelKey: m2.kernelKey, identity: A, origin: sess.origin }); out.rtcOldGrantOnNewChannel = connectedOk(v) ? 'ADMITTED' : 'denied: ' + refusedReason(v); c2.close() }
      const ok = out.epochChanged && /^denied/.test(out.oldGrantOldEpoch) && /^denied/.test(out.oldGrantNewEpoch) && out.control_freshGrant === 'ADMITTED' && /^denied/.test(out.rtcOldGrantOnNewChannel)
      cell('F4r stale epoch after a real kernel restart denied', ok ? 'PASS' : 'FAIL', out)
    },
  }
}
