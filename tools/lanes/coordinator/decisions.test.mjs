// MP-08 / MP-09 / MP-10 / MP-11: the routine/critical split for provider decisions.
import test from 'node:test';
import assert from 'node:assert/strict';
import { classifyDecision } from './decisions.mjs';

const routine = { id: 'p', kind: 'permission', level: 'warning', ownerOnly: false, title: 'Approve Claude Code Bash?', choices: ['allow_once', 'allow_session', 'deny'] };

test('an ordinary Claude tool permission with an allow choice is routine', () => {
  assert.deepEqual(classifyDecision(routine), { routine: true });
  assert.deepEqual(classifyDecision({ ...routine, choices: ['allow_once'] }), { routine: true });
});

test('everything else goes to the coordinator, each with its own reason', () => {
  const reason = changes => classifyDecision({ ...routine, ...changes }).reason;
  assert.equal(reason({ level: 'critical' }), 'critical');
  assert.equal(reason({ ownerOnly: true }), 'owner_only');
  assert.equal(reason({ kind: 'choice' }), 'question');
  assert.equal(reason({ title: 'Approve Claude Code credential read?' }), 'credential_related');
  assert.equal(reason({ title: 'Unlock the Vault with your passkey' }), 'credential_related');
  assert.equal(reason({ title: 'Approve Codex command?' }), 'not_a_claude_permission');
  assert.equal(reason({ choices: ['deny'] }), 'no_allow_choice');
  assert.equal(classifyDecision(undefined).reason, 'unclassified');
});
