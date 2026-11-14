import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { FontSubsets, SUBSET_MIN_BYTES, subsetCodePoints } from './kernel-browser-font-subset.mjs';
import { Mirror2 } from './kernel-browser-mirror2.mjs';

const liberation = readFileSync(new URL('../../../../examples/apps/excalidraw/assets/LiberationSans-Regular.woff2', import.meta.url));
const text = string => [...string].map(char => char.codePointAt(0));
const id = bytes => createHash('sha256').update(bytes).digest('hex');

test('MIRROR-P2-FONTS-SUBSET: the code points kept are ASCII, the page\'s characters, the whole block of a script the page uses, and an ideograph alone', () => {
  const cyrillic = text('абвгдеёжзийклмнопрстуфхцчшщъыьэюя'), strayCyrillic = text('Русская');
  const kept = new Set(subsetCodePoints([...text('ł'), ...cyrillic, ...text('é'), 0x4e2d, 0x4e2e]));
  for (const code of [0x20, 0x7e, 0xa0, 0x2019, 0x20ac, 0x2122]) assert.equal(kept.has(code), true, code.toString(16));
  assert.equal(kept.has(0xe9) && kept.has(0x142), true, 'the characters of the page');
  assert.equal(kept.has(0xe8) || kept.has(0x141), false, 'a single accented letter keeps itself only');
  assert.equal(kept.has(0x0400) && kept.has(0x047f), true, 'a Cyrillic page keeps its Cyrillic block');
  assert.equal(kept.has(0x0480), false, 'and no further block');
  assert.equal(kept.has(0x4e2d) && kept.has(0x4e2e), true);
  assert.equal(kept.has(0x4e2f), false, 'an ideograph keeps itself only');
  const some = new Set(subsetCodePoints([...strayCyrillic, ...text('àâçèéêëîïôùûü')]));
  assert.equal(some.has(0x0430) && some.has(0x0431), false, 'a few Cyrillic letters (a menu entry) keep themselves only');
  assert.equal(some.has(0xc0) && some.has(0xff), true, 'a language with accents keeps the Latin-1 block');
});

test('MIRROR-P2-FONTS-SUBSET: a large face travels as WOFF2 of the glyphs the page shows, once per face and character set', async () => {
  assert.equal(liberation.length >= SUBSET_MIN_BYTES, true);
  const subsets = new FontSubsets(); let asked = 0;
  const first = await subsets.subset(liberation, async () => (asked++, text('Hello world')));
  assert.equal(first.toString('ascii', 0, 4), 'wOF2');
  assert.equal(first.length < liberation.length / 3, true, `${first.length} of ${liberation.length}`);
  assert.equal(await subsets.subset(liberation, async () => text('Another page')), first, 'a later page the subset covers gets the same bytes');
  const cyrillic = await subsets.subset(liberation, async () => text('Привет'));
  assert.equal(cyrillic.length > first.length, true, 'a page with letters the subset lacks gets a new subset');
  assert.equal((await subsets.subset(liberation, async () => text('Hello'))).equals(cyrillic), true, 'which then covers the earlier pages too');
  assert.equal(asked, 1);
  assert.equal(await subsets.subset(Buffer.alloc(SUBSET_MIN_BYTES - 1), async () => assert.fail('a small face does not read the page')), null);
});

test('MIRROR-P2-FONTS-SUBSET: a face the subsetter cannot read stays whole', async () => {
  assert.equal(await new FontSubsets().subset(Buffer.alloc(SUBSET_MIN_BYTES * 2, 7), async () => text('x')), null);
});

test('MIRROR-P2-FONTS-SUBSET: the mirror sends the subset, and names it when the viewer retains it on the next document', async () => {
  const events = [], world = { connection: { async send() { throw Error('unrelated document resources are still loading'); } }, sessionId: 'S', frame: { id: 'F', loaderId: 'D' }, ref: 'observer' };
  const mirror = new Mirror2({ host: { timing: { event: (name, fields) => events.push([name, fields]) }, browser: { responseBodies: { async read() { return liberation; } } } } });
  mirror.evaluate = async (_world, expression) => expression.includes('textCodePoints') ? text('Hello world') : expression.includes('nearImages') ? { near: [], all: [] } : ['r1'];
  const due = () => [{ key: 'r1', url: 'https://public.example/a.woff2', kind: 'font', state: 'new' }];
  const stream = known => ({ issued: 0, movedAt: 0, loadedCount: 1, knownFonts: new Set(known), resources: new Map() });
  const [sent] = await mirror.materializeWorld(world, due(), 0, 262144, stream([]));
  const bytes = Buffer.from(sent.data_base64, 'base64');
  assert.equal(sent.mime_type, 'font/woff2');
  assert.equal(bytes.length < liberation.length / 3, true);
  assert.equal(sent.resource_id, id(bytes));
  assert.deepEqual(events.map(([name, fields]) => [name, fields.whole_bytes, fields.subset_bytes]), [['mirror2_font_subset', liberation.length, bytes.length]]);
  const [named] = await mirror.materializeWorld(world, due(), 0, 262144, stream([sent.resource_id.slice(0, 24)]));
  assert.deepEqual(named, { key: 'r1', resource_id: sent.resource_id, mime_type: 'font/woff2', data_base64: '', retained: true });
});
