// MP-08/MP-10/MP-11: DOM mirror v2 sanitizer and resource typing (unit level).
import test from 'node:test';
import assert from 'node:assert/strict';
import { sanitizeMirrorCss } from './kernel-browser-mirror2-observer.mjs';
import { mirrorResourceType } from './kernel-browser-mirror2.mjs';

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

test('MP-11: executable CSS is invalidated and Vault values never leave in CSS text', () => {
  const { resource } = keys();
  const out = sanitizeMirrorCss('@import url(a.css);\n.c{background:url(javascript:alert(1))}.d::before{content:"javascript:x"}\n.a{width:expression(alert(1));behavior:url(x.htc);-moz-binding:url(b.xml);} .b::after{content:"hunter2-SECRET"}\n@namespace svg url(http://www.w3.org/2000/svg);\n@namespace x url(https://evil.example/);',
    'https://site.example/', resource, ['hunter2-SECRET']);
  assert.ok(!/@import|expression\s*\(|javascript:|-moz-binding|(^|[;{\s])behavior\s*:|hunter2-SECRET|evil/i.test(out), out);
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
