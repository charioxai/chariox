// MP-08/MP-10/MP-11: a mirror subscription on a page whose renderer is busy (a long parse) on real Chromium.
import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtemp, rm } from "node:fs/promises";
import path from "node:path";
import { KernelBrowserHost } from "./kernel-browser-host.mjs";
import { timestamp } from "./kernel-browser-timing.mjs";

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "MP-11: explicit disposable drill root required");

const stall = ms => `<!doctype html><title>busy</title><h1>Busy page</h1><p>${'text '.repeat(40)}</p><script>for (const end = Date.now() + ${ms}; Date.now() < end;);</script><p>after the stall</p>`;

test("MP-08/MP-11: a navigation to a page that blocks its renderer yields a snapshot shortly after the renderer frees", async () => {
  const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "busy-"));
  const server = createServer((request, response) => {
    response.setHeader("Content-Type", "text/html");
    if (request.url !== "/big") { response.end(request.url === "/busy" ? stall(Number(process.env.STALL_MS ?? 12000)) : "<!doctype html><title>quiet</title><p>quiet</p>"); return; }
    // A 16 MB single-page specification: a long parse of a huge DOM.
    const row = i => `<div class="entry" id="e${i}"><h5>Section ${i}</h5><p>Text <a href="#e${i}">link ${i}</a> <code>code${i}</code> <span>span ${i}</span> <em>emphasis</em> more words of the specification text for entry ${i}.</p></div>\n`;
    response.write("<!doctype html><title>big</title><style>body{font:16px serif;margin:2em}.entry{margin:.5em 0}</style><h1>The big specification</h1>");
    let i = 0; const total = Number(process.env.BIG_ROWS ?? 60000);
    const pump = () => { for (let n = 0; n < 200 && i < total; n++) response.write(row(i++)); if (i < total) setImmediate(pump); else response.end("<p>end</p>"); };
    pump();
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const host = new KernelBrowserHost(root), port = server.address().port, t0 = Date.now(), log = [];
  const marks = process.env.MARKS === "1";
  if (marks) { const timing = host.timing; host.timing = Object.assign((stage, started, ...rest) => { const took = timestamp() - started; if (/^(mirror2_|native_unav)/.test(stage) && took > 200) log.push(`${at()}   mark ${stage} ${Math.round(took)} ms`); return timing(stage, started, ...rest); }, timing); }
  const at = () => ((Date.now() - t0) / 1000).toFixed(1);
  try {
    const opened = await host.request({ op: "open", url: `http://127.0.0.1:${port}/` });
    await host.request({ op: "state" });
    const { subscription_id } = await host.request({ op: "mirror_subscribe", tab_id: opened.tab_id, generation: opened.generation, device_scale_factor: 1, wire: 2, font_requirements: true });
    let applied = 0;
    const next = async () => { const packet = await host.request({ op: "mirror_next", subscription_id, generation: opened.generation, after_sequence: applied, drift_nodes: [], wait_ms: 2000 }); applied = packet.sequence; return packet; };
    await next();
    log.push(`${at()} first quiet packet`);
    const navigated = host.request({ op: "navigate", tab_id: opened.tab_id, generation: opened.generation, url: process.env.BUSY_URL ?? `http://127.0.0.1:${port}${process.env.BUSY_PATH ?? "/busy"}` }).catch(error => log.push(`${at()} navigate ${error.code}`));
    // A snapshot is the reset packet that carries the page (a deflated, possibly chunked body).
    const navStart = Date.now(); let snapshot = null, timeouts = 0;
    for (let i = 0; i < 40 && !snapshot; i++) {
      const started = Date.now();
      try { const packet = await next(); log.push(`${at()} packet seq ${packet.sequence} reset ${!!packet.reset} bytes ${packet.packet_bytes} keys ${Object.keys(packet).filter(k=>!/base64/.test(k)).join(',')} (${Date.now() - started} ms)`); if (packet.reset && packet.packet_bytes > 500) snapshot = packet; }
      catch (error) { if (error.code === "browser_cdp_timeout") timeouts++; log.push(`${at()} next ERR ${error.code} ${String(error.message).slice(0, 80)} (${Date.now() - started} ms)`); applied = 0; }
    }
    await navigated;
    console.log(log.join("\n"));
    assert(snapshot, "MP-08: a snapshot arrives");
    assert.equal(timeouts, 0, "a busy renderer is waited for, not reported as a failed subscription");
    assert(Date.now() - navStart < Number(process.env.STALL_MS ?? 12000) + 4000, `snapshot ${(Date.now() - navStart) / 1000}s after the navigation`);
  } finally {
    await host.stop();
    await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
});
