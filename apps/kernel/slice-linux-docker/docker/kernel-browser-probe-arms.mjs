// MP-08/MP-10/MP-11: launch the kernel browser (through the real KernelBrowserHost) or a stock Chrome of the same build
// behind one page interface, so every signal check and every external site check runs identically on both.
import { spawn } from 'node:child_process';
import { mkdtemp, rm, access } from 'node:fs/promises';
import path from 'node:path';
import { createServer } from 'node:http';
import { createServer as createNetServer } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';
import { pageSignalsExpression, identityServerHandler } from './kernel-browser-signals.mjs';


function pageOver(send) {
  const evaluate = async expression => {
    const r = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true, userGesture: true });
    if (r.exceptionDetails) throw new Error(`page evaluation failed: ${JSON.stringify(r.exceptionDetails).slice(0, 300)}`);
    return r.result.value;
  };
  return {
    evaluate,
    async navigate(url, settleMs = 2500) { await send('Page.navigate', { url }); await delay(settleMs); },
    url: () => evaluate('location.href'),
    text: async (limit = 1500) => (await evaluate(`document.body ? document.body.innerText.slice(0, ${limit}) : ''`)),
    async screenshot() { return Buffer.from((await send('Page.captureScreenshot', { format: 'png' })).data, 'base64'); },
    // Real key events, like a person typing: no value assignment.
    async type(text) { for (const ch of text) { await send('Input.dispatchKeyEvent', { type: 'keyDown', text: ch, key: ch, unmodifiedText: ch }); await send('Input.dispatchKeyEvent', { type: 'keyUp', key: ch }); await delay(40); } },
    async press(key, code, vk) { await send('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk }); await send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk }); },
    async click(x, y) { for (const type of ['mousePressed', 'mouseReleased']) await send('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1 }); },
  };
}

// arm: 'kernel' | 'stock'; mode: 'headful' | 'headless' (the Mac kernel always runs headless); dockerDir: the tree whose
// host launches the kernel browser (so older commits can be compared); executable: pinned Chrome for Testing.
export async function withBrowser({ arm, mode, dockerDir, executable, scratchRoot, env = {}, extraArgs = [] }, run) {
  const scratch = await mkdtemp(path.join(scratchRoot, 'arm-'));
  try {
    if (arm === 'kernel') return await withKernel({ mode, dockerDir, executable, scratch, env }, run);
    return await withStock({ mode, executable, scratch, extraArgs }, run);
  } finally { await rm(scratch, { recursive: true, force: true }); }
}

async function withKernel({ mode, dockerDir, executable, scratch, env }, run) {
  const { KernelBrowserHost } = await import(path.join(dockerDir, 'kernel-browser-host.mjs'));
  const { HostChromium } = await import(path.join(dockerDir, 'kernel-browser-process.mjs'));
  const environment = { ...process.env, ...(executable ? { CHARIOX_KERNEL_BROWSER_EXECUTABLE: executable } : {}), ...(mode === 'headless' ? { CHARIOX_KERNEL_BROWSER_HEADLESS: '1' } : {}), ...env };
  const appStorageRoot = path.join(scratch, 'apps');
  const host = new KernelBrowserHost(scratch, { appStorageRoot, chromium: new HostChromium(scratch, { appStorageRoot, environment }) });
  try {
    const opened = await host.request({ op: 'open', url: 'about:blank' });
    const tab = host.tabs.get(opened.tabs[0].tab_id);
    const { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
    await delay(1500);
    const send = (method, params) => connection.send(method, params ?? {}, sessionId);
    const version = (await connection.send('Browser.getVersion')).product;
    return await run({ ...pageOver(send), version });
  } finally { await host.stop(); }
}

// Not port 0: Chrome treats an ephemeral debugging port as automation (navigator.webdriver true), a fixed port is not.
async function freePort() {
  for (;;) {
    const port = 20000 + Math.floor(Math.random() * 30000);
    const free = await new Promise(resolve => { const s = createNetServer(); s.once('error', () => resolve(false)); s.listen(port, '127.0.0.1', () => s.close(() => resolve(true))); });
    if (free) return port;
  }
}

async function freeDisplay() {
  for (let n = 100 + Math.floor(Math.random() * 100); ; n++) { try { await access(`/tmp/.X11-unix/X${n}`); } catch { return `:${n}`; } }
}

async function withStock({ mode, executable, scratch, extraArgs }, run) {
  const profile = path.join(scratch, 'stock-profile');
  const port = await freePort();
  const args = [`--user-data-dir=${profile}`, '--no-first-run', '--no-default-browser-check', `--remote-debugging-port=${port}`, '--no-sandbox', '--window-size=1280,887', ...extraArgs];
  const childEnv = { ...process.env };
  let xvfb;
  if (mode === 'headless') args.push('--headless=new');
  else {
    const display = await freeDisplay();
    xvfb = spawn('Xvfb', [display, '-screen', '0', '1920x1080x24'], { stdio: 'ignore' }); await delay(1500); childEnv.DISPLAY = display;
  }
  const chrome = spawn(executable, [...args, 'about:blank'], { env: childEnv, stdio: 'ignore' });
  try {
    let url;
    for (let i = 0; i < 60 && !url; i++) { try { url = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.type === 'page')?.webSocketDebuggerUrl; } catch {} if (!url) await delay(250); }
    if (!url) throw new Error('stock Chrome did not start');
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
    let id = 0; const pending = new Map();
    socket.onmessage = message => { const data = JSON.parse(message.data); pending.get(data.id)?.(data); };
    const send = (method, params = {}) => new Promise((resolve, reject) => { const i = ++id; pending.set(i, data => (data.error ? reject(new Error(`${method}: ${data.error.message}`)) : resolve(data.result))); socket.send(JSON.stringify({ id: i, method, params })); });
    await send('Page.enable');
    const version = (await (await fetch(`http://127.0.0.1:${port}/json/version`)).json()).Browser;
    try { return await run({ ...pageOver(send), version }); } finally { socket.close(); }
  } finally { chrome.kill('SIGTERM'); xvfb?.kill('SIGTERM'); await delay(800); }
}

// Everything a page can read about the browser, plus the identity headers its requests carry, on a local page.
export async function collectSignals(options) {
  const seen = [];
  const server = createServer(identityServerHandler(seen));
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  try {
    return await withBrowser(options, async page => {
      await page.navigate(`http://127.0.0.1:${server.address().port}/`, 2500);
      const signals = JSON.parse(await page.evaluate(pageSignalsExpression));
      await page.evaluate("fetch('/echo').then(r => r.text())");
      return { version: page.version, signals, seen };
    });
  } finally { await new Promise(resolve => server.close(resolve)); }
}
