// MIRROR-P2-FONTS-SUBSET in real Chromium: the viewer loads the subset and lays the page's text out as the whole face does.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import path from 'node:path';
import { readFileSync } from 'node:fs';
import { FontSubsets } from './kernel-browser-font-subset.mjs';
import { mirror2ObserverExpression } from './kernel-browser-mirror2-observer.mjs';

assert(process.env.MIRROR_LAB_NODE_MODULES && path.isAbsolute(process.env.MIRROR_LAB_NODE_MODULES));
const { chromium } = createRequire(path.join(process.env.MIRROR_LAB_NODE_MODULES, 'x.js'))('playwright-core');
const launch = () => chromium.launch({ executablePath: process.env.MIRROR_LAB_CHROME ?? '/opt/google/chrome/chrome', headless: true });
const whole = readFileSync(new URL('../../../../examples/apps/excalidraw/assets/LiberationSans-Regular.woff2', import.meta.url));
const code = text => [...text].map(char => char.codePointAt(0));

test('MIRROR-P2-FONTS-SUBSET: Chromium loads the subset and measures the page text exactly as the whole face', async () => {
  const text = 'Hello, wörld! Fish & chips — £4.50 “quoted” fi ff AVATAR 0123456789';
  const subset = await new FontSubsets().subset(whole, async () => code(text));
  assert(subset.length < whole.length / 3);
  const browser = await launch();
  try {
    const page = await browser.newPage();
    await page.setContent('<body></body>');
    const widths = await page.evaluate(async ([text, faces]) => {
      const out = {};
      for (const [name, base64] of Object.entries(faces)) {
        const font = new FontFace(name, Uint8Array.from(atob(base64), c => c.charCodeAt(0)).buffer);
        await font.load(); document.fonts.add(font);
        const probe = document.createElement('span'); probe.style.cssText = `font:32px "${name}", monospace;white-space:pre`; probe.textContent = text; document.body.append(probe);
        out[name] = { status: font.status, width: probe.getBoundingClientRect().width, kerned: (() => { probe.textContent = 'AVATAR'; return probe.getBoundingClientRect().width; })() };
        probe.remove();
      }
      return out;
    }, [text, { Whole: whole.toString('base64'), Subset: Buffer.from(subset).toString('base64') }]);
    assert.deepEqual([widths.Whole.status, widths.Subset.status], ['loaded', 'loaded']);
    assert.equal(widths.Subset.width, widths.Whole.width);
    assert.equal(widths.Subset.kerned, widths.Whole.kerned, 'kerning survives');
  } finally { await browser.close(); }
});

test('MIRROR-P2-FONTS-SUBSET: the observer reports the characters a document can show, pseudo-element text and field values included', async () => {
  const browser = await launch();
  try {
    const page = await browser.newPage();
    await page.setContent('<style>i::before{content:"\\f507"} b::after{content:attr(data-x) "\\2192"} script{display:block}</style><p>h\u00e9llo \u0436</p><i></i><b data-x="\u00f1"></b><input value="q\u0153" placeholder="\u00df"><textarea>\u00fc</textarea><script>var hidden = "\u2603"</script>');
    await page.evaluate(mirror2ObserverExpression());
    const codes = await page.evaluate(() => globalThis.__charioxMirror2.get('font-subset-test').textCodePoints());
    const has = char => codes.includes(char.codePointAt(0));
    for (const char of ['h', '\u00e9', '\u0436', '\uf507', '\u00f1', '\u2192', '\u0153', '\u00df', '\u00fc']) assert.equal(has(char), true, char.codePointAt(0).toString(16));
    assert.equal(has('\u2603'), false, 'a script is not text');
  } finally { await browser.close(); }
});
