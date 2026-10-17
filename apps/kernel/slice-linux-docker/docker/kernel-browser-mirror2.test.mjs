// MP-08/MP-10/MP-11: DOM mirror v2 sanitizer and resource typing (unit level).
import test from 'node:test';
import assert from 'node:assert/strict';
import { sanitizeMirrorCss } from './kernel-browser-mirror2-observer.mjs';
import { inflateRawSync, constants as zlib } from 'node:zlib';
import { Mirror2, mirrorResourceType, encodeMirrorRecords, dedupeMirrorSheets, regionArea, regionOverBudget, cssImports } from './kernel-browser-mirror2.mjs';

const keys = () => { const seen = []; return { seen, resource: (url, base, kind) => { const href = new URL(url, base).href; if (!/^(https?|data):/.test(href)) return null; seen.push([href, kind]); return `r${seen.length}`; } }; };
const onlyKernelUrls = css => { const all = css.match(/url\s*\(/gi)?.length ?? 0, allowed = css.match(/url\("(?:mr:r[0-9]{1,6}|#[\w-]*)"\)/g)?.length ?? 0; return all === allowed; };

test('MP-08/MP-10/MP-11 document font operations require viewer negotiation',()=>{
 const mirror=new Mirror2({host:{}}),ops=[{op:'fonts',id:'n1',required:[]},{op:'text',id:'n2',text:'paint'}];
 assert.deepEqual(mirror.viewerOps({},ops),[ops[1]]);
 assert.deepEqual(mirror.viewerOps({fontRequirements:false},ops),[ops[1]]);
 assert.equal(mirror.viewerOps({fontRequirements:true},ops),ops);
});

test('MP-08/MP-10/MP-11 native font negotiation is acknowledged only on an opted-in v2 stream',async()=>{
 const {MirrorService}=await import('./kernel-browser-mirror.mjs');
 const connection={send:async method=>{assert.equal(method,'Page.getFrameTree');return {frameTree:{frame:{id:'T',loaderId:'D'}}};}};
 const host={generation:1,scales:new Map(),viewport:()=>({device_scale_factor:1}),displayTarget:async()=>({tab_id:'t',target_id:'T',document_id:'D'}),browser:{resolvePageTarget:async()=>({connection,sessionId:'s'}),inputCapture:{hold:async()=>async()=>{}}}};
 const service=new MirrorService(host);
 try {
  for(const requested of [undefined,true]){
   const reply=await service.subscribe({wire:2,device_scale_factor:1,...(requested?{font_requirements:true}:{})},'owner');
   assert.equal(reply.font_requirements,requested);
   assert.equal(service.streams.get(reply.subscription_id).fontRequirements,requested===true);
  }
  for(const command of [{wire:2,font_requirements:false},{wire:2,font_requirements:'true'},{wire:1,font_requirements:true}])await assert.rejects(service.subscribe({...command,device_scale_factor:1},'owner'),/mirror font negotiation/);
 } finally {service.clear();clearInterval(service.expiry);}
});

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
    { id: 'n1000000002', parent: 'n1000000001', kind: 'document', fragment: 'section' }, // protocol 503
  ];
  assert.deepEqual(encodeMirrorRecords(records, null), [[1, 0, 1], [1, 1, 'html', { lang: 'en' }], [1, 1, 'body'], [2, 1, 0, 'Hello'], [1, 2, 4, 0, { size: [10, 20], display: 'inline-block', tag: 'span' }], [1, 3, 'svg', { viewBox: '0 0 1 1' }, { ns: 'svg' }], [999999994, 4, 3, 0, { tag: 'iframe' }], [1, 1, 1, 0, { fragment: 'section' }]]);
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

test('MP-08/MP-10: an article lead video stays a region of the mirrored page; only a media-dominated viewport hands off', () => {
  // Real BBC article (c32l4ekpvzl8o, 1280x800): the lead video is 721x406 CSS px (29% of the viewport).
  assert.equal(regionOverBudget([{ box: [272, 141, 721, 406] }]), false);
  assert.equal(regionOverBudget([{ box: [272, 141, 721, 406] }, { box: [957, 124, 300, 250] }]), false);
  assert.equal(regionOverBudget([{ box: [0, 0, 1280, 800] }]), true);
  assert.equal(regionOverBudget([{ box: [0, 0, 1280, 480] }, { box: [0, 480, 1280, 40] }]), true);
});

test('MP-08/MP-10: a credit replayed after a reconnect gets its original packet, not a new sequence', async () => {
  const mirror = new Mirror2({ host: {} }), stream = { chain: Promise.resolve(), issued: 0 };
  let runs = 0; mirror.packet = async () => ({ sequence: ++stream.issued, run: ++runs });
  const first = await mirror.next(stream, { command_id: 'c1', after_sequence: 3, wait_ms: 0 }, 'scope');
  const replay = await mirror.next(stream, { command_id: 'c1', after_sequence: 3, wait_ms: 0 }, 'scope');
  const other = await mirror.next(stream, { command_id: 'c2', after_sequence: 4, wait_ms: 0 }, 'scope');
  assert.deepEqual([first, replay, other.sequence, runs], [first, first, 2, 2]);
});

test('BROWSER-UX-05: the first packet is built at registration; the first credit takes it without a second build', async () => {
  const mirror = new Mirror2({ host: {} }), stream = { id: 's1', chain: Promise.resolve(), issued: 0 }, built = [];
  mirror.packet = async (s, command) => { built.push(command.after_sequence); await new Promise(resolve => setTimeout(resolve, 20)); return { sequence: ++stream.issued }; };
  mirror.begin(stream, 'scope', 7);
  await new Promise(resolve => setTimeout(resolve, 5));
  assert.deepEqual(built, [0], 'building started at registration, before any credit');
  const first = await mirror.next(stream, { command_id: 'c1', after_sequence: 0, wait_ms: 0 }, 'scope');
  assert.equal(first.sequence, 1);
  assert.deepEqual(built, [0], 'the credit reused the early packet');
  const second = await mirror.next(stream, { command_id: 'c2', after_sequence: 1, wait_ms: 0 }, 'scope');
  assert.deepEqual([second.sequence, built], [2, [0, 1]]);
});

test('BROWSER-UX-05: a failed early packet leaves the first credit to build its own; a resend is never answered by it', async () => {
  const mirror = new Mirror2({ host: {} }), stream = { id: 's1', chain: Promise.resolve(), issued: 0 };
  let runs = 0; mirror.packet = async () => { if (++runs === 1) throw new Error('early failed'); return { sequence: ++stream.issued, run: runs }; };
  mirror.begin(stream, 'scope', 7);
  await new Promise(resolve => setTimeout(resolve, 5));
  assert.equal((await mirror.next(stream, { command_id: 'c1', after_sequence: 0, wait_ms: 0 }, 'scope')).run, 2);
  const early = new Mirror2({ host: {} }), other = { id: 's2', chain: Promise.resolve(), issued: 0 };
  early.packet = async (s, command) => ({ sequence: ++other.issued, after: command.after_sequence, resend: command.resend });
  early.begin(other, 'scope', 7);
  const resent = await early.next(other, { command_id: 'r1', after_sequence: 2, resend: true, wait_ms: 0 }, 'scope');
  assert.equal(resent.resend, true)
});

