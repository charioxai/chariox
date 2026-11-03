// VAUDIT S3: real Chromium regression for capture_browser and fill_field,
// supplementary to the live drill. Reveal, password toggle and copy-button
// pages; the captured value and a plain-field fill stay masked in pixels.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { launchChromium } from './browser-protection-fixture.mjs';
import { measureBrowserProtection } from './browser-protection-regions.mjs';
import { captureProtectedPage, decodePng } from './kernel-browser-pixels.mjs';

const secret = 'sk_test_51VaudCapbrowserRevealValue';
// A fictional enrolment QR code (see kernel_browser_qr.rs), drawn with no quiet zone.
const qrPng = 'iVBORw0KGgoAAAANSUhEUgAAAHsAAAB7AQAAAAB8vMLSAAAB00lEQVR42o1Vvc3kIBR8PgIyuwEk2iCjJbsB79KA3RIZbSDRgJ0RIL8bf192l8wK7UorjR7z8wYRsXpm/8SxxiEif+Sfz39/iNfitRvVuhX3UQbipoJBVaxJOlbLQpbs9T08RG91q9VvICHgUpc+PkWvODguIOJA/PdwiqmqT93jJym+K3cxDGpa2tfWXRhIu/uQWOdQZzHfSE25BKa4PfpTFYMIiEl9LNkt6rYOnamLXbbdpT3RzVGWzkBkDU4CRBi7FQmUyClXge8iU1aO/viAgrgtjy3XKQtFvyD2RrPMQQ/KfZm6P+yYBXLJRGWsHdKQk6njyEZB4CB80SdgENSjuGCzZnFiPWJ5UBkzp8pU3lg+sJXLWMr+snrBx2i4jLVTEeOxlDdmyAwz5cEI0bO3s1cOInvwV3QQDTEWCuIg14Q6ithK7DKV5I/iSu5T/PnTgVQsC5Ip+5tP6EyJfGMEYqz+Llhkyv27yGzNndsVyBr3V3itTMWntzApCJJ85pZU1kjSf9vyiS31urx0KtfJQ34CORX0P1mwY3mfMHN38Krc+wIrzWOxnlhqFvINbsHDBCvZKWMO5lZUJeqC5NKOgGoawG6FfMWwMnVBBkKlFPsLQsNA8QWNm7YAAAAASUVORK5CYII=';
const hash = (value) => createHash('sha256').update(value).digest('hex');
const box = 'position:absolute;left:80px;width:420px;height:40px;background:magenta;color:magenta;font:20px monospace;border:0;margin:0';
const html = `<!doctype html><body style="margin:0;background:white">
<code id=key style="${box};top:40px">sk_test_••••••</code>
<button id=reveal style="position:absolute;left:560px;top:40px" onclick="setTimeout(()=>{document.querySelector('#key').textContent='${secret}'},50)">Reveal test key</button>
<input id=hidden type=password value="${secret}" style="${box};top:120px">
<button id=toggle style="position:absolute;left:560px;top:120px" onclick="const f=document.querySelector('#hidden');f.type=f.type==='password'?'text':'password'">Show</button>
<button id=copy style="position:absolute;left:560px;top:200px" onclick="navigator.clipboard.writeText('${secret}').then(()=>{this.textContent='Copied'})">Copy</button>
<input id=env style="${box};top:280px">
<img id=qr alt="" style="position:absolute;left:80px;top:360px;width:150px;height:150px;image-rendering:pixelated" src="data:image/png;base64,${qrPng}">
</body>`;

async function setup(dpr, run) {
  const root = await mkdtemp(path.join(tmpdir(), 'vaudit-capture-'));
  const server = createServer((req, res) => { res.setHeader('content-type', 'text/html'); res.end(html); });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  let chrome;
  try {
    chrome = await launchChromium({ executable: process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, root, dpr });
    const { browser, connection } = chrome, { targetInfos } = await connection.send('Target.getTargets');
    const targetId = targetInfos.find((t) => t.type === 'page').targetId, { sessionId } = await browser.resolvePageTarget(targetId);
    await connection.send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: dpr, mobile: false }, sessionId);
    const url = `http://127.0.0.1:${server.address().port}/`;
    await connection.send('Page.navigate', { url }, sessionId);
    for (let i = 0; i < 100; i++) { const { result } = await connection.send('Runtime.evaluate', { expression: 'document.readyState', returnByValue: true }, sessionId); if (result.value === 'complete') break; await new Promise((r) => setTimeout(r, 30)); }
    const documentId = (await connection.send('Page.getFrameTree', {}, sessionId)).frameTree.frame.loaderId;
    const evaluate = async (expression) => (await connection.send('Runtime.evaluate', { expression, returnByValue: true }, sessionId)).result.value;
    const ref = async (selector) => {
      const { root } = await connection.send('DOM.getDocument', {}, sessionId);
      const { nodeId } = await connection.send('DOM.querySelector', { nodeId: root.nodeId, selector }, sessionId);
      return `backend:${(await connection.send('DOM.describeNode', { nodeId }, sessionId)).node.backendNodeId}`;
    };
    const policy = { unknown: false, values: [], targets: [] };
    // What the kernel registers after a capture or fill (room_secret_observation register_target).
    const protect = (node_ref, value) => { policy.values.push(value); policy.targets.push({ kind: 'browser', target_id: targetId, document_id: documentId, node_ref, value_hash: hash(value) }); };
    const capture = (fields) => browser.captureValue({ target_id: targetId, document_id: documentId, expected_document_url: url, ...fields });
    const pixel = async (x, y) => {
      const data = await captureProtectedPage(browser, { target_id: targetId, document_id: documentId }, policy.values, policy.targets,
        async () => (await connection.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false }, sessionId)).data, dpr, null, () => {});
      const image = decodePng(data, dpr), offset = ((y * dpr) * image.width + x * dpr) * 4;
      return [...image.pixels.subarray(offset, offset + 3)];
    };
    const regions = async () => (await measureBrowserProtection(browser, policy)).pages.find((page) => page.target_id === targetId)?.regions ?? [];
    await run({ browser, evaluate, ref, policy, protect, capture, pixel, regions, targetId, documentId, url });
  } finally {
    try { await chrome?.close(); }
    finally {
      server.closeAllConnections();
      try { await new Promise((r) => server.close(r)); }
      finally { await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 }); }
    }
  }
}

