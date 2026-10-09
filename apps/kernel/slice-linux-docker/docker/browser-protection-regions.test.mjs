// MP-08/MP-11: pure per-document protection decisions (real-browser placement
// is covered by the opt-in *.browser-test.mjs files).
import test from 'node:test';
import assert from 'node:assert/strict';
import { ProtectionGate, documentProtection, translatedQuad } from './browser-protection-regions.mjs';
import { RENDER_ORDER_STYLES } from './browser-controller-snapshot.mjs';
import { DEFAULT_RENDER_STYLE } from './browser-protection-fixture.mjs';

// nodes: [name, parent, attributes, extra]; every node gets a 10x10 box at (i*10, 0)
// unless boxes overrides it. extra is a text node value or pseudo-element layout
// text; rendered: further layout entries [node, text] (generated content split
// by counters); css: computed style overrides by node (text nodes inherit).
function snapshot(nodes, { scroll = [0, 0], inputValue = {}, rendered = [], css = {}, boxes = {} } = {}) {
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
      bounds: [...nodes.map((_, i) => boxes[i] ?? [i * 10, 0, 10, 10]), ...rendered.map((_, k) => [k * 10, 20, 10, 10])],
      text: [...nodes.map(([name, , , text]) => ((name === '#text' || name.startsWith('::')) && text ? id(text) : -1)), ...rendered.map(([, text]) => id(text))],
    },
  };
  const style = i => ({ ...DEFAULT_RENDER_STYLE, ...css[nodes[i][0] === '#text' ? nodes[i][1] : i] });
  document.layout.styles = document.layout.nodeIndex.map(i => (nodes[i][0] === '#document' ? [] : RENDER_ORDER_STYLES.map(p => id(style(i)[p]))));
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
  assert.deepEqual(documentProtection(doc, 0, { values: ['vault-value'], viewport: [0, 0, 1100, 750] }).regions, at(2, 3, 4, 5, 6, 7, 8));
  // Plugins have no inspectable document: returned as owners to withhold.
  assert.deepEqual(documentProtection(doc, 0).owners.map(owner => [owner.backendNodeId, owner.plugin]), [[110, true], [111, true]]);
});

test('rendered layout text is checked for Vault values, joined per element in visual order', () => {
  // Generated content has no DOM value: a ::before split by a counter (node 2),
  // two text nodes (6, 7) and ::marker + text + ::after (snapshot order 9, 10, 11) with their containers (5, 8).
  const doc = snapshot([['#document', -1], ['DIV', 0], ['::before', 1, [], 'vault-'], ['P', 0], ['#text', 3, [], 'ordinary'],
    ['P', 0], ['#text', 5, [], 'vault-'], ['#text', 5, [], 'value'], ['LI', 0], ['::marker', 8, [], 'vau'], ['::after', 8, [], 'value'], ['#text', 8, [], 'lt-']],
  { rendered: [[2, 'val'], [2, 'ue']] });
  assert.deepEqual(documentProtection(doc, 0).regions, []);
  assert.deepEqual(documentProtection(doc, 0, { values: ['vault-value'], viewport: [0, 0, 1100, 750] }).regions, [...at(2, 5, 6, 7, 8, 9, 10, 11), [0, 20, 10, 10], [10, 20, 10, 10]]);
  // Rendered text that cannot be read fails closed while values are registered.
  delete doc.documents[0].layout.text;
  assert.deepEqual(documentProtection(doc, 0).regions, []);
  assert.throws(() => documentProtection(doc, 0, { values: ['vault-value'], viewport: [0, 0, 1100, 750] }), /layout text/);
});