test('MP-10: queued credits wait one period each after their predecessor (one heartbeat per wait, capped from arrival)', async () => {
  const mirror = new Mirror2({ host: {} }), stream = { chain: Promise.resolve(), issued: 0 }, waits = [];
  mirror.packet = async (s, command, scope, signal, deadline) => { const start = Date.now(); waits.push(deadline - start); await new Promise(resolve => setTimeout(resolve, Math.max(0, deadline - Date.now()))); return { sequence: ++stream.issued }; };
  const started = Date.now();
  await Promise.all([1, 2, 3, 4].map(n => mirror.next(stream, { command_id: `c${n}`, after_sequence: 1, wait_ms: 100 }, 'scope')));
  assert.ok(waits.slice(0, 3).every(ms => ms >= 90), JSON.stringify(waits));
  assert.ok(waits[3] <= 10, `the fourth credit is bounded at 3 waits from its arrival: ${waits}`);
  assert.ok(Date.now() - started >= 290);
});

test('MP-10: resources travel in 4-aligned slices within the packet budget (64 KB during input, 256 KB when quiet) and reassemble exactly', async () => {
  const png = size => Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), Buffer.alloc(size - 8, 7)]);
  const bodies = { 'https://x/a.png': png(150_000), 'https://x/b.png': png(150_000), 'https://x/big.png': png(400_000) };
  const world = { connection: { send: async (method, params) => method === 'Page.getResourceTree' ? { frameTree: { frame: { id: 'f' }, resources: Object.keys(bodies).map(url => ({ url })) } } : { base64Encoded: true, content: bodies[params.url].toString('base64') } }, sessionId: 's', ref: 'm', frame: { id: 'f' } };
  const mirror = new Mirror2({ host: {} });
  mirror.evaluate = async (w, expression) => expression.includes('nearImages') ? { near: [], all: [] } : expression.includes('loadedCount') ? 3 : ['r1', 'r2', 'r3'];
  const stream = { resources: new Map(), frameSlots: new Map(), movedAt: 0, inputAt: Date.now() };
  mirror.register(stream, { resources: [{ key: 'r1', url: 'https://x/a.png', kind: 'image' }, { key: 'r2', url: 'https://x/b.png', kind: 'image' }, { key: 'r3', url: 'https://x/big.png', kind: 'image' }] });
  const packets = [], whole = new Map();
  for (let i = 0; i < 40 && [...stream.resources.values()].some(entry => !entry.sent); i++) {
    if (i === 3) stream.inputAt = Date.now() - 2000;
    const packet = await mirror.materialize(world, stream, null);
    packets.push({ input: i < 3, bytes: packet.reduce((n, r) => n + r.data_base64.length, 0) });
    for (const r of packet) {
      if (r.total === undefined) { whole.set(r.key, r.data_base64); continue; }
      assert.equal(r.offset % 4, 0); assert.equal(r.offset, (whole.get(r.key) ?? '').length, 'slices arrive in order');
      whole.set(r.key, (whole.get(r.key) ?? '') + r.data_base64);
    }
  }
  assert.ok(packets.filter(p => p.input).every(p => p.bytes <= 64 * 1024 && p.bytes > 0), JSON.stringify(packets));
  assert.ok(packets.every(p => p.bytes <= 256 * 1024), JSON.stringify(packets));
  for (const [key, url] of [['r1', 'https://x/a.png'], ['r2', 'https://x/b.png'], ['r3', 'https://x/big.png']]) assert.equal(whole.get(key), bodies[url].toString('base64'), key);
});

test('MP-10: while the view moves a large near image travels as a preview, even mid-slices; its exact slices continue once settled', async () => {
  const png = Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), Buffer.alloc(400_000 - 8, 7)]);
  const world = { connection: { send: async method => method === 'Page.getResourceTree' ? { frameTree: { frame: { id: 'f' }, resources: [{ url: 'https://x/big.png' }] } } : { base64Encoded: true, content: png.toString('base64') } }, sessionId: 's', ref: 'm', frame: { id: 'f' } };
  const mirror = new Mirror2({ host: {} });
  mirror.evaluate = async (w, expression) => expression.includes('nearImages') ? { near: [['r1', 250, 300]], all: ['r1'] } : expression.includes('loadedCount') ? 1 : expression.includes('preview(') ? '<svg xmlns="http://www.w3.org/2000/svg"/>' : ['r1'];
  // Typing (recent input, view still): the first exact slice travels.
  const stream = { resources: new Map(), frameSlots: new Map(), movedAt: 0, inputAt: Date.now(), issued: 0 };
  mirror.register(stream, { resources: [{ key: 'r1', url: 'https://x/big.png', kind: 'image' }] });
  assert.deepEqual((await mirror.materialize(world, stream, null)).map(r => [r.key, r.mime_type, r.offset]), [['r1', 'image/png', 0]]);
  stream.movedAt = Date.now(); stream.issued++;
  assert.deepEqual((await mirror.materialize(world, stream, null)).map(r => [r.key, r.mime_type]), [['r1', 'image/svg+xml']], 'MP-10: preview while moving');
  stream.issued++;
  assert.deepEqual(await mirror.materialize(world, stream, null), [], 'MP-10: no exact bytes while moving');
  stream.movedAt = stream.inputAt = Date.now() - 2000; stream.issued++;
  assert.deepEqual((await mirror.materialize(world, stream, null)).map(r => [r.key, r.mime_type, r.offset > 0]), [['r1', 'image/png', true]], 'MP-10: exact slices continue once quiet');
});

