// Captcha/iframe fidelity scenarios for browser-mirror2-lab.mjs (MIRROR_LAB_SCENARIOS).
// Real public hCaptcha demo (non-Google) plus a local nested cross-origin iframe image grid.
// The scenarios only open and screenshot challenges; they never solve or evade a bot check.
import { createServer } from 'node:http';
const servers = [];
const listen = async handler => { const server = createServer(handler); servers.push(server); await new Promise(r => server.listen(0, '127.0.0.1', r)); return server.address().port; };
export const close = async () => { for (const server of servers) server.close(); };

// An image-grid challenge as a cross-origin popup frame (document with a doctype, 400x580, like the
// widgets that sit in the page's body): the layout rules are the table/tile rules of the widgets'
// public stylesheet, so row geometry depends on the frame document being in standards mode.
const tile = 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="378" height="378">' + Array.from({ length: 9 }, (_, i) => `<rect x="${(i % 3) * 126}" y="${Math.floor(i / 3) * 126}" width="126" height="126" fill="hsl(${i * 40} 60% 50%)"/><text x="${(i % 3) * 126 + 40}" y="${Math.floor(i / 3) * 126 + 70}" font-size="40" fill="#fff">${i + 1}</text>`).join('') + '</svg>');
const tile2 = 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="378" height="378"><rect width="378" height="378" fill="teal"/></svg>');
const measureDoc = d => ({ compat: d.compatMode, rows: [...d.querySelectorAll('tr')].map(r => { const b = r.getBoundingClientRect(); return [Math.round(b.top), Math.round(b.height)]; }), tiles: [...d.querySelectorAll('.rc-image-tile-wrapper')].map(w => { const b = w.getBoundingClientRect(); return [Math.round(b.width), Math.round(b.height)]; }) });
const gridFrame = doctype => `${doctype}<html><head><meta charset="utf-8"><style>
body{margin:0;font-family:Roboto,arial,sans-serif;height:100%}html{height:100%}
.rc-imageselect-challenge{position:relative;width:100%;height:100%;box-sizing:border-box;text-align:center;user-select:none}
.rc-imageselect-desc-wrapper{background-color:#1a73e8;position:relative;padding:24px;color:white;height:66px;font-size:16px;margin-bottom:6px;text-align:left}
.rc-imageselect-payload{min-width:240px;margin:0 7px;padding:7px 0}
.rc-imageselect-table-33{border-collapse:separate;border-spacing:0;width:100%;height:100%;margin:-2px;transition:all 1s ease}
.rc-imageselect-table-33 td{padding:2px}.rc-image-tile-target{position:relative;opacity:1}
.rc-image-tile-wrapper{overflow:hidden;position:relative;transition:.1s ease;transform:scale(1)}
.rc-image-tile-wrapper img{position:relative;backface-visibility:hidden}.rc-image-tile-33{width:300%;height:300%}
.rc-imageselect-tileselected .rc-image-tile-wrapper{transform:scale(.8)}
.rc-footer{height:60px;text-align:left;padding:10px 7px;box-sizing:border-box}button{height:36px;padding:0 24px;background:#1a73e8;color:#fff;border:0;border-radius:3px}
</style></head><body><div class="rc-imageselect-challenge"><div class="rc-imageselect-desc-wrapper">Select all images with <b>traffic lights</b></div>
<div class="rc-imageselect-payload"><table class="rc-imageselect-table-33"><tbody>${[0, 1, 2].map(i => `<tr>${[0, 1, 2].map(j => `<td role="button" tabindex="0" class="rc-imageselect-tile" onclick="pick(this)"><div class="rc-image-tile-target"><div class="rc-image-tile-wrapper" style="width:126px;height:126px"><img class="rc-image-tile-33" src="${tile}" style="top:-${i * 100}%;left:-${j * 100}%"></div></div></td>`).join('')}</tr>`).join('')}</tbody></table></div>
<div class="rc-footer"><button>Verify</button></div></div>
<script>const next=${JSON.stringify(tile2)};function pick(td){td.classList.toggle('rc-imageselect-tileselected');document.title=document.querySelectorAll('.rc-imageselect-tileselected').length;parent.postMessage('picked','*');const img=td.querySelector('img');setTimeout(()=>{img.src=next},300)}</script></body></html>`;
const tilePage = (frameUrl, top = 700, doctype = '<!doctype html>') => `${doctype}<html><head><meta charset="utf-8"><title>Challenge host</title></head><body style="margin:0;font:16px sans-serif"><div style="height:2400px;position:relative;background:linear-gradient(#fff,#dde)"><h1 style="padding:24px">Human check</h1><form style="padding:24px"><input aria-label="name" value="drill"><button type="button">Register</button></form>
<iframe title="challenge" src="${frameUrl}" style="position:absolute;left:300px;top:${top}px;width:400px;height:580px;border:1px solid #ccc;background:#fff;box-shadow:0 2px 8px #0004"></iframe></div></body></html>`;