test('rendered text is flattened across nested and sibling inline elements; matches map to every piece and their container', () => {
  // DIV: ::before + SPAN(text, B(text)) + ::after; P: two sibling SPANs.
  const doc = snapshot([['#document', -1], ['DIV', 0], ['::before', 1, [], 'va'], ['::after', 1, [], 'ue'], ['SPAN', 1], ['#text', 4, [], 'ult-'],
    ['B', 4], ['#text', 6, [], 'val'], ['P', 0], ['SPAN', 8], ['#text', 9, [], 'vault'], ['SPAN', 8], ['#text', 11, [], '-value']]);
  assert.deepEqual(documentProtection(doc, 0).regions, []);
  assert.deepEqual(documentProtection(doc, 0, { values: ['vault-value'], viewport: [0, 0, 1100, 750] }).regions, at(1, 2, 3, 5, 7, 8, 10, 12));
  // A match whose pieces share no laid-out container cannot be placed: fail closed.
  const roots = snapshot([['#text', -1, [], 'vault-'], ['#text', -1, [], 'value']]);
  assert.throws(() => documentProtection(roots, 0, { values: ['vault-value'], viewport: [0, 0, 1100, 750] }), /rendered text/);
});

// Coordinator rule 2026-10-09: while values are registered, a container whose
// visual order can differ from DOM order is masked whole (never the page),
// unless its rendered boxes prove DOM reading order.
const vault = { values: ['vault-value'], viewport: [0, 0, 1100, 750] };
const inline = { display: 'inline' };
// DIV [0,0,200,20] > SPAN 'value', SPAN 'vault-': rendered 'vault-' first (reversed) or in DOM order.
const reversed = { 1: [0, 0, 200, 20], 2: [50, 0, 40, 20], 3: [50, 0, 40, 20], 4: [0, 0, 50, 20], 5: [0, 0, 50, 20] };
const inOrder = { 1: [0, 0, 200, 20], 2: [0, 0, 40, 20], 3: [0, 0, 40, 20], 4: [40, 0, 50, 20], 5: [40, 0, 50, 20] };
const pair = (css, boxes = reversed, outer = 'DIV') => snapshot([['#document', -1], [outer, 0], ['SPAN', 1], ['#text', 2, [], 'value'], ['SPAN', 1], ['#text', 4, [], 'vault-'], ['P', 0], ['#text', 6, [], 'ordinary']], { css, boxes });
test('MP-08/MP-11 rendered boxes keep scaled small or faint text in the order proof', () => {
  for (const style of [{ 'font-size': '2px', transform: 'matrix(10,0,0,10,0,0)' },
    { opacity: '0.05' }, { visibility: 'hidden' }, { '-webkit-text-fill-color': 'transparent' }]) {
    const doc = pair({ 1: { display: 'flex', 'flex-direction': 'row-reverse' }, 4: style });
    doc.documents[0].textBoxes = { layoutIndex: [3, 5, 7], bounds: [reversed[3], reversed[5], [60, 0, 10, 10]] };
    doc.documents[0].layout.textColorOpacities = doc.documents[0].layout.nodeIndex.map(() => 1);
    assert(documentProtection(doc, 0, vault).regions.some(box => JSON.stringify(box) === JSON.stringify(reversed[1])), JSON.stringify(style));
  }
});

test('MP-08/MP-11 tiny and unknown text boxes stay covered locally', () => {
  for (const box of [[0, 0, 1, 1], [0, 0, 0, 0], undefined]) {
    const doc = pair({ 1: { display: 'flex', 'flex-direction': 'row-reverse' } });
    doc.documents[0].textBoxes = { layoutIndex: [3, 5], bounds: [reversed[3], box] };
    if (!box) doc.documents[0].layout.styles[1] = RENDER_ORDER_STYLES.map(p => {
      if (p.startsWith('overflow-')) { doc.strings.push('hidden'); return doc.strings.length - 1; }
      return doc.documents[0].layout.styles[1][RENDER_ORDER_STYLES.indexOf(p)];
    });
    const regions = documentProtection(doc, 0, vault).regions;
    assert(regions.some(b => JSON.stringify(b) === JSON.stringify(reversed[1])), String(box));
    assert(!regions.some(([x,y,w,h]) => x <= 300 && x+w > 300), 'ordinary distant content stays visible');
  }
});

