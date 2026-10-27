// MP-08/MP-10/MP-11: headed geometry regression, supplementary to public-site Web drills.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { captureProtectedBrowserImage } from './browser-controller-image.mjs';
import { launchChromium } from './browser-protection-fixture.mjs';

test('MP-08/MP-10/MP-11: real headed page footer remains inside the desktop below Chromium chrome', { timeout: 60000 }, async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'headed-viewport-'));
  const xvfb = spawn('Xvfb', ['-displayfd', '3', '-screen', '0', '3840x2160x24', '-nolisten', 'tcp'], { stdio: ['ignore', 'ignore', 'ignore', 'pipe'] });
  let chrome;
  try {
    const display = await new Promise((resolve, reject) => {
      xvfb.once('error', reject);
      xvfb.stdio[3].once('data', value => resolve(':' + value.toString().trim()));
      xvfb.once('exit', code => reject(Error(`Xvfb exited: ${code}`)));
    });
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
      await connection.send('Browser.setWindowBounds', { windowId, bounds: { width: width * dpr, height: height * dpr, windowState: 'normal' } });
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
      await browser.reconcile(source);
      await connection.send('Runtime.evaluate', { expression: `document.body.innerHTML='<main style="height:3000px">Page</main><footer id="bottom">Reachable footer</footer>'; scrollTo(0,document.documentElement.scrollHeight);`, returnByValue: true }, sessionId);
      const { result } = await connection.send('Runtime.evaluate', { expression: '({height:innerHeight,bottom:document.getElementById("bottom").getBoundingClientRect().bottom})', returnByValue: true }, sessionId);
      assert.ok(toolbar + result.value.height * dpr <= height * dpr,
        `layout bottom clipped at ${width}x${height} DPR${dpr}: toolbar=${toolbar}, page=${result.value.height}`);
      assert.ok(toolbar + result.value.bottom * dpr <= height * dpr + 1, 'footer must fit in the visible content area');
      const documentId = browser.documentIdsByTarget.get(targetId);
      await captureProtectedBrowserImage({ connection, sessionId, targetId, documentId, viewport: browser.layoutViewport(targetId, source), protectedValues: new Set() });
    }
  } finally {
    await chrome?.close();
    if (Number.isSafeInteger(xvfb.pid) && xvfb.pid > 1 && xvfb.exitCode === null) {
      xvfb.kill('SIGTERM'); await new Promise(resolve => xvfb.once('exit', resolve));
    }
    await rm(root, { recursive: true, force: true });
  }
});
