import test from 'node:test';
import assert from 'node:assert/strict';
import { knownDifferences, unexpectedDifferences, unexpectedHeaderDifferences, pageSignalsExpression } from './kernel-browser-signals.mjs';

test('MP-08/10/11 an unlisted difference from stock Chrome is reported, a listed one is not', () => {
  const stock = { 'navigator.webdriver': false, 'navigator.userAgent': 'Chrome', 'screen': '1x1', webgl: 'x', worker: '{"ua":"a","gl":"A"}' };
  const kernel = { ...stock, 'navigator.webdriver': true, screen: '2x2' };
  assert.deepEqual(unexpectedDifferences(stock, kernel).map(d => d.key), ['navigator.webdriver']);
  assert.deepEqual(unexpectedDifferences(stock, { ...stock, 'navigator.userAgent': 'HeadlessChrome' }).map(d => d.key), ['navigator.userAgent']);
  assert.deepEqual(unexpectedDifferences(stock, { ...stock, worker: '{"ua":"HeadlessChrome","gl":"A"}' }).map(d => d.key), ['worker']);
});

test('MP-08/10/11 WebGL differs from stock only where stock Chrome has none (a host without a GPU)', () => {
  assert.deepEqual(unexpectedDifferences({ webgl: 'none' }, { webgl: 'a | b' }), []);
  assert.deepEqual(unexpectedDifferences({ webgl: 'a | b' }, { webgl: 'a | c' }).map(d => d.key), ['webgl'], 'with a GPU the renderer must equal stock');
});

test('MP-08/10/11 identity headers of the last request must equal stock', () => {
  const seen = (ua, extra = {}) => [{ headers: { 'user-agent': ua, ...extra } }];
  assert.deepEqual(unexpectedHeaderDifferences(seen('A'), seen('A')), []);
  assert.deepEqual(unexpectedHeaderDifferences(seen('A'), seen('Headless')).map(d => d.key), ['user-agent']);
  assert.deepEqual(unexpectedHeaderDifferences(seen('A', { 'sec-ch-ua-arch': 'x' }), seen('A'), ['sec-ch-ua-arch']), []);
});

test('MP-08/10/11 every known difference says why, and the probe is one self-contained expression', () => {
  for (const [key, reason] of Object.entries(knownDifferences)) assert.ok(reason.length > 20, `${key} needs a reason`);
  assert.doesNotThrow(() => new Function(`return ${pageSignalsExpression}`));
});

// On a host without a GPU only the worker's WebGL renderer may differ; every other worker field is compared as for the page.
const worker = (extra = {}) => JSON.stringify({ ua: 'Chrome', platform: 'Linux', hc: 4, languages: 'en-US', webdriver: false, brands: 'Chromium,Google Chrome', gl: 'ERR', ...extra });
test('MP-08/10/11 without a GPU the worker may differ in its WebGL renderer only', () => {
  const stock = { webgl: 'none', worker: worker() };
  assert.deepEqual(unexpectedDifferences(stock, { webgl: 'a | b', worker: worker({ gl: 'SwiftShader' }) }), []);
  for (const change of [{ brands: 'Chromium,HeadlessChrome' }, { brands: 'Chromium' }, { ua: 'HeadlessChrome' }, { hc: 8 }, { languages: 'de' }, { webdriver: true }, { platform: 'MacIntel' }]) {
    assert.deepEqual(unexpectedDifferences(stock, { webgl: 'a | b', worker: worker({ gl: 'SwiftShader', ...change }) }).map(d => d.key), ['worker'], `worker ${JSON.stringify(change)} must be reported`);
  }
  assert.deepEqual(unexpectedDifferences(stock, { webgl: 'a | b', worker: 'TIMEOUT' }).map(d => d.key), ['worker'], 'a worker that never answered is not a match');
});

test('MP-08/10/11 with a GPU the worker renderer must equal stock', () => {
  const stock = { webgl: 'NVIDIA | x', worker: worker({ gl: 'NVIDIA' }) };
  assert.deepEqual(unexpectedDifferences(stock, { ...stock }), []);
  assert.deepEqual(unexpectedDifferences(stock, { ...stock, worker: worker({ gl: 'Google SwiftShader' }) }).map(d => d.key), ['worker']);
});
