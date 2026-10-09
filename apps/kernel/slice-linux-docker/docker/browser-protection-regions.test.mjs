// MP-08/MP-11: pure per-document protection decisions (real-browser placement
// is covered by the opt-in *.browser-test.mjs files).
import test from 'node:test';
import assert from 'node:assert/strict';
import { ProtectionGate, documentProtection } from './browser-protection-regions.mjs';

// nodes: [name, parent, attributes, extra]; every node gets a 10x10 box at (i*10, 0).
function snapshot(nodes, { scroll = [0, 0], inputValue = {} } = {}) {
  const strings = [];
  const id = value => { const at = strings.indexOf(value); return at >= 0 ? at : strings.push(value) - 1; };
  const document = {
    scrollOffsetX: scroll[0], scrollOffsetY: scroll[1],
    nodes: {
      nodeName: nodes.map(([name]) => id(name)), parentIndex: nodes.map(([, parent]) => parent),
      backendNodeId: nodes.map((_, i) => 100 + i), nodeValue: nodes.map(([, , , text]) => (text ? id(text) : -1)),
      attributes: nodes.map(([, , attributes = []]) => attributes.map(id)),
      inputValue: { index: Object.keys(inputValue).map(Number), value: Object.values(inputValue).map(id) },
      contentDocumentIndex: { index: nodes.flatMap(([name], i) => (name === 'IFRAME-INPROCESS' ? [i] : [])), value: [1] },
    },
    layout: { nodeIndex: nodes.map((_, i) => i), bounds: nodes.map((_, i) => [i * 10, 0, 10, 10]) },
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

test('Vault echoes are protected only with registered values; media and plugins are not masked whole', () => {
  const doc = snapshot([
    ['#document', -1], ['P', 0], ['#text', 1, [], 'pre vault-value post'], ['INPUT', 0], ['A', 0, ['href', '/x?vault-value']],
    ['CANVAS', 0], ['IMG', 0], ['SVG', 0], ['VIDEO', 0], ['P', 0], ['EMBED', 0], ['OBJECT', 0],
  ], { inputValue: { 3: 'vault-value' } });
  assert.deepEqual(documentProtection(doc, 0).regions, []);
  assert.deepEqual(documentProtection(doc, 0, { values: ['vault-value'] }).regions, at(2, 3, 4));
  // Plugins have no inspectable document: returned as owners to withhold.
  assert.deepEqual(documentProtection(doc, 0).owners.map(owner => [owner.backendNodeId, owner.plugin]), [[110, true], [111, true]]);
});

test('frame owners are returned for mapping unless protected; scroll is removed', () => {
  const doc = snapshot([['#document', -1], ['IFRAME', 0], ['IFRAME-INPROCESS', 0], ['DIV', 0, ['data-chariox-secret', '']], ['IFRAME', 3]], { scroll: [5, 7] });
  const { regions, owners } = documentProtection(doc, 0);
  assert.deepEqual(regions, [[25, -7, 10, 10], [35, -7, 10, 10]]);
  assert.deepEqual(owners, [{ backendNodeId: 101, rect: [5, -7, 10, 10], contentDocument: undefined }, { backendNodeId: 102, rect: [15, -7, 10, 10], contentDocument: 1 }]);
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

// MP-11: only CDP's "no layout box" errors mean no pixels on the search path;
// any other box failure fails the whole measurement closed.
test('MP-11 search path: an unknown box failure fails closed, an unrendered field is skipped', async () => {
  const { measurePageProtection } = await import('./browser-protection-regions.mjs');
  const connection = boxError => ({ async send(method, params) {
    if (method === 'Page.getFrameTree') return { frameTree: { frame: { id: 'f', loaderId: 'l', url: 'https://example.test/' } } };
    if (method === 'Page.createIsolatedWorld') return { executionContextId: 1 };
    if (method === 'Runtime.evaluate') return { result: { value: ['visible', 1, 1280, 800] } };
    if (method === 'Page.getLayoutMetrics') return { cssVisualViewport: { scale: 1, zoom: 1 } };
    if (method === 'Target.getTargets') return { targetInfos: [] };
    if (method === 'DOM.getDocument') return { root: { nodeId: 1 } };
    if (method === 'DOM.performSearch') return { searchId: 's', resultCount: params.query.includes('password') ? 1 : 0 };
    if (method === 'DOM.getSearchResults') return { nodeIds: [2] };
    if (method === 'DOM.describeNode') return { node: { localName: 'input', backendNodeId: 7 } };
    if (method === 'DOM.getBoxModel') throw new Error(boxError);
    return {};
  } });
  const policy = { values: [], targets: [] };
  assert.deepEqual((await measurePageProtection(connection('Could not compute box model.'), 's', 't', policy)).regions, []);
  await assert.rejects(measurePageProtection(connection('Session closed'), 's', 't', policy), /Session closed/);
});
