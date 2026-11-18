import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { emptyNote, parsePatch, splitRows } from '../view/diff.mjs';
import { avatarColor, labelColors } from '../view/labels.mjs';
import { parse, parseInline } from '../view/markdown.mjs';
import { ago, clock, duration } from '../view/time.mjs';

const NOW = Date.parse('2026-11-15T12:00:00Z');

test('times read as GitHub reads them', () => {
  const at = ms => new Date(NOW - ms).toISOString();
  assert.equal(ago(at(20e3), NOW), 'just now');
  assert.equal(ago(at(60e3), NOW), '1 minute ago');
  assert.equal(ago(at(11 * 60e3), NOW), '11 minutes ago');
  assert.equal(ago(at(3 * 3600e3), NOW), '3 hours ago');
  assert.equal(ago(at(30 * 3600e3), NOW), 'yesterday');
  assert.equal(ago(at(5 * 86400e3), NOW), '5 days ago');
  assert.equal(ago('2026-09-03T10:00:00Z', NOW), 'on Sep 3');
  assert.equal(ago('2025-09-03T10:00:00Z', NOW), 'on Sep 3, 2025');
  assert.equal(ago('nonsense', NOW), '');
  assert.equal(duration('2026-11-15T10:00:00Z', '2026-11-15T10:00:05Z'), '5s');
  assert.equal(duration('2026-11-15T10:00:00Z', '2026-11-15T10:02:03Z'), '2m 3s');
  assert.equal(clock(Date.parse('2026-11-15T14:05:00Z')), '14:05');
});

test('a label is text on its own colour, dark or light by lightness, and a bad colour falls back to grey', () => {
  assert.deepEqual(labelColors('d73a4a'), { background: '#d73a4a', color: '#ffffff', border: 'rgba(215, 58, 74, 0.3)' });
  assert.equal(labelColors('fbca04').color, '#1f2328');
  assert.equal(labelColors('nonsense').background, '#ededed');
  assert.equal(avatarColor('octo'), avatarColor('octo'));
  assert.notEqual(avatarColor('octo'), avatarColor('hubot'));
});

test('a patch becomes numbered rows: context on both sides, additions and deletions on one', () => {
  const rows = parsePatch('@@ -3,4 +3,5 @@ function a() {\n keep\n-old one\n-old two\n+new one\n+new two\n+new three\n tail\n\\ No newline at end of file');
  assert.deepEqual(rows.map(r => [r.kind, r.old ?? null, r.new ?? null]), [
    ['hunk', null, null], ['context', 3, 3], ['del', 4, null], ['del', 5, null], ['add', null, 4], ['add', null, 5], ['add', null, 6], ['context', 6, 7], ['note', null, null],
  ]);
  assert.equal(rows[2].text, 'old one');
  assert.equal(rows[8].text, 'No newline at end of file');
  assert.deepEqual(parsePatch(''), []);
});

test('side by side: a run of deletions meets the additions after it, the rest is empty on the other side', () => {
  const split = splitRows(parsePatch('@@ -1,3 +1,4 @@\n a\n-b\n-c\n+B\n+C\n+D\n z'));
  assert.deepEqual(split.map(r => r.kind === 'pair' ? [r.left?.text ?? null, r.right?.text ?? null] : r.kind), [
    'hunk', ['a', 'a'], ['b', 'B'], ['c', 'C'], [null, 'D'], ['z', 'z'],
  ]);
  const onlyAdds = splitRows(parsePatch('@@ -0,0 +1,2 @@\n+x\n+y'));
  assert.deepEqual(onlyAdds.slice(1).map(r => [r.left, r.right.text]), [[null, 'x'], [null, 'y']]);
});

