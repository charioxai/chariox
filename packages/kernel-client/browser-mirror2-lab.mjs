// MP-08/MP-10/MP-11: DOM mirror v2 component lab. The real Node host controller
// drives system Chrome on real public sites; a second headless Chrome renders
// the real v2 renderer bundle over a loopback JSON bridge. Supplementary only:
// no Rust kernel, relay, Cloud UI, shaping or real desktop viewer.
// usage (root): node browser-mirror2-lab.mjs <output-dir> [sites] [dprs] [wire]
import { createServer } from 'node:http';
import { readFile, writeFile, mkdir, cp, mkdtemp, chmod, chown, rm, readdir } from 'node:fs/promises';
import path from 'node:path';
import { spawn, execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { gzipSync } from 'node:zlib';
const here = path.dirname(fileURLToPath(import.meta.url));
const CHROME = '/opt/google/chrome/chrome';
export const labSites = [
  { id: 'wikipedia-article', url: 'https://en.wikipedia.org/wiki/Ada_Lovelace' },
  { id: 'wikipedia-portal', url: 'https://www.wikipedia.org/' },
  { id: 'github-repository', url: 'https://github.com/charioxai/chariox' },
  { id: 'google-search', url: 'https://www.google.com/search?q=Chariox+open+source' },
  { id: 'bbc-news', url: 'https://www.bbc.com/news' },
  { id: 'mdn-docs', url: 'https://developer.mozilla.org/en-US/docs/Web/JavaScript' },
];
if (process.argv[2] !== 'child') {
  const [output, sites = labSites.map(s => s.id).join(','), dprs = '1,2', wire = '2'] = process.argv.slice(2);
  if (!output || !path.isAbsolute(output)) throw Error('MP-10: absolute external evidence path required');
  const tools = process.env.MIRROR_LAB_NODE_MODULES; // Cloud checkout node_modules (playwright-core, esbuild)
  if (!tools || !path.isAbsolute(tools)) throw Error('MP-10: MIRROR_LAB_NODE_MODULES required');
  const parent = process.env.MIRROR_LAB_ROOT ?? '/var/lib/chariox/dev/mirror';
  await mkdir(parent, { recursive: true, mode: 0o755 });
  const root = await mkdtemp(path.join(parent, 'lab-'));
  await chmod(root, 0o755); await mkdir(output, { recursive: true });
  let code = 1;
  try {
    const repo = path.join(here, '../..');
    const source = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repo, encoding: 'utf8' }).trim();
    const dirty = execFileSync('git', ['status', '--porcelain', '--untracked-files=no'], { cwd: repo, encoding: 'utf8' }).trim();
    await cp(path.join(here, 'browser-mirror2-lab.mjs'), path.join(root, 'run.mjs'));
    await cp(path.join(here, 'browser-mirror-metrics.mjs'), path.join(root, 'browser-mirror-metrics.mjs'));
    await mkdir(path.join(root, 'controller'));
    const docker = path.join(repo, 'apps/kernel/slice-linux-docker/docker');
    for (const name of await readdir(docker)) if (name.endsWith('.mjs') && !name.includes('.test.')) await cp(path.join(docker, name), path.join(root, 'controller', name));
    await mkdir(path.join(root, 'client'));
    execFileSync(process.env.MIRROR_LAB_ESBUILD ?? path.join(tools, '.bin/esbuild'), [path.join(here, 'src/browser-mirror2.ts'), '--bundle', '--format=esm', '--target=es2022', `--outfile=${path.join(root, 'client/mirror2.js')}`], { stdio: 'inherit' });
    execFileSync(process.env.MIRROR_LAB_ESBUILD ?? path.join(tools, '.bin/esbuild'), [path.join(here, 'src/browser-mirror.ts'), '--bundle', '--format=esm', '--target=es2022', `--outfile=${path.join(root, 'client/mirror1.js')}`], { stdio: 'inherit' });
    const playwright = path.dirname(execFileSync('node', ['-e', 'console.log(require.resolve("playwright-core/package.json"))'], { cwd: path.dirname(tools), encoding: 'utf8' }).trim());
    await cp(playwright, path.join(root, 'node_modules/playwright-core'), { recursive: true, dereference: true });
    for (const name of ['home', 'evidence']) { await mkdir(path.join(root, name), { mode: 0o700 }); await chown(path.join(root, name), 65534, 65534); }
    const child = spawn(process.execPath, [path.join(root, 'run.mjs'), 'child', root, sites, dprs, wire], { uid: 65534, gid: 65534, cwd: root,
      env: { PATH: '/usr/bin:/bin', HOME: path.join(root, 'home'), TMPDIR: path.join(root, 'home'), CHARIOX_KERNEL_BROWSER_HEADLESS: '1', CHARIOX_KERNEL_BROWSER_MIRROR: '1', CHARIOX_KERNEL_BROWSER_EXECUTABLE: CHROME, CHARIOX_BROWSER_DISPLAY_TIMING: '1', MIRROR_LAB_SETTLE_MS: process.env.MIRROR_LAB_SETTLE_MS ?? '', MIRROR_LAB_DUMP: process.env.MIRROR_LAB_DUMP ?? '' },
      stdio: ['ignore', 'pipe', 'pipe'] });
    let logs = ''; for (const stream of [child.stdout, child.stderr]) stream.on('data', chunk => { process.stdout.write(chunk); logs += chunk; if (logs.length > 1 << 20) logs = logs.slice(-(1 << 20)); });
    code = await new Promise(resolve => child.once('exit', exit => resolve(exit ?? 1)));
    await cp(path.join(root, 'evidence'), output, { recursive: true });
    await writeFile(path.join(output, 'run.log'), logs);
    await writeFile(path.join(output, 'provenance.json'), JSON.stringify({ items: ['MP-08', 'MP-10', 'MP-11'], source, tracked_dirty: Boolean(dirty), command: process.argv.slice(1), exit_code: code, chrome: execFileSync(CHROME, ['--version'], { encoding: 'utf8' }).trim(), topology: 'in-process Node host controller + system Chrome (headless, sandboxed, uid 65534) + headless viewer Chrome, loopback JSON bridge; no Rust, relay, Cloud UI or shaping', cleanup: 'host Chrome and viewer stopped; disposable root removed' }, null, 2));
  } finally { await rm(root, { recursive: true, force: true }); }
  process.exitCode = code;
} else {
  const [root, siteList, dprList, wireArg] = process.argv.slice(3);
  const require = createRequire(path.join(root, 'run.mjs')), { chromium } = require('playwright-core');
  const { KernelBrowserHost } = await import(path.join(root, 'controller/kernel-browser-host.mjs'));
  const { decodePng } = await import(path.join(root, 'controller/kernel-browser-pixels.mjs'));
  const { mirrorRasterMetrics } = await import(path.join(root, 'browser-mirror-metrics.mjs'));
  const wire = Number(wireArg), evidence = path.join(root, 'evidence'), settle = Number(process.env.MIRROR_LAB_SETTLE_MS || 4000);
  const host = new KernelBrowserHost(path.join(root, 'home/source'));
  const results = []; const wireLog = []; let currentCell = '';
  const kernel = async request => {
    const command = request.KernelBrowser.command;
    let result;
    try {
    result = command.op === 'mirror_input'
      ? await host.request({ op: 'input', tab_id: command.tab_id, generation: command.generation, document_id: command.document_id, input: { kind: 'mirror', subscription_id: command.subscription_id, sequence: command.sequence, action: command.action }, observed_by: 'lab' })
      : await host.request({ ...command, observed_by: 'lab' });
    } catch (error) { if (command.op === 'mirror_next') wireLog.push({ at: Date.now(), error: String(error?.message ?? error).slice(0, 200) }); throw error; }
    const json = JSON.stringify({ KernelBrowser: { result } });
    if (command.op === 'mirror_next' && process.env.MIRROR_LAB_DUMP === '1' && !wireLog.some(entry => entry.raw)) await writeFile(path.join(evidence, `${currentCell}-first-packet.json.gz`), gzipSync(json));
    if (command.op === 'mirror_next') wireLog.push({ at: Date.now(), sequence: result.sequence, reset: result.reset, fallback: result.fallback ?? result.fallback_reason ?? result.nodes?.find?.(n => n.reason === 'observer_bounds_or_unavailable')?.reason ?? null, tile_reasons: wire === 1 ? Object.entries((result.nodes ?? []).reduce((m, n) => (n.reason ? (m[n.reason] = (m[n.reason] ?? 0) + 1) : 0, m), {})) : undefined, raw: json.length, gzip: gzipSync(json).length, ops: result.ops?.length ?? null, nodes: result.nodes?.length ?? null, resources: result.resources?.length ?? 0, tiles: result.tiles?.length ?? 0 });
    return json;
  };
  const viewerPage = `<!doctype html><html><body style="margin:0"><div id="mirror"></div><script type="module">
    const rpc=async request=>{const reply=await fetch('/kernel',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(request)});const text=await reply.text();if(!reply.ok)throw Error(text);return JSON.parse(text)};
    window.packets=[];window.failures=[];
    window.start=async (binding,wire)=>{
      if(wire===2){const {attachBrowserMirror2}=await import('/mirror2.js');window.mirror=await attachBrowserMirror2({protocolVersion:482,request:rpc},document.querySelector('#mirror'),binding,{failure:e=>window.failures.push(String(e?.message??e)),packet:p=>window.packets.push({at:performance.now(),sequence:p.sequence,reset:p.reset,fallback:p.fallback??null})});}
      else{const {attachBrowserMirror}=await import('/mirror1.js');window.mirror=await attachBrowserMirror({protocolVersion:443,request:rpc},document.querySelector('#mirror'),binding,e=>window.failures.push(String(e?.message??e)));window.pump=(async()=>{for(;;){try{const p=await window.mirror.next();window.packets.push({at:performance.now(),sequence:p.sequence,reset:p.reset,fallback:p.nodes.find(n=>n.reason==='observer_bounds_or_unavailable')?'observer_bounds_or_unavailable':null})}catch(e){window.failures.push(String(e?.message??e));break}await new Promise(r=>setTimeout(r,250))}})();}
    };</script></body></html>`;
  const server = createServer(async (req, res) => {
    try {
      if (req.method === 'POST' && req.url === '/kernel') { let body = ''; for await (const chunk of req) body += chunk; res.setHeader('content-type', 'application/json'); res.end(await kernel(JSON.parse(body))); return; }
      if (req.url === '/') { res.setHeader('content-type', 'text/html'); res.end(viewerPage); return; }
      if (req.url === '/mirror2.js' || req.url === '/mirror1.js') { res.setHeader('content-type', 'text/javascript'); res.end(await readFile(path.join(root, 'client', req.url.slice(1)))); return; }
      res.writeHead(404).end();
    } catch (error) { res.writeHead(500).end(String(error?.message ?? error)); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const viewerUrl = `http://127.0.0.1:${server.address().port}/`;
  const browser = await chromium.launch({ executablePath: CHROME, headless: true, ignoreDefaultArgs: ['--hide-scrollbars'], args: ['--disable-frame-rate-limit'] });
  const sites = labSites.filter(s => siteList.split(',').includes(s.id));
  const main = async (tab, expression) => { const { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id); const reply = await connection.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }, sessionId); if (reply.exceptionDetails) throw Error(reply.exceptionDetails.exception?.description ?? 'evaluate failed'); return reply.result.value; };
  try {
    for (const dpr of dprList.split(',').map(Number)) for (const site of sites) {
      const row = { site: site.id, url: site.url, dpr, wire }; results.push(row); wireLog.length = 0; currentCell = `${site.id}-dpr${dpr}`;
      let page, tabId, generation;
      try {
        const opened = await host.request({ op: 'open', url: site.url, observed_by: 'lab' }); generation = opened.generation; tabId = opened.tab_id;
        const state0 = await host.request({ op: 'state', observed_by: 'lab' }); let tab = host.tabs.get(tabId);
        // Fixed device metrics before the page settles (as the product subscribe does).
        const prepared = await host.request({ op: 'mirror_subscribe', tab_id: tabId, generation, device_scale_factor: dpr, wire: wire === 2 ? 2 : undefined, observed_by: 'lab' });
        await host.request({ op: 'mirror_close', subscription_id: prepared.subscription_id, generation, observed_by: 'lab' });
        void state0;
        for (let i = 0; i < 100; i++) { try { if (await main(tab, 'document.readyState') === 'complete') break; } catch {} await new Promise(r => setTimeout(r, 200)); }
        await new Promise(r => setTimeout(r, 1500));
        tab = host.tabs.get(tabId);
        row.title = await main(tab, 'document.title').catch(() => null);
        row.loaded = !!row.title && !/just a moment|verify you are human|access denied|unusual traffic|robot check|sorry/i.test(row.title);
        const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: dpr });
        page = await context.newPage(); const consoleLog = []; page.on('console', m => consoleLog.push(`${m.type()}: ${m.text()}`)); page.on('pageerror', e => consoleLog.push(`pageerror: ${e.message}`));
        await page.goto(viewerUrl); await page.waitForFunction(() => typeof window.start === 'function');
        const startedAt = Date.now();
        await page.evaluate(([binding, w]) => window.start(binding, w), [{ tab_id: tabId, generation, device_scale_factor: dpr }, wire]);
        await page.waitForFunction(() => window.packets.length > 0 || window.failures.length > 0, null, { timeout: 30000 });
        row.first_packet_ms = Date.now() - startedAt;
        await new Promise(r => setTimeout(r, settle));
        row.failures = await page.evaluate(() => window.failures);
        const packets = await page.evaluate(() => window.packets);
        row.packets = packets.length; row.fallback = packets.find(p => p.fallback)?.fallback ?? null;
        row.wire = { first: wireLog[0] ?? null, total_raw: wireLog.reduce((a, b) => a + b.raw, 0), total_gzip: wireLog.reduce((a, b) => a + b.gzip, 0), packets: wireLog.length };
        // Source and viewer rasters (masked protected regions are not part of this lab's fixtures).
        tab = host.tabs.get(tabId);
        const shot = await host.screenshot(tab); await writeFile(path.join(evidence, `${site.id}-dpr${dpr}-source.png`), Buffer.from(shot.data_base64, 'base64'));
        const frameBox = await page.evaluate(() => { const f = document.querySelector('iframe'); const r = f?.getBoundingClientRect(); return r ? [r.x, r.y] : null; });
        const viewerShot = frameBox ? await page.screenshot({ clip: { x: frameBox[0], y: frameBox[1], width: 1280, height: 800 } }) : null;
        if (viewerShot) await writeFile(path.join(evidence, `${site.id}-dpr${dpr}-mirror.png`), viewerShot);
        if (viewerShot) {
          const a = decodePng(shot.data_base64, dpr), b = decodePng(viewerShot.toString('base64'), dpr);
          let differ = 0; const total = Math.min(a.width, b.width) * Math.min(a.height, b.height);
          for (let y = 0; y < Math.min(a.height, b.height); y++) for (let x = 0; x < Math.min(a.width, b.width); x++) { const i = (y * a.width + x) * 4, j = (y * b.width + x) * 4; if (Math.abs(a.pixels[i] - b.pixels[j]) > 24 || Math.abs(a.pixels[i + 1] - b.pixels[j + 1]) > 24 || Math.abs(a.pixels[i + 2] - b.pixels[j + 2]) > 24) differ++; }
          row.pixel_mismatch = differ / total;
          if (a.width === b.width && a.height === b.height) { const m = mirrorRasterMetrics({ width: a.width, height: a.height, data: a.pixels }, { width: b.width, height: b.height, data: b.pixels }); row.raster = { outside_edge_mismatch_fraction: m.outside_edge_mismatch_fraction, pixel_mse: m.pixel_mse }; }
        }
        if (wire === 2) {
          const { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
          const { frameTree } = await connection.send('Page.getFrameTree', {}, sessionId);
          const world = await host.browser.ensureFocusWorld(connection, sessionId, tab.target_id, frameTree.frame);
          const evalWorld = async expression => (await connection.send('Runtime.evaluate', { expression, contextId: world.contextId, returnByValue: true }, sessionId)).result?.value;
          row.c_text = await evalWorld('globalThis.__charioxMirror2?.textCoverage()');
          const opaque = await evalWorld('globalThis.__charioxMirror2?.opaqueBoxes()') ?? [];
          let area = 0; for (const { box } of opaque) { const w = Math.max(0, Math.min(1280, box[0] + box[2]) - Math.max(0, box[0])), h = Math.max(0, Math.min(800, box[1] + box[3]) - Math.max(0, box[1])); area += w * h; }
          row.c_area = 1 - Math.min(1, area / (1280 * 800)); row.opaque_regions = opaque.length;
          // Delta path latency without network: a page mutation -> viewer DOM.
          const samples = [];
          for (let i = 0; i < 10; i++) {
            const marker = `mirror-lab-${Date.now()}-${i}`;
            const t0 = Date.now();
            await main(tab, `(()=>{const p=document.createElement('p');p.textContent=${JSON.stringify(marker)};p.style.cssText='position:fixed;left:0;top:0;z-index:2147483647;background:#ff0';document.body.append(p);setTimeout(()=>p.remove(),400);return true})()`);
            await page.waitForFunction(m => document.querySelector('iframe')?.contentDocument?.body?.textContent.includes(m), marker, { timeout: 5000 }).catch(() => null);
            samples.push(Date.now() - t0); await new Promise(r => setTimeout(r, 500));
          }
          row.delta_latency_ms = samples.sort((x, y) => x - y); row.delta_p50 = samples[4]; row.delta_p95 = samples[9];
        }
        row.timings = host.timing?.summary?.() ?? null;
        const table = [...host.mirror.streams.values()].filter(s => s.wire === 2).flatMap(s => [...s.resources.values()].map(({ key, url, kind, state, tries, resource }) => ({ key, url: url.slice(0, 160), kind, state, tries, mime: resource?.mime_type ?? null, bytes: resource?.data_base64?.length ?? 0 })));
        row.resource_states = table.reduce((m, e) => (m[`${e.kind}:${e.state}`] = (m[`${e.kind}:${e.state}`] ?? 0) + 1, m), {});
        await writeFile(path.join(evidence, `${site.id}-dpr${dpr}-resources.json`), JSON.stringify(table, null, 1));
        await writeFile(path.join(evidence, `${site.id}-dpr${dpr}-console.json`), JSON.stringify(consoleLog.slice(0, 500), null, 2));
        await writeFile(path.join(evidence, `${site.id}-dpr${dpr}-wire.json`), JSON.stringify(wireLog, null, 2));
        row.status = row.failures.length || row.fallback ? 'FAIL' : 'OK';
      } catch (error) { row.status = 'ERROR'; row.error = String(error?.stack ?? error).slice(0, 2000); }
      finally {
        await page?.context().close().catch(() => {});
        if (tabId) await host.request({ op: 'close', tab_id: tabId, generation: host.generation, observed_by: 'lab' }).catch(() => {});
        await writeFile(path.join(evidence, 'RESULTS.json'), JSON.stringify(results, null, 2));
        console.log(JSON.stringify({ site: row.site, dpr: row.dpr, status: row.status, fallback: row.fallback, res: row.resource_states, first: row.wire?.first, c_text: row.c_text, c_area: row.c_area, mismatch: row.pixel_mismatch, outside_edge: row.raster?.outside_edge_mismatch_fraction, delta_p50: row.delta_p50, delta_p95: row.delta_p95, error: row.error?.slice(0, 300) }));
      }
    }
  } finally { await browser.close().catch(() => {}); await host.stop(); server.close(); }
}
