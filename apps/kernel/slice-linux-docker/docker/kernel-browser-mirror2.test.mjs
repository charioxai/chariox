// MP-08/MP-10/MP-11: DOM mirror v2 sanitizer and resource typing (unit level).
import test from 'node:test';
import assert from 'node:assert/strict';
import { sanitizeMirrorCss } from './kernel-browser-mirror2-observer.mjs';
import { Mirror2, mirrorResourceType, encodeMirrorRecords, dedupeMirrorSheets, regionArea } from './kernel-browser-mirror2.mjs';

const keys = () => { const seen = []; return { seen, resource: (url, base, kind) => { const href = new URL(url, base).href; if (!/^(https?|data):/.test(href)) return null; seen.push([href, kind]); return `r${seen.length}`; } }; };
const onlyKernelUrls = css => { const all = css.match(/url\s*\(/gi)?.length ?? 0, allowed = css.match(/url\("(?:mr:r[0-9]{1,6}|#[\w-]*)"\)/g)?.length ?? 0; return all === allowed; };

test('MP-11: every url() spelling becomes a kernel resource key (custom properties, var() shorthands)', () => {
  // Real shapes from MDN (custom property) and GitHub (var() shorthand) that
  // the first v2 sanitizer left unquoted; the client refused those packets.
  const { seen, resource } = keys();
  const css = [
    '.icon { --baseline-img: url(/static/client/obsolete.103e80a32b90e7f1.svg); }',
    '.box { background: var(--bgColor-default) url(data:image/svg+xml;base64,PHN2Zy8+) no-repeat; }',
    ".a { background-image: url('x.png'); }",
    '.b { background-image: url("y.png"); mask: url( "#clip" ); }',
    '.c { background-image: -webkit-image-set("lo.png" 1x, url(hi.png) 2x); }',
    '.d { background: url(\\68 ttp\\3a //evil.example/x.png); }',
    '@font-face { font-family: X; src: url(f.woff2) format("woff2"), local("X"); }',
    '.e { background: url(); }',
  ].join('\n');
  const out = sanitizeMirrorCss(css, 'https://site.example/css/main.css', resource);
  assert.ok(onlyKernelUrls(out), out);
  assert.ok(!/obsolete|evil|data:|x\.png|hi\.png|lo\.png/.test(out), out);
  assert.ok(out.includes('url("#clip")'));
  assert.deepEqual(seen.map(([, kind]) => kind), ['image', 'image', 'image', 'image', 'image', 'image', 'image', 'font']);
  assert.ok(seen.some(([href]) => href === 'http://evil.example/x.png'), 'escaped URL decoded before resolution');
  assert.ok(seen.some(([href, kind]) => href === 'https://site.example/css/f.woff2' && kind === 'font'));
});

test('MP-11: CSS-escaped fetching function names are decoded before the url() rewrite', () => {
  const { seen, resource } = keys();
  // Custom properties keep raw tokens; `u\\72l(` is a url() token once substituted.
  const out = sanitizeMirrorCss('.a { --x: u\\72l(https://leak.example/p.png); } .b { --y: \\75\\72\\6c (//leak2.example/q.png); } .md\\:flex:not(.x) { display: flex; } .c { --z: im\\61 ge-set("r.png" 1x); }', 'https://site.example/', resource);
  assert.ok(onlyKernelUrls(out), out);
  assert.ok(!/leak|\\7|\\6/.test(out), out);
  assert.ok(out.includes('.md\\:flex:not(.x)'), 'escaped selectors are untouched');
  assert.equal(seen.length, 3);
});

test('MP-11: unparseable url( and @import spellings inside strings are neutralized', () => {
  const { resource } = keys();
  const out = sanitizeMirrorCss('.a::after { content: "see url(" } .b { --x: "@import" } .c { background: url("x.png") }', 'https://site.example/', resource);
  assert.ok(onlyKernelUrls(out), out);
  assert.ok(!/@import/i.test(out), out);
  assert.ok(out.includes('url("mr:r1")'), out);
});

// Owner rule (2026-10-09): no value scanning in page CSS; only Vault-filled
// plain text fields are masked (by node identity, in the observer).
test('MP-11: executable CSS is invalidated; page CSS text is not value-scanned', () => {
  const { resource } = keys();
  const out = sanitizeMirrorCss('@import url(a.css);\n.c{background:url(javascript:alert(1))}.d::before{content:"javascript:x"}\n.a{width:expression(alert(1));behavior:url(x.htc);-moz-binding:url(b.xml);} .b::after{content:"page-text"}\n@namespace svg url(http://www.w3.org/2000/svg);\n@namespace x url(https://evil.example/);',
    'https://site.example/', resource);
  assert.ok(!/@import|expression\s*\(|javascript:|-moz-binding|(^|[;{\s])behavior\s*:|evil/i.test(out), out);
  assert.ok(out.includes('content:"page-text"'), out);
  assert.ok(out.includes('@namespace svg "http://www.w3.org/2000/svg";'));
  assert.ok(onlyKernelUrls(out), out);
});

test('MP-11: resource bytes are typed by content, not by URL or header', () => {
  assert.equal(mirrorResourceType(Buffer.from('<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"/>'), 'image'), 'image/svg+xml');
  assert.equal(mirrorResourceType(Buffer.from('<html><script>alert(1)</script>'), 'image'), null);
  assert.equal(mirrorResourceType(Buffer.from('wOF2' + '\0'.repeat(60)), 'font'), 'font/woff2');
  assert.equal(mirrorResourceType(Buffer.from('wOF2' + '\0'.repeat(60)), 'image'), null);
  assert.equal(mirrorResourceType(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10, ...Array(16).fill(0)]), 'font'), null);
});

test('MP-10: compact wire records keep ids, parents, kinds and fields (fixture shared with the client decoder)', () => {
  const records = [
    { id: 'n1', parent: null, kind: 'document' },
    { id: 'n2', parent: 'n1', kind: 'element', tag: 'html', attrs: { lang: 'en' } },
    { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
    { id: 'n5', parent: 'n3', kind: 'text', text: 'Hello' },
    { id: 'n6', parent: 'n3', kind: 'mask', tag: 'span', size: [10, 20], display: 'inline-block' },
    { id: 'n7', parent: 'n3', kind: 'element', tag: 'svg', ns: 'svg', attrs: { viewBox: '0 0 1 1' } },
    { id: 'n1000000001', parent: 'n3', kind: 'frame', tag: 'iframe' },
  ];
  assert.deepEqual(encodeMirrorRecords(records, null), [[1, 0, 1], [1, 1, 'html', { lang: 'en' }], [1, 1, 'body'], [2, 1, 0, 'Hello'], [1, 2, 4, 0, { size: [10, 20], display: 'inline-block', tag: 'span' }], [1, 3, 'svg', { viewBox: '0 0 1 1' }, { ns: 'svg' }], [999999994, 4, 3, 0, { tag: 'iframe' }]]);
  assert.deepEqual(encodeMirrorRecords([{ id: 'n9', parent: 'n3', kind: 'text', text: 'a' }, { id: 'n10', parent: 'n3', kind: 'element', tag: 'b' }], 'n3'), [[9, 0, 0, 'a'], [1, 0, 'b']]);
  assert.throws(() => encodeMirrorRecords([{ id: 'n9', parent: 'n4', kind: 'text', text: 'a' }], 'n3'), /unordered/);
});

test('MP-10: a repeated large stylesheet travels once per snapshot epoch', () => {
  const big = '.a{color:red}'.repeat(400), sent = new Set();
  const packet = { nodes: [{ id: 'n1', kind: 'element', tag: 'style', css: big }, { id: 'n2', kind: 'element', tag: 'style', css: big }, { id: 'n3', kind: 'shadow', adopted: [big, '.b{}'] }, { id: 'n4', kind: 'element', tag: 'style', css: '.small{}' }] };
  dedupeMirrorSheets(packet, sent);
  const digest = packet.nodes[0].css_ref;
  assert.match(digest, /^[a-f0-9]{24}$/);
  assert.deepEqual(Object.keys(packet.sheets), [digest]);
  assert.equal(packet.nodes[1].css_ref, digest); assert.deepEqual(packet.nodes[2].adopted, [{ ref: digest }, '.b{}']); assert.equal(packet.nodes[3].css, '.small{}');
  const next = { ops: [{ op: 'css', id: 'n5', css: big }] }; dedupeMirrorSheets(next, sent);
  assert.equal(next.sheets, undefined); assert.deepEqual(next.ops[0], { op: 'css', id: 'n5', css_ref: digest });
});

test('MP-10: opaque region area is clipped to the viewport for the 25% handoff budget', () => {
  assert.equal(regionArea([{ box: [0, 0, 640, 400] }]), 256000);
  assert.equal(regionArea([{ box: [-100, 700, 400, 400] }]), 300 * 100);
  assert.ok(regionArea([{ box: [0, 0, 1280, 300] }]) > 0.25 * 1280 * 800);
  assert.ok(regionArea([{ box: [10, 10, 300, 168] }]) < 0.25 * 1280 * 800);
});

test('MP-08/MP-10: a credit replayed after a reconnect gets its original packet, not a new sequence', async () => {
  const mirror = new Mirror2({ host: {} }), stream = { chain: Promise.resolve(), issued: 0 };
  let runs = 0; mirror.packet = async () => ({ sequence: ++stream.issued, run: ++runs });
  const first = await mirror.next(stream, { command_id: 'c1', after_sequence: 3, wait_ms: 0 }, 'scope');
  const replay = await mirror.next(stream, { command_id: 'c1', after_sequence: 3, wait_ms: 0 }, 'scope');
  const other = await mirror.next(stream, { command_id: 'c2', after_sequence: 4, wait_ms: 0 }, 'scope');
  assert.deepEqual([first, replay, other.sequence, runs], [first, first, 2, 2]);
});

test('MP-10: queued credits wait one period each after their predecessor (one heartbeat per wait, capped from arrival)', async () => {
  const mirror = new Mirror2({ host: {} }), stream = { chain: Promise.resolve(), issued: 0 }, waits = [];
  mirror.packet = async (s, command, scope, signal, deadline) => { const start = Date.now(); waits.push(deadline - start); await new Promise(resolve => setTimeout(resolve, Math.max(0, deadline - Date.now()))); return { sequence: ++stream.issued }; };
  const started = Date.now();
  await Promise.all([1, 2, 3, 4].map(n => mirror.next(stream, { command_id: `c${n}`, after_sequence: 1, wait_ms: 100 }, 'scope')));
  assert.ok(waits.slice(0, 3).every(ms => ms >= 90), JSON.stringify(waits));
  assert.ok(waits[3] <= 10, `the fourth credit is bounded at 3 waits from its arrival: ${waits}`);
  assert.ok(Date.now() - started >= 290 && Date.now() - started < 450);
});
