// Connect to a coordinator-prepared disposable kernel seat. This runner never
// creates a kernel home, relay identity, credential, helper build or grant.
import assert from 'node:assert/strict';
import { revokeDuringInput } from './seat-revocation.mjs';
import { verifyNativeCapture } from './verify-native-capture.mjs';
import { readFileSync, writeFileSync, mkdirSync, statSync, existsSync } from 'node:fs';
import path from 'node:path';
import { parseArgs } from 'node:util';
const { values: args } = parseArgs({ options: {
  port: { type: 'string' }, 'token-file': { type: 'string' }, 'scope-file': { type: 'string' },
  'fixture-log': { type: 'string' }, evidence: { type: 'string' },
} });
for (const key of ['token-file', 'scope-file', 'fixture-log', 'evidence']) assert.ok(args[key] && path.isAbsolute(args[key]), `${key} must be absolute`);
assert.ok(args.evidence.startsWith('/Users/miguel/.codex/evidence/'), 'evidence must stay under the owner evidence root');
assert.ok(Number.isInteger(Number(args.port)) && Number(args.port) > 0 && Number(args.port) < 65536, 'valid approved endpoint port required');
const tokenStat = statSync(args['token-file']);
assert.ok(tokenStat.isFile() && (tokenStat.mode & 0o077) === 0 && tokenStat.uid === process.getuid(), 'private local kernel token required');
const scope = JSON.parse(readFileSync(args['scope-file'], 'utf8'));
assert.ok(scope.pid > 0 && scope.window > 0 && [1, 2].includes(scope.scale));
assert.ok(scope.bounds && scope.point && Number.isInteger(scope.expected_location));
assert.ok(scope.revoke_point && Number.isInteger(scope.revoke_location) && scope.revoke_location !== scope.expected_location);
const x = Math.round((scope.point.x - scope.bounds.X) * scope.scale);
const y = Math.round((scope.point.y - scope.bounds.Y) * scope.scale);
assert.ok(x >= 0 && y >= 0, 'bound fixture point required');
const lane = '/Users/miguel/.chariox/dev/browser-computer-use/resume-20260930/agents/cumac';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const deadline = Date.now() + 20 * 60_000;
while (existsSync(path.join(lane, 'GUI_HOLD')) && !existsSync(path.join(lane, 'GUI_OK')) && Date.now() < deadline) {
  writeFileSync(path.join(lane, 'GUI_READY.md'), 'R6 seat revocation drill waits for GUI_HOLD.\n');
  await sleep(Math.min(60_000, deadline - Date.now()));
}
mkdirSync(args.evidence, { recursive: true });
const report = { status: 'FAIL', checks: [], started_at: new Date().toISOString() };
const pending = new Map(); let sequence = 0, socket, control;
const selected = () => [...readFileSync(args['fixture-log'], 'utf8').matchAll(/fixture selection location=(\d+) length=(\d+)/g)].at(-1)?.slice(1).map(Number);
function computer(command, channel = socket) {
  const id = `cumac-r6-${++sequence}`;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error('kernel request timeout')); }, 10_000);
    pending.set(id, frame => {
      clearTimeout(timer); pending.delete(id);
      if (frame.response?.KernelBrowser) resolve(frame.response.KernelBrowser.result);
      else { const error=frame.error ?? {code:'kernel_refused'}; reject(Object.assign(new Error(error.message ?? 'kernel refused'), {code:error.code})); }
    });
    const request = command.op === 'revoke_grants' ? { KernelBrowser: { command } } : { KernelBrowser: { command: { op: 'computer', command } } };
    channel.send(JSON.stringify({ type: 'request', request_id: id, command_id: id, request }));
  });
}
try {
  // Authorization bytes remain inside this process/header, never in reports.
  socket = new WebSocket(`ws://127.0.0.1:${Number(args.port)}`, { headers: { authorization: `Bearer ${readFileSync(args['token-file'], 'utf8').trim()}` } });
  socket.onmessage = event => { const frame = JSON.parse(event.data); pending.get(frame.request_id)?.(frame); };
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = () => reject(new Error('local kernel connection failed')); });
  const state = await computer({ op: 'start' });
  assert.equal(state.mode, 'scoped-drill');
  const target = { surface_id: state.surface_id, generation: state.generation };
  const input = { op: 'input', target, input: { kind: 'click', x, y, button: 1 } };
  const delivered = await computer(input);
  assert.equal(delivered.dispatched, true);
  assert.ok(delivered.receipts.some(value => value.includes(`location=${scope.expected_location} length=0`)));
  await sleep(100); assert.deepEqual(selected(), [scope.expected_location, 0]);
  report.checks.push({ name: 'live-paired-caret-before-revoke', passed: true });
  const snapshot = await computer({ op: 'screenshot', target });
  assert.equal(snapshot.mime_type, 'image/png');
  assert.equal(snapshot.surface_id, target.surface_id); assert.equal(snapshot.generation, target.generation);
  assert.equal(snapshot.width, state.width); assert.equal(snapshot.height, state.height);
  writeFileSync(path.join(args.evidence, 'protected-fixture.png'), Buffer.from(snapshot.data_base64, 'base64'), { mode: 0o600 });
  report.checks.push({ name: 'live-paired-protected-screenshot', passed: true, width: snapshot.width, height: snapshot.height, protected: snapshot.protected });
  const damageInput = { ...input, input: { kind: 'click', button: 1,
    x: Math.round((scope.revoke_point.x - scope.bounds.X) * scope.scale),
    y: Math.round((scope.revoke_point.y - scope.bounds.Y) * scope.scale) } };
  const damageReceipt = await computer(damageInput);
  assert.ok(damageReceipt.receipts.some(value => value.includes(`location=${scope.revoke_location} length=0`)));
  await sleep(100); assert.deepEqual(selected(), [scope.revoke_location, 0]);
  await sleep(100);
  const changed = await computer({ op: 'screenshot', target });
  assert.equal(changed.width, snapshot.width); assert.equal(changed.height, snapshot.height);
  const changedPath = path.join(args.evidence, 'protected-fixture-changed.png');
  writeFileSync(changedPath, Buffer.from(changed.data_base64, 'base64'), { mode: 0o600 });
  report.checks.push(verifyNativeCapture(path.join(args.evidence, 'protected-fixture.png'), changedPath, scope.scale, args.evidence));
  // Independent authenticated connection lets Revoke all cancel before the
  // in-flight input request releases the native seat lock.
  control = new WebSocket(`ws://127.0.0.1:${Number(args.port)}`, { headers: { authorization: `Bearer ${readFileSync(args['token-file'], 'utf8').trim()}` } });
  control.onmessage = event => { const frame = JSON.parse(event.data); pending.get(frame.request_id)?.(frame); };
  await new Promise((resolve, reject) => { control.onopen = resolve; control.onerror = () => reject(new Error('control connection failed')); });
  const revokeInput = input;
  const revoked = await revokeDuringInput({ dispatch: () => computer(revokeInput),
    observe: selected, changed: value => value?.[0] === scope.expected_location && value?.[1] === 0,
    revoke: () => computer({ op: 'revoke_grants', agent_id: null }, control) });
  const before = selected();
  const denial = await computer(input).then(() => null, error => error.message);
  assert.ok(denial && /explicitly start|not running|not_granted|revoked|unavailable/.test(denial), 'old seat must refuse after Revoke all');
  await sleep(500); assert.deepEqual(selected(), before, 'no input after revoke acknowledgment');
  report.checks.push({ name: 'kernel-revoke-mid-input-stops-input', passed: true,
    ...revoked, selection_unchanged_after_ack: true });
  report.status = 'PASS';
} catch (error) {
  // No untrusted target strings, frames or authentication values enter the report.
  report.failure = /permission|helper refused/.test(String(error)) ? 'native-drill-refused' : 'seat-drill-assertion-or-transport-failed';
  process.exitCode = 1;
} finally {
  // Stop only the explicitly configured native seat; never delete kernel state.
  if (socket?.readyState === WebSocket.OPEN) await computer({ op: 'revoke_grants', agent_id: null }).catch(() => {});
  socket?.close(); control?.close(); report.finished_at = new Date().toISOString();
  writeFileSync(path.join(args.evidence, 'result.json'), JSON.stringify(report, null, 2) + '\n');
}
console.log(`${report.status} paired grant revocation and protected capture`);