test('MP-10: protocol 489 bodies from 512 bytes deflate in the subscription context; echoes stay plain; a reset starts a fresh context', async () => {
  const mirror = new Mirror2({ host: {} }), stream = {}, rows = Array.from({ length: 8 }, (_, i) => [1, 1, 'a', { class: 'suggestion-link', href: '#' }, { title: `Suggestion ${i}` }]);
  const echo = { wire: 2, sequence: 2, ops: [{ op: 'form', id: 'n9', form: { value: 'a', selection_start: 1, selection_end: 1 } }] };
  assert.deepEqual(await mirror.encode(stream, echo), echo, 'MP-10: an echo travels plain, outside the context');
  const big = sequence => ({ wire: 2, sequence, ops: [{ op: 'children', id: 'n9', children: ['n10'], nodes: rows }], resources: [{ key: 'r1', data_base64: 'AAAA' }] });
  const first = await mirror.encode(stream, big(3)), second = await mirror.encode(stream, big(4));
  assert.deepEqual(Object.keys(first), ['wire', 'sequence', 'encoding', 'packet_bytes', 'packet_base64', 'resources']);
  assert.deepEqual(first.resources, [{ key: 'r1', data_base64: 'AAAA' }]);
  assert.ok(second.packet_base64.length < first.packet_base64.length / 3, `${second.packet_base64.length} vs ${first.packet_base64.length}`);
  // The viewer's in-order inflation of the concatenated flushes reproduces each body.
  const inflated = inflateRawSync(Buffer.concat([Buffer.from(first.packet_base64, 'base64'), Buffer.from(second.packet_base64, 'base64')]), { finishFlush: zlib.Z_SYNC_FLUSH }).toString();
  const { resources: _r, ...body3 } = big(3), { resources: _s, ...body4 } = big(4);
  assert.equal(inflated, JSON.stringify(body3) + JSON.stringify(body4));
  const reset = await mirror.encode(stream, { ...big(5), reset: true, subscription_id: 's' });
  assert.equal(reset.reset, true);
  assert.equal(inflateRawSync(Buffer.from(reset.packet_base64, 'base64'), { finishFlush: zlib.Z_SYNC_FLUSH }).toString(), JSON.stringify({ wire: 2, sequence: 5, ops: big(5).ops, reset: true, subscription_id: 's' }), 'MP-10: a reset packet inflates on its own');
  mirror.dispose({ ...stream, frames: new Map() }); stream.deflate?.close();
});

test('MP-08/MP-10/MP-11 compression spans cover only compressed packets and retain exact Unicode bodies', async () => {
  const samples = [], mirror = new Mirror2({ host: { timing: (stage, started) => samples.push({ stage, started }) } }), stream = {};
  const small = { wire: 2, sequence: 1, ops: [] };
  const body = sequence => ({ wire: 2, sequence, ops: [{ op: 'text', id: 'n2', text: 'HTML λ🙂'.repeat(4096) }] });
  try {
    assert.deepEqual(await mirror.encode(stream, small), small);
    assert.equal(samples.length, 0, 'plain input echoes do not report compression work');
    const packets = [await mirror.encode(stream, body(2)), await mirror.encode(stream, body(3))];
    const decoded = inflateRawSync(Buffer.concat(packets.map(p => Buffer.from(p.packet_base64, 'base64'))), { finishFlush: zlib.Z_SYNC_FLUSH });
    assert.equal(decoded.toString(), JSON.stringify(body(2)) + JSON.stringify(body(3)));
    assert.equal(samples.length, 2);
    for (const sample of samples) { assert.equal(sample.stage, 'mirror2_deflate'); assert(Number.isFinite(sample.started) && sample.started > 0); }
  } finally { stream.deflate?.close(); }
});

test("MP-10: review #941-4 leading @import rules keep their layer/supports/media conditions; later or brace-bearing ones are skipped", () => {
  const text = '@charset "utf-8";/* c */@layer a, b;@import url("../a.css") screen;\n@import "b.css" layer(x) supports(not (display: grid)) print and (min-width: 10px);@import url(c.css) layer;@import url(d.css) screen{} x;.y{}@import url(e.css);';
  assert.deepEqual(cssImports(text), { layers: "@layer a, b;", imports: [
    { url: "../a.css", open: "@media screen{", close: "}" },
    { url: "b.css", open: "@layer x{@supports (not (display: grid)){@media print and (min-width: 10px){", close: "}}}" },
    { url: "c.css", open: "@layer {", close: "}" },
  ] });
  // Review #941-4: only leading layer statements precede imports; Chrome ignores an @import after a later one.
  assert.deepEqual(cssImports('/* @layer z; */@layer a;\n@layer b, c;@import "x.css" layer(c);@layer d;@import "y.css";'), { layers: "@layer a;@layer b, c;", imports: [{ url: "x.css", open: "@layer c{", close: "}" }] });
});

// Audit 2026-10-10 step 5: a child frame's delta is rebased by schema, field by field; an unnamed field is refused.
test('MP-11: every field of a child-frame delta is rebased into the frame slot; unnamed delta, op and record fields are refused', async () => {
  const frames = await import('./kernel-browser-mirror2-frames.mjs');
  const slot = 2, n = local => `n${2e9 + local}`, r = local => `r${2e6 + local}`;
  assert.equal(typeof frames.rebaseDelta, 'function', 'MP-11: a schema-driven delta rebase exists');
  const record = { id: 'n7', parent: 'n6', kind: 'element', tag: 'div', attrs: { class: 'x', style: 'background:url("mr:r3")' }, res: 'r4', form: { value: 'v' }, scroll: [0, 1], state: { modal: true } };
  const delta = { ops: [{ op: 'children', id: 'n6', children: ['n7'], nodes: [record] }, { op: 'attr', id: 'n7', name: 'style', value: 'color:red;background:url("mr:r5")' }, { op: 'text', id: 'n8', text: 't' },
    { op: 'css', id: 'n9', css: 'a{background:url("mr:r6")}' }, { op: 'adopted', id: 'n1', sheets: ['b{background:url("mr:r7")}'] }, { op: 'form', id: 'n7', form: { value: 'w' } }, { op: 'scroll', id: 'n7', scroll: [1, 2] },
    { op: 'size', id: 'n7', size: [3, 4] }, { op: 'res', id: 'n7', res: 'r8' }, { op: 'res', id: 'n7', res: null }, { op: 'state', id: 'n7', state: { popover_open: true } }],
    changed: ['n8'], resources: [{ key: 'r9', url: 'https://x/a.png', kind: 'image' }], sheets: [{ id: 'n9', url: 'https://x/a.css' }], scroll: [0, 5], focused: 'n7',
    selection: { anchor_id: 'n8', anchor_offset: 0, focus_id: 'n8', focus_offset: 1 }, fragment: 'top', revision: 3, snapshot_complete: true, font_failures:['a'.repeat(64)] };
  const out = frames.rebaseDelta(delta, slot);
  assert.equal(out.snapshot_complete, true, 'child completion is a scalar, never a parent completion override');
  assert.deepEqual(out.font_failures,['a'.repeat(64)],'font descriptors are hashes, never rebased as node or resource ids');
  assert.deepEqual(frames.rebaseOp({op:'fonts',id:'n1',required:['a'.repeat(64)]},slot),{op:'fonts',id:n(1),required:['a'.repeat(64)]});
  assert.deepEqual(frames.rebaseOp({op:'fonts',id:'n1',required:null},slot),{op:'fonts',id:n(1),required:null});
  assert.deepEqual(out.ops[0], { op: 'children', id: n(6), children: [n(7)], nodes: [{ ...record, id: n(7), parent: n(6), attrs: { class: 'x', style: `background:url("mr:${r(3)}")` }, res: r(4) }] });
  assert.deepEqual(out.ops.slice(1).map(op => op.id), [n(7), n(8), n(9), n(1), n(7), n(7), n(7), n(7), n(7), n(7)]);
  assert.match(JSON.stringify(out.ops), new RegExp(`mr:${r(5)}.*mr:${r(6)}.*mr:${r(7)}.*"res":"${r(8)}".*"res":null`));
  assert.deepEqual([out.changed, out.resources[0].key, out.resources[0].slot, out.focused, out.selection.anchor_id, out.selection.focus_id, out.sheets, out.scroll], [[n(8)], r(9), slot, n(7), n(8), n(8), delta.sheets, [0, 5]]);
  assert.throws(() => frames.rebaseDelta({ ...delta, target_id: 'n3' }, slot), /unrebased mirror delta field/);
  assert.throws(() => frames.rebaseOp({ op: 'attr', id: 'n3', name: 'x', value: 'y', owner_id: 'n4' }, slot), /unrebased mirror op field/);
  assert.throws(() => frames.rebaseOp({ op: 'focus_ring', id: 'n3' }, slot), /unrebased mirror op/);
  assert.throws(() => frames.rebaseRecord({ ...record, label_for: 'n3' }, slot), /unrebased mirror record field/);
});