// The challenge host scrolls by itself: it drifts down for 8 s while its clock ticks (the kernel keeps reporting its scroll), and centres its challenge after every answer.
const selfScrollTilePage = frameUrl => tilePage(frameUrl, 900).replace('</body>', `<script>const f=document.querySelector('iframe'),centre=()=>f.scrollIntoView({block:'center',behavior:'instant'});addEventListener('message',()=>setTimeout(centre,350));let n=0;const drift=setInterval(()=>{scrollBy(0,3);if(++n>=80)clearInterval(drift)},100);setInterval(()=>{document.getElementById('clock').textContent=String(Date.now())},50)</script></body>`).replace('<h1', '<p id="clock" style="position:fixed;right:8px;top:8px">0</p><h1');

// Samples the viewer's own scroll positions (page and the challenge frame) every animation frame while the
// person scrolls/clicks; any move against the person's direction is a jump the person did not make.
const sampler = `(()=>{const top=document.querySelector('iframe').contentDocument;window.__trace=[];const read=()=>{const nested=top.querySelector('iframe[title="challenge"]')?.contentDocument;return [Math.round(performance.now()),Math.round(top.defaultView.scrollY),nested?Math.round(nested.defaultView.scrollY):null]};const tick=()=>{window.__trace.push(read());window.__raf=requestAnimationFrame(tick)};tick();return true})()`;
const jumps = trace => { const out = []; for (let i = 1; i < trace.length; i++) for (const k of [1, 2]) if (trace[i][k] !== null && trace[i - 1][k] !== null && trace[i][k] < trace[i - 1][k] - 2) out.push({ t: trace[i][0] - trace[0][0], what: k === 1 ? 'page' : 'frame', from: trace[i - 1][k], to: trace[i][k] }); return out; };
const wheel = async (page, x, y, dy, steps = 8) => { await page.mouse.move(x, y); for (let i = 0; i < steps; i++) { await page.mouse.wheel(0, dy / steps); await page.waitForTimeout(16); } };