test('MP-08/MP-11 uncertain containers cover overflowing descendant layout and text boxes, including zero-sized containers', () => {
  for(const box of [[100,0,1,20],[100,0,0,0]]){
    const doc=pair({1:{display:'flex','flex-direction':'row-reverse'}},{1:box,2:[80,0,20,20],3:[80,0,20,20],4:[20,0,60,20],5:[20,0,60,20]});
    doc.documents[0].textBoxes={layoutIndex:[3],bounds:[[80,0,28,20]]};
    assert.deepEqual(documentProtection(doc,0).regions,[]);
    const regions=documentProtection(doc,0,vault).regions;
    for(const x of [20,79,80,99,107])assert(regions.some(([left,top,w,h])=>left<=x&&x<left+w&&top<=5&&5<top+h),`overflow x=${x} container=${box}`);
    assert(!regions.some(([x,y,w,h])=>x<=200&&200<x+w&&y<=5&&5<y+h),'ordinary sibling is outside the mask');
  }
});

test('MP-08/MP-11 unreadable descendant geometry uses the nearest local overflow clip', () => {
  const doc=snapshot([['#document',-1],['DIV',0],['DIV',1],['SPAN',2],['#text',3,[],'value'],['SPAN',2],['#text',5,[],'vault-'],['P',0],['#text',7,[],'ordinary']],{
    css:{1:{'overflow-x':'hidden','overflow-y':'hidden'},2:{display:'flex','flex-direction':'row-reverse'}},
    boxes:{1:[10,0,120,20],2:[100,0,0,0],3:[80,0,20,20],4:[80,0,20,20],5:[20,0,60,20],6:[20,0,60,20]}});
  doc.documents[0].layout.bounds[3]=undefined;
  assert(documentProtection(doc,0,vault).regions.some(box=>JSON.stringify(box)==='[10,0,120,20]'));
  assert(!documentProtection(doc,0,vault).regions.some(([x,y,w,h])=>x<=200&&200<x+w),'does not mask an unrelated sibling');
  // Missing per-line text geometry follows the same bounded fallback.
  doc.documents[0].textBoxes={layoutIndex:[4],bounds:[undefined]};
  assert(documentProtection(doc,0,vault).regions.some(box=>JSON.stringify(box)==='[10,0,120,20]'));
});

