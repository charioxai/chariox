// Connect to a coordinator-prepared disposable kernel seat. This runner never
// creates a kernel home, relay identity, credential, helper build or grant.
// The owner starts and revokes; native input comes only from the admitted agent.
import assert from 'node:assert/strict';
import { revokeDuringInput, requirePhysicalClick } from './seat-revocation.mjs';
import { verifyNativeCapture } from './verify-native-capture.mjs';
import { connectKernel, createAgentCallback } from './agent-admission-drill.mjs';
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import path from 'node:path';
import { parseArgs } from 'node:util';
const { values: args } = parseArgs({ options: {
  workspace: { type: 'string' }, 'scope-file': { type: 'string' },
  'fixture-log': { type: 'string' }, evidence: { type: 'string' },
} });
for (const key of ['workspace', 'scope-file', 'fixture-log', 'evidence']) assert.ok(args[key] && path.isAbsolute(args[key]), `${key} must be absolute`);
assert.ok(args.evidence.startsWith('/Users/miguel/.codex/evidence/'), 'evidence must stay under the owner evidence root');
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
let owner, control, callback;
const selected = () => [...readFileSync(args['fixture-log'], 'utf8').matchAll(/fixture selection location=(\d+) length=(\d+)/g)].at(-1)?.slice(1).map(Number);
const computer = async (client, command) => (await client.send({ KernelBrowser: { command: { op: 'computer', command } } })).KernelBrowser.result;
const revokeAll = client => client.send({ KernelBrowser: { command: { op: 'revoke_grants', agent_id: null } } });
async function agent(command) {
  const envelope = await callback.input(command);
  const text = envelope?.result?.content?.find(item => item.type === 'text')?.text;
  if (envelope?.error || envelope?.result?.isError) {
    // Keep only the kernel refusal code; the message is product text.
    const message = String(envelope.error?.message ?? text ?? '');
    const code = message.match(/\b(browser_action_cancelled|user_domain_not_granted|not_granted)\b/)?.[1] ?? 'agent_refused';
    throw Object.assign(new Error(`agent Computer refused: ${code}`), { code });
  }
  return JSON.parse(text);
}
try {
  owner = connectKernel();
  callback = await createAgentCallback(owner, args.workspace);
  await callback.call('tools/call', { name: 'chariox.load_kernel_computer', arguments: {} });
  const state = await computer(owner, { op: 'start' });
  assert.equal(state.mode, 'scoped-drill');
  const target = { surface_id: state.surface_id, generation: state.generation };
  const input = { op: 'input', target, input: { kind: 'click', x, y, button: 1 } };
  requirePhysicalClick(await agent(input));
  await sleep(100); assert.deepEqual(selected(), [scope.expected_location, 0]);
  report.checks.push({ name: 'live-paired-agent-caret-before-revoke', passed: true });
  const snapshot = await computer(owner, { op: 'screenshot', target });
  assert.equal(snapshot.mime_type, 'image/png');
  assert.equal(snapshot.surface_id, target.surface_id); assert.equal(snapshot.generation, target.generation);
  assert.equal(snapshot.width, state.width); assert.equal(snapshot.height, state.height);
  writeFileSync(path.join(args.evidence, 'protected-fixture.png'), Buffer.from(snapshot.data_base64, 'base64'), { mode: 0o600 });
  report.checks.push({ name: 'live-paired-protected-screenshot', passed: true, width: snapshot.width, height: snapshot.height, protected: snapshot.protected });
  const damageInput = { ...input, input: { kind: 'click', button: 1,
    x: Math.round((scope.revoke_point.x - scope.bounds.X) * scope.scale),
    y: Math.round((scope.revoke_point.y - scope.bounds.Y) * scope.scale) } };
  requirePhysicalClick(await agent(damageInput));
  await sleep(100); assert.deepEqual(selected(), [scope.revoke_location, 0]);
  await sleep(100);
  const changed = await computer(owner, { op: 'screenshot', target });
  assert.equal(changed.width, snapshot.width); assert.equal(changed.height, snapshot.height);
  const changedPath = path.join(args.evidence, 'protected-fixture-changed.png');
  writeFileSync(changedPath, Buffer.from(changed.data_base64, 'base64'), { mode: 0o600 });
  report.checks.push(verifyNativeCapture(path.join(args.evidence, 'protected-fixture.png'), changedPath, scope.scale, args.evidence));
  // Independent owner connection lets Revoke all cancel while agent input is in flight.
  control = connectKernel();
  await control.ready;
  // The owner's shared Action ledger is the cancellation authority: the agent's
  // own Action must be running at revoke and end cancelled by the controller.
  const actions = async () => (await computer(control, { op: 'actors' })).actions;
  let inFlight;
  const revoked = await revokeDuringInput({ dispatch: () => agent(input),
    observe: selected, changed: value => value?.[0] === scope.expected_location && value?.[1] === 0,
    revoke: async () => {
      const running = (await actions()).filter(action => action.state === 'running');
      assert.equal(running.length, 1, 'agent input must be running at revoke');
      inFlight = running[0].action_id;
      return revokeAll(control);
    },
    cancelled: async error => {
      const action = (await actions()).find(action => action.action_id === inFlight);
      return error.code === 'user_domain_not_granted' && action?.state === 'cancelled'
        && action.outcome?.status === 'cancelled' && action.outcome?.reason === 'controller_cancellation';
    } });
  const before = selected();
  const denial = await agent(input).then(() => null, error => error.code);
  assert.ok(/not_granted/.test(denial ?? ''), 'revoked agent must be refused');
  await sleep(500); assert.deepEqual(selected(), before, 'no input after revoke acknowledgment');
  report.checks.push({ name: 'kernel-revoke-mid-input-stops-input', passed: true,
    ...revoked, action_id: inFlight, selection_unchanged_after_ack: true });
  report.status = 'PASS';
} catch (error) {
  // No untrusted target strings, frames or authentication values enter the report.
  report.failure = /permission|helper refused/.test(String(error)) ? 'native-drill-refused' : 'seat-drill-assertion-or-transport-failed';
  // Kernel refusals keep only their product code; local check messages are ours.
  report.error = error.code ?? String(error.message).slice(0, 300);
  process.exitCode = 1;
} finally {
  // Stop only the explicitly configured native seat; never delete kernel state.
  if (owner) await revokeAll(owner).catch(() => {});
  if (callback) await callback.stop().catch(() => {});
  owner?.close(); control?.close(); report.finished_at = new Date().toISOString();
  writeFileSync(path.join(args.evidence, 'result.json'), JSON.stringify(report, null, 2) + '\n');
}
console.log(`${report.status} paired grant revocation and protected capture`);