test('MP-08: a page without opaque regions (and no attached frame) places no tiles without evaluating in the page', async () => {
  const mirror = new Mirror2({ host: { scales: new Map() } }), calls = [];
  mirror.evaluate = async (world, expression) => { calls.push(expression); return []; };
  mirror.frames = { opaqueBoxes: async () => [] };
  const stream = { frames: new Map(), tileKeys: 'n9', tilesAt: 0 }, world = { ref: 'm' };
  assert.deepEqual(await mirror.tiles(world, { tab_id: 't' }, stream, false, false, true), []);
  assert.deepEqual([calls, stream.tileKeys], [[], ''], 'MP-08: a DOM echo does not wait for an evaluation in the page');
  await mirror.tiles(world, { tab_id: 't' }, stream, false, false, false);
  assert.deepEqual(calls, ['m.opaqueBoxes()'], 'MP-10: opaque regions are still measured for the 25% budget');
  stream.frames.set('n5', {}); calls.length = 0;
  await mirror.tiles(world, { tab_id: 't' }, stream, false, false, true);
  assert.deepEqual(calls, ['m.opaqueBoxes()'], 'MP-10: an attached child frame may hold opaque regions');
});

test('MP-08/MP-11: a DOM echo is fenced by the page session\'s navigation events; any other answer asks the renderer (and a moved document is refused)', async () => {
  const run = async cached => {
    const frameTrees = [], tab = { tab_id: 't', target_id: 'T', document_id: 'D1' }, stream = { tab_id: 't', issued: 1, document_id: 'D1', policy: 'p', fallback: null, header: {}, resources: new Map(), attrSequence: new Map(), ownScrolls: new Map(), frames: new Map(), frameSlots: new Map(), frameSlot: 0, tileKeys: '' };
    const connection = { send: async method => { if (method === 'Page.getFrameTree') { frameTrees.push(method); return { frameTree: { frame: { loaderId: 'D2' } } }; } return {}; } };
    const host = { protection: 'p', generation: 1, scales: new Map(), displayTarget: async () => tab, browser: { documentIdsByTarget: new Map([['T', cached]]) } };
    const mirror = new Mirror2({ host, require: () => stream, assertWebTab: () => {}, streams: new Map([['s', stream]]) });
    mirror.world = async () => ({ connection, sessionId: 'S', ref: 'm' });
    mirror.evaluate = async () => ({ ops: [{ op: 'text', id: 'n5', text: 'menu' }], changed: [], resources: [], sheets: [], scroll: [0, 0], focused: null, selection: null, fragment: null, opaque: false, pressed: 0 });
    mirror.frames = { drain: async () => ({ ops: [], resources: [], changed: [] }), project: (_, source) => source };
    return { frameTrees, packet: await mirror.packet(stream, { subscription_id: 's', generation: 1, after_sequence: 1 }, 'scope').catch(error => error) };
  };
  const fresh = await run('D1');
  assert.deepEqual([fresh.frameTrees, fresh.packet.ops], [[], [{ op: 'text', id: 'n5', text: 'menu' }]], 'MP-08: no renderer round trip ahead of the echo');
  const moved = await run('D2');
  assert.deepEqual([moved.frameTrees.length, moved.packet.code], [0, 'stale_document_reference'], 'MP-11: the committed navigation event refuses before renderer discovery');
  const unknown = await run(undefined);
  assert.deepEqual([unknown.frameTrees.length, unknown.packet.code], [1, 'stale_document_reference']);
});

// MP-08/MP-10/MP-11: the applied snapshot already owns an isolated world. A
// click reuses that document binding, but still queries the document and live
// hit target immediately before dispatch (including after focus emulation).
test('MP-08/MP-10/MP-11: mirror input reuses the snapshot world and keeps the final document/hit fence', async () => {
  const calls = [], policy = {}, world = { contextId: 7, sessionId: 's', frame: { id: 'f', loaderId: 'doc' }, ref: 'm' };
  let loader = 'doc', resolutions = 0;
  const connection = { isOpen: () => true, send: async method => { calls.push(method); return { frameTree: { frame: { loaderId: loader } } }; } };
  world.connection = connection;
  const tab = { tab_id: 't', target_id: 'target', document_id: 'doc' };
  const stream = { id: 's', world, tab_id: 't', document_id: 'doc', policy, issued: 1, resetAt: 1, attrSequence: new Map(), frameSlots: new Map() };
  const host = { generation: 1, protection: policy, browser: { connection } };
  const mirror = new Mirror2({ host, assertWebTab() {}, require: () => stream });
  mirror.world = async () => { resolutions++; return world; };
  mirror.evaluate = async (_world, expr) => { calls.push(expr); return expr.includes('.point(') ? { x: 10, y: 20 } : true; };
  const input = { subscription_id: 's', sequence: 1, action: { kind: 'click', node_id: 'n1', x: 1, y: 1 } };
  const resolved = await mirror.resolveInput(stream, tab, input, 'scope');
  assert.equal(resolutions, 0, 'a second target/world lookup is unnecessary before the final live fence');
  calls.length = 0;
  await resolved.guard();
  assert.equal(calls[0], 'Page.getFrameTree'); assert.match(calls[1], /hitCheck/);
  const stale = await mirror.resolveInput(stream, tab, input, 'scope');
  loader = 'new'; calls.length = 0;
  await assert.rejects(stale.guard(), { code: 'stale_document_reference' });
  assert.deepEqual(calls, ['Page.getFrameTree'], 'navigation is refused before the hit check or any physical input');
  loader = 'doc';
  for (const cache of [null, { ...world, frame: { loaderId: 'old' } }, { ...world, connection: {} }]) {
    stream.world = cache;
    const before = resolutions;
    await mirror.resolveInput(stream, tab, input, 'scope');
    assert.equal(resolutions, before + 1, 'a missing, navigated or foreign-connection cache resolves afresh');
  }
});