test('flex, grid, -webkit-box and table containers whose rendered order differs from DOM order are masked whole', () => {
  for (const [label, css] of [
    ['row-reverse', { 1: { display: 'flex', 'flex-direction': 'row-reverse' } }],
    ['column-reverse', { 1: { display: 'flex', 'flex-direction': 'column-reverse' } }],
    ['wrap-reverse', { 1: { display: 'flex', 'flex-wrap': 'wrap-reverse' } }],
    ['column wrap', { 1: { display: 'flex', 'flex-direction': 'column', 'flex-wrap': 'wrap' } }],
    ['order', { 1: { display: 'flex' }, 2: { order: '2' } }],
    ['inline-flex order', { 1: { display: 'inline-flex' }, 4: { order: '-1' } }],
    ['grid placement', { 1: { display: 'grid' }, 2: { 'grid-column-start': '2' } }],
    ['grid area', { 1: { display: 'grid' }, 4: { 'grid-row-start': 'top', 'grid-row-end': 'top' } }],
    ['grid order', { 1: { display: 'grid' }, 2: { order: '1' } }],
    ['grid column flow', { 1: { display: 'grid', 'grid-auto-flow': 'column' } }],
    ['grid dense flow', { 1: { display: 'grid', 'grid-auto-flow': 'row dense' } }],
    ['-webkit-box reverse', { 1: { display: '-webkit-box', '-webkit-box-direction': 'reverse' } }],
    ['-webkit-box ordinal group', { 1: { display: '-webkit-inline-box' }, 2: { '-webkit-box-ordinal-group': '2' } }],
  ]) {
    assert.deepEqual(documentProtection(pair(css), 0).regions, [], `${label}: ordinary without values`);
    assert.deepEqual(documentProtection(pair(css), 0, vault).regions, [reversed[1]], label);
    // Rendered in DOM reading order (same line left to right, or a later line): certain.
    assert.deepEqual(documentProtection(pair(css, inOrder), 0, vault).regions, [], `${label}, rendered in DOM order`);
    assert.deepEqual(documentProtection(pair(css, { ...reversed, 2: [0, 0, 40, 20], 3: [0, 0, 40, 20], 4: [0, 20, 50, 20], 5: [0, 20, 50, 20] }), 0, vault).regions, [], `${label}, next line`);
  }
  // Without a trigger, rendered positions are not inspected; text inherits its flex container's own order.
  for (const css of [{ 1: { display: 'flex', order: '3' } }, { 1: { display: 'grid' } }, { 1: { display: 'flex', 'flex-direction': 'column' } }]) {
    assert.deepEqual(documentProtection(pair(css), 0, vault).regions, [], JSON.stringify(css));
  }
  // Adjacency is measured between glyphs: reversed items whose boxes abut but whose text is 1em apart are separate.
  const padded = { ...reversed, 3: [70, 0, 20, 20], 5: [0, 0, 30, 20] };
  assert.deepEqual(documentProtection(pair({ 1: { display: 'flex', 'flex-direction': 'row-reverse' } }, padded), 0, vault).regions, []);
  assert.deepEqual(documentProtection(pair({ 1: { display: 'flex', 'flex-direction': 'row-reverse' } }, { ...padded, 3: [45, 0, 20, 20] }), 0, vault).regions, [reversed[1]]);
  // Only Chromium's final zero text opacity excludes a rendered box.
  const transparent = pair({ 1: { display: 'flex', 'flex-direction': 'row-reverse' } });
  transparent.documents[0].layout.textColorOpacities = [1, 1, 1, 1, 0, 0, 1, 1];
  assert.deepEqual(documentProtection(transparent, 0, vault).regions, []);
  // TABLE > [caption or row group with 'value'], TBODY > TR > TD 'vault-', rendered above it or below.
  const table = (first, css, below = true) => snapshot([['#document', -1], ['TABLE', 0], [first, 1], ['#text', 2, [], 'value'], ['TBODY', 1], ['TR', 4], ['TD', 5], ['#text', 6, [], 'vault-']],
    { css: { 1: { display: 'table' }, 4: { display: 'table-row-group' }, 5: { display: 'table-row' }, 6: { display: 'table-cell' }, ...css },
      boxes: { 1: [0, 0, 100, 40], ...Object.fromEntries([2, 3].map(i => [i, [0, below ? 20 : 0, 60, 20]])), ...Object.fromEntries([4, 5, 6, 7].map(i => [i, [0, below ? 0 : 20, 60, 20]])) } });
  assert.deepEqual(documentProtection(table('CAPTION', { 2: { display: 'table-caption', 'caption-side': 'bottom' } }), 0, vault).regions, [[0, 0, 100, 40]], 'bottom caption');
  assert.deepEqual(documentProtection(table('TFOOT', { 2: { display: 'table-footer-group' } }), 0, vault).regions, [[0, 0, 100, 40]], 'footer before body');
  assert.deepEqual(documentProtection(table('CAPTION', { 2: { display: 'table-caption' } }, false), 0, vault).regions, [], 'top caption first');
  assert.deepEqual(documentProtection(table('THEAD', { 2: { display: 'table-header-group' } }, false), 0, vault).regions, [], 'header first');
});

