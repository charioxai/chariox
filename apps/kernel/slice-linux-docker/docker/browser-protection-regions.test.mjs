// MP-08/MP-11: pure per-document protection decisions (real-browser placement
// is covered by the opt-in *.browser-test.mjs files).
import test from 'node:test';
import assert from 'node:assert/strict';
import { ProtectionGate, documentProtection, translatedQuad } from './browser-protection-regions.mjs';

// nodes: [name, parent, attributes, extra]; every node gets a 10x10 box at (i*10, 0).
// extra is a text node value or pseudo-element layout text; rendered: further
// layout entries [node, text] (generated content split by counters).
function snapshot(nodes, { scroll = [0, 0], inputValue = {}, rendered = [] } = {}) {
  const strings = [];
  const id = value => { const at = strings.indexOf(value); return at >= 0 ? at : strings.push(value) - 1; };
  const document = {
    scrollOffsetX: scroll[0], scrollOffsetY: scroll[1],
    nodes: {
      nodeName: nodes.map(([name]) => id(name)), parentIndex: nodes.map(([, parent]) => parent),
      backendNodeId: nodes.map((_, i) => 100 + i), nodeValue: nodes.map(([name, , , text]) => (name === '#text' && text ? id(text) : -1)),
      nodeType: nodes.map(([name]) => (name === '#text' ? 3 : 1)),
      attributes: nodes.map(([, , attributes = []]) => attributes.map(id)),
      inputValue: { index: Object.keys(inputValue).map(Number), value: Object.values(inputValue).map(id) },
      contentDocumentIndex: { index: nodes.flatMap(([name], i) => (name === 'IFRAME-INPROCESS' ? [i] : [])), value: [1] },
    },
    layout: {
      nodeIndex: [...nodes.map((_, i) => i), ...rendered.map(([node]) => node)],
      bounds: [...nodes.map((_, i) => [i * 10, 0, 10, 10]), ...rendered.map((_, k) => [k * 10, 20, 10, 10])],
      text: [...nodes.map(([name, , , text]) => ((name === '#text' || name.startsWith('::')) && text ? id(text) : -1)), ...rendered.map(([, text]) => id(text))],
    },
  };
  for (let i = 0; i < nodes.length; i++) if (nodes[i][0] === 'IFRAME-INPROCESS') document.nodes.nodeName[i] = id('IFRAME');
  return { strings, documents: [document] };
}
const at = (...indexes) => indexes.map(i => [i * 10, 0, 10, 10]);

test('markers, secret fields and policy targets protect themselves and all descendants', () => {
  const doc = snapshot([
    ['#document', -1], ['DIV', 0, ['data-chariox-observation-protected', '']], ['#text', 1, [], 'inherited'],
    ['INPUT', 0, ['type', 'PassWord']], ['INPUT', 0, ['autocomplete', 'cc-number']], ['SPAN', 0, ['data-observation-protected', '']],
    ['SPAN', 0, ['data-chariox-secret', '']], ['P', 0], ['INPUT', 0, ['type', 'text']], ['B', 0],
  ]);
  assert.deepEqual(documentProtection(doc, 0, { targetNodes: new Set([109]) }).regions, at(1, 2, 3, 4, 5, 6, 9));
});

test('Vault echoes and opaque media are protected only with registered values; plugins are owners', () => {
  const doc = snapshot([
    ['#document', -1], ['P', 0], ['#text', 1, [], 'pre vault-value post'], ['INPUT', 0], ['A', 0, ['href', '/x?vault-value']],
    ['CANVAS', 0], ['IMG', 0], ['SVG', 0], ['VIDEO', 0], ['P', 0], ['EMBED', 0], ['OBJECT', 0],
  ], { inputValue: { 3: 'vault-value' } });
  assert.deepEqual(documentProtection(doc, 0).regions, []);
  // A registered value can be drawn into media with no DOM echo left.
  assert.deepEqual(documentProtection(doc, 0, { values: ['vault-value'] }).regions, at(2, 3, 4, 5, 6, 7, 8));
  // Plugins have no inspectable document: returned as owners to withhold.
  assert.deepEqual(documentProtection(doc, 0).owners.map(owner => [owner.backendNodeId, owner.plugin]), [[110, true], [111, true]]);
});

