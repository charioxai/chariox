// MP-08/MP-10/MP-11: headed geometry regression, supplementary to public-site Web drills.
import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { BrowserCdpClient } from './browser-controller-cdp.mjs';
import { captureProtectedBrowserImage } from './browser-controller-image.mjs';
import { launchChromium } from './browser-protection-fixture.mjs';

async function startXvfb(size) {
  const xvfb = spawn('Xvfb', ['-displayfd', '3', '-screen', '0', size, '-nolisten', 'tcp'], { stdio: ['ignore', 'ignore', 'ignore', 'pipe'] });
  const display = await new Promise((resolve, reject) => {
    xvfb.once('error', reject);
    xvfb.stdio[3].once('data', value => resolve(':' + value.toString().trim()));
    xvfb.once('exit', code => reject(Error(`Xvfb exited: ${code}`)));
  });
  return { xvfb, display };
}

async function stopXvfb(xvfb) {
  if (Number.isSafeInteger(xvfb.pid) && xvfb.pid > 1 && xvfb.exitCode === null) {
    xvfb.kill('SIGTERM'); await new Promise(resolve => xvfb.once('exit', resolve));
  }
}

test('MP-08/MP-10/MP-11: real headed page footer remains inside the desktop below Chromium chrome', { timeout: 60000 }, async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'headed-viewport-'));
  let xvfb, chrome;
  try {
    const started = await startXvfb('3840x2160x24');
    xvfb = started.xvfb;
    const display = started.display;
    chrome = await launchChromium({ executable: process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE,
      root, headless: false, env: { ...process.env, DISPLAY: display }, args: ['--disable-dev-shm-usage'] });
    const { browser, connection } = chrome;
    const { targetInfos } = await connection.send('Target.getTargets');
    const targetId = targetInfos.find(t => t.type === 'page').targetId;
    const { sessionId } = await browser.resolvePageTarget(targetId);
    const { windowId } = await connection.send('Browser.getWindowForTarget', { targetId });
    browser.applyCanonicalDisplay = async () => {};
    for (const [width, height] of [[1280, 800], [1920, 1080]]) for (const dpr of [1, 2]) {
      await connection.send('Emulation.clearDeviceMetricsOverride', {}, sessionId);
      await connection.send('Browser.setWindowBounds', { windowId, bounds: { left: 0, top: 0, width: width * dpr, height: height * dpr, windowState: 'normal' } });
      // Window geometry is asynchronous even after CDP acknowledges its bounds.
      let content;
      for (let n = 0; n < 100; n++) {
        content = (await connection.send('Page.getLayoutMetrics', {}, sessionId)).cssLayoutViewport;
        if (content.clientWidth === width * dpr && content.clientHeight < height * dpr) break;
        await new Promise(r => setTimeout(r, 20));
      }
      assert.ok(content.clientWidth <= width * dpr && content.clientWidth >= width * dpr - 32);
      const toolbar = height * dpr - content.clientHeight;
      assert.ok(toolbar > 0 && toolbar < 200);
      const source = { css_width: width, css_height: height, device_scale_factor: dpr,
        desktop_pixel_width: width * dpr, desktop_pixel_height: height * dpr };
      browser.layoutByTarget.clear(); browser.viewportByTarget.clear();
      await browser.reconcile(source);
      await connection.send('Runtime.evaluate', { expression: `document.body.innerHTML='<main style="height:3000px">Page</main><footer id="bottom">Reachable footer</footer>'; scrollTo(0,document.documentElement.scrollHeight);`, returnByValue: true }, sessionId);
      const { result } = await connection.send('Runtime.evaluate', { expression: '({height:innerHeight,bottom:document.getElementById("bottom").getBoundingClientRect().bottom})', returnByValue: true }, sessionId);
      assert.ok(toolbar + result.value.height * dpr <= height * dpr,
        `layout bottom clipped at ${width}x${height} DPR${dpr}: toolbar=${toolbar}, page=${result.value.height}`);
      assert.ok(toolbar + result.value.bottom * dpr <= height * dpr + 1, 'footer must fit in the visible content area');
      // Reconcile a changed layout repeatedly without resizing the native window.
      // Some Chromium builds retain the visible-size override when it is cleared.
      const stable = browser.layoutViewport(targetId, source);
      for (let repeat = 0; repeat < 4; repeat++) {
        await browser.setLayout(targetId, { width: width - (repeat % 2), height });
        const current = browser.layoutViewport(targetId, source);
        assert.ok(current.css_width >= stable.css_width - 1 && current.css_height === stable.css_height,
          `repeated metrics shrank native content: ${current.css_width}x${current.css_height} from ${stable.css_width}x${stable.css_height}`);
      }
      const replacement = new BrowserCdpClient({ connectionFactory: async () => connection, applyCanonicalDisplay: async () => {} });
      try {
        await replacement.reconcile(source);
        const resumed = replacement.layoutViewport(targetId, source);
        assert.ok(resumed.css_height === stable.css_height && resumed.css_width >= stable.css_width - 1,
          `a fresh CDP session inherited emulated content: ${resumed.css_width}x${resumed.css_height} from ${stable.css_width}x${stable.css_height}`);
      } finally { replacement.memory.close(); replacement.rendererRecovery.close(); }
      const documentId = browser.documentIdsByTarget.get(targetId);
      await captureProtectedBrowserImage({ connection, sessionId, targetId, documentId, viewport: browser.layoutViewport(targetId, source), protectedValues: new Set() });
    }
  } finally {
    await chrome?.close();
    if (xvfb) await stopXvfb(xvfb);
    await rm(root, { recursive: true, force: true });
  }
});

