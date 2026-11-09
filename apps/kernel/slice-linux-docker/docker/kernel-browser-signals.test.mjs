import test from 'node:test';
import assert from 'node:assert/strict';
import { knownDifferences, unexpectedDifferences, unexpectedHeaderDifferences, pageSignalsExpression } from './kernel-browser-signals.mjs';

test('MP-08/10/11 an unlisted difference from stock Chrome is reported, a listed one is not', () => {
  const stock = { 'navigator.webdriver': false, 'navigator.userAgent': 'Chrome', 'screen': '1x1', webgl: 'x', worker: '{"ua":"a","gl":"A"}' };
  const kernel = { ...stock, 'navigator.webdriver': true, screen: '2x2', worker: '{"ua":"a","gl":"B"}' };
  assert.deepEqual(unexpectedDifferences(stock, kernel).map(d => d.key), ['navigator.webdriver']);
  assert.deepEqual(unexpectedDifferences(stock, { ...stock, 'navigator.userAgent': 'HeadlessChrome' }).map(d => d.key), ['navigator.userAgent']);
  assert.deepEqual(unexpectedDifferences(stock, { ...stock, worker: '{"ua":"HeadlessChrome","gl":"A"}' }).map(d => d.key), ['worker'], 'only the worker\'s GL renderer may differ');
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
