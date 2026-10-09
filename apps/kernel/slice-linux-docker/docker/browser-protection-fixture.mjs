// MP-08/MP-11 regression fixture (opt-in browser tests only): protected
// content is pure magenta, ordinary content (consent dialog, a cross-site
// "reCAPTCHA" frame, article text) pure cyan. Never an acceptance substitute.
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { connectCdpPipe } from './kernel-browser-cdp-pipe.mjs';
import { BrowserCdpClient } from './browser-controller-cdp.mjs';

export const VAULT_VALUE = 'disposable-vault-echo-7d1c';
const M = 'background:#ff00ff', C = 'background:#00ffff';
const page = body => `<!doctype html><meta charset=utf-8><body style="margin:0;font:14px sans-serif">${body}</body>`;
// Query flags: novault (no Vault echo text), nomarkers (secret fields only).
// Mirrored/rotated frames paint their password field away from its layout x.
export function fixtureHtml(url, cross) {
  const markers = !url.includes('nomarkers');
  switch (url.split('?')[0]) {
    case '/captcha': return page(`<div style="${C};width:300px;height:74px"><span style="display:inline-block;width:24px;height:24px;margin:24px;border:2px solid #555;background:#fff"></span></div>`);
    case '/login': return page(`<div style="${C};height:30px"></div><input type=password value=hunter2 style="${M};border:0;margin:10px;width:150px;height:24px">`);
    case '/field': return page(`<input type=password value=hunter2 style="${M};border:0;margin:10px;width:150px;height:24px">`);
    case '/whole': return page(`<div style="${M};width:100%;height:100vh"></div>`);
    case '/echo': return page(`<div style="${C};height:30px"></div><p style="color:#ff00ff;font:bold 18px sans-serif;margin:4px">token ${VAULT_VALUE}</p>`);
    case '/nested': return page(`<iframe src="${cross}/login" style="border:0;margin:6px;width:220px;height:90px"></iframe>`);
    case '/same': return page(`<div style="${C};height:400px"></div><input type=password style="${M};border:0;width:120px;height:20px"><div style="height:400px"></div>`);
    default: return page(`
      <div id=consent role=dialog style="${C};position:absolute;left:20px;top:20px;width:360px;height:120px">We use cookies <button>Accept all</button></div>
      <input id=pw type=password style="${M};position:absolute;left:420px;top:30px;width:160px;height:28px;border:0">
      <input id=otp autocomplete=one-time-code style="${M};position:absolute;left:600px;top:30px;width:100px;height:28px;border:0">
      ${markers ? `<div data-chariox-observation-protected style="display:contents"><div style="${M};position:absolute;left:720px;top:30px;width:80px;height:28px"></div></div>` : ''}
      <iframe id=captcha src="${cross}/captcha" style="position:absolute;left:20px;top:170px;width:304px;height:78px;border:0"></iframe>
      <iframe id=login src="${cross}/login" style="position:absolute;left:360px;top:170px;width:200px;height:80px;border:2px solid #000;padding:3px"></iframe>
      ${markers ? `<iframe id=owner data-chariox-secret src="${cross}/whole" style="position:absolute;left:600px;top:170px;width:120px;height:80px;border:0"></iframe>` : ''}
      <iframe id=same src="/same" style="position:absolute;left:760px;top:170px;width:160px;height:100px;border:0"></iframe>
      <iframe id=nested src="${cross}/nested" style="position:absolute;left:20px;top:280px;width:260px;height:110px;border:0"></iframe>
      <iframe id=scaled src="${cross}/login" style="position:absolute;left:320px;top:280px;width:200px;height:80px;border:0;transform:scale(.5);transform-origin:0 0"></iframe>
      <iframe id=mirrored src="${cross}/field" style="position:absolute;left:320px;top:330px;width:200px;height:80px;border:0;transform:scaleX(-1)"></iframe>
      <iframe id=rotated src="/field" style="position:absolute;left:420px;top:70px;width:200px;height:90px;border:0;transform:rotate(180deg)"></iframe>
      <canvas id=media width=60 height=40 style="position:absolute;left:880px;top:280px;z-index:1"></canvas>
      ${url.includes('novault') ? '' : `<p id=echo style="color:#ff00ff;font:bold 18px sans-serif;position:absolute;left:560px;top:280px;margin:0">token ${VAULT_VALUE}</p>`}
      ${url.includes('novault') ? '' : `<iframe id=echoframe src="${cross}/echo" style="position:absolute;left:560px;top:350px;width:300px;height:60px;border:0"></iframe>`}
      ${url.includes('novault') ? '' : `<canvas id=drawn width=340 height=30 style="position:absolute;left:640px;top:80px"></canvas>
        <script>{ const c = document.getElementById('drawn').getContext('2d'); c.fillStyle = '#ff00ff'; c.font = 'bold 24px sans-serif';
          c.fillText(${JSON.stringify(VAULT_VALUE)}, 0, 24); document.currentScript.remove(); }</script>`}
      <div id=host style="position:absolute;left:560px;top:320px"></div>
      <input id=fixed type=password style="${M};position:fixed;right:10px;bottom:10px;width:90px;height:22px;border:0">
      <article style="${C};position:absolute;left:20px;top:420px;width:880px;height:1600px">Ordinary article text.</article>
      <script>document.getElementById('host').attachShadow({mode:'closed'}).innerHTML='<input type=password style="${M};width:110px;height:22px;border:0">';
        { const c = document.getElementById('media').getContext('2d'); c.fillStyle = '#00ffff'; c.fillRect(0, 0, 60, 40); }
        document.getElementById('same').onload = e => e.target.contentWindow.scrollTo(0, 360);</script>`);
  }
}