test('bidi overrides, direction changes and right-to-left or bidi-control text mask their block container', () => {
  const line = (second, css) => snapshot([['#document', -1], ['P', 0], ['#text', 1, [], 'vault-'], ['B', 1], ['#text', 3, [], second], ['P', 0], ['#text', 5, [], 'ordinary']], { css: { 3: inline, ...css } });
  for (const [label, second, css] of [
    ['bdo override', 'eulav', { 3: { display: 'inline', direction: 'rtl', 'unicode-bidi': 'isolate-override' } }],
    ['bidi-override', 'eulav', { 3: { display: 'inline', 'unicode-bidi': 'bidi-override' } }],
    ['plaintext', 'eulav', { 3: { display: 'inline', 'unicode-bidi': 'plaintext' } }],
    ['inline direction change', 'eulav', { 3: { display: 'inline', direction: 'rtl', 'unicode-bidi': 'isolate' } }],
    ['RLO control', '\u202eeulav\u202c', {}],
    ['RLM', 'eulav\u200f', {}],
    ['Hebrew text', 'ulav \u05d0', {}],
    ['Arabic text', '\u0627 eulav', {}],
  ]) {
    assert.deepEqual(documentProtection(line(second, css), 0).regions, [], `${label}: ordinary without values`);
    assert.deepEqual(documentProtection(line(second, css), 0, vault).regions.sort((a,b)=>a[0]-b[0]), at(1,2,3,4), label);
  }
  // A block that changes direction is its own container; a right-to-left root differs from the default.
  const block = snapshot([['#document', -1], ['DIV', 0], ['SPAN', 1], ['#text', 2, [], 'value'], ['SPAN', 1], ['#text', 4, [], 'vault-'], ['P', 0], ['#text', 6, [], 'ordinary']],
    { css: { 1: { direction: 'rtl' }, 2: { ...inline, direction: 'rtl' }, 4: { ...inline, direction: 'rtl' } } });
  assert.deepEqual(documentProtection(block, 0, vault).regions.sort((a,b)=>a[0]-b[0]), at(1,2,3,4,5));
  const root = snapshot([['#document', -1], ['HTML', 0], ['BODY', 1], ['#text', 2, [], 'x']], { css: { 1: { direction: 'rtl' }, 2: { direction: 'rtl' } } });
  assert.deepEqual(documentProtection(root, 0, vault).regions.sort((a,b)=>a[0]-b[0]), at(1,2,3));
  assert.deepEqual(documentProtection(line('ordinary', {}), 0, vault).regions, []);
});

test('negative margins that can pull text across its siblings mask the block container', () => {
  const pulled = (outer, css, boxes) => pair(css, boxes, outer);
  assert.deepEqual(documentProtection(pulled('P', { 2: inline, 4: { display: 'inline-block', 'margin-left': '-100px' } }), 0, vault).regions, [reversed[1]], 'inline-level');
  assert.deepEqual(documentProtection(pulled('P', { 2: inline, 4: { ...inline, 'margin-right': '-10px' } }), 0, vault).regions, [reversed[1]], 'inline, half a rendered line height');
  assert.deepEqual(documentProtection(pulled('DIV', { 1: { display: 'flex' }, 4: { 'margin-left': '-100px' } }), 0, vault).regions, [reversed[1]], 'flex item');
  assert.deepEqual(documentProtection(pulled('DIV', { 4: { 'margin-top': '-20px' } }), 0, vault).regions, [reversed[1]], 'vertical');
  assert.deepEqual(documentProtection(pulled('DIV', { 2: { 'margin-bottom': '-16px' } }), 0, vault).regions, [reversed[1]], 'vertical, previous sibling');
  // Shifts add up per container; below half a rendered line height in total they cannot reorder glyphs.
  assert.deepEqual(documentProtection(pulled('P', { 2: { ...inline, 'margin-left': '-5px' }, 4: { ...inline, 'margin-left': '-5px' } }), 0, vault).regions, [reversed[1]], 'summed');
  assert.deepEqual(documentProtection(pulled('P', { 2: inline, 4: { ...inline, 'margin-right': '-7px' } }), 0, vault).regions, [], 'below half a rendered line height');
  assert.deepEqual(documentProtection(pulled('DIV', { 4: { 'margin-left': '-15px', 'margin-right': '-15px' } }), 0, vault).regions, [], 'block rows stay certain');
  // Pulled, yet still rendered in DOM reading order.
  assert.deepEqual(documentProtection(pulled('DIV', { 4: { 'margin-top': '-20px' } }, inOrder), 0, vault).regions, [], 'in DOM order');
  // A style clip cannot prove invisibility. Chromium must report absent opacity.
  const sr = snapshot([['#document', -1], ['BODY', 0], ['A', 1], ['#text', 2, [], 'Jump to content'], ['H1', 1], ['#text', 4, [], 'Title']],
    { css: { 2: { position: 'absolute', 'overflow-x': 'hidden', 'overflow-y': 'hidden', 'margin-top': '-1px', 'margin-right': '-1px', 'margin-bottom': '-1px', 'margin-left': '-1px' } },
      boxes: { 1: [0, 0, 900, 600], 2: [10, 40, 1, 1], 3: [10, 40, 120, 20], 4: [10, 40, 400, 40], 5: [10, 40, 100, 40] } });
  assert(documentProtection(sr, 0, vault).regions.length > 0, 'unproven clipping stays covered');
  sr.documents[0].layout.textColorOpacities = [1, 1, 1, 0, 1, 1];
  assert.deepEqual(documentProtection(sr, 0, vault).regions, []);
  // The same with a legacy clip rect instead of overflow; an auto clip shows the text.
  const rect = clip => snapshot([['#document', -1], ['BODY', 0], ['A', 1], ['#text', 2, [], 'Jump to content'], ['H1', 1], ['#text', 4, [], 'Title']],
    { css: { 2: { position: 'absolute', clip } }, boxes: { 1: [0, 0, 900, 600], 2: [10, 40, 120, 20], 3: [10, 40, 120, 20], 4: [10, 40, 400, 40], 5: [10, 40, 100, 40] } });
  assert.deepEqual(documentProtection(rect('rect(1px, 1px, 1px, 1px)'), 0, vault).regions, [[10, 40, 120, 20], [10, 40, 400, 40]], 'unproven legacy clipping stays covered');
  assert.deepEqual(documentProtection(rect('rect(0px, auto, auto, 0px)'), 0, vault).regions, [[10, 40, 120, 20], [10, 40, 400, 40]]);
});

