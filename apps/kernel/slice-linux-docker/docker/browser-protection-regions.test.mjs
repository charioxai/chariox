// MP-08/MP-11: pure per-document protection decisions (real-browser placement
// is covered by the opt-in *.browser-test.mjs files).
import test from 'node:test';
import assert from 'node:assert/strict';
import { documentProtection } from './browser-protection-regions.mjs';

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

test('Vault echoes and opaque media are protected only with registered values', () => {
  const doc = snapshot([
    ['#document', -1], ['P', 0], ['#text', 1, [], 'pre vault-value post'], ['INPUT', 0], ['A', 0, ['href', '/x?vault-value']],
    ['CANVAS', 0], ['IMG', 0], ['SVG', 0], ['VIDEO', 0], ['P', 0], ['EMBED', 0], ['OBJECT', 0],
  ], { inputValue: { 3: 'vault-value' } });
  assert.deepEqual(documentProtection(doc, 0).regions, at(10, 11), 'plugins are never inspectable');
  assert.deepEqual(documentProtection(doc, 0, { values: ['vault-value'] }).regions, at(2, 3, 4, 5, 6, 7, 8, 10, 11));
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