export async function serveFixture() {
  const server = createServer((request, response) => {
    const port = server.address().port, host = String(request.headers.host ?? '');
    // Cross-site: 127.0.0.1 and localhost are different sites (isolated frames).
    const cross = host.startsWith('localhost') ? `http://127.0.0.1:${port}` : `http://localhost:${port}`;
    response.setHeader('content-type', 'text/html'); response.end(fixtureHtml(request.url, cross));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return { url: `http://127.0.0.1:${server.address().port}/`, close: () => new Promise(resolve => { server.close(resolve); server.closeAllConnections(); }) };
}

export async function launchChromium({ executable, dpr = 1, headless = true, env = process.env, root, args = [] }) {
  const profile = await mkdtemp(path.join(root, 'chromium-'));
  const child = spawn(executable, [`--user-data-dir=${profile}`, '--remote-debugging-pipe', '--no-first-run', '--no-default-browser-check',
    '--disable-background-networking', `--force-device-scale-factor=${dpr}`, '--window-size=1000,760', ...(headless ? ['--headless=new'] : []),
    // Test harness only: Chromium refuses its sandbox for uid 0 (root builders).
    ...(process.getuid?.() === 0 ? ['--no-sandbox'] : []), ...args, 'about:blank'],
  { env, stdio: ['ignore', 'ignore', 'ignore', 'pipe', 'pipe'] });
  child.on('error', () => {});
  const connection = connectCdpPipe(child.stdio[3], child.stdio[4], 10000);
  const browser = new BrowserCdpClient({ connectionFactory: () => connection });
  await connection.send('Browser.getVersion');
  return { connection, browser, child, async close() {
    await connection.send('Browser.close').catch(() => {});
    for (let i = 0; i < 50 && child.exitCode === null && child.signalCode === null; i++) await delay(20);
    if (child.exitCode === null && child.signalCode === null && Number.isSafeInteger(child.pid) && child.pid > 1) child.kill('SIGKILL');
    await rm(profile, { recursive: true, force: true });
  } };
}

export async function openFixture(browser, url) {
  const connection = await browser.ensureConnection();
  const { targetInfos } = await connection.send('Target.getTargets');
  const target = targetInfos.find(info => info.type === 'page');
  const { sessionId } = await browser.resolvePageTarget(target.targetId);
  await connection.send('Page.navigate', { url }, sessionId);
  for (let i = 0; i < 100; i++) {
    const expected = 7 + !url.includes('nomarkers') + !url.includes('novault');
    const { result } = await connection.send('Runtime.evaluate', { returnByValue: true, expression: `document.readyState === 'complete' && [...document.querySelectorAll('iframe')].length === ${expected}` }, sessionId);
    // Isolated frame targets: the two same-site frames are in-process; nested adds one.
    const frames = (await connection.send('Target.getTargets')).targetInfos.filter(info => info.type === 'iframe').length;
    if (result.value && frames >= expected - 1) break;
    await delay(50);
  }
  await delay(300);
  return { targetId: target.targetId, sessionId, connection };
}

// Exact-color census of an RGBA buffer, optionally after painting regions black.
export function census({ width, pixels }, regions = []) {
  const masked = Buffer.from(pixels);
  for (const [x, y, w, h] of regions) for (let py = Math.max(0, y); py < Math.min(pixels.length / 4 / width, y + h); py++) for (let px = Math.max(0, x); px < Math.min(width, x + w); px++) masked.fill(0, (py * width + px) * 4, (py * width + px) * 4 + 3);
  let magenta = 0, cyan = 0;
  for (let i = 0; i < masked.length; i += 4) {
    if (masked[i] === 255 && masked[i + 1] === 0 && masked[i + 2] === 255) magenta++;
    if (masked[i] === 0 && masked[i + 1] === 255 && masked[i + 2] === 255) cyan++;
  }
  return { magenta, cyan };
}
