// MP-08/MP-10/MP-11: supplementary privacy/export checks, never live acceptance.
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {mkdtemp, readFile, rm, stat, symlink} from 'node:fs/promises';
import path from 'node:path';
import {Readable} from 'node:stream';
import {captureHomeRequestTiming, homeRequestTimingRow, homeRequestTimingSummary} from './home-request-timing.mjs';
const mark = (stage, packet = 123, at_ms = 1000) => `MP-10-PACKET-TIMING ${JSON.stringify({stage, packet, at_ms})}`;
async function scratch(fn) {
  assert(process.env.TMPDIR && path.isAbsolute(process.env.TMPDIR));
  const root = await mkdtemp(path.join(process.env.TMPDIR, 'home-timing-'));
  try {await fn(root);} finally {await rm(root, {recursive: true, force: true});}
}
test('exports receive/end across arbitrary chunks and retains only numeric opaque joins', () => scratch(async root => {
  const start = mark('kernel_receive'), end = mark('kernel_request_end', 123, 1234);
  const stats = await captureHomeRequestTiming(Readable.from([start.slice(0, 12), start.slice(12) + '\n', end]), root);
  const rows = (await readFile(path.join(root, 'home-request-timing.jsonl'), 'utf8')).trim().split('\n').map(JSON.parse);
  assert.deepEqual(rows, [homeRequestTimingRow(start), homeRequestTimingRow(end)]);
  assert.equal(stats.receive, 1); assert.equal(stats.end, 1);
  assert.equal(rows[1].at_ms - rows[0].at_ms, 234);
  assert.equal((await stat(path.join(root, 'home-request-timing.jsonl'))).mode & 0o777, 0o600);
}));
test('drops raw diagnostics, auth envelopes, unknown stages/fields and invalid joins', () => scratch(async root => {
  const canary = 'synthetic-private-canary';
  const rows = [canary, `MP-10-PACKET-TIMING {"stage":"kernel_receive","packet":1,"at_ms":1,"auth":"${canary}"}`,
    mark(canary), mark('kernel_receive', canary), mark('kernel_receive', -1), mark('kernel_receive', 2 ** 52),
    mark('kernel_receive', 1.5), mark('kernel_receive', 1, 0), 'MP-10-PACKET-TIMING null',
    'MP-10-PACKET-TIMING {', `{"auth_token":"${canary}"}`, mark('kernel_receive')];
  const stats = await captureHomeRequestTiming(Readable.from([rows.join('\n') + '\n']), root);
  const output = await readFile(path.join(root, 'home-request-timing.jsonl'), 'utf8');
  assert.equal(stats.receive, 1); assert.equal(stats.discarded, rows.length - 1);
  assert(!output.includes(canary)); assert.equal(output.trim(), JSON.stringify(homeRequestTimingRow(mark('kernel_receive'))));
}));
test('bounds oversized lines across chunks and recovers on the next newline', () => scratch(async root => {
  const stats = await captureHomeRequestTiming(Readable.from(['x'.repeat(3000), 'y'.repeat(3000), '\n' + mark('kernel_receive') + '\n']), root);
  assert.equal(stats.oversized, 1); assert.equal(stats.receive, 1);
}));
test('bounds output and never overwrites existing files or follows a leaf symlink', () => scratch(async root => {
  const stats = await captureHomeRequestTiming(Readable.from([mark('kernel_receive') + '\n' + mark('kernel_request_end')]), root, {maxBytes: 90});
  assert(stats.capped); assert(stats.bytes <= 90);
  await assert.rejects(captureHomeRequestTiming(Readable.from([]), root), {code: 'EEXIST'});
  const other = path.join(root, 'other');
  const {mkdir} = await import('node:fs/promises'); await mkdir(other);
  await symlink(path.join(root, 'home-request-timing.jsonl'), path.join(other, 'home-request-timing.jsonl'));
  await assert.rejects(captureHomeRequestTiming(Readable.from([]), other), {code: 'EEXIST'});
}));

test('MP-10 a viewer finishing during a collector write uses complete joined spans', () => {
  const rows = [mark('kernel_receive'), mark('kernel_request_end', 123, 1234)]
    .map(line => JSON.stringify(homeRequestTimingRow(line))).join('\n') + '\n';
  assert.deepEqual(homeRequestTimingSummary(rows + '{"stage":"kernel_'), {rows: 2, completed: 1, partial_tail_ignored: true});
  assert.equal(homeRequestTimingSummary(rows).partial_tail_ignored, false);
  assert.throws(() => homeRequestTimingSummary(rows + '{"private":"synthetic"}\n'), /invalid home timing evidence/);
  assert.throws(() => homeRequestTimingSummary(JSON.stringify(homeRequestTimingRow(mark('kernel_receive'))) + '\n'), /missing or invalid/);
});
