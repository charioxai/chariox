import test from 'node:test';
import assert from 'node:assert/strict';
import { fileTransfer } from '../view/transfer.mjs';

test('file transfer preserves multi-part Unicode within existing App-call budgets', async () => {
  const original = ('中文 😀 \" \\ \n').repeat(15000);
  let uploaded = '', offered = 0, parts = 0;
  const transfer = fileTransfer({ async call(name, input) {
    assert.ok(Buffer.byteLength(JSON.stringify(input)) < 512 * 1024);
    if (name === 'write_file') {
      parts++;
      assert.equal(Buffer.byteLength(uploaded), input.offset);
      assert.ok(!/[\ud800-\udbff]$/.test(input.text));
      uploaded += input.text;
      if (input.final && input.offer) offered++;
      return { bytes: Buffer.byteLength(uploaded), grantId: input.final ? 'offer' : null };
    }
    assert.equal(name, 'read_file');
    assert.equal(input.grantId, input.offset ? undefined : 'grant');
    const bytes = Buffer.from(original), start = input.offset;
    let end = Math.min(start + 65536, bytes.length);
    while (end < bytes.length && (bytes[end] & 0xc0) === 0x80) end--;
    return { text: bytes.subarray(start, end).toString(), next: end, done: end === bytes.length };
  } });
  await transfer.write({ name: 'unicode.univer.json', text: original, offer: true });
  assert.equal(uploaded, original);
  assert.ok(parts > 2);
  assert.equal(offered, 1);
  assert.equal((await transfer.readFile({ name: 'unicode.univer.json', grantId: 'grant' })).text, original);
});

test('loading refuses chunks from different saved versions', async () => {
  const transfer = fileTransfer({ async call(_, { offset }) {
    return { text: '{}', next: offset + 2, done: offset > 0, version: offset ? '2' : '1' };
  } });
  await assert.rejects(transfer.load(), /changed while loading/);
});
