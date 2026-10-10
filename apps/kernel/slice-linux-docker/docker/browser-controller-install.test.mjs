// MP-08/MP-11: exercise the separately installed slice controller, not host assets.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, copyFile, mkdir, mkdtemp, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { launchChromium } from './browser-protection-fixture.mjs';

const repo = fileURLToPath(new URL('../../../../', import.meta.url));
test('MP-08/MP-11: Docker-installed controller starts and captures a canonical image', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'protxform-installed-'));
  let chrome;
  try {
    const dockerfile = await readFile(new URL('./Dockerfile', import.meta.url), 'utf8');
    // Source COPY declarations only: --from build-stage binaries have no
    // checkout path and are outside this module-closure regression.
    for (const match of dockerfile.matchAll(/^COPY(?: --(?:chown|chmod)=\S+)* (?!-)(.+?) (\/opt\/chariox-slice\/(\S*))$/gm)) {
      const sources = match[1].split(/\s+/);
      const directory = match[2].endsWith('/');
      assert.ok(directory || sources.length === 1, 'Docker COPY destination');
      for (const source of sources) {
        const destination = path.join(root, match[3], directory ? path.basename(source) : '');
        await mkdir(path.dirname(destination), { recursive: true });
        await copyFile(path.join(repo, source), destination);
      }
    }
    const health = spawnSync(process.execPath, [path.join(root, 'browser-controller.mjs'), 'stdio'], {
      input: '{"id":1,"method":"health"}\n', encoding: 'utf8', timeout: 10000,
    });
    assert.equal(health.status, 0, health.stderr);
    assert.equal(JSON.parse(health.stdout).result.state, 'ready');
    chrome = await launchChromium({ executable: process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, root });
    const { browser, connection } = chrome;
    const { targetInfos } = await connection.send('Target.getTargets');
    const targetId = targetInfos.find(t => t.type === 'page').targetId;
    const { sessionId } = await browser.resolvePageTarget(targetId);
    await connection.send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false }, sessionId);
    const documentId = (await connection.send('Page.getFrameTree', {}, sessionId)).frameTree.frame.loaderId;
    const { captureProtectedBrowserImage } = await import(pathToFileURL(path.join(root, 'browser-controller-image.mjs')));
    const image = await captureProtectedBrowserImage({ connection, sessionId, targetId, documentId,
      viewport: { css_width: 1280, css_height: 800, device_scale_factor: 1 }, protectedValues: [] });
    assert.equal(image.redaction, 'none');
    assert.equal(image.bytes.readUInt32BE(16), 1280);
    assert.equal(image.bytes.readUInt32BE(20), 800);
  } finally {
    await chrome?.close();
    await rm(root, { recursive: true, force: true });
  }
});