test('a file with no patch says why, as GitHub does', () => {
  assert.equal(emptyNote({ status: 'removed', patch: null, changes: 3 }), 'This file was deleted.');
  assert.equal(emptyNote({ status: 'renamed', patch: null, changes: 0 }), 'File renamed without changes.');
  assert.equal(emptyNote({ status: 'added', patch: null, changes: 0 }), 'Binary file not shown.');
  assert.equal(emptyNote({ status: 'added', patch: null, changes: 42000 }), null);
});

test('markdown from GitHub never becomes HTML: raw tags, scripts and event attributes stay text', () => {
  const blocks = parse('Hello <script>alert(1)</script> <img src=x onerror=alert(1)>\n\n[x](javascript:alert(1)) and **bold** `code` @octo #12\n\n```js\n<b>not bold</b>\n```');
  assert.equal(blocks[0].t, 'paragraph');
  const flat = JSON.stringify(blocks[0]);
  assert.match(flat, /<script>alert\(1\)<\/script>/);
  assert.ok(!flat.includes('"t":"html"'));
  const inline = parseInline('[x](javascript:alert(1))');
  assert.equal(inline[0].t, 'link');
  assert.equal(blocks[2].t, 'fence');
  assert.equal(blocks[2].text, '<b>not bold</b>');
  assert.deepEqual(parseInline('a **b** `c` @octo #12').map(n => n.t), ['text', 'strong', 'text', 'code', 'text', 'mention', 'text', 'ref']);
  assert.deepEqual(parseInline('mail@host and a#1').map(n => n.t), ['text'], 'neither an e-mail address nor a#1 is a mention or a reference');
});

test('markdown blocks: headings, lists, task lists, quotes, tables, rules', () => {
  const blocks = parse('# Title\n\n- [x] done\n- [ ] todo\n\n1. one\n2. two\n\n> quoted\n> more\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n---\n');
  assert.deepEqual(blocks.map(b => b.t), ['heading', 'list', 'list', 'quote', 'table', 'rule']);
  assert.deepEqual(blocks[1].items.map(i => i.checked), [true, false]);
  assert.equal(blocks[2].ordered, true);
  assert.deepEqual(blocks[4].rows.length, 1);
});

test('the window builds as one self-contained view: no external asset, no inline style, only the icons it names', () => {
  const out = mkdtempSync(join(tmpdir(), 'github-view-'));
  try {
    execFileSync(process.execPath, [new URL('../scripts/build-package.mjs', import.meta.url).pathname, join(out, 'app')], { stdio: 'pipe' });
    const view = readFileSync(join(out, 'app/bundle/ui/index.html'), 'utf8');
    assert.ok(view.startsWith('<!doctype html><meta charset="utf-8"><script type="application/octet-stream" id="chariox-app-view">'));
    assert.ok(view.length < 4 * 1024 * 1024, 'a view is at most 16 MiB; this one stays far below');
    const style = readFileSync(join(out, 'app-build/app.css'), 'utf8');
    assert.ok(!/url\((?!["']?(data:|#))/.test(style), 'the stylesheet loads nothing');
    const script = readFileSync(join(out, 'app-build/app.js'), 'utf8');
    assert.ok(!/\binnerHTML\b|\binsertAdjacentHTML\b|\bdocument\.write\b|\beval\(|\bnew Function\b/.test(script.replace(/\/\*[\s\S]*?\*\//g, '')), 'no HTML parsing of publisher text');
    assert.ok(!/setAttribute\(["']style["']/.test(script), 'no inline style attribute (the page CSP forbids it)');
    assert.deepEqual(readdirSync(join(out, 'app/bundle/licenses')).sort(), ['THIRD-PARTY-NOTICES.md', 'primer-css.LICENSE', 'primer-octicons.LICENSE', 'primer-primitives.LICENSE']);
    assert.match(readFileSync(join(out, 'app/bundle/licenses/primer-octicons.LICENSE'), 'utf8'), /MIT License/);
  } finally { rmSync(out, { recursive: true, force: true }); rmSync(`${out}-build`, { recursive: true, force: true }); }
});