// A long page; `selfScroll` makes the page scroll itself (an anchor restore / focus-driven scroll) while the person scrolls.
const longPage = (selfScroll, ticker = false) => `<!doctype html><html><head><meta charset="utf-8"><title>Long page</title><style>body{margin:0;font:16px sans-serif}section{height:600px;padding:24px;border-bottom:1px solid #ccc}</style></head><body>${Array.from({ length: 12 }, (_, i) => `<section id="s${i}"><h2>Section ${i}</h2><p>Lorem ipsum ${i}</p></section>`).join('')}${ticker ? '<p id="clock" style="position:fixed;right:8px;top:8px">0</p><script>setInterval(()=>{document.getElementById("clock").textContent=String(Date.now())},50)</script>' : ''}${selfScroll ? '<script>setTimeout(()=>window.scrollTo({top:0,behavior:"instant"}),2500)</script>' : ''}</body></html>`;
// Short wheel bursts with human pauses (the kernel answers a burst while the next one starts).
const burstSession = async ({ page }) => {
  const viewerFrame = await page.evaluate(() => { const r = document.querySelector('iframe').getBoundingClientRect(); return [r.x, r.y]; });
  await page.evaluate(sampler.replace("top.querySelector('iframe[title=\"challenge\"]')?.contentDocument", "null"));
  await page.mouse.move(viewerFrame[0] + 300, viewerFrame[1] + 400);
  const t0 = Date.now();
  for (let burst = 0; burst < 14; burst++) { for (let i = 0; i < 4; i++) { await page.mouse.wheel(0, 80); await page.waitForTimeout(30); } await page.waitForTimeout(550 + (burst % 3) * 150); }
  await page.waitForTimeout(2000);
  const trace = await page.evaluate(() => { cancelAnimationFrame(window.__raf); return window.__trace; });
  const found = jumps(trace).filter(j => j.what === 'page');
  const kernelY = await page.evaluate(() => 0);
  return { ms: Date.now() - t0, samples: trace.length, final: trace.at(-1), jumps: found.slice(0, 10), jump_count: found.length, kernelY };
};
// The person's wheel session on the viewer page: moves against the scroll direction are jumps the person did not make.
const scrollSession = async ({ page }) => {
  const viewerFrame = await page.evaluate(() => { const r = document.querySelector('iframe').getBoundingClientRect(); return [r.x, r.y]; });
  await page.evaluate(sampler.replace("top.querySelector('iframe[title=\"challenge\"]')?.contentDocument", "null"));
  await page.mouse.move(viewerFrame[0] + 300, viewerFrame[1] + 400);
  const downs = []; // [t, y] of the page while the wheel runs
  const t0 = Date.now();
  for (let i = 0; i < 100; i++) { await page.mouse.wheel(0, 60); await page.waitForTimeout(35); } // 3.5 s down, through the page's own scroll at 2.5 s
  const endWheel = Date.now() - t0;
  await page.waitForTimeout(1500);
  const trace = await page.evaluate(() => { cancelAnimationFrame(window.__raf); return window.__trace; });
  const found = jumps(trace).filter(j => j.what === 'page');
  return { wheel_ms: endWheel, samples: trace.length, final: trace.at(-1), jumps: found.slice(0, 10), jumps_while_wheel: found.filter(j => j.t < endWheel).length };
};