test('MP-08/MP-10/MP-11: real headed fullscreen page applies its viewport under browser zoom', { timeout: 60000 }, async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'headed-zoom-'));
  let xvfb, chrome;
  try {
    const started = await startXvfb('1280x800x24');
    xvfb = started.xvfb;
    const env = { ...process.env, DISPLAY: started.display };
    chrome = await launchChromium({ executable: process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE,
      root, headless: false, env, args: ['--disable-dev-shm-usage'] });
    const { browser, connection } = chrome;
    const { targetInfos } = await connection.send('Target.getTargets');
    const targetId = targetInfos.find(t => t.type === 'page').targetId;
    const { sessionId } = await browser.resolvePageTarget(targetId);
    const { windowId } = await connection.send('Browser.getWindowForTarget', { targetId });
    await connection.send('Browser.setWindowBounds', { windowId, bounds: { windowState: 'fullscreen' } });
    // The X window is replaced while Chromium enters fullscreen; poll until it is stable.
    let window;
    for (let n = 0; n < 100 && !window; n++) {
      try { window = execFileSync('xdotool', ['search', '--onlyvisible', '--class', 'chrome'], { env, stdio: ['ignore', 'pipe', 'ignore'] }).toString().trim().split('\n')[0]; }
      catch { await new Promise(resolve => setTimeout(resolve, 50)); }
    }
    assert.ok(window, 'Chromium X window not found');
    await new Promise(resolve => setTimeout(resolve, 500));
    // Two steps of browser zoom: 110%, then 125%.
    for (let step = 0; step < 2; step++) {
      execFileSync('xdotool', ['windowfocus', '--sync', window], { env });
      execFileSync('xdotool', ['key', '--clearmodifiers', 'ctrl+plus'], { env });
      await new Promise(resolve => setTimeout(resolve, 500));
    }
    let bounds, layout;
    for (let n = 0; n < 100; n++) {
      ({ bounds } = await connection.send('Browser.getWindowForTarget', { targetId }));
      layout = await connection.send('Page.getLayoutMetrics', {}, sessionId);
      if (layout.cssVisualViewport.zoom === 1.25) break;
      await new Promise(resolve => setTimeout(resolve, 20));
    }
    assert.equal(layout.cssVisualViewport.zoom, 1.25, 'Chromium did not apply the browser zoom');
    assert.equal(bounds.windowState, 'fullscreen');
    browser.applyCanonicalDisplay = async () => {};
    const source = { css_width: bounds.width, css_height: bounds.height, device_scale_factor: 1,
      desktop_pixel_width: bounds.width, desktop_pixel_height: bounds.height };
    await browser.reconcile(source);
    await connection.send('Runtime.evaluate', { expression: 'document.body.innerHTML=\'<footer id="bottom" style="position:fixed;bottom:0">Reachable footer</footer>\'' }, sessionId);
    const { result } = await connection.send('Runtime.evaluate', { expression: '({width:innerWidth,height:innerHeight,bottom:document.getElementById("bottom").getBoundingClientRect().bottom})', returnByValue: true }, sessionId);
    const applied = browser.viewportByTarget.get(targetId).metrics;
    // Without a window manager Chromium's fullscreen window keeps its left offset; the desktop edge clips it.
    const visibleWidth = bounds.width - bounds.left;
    assert.ok(applied.width <= visibleWidth && applied.width >= visibleWidth - 2, `applied width ${applied.width} in a ${visibleWidth}px visible window`);
    assert.ok(applied.height <= bounds.height && applied.height >= bounds.height - 100, `applied height ${applied.height}`);
    assert.ok(result.value.bottom <= applied.height + 1, 'footer must fit in the applied content area');
  } finally {
    await chrome?.close();
    if (xvfb) await stopXvfb(xvfb);
    await rm(root, { recursive: true, force: true });
  }
});