test('rendered layout text is checked for Vault values, joined per element in visual order', () => {
  // Generated content has no DOM value: a ::before split by a counter (node 2),
  // two text nodes (6, 7), and ::marker + text + ::after (snapshot order 9, 10, 11).
  const doc = snapshot([['#document', -1], ['DIV', 0], ['::before', 1, [], 'vault-'], ['P', 0], ['#text', 3, [], 'ordinary'],
    ['P', 0], ['#text', 5, [], 'vault-'], ['#text', 5, [], 'value'], ['LI', 0], ['::marker', 8, [], 'vau'], ['::after', 8, [], 'value'], ['#text', 8, [], 'lt-']],
  { rendered: [[2, 'val'], [2, 'ue']] });
  assert.deepEqual(documentProtection(doc, 0).regions, []);
  assert.deepEqual(documentProtection(doc, 0, { values: ['vault-value'] }).regions, [...at(2), [0, 20, 10, 10], [10, 20, 10, 10], ...at(6, 7, 9, 10, 11)]);
  // Rendered text that cannot be read fails closed while values are registered.
  delete doc.documents[0].layout.text;
  assert.deepEqual(documentProtection(doc, 0).regions, []);
  assert.throws(() => documentProtection(doc, 0, { values: ['vault-value'] }), /layout text/);
});

test('frame owners are returned for mapping unless protected; scroll is removed', () => {
  const doc = snapshot([['#document', -1], ['IFRAME', 0], ['IFRAME-INPROCESS', 0], ['DIV', 0, ['data-chariox-secret', '']], ['IFRAME', 3]], { scroll: [5, 7] });
  const { regions, owners } = documentProtection(doc, 0);
  assert.deepEqual(regions, [[25, -7, 10, 10], [35, -7, 10, 10]]);
  assert.deepEqual(owners, [{ backendNodeId: 101, rect: [5, -7, 10, 10], contentDocument: undefined }, { backendNodeId: 102, rect: [15, -7, 10, 10], contentDocument: 1 }]);
});

test('only a pure translation of the layout box maps frame content', () => {
  const box = [10, 20, 210, 20, 210, 70, 10, 70]; // 200x50 at (10, 20): TL, TR, BR, BL.
  assert.equal(translatedQuad(box, 200, 50), true);
  assert.equal(translatedQuad(box.map(v => v + 0.25), 200, 50), true);
  assert.equal(translatedQuad([210, 20, 10, 20, 10, 70, 210, 70], 200, 50), false, 'scaleX(-1)');
  assert.equal(translatedQuad([10, 70, 210, 70, 210, 20, 10, 20], 200, 50), false, 'scaleY(-1)');
  assert.equal(translatedQuad([210, 70, 10, 70, 10, 20, 210, 20], 200, 50), false, 'rotate(180deg)');
  assert.equal(translatedQuad([60, 20, 60, 70, 10, 70, 10, 20], 50, 50), false, 'rotate(90deg), square');
  assert.equal(translatedQuad([10, 20, 110, 20, 110, 45, 10, 45], 200, 50), false, 'scale(.5)');
  assert.equal(translatedQuad([10, 20, 210, 30, 210, 80, 10, 70], 200, 50), false, 'skewY');
  assert.equal(translatedQuad([10, 20, 210, 20, 210, 70, 10], 200, 50), false, 'malformed');
});

test('malformed snapshots fail closed', () => {
  const unordered = snapshot([['#document', -1], ['DIV', 2], ['DIV', 0]]);
  assert.throws(() => documentProtection(unordered, 0));
  const bounds = snapshot([['#document', -1], ['DIV', 0]]);
  bounds.documents[0].layout.bounds[1] = [0, 0, NaN, 1];
  assert.throws(() => documentProtection(bounds, 0));
});

test('stream gate adopts only stable protection and releases only verified frames', async () => {
  const sequence = ['A', 'A', 'A', 'B', 'B', null, 'B'];
  let clock = 0;
  const gate = new ProtectionGate(async () => { const next = sequence.shift(); if (next === null) throw new Error('unbound'); return { pages: [next] }; });
  const steps = [];
  for (let i = 0; i < 7; i++) steps.push({ ...(await gate.step(() => ++clock)), serial: gate.protectionSerial });
  assert.deepEqual(steps.map(({ verified, changed, serial }) => [verified, changed, serial]), [
    [null, false, 0], // first sight: withheld
    [null, true, 1],  // stable across a presented frame: adopt
    [1, false, 1],    // frames captured with 1 before this step are released
    [null, true, 0],  // changed layout: drop frames, withhold
    [null, true, 2],  // new layout stable: adopt
    [null, true, 0],  // unbound measurement: withhold
    [null, false, 0], // one sighting is not enough
  ]);
  assert.equal(gate.protection, null);
});