// The person wheels the page in bursts while it drifts down by itself (any move up is a jump the person did not make), then answers two
// tiles; the page centres its challenge by itself after each answer: once the viewer reached that position it must stay there.
const selfScrollGridSession = async ({ page }) => {
  const viewerFrame = await page.evaluate(() => { const r = document.querySelector('iframe').getBoundingClientRect(); return [r.x, r.y]; });
  await page.evaluate(sampler);
  await page.mouse.move(viewerFrame[0] + 100, viewerFrame[1] + 300);
  const t0 = Date.now(), at = () => Date.now() - t0, events = [];
  for (let burst = 0; burst < 12; burst++) { for (let i = 0; i < 4; i++) { await page.mouse.wheel(0, 40); await page.waitForTimeout(30); } await page.waitForTimeout(450 + (burst % 3) * 150); }
  const wheelEnd = at();
  await page.waitForTimeout(1500);
  for (const nth of [4, 6]) {
    const tileBox = await page.evaluate(n => { const top = document.querySelector('iframe'), r = top.getBoundingClientRect(), f = top.contentDocument.querySelector('iframe[title="challenge"]'), fr = f.getBoundingClientRect(), t = f.contentDocument.querySelectorAll('.rc-imageselect-tile')[n].getBoundingClientRect(); return [r.x + fr.x + t.x + t.width / 2, r.y + fr.y + t.y + t.height / 2]; }, nth);
    if (tileBox[1] < 0 || tileBox[1] > 780) await page.mouse.wheel(0, tileBox[1] < 0 ? -600 : 600), await page.waitForTimeout(1200); // bring the tile into view like a person would
    const again = await page.evaluate(n => { const top = document.querySelector('iframe'), r = top.getBoundingClientRect(), f = top.contentDocument.querySelector('iframe[title="challenge"]'), fr = f.getBoundingClientRect(), t = f.contentDocument.querySelectorAll('.rc-imageselect-tile')[n].getBoundingClientRect(); return [r.x + fr.x + t.x + t.width / 2, r.y + fr.y + t.y + t.height / 2]; }, nth);
    events.push({ what: `answer-${nth}`, from: at() });
    await page.mouse.click(again[0], again[1]);
    await page.waitForTimeout(2500);
  }
  const trace = await page.evaluate(() => { cancelAnimationFrame(window.__raf); return window.__trace; });
  // For each event, the first sample within 2 px of where the page ended up 2 s later; from there it must not leave that position.
  const base = trace[0][0], reverts = [];
  for (const ev of events) {
    const window_ = trace.filter(p => p[0] - base >= ev.from && p[0] - base <= ev.from + 2200);
    if (!window_.length) continue;
    const target = window_.at(-1)[1], reached = window_.findIndex(p => Math.abs(p[1] - target) <= 2);
    const left = window_.slice(reached).filter(p => Math.abs(p[1] - target) > 2);
    if (left.length) reverts.push({ event: ev.what, target, left: left.slice(0, 3).map(p => [p[0] - base, p[1]]) });
  }
  const drifting = jumps(trace).filter(j => j.what === 'page' && j.t < wheelEnd);
  return { samples: trace.length, final: trace.at(-1), events, wheel_ms: wheelEnd, jumps_while_wheel: drifting.length, jumps: drifting.slice(0, 6), reverts, revert_count: reverts.length };
};