for (const dpr of [1, 2]) {
  test(`VAUDIT S3 DPR${dpr}: reveal-and-read captures page text and masks it in pixels`, () => setup(dpr, async ({ ref, protect, capture, pixel, regions, evaluate }) => {
    const node_ref = await ref('#key');
    assert.equal(await capture({ node_ref, reveal_node_ref: await ref('#reveal') }), secret);
    assert.equal(await evaluate("document.querySelector('#key').textContent"), secret, 'the page itself stays revealed for the owner');
    assert.deepEqual(await pixel(100, 60), [255, 0, 255], 'unregistered text is plain page pixels');
    protect(node_ref, secret);
    assert.ok((await regions()).length > 0, 'a captured text node must contribute a protected region');
    assert.deepEqual(await pixel(100, 60), [0, 0, 0], 'the revealed key is masked in model-bound pixels');
    await evaluate("document.querySelector('#key').textContent='sk_test_••••'");
    assert.equal((await regions()).length, 0, 'a re-hidden value no longer masks its element');
  }));

  test(`VAUDIT S3 DPR${dpr}: password toggle and copy button capture the value`, () => setup(dpr, async ({ ref, protect, capture, pixel, evaluate }) => {
    const hidden = await ref('#hidden');
    assert.equal(await capture({ node_ref: hidden }), secret, 'a password input holds its value without reveal');
    assert.equal(await capture({ node_ref: await ref('#copy'), read: 'clipboard' }), secret);
    for (let i = 0; i < 50 && await evaluate("document.querySelector('#copy').textContent") !== 'Copied'; i++) await new Promise((r) => setTimeout(r, 20));
    assert.equal(await evaluate("document.querySelector('#copy').textContent"), 'Copied', 'the page sees its copy succeed');
    assert.equal(await evaluate('Object.keys(window).filter(k=>k.startsWith("__charioxCapture")).length'), 0, 'the clipboard recorder is removed');
    protect(hidden, secret);
    await evaluate("document.querySelector('#toggle').click()");
    assert.equal(await evaluate("document.querySelector('#hidden').type"), 'text');
    assert.deepEqual(await pixel(100, 140), [0, 0, 0], 'the shown password field stays masked for the model');
    await assert.rejects(capture({ node_ref: await ref('#key'), reveal_node_ref: await ref('#copy'), read: 'clipboard' }), /omit reveal_node_ref/);
  }));

  test(`VAUDIT S3 DPR${dpr}: a QR code image is captured as a picture the kernel decodes`, () => setup(dpr, async ({ ref, capture, policy, browser }) => {
    const data = await capture({ node_ref: await ref('#qr'), read: 'qr' });
    const image = decodePng(data, dpr);
    assert.ok(image.width >= 400 * dpr && image.width === image.height, `scaled up so modules stay legible: ${image.width}`);
    let dark = 0;
    for (let i = 0; i < image.pixels.length; i += 4) if (image.pixels[i] < 64) dark++;
    assert.ok(dark > image.width * image.height * 0.2 && dark < image.width * image.height * 0.7, 'the picture holds the code');
    assert.ok(!browser.protectedValues.has(data), 'a picture of the element is not a protected secret');
    await assert.rejects(capture({ node_ref: await ref('#qr'), read: 'qr', reveal_node_ref: await ref('#reveal') }), /omit reveal_node_ref/);
    assert.equal(policy.values.length, 0);
  }));

  test(`VAUDIT S3 DPR${dpr}: fill_field fills a plain field without changing the page`, () => setup(dpr, async ({ browser, ref, protect, pixel, regions, evaluate, targetId, documentId, url }) => {
    const node_ref = await ref('#env');
    const fill = (extra) => browser.performAction({ target_id: targetId, document_id: documentId, node_ref, action: { kind: 'fill', text: secret, expected_document_url: url, ...extra } });
    await fill({});
    assert.equal(await evaluate("document.querySelector('#env').value"), secret, 'the shared editable-field fill keeps the owner field unchanged');
    await fill({ plain_target: true });
    assert.equal(await evaluate("document.querySelector('#env').value"), secret);
    assert.equal(await evaluate("document.querySelector('#env').type"), 'text', 'the owner sees the field as the page made it');
    protect(node_ref, secret);
    assert.ok((await regions()).length > 0, 'a plain fill passes the protected capture fence');
    assert.deepEqual(await pixel(100, 300), [0, 0, 0], 'the filled plain field is masked for the model');
    const reveal = await ref('#reveal');
    await assert.rejects(browser.performAction({ target_id: targetId, document_id: documentId, node_ref: reveal,
      action: { kind: 'fill', text: secret, expected_document_url: url, plain_target: true }, timeout_ms: 500 }), /not become actionable|editable text field/);
  }));
}