// MP-08/MP-10/MP-11: the next credit re-arms its long poll without renderer round trips while the page
// session's navigation events show the same document; any other answer resolves the target and world afresh.
test('MP-08/MP-10/MP-11: a packet on an unchanged document reuses its world; a navigated, re-attached or closed session resolves afresh', async () => {
  const run = async ({ document = 'D1', session = 'S', open = true, generation = 1 } = {}) => {
    const calls = [], tab = { tab_id: 't', target_id: 'T', document_id: 'D1' };
    const connection = { isOpen: () => open, send: async method => { calls.push(method); return { frameTree: { frame: { loaderId: 'D1' } } }; } };
    const world = { connection, sessionId: 'S', frame: { loaderId: 'D1' }, ref: 'm' };
    const stream = { tab_id: 't', world, issued: 1, document_id: 'D1', policy: 'p', fallback: null, header: {}, resources: new Map(), attrSequence: new Map(), ownScrolls: new Map(), frames: new Map(), frameSlots: new Map(), frameSlot: 0, tileKeys: '' };
    const host = { protection: 'p', generation: 1, scales: new Map(), tabs: new Map([['t', tab]]), displayTarget: async () => { calls.push('displayTarget'); return tab; },
      browser: { connection, documentIdsByTarget: new Map([['T', document]]), sessionsByTarget: new Map([['T', session]]) } };
    const mirror = new Mirror2({ host, require: () => stream, assertWebTab: () => {}, streams: new Map([['s', stream]]) });
    mirror.world = async () => { calls.push('world'); return world; };
    mirror.evaluate = async (_world, expression) => { calls.push(expression); return { ops: [{ op: 'text', id: 'n5', text: 'menu' }], changed: [], resources: [], sheets: [], scroll: [0, 0], focused: null, selection: null, fragment: null, opaque: false, pressed: 0 }; };
    mirror.frames = { drain: async () => ({ ops: [], resources: [], changed: [] }), project: (_, source) => source };
    await mirror.packet(stream, { subscription_id: 's', generation, after_sequence: 1 }, 'scope').catch(() => {});
    const drain = calls.indexOf('m.drain()'); return drain < 0 ? calls : calls.slice(0, drain);
  };
  assert.deepEqual(await run(), [], 'MP-08: no target or frame round trip before the drain');
  for (const changed of [{ document: 'D2' }, { document: null }, { session: 'S2' }, { open: false }, { generation: 2 }])
    assert.deepEqual(await run(changed), ['displayTarget', 'world'], `MP-11: ${JSON.stringify(changed)} resolves afresh`);
});

// MP-08/MP-10: an echo right after viewer input does not wait for the page to measure opaque regions; the next
// quiet packet measures them (and the 25% budget), even on a page that had none.
test('MP-08/MP-10: opaque regions are measured by the next quiet packet, not by an echo after input', async () => {
  const mirror = new Mirror2({ host: { scales: new Map() } }), calls = [];
  mirror.evaluate = async (_world, expression) => { calls.push(expression); return []; };
  mirror.frames = { opaqueBoxes: async () => [] };
  const stream = { frames: new Map(), tileKeys: '', tilesAt: 0, inputAt: Date.now() }, world = { ref: 'm' };
  assert.deepEqual(await mirror.tiles(world, { tab_id: 't' }, stream, false, false, false), []);
  assert.deepEqual(calls, [], 'MP-08: the echo leaves without an evaluation in the page');
  stream.inputAt = Date.now() - 2000;
  assert.equal(mirror.tilesDue(stream), true, 'MP-10: the skipped measurement is due once input is quiet');
  await mirror.tiles(world, { tab_id: 't' }, stream, false, true, false);
  assert.deepEqual([calls, mirror.tilesDue(stream)], [['m.opaqueBoxes()'], false]);
  stream.inputAt = Date.now(); calls.length = 0;
  await mirror.tiles(world, { tab_id: 't' }, stream, true, true, false);
  assert.deepEqual(calls, ['m.opaqueBoxes()'], 'MP-10: a reset always measures');
});