test('displaced text within a rendered line height of other text masks both block containers; text it separates in DOM order still matches', () => {
  // P [0,0,200,20] > SPAN (displaced) 'value' at [60,0,40,20], SPAN 'vault-' at [0,0,50,20]; far P [0,500,200,20].
  const boxes = { 1: [0, 0, 200, 20], 2: [60, 0, 40, 20], 3: [60, 0, 40, 20], 4: [0, 0, 50, 20], 5: [0, 0, 50, 20], 6: [0, 500, 200, 20], 7: [0, 500, 60, 20] };
  const moved = (css, at = boxes) => snapshot([['#document', -1], ['P', 0], ['SPAN', 1], ['#text', 2, [], 'value'], ['SPAN', 1], ['#text', 4, [], 'vault-'], ['P', 0], ['#text', 6, [], 'ordinary']],
    { css: { 4: inline, 2: css }, boxes: at });
  for (const [label, css] of [
    ['absolute', { position: 'absolute' }], ['fixed', { position: 'fixed' }], ['sticky', { position: 'sticky', top: '0px' }],
    ['relative offset', { ...inline, position: 'relative', left: '60px' }], ['float', { float: 'right' }],
    ['transform', { ...inline, transform: 'matrix(1, 0, 0, 1, 60, 0)' }], ['translate', { ...inline, translate: '60px' }],
    ['rotate', { ...inline, rotate: '180deg' }], ['scale', { ...inline, scale: '-1 1' }], ['offset-path', { ...inline, 'offset-path': 'path("M 0 0 H 60")' }],
  ]) {
    assert.deepEqual(documentProtection(moved(css), 0).regions, [], `${label}: ordinary without values`);
    assert.deepEqual(documentProtection(moved(css), 0, vault).regions, css.display === 'inline' ? [boxes[1]] : [boxes[1], boxes[2]], label);
  }
  // Rendered line heights set the gap even if the computed font size is tiny.
  const gap = gapPx => ({ ...boxes, 2: [50 + gapPx, 0, 40, 20], 3: [50 + gapPx, 0, 40, 20] });
  assert.deepEqual(documentProtection(moved({ position: 'absolute' }, gap(15)), 0, vault).regions, [boxes[1], gap(15)[2]], '15px gap');
  assert.deepEqual(documentProtection(moved({ position: 'absolute', 'font-size': '4px' }, gap(9)), 0, vault).regions, [boxes[1], gap(9)[2]], 'rendered height sets the gap');
  assert.deepEqual(documentProtection(moved({ position: 'absolute', 'font-size': '4px' }, gap(15)), 0, vault).regions, [boxes[1], gap(15)[2]], 'rendered height sets the gap');
  assert.deepEqual(documentProtection(moved({ position: 'absolute' }, gap(21)), 0, vault).regions, [], '21px gap');
  assert.deepEqual(documentProtection(moved({ ...inline, position: 'relative', top: '0px', left: 'auto' }), 0, vault).regions, [], 'relative without offset');
  // Style values alone never hide rendered boxes; only final opacity does.
  for (const css of [{ visibility: 'hidden' }, { opacity: '0' }, { '-webkit-text-fill-color': 'rgba(0, 0, 0, 0)' }]) {
    const doc = moved({ position: 'absolute', ...css });
    assert.deepEqual(documentProtection(doc, 0, vault).regions, [boxes[1], boxes[2]], JSON.stringify(css));
    doc.documents[0].layout.textColorOpacities = [1, 1, 0, 0, 1, 1, 1, 1];
    assert.deepEqual(documentProtection(doc, 0, vault).regions, [], 'Chromium reports fully transparent text');
  }
  // A CSS clip box alone is not proof that transformed text was clipped away.
  const clipped = (css, box2) => moved({ position: 'absolute', 'overflow-x': 'hidden', 'overflow-y': 'hidden', ...css }, { ...boxes, 2: box2 });
  assert(documentProtection(clipped({}, [60, 0, 0, 20]), 0, vault).regions.some(box => JSON.stringify(box) === JSON.stringify(boxes[1])), 'unproven zero-width clip remains covered');
  assert.deepEqual(documentProtection(clipped({}, [60, 0, 40, 20]), 0, vault).regions, [boxes[1], boxes[2]], 'clip shows the text');
  // P > 'vault-', SPAN (absolute, far away) 'X', 'value': the visible run spells the value.
  const split = css => snapshot([['#document', -1], ['P', 0], ['#text', 1, [], 'vault-'], ['SPAN', 1], ['#text', 3, [], 'X'], ['#text', 1, [], 'value']],
    { css: { 3: css }, boxes: { 1: [0, 0, 200, 20], 2: [0, 0, 50, 20], 3: [0, 900, 10, 20], 4: [0, 900, 10, 20], 5: [50, 0, 40, 20] } });
  for (const [label, css] of [['absolute', { position: 'absolute' }], ['float', { float: 'left' }], ['transform', { ...inline, transform: 'matrix(1, 0, 0, 1, 0, 900)' }],
    ['clipped', { display: 'inline-block', 'overflow-x': 'hidden', 'overflow-y': 'hidden' }], ['clip-path', { ...inline, 'clip-path': 'inset(50%)' }]]) {
    assert.deepEqual(documentProtection(split(css), 0).regions, [], `${label}: ordinary without values`);
    assert.deepEqual(documentProtection(split(css), 0, vault).regions, [[0, 0, 200, 20], [0, 0, 50, 20], [50, 0, 40, 20]], label);
  }
  const invisible = split(inline);
  invisible.documents[0].layout.textColorOpacities = [1, 1, 1, 1, 0, 1];
  assert.deepEqual(documentProtection(invisible, 0, vault).regions, [[0, 0, 200, 20], [0, 0, 50, 20], [50, 0, 40, 20]], 'final zero opacity removes the separator');
  assert.deepEqual(documentProtection(split(inline), 0, vault).regions, [], 'a visible separator breaks the value');
});

test('render-order styles are required while values are registered', () => {
  const doc = snapshot([['#document', -1], ['P', 0], ['#text', 1, [], 'ordinary']]);
  delete doc.documents[0].layout.styles;
  assert.deepEqual(documentProtection(doc, 0).regions, []);
  assert.throws(() => documentProtection(doc, 0, vault), /render-order styles/);
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

// MP-11: only CDP's "no layout box" errors mean no pixels on the search path;
// any other box failure fails the whole measurement closed.
test('MP-11 search path: box failures use a trusted snapshot or fail closed', async () => {
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
    if (method === 'DOMSnapshot.captureSnapshot') {
      if (boxError === 'Session closed') throw new Error(boxError);
      return {strings: [],documents: [{nodes: {},layout: {nodeIndex: [],bounds: []}}]};
    }
    return {};
  } });
  const policy = { values: [], targets: [] };
  assert.deepEqual((await measurePageProtection(connection('Could not compute box model.'), 's', 't', policy)).regions, []);
  await assert.rejects(measurePageProtection(connection('Session closed'), 's', 't', policy), /Session closed/);
});
