#!/usr/bin/env node
// Kernel-layer baselines for rows 545 / 546 / 547 on the REAL KernelBrowserHost + Chrome, against the owned fixtures.
//   node kernel-layer-drill.mjs <545|546|547> [--dpr 1|2]
// What this proves, and what it does not: each check is something the KERNEL hands the person's client (tabs with openers,
// a mirror that keeps delivering after a navigation or popup, a canvas tile that is drawn). On main all of them HOLD (GREEN),
// so rows 545/546/547 are NOT reproduced at this layer: the failing part is in the web client / relay path (card binding,
// mirror re-subscription, region projection) and needs the hosted web stack: person/person-driver.mjs scenes d3-3/d3-4/d3-5.
// Run on a host that is not inside Bubblewrap (see host-harness.mjs).
import { readFileSync } from "node:fs"
import { startAll } from "../fixtures/serve.mjs"
import { delay, packetText, reporter, withHost } from "./host-harness.mjs"

import { dprFrom } from "./args.mjs"
const row = process.argv[2], dpr = dprFrom(process.argv)
if (!["545", "546", "547"].includes(row)) { console.error("usage: kernel-layer-drill.mjs <545|546|547> [--dpr 1|2]"); process.exit(2) }
const r = reporter(row)
// D3_ORIGINS_FILE: the origins.json of an already running fixture host (the Boat); otherwise fixtures start locally on 127.0.0.1 / localhost.
const all = process.env.D3_ORIGINS_FILE
  ? { origins: JSON.parse(readFileSync(process.env.D3_ORIGINS_FILE, "utf8")), close: async () => {} }
  : await startAll({ stateDir: null, basePort: 19000 + Math.floor(Math.random() * 3000), altHost: "localhost" })
const click = async (host, tab, selector) => {
  const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(tab.tab_id).target_id)
  const at = await connection.send("Runtime.evaluate", { expression: `(()=>{const b=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return [b.x+b.width/2,b.y+b.height/2]})()`, returnByValue: true }, sessionId)
  for (const type of ["mousePressed", "mouseReleased"]) await connection.send("Input.dispatchMouseEvent", { type, x: at.result.value[0], y: at.result.value[1], button: "left", clickCount: 1 }, sessionId)
}
const drain = async (mirror, count) => { const seen = []; for (let i = 0; i < count; i++) seen.push(await mirror.next(700)); return seen }

try {
  await withHost(async ({ host, open, subscribe }) => {
    if (row === "546") {
      const tab = await open(all.origins.login + "/"), mirror = await subscribe(tab); await mirror.next(1000)
      await tab.evaluate("document.querySelector('#user').value='alice';document.querySelector('#pass').value='x'.repeat(12);document.querySelector('#go').click();true")
      let packets, error = null
      try { packets = await drain(mirror, 4) } catch (e) { error = e }
      r.check("cross-site form navigation: the mirror subscription keeps answering (no mirror_subscription_retired)", !error, error ? `${error.code}: ${String(error.message).slice(0, 120)}` : "")
      r.check("the new document's text reaches the viewer (reset packet carries 'Welcome')", !error && packets.some(p => packetText(p).includes("Welcome")))
      const rp = await open(all.origins.oauth_rp + "/"), m2 = await subscribe(rp); await m2.next(1000)
      await click(host, rp, "#google"); await delay(1500)
      let popupError = null; try { await drain(m2, 3) } catch (e) { popupError = e }
      r.check("a popup opened by the page does not retire the opener tab's mirror subscription", !popupError, popupError ? `${popupError.code}: ${String(popupError.message).slice(0, 120)}` : "")
    }
    if (row === "545") {
      const rp = await open(all.origins.oauth_rp + "/")
      await delay(800); await click(host, rp, "#google")
      let state, popup
      for (let i = 0; i < 20 && !popup; i++) { await delay(250); state = await host.request({ op: "state" }); popup = state.tabs.find(t => t.opener_tab_id === rp.tab_id) }
      r.check("kernel: the popup is a tab that names its opener", !!popup)
      r.check("kernel: the popup becomes the active tab (the card stays bound to the opener: that is what the person sees as 'opened in another tab')", popup && state.active_tab_id === popup.tab_id, popup ? `active=${state.active_tab_id.slice(-6)} popup=${popup.tab_id.slice(-6)}` : "")
      if (!popup) return   // the checks above already failed; unwind (browser, state, fixtures) before the verdict is reported
      await delay(800)
      const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(popup.tab_id).target_id)
      const at = await connection.send("Runtime.evaluate", { expression: "(()=>{const b=document.getElementById('allow').getBoundingClientRect();return [b.x+b.width/2,b.y+b.height/2]})()", returnByValue: true }, sessionId)
      for (const type of ["mousePressed", "mouseReleased"]) await connection.send("Input.dispatchMouseEvent", { type, x: at.result.value[0], y: at.result.value[1], button: "left", clickCount: 1 }, sessionId)
      await delay(2500)
      r.check("kernel: sign-in completes, the popup closes and the opener reports 'OAuth login OK'", (await rp.evaluate("document.getElementById('status').textContent")) === "OAuth login OK" && (await host.request({ op: "state" })).tabs.length === 1)
    }
    if (row === "547") {
      const tab = await open(all.origins.human + "/"); await delay(800)
      const mirror = await subscribe(tab, { dpr }); const first = await mirror.next(800)
      const canvas = (first.tiles ?? []).find(t => t.width === 300 && t.height === 300)
      r.check("the cross-origin iframe's canvas reaches the viewer as a 300x300 tile", !!canvas, `tiles=${(first.tiles ?? []).map(t => `${t.width}x${t.height}`).join(",")}`)
      if (canvas) {
        const png = Buffer.from(canvas.data_base64, "base64")
        const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(tab.tab_id).target_id)
        // Decode in the browser: colour count of the tile, so "black" is a number.
        const out = await connection.send("Runtime.evaluate", { awaitPromise: true, returnByValue: true, expression: `(async()=>{const b=await createImageBitmap(await (await fetch('data:image/png;base64,${png.toString("base64")}')).blob());const c=new OffscreenCanvas(b.width,b.height),g=c.getContext('2d');g.drawImage(b,0,0);const d=g.getImageData(0,0,b.width,b.height).data;const s=new Set();let dark=0;for(let i=0;i<d.length;i+=4){s.add(d[i]<<16|d[i+1]<<8|d[i+2]);if(d[i]+d[i+1]+d[i+2]<30)dark++}return {colours:s.size,dark_share:dark/(d.length/4)}})()` }, sessionId)
        r.check("the tile is drawn, not black (more than 8 colours, under 50% near-black)", out.result.value.colours > 8 && out.result.value.dark_share < 0.5, JSON.stringify(out.result.value))
      }
    }
  })
} finally { await all.close() }
r.done()