export const sites = async () => {
  const longTop = await listen((req, res) => { res.setHeader('content-type', 'text/html'); res.end(longPage(false)); });
  const tickTop = await listen((req, res) => { res.setHeader('content-type', 'text/html'); res.end(longPage(false, true)); });
  const selfTop = await listen((req, res) => { res.setHeader('content-type', 'text/html'); res.end(longPage(true)); });
  const frameStandards = await listen((req, res) => { res.setHeader('content-type', 'text/html'); res.end(gridFrame('<!doctype html>')); });
  const frameOrigin = `http://localhost:${frameStandards}`;
  const top = await listen((req, res) => { res.setHeader('content-type', 'text/html'); res.end(tilePage(`${frameOrigin}/`)); });
  const topSelf = await listen((req, res) => { res.setHeader('content-type', 'text/html'); res.end(selfScrollTilePage(`${frameOrigin}/`)); }); // the challenge below the fold, centred by the page itself
  const topNear = await listen((req, res) => { res.setHeader('content-type', 'text/html'); res.end(tilePage(`${frameOrigin}/`, 90)); }); // the challenge in view without scrolling (real-path drill)
  return [
  { id: 'grid-near-fixture', url: `http://127.0.0.1:${topNear}/` },
  { id: 'selfscroll-grid-fixture', url: `http://127.0.0.1:${topSelf}/`, interact: selfScrollGridSession },
  { id: 'scroll-fixture', url: `http://127.0.0.1:${longTop}/`, interact: scrollSession },
  { id: 'ticker-fixture', url: `http://127.0.0.1:${tickTop}/`, interact: burstSession },
  { id: 'selfscroll-fixture', url: `http://127.0.0.1:${selfTop}/`, interact: scrollSession },
  {
    id: 'grid-popup-fixture', url: `http://127.0.0.1:${top}/`,
    interact: async ({ page }) => {
      const viewerFrame = await page.evaluate(() => { const r = document.querySelector('iframe').getBoundingClientRect(); return [r.x, r.y]; });
      await page.evaluate(sampler);
      await wheel(page, viewerFrame[0] + 100, viewerFrame[1] + 300, 700); await page.waitForTimeout(1200);
      const box = await page.evaluate(() => { const top = document.querySelector('iframe'); const r = top.getBoundingClientRect(), f = top.contentDocument.querySelector('iframe[title="challenge"]').getBoundingClientRect(); return [r.x + f.x, r.y + f.y, f.width, f.height]; });
      const visible = box[1] < 800;
      if (visible) await wheel(page, box[0] + box[2] / 2, box[1] + box[3] / 2, 300); // the frame's own scrollbar (quirks) or the page
      await page.waitForTimeout(1500);
      if (visible) { // a tile: the person answers (twice, with the grid's own re-render after each)
        for (const nth of [4, 6]) {
          const tileBox = await page.evaluate(n => { const top = document.querySelector('iframe'), r = top.getBoundingClientRect(), f = top.contentDocument.querySelector('iframe[title="challenge"]'), fr = f.getBoundingClientRect(), t = f.contentDocument.querySelectorAll('td')[n].getBoundingClientRect(); return [r.x + fr.x + t.x + t.width / 2, r.y + fr.y + t.y + t.height / 2]; }, nth);
          await page.mouse.click(tileBox[0], tileBox[1]); await page.waitForTimeout(1500);
        }
      }
      const trace = await page.evaluate(() => { cancelAnimationFrame(window.__raf); return window.__trace; });
      return { frame_visible: visible, samples: trace.length, final: trace.at(-1), jumps: jumps(trace).slice(0, 10), title: await page.evaluate(() => document.querySelector('iframe').contentDocument.querySelector('iframe[title="challenge"]').contentDocument.title) };
    },
    probe: async ({ host, tab, page }) => {
      const { connection } = await host.browser.resolvePageTarget(tab.target_id);
      const { targetInfos } = await connection.send('Target.getTargets');
      const target = targetInfos.find(t => t.type === 'iframe' && t.url.startsWith(frameOrigin));
      const session = target && host.browser.frameSession?.(target.targetId, connection);
      if (!session) throw Error('challenge frame session not found');
      const source = (await connection.send('Runtime.evaluate', { expression: `(${measureDoc})(document)`, returnByValue: true }, session)).result.value;
      const mirror = await page.evaluate(`(()=>{const top=document.querySelector('iframe').contentDocument;const nested=top.querySelector('iframe[title="challenge"]')?.contentDocument;return nested?(${measureDoc})(nested):null})()`);
      return { source, mirror, same_rows: JSON.stringify(source?.rows) === JSON.stringify(mirror?.rows), same_compat: source?.compat === mirror?.compat };
    },
  },
  ...hcaptcha(),
];};
const hcaptcha = () => [
  {
    id: 'hcaptcha-demo', url: 'https://democaptcha.com/demo-form-eng/hcaptcha.html',
    probe: async ({ host, tab, page }) => {
      const { connection } = await host.browser.resolvePageTarget(tab.target_id);
      const { targetInfos } = await connection.send('Target.getTargets');
      const out = { source: null, mirror: null };
      for (const target of targetInfos.filter(t => t.type === 'iframe')) {
        const session = host.browser.frameSession?.(target.targetId, connection); if (!session) continue;
        const r = await connection.send('Runtime.evaluate', { expression: `(()=>{const h=document.querySelector('.challenge-header');if(!h)return null;const bg=h.querySelector('.prompt-bg');return {html:h.outerHTML.slice(0,2500),bg:bg&&getComputedStyle(bg).cssText.length?{color:getComputedStyle(bg).backgroundColor,opacity:getComputedStyle(bg).opacity,filter:getComputedStyle(bg).filter,mix:getComputedStyle(bg).mixBlendMode,transform:getComputedStyle(bg).transform}:null}})()`, returnByValue: true }, session).catch(() => null);
        if (r?.result?.value) out.source = r.result.value;
      }
      out.mirror = await page.evaluate(`(()=>{const find=d=>{const h=d.querySelector('.challenge-header');if(h)return h;for(const f of d.querySelectorAll('iframe')){try{if(f.contentDocument){const x=find(f.contentDocument);if(x)return x}}catch{}}return null};const h=find(document.querySelector('iframe').contentDocument);if(!h)return null;const bg=h.querySelector('.prompt-bg'),cs=bg&&h.ownerDocument.defaultView.getComputedStyle(bg);const r=h.getBoundingClientRect();const at=h.ownerDocument.elementsFromPoint(r.x+r.width/2,r.y+r.height/2).slice(0,6).map(e=>e.localName+'.'+e.className);return {html:h.outerHTML.slice(0,2500),bg:cs&&{color:cs.backgroundColor,opacity:cs.opacity,filter:cs.filter,mix:cs.mixBlendMode,transform:cs.transform},at}})()`);
      return out;
    },
    // The person scrolls the page, then clicks inside the open challenge frame (no solving: the first click only
    // selects or opens something) while the viewer's scroll position is traced for moves against the scroll.
    interact: async ({ page }) => {
      const viewerFrame = await page.evaluate(() => { const r = document.querySelector('iframe').getBoundingClientRect(); return [r.x, r.y]; });
      await page.evaluate(`(()=>{const top=document.querySelector('iframe').contentDocument;window.__trace=[];const tick=()=>{window.__trace.push([Math.round(performance.now()),Math.round(top.defaultView.scrollY),null]);window.__raf=requestAnimationFrame(tick)};tick()})()`);
      await wheel(page, viewerFrame[0] + 100, viewerFrame[1] + 400, 300); await page.waitForTimeout(1200);
      await wheel(page, viewerFrame[0] + 100, viewerFrame[1] + 400, -300); await page.waitForTimeout(1200);
      const box = await page.evaluate(() => { const top = document.querySelector('iframe'), r = top.getBoundingClientRect(), f = [...top.contentDocument.querySelectorAll('iframe')].find(f => f.getBoundingClientRect().height > 200)?.getBoundingClientRect(); return f ? [r.x + f.x, r.y + f.y, f.width, f.height] : null; });
      if (box) { await page.mouse.click(box[0] + box[2] / 2, box[1] + box[3] * 0.4); await page.waitForTimeout(2500); }
      await wheel(page, viewerFrame[0] + 100, viewerFrame[1] + 400, 200); await page.waitForTimeout(1500);
      const trace = await page.evaluate(() => { cancelAnimationFrame(window.__raf); return window.__trace; });
      return { challenge_frame: box, samples: trace.length, final: trace.at(-1), jumps: jumps(trace).slice(0, 10) };
    },
    prepare: async ({ host, tab, main }) => {
      const { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
      const rect = async selector => main(tab, `(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e)return null;const r=e.getBoundingClientRect();return [r.x,r.y,r.width,r.height]})()`);
      let box = null;
      for (let i = 0; i < 40 && !box; i++) { box = await rect('iframe[src*="hcaptcha-checkbox"], iframe[title*="checkbox"]'); if (!box) await new Promise(r => setTimeout(r, 500)); }
      if (!box) throw Error('hcaptcha checkbox frame not found: ' + JSON.stringify(await main(tab, `({title:document.title,url:location.href,frames:[...document.querySelectorAll('iframe')].map(f=>[f.src.slice(0,120),f.title])})`)));
      const x = box[0] + 28, y = box[1] + box[3] / 2;
      for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) await connection.send('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1 }, sessionId);
      for (let i = 0; i < 40; i++) { const c = await rect('iframe[src*="hcaptcha-challenge"], iframe[title*="hCaptcha challenge"]'); if (c && c[3] > 200) return; await new Promise(r => setTimeout(r, 500)); }
      throw Error('hcaptcha challenge frame did not open');
    },
  },
];
