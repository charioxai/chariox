// F7: a localhost impostor listener posing as the enrolled kernel's loopback endpoint, against the real web client.
import { pathToFileURL } from 'node:url'
import { repo } from './forged-client.mjs'

/** Runs in the page before the app: the page's own client code believes it opened its data channel to the kernel, but the far end is this
 * in-page impostor. A real inner peer still produces a valid offer so the real kernel mints a real grant for it (the grant never reaches the impostor). */
function impostorShim({ mode, kernelId, realKey, realEpoch }) {
  const Real = window.RTCPeerConnection
  const b64 = u8 => btoa(String.fromCharCode(...u8))
  window.RTCPeerConnection = function (cfg) {
    const real = new Real(cfg); const chan = new EventTarget(); chan.binaryType = 'arraybuffer'; chan.bufferedAmount = 0; chan.readyState = 'connecting'
    const emit = obj => { const t = new TextEncoder().encode(JSON.stringify(obj)); const f = new Uint8Array(t.length + 1); f[0] = 0; f.set(t, 1); chan.dispatchEvent(new MessageEvent('message', { data: f.buffer })) }
    chan.send = data => {
      const u = new Uint8Array(data); if (u[0] === 3 || u[0] === 2) return
      let f; try { f = JSON.parse(new TextDecoder().decode(u.subarray(1))) } catch { return }
      window.fcSeen('frame', JSON.stringify(f))
      if (f.kind !== 'local_connect' || mode === 'silentImpostor') return
      const garbage = { sender_public_key: realKey, nonce: 'AAAAAAAAAAAAAAAA', ciphertext: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' }
      crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits']).then(async kp => {
        const raw = new Uint8Array(await crypto.subtle.exportKey('raw', kp.publicKey))
        emit({ kind: 'local_connected', daemon_public_key: mode === 'wrongKeyImpostor' ? b64(raw) : realKey, proof: garbage })
      })
    }
    chan.close = () => { if (chan.readyState !== 'closed') { chan.readyState = 'closed'; chan.dispatchEvent(new Event('close')) } }
    const peer = {
      createDataChannel: (l, o) => { real.createDataChannel(l, o); return chan },
      createOffer: (...a) => real.createOffer(...a), setLocalDescription: d => real.setLocalDescription(d),
      get localDescription() { return real.localDescription }, get iceGatheringState() { return real.iceGatheringState },
      get connectionState() { return 'connected' },
      addEventListener: (t, f) => { if (t === 'icegatheringstatechange') real.addEventListener(t, f) }, removeEventListener: (t, f) => real.removeEventListener(t, f),
      close: () => { real.close() },
      setRemoteDescription: async () => {
        window.fcSeen('upgrade', '')
        setTimeout(() => { chan.readyState = 'open'; chan.dispatchEvent(new Event('open'))
          if (mode !== 'silentImpostor') emit({ kind: 'local_challenge', kernel_id: mode === 'wrongKernelIdImpostor' ? 'kernel-0000000000000000' : kernelId, endpoint_epoch: mode === 'wrongEpochImpostor' ? 'impostor-epoch' : realEpoch, challenge: 'impostor-challenge' }) }, 50)
      },
    }
    return peer
  }
  window.RTCPeerConnection.prototype = Real.prototype
}