// MP-08/MP-10: after an empty drain the long poll re-arms with one renderer round trip: the drain reports the main
// document's loaded count (a separate loadedCount() would wait for a busy page's next task first).
test('MP-08/MP-10: an empty drain re-arms the long poll without a separate loaded-count evaluation', async () => {
  const calls = [], tab = { tab_id: 't', target_id: 'T', document_id: 'D1' };
  const connection = { isOpen: () => true, send: async () => ({}) };
  const world = { connection, sessionId: 'S', frame: { loaderId: 'D1' }, ref: 'm' };
  const header = { snapshot_complete: 'false', font_failures:'[]', scroll: '[0,0]', focused: 'null', selection: 'null', fragment: 'null' };
  const stream = { tab_id: 't', world, issued: 1, document_id: 'D1', policy: 'p', fallback: null, header, sentScroll: [0, 0], resources: new Map(), attrSequence: new Map(), ownScrolls: new Map(), frames: new Map(), frameSlots: new Map(), frameSlot: 0, tileKeys: '', movedAt: 0 };
  const host = { protection: 'p', generation: 1, scales: new Map(), tabs: new Map([['t', tab]]), displayTarget: async () => tab,
    browser: { connection, documentIdsByTarget: new Map([['T', 'D1']]), sessionsByTarget: new Map([['T', 'S']]) } };
  const mirror = new Mirror2({ host, require: () => stream, assertWebTab: () => {}, streams: new Map([['s', stream]]) });
  const delta = ops => ({ ops, changed: [], resources: [], sheets: [], scroll: [0, 0], focused: null, selection: null, fragment: null, opaque: false, pressed: 0, loaded: 4 });
  mirror.evaluate = async (_world, expression) => { calls.push(expression.replace(/\(.*$/, '')); return expression.includes('loadedCount') ? 4 : delta(expression.includes('waitDrain') ? [{ op: 'text', id: 'n5', text: 'menu' }] : []); };
  mirror.frames = { drain: async () => ({ ops: [], resources: [], changed: [] }), project: (_, source) => source };
  const packet = await mirror.packet(stream, { subscription_id: 's', generation: 1, after_sequence: 1, wait_ms: 1000 }, 'scope', undefined, Date.now() + 1000);
  assert.deepEqual(packet.ops, [{ op: 'text', id: 'n5', text: 'menu' }]);
  assert.deepEqual(calls, ['m.drain', 'm.waitDrain'], 'MP-08: no loadedCount() between the drain and the long poll');
});

// MP-08/MP-10 (owner A/B 2026-10-14, finding 4/6): a navigated document's first snapshot waits for its first
// contentful paint (bounded); a packet on the same document never waits.
test('MP-08/MP-10: a new document is held until its first contentful paint before the reset snapshot', async () => {
  const run = async (document, after = 1, issued = 1, previous = 'D1', age = 0) => {
    const policy = { targets: [] }, calls = [], tab = { tab_id: 't', target_id: 'T', document_id: document };
    const connection = { isOpen: () => true, send: async () => ({ frameTree: { frame: { loaderId: document } } }) };
    const world = { connection, sessionId: 'S', frame: { loaderId: document }, ref: 'm', navigatedAt: performance.timeOrigin + performance.now() - age };
    const stream = { tab_id: 't', world, issued, document_id: previous, policy, fallback: null, header: {}, resources: new Map(), attrSequence: new Map(), ownScrolls: new Map(), frames: new Map(), frameSlots: new Map(), frameSlot: 0, tileKeys: '' };
    const host = { protection: policy, generation: 1, scales: new Map(), tabs: new Map([['t', tab]]), displayTarget: async () => tab,
      browser: { connection, documentIdsByTarget: new Map([['T', document]]), sessionsByTarget: new Map([['T', 'S']]) } };
    const mirror = new Mirror2({ host, require: () => stream, assertWebTab: () => {}, streams: new Map([['s', stream]]) });
    mirror.world = async () => world;
    mirror.evaluate = async (_world, expression) => { calls.push(expression); if (expression.endsWith('customHosts()')) return false; return { ops: [], changed: [], resources: [], sheets: [], scroll: [0, 0], focused: null, selection: null, fragment: null, opaque: false, pressed: 0, nodes: [], root: 'n1' }; };
    mirror.frames = { attach: async () => ({ records: [], ops: [], opaque: [] }), drain: async () => ({ ops: [], resources: [], changed: [] }), project: (_, source) => source };
    await mirror.packet(stream, { subscription_id: 's', generation: 1, after_sequence: after }, 'scope').catch(() => {});
    return calls;
  };
  const held = calls => calls.some(call => call.includes('first-contentful-paint'));
  const navigated = await run('D2'), hold = navigated.findIndex(call => call.includes('first-contentful-paint')), snapshot = navigated.indexOf('m.snapshot()');
  assert.ok(hold >= 0 && snapshot > hold, 'the hold precedes the new document snapshot');
  assert.equal(held(await run('D1')), false, 'same document: no hold');
  const expired = (await run('D2', 1, 1, 'D1', 3000)).find(call => call.includes('first-contentful-paint'));
  assert.ok(expired.includes('Date.now() + 0;'), 'world/CSS setup consumes the navigation hold budget instead of restarting it');
  // Review #1009 P2: a viewer resynchronizing while the navigation commits still gets the held first paint.
  assert.ok(held(await run('D2', 0)), 'client reset (after_sequence 0) on a new document: held');
  assert.ok(held(await run('D2', 2, 20)), 'credit far behind on a new document: held');
  assert.ok(held(await run('D2', 5)), 'credit ahead on a new document: held');
  assert.equal(held(await run('D2', 0, 0, null)), false, 'a new subscription has no previous page to keep');
});

// MP-08/MP-10: a long poll on a document the Tab has navigated away from ends at once (no 2 s wait on the old page).
test('MP-08/MP-10: a navigation ends a waiting long poll before its deadline', async () => {
  const policy = { targets: [] }, tab = { tab_id: 't', target_id: 'T', document_id: 'D1' };
  const documents = new Map([['T', 'D1']]);
  const connection = { isOpen: () => true, send: async () => ({ frameTree: { frame: { loaderId: documents.get('T') } } }) };
  const world = { connection, sessionId: 'S', frame: { loaderId: 'D1' }, ref: 'm' };
  const stream = { tab_id: 't', world, issued: 1, document_id: 'D1', policy, fallback: null, resources: new Map(), attrSequence: new Map(), ownScrolls: new Map(), frames: new Map(), frameSlots: new Map(), frameSlot: 0, tileKeys: '', hurry: 0, header: { scroll: '[0,0]', focused: 'null', selection: 'null', fragment: 'null' } };
  const host = { protection: policy, generation: 1, scales: new Map(), tabs: new Map([['t', tab]]), displayTarget: async () => tab,
    browser: { connection, documentIdsByTarget: documents, sessionsByTarget: new Map([['T', 'S']]) } };
  const mirror = new Mirror2({ host, require: () => stream, assertWebTab: () => {}, streams: new Map([['s', stream]]) });
  mirror.world = async () => world;
  let waits = 0;
  mirror.evaluate = async (_world, expression) => {
    if (expression.includes('waitDrain')) { waits++; await new Promise(resolve => setTimeout(resolve, 20)); if (waits === 2) documents.set('T', 'D2'); }
    return { ops: [], changed: [], resources: [], sheets: [], scroll: [0, 0], focused: null, selection: null, fragment: null, opaque: false, pressed: 0 };
  };
  mirror.frames = { drain: async () => ({ ops: [], resources: [], changed: [] }), project: (_, source) => source };
  mirror.materialize = async () => [];
  const started = Date.now();
  await mirror.packet(stream, { subscription_id: 's', generation: 1, after_sequence: 1 }, 'scope', undefined, Date.now() + 2000).catch(() => {}); // the moved document then refuses this credit (existing fence); the next one resets
  assert.ok(Date.now() - started < 1000, `the wait ended after the navigation (${Date.now() - started} ms)`);
});

test('MP-10 M2: completed stylesheet bodies do not wait for Page resource-content loading',async()=>{
 const body=Buffer.from('body{color:green}'),calls=[];
 const world={connection:{async send(method){calls.push(method);throw Error('unrelated document resources are still loading')}},sessionId:'S',frame:{id:'F',loaderId:'D'},ref:'observer'};
 const mirror=new Mirror2({host:{browser:{responseBodies:{async read(actual,address,limit){assert.equal(actual,world);assert.equal(address,'https://public.example/style.css');assert.equal(limit,16*1024*1024);return body}}}}});
 mirror.evaluate=async(_world,expression)=>{assert(expression.includes(body.toString()));return body.toString()};
 assert.equal(await mirror.crossOriginSheet(world,'https://public.example/style.css'),body.toString());assert.deepEqual(calls,[]);
});

test('MP-10 M2: first-packet font bytes use the completed response before the Page content gate',async()=>{
 const body=Buffer.alloc(48);body.write('wOF2');const calls=[];
 const world={connection:{async send(method){calls.push(method);throw Error('unrelated document resources are still loading')}},sessionId:'S',frame:{id:'F',loaderId:'D'},ref:'observer'};
 const mirror=new Mirror2({host:{browser:{responseBodies:{async read(actual,address){assert.equal(actual,world);assert.equal(address,'https://public.example/font.woff2');return body}}}}});
 mirror.evaluate=async(_world,expression)=>expression.includes('nearImages')?{near:[],all:[]}:['r1'];const entry={key:'r1',url:'https://public.example/font.woff2',kind:'font',state:'new'};
 const out=await mirror.materializeWorld(world,[entry],0,262144,{issued:0,movedAt:0,loadedCount:1,resources:new Map([['r1',entry]])});assert.equal(out[0].mime_type,'font/woff2');assert.deepEqual(calls,[]);
});

// Review #1016 @f0ccdb5c2 P2 (protocol 530): mirrored undo/redo arrive as named mirror keys and resolve to the
// editor's own chords under the mirror's guard (observed frames, live focus, epoch); other names stay plain keys.
test('MP-08/MP-11 mirrored Ctrl+Z / Ctrl+Shift+Z resolve to modifier chords under the mirror guard', async () => {
  const policy = {}, world = { contextId: 7, sessionId: 's', frame: { id: 'f', loaderId: 'doc' }, ref: 'm' };
  const connection = { isOpen: () => true, send: async () => ({ frameTree: { frame: { loaderId: 'doc' } } }) };
  world.connection = connection;
  const tab = { tab_id: 't', target_id: 'target', document_id: 'doc' };
  const stream = { id: 's', world, tab_id: 't', document_id: 'doc', policy, issued: 1, resetAt: 1, attrSequence: new Map(), frameSlots: new Map(), frames: new Map() };
  const mirror = new Mirror2({ host: { generation: 1, protection: policy, browser: { connection } }, assertWebTab() {}, require: () => stream });
  mirror.world = async () => world;
  for (const [key, expected] of [['Ctrl+Z', { kind: 'key', key: 'z', modifiers: 2 }], ['Ctrl+Shift+Z', { kind: 'key', key: 'z', modifiers: 10 }], ['Tab', { kind: 'key', key: 'Tab' }], ['constructor', { kind: 'key', key: 'constructor' }]]) {
    const resolved = await mirror.resolveInput(stream, tab, { subscription_id: 's', sequence: 1, action: { kind: 'key', key } }, 'scope');
    assert.deepEqual(resolved.input, expected, key);
    assert.equal(resolved.observedFrameInput, true); assert.equal(typeof resolved.guard, 'function'); assert.equal(typeof resolved.frameSensitive, 'function');
  }
});

// MP-08/MP-10/MP-11: fixed timing labels never expose document identity or content.
test('MP-10: mirror world attributes target, observer installation and CSS setup separately', async () => {
  const marks = [], connection = { send: async method => method === 'Page.getFrameTree' ? { frameTree: { frame: { id: 'frame', loaderId: 'doc' } } } : { result: { value: true } } };
  const mirror = new Mirror2({ host: { timing: (stage, at) => marks.push([stage, at]), browser: { resolvePageTarget: async () => ({ connection, sessionId: 'session' }), ensureFocusWorld: async () => ({ contextId: 1 }) } } });
  mirror.watchCss = async () => {};
  await mirror.world({ target_id: 'target', document_id: 'doc' }, { id: 'stream' });
  assert.deepEqual(marks.map(([stage]) => stage), ['mirror2_world_target', 'mirror2_world_install', 'mirror2_world_css']);
  assert.ok(marks.every(([, at]) => Number.isFinite(at)));
});


test('MP-10: an older credit base does not resend a 700 KB packet still in transit', () => {
  const stream = { issued: 2 }, service = { host: {}, streams: new Map([['s', stream]]), require() {} }, mirror = new Mirror2(service);
  mirror.remember(stream, 2, { wire: 2, sequence: 2, packet_base64: 'a'.repeat(700_000) }, false);
  stream.history.get(2).at = Date.now() - 2000;
  const command = { subscription_id: 's', generation: 1, after_sequence: 1 };
  assert.equal(mirror.lost(stream, command, 'scope'), null);
  assert.equal(mirror.lost(stream, { ...command, resend: true }, 'scope', true), stream.history.get(2).wire, 'explicit recovery still returns the original packet');
});


test('MP-10: CSS activation does not block the first mirror world', async () => {
  const mirror = new Mirror2({ host: {} });
  let finish;
  mirror.evaluate = async () => false;
  mirror.css.watch = () => new Promise(resolve => { finish = resolve; });
  const world = { sessionId: 's' };
  try {
    assert.equal(await Promise.race([mirror.watchCss(world, 'f', 'stream').then(() => true), new Promise(resolve => setTimeout(() => resolve(false), 50))]), true);
  } finally { finish(); }
});


test('MP-10: a current event-bound frame avoids duplicate cold target/frame queries', async () => {
 const frame={id:'f',loaderId:'doc'},connection={send:async(method)=>{assert.equal(method,'Runtime.evaluate');return{result:{value:true}}}};
 const browser={currentPageFrame:()=>({connection,sessionId:'s',frame,navigatedAt:1}),resolvePageTarget:async()=>{throw Error('duplicate target lookup')},ensureFocusWorld:async()=>({contextId:1})};
 const mirror=new Mirror2({host:{browser}});mirror.watchCss=async()=>{};
 assert.equal((await mirror.world({target_id:'t',document_id:'doc'},{id:'stream'})).frame,frame);
});

test('MP-11 retained sheet advertisements are bounded before issuing a credit', async () => {
 const mirror=new Mirror2({host:{}}),stream={chain:Promise.resolve(),issued:0};mirror.packet=async()=>({sequence:1});
 for(const known_sheets of [false,['not-a-digest'],Array(129).fill('a'.repeat(24))]) await assert.rejects(mirror.next(stream,{known_sheets},'scope'),/invalid mirror sheet dictionary/);
 assert.equal((await mirror.next(stream,{known_sheets:['a'.repeat(24)]},'scope')).sequence,1);
});

test('MP-10 independent sheet bodies overlap in batches of at most four and retain cascade order', async () => {
 const mirror=new Mirror2({host:{}});let active=0,peak=0;
 mirror.crossOriginSheet=async(_world,url)=>{peak=Math.max(peak,++active);await new Promise(resolve=>setTimeout(resolve,2));active--;return url;};
 const sheets=Array.from({length:10},(_,n)=>({id:`n${n}`,url:`sheet${n}`}));
 const ops=await mirror.sheetOps({},sheets);
 assert.equal(peak,4);assert.deepEqual(ops.map(op=>op.id),sheets.map(sheet=>sheet.id));
});

test('MP-08/MP-10/MP-11 native navigation preview precedes observer work and retires old input epochs', async () => {
 const tab={tab_id:'t',target_id:'T',document_id:'new'},stream={tab_id:'t',scope:'scope',navigationPreview:true,issued:5,document_id:'old',resetAt:1};
 const host={generation:1,scales:new Map(),protection:{},browser:{},mirrorTarget:async()=>tab};
 const mirror=new Mirror2({host,require:()=>stream,streams:new Map([['s',stream]])});
 let retired=0;mirror.regions.retireStream=()=>{retired++};mirror.world=async()=>{throw Error('observer must not delay preview')};
 const packet=await mirror.packet(stream,{subscription_id:'s',generation:1,after_sequence:5},'scope');
 assert.equal(packet.fallback,'navigation_preview');assert.equal(packet.document_id,'new');assert.equal(packet.snapshot_complete,false);
 assert.equal(stream.issued,6);assert.equal(stream.resetAt,7);assert.equal(retired,1);
 assert.equal(stream.document_id,'old','actual snapshot must still reset');
 await assert.rejects(mirror.packet(stream,{subscription_id:'s',generation:1,after_sequence:6},'scope'),/observer must not delay preview/);
});

test('MP-08/MP-10/MP-11 explicit handover flush ends older polls before a fresh drain', async()=>{
 const stream={chain:Promise.resolve(),issued:2},mirror=new Mirror2({host:{}});let hurried=0;
 mirror.hurry=s=>{s.hurry=(s.hurry??0)+1;hurried++};mirror.packet=async(_s,c,_scope,_signal,deadline)=>{assert.equal(c.flush,true);assert(deadline<=Date.now());return{sequence:3}};
 await mirror.next(stream,{subscription_id:'s',generation:1,after_sequence:2,wait_ms:0,flush:true},'scope');assert.equal(hurried,1);
});

test('MP-08/MP-10/MP-11 native preview follows a document transition, not initial attachment',async()=>{
 const tab={tab_id:'t',target_id:'T',document_id:'initial'},stream={tab_id:'t',scope:'scope',navigationPreview:true,initialDocument:'initial',issued:0};
 const mirror=new Mirror2({host:{generation:1,scales:new Map(),protection:{},browser:{},mirrorTarget:async()=>tab},require:()=>stream,streams:new Map([['s',stream]])});
 mirror.world=async()=>{throw Error('initial document uses the ordinary mirror path')};
 await assert.rejects(mirror.packet(stream,{subscription_id:'s',generation:1,after_sequence:0},'scope'),/initial document uses the ordinary mirror path/);
 tab.document_id='typed';mirror.regions.retireStream=()=>{};
 const packet=await mirror.packet(stream,{subscription_id:'s',generation:1,after_sequence:0},'scope');
 assert.equal(packet.fallback,'navigation_preview');assert.equal(packet.document_id,'typed');
});

test('MP-08/MP-10/MP-11 the protected native preview can initialize before observer work',async()=>{
 const tab={tab_id:'t',target_id:'T',document_id:'new'},stream={tab_id:'t',scope:'scope',navigationPreview:true,previewDocument:'new',issued:6,document_id:'old',resetAt:7};
 const displays=new Map(),host={generation:1,scales:new Map(),protection:{},browser:{},displays,chromium:{desktop:{binding:()=>({})}},mirrorTarget:async()=>tab};
 const mirror=new Mirror2({host,require:()=>stream,streams:new Map([['s',stream]])});let ready=false;
 mirror.world=async()=>{assert(ready,'observer must leave startup time for the protected native source');throw Error('observer reached after preview')};
 const pending=mirror.packet(stream,{subscription_id:'s',generation:1,after_sequence:6},'scope');pending.catch(()=>{});
 await new Promise(resolve=>setTimeout(resolve,10));ready=true;displays.set('native',{tab_id:'t',generation:1,observed_by:'scope',expires:Date.now()+60000,regionTarget:{kind:'visible_page'},regionLease:{document_id:'new'},regionSource:{attested:true,valid:()=>true,sample:()=>({})}});
 await assert.rejects(pending,/observer reached after preview/);
});

for(const reply of ['evicted','delta','rejected'])test('MP-08/MP-10/MP-11 navigation during '+reply+' refuses before a snapshot or renderer query',async()=>{
 const policy={targets:[]},tab={tab_id:'t',target_id:'T',document_id:'D1'},documents=new Map([['T','D1']]),calls=[];
 const connection={isOpen:()=>true,send:async method=>{calls.push(method);return{frameTree:{frame:{loaderId:'D2'}}};}};
 const world={connection,sessionId:'S',frame:{loaderId:'D1'},ref:'m'};
 const stream={tab_id:'t',world,issued:1,document_id:'D1',policy,fallback:null,header:{},resources:new Map(),attrSequence:new Map(),ownScrolls:new Map(),frames:new Map(),frameSlots:new Map(),frameSlot:0,tileKeys:''};
 const host={protection:policy,generation:1,scales:new Map(),tabs:new Map([['t',tab]]),displayTarget:async()=>tab,browser:{connection,documentIdsByTarget:documents,sessionsByTarget:new Map([['T','S']])}};
 const mirror=new Mirror2({host,require:()=>stream,streams:new Map([['s',stream]])});
 const delta={ops:[],changed:[],resources:[],sheets:[],scroll:[0,0],focused:null,selection:null,fragment:null,opaque:false,pressed:0,nodes:[],root:'n1'};
 mirror.evaluate=async(_,expression)=>{calls.push(expression);if(expression==='m.drain()'){documents.set('T','D2');if(reply==='rejected')throw Error('context destroyed');return reply==='evicted'?{resync:'evicted'}:delta;}return delta;};
 mirror.frames={attach:async()=>({records:[],ops:[],opaque:[]}),drain:async()=>({ops:[],resources:[],changed:[]}),project:(_,source)=>source};mirror.materialize=async()=>[];mirror.sheetOps=async()=>[];mirror.tiles=async()=>[];
 let refused;try{await mirror.packet(stream,{subscription_id:'s',generation:1,after_sequence:1},'scope');}catch(error){refused=error;}
 assert(!calls.includes('m.snapshot()'),'a reused execution-context id must not resnapshot the new page under the old credit');
 assert(!calls.includes('Page.getFrameTree'),'the ordered navigation event already proves retirement; no renderer query');
 assert.equal(refused?.code,'stale_document_reference');
});