export function defineF7(h) {
  const { note, mint, sess, K, target, cell, newIdentity, wsCarrier, prove, connectedOk, goodHeaders, sleep, refusedReason, createRelayKeypair, RelayClientIdentity } = h
  const ipc = async (dir, port) => { process.env.CHARIOX_HOME = `${dir}/chariox`; const { LocalIpcClient } = await import(pathToFileURL(`${repo}/packages/kernel-client/dist/ipc.js`).href); return new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`) }
  const carriers = async page => { await page.keyboard.press('F8'); await sleep(700); const r = await page.evaluate(() => [...document.querySelectorAll('[data-link-carrier]')].map(e => e.getAttribute('data-link-carrier'))); await page.keyboard.press('F8'); return r }
  return {
    async F7() {
      const out = {}
      const local = await ipc('/var/tmp/forgedclient/k1', K.k1.port)
      const dir = '/var/tmp/forgedclient/k1/work/f7room'; (await import('node:fs')).mkdirSync(dir, { recursive: true }); (await import('node:child_process')).execFileSync('git', ['init', '-q', dir])
      const defaults = { provider: 'opencode', account_profile: 'default', model: 'opencode/big-pickle', effort: 'low', permission_level: 'yolo', execution_mode: 'build' }
      const { session, agent } = (await local.send({ CreateSession: { workspace_id: dir, worktree_id: dir, alias: 'f7-room-' + Date.now().toString(36), agent_defaults: defaults } })).SessionCreated
      await local.close?.().catch(() => {})
      const attacker = createRelayKeypair(); const attackerId = new RelayClientIdentity(attacker.privateKey)
      const probe = await mint('k1', newIdentity()); const realKey = probe.kernelKey; const { sdp: psdp, carrier: pcar } = await h.rtc.offer(); const pm = await mint('k1', newIdentity(), { offer: psdp }); const realEpoch = pm.issued.endpoint_epoch; pcar.close(); out.realEpochStable = realEpoch === (await (async () => { const { sdp: s2, carrier: c2 } = await h.rtc.offer(); const m2 = await mint('k1', newIdentity(), { offer: s2 }); c2.close(); return m2.issued.endpoint_epoch })())
      const modes = ['observe', 'wrongKeyImpostor', 'realKeyGarbageProof', 'silentImpostor', 'wrongKernelIdImpostor', 'wrongEpochImpostor']
      for (const mode of modes) {
        const { ctx, page } = await sess.freshPage()
        const seen = { upgrades: [], clientFrames: [], serverFrames: [], endpoint: null }
        await page.exposeFunction('fcSeen', (kind, v) => { if (kind === 'upgrade') seen.upgrades.push('rtc-shim'); else seen.clientFrames.push(v) })
        if (mode !== 'observe') await page.addInitScript(impostorShim, { mode, kernelId: K.k1.daemon, realKey, realEpoch })
        try {
          await page.goto(`${sess.origin}/terminal?ui=os`, { waitUntil: 'domcontentloaded' })
          await page.locator(`[data-room-id="${session.id}"] [data-action="open-room"]`).first().click({ timeout: 90000 })
          await page.locator(`[data-room-pane="${agent.id}"]`).click({ timeout: 60000 }); await page.locator('[data-prompt-input]').waitFor({ timeout: 60000 })
          const end = Date.now() + 45000; let rows = []
          while (Date.now() < end) { rows = await carriers(page); if (mode === 'observe' && rows.includes('direct')) break; if (mode !== 'observe' && seen.upgrades.length && Date.now() > end - 30000) { if (!rows.includes('direct')) {} } await sleep(2500); if (mode !== 'observe' && seen.clientFrames.length && Date.now() > end - 35000) break }
          await sleep(mode === 'observe' ? 0 : 4000); rows = await carriers(page)
          out[mode] = { upgradeAttempts: seen.upgrades.length, clientFramesSeen: seen.clientFrames.length, carriers: rows, impostorCouldRead: seen.impostorCouldRead ?? null, pageStillUsable: await page.locator('[data-prompt-input]').isVisible() }
          if (mode === 'wrongKeyImpostor' && seen.clientFrames[0]) {
            // replay what the impostor captured to the real kernel endpoint: the real challenge differs, so it must be refused
            const id = newIdentity(); const { sdp, carrier } = await h.rtc.offer(); const m = await mint('k1', id, { offer: sdp }); await carrier.accept(m.issued.answer); await carrier.next(5000)
            carrier.send(JSON.parse(seen.clientFrames[0])); const v = await carrier.next(5000); out.replayCapturedProofAtRealKernel = v.kind === 'local_connected' ? 'ADMITTED' : 'denied: ' + (v.reason ?? JSON.stringify(v)); carrier.close()
          }
          await page.screenshot({ path: `${process.env.F7_EVIDENCE ?? '/var/tmp/forgedclient'}/f7-${mode}.png` }).catch(() => {})
        } catch (e) { out[mode] = { error: String(e.message).slice(0, 300), upgradeAttempts: seen.upgrades.length } }
        await ctx.close()
      }
      const direct = r => r?.carriers?.includes('direct')
      const imp = ['wrongKeyImpostor', 'realKeyGarbageProof', 'silentImpostor', 'wrongKernelIdImpostor', 'wrongEpochImpostor']; const strong = ['wrongKeyImpostor', 'realKeyGarbageProof']
      const attempted = imp.every(m => out[m]?.upgradeAttempts > 0)
      const reachedProof = strong.every(m => out[m]?.clientFramesSeen > 0); out.reachedProofStage = reachedProof
      const ok = reachedProof && direct(out.observe) && attempted && imp.every(m => !direct(out[m]) && out[m].pageStillUsable && out[m].impostorCouldRead !== true) && /^denied/.test(out.replayCapturedProofAtRealKernel ?? '')
      cell('F7 localhost impostor cannot pose as the enrolled kernel (real web client)', ok ? 'PASS' : 'FAIL', out)
    },
  }
}
